//! The engine — run and simulation alike.
//!
//! The debugger-shaped surface of the blueprint: [`Vm::boot`] once,
//! [`Vm::step`] for exactly one node transaction, [`Vm::drain`] to
//! quiescence. No I/O, no clock, no randomness live here: the world
//! enters through the journal and the boundary traits, and the same
//! machine serves run and simulation.
//!
//! # The delivery protocol
//!
//! A transaction is atomic in [`TransactionExecutor`]; this module
//! keeps it atomic across the machine. Every step follows the same
//! discipline: *reserve* the head of a mailbox without removing it,
//! *evaluate* on isolated state, *reserve capacity* in every target
//! mailbox, *commit* to the journal — and only then remove the
//! delivery, apply the state, and route. A fault or refusal anywhere
//! before the commit leaves the delivery exactly where it was; after
//! the commit, nothing can fail.

use crate::boundary::{
    BoundaryEvent, CallRef, ClosingDeadline, Completion, DatedPayload, Dispatcher,
    InboundAcceptance, InboundAdmission, InboundProposal, InboundRejection, InboundSession,
    InboundSubmission, InstanceDeadline, Intent, OutboundConnectorBindings, RequestPreparation,
    Response, SchedulerCompletion,
};
use crate::envelope::{Data, Envelope, Id};
use crate::identity::{InstanceAddress, InstanceFrame, InstanceRef};
use crate::journal::{Journal, RouteTarget, RoutedDelivery, ids};
use crate::node;
use crate::plan::{
    AttrSetting, Edge, EndpointRef, EvaluationSite, GraphVersion, InstanceLifetime, NodeKind,
    OwnerKind, OwnerPath, OwnerSlot, Slot, ValidatedExecutionPlan,
};
use crate::runtime::{
    DataSlot, Delivery, Fault, InstanceState, JoinCandidate, LifecycleFiring, Mailboxes,
    TransactionExecutor, eval_lazy, meta_value, resolve_payload,
};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

/// A machine fault; the step that raised it committed nothing, and
/// the work that triggered it stays reserved — a retry sees it again.
/// A machine-integrity refusal — never a business error.
#[derive(Debug, Clone, PartialEq)]
pub enum VmError {
    /// An evaluation or engine fault. It is outside every graph port
    /// contract: the transaction commits nothing and supervision decides.
    Fault(Fault),
    /// The journal refused an operation.
    Journal(crate::journal::Error),
    /// The dispatcher raised an integrity fault.
    Dispatch(crate::boundary::DispatchError),
    /// A target mailbox has no room for a transaction's deliveries —
    /// backpressure surfaced before the commit.
    MailboxFull {
        /// The refusing node.
        node: node::Path,
    },
    /// A route reached an owner generation whose finite lifetime has already
    /// won. The producer must retain normal backpressure; it cannot journal a
    /// route into the closing generation.
    InstanceClosing {
        /// Owner refusing new external admission.
        owner: OwnerPath,
    },
    /// A processing-time box has already closed. Its address has no second
    /// forward generation, so retry cannot reopen or spill it.
    ClosedTimeBox {
        /// Tumbling owner whose box is closed.
        owner: OwnerPath,
        /// Canonical start of the closed box.
        start_ns: i64,
    },
    /// A causal closure arrived for a call this machine holds no
    /// pending request for — an integrity breach, never a silence.
    MissingPending(CallRef),
    /// A plan state the validator's invariants exclude reached the
    /// machine anyway — the typed belt behind those doors, never a
    /// panic and never a data-representation fault.
    PlanInvariant {
        /// The offending node.
        node: node::Path,
        /// The violated invariant.
        reason: &'static str,
    },
    /// An injection at a port that is not a materialized graph input.
    /// Checked sources always materialize conventional `in`; the raw API is
    /// verified, not trusted.
    UnknownIngress(node::Port),
    /// A Driver proposal names no inbound Connector output in the validated
    /// plan.
    UnknownInboundSource {
        /// Proposed Connector node.
        connector: node::Path,
        /// Proposed output port.
        port: node::Port,
    },
    /// An ingress draft carries metadata outside the declared vocabulary or
    /// outside a declared field's type.
    IngressMetadataViolation {
        /// Rejected metadata field.
        field: String,
        /// Stable human-readable contract judgement.
        reason: String,
    },
    /// An ingress draft that violates a pin — the ingress port's own
    /// or a destination's it would fan out to. The CLIENT's value is
    /// wrong, not the plan; refused before any append, with the
    /// STRUCTURE intact: where it was submitted, where it was
    /// refused, and the judgement.
    IngressPinViolation {
        /// The submission's port.
        ingress: node::Port,
        /// The endpoint whose pin actually refused — the ingress
        /// itself, or a fan-out destination.
        rejected_at: EndpointRef,
        /// The SEMANTIC judgement: expected type, found kind, and
        /// the reason — a tool matches, `Display` renders.
        violation: Box<crate::runtime::ConformanceViolation>,
    },
}

fn expression_roots(expression: &crate::expr::Expr, roots: &mut Vec<String>) {
    let visit = |child, roots: &mut Vec<String>| expression_roots(child, roots);
    match expression {
        crate::expr::Expr::Literal(_) | crate::expr::Expr::Asset(_) => {}
        crate::expr::Expr::Path(segments) => {
            if let Some(root) = segments.first()
                && !roots.contains(root)
            {
                roots.push(root.clone());
            }
        }
        crate::expr::Expr::Unary { expr, .. }
        | crate::expr::Expr::Is { expr, .. }
        | crate::expr::Expr::As { expr, .. } => visit(expr, roots),
        crate::expr::Expr::Binary { left, right, .. }
        | crate::expr::Expr::Index {
            base: left,
            index: right,
        } => {
            visit(left, roots);
            visit(right, roots);
        }
        crate::expr::Expr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            visit(cond, roots);
            visit(then, roots);
            visit(otherwise, roots);
        }
        crate::expr::Expr::Call { args, .. } | crate::expr::Expr::List(args) => {
            for arg in args {
                visit(arg, roots);
            }
        }
        crate::expr::Expr::MapLit(fields)
        | crate::expr::Expr::Struct(fields)
        | crate::expr::Expr::Tagged { fields, .. } => {
            for (_, value) in fields {
                visit(value, roots);
            }
        }
    }
}

impl fmt::Display for VmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fault(fault) => fault.fmt(f),
            Self::Journal(error) => error.fmt(f),
            Self::Dispatch(error) => error.fmt(f),
            Self::MailboxFull { node } => {
                write!(f, "the mailbox of `{node}` is full")
            }
            Self::InstanceClosing { owner } => {
                write!(f, "instance owner `{owner}` is closing")
            }
            Self::ClosedTimeBox { owner, start_ns } => write!(
                f,
                "processing-time box {start_ns} of `{owner}` is already closed"
            ),
            Self::MissingPending(_) => {
                f.write_str("a closure settled a call with no pending request")
            }
            Self::PlanInvariant { node, reason } => {
                write!(f, "`{node}` violates a validated-plan invariant: {reason}")
            }
            Self::UnknownIngress(port) => {
                write!(f, "`{port}` is not an input of this graph")
            }
            Self::UnknownInboundSource { connector, port } => write!(
                f,
                "`{connector}.{port}` is not an inbound Connector output of this graph"
            ),
            Self::IngressMetadataViolation { field, reason } => {
                write!(
                    f,
                    "the draft metadata field `meta.ext.{field}` is invalid: {reason}"
                )
            }
            Self::IngressPinViolation {
                ingress, violation, ..
            } => {
                write!(f, "the draft cannot enter `{ingress}`: {violation}")
            }
        }
    }
}

impl core::error::Error for VmError {}

/// What one boundary round observed besides causal progress: the
/// closures that did not become transactions. Nothing here is silent —
/// the caller decides what to surface.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PumpReport {
    /// Closures that settled into pending completion transactions.
    pub settled: usize,
    /// Late answers to already-closed calls — observations, not
    /// causal facts.
    pub discarded: Vec<Response>,
    /// Answers matching no journaled call at all.
    pub uncorrelated: Vec<Response>,
    /// Answers the contract refused before closure, with reasons —
    /// the call stays open for the timeout machinery.
    pub rejected: Vec<(Response, String)>,
}

impl PumpReport {
    fn append(&mut self, mut other: Self) {
        self.settled = self.settled.saturating_add(other.settled);
        self.discarded.append(&mut other.discarded);
        self.uncorrelated.append(&mut other.uncorrelated);
        self.rejected.append(&mut other.rejected);
    }
}

/// One held request: the delivery whose cursor the completion
/// transaction will consume. A budget obligation resolves against
/// its reference — the canonical edge and the envelope id, both on
/// the delivery itself.
#[derive(Debug, Clone, PartialEq)]
struct PendingCall {
    slot: Slot,
    delivery: Delivery,
    instance: InstanceRef,
}

/// One selected owner generation and its immutable instance bundle.
#[derive(Debug, Clone)]
struct Instance {
    owner: OwnerSlot,
    parent: Option<InstanceRef>,
    frames: Vec<InstanceFrame>,
    values: BTreeMap<String, crate::value::Value>,
    settings: BTreeMap<String, crate::value::Value>,
    created_by: Envelope,
    last_activity: Envelope,
    /// Deliveries discarded only after the drain budget expired. Their cursor
    /// positions are sealed by this instance's eventual `l:stop`.
    residual: Vec<Delivery>,
    phase: InstancePhase,
}

/// Admission and closure are mutually exclusive states. The first close
/// request owns the causal envelope; later expiries are stale observations.
#[derive(Debug, Clone)]
enum InstancePhase {
    Open,
    Closing {
        cause: Envelope,
        scope: InstanceRef,
        stage: ClosingStage,
    },
    Stopping {
        cause: Envelope,
        scope: InstanceRef,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClosingStage {
    Draining,
    Settling,
}

/// Pure instance construction. Installation happens only after the producer
/// route commits.
#[derive(Debug, Clone)]
struct InstanceCandidate {
    owner: OwnerSlot,
    reference: InstanceRef,
    parent: Option<InstanceRef>,
    frames: Vec<InstanceFrame>,
    values: BTreeMap<String, crate::value::Value>,
    settings: BTreeMap<String, crate::value::Value>,
    /// Root admission claimed by a single-pass owner. Reuse is legal only for
    /// fan-out of this exact envelope inside the same atomic preparation.
    single_pass_root: Option<Id>,
}

/// A successor-generation bundle selected and journaled while its previous
/// generation is still Closing. Deliveries wait in the ordinary bounded
/// mailbox; this map carries only the immutable instance bundle needed when
/// the journal fold allows `l:start`.
#[derive(Debug, Clone)]
struct WaitingInstance {
    candidate: InstanceCandidate,
    created_by: Envelope,
    last_activity: Envelope,
    last_activity_at: i64,
}

#[derive(Debug, Clone)]
struct PreparedTarget {
    reference: InstanceRef,
    scope: InstanceRef,
    chain: Vec<InstanceRef>,
    /// Owners whose boundary this route crosses. Sticky ancestors are not
    /// re-admitted and therefore do not move an idle deadline.
    admitted: Vec<InstanceRef>,
}

/// Lifecycle work owns its causal envelope and names the lexical owner. The
/// live bundle remains installed through `l:stop` so stop emissions retain
/// their complete source chain; reclamation follows the successful commit.
#[derive(Debug, Clone)]
struct LifecycleWork {
    owner: OwnerSlot,
    instance: InstanceRef,
    parent_instance: Option<InstanceRef>,
    lifecycle: node::Lifecycle,
    cause: Envelope,
}

/// One edge whose target instance was selected before the causing record
/// committed. Reusing this value after the commit makes routing infallible and
/// prevents a replay-sensitive second evaluation of `instances.key`.
#[derive(Debug, Clone)]
struct PreparedEdge {
    edge: Edge,
    target: Option<PreparedTarget>,
}

/// One emission coupled to the edge fan-out selected for its exact envelope.
#[derive(Debug)]
struct PreparedEmission {
    emission: crate::journal::Emission,
    routes: Vec<PreparedEdge>,
}

/// A transaction's post-commit work. The private constructor couples every
/// emission to its routes, so routing cannot lose alignment after commit.
#[derive(Debug)]
struct PreparedEmissions {
    emissions: Vec<PreparedEmission>,
    candidates: BTreeMap<InstanceRef, InstanceCandidate>,
}

/// One Operator activation selected without removing mailbox work. Join
/// inputs remain ordinary durable routed deliveries until their transaction
/// commits; an eviction follows the same reserve/commit/remove discipline.
#[derive(Debug, Clone)]
enum OperatorWork {
    Single(Delivery),
    Join {
        activation: node::HandlerActivation,
        trigger: Delivery,
        inputs: Vec<Delivery>,
    },
    Evict(Delivery),
}

fn consider_operator_work(
    best: &mut Option<(usize, u8, OperatorWork)>,
    at: usize,
    rank: u8,
    work: OperatorWork,
) {
    if best
        .as_ref()
        .is_none_or(|(current, current_rank, _)| (at, rank) < (*current, *current_rank))
    {
        *best = Some((at, rank, work));
    }
}

/// The plan-backed gate for one atomic boundary completion.
///
/// Format v10 retains every Connector's exact resolved output types. This
/// gate judges that table, so no malformed Driver fact can become causal.
struct PlanGate<'plan> {
    plan: &'plan ValidatedExecutionPlan,
}

/// The explicit no-world binding used by the pure `step`/`drain` helpers.
/// It lets Operator-only plans keep their compact API while a Connector call
/// fails loudly instead of falling back to a concrete built-in contract.
struct NoOutboundBindings;

impl OutboundConnectorBindings for NoOutboundBindings {
    fn prepare_request(
        &mut self,
        connector: &node::Path,
        _: &Data,
        _: &BTreeMap<String, crate::value::Value>,
    ) -> Result<RequestPreparation, crate::boundary::DriverRouteError> {
        Err(crate::boundary::DriverRouteError::UnboundOutbound(
            connector.clone(),
        ))
    }

    fn scheduler_completion(
        &mut self,
        connector: &node::Path,
        _: SchedulerCompletion,
    ) -> Result<Completion, crate::boundary::DriverRouteError> {
        Err(crate::boundary::DriverRouteError::UnboundOutbound(
            connector.clone(),
        ))
    }

    fn cancel(
        &mut self,
        connector: &node::Path,
        _: &CallRef,
    ) -> Result<(), crate::boundary::DriverRouteError> {
        Err(crate::boundary::DriverRouteError::UnboundOutbound(
            connector.clone(),
        ))
    }

    fn start(
        &mut self,
        intent: &Intent,
        _: &Envelope,
    ) -> Result<crate::boundary::CallStart, crate::boundary::DriverRouteError> {
        Err(crate::boundary::DriverRouteError::UnboundOutbound(
            intent.connector_path.clone(),
        ))
    }

    fn events(&mut self) -> Result<Vec<Response>, crate::boundary::DriverRouteError> {
        Ok(Vec::new())
    }
}

impl crate::boundary::CompletionGate for PlanGate<'_> {
    fn admit(&self, connector: &node::Path, completion: &Completion) -> Result<(), String> {
        let Some((index, planned)) = self
            .plan
            .plan()
            .nodes
            .iter()
            .enumerate()
            .find(|(_, node)| &node.path == connector)
        else {
            return Err(format!("`{connector}` is absent from the validated plan"));
        };
        let Some(_) = planned.connector() else {
            return Err(format!("`{connector}` is not a connector"));
        };
        let slot =
            Slot::new(u32::try_from(index).map_err(|_| {
                format!("the slot of `{connector}` is not representable by the plan")
            })?);
        for output in completion.outputs() {
            let resolved = self
                .plan
                .resolved_ports()
                .node(slot)
                .and_then(|ports| ports.iter().find(|port| port.port() == &output.port))
                .ok_or_else(|| {
                    format!(
                        "`{connector}` has no resolved port `{}`",
                        output.port.name()
                    )
                })?;
            if resolved.direction() != node::Direction::Output {
                return Err(format!(
                    "`{connector}.{}` is not an output port",
                    output.port.name()
                ));
            }
            // Validation has already proved the transported contract row.
            // The resolved table is therefore the sole admission authority
            // here too, independently of the concrete Connector family.
            let expected = resolved.ty();
            if output.port.name() == "exit" && !matches!(&output.data, Data::Materialized(_)) {
                return Err(format!(
                    "`{connector}.exit` must carry a materialized terminal union"
                ));
            }
            if let Data::Materialized(value) = &output.data
                && let Some(violation) = expected.violation(output.port.name(), value)
            {
                return Err(format!(
                    "`{connector}.{}` refuses the completion value: {violation}",
                    output.port.name()
                ));
            }
            if let Data::Materialized(value) = &output.data {
                let from = EndpointRef {
                    node: Some(slot),
                    port: output.port.clone(),
                };
                crate::runtime::crossing_pins_hold(self.plan, &from, value)
                    .map_err(|refused| refused.into_fault().to_string())?;
            }
            for (field, value) in &output.ext {
                let expected = crate::plan::standard_ext_field_type(field).or_else(|| {
                    self.plan
                        .plan()
                        .ext_schema
                        .iter()
                        .find(|(known, _)| known == field)
                        .map(|(_, ty)| ty.clone())
                });
                let Some(expected) = expected else {
                    return Err(format!(
                        "`{connector}` returned undeclared metadata `meta.ext.{field}`"
                    ));
                };
                if !crate::runtime::meta_conforms(value, &expected) {
                    return Err(format!(
                        "`{connector}` returned invalid metadata `meta.ext.{field}`"
                    ));
                }
            }
        }
        Ok(())
    }
}

/// One envelope that left the graph through its boundary — WITH the
/// port it left by: two named outputs are two different facts, and
/// the business API must not flatten them into an anonymous stream.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundaryOutput {
    /// The boundary port the envelope left through — `out` or a
    /// declared output.
    pub port: node::Port,
    /// The envelope.
    pub envelope: Envelope,
}

/// One observed delivery — the debugger-shaped observation surface:
/// every envelope reaching a node port, the boundary, or the root
/// error sink is logged here in arrival order. Expectations read
/// windows of this log; nothing consumes it.
#[derive(Debug, Clone, PartialEq)]
pub struct Observed {
    /// The receiving node, or `None` at the App boundary (the root
    /// error sink logs as the boundary `err`).
    pub node: Option<node::Path>,
    /// The receiving port.
    pub port: node::Port,
    /// The delivered envelope.
    pub envelope: Envelope,
}

/// One won closure and the trigger its completion transaction fires
/// under: `p:in` for a driver answer, `l:timeout` for an expired
/// budget.
#[derive(Debug, Clone, PartialEq)]
struct SettledEntry {
    event: BoundaryEvent,
    trigger: node::Trigger,
}

/// The run-to-completion scheduler: it owns the mailboxes and picks
/// the next work deterministically — the same journal always replays
/// to the same order.
#[derive(Debug)]
pub struct Scheduler {
    mailboxes: Mailboxes,
    /// The machine's one dated-event queue, ordered by due date; the
    /// arm sequence breaks ties deterministically. Typed on the
    /// boundary vocabulary so every future obligation (join windows,
    /// lifetimes) crosses the same frontier as call budgets.
    dated: BTreeMap<(i64, u64), DatedPayload>,
    arm_seq: u64,
}

impl Scheduler {
    fn new(capacity: usize) -> Self {
        Self {
            mailboxes: Mailboxes::new(capacity),
            dated: BTreeMap::new(),
            arm_seq: 0,
        }
    }

    fn arm(&mut self, at: i64, payload: DatedPayload) {
        self.dated.insert((at, self.arm_seq), payload);
        self.arm_seq = self.arm_seq.wrapping_add(1);
    }

    /// The next dated event due at or before `target`, WITHOUT
    /// removing it — the reservation: an expiry that fails to commit
    /// stays armed and replays.
    fn peek_due(&self, target: i64) -> Option<((i64, u64), DatedPayload)> {
        self.dated
            .first_key_value()
            .filter(|((at, _), _)| *at <= target)
            .map(|(key, payload)| (*key, payload.clone()))
    }

    /// Acknowledges a resolved expiry.
    fn ack(&mut self, key: (i64, u64)) {
        self.dated.remove(&key);
    }
}

/// The pure execution engine. One instance serves one booted plan;
/// run and simulation drive it identically.
#[derive(Debug)]
pub struct Vm<'plan> {
    plan: &'plan ValidatedExecutionPlan,
    /// Plan-derived owner chains, indexed once so route selection allocates
    /// only the dynamic reference chain it must commit.
    owner_chains: Vec<Option<Vec<OwnerSlot>>>,
    /// Whether any activation can retain a partial presence set. The common
    /// no-join plan can keep FIFO dispatch on its original direct path.
    has_join_activations: bool,
    graph_version: GraphVersion,
    scheduler: Scheduler,
    state: InstanceState,
    dispatcher: Dispatcher,
    /// Requests delivered to a connector, held until their call
    /// settles: the completion transaction consumes this cursor, and the
    /// connector's mailbox stays claimed the whole time.
    pending: BTreeMap<CallRef, PendingCall>,
    /// Live keyed/singleton instances whose lifecycle is observable.
    instances: BTreeMap<InstanceRef, Instance>,
    /// Selected successor generations not yet allowed to start.
    waiting: BTreeMap<InstanceRef, WaitingInstance>,
    /// Rebuildable index of instances that have won a close request but have
    /// not yet queued `l:stop`. `InstancePhase` remains the authority; this
    /// set keeps open-only traffic off the closure scan.
    closing: BTreeSet<InstanceRef>,
    /// Journal-before-next-work lifecycle transitions. This queue outranks
    /// ordinary deliveries so `l:start` precedes the creating arrival and a
    /// detached `l:stop` precedes the following generation.
    lifecycle: VecDeque<LifecycleWork>,
    /// Won closures awaiting their completion transaction — replayable:
    /// an entry leaves this queue only when its transaction commits.
    settled: VecDeque<SettledEntry>,
    /// Non-causal observations collected by internal boundary polls between
    /// drain and settlement deadlines. The next public pump returns them.
    buffered_pump: PumpReport,
    /// The machine's virtual date, in nanoseconds; only
    /// [`Self::advance`] moves it, discretely.
    now: i64,
    outputs: Vec<BoundaryOutput>,
    errors: Vec<Envelope>,
    observed: Vec<Observed>,
}

impl<'plan> Vm<'plan> {
    /// Boots the machine over a VALIDATED plan — once. The machine
    /// dereferences the plan's tables without checking; the type is
    /// what makes that safe, so no raw plan is accepted here. This
    /// is a fresh boot; recovering obligations from an existing
    /// journal is the recovery fold's job and is not in this build.
    #[must_use]
    pub fn boot(plan: &'plan ValidatedExecutionPlan, graph_version: GraphVersion) -> Self {
        let mut owner_chains: Vec<Option<Vec<OwnerSlot>>> = Vec::with_capacity(plan.owners.len());
        for (index, owner) in plan.owners.iter().enumerate() {
            let slot = OwnerSlot::new(u32::try_from(index).unwrap_or(u32::MAX));
            let mut chain = match owner.parent {
                None => Some(Vec::new()),
                Some(parent) => owner_chains.get(parent.index()).cloned().flatten(),
            };
            if let Some(chain) = &mut chain {
                chain.push(slot);
            }
            owner_chains.push(chain);
        }
        Self {
            plan,
            owner_chains,
            has_join_activations: plan
                .handlers
                .iter()
                .any(|handler| matches!(&handler.activation, node::HandlerActivation::Join { .. })),
            graph_version,
            scheduler: Scheduler::new(1024),
            state: InstanceState::new(),
            dispatcher: Dispatcher::new(),
            pending: BTreeMap::new(),
            instances: BTreeMap::new(),
            waiting: BTreeMap::new(),
            closing: BTreeSet::new(),
            lifecycle: VecDeque::new(),
            settled: VecDeque::new(),
            buffered_pump: PumpReport::default(),
            now: 0,
            outputs: Vec::new(),
            errors: Vec::new(),
            observed: Vec::new(),
        }
    }

    /// Accepts one submission — the journal's one accepting
    /// transaction — and delivers the born envelope along the
    /// the submission's boundary-input edges. Everything is verified
    /// before the journal hears: a refused injection commits
    /// nothing, and backpressure reaches the ingress instead of
    /// stranding a committed birth.
    ///
    /// # Errors
    ///
    /// [`VmError::UnknownIngress`] when the port is not a graph
    /// input; [`VmError::IngressPinViolation`] when a materialized
    /// draft violates the ingress pin or a destination's; journal
    /// refusals and full target mailboxes. Every refusal leaves the
    /// journal untouched by this call.
    pub fn inject(
        &mut self,
        journal: &mut dyn Journal,
        submission: InboundSubmission,
    ) -> Result<Id, VmError> {
        self.inject_submission(journal, submission, None, None)
            .map(|accepted| accepted.envelope)
    }

    /// Attempts one retained Driver proposal against bounded admission.
    /// `Accepted` is returned only after the inbound record and every route
    /// committed. `Pending` and `Rejected` leave the journal untouched.
    ///
    /// # Errors
    ///
    /// Storage failures and machine-integrity faults remain fail-stop errors;
    /// portable peer/data refusals are represented by [`InboundAdmission`].
    pub fn admit_inbound(
        &mut self,
        journal: &mut dyn Journal,
        proposal: &InboundProposal,
    ) -> Result<InboundAdmission, VmError> {
        let connector_path = proposal.submission.provenance.connector_path().clone();
        match self.inject_submission(
            journal,
            proposal.submission.clone(),
            proposal.session,
            Some(connector_path),
        ) {
            Ok(accepted) => Ok(InboundAdmission::Accepted(accepted)),
            Err(VmError::MailboxFull { .. } | VmError::InstanceClosing { .. }) => {
                Ok(InboundAdmission::Pending)
            }
            Err(VmError::UnknownIngress(_) | VmError::UnknownInboundSource { .. }) => {
                Ok(InboundAdmission::Rejected(InboundRejection::UnknownPort))
            }
            Err(VmError::IngressPinViolation { .. }) => {
                Ok(InboundAdmission::Rejected(InboundRejection::InvalidPayload))
            }
            Err(VmError::IngressMetadataViolation { .. }) => Ok(InboundAdmission::Rejected(
                InboundRejection::InvalidMetadata,
            )),
            Err(VmError::Journal(crate::journal::Error::UnknownSession)) => Ok(
                InboundAdmission::Rejected(InboundRejection::SessionUnavailable),
            ),
            Err(error) => Err(error),
        }
    }

    fn inject_submission(
        &mut self,
        journal: &mut dyn Journal,
        submission: InboundSubmission,
        session: Option<InboundSession>,
        connector_path: Option<node::Path>,
    ) -> Result<InboundAcceptance, VmError> {
        // Boundary injection and Connector-origin roots are distinct causal
        // sources. The caller chooses the category; the validated plan proves
        // the exact endpoint before the journal reserves a sequence.
        let from = if let Some(connector) = connector_path.as_ref() {
            let source = self
                .plan
                .nodes
                .iter()
                .enumerate()
                .find(|(_, planned)| &planned.path == connector)
                .and_then(|(index, planned)| {
                    let spec = planned.connector()?;
                    let output = planned.ports.iter().any(|decl| {
                        decl.port == submission.port && decl.direction == node::Direction::Output
                    });
                    (spec.roles.has_inbound() && output).then_some(index)
                });
            let Some(source) = source else {
                return Err(VmError::UnknownInboundSource {
                    connector: connector.clone(),
                    port: submission.port.clone(),
                });
            };
            EndpointRef {
                node: Some(Slot::new(u32::try_from(source).map_err(|_| {
                    VmError::UnknownInboundSource {
                        connector: connector.clone(),
                        port: submission.port.clone(),
                    }
                })?)),
                port: submission.port.clone(),
            }
        } else {
            let ingress_ok = self.plan.boundary.iter().any(|decl| {
                decl.port == submission.port && decl.direction == node::Direction::Input
            });
            if !ingress_ok {
                return Err(VmError::UnknownIngress(submission.port.clone()));
            }
            EndpointRef {
                node: None,
                port: submission.port.clone(),
            }
        };
        // The ext zone, held to the travelling schema BEFORE any
        // append (D-2026-08-10-1): an external caller cannot seed a
        // field no sheet declares — the same grid the checker
        // enforces on sources, applied to the API.
        for (field, value) in &submission.draft.ext {
            // Session correlation is derived by the accepting journal fact.
            // Peer spelling is ignored and overwritten, never trusted as a
            // causal session identity.
            if session.is_some() && field == "session_id" {
                continue;
            }
            // Name AND type, one shared rule (D-2026-08-10-1): the
            // standard field is implicitly `text`, sheet fields
            // answer to their declared types.
            let ty = crate::plan::standard_ext_field_type(field).or_else(|| {
                self.plan
                    .plan()
                    .ext_schema
                    .iter()
                    .find(|(known, _)| known == field)
                    .map(|(_, ty)| ty.clone())
            });
            let Some(ty) = ty else {
                return Err(VmError::IngressMetadataViolation {
                    field: field.clone(),
                    reason: String::from("the field is outside the declared schema"),
                });
            };
            if !crate::runtime::meta_conforms(value, &ty) {
                return Err(VmError::IngressMetadataViolation {
                    field: field.clone(),
                    reason: alloc::format!("the field is declared {ty:?}; this value is not"),
                });
            }
        }
        // The pins, held BEFORE any append: a materialized draft is
        // judged against the ingress port's own pin and every
        // destination pin it will fan out to — replay never meets a
        // record a pin refuses. Encoded bytes stay the codecs' door.
        if let Data::Materialized(value) = &submission.draft.data {
            value
                .validate()
                .map_err(|error| VmError::Fault(crate::runtime::Fault::InvalidValue(error)))?;
            crate::runtime::crossing_pins_hold(self.plan, &from, value).map_err(|refused| {
                VmError::IngressPinViolation {
                    ingress: submission.port.clone(),
                    rejected_at: refused.at,
                    violation: refused.violation,
                }
            })?;
        }
        // The journal mints the REAL root identity without consuming the
        // sequence. Key expressions may read every immutable sys field, so a
        // zero-id provisional envelope would be a semantic lie. Destination
        // admission and the accepting record commit only after this complete
        // envelope has selected every route.
        let inbound = match (connector_path, session) {
            (Some(_), session) => journal.prepare_connector_inbound(submission, session),
            (None, Some(session)) => journal.prepare_session_inbound(submission, session),
            (None, None) => journal.prepare_inbound(submission),
        }
        .map_err(VmError::Journal)?;
        let mut candidates = BTreeMap::new();
        let prepared =
            self.prepare_routes(journal, &from, None, inbound.envelope(), &mut candidates)?;
        self.check_room_for_routes(core::iter::once(prepared.as_slice()))?;
        let recorded_routes = self.inbound_route_targets(&prepared);
        let committed = journal
            .commit_inbound(inbound, recorded_routes)
            .map_err(VmError::Journal)?;
        let id = committed.envelope.meta().sys().id();
        let session = committed
            .record
            .session
            .as_deref()
            .copied()
            .map(|session| match session {
                crate::journal::SessionIngressRecord::Open(reference)
                | crate::journal::SessionIngressRecord::Message(reference) => reference,
            });
        self.route_prepared(prepared, &candidates, &committed.envelope, None);
        Ok(InboundAcceptance {
            envelope: id,
            session,
        })
    }

    /// Executes exactly one node transaction, if any work is ready:
    /// settled closures first, then the first unclaimed mailbox.
    /// Returns whether a transaction ran.
    ///
    /// # Errors
    ///
    /// A fault leaves the machine exactly as it was — the failed step
    /// committed nothing, and its work stays reserved for a retry.
    pub fn step(&mut self, journal: &mut dyn Journal) -> Result<bool, VmError> {
        self.step_with_bindings(journal, &mut NoOutboundBindings)
    }

    /// Executes one transaction with the verified outbound bindings available
    /// for Connector request preparation.
    ///
    /// # Errors
    ///
    /// The same transaction or binding-integrity failures as [`Self::step`].
    pub fn step_with_bindings<B: OutboundConnectorBindings + ?Sized>(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut B,
    ) -> Result<bool, VmError> {
        self.progress_closures();
        if !self.lifecycle.is_empty() {
            self.fire_lifecycle_next(journal)?;
            return Ok(true);
        }
        if let Some(index) = self.settled.iter().position(|entry| {
            self.pending
                .get(&entry.event.call_ref)
                .is_some_and(|pending| self.instances.contains_key(&pending.instance))
        }) {
            self.settle_next(journal, index)?;
            return Ok(true);
        }
        // Preserve the direct FIFO dispatcher for the overwhelmingly common
        // plan with no joins. R6's wider scan exists solely because a partial
        // join head may let later independent work run; paying for it on
        // every ordinary relay was a measured Cortex-M33 regression.
        if !self.has_join_activations {
            let instances = &self.instances;
            let Some((slot, instance, delivery)) =
                self.scheduler.mailboxes.peek_next_where(|_, reference| {
                    matches!(
                        instances.get(reference),
                        Some(Instance {
                            phase: InstancePhase::Open,
                            ..
                        }) | Some(Instance {
                            phase: InstancePhase::Closing {
                                stage: ClosingStage::Draining,
                                ..
                            },
                            ..
                        })
                    )
                })
            else {
                return Ok(false);
            };
            match &self.plan.nodes[slot.index()].kind {
                NodeKind::Operator => self.fire_operator(journal, slot, &instance, &delivery)?,
                NodeKind::Connector(_) => {
                    self.open_call(journal, bindings, slot, &instance, &delivery)?
                }
            }
            return Ok(true);
        }
        self.step_join_mailboxes(journal, bindings)
    }

    // Keep join reservation state out of the ordinary relay's stack frame.
    // This boundary is measured on Cortex-M33: inlining the wider candidate
    // scan made a plan with no joins miss the established relay budget even
    // though its branch was never taken.
    #[inline(never)]
    fn step_join_mailboxes<B: OutboundConnectorBindings + ?Sized>(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut B,
    ) -> Result<bool, VmError> {
        let mut after = None;
        while let Some((slot, instance, delivery)) =
            self.scheduler
                .mailboxes
                .next_runnable_key(after.as_ref(), |_, reference| {
                    matches!(
                        self.instances.get(reference),
                        Some(Instance {
                            phase: InstancePhase::Open,
                            ..
                        }) | Some(Instance {
                            phase: InstancePhase::Closing {
                                stage: ClosingStage::Draining,
                                ..
                            },
                            ..
                        })
                    )
                })
        {
            after = Some((slot, instance.clone()));
            match &self.plan.nodes[slot.index()].kind {
                NodeKind::Connector(_) => {
                    self.open_call(journal, bindings, slot, &instance, &delivery)?;
                    return Ok(true);
                }
                NodeKind::Operator => {
                    // An ordinary FIFO head necessarily precedes any work a
                    // later delivery could make runnable. Only a join-owned
                    // head can be partial and require the wider candidate
                    // scan, so the common single-port path stays allocation
                    // free.
                    let join_owned = self.plan.handlers.iter().any(|handler| {
                        handler.node == slot
                            && matches!(
                                &handler.activation,
                                node::HandlerActivation::Join { ports, .. }
                                    if ports.contains(&delivery.port)
                            )
                    });
                    let work = if join_owned {
                        let Some(work) = self.operator_work(slot, &instance)? else {
                            continue;
                        };
                        work
                    } else {
                        OperatorWork::Single(delivery)
                    };
                    match work {
                        OperatorWork::Single(delivery) => {
                            self.fire_operator(journal, slot, &instance, &delivery)?
                        }
                        OperatorWork::Join {
                            activation,
                            trigger,
                            inputs,
                        } => self.fire_join_operator(
                            journal,
                            slot,
                            &instance,
                            &activation,
                            &trigger,
                            &inputs,
                        )?,
                        OperatorWork::Evict(delivery) => {
                            self.evict_join_delivery(journal, slot, &instance, &delivery)?
                        }
                    }
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Runs every ready transaction: settled closures and unclaimed
    /// mailboxes. Open calls are not waited for — quiescence here is
    /// not stability; see [`Self::is_stable`].
    ///
    /// # Errors
    ///
    /// The first faulting step; everything before it committed, and
    /// the faulting work stays reserved.
    pub fn drain(&mut self, journal: &mut dyn Journal) -> Result<(), VmError> {
        self.drain_with_bindings(journal, &mut NoOutboundBindings)
    }

    /// Drains ready work with the verified outbound bindings available.
    ///
    /// # Errors
    ///
    /// The first transaction or binding-integrity failure.
    pub fn drain_with_bindings<B: OutboundConnectorBindings + ?Sized>(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut B,
    ) -> Result<(), VmError> {
        while self.step_with_bindings(journal, bindings)? {}
        Ok(())
    }

    /// One boundary round: the dispatcher starts journaled calls and
    /// settles answers. Won closures queue as pending outcome
    /// transactions — [`Self::step`] commits them — and every
    /// non-causal observation is returned, never dropped.
    ///
    /// # Errors
    ///
    /// Dispatcher integrity faults propagate.
    pub fn pump<B: OutboundConnectorBindings + ?Sized>(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut B,
    ) -> Result<PumpReport, VmError> {
        let mut report = core::mem::take(&mut self.buffered_pump);
        report.append(self.poll_outbound_driver(journal, bindings)?);
        Ok(report)
    }

    fn poll_outbound_driver<B: OutboundConnectorBindings + ?Sized>(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut B,
    ) -> Result<PumpReport, VmError> {
        let gate = PlanGate { plan: self.plan };
        let batch = self
            .dispatcher
            .pump(journal, bindings, &gate)
            .map_err(VmError::Dispatch)?;
        let settled = batch.events.len();
        let in_port =
            node::Port::new("in").unwrap_or_else(|_| unreachable!("`in` is a valid port"));
        self.settled
            .extend(batch.events.into_iter().map(|event| SettledEntry {
                event,
                trigger: node::Trigger::Port(in_port.clone()),
            }));
        Ok(PumpReport {
            settled,
            discarded: batch.discarded,
            uncorrelated: batch.uncorrelated,
            rejected: batch.rejected,
        })
    }

    /// Advances the virtual date to `target`, firing each armed
    /// budget in expiry order — the discrete advance: time moves in
    /// jumps, and the machine drains between them. Only the machine
    /// mints a timeout — a driver answers or stays silent, never
    /// times itself out. A budget runs from the DELIVERY, so an
    /// expiry resolves wherever the obligation lives: a call still
    /// open closes with a Timeout fact (and the driver is told to
    /// cancel the in-flight work); a delivery still queued follows
    /// the call-closed rule — its call opens and closes at the
    /// expiry, back to back; an obligation already settled loses the
    /// race and fires nothing.
    ///
    /// # Errors
    ///
    /// Journal refusals and faulting steps, as in [`Self::drain`].
    pub fn advance<B: OutboundConnectorBindings + ?Sized>(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut B,
        target: i64,
    ) -> Result<(), VmError> {
        // Ready work at the current logical instant precedes any future
        // expiry. In particular, a just-created instance commits `l:start`
        // before its first idle deadline can close it.
        self.drain_with_bindings(journal, bindings)?;
        while let Some((key, payload)) = self.scheduler.peek_due(target) {
            self.now = key.0;
            let mut polled_at_settlement = false;
            if matches!(
                &payload,
                DatedPayload::InstanceClosing {
                    deadline: ClosingDeadline::Settlement,
                    ..
                }
            ) {
                let report = self.poll_outbound_driver(journal, bindings)?;
                self.buffered_pump.append(report);
                self.drain_with_bindings(journal, bindings)?;
                polled_at_settlement = true;
            }
            let mut saw_drain = matches!(
                &payload,
                DatedPayload::InstanceClosing {
                    deadline: ClosingDeadline::Drain,
                    ..
                }
            );
            // Every expiry OF THIS INSTANT fires before the machine
            // drains: the drain happens between jumps of the clock,
            // never between simultaneous events — a same-instant
            // budget must not see the world advanced by its sibling.
            // A failed expiry stays armed: reserve, resolve, ack.
            self.expire(journal, bindings, payload)?;
            self.scheduler.ack(key);
            while let Some((sibling_key, sibling)) = self.scheduler.peek_due(key.0) {
                if !polled_at_settlement
                    && matches!(
                        &sibling,
                        DatedPayload::InstanceClosing {
                            deadline: ClosingDeadline::Settlement,
                            ..
                        }
                    )
                {
                    let report = self.poll_outbound_driver(journal, bindings)?;
                    self.buffered_pump.append(report);
                    self.drain_with_bindings(journal, bindings)?;
                    polled_at_settlement = true;
                }
                saw_drain |= matches!(
                    &sibling,
                    DatedPayload::InstanceClosing {
                        deadline: ClosingDeadline::Drain,
                        ..
                    }
                );
                self.expire(journal, bindings, sibling)?;
                self.scheduler.ack(sibling_key);
            }
            if saw_drain {
                let report = self.poll_outbound_driver(journal, bindings)?;
                self.buffered_pump.append(report);
            }
            self.drain_with_bindings(journal, bindings)?;
        }
        self.now = self.now.max(target);
        Ok(())
    }

    /// Resolves one expired budget wherever the obligation lives.
    fn expire<B: OutboundConnectorBindings + ?Sized>(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut B,
        payload: DatedPayload,
    ) -> Result<(), VmError> {
        let (connector, edge, delivery, instance) = match payload {
            DatedPayload::DeliveryBudget {
                connector,
                edge,
                delivery,
                instance,
            } => (connector, edge, delivery, instance),
            DatedPayload::JoinWithin {
                operator,
                edge,
                delivery,
                instance,
            } => {
                let slot = self.slot_of(&operator);
                let buffered = self
                    .scheduler
                    .mailboxes
                    .find_by_ref(slot, &instance, &edge, delivery)
                    .cloned();
                if let Some(buffered) = buffered {
                    self.evict_join_delivery(journal, slot, &instance, &buffered)?;
                }
                return Ok(());
            }
            DatedPayload::InstanceLifetime {
                owner,
                instance,
                cause,
                deadline,
            } => {
                self.expire_instance(journal, owner, instance, cause, deadline)?;
                return Ok(());
            }
            DatedPayload::InstanceClosing {
                instance,
                cause,
                deadline,
            } => {
                self.expire_closing(journal, bindings, &instance, cause, deadline)?;
                return Ok(());
            }
        };
        // An open call for THIS obligation — the delivered edge and
        // envelope both: a fan-out of one envelope arms one budget
        // per receiving edge, and each expires its own call.
        if let Some(call_ref) = self
            .pending
            .iter()
            .find(|(_, held)| {
                held.instance == instance
                    && held.delivery.edge == edge
                    && held.delivery.envelope.meta().sys().id() == delivery
            })
            .map(|(call_ref, _)| *call_ref)
        {
            // Contract projection is pure, but it can still expose a broken
            // binding. Prove it before the close-once mutation so an
            // integrity refusal cannot strand a journaled obligation.
            let completion = bindings
                .scheduler_completion(&connector, SchedulerCompletion::TimeoutExpired)
                .map_err(crate::boundary::DispatchError::Driver)
                .map_err(VmError::Dispatch)?;
            // Cancellation is a fallible Driver boundary. Prove it before
            // close-once so a refusal leaves both the call and its deadline
            // replayable; no terminal fact can strand a claimed mailbox.
            bindings
                .cancel(&connector, &call_ref)
                .map_err(crate::boundary::DispatchError::Driver)
                .map_err(VmError::Dispatch)?;
            match journal
                .close_outbound(&call_ref, crate::journal::Closure::Timeout)
                .map_err(VmError::Journal)?
            {
                crate::journal::CloseVerdict::AlreadyClosed => return Ok(()),
                crate::journal::CloseVerdict::Won => {}
            }
            let intent = journal
                .find_call(&call_ref)
                .unwrap_or_else(|| unreachable!("a closed call is journaled"));
            self.settled.push_back(SettledEntry {
                event: BoundaryEvent {
                    call_ref,
                    connector_path: intent.connector_path,
                    emitting_id: intent.emitting_id,
                    completion,
                },
                trigger: node::Trigger::Lifecycle(node::Lifecycle::Timeout),
            });
            return Ok(());
        }
        // Still queued: the queue time consumed the whole budget.
        // The call-closed rule applies even to a call that expires
        // before it started (execution-model §8): the call OPENS and
        // CLOSES with a Timeout fact, and the completion transaction
        // fires under `l:timeout` — the same causality as any expiry.
        // Journal facts come first; the mailbox is touched only after
        // they hold, so a refusal loses nothing.
        let slot = self.slot_of(&connector);
        let queued_intent = {
            let Some(queued) = self
                .scheduler
                .mailboxes
                .find_by_ref(slot, &instance, &edge, delivery)
            else {
                // Already settled — the budget lost the race.
                return Ok(());
            };
            let planned = &self.plan.nodes[slot.index()];
            let NodeKind::Connector(spec) = &planned.kind else {
                unreachable!("budgets arm connectors only")
            };
            // Validation refuses an effectless callee
            // (`EffectlessCallee`); this is the typed belt behind
            // that door, never a panic.
            let node::Effect::Class(effect) = spec.effect else {
                return Err(VmError::PlanInvariant {
                    node: planned.path.clone(),
                    reason: "a delivery opened a call on a connector without an effect class",
                });
            };
            let resolved_settings =
                self.resolve_invocation_settings(&instance, spec, &planned.path, &queued.envelope)?;
            match bindings
                .prepare_request(&planned.path, queued.envelope.data(), &resolved_settings)
                .map_err(crate::boundary::DispatchError::Driver)
                .map_err(VmError::Dispatch)?
            {
                RequestPreparation::Rejected(completion) => Err(completion),
                RequestPreparation::Prepared(prepared) => {
                    let emitting_id = queued.envelope.meta().sys().id();
                    Ok(Intent {
                        call_ref: ids::call_ref(
                            ids::delivery_ref(&queued.edge, emitting_id),
                            prepared.digest(),
                        ),
                        emitting_id,
                        edge: queued.edge.clone(),
                        connector_path: planned.path.clone(),
                        prepared,
                        idempotency_key: None,
                        effect,
                        resolved_settings,
                    })
                }
            }
        };
        match queued_intent {
            Err(completion) => {
                // Unframeable AND expired: the decode refusal wins —
                // the call could never have been attempted. Facts
                // first: the delivery leaves the queue only after
                // the commit holds, so a refusal replays with
                // nothing lost.
                let mut outcome = {
                    let Some(queued) = self
                        .scheduler
                        .mailboxes
                        .find_by_ref(slot, &instance, &edge, delivery)
                    else {
                        return Ok(());
                    };
                    let executor = TransactionExecutor::new(self.plan, self.graph_version);
                    let trigger = node::Trigger::Port(queued.port.clone());
                    executor
                        .connector_completion(slot, &instance, queued, &trigger, &completion)
                        .map_err(VmError::Fault)?
                };
                let prepared =
                    self.validate_and_prepare_emissions(journal, slot, &instance, &outcome.record)?;
                outcome.record.routes = self.routed_deliveries(&prepared);
                journal
                    .commit(outcome.record.clone())
                    .map_err(VmError::Journal)?;
                let Some(_) = self
                    .scheduler
                    .mailboxes
                    .remove_by_ref(slot, &instance, &edge, delivery)
                else {
                    return Err(VmError::PlanInvariant {
                        node: self.plan.nodes[slot.index()].path.clone(),
                        reason: "a reserved timeout delivery disappeared after commit",
                    });
                };
                self.route_emissions(slot, &instance, prepared);
                self.progress_closures();
            }
            Ok(intent) => {
                // As above, establish the contract-owned representation
                // before the open/close facts or mailbox state can move.
                let completion = bindings
                    .scheduler_completion(&connector, SchedulerCompletion::TimeoutExpired)
                    .map_err(crate::boundary::DispatchError::Driver)
                    .map_err(VmError::Dispatch)?;
                let call_ref = intent.call_ref;
                let connector_path = intent.connector_path.clone();
                let emitting_id = intent.emitting_id;
                // Idempotent open: a replay after a refused close
                // finds its own open fact and proceeds to close it.
                if journal.find_call(&call_ref).is_none() {
                    journal.open_outbound(intent).map_err(VmError::Journal)?;
                }
                match journal
                    .close_outbound(&call_ref, crate::journal::Closure::Timeout)
                    .map_err(VmError::Journal)?
                {
                    crate::journal::CloseVerdict::Won => {}
                    // A replay can find its own earlier closure.
                    crate::journal::CloseVerdict::AlreadyClosed => {}
                }
                // The facts hold; only now does the delivery leave
                // the queue and join the settled path.
                let Some(queued) = self
                    .scheduler
                    .mailboxes
                    .remove_by_ref(slot, &instance, &edge, delivery)
                else {
                    return Err(VmError::PlanInvariant {
                        node: self.plan.nodes[slot.index()].path.clone(),
                        reason: "a reserved timeout delivery disappeared before settlement",
                    });
                };
                self.pending.insert(
                    call_ref,
                    PendingCall {
                        slot,
                        delivery: queued,
                        instance,
                    },
                );
                self.settled.push_back(SettledEntry {
                    event: BoundaryEvent {
                        call_ref,
                        connector_path,
                        emitting_id,
                        completion,
                    },
                    trigger: node::Trigger::Lifecycle(node::Lifecycle::Timeout),
                });
            }
        }
        Ok(())
    }

    /// Resolves one generation-fenced lifetime deadline. Only idle deadlines
    /// are superseded by later admission; fixed and tumbling stay anchored to
    /// their creating start/box. Missing references are stale observations.
    fn expire_instance(
        &mut self,
        journal: &mut dyn Journal,
        owner: OwnerSlot,
        reference: InstanceRef,
        cause: Id,
        deadline: InstanceDeadline,
    ) -> Result<(), VmError> {
        let current = self.instances.get(&reference).filter(|instance| {
            instance.owner == owner
                && matches!(instance.phase, InstancePhase::Open)
                && match deadline {
                    InstanceDeadline::Idle => instance.last_activity.meta().sys().id() == cause,
                    InstanceDeadline::Fixed | InstanceDeadline::Tumbling => {
                        instance.created_by.meta().sys().id() == cause
                    }
                }
        });
        let Some(cause) = current.map(|instance| match deadline {
            InstanceDeadline::Idle => instance.last_activity.clone(),
            InstanceDeadline::Fixed | InstanceDeadline::Tumbling => instance.created_by.clone(),
        }) else {
            return Ok(());
        };
        self.request_close(journal, &reference, cause)?;
        self.progress_closures();
        Ok(())
    }

    /// The machine's virtual date, in nanoseconds.
    #[must_use]
    pub fn now(&self) -> i64 {
        self.now
    }

    /// The envelopes that left the graph through its boundary, each
    /// with the port it left by.
    #[must_use]
    pub fn outputs(&self) -> &[BoundaryOutput] {
        &self.outputs
    }

    /// Legacy envelope-only view of the conventional `err` root sink.
    ///
    /// This view contains explicit graph emissions only; it never classifies
    /// or converts [`VmError::Fault`]. `exit` is an independent ordinary port
    /// and remains visible, with its name, through [`Self::observations`].
    #[must_use]
    pub fn errors(&self) -> &[Envelope] {
        &self.errors
    }

    /// Every delivery the machine made, in arrival order — the
    /// observation log expectations read windows of.
    #[must_use]
    pub fn observations(&self) -> &[Observed] {
        &self.observed
    }

    /// Whether the machine is stable: no activation can run, no completion
    /// waits to commit, and no call is open. A partial join is retained
    /// mailbox state, not runnable work; it is therefore quiescent until a
    /// matching arrival or its dated/count eviction makes progress possible.
    #[must_use]
    pub fn is_stable(&self) -> bool {
        if !self.lifecycle.is_empty() || !self.settled.is_empty() || !self.pending.is_empty() {
            return false;
        }
        if !self.has_join_activations {
            return self.scheduler.mailboxes.is_empty();
        }
        let mut after = None;
        while let Some((slot, instance, delivery)) =
            self.scheduler
                .mailboxes
                .next_runnable_key(after.as_ref(), |_, reference| {
                    matches!(
                        self.instances.get(reference),
                        Some(Instance {
                            phase: InstancePhase::Open,
                            ..
                        }) | Some(Instance {
                            phase: InstancePhase::Closing {
                                stage: ClosingStage::Draining,
                                ..
                            },
                            ..
                        })
                    )
                })
        {
            if !matches!(self.plan.nodes[slot.index()].kind, NodeKind::Operator) {
                return false;
            }
            let join_owned = self.plan.handlers.iter().any(|handler| {
                handler.node == slot
                    && matches!(
                        &handler.activation,
                        node::HandlerActivation::Join { ports, .. }
                            if ports.contains(&delivery.port)
                    )
            });
            if !join_owned || !matches!(self.operator_work(slot, &instance), Ok(None)) {
                return false;
            }
            after = Some((slot, instance));
        }
        true
    }

    /// Whether calls are open against the world — work [`Self::drain`]
    /// cannot finish alone.
    #[must_use]
    pub fn has_open_obligations(&self) -> bool {
        !self.pending.is_empty()
    }

    fn instance_has_direct_work(&self, reference: &InstanceRef) -> bool {
        let Some(instance) = self.instances.get(reference) else {
            return false;
        };
        let node_work = match self.plan.owners[instance.owner.index()].kind {
            OwnerKind::Graph => false,
            OwnerKind::Node(slot) => {
                self.scheduler.mailboxes.has_work(slot, reference)
                    || self
                        .pending
                        .values()
                        .any(|pending| pending.slot == slot && pending.instance == *reference)
            }
        };
        node_work
            || self
                .lifecycle
                .iter()
                .any(|work| work.instance == *reference)
    }

    fn is_descendant_instance(&self, child: &InstanceRef, ancestor: &InstanceRef) -> bool {
        let mut cursor = self
            .instances
            .get(child)
            .and_then(|instance| instance.parent.clone());
        while let Some(reference) = cursor {
            if reference == *ancestor {
                return true;
            }
            cursor = self
                .instances
                .get(&reference)
                .and_then(|instance| instance.parent.clone());
        }
        false
    }

    fn is_within_instance(&self, candidate: &InstanceRef, scope: &InstanceRef) -> bool {
        candidate == scope || self.is_descendant_instance(candidate, scope)
    }

    fn instance_has_descendants(&self, reference: &InstanceRef) -> bool {
        self.instances.keys().any(|candidate| {
            candidate != reference && self.is_descendant_instance(candidate, reference)
        })
    }

    fn closing_timers(&self, reference: &InstanceRef) -> Vec<(u8, Id)> {
        let Some(instance) = self.instances.get(reference) else {
            return Vec::new();
        };
        let Some(_) = self.plan.owners[instance.owner.index()].instance.closing else {
            return Vec::new();
        };
        vec![
            (
                crate::journal::timer_kind::INSTANCE_DRAIN,
                ids::instance_closing_ref(reference, crate::journal::timer_kind::INSTANCE_DRAIN),
            ),
            (
                crate::journal::timer_kind::INSTANCE_SETTLEMENT,
                ids::instance_closing_ref(
                    reference,
                    crate::journal::timer_kind::INSTANCE_SETTLEMENT,
                ),
            ),
        ]
    }

    fn arm_closing_deadlines(&mut self, reference: &InstanceRef, cause: Id) {
        let Some(instance) = self.instances.get(reference) else {
            return;
        };
        let Some(policy) = self.plan.owners[instance.owner.index()].instance.closing else {
            return;
        };
        let drain_at = self.now.saturating_add(policy.drain_for);
        self.scheduler.arm(
            drain_at,
            DatedPayload::InstanceClosing {
                instance: reference.clone(),
                cause,
                deadline: ClosingDeadline::Drain,
            },
        );
        self.scheduler.arm(
            drain_at.saturating_add(policy.settle_for),
            DatedPayload::InstanceClosing {
                instance: reference.clone(),
                cause,
                deadline: ClosingDeadline::Settlement,
            },
        );
    }

    /// Commits the close request before changing admission. Expiry and
    /// single-pass use this path; an explicit `stop` carries the same
    /// `close_scope` in its already-committed handler transaction.
    fn request_close(
        &mut self,
        journal: &mut dyn Journal,
        reference: &InstanceRef,
        cause: Envelope,
    ) -> Result<(), VmError> {
        if !self
            .instances
            .get(reference)
            .is_some_and(|instance| matches!(instance.phase, InstancePhase::Open))
        {
            return Ok(());
        }
        let owner = self.instances[reference].owner;
        let record = crate::journal::TransactionRecord {
            owner_path: self.plan.owners[owner.index()].path.clone(),
            instance: reference.clone(),
            parent_instance: None,
            close_scope: Some(reference.clone()),
            session_exit: None,
            trigger: node::Trigger::Lifecycle(node::Lifecycle::Closing),
            cause: Some(cause.meta().sys().id()),
            consumed: vec![],
            join_inputs: vec![],
            state_delta: vec![],
            emissions: vec![],
            routes: vec![],
            timers: self.closing_timers(reference),
        };
        journal.commit(record).map_err(VmError::Journal)?;
        self.activate_close(reference, cause.clone());
        self.arm_closing_deadlines(reference, cause.meta().sys().id());
        Ok(())
    }

    /// The first close request wins causally. A Graph request marks every live
    /// descendant, but reclamation remains inner-first: the parent cannot
    /// queue `l:stop` while a child bundle or child stop fact is outstanding.
    fn activate_close(&mut self, reference: &InstanceRef, cause: Envelope) {
        let closes_subtree = self.instances.get(reference).is_some_and(|instance| {
            self.plan.owners[instance.owner.index()].kind == OwnerKind::Graph
        });
        let targets: Vec<InstanceRef> = self
            .instances
            .keys()
            .filter(|candidate| {
                *candidate == reference
                    || (closes_subtree && self.is_descendant_instance(candidate, reference))
            })
            .cloned()
            .collect();
        for target in targets {
            let widen = self
                .instances
                .get(&target)
                .and_then(|instance| match &instance.phase {
                    InstancePhase::Closing { scope, .. }
                        if self.is_within_instance(scope, reference) =>
                    {
                        Some(true)
                    }
                    _ => None,
                });
            if let Some(instance) = self.instances.get_mut(&target) {
                match &mut instance.phase {
                    InstancePhase::Open => {
                        instance.phase = InstancePhase::Closing {
                            cause: cause.clone(),
                            scope: reference.clone(),
                            stage: ClosingStage::Draining,
                        };
                        self.closing.insert(target);
                    }
                    InstancePhase::Closing { scope, .. } if widen == Some(true) => {
                        *scope = reference.clone();
                    }
                    InstancePhase::Closing { .. } | InstancePhase::Stopping { .. } => {}
                }
            }
        }
    }

    fn progress_closures(&mut self) {
        if self.closing.is_empty() {
            return;
        }
        let mut ready: Vec<(usize, InstanceRef)> = self
            .closing
            .iter()
            .filter_map(|reference| {
                self.instances
                    .get(reference)
                    .map(|instance| (reference, instance))
            })
            .filter(|(reference, _)| {
                let scope = match &self.instances[*reference].phase {
                    InstancePhase::Closing { scope, .. } => scope,
                    _ => return false,
                };
                !self.scope_has_work(scope)
                    && !self.instance_has_direct_work(reference)
                    && !self.instance_has_descendants(reference)
            })
            .map(|(reference, instance)| {
                (
                    self.plan.owners[instance.owner.index()]
                        .path
                        .segments()
                        .len(),
                    reference.clone(),
                )
            })
            .collect();
        ready.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
        if let Some((_, reference)) = ready.into_iter().next() {
            self.begin_stop(&reference);
        }
    }

    fn scope_has_work(&self, scope: &InstanceRef) -> bool {
        self.instances.keys().any(|reference| {
            self.is_within_instance(reference, scope) && self.instance_has_direct_work(reference)
        })
    }

    fn residualize_scope(&mut self, scope: &InstanceRef) {
        let targets: Vec<(Slot, InstanceRef)> = self
            .instances
            .iter()
            .filter(|(reference, _)| self.is_within_instance(reference, scope))
            .filter_map(|(reference, instance)| {
                match self.plan.owners[instance.owner.index()].kind {
                    OwnerKind::Graph => None,
                    OwnerKind::Node(slot) => Some((slot, reference.clone())),
                }
            })
            .collect();
        for (slot, reference) in targets {
            let residual = self.scheduler.mailboxes.take_all(slot, &reference);
            if let Some(instance) = self.instances.get_mut(&reference) {
                instance.residual.extend(residual);
            }
        }
    }

    fn expire_closing<B: OutboundConnectorBindings + ?Sized>(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut B,
        reference: &InstanceRef,
        cause: Id,
        deadline: ClosingDeadline,
    ) -> Result<(), VmError> {
        let current = self
            .instances
            .get(reference)
            .and_then(|instance| match &instance.phase {
                InstancePhase::Closing {
                    cause: current_cause,
                    scope,
                    stage,
                } if scope == reference && current_cause.meta().sys().id() == cause => Some(*stage),
                InstancePhase::Open
                | InstancePhase::Closing { .. }
                | InstancePhase::Stopping { .. } => None,
            });
        let Some(stage) = current else {
            return Ok(());
        };
        match deadline {
            ClosingDeadline::Drain if stage == ClosingStage::Draining => {
                self.residualize_scope(reference);
                let targets: Vec<InstanceRef> = self
                    .instances
                    .keys()
                    .filter(|candidate| self.is_within_instance(candidate, reference))
                    .cloned()
                    .collect();
                for target in targets {
                    if let Some(instance) = self.instances.get_mut(&target)
                        && let InstancePhase::Closing { stage, .. } = &mut instance.phase
                    {
                        *stage = ClosingStage::Settling;
                    }
                }
                let calls: Vec<CallRef> = self
                    .pending
                    .iter()
                    .filter(|(_, pending)| self.is_within_instance(&pending.instance, reference))
                    .map(|(call_ref, _)| *call_ref)
                    .collect();
                for call_ref in calls {
                    let pending = self
                        .pending
                        .get(&call_ref)
                        .ok_or(VmError::MissingPending(call_ref))?;
                    let connector = &self.plan.nodes[pending.slot.index()].path;
                    bindings
                        .cancel(connector, &call_ref)
                        .map_err(crate::boundary::DispatchError::Driver)
                        .map_err(VmError::Dispatch)?;
                }
            }
            ClosingDeadline::Settlement => {
                self.residualize_scope(reference);
                let calls: Vec<CallRef> = self
                    .pending
                    .iter()
                    .filter(|(_, pending)| self.is_within_instance(&pending.instance, reference))
                    .map(|(call_ref, _)| *call_ref)
                    .collect();
                for call_ref in calls {
                    let pending = self
                        .pending
                        .get(&call_ref)
                        .ok_or(VmError::MissingPending(call_ref))?;
                    let connector = self.plan.nodes[pending.slot.index()].path.clone();
                    let completion = bindings
                        .scheduler_completion(&connector, SchedulerCompletion::OutcomeUnknown)
                        .map_err(crate::boundary::DispatchError::Driver)
                        .map_err(VmError::Dispatch)?;
                    match journal
                        .abandon_outbound(&call_ref)
                        .map_err(VmError::Journal)?
                    {
                        crate::journal::CloseVerdict::AlreadyClosed => {}
                        crate::journal::CloseVerdict::Won => {
                            self.settled.push_back(SettledEntry {
                                event: BoundaryEvent {
                                    call_ref,
                                    connector_path: connector.clone(),
                                    emitting_id: pending.delivery.envelope.meta().sys().id(),
                                    completion,
                                },
                                trigger: node::Trigger::Lifecycle(node::Lifecycle::Recover),
                            });
                        }
                    }
                }
            }
            ClosingDeadline::Drain => {}
        }
        self.progress_closures();
        Ok(())
    }

    fn begin_stop(&mut self, reference: &InstanceRef) {
        let Some(bundle) = self.instances.get_mut(reference) else {
            return;
        };
        let InstancePhase::Closing { cause, scope, .. } = &bundle.phase else {
            return;
        };
        let cause = cause.clone();
        let scope = scope.clone();
        self.closing.remove(reference);
        bundle.phase = InstancePhase::Stopping {
            cause: cause.clone(),
            scope,
        };
        self.lifecycle.push_back(LifecycleWork {
            owner: bundle.owner,
            instance: reference.clone(),
            parent_instance: bundle.parent.clone(),
            lifecycle: node::Lifecycle::Stop,
            cause,
        });
    }

    /// Commits exactly one queued lifecycle transition. The instance remains
    /// installed through `l:stop`, so its handler can route from the exact
    /// source chain; state and bundle are reclaimed only after the commit.
    fn fire_lifecycle_next(&mut self, journal: &mut dyn Journal) -> Result<(), VmError> {
        let Some(work) = self.lifecycle.front().cloned() else {
            return Ok(());
        };
        let parent_instance = (work.lifecycle == node::Lifecycle::Start)
            .then_some(work.parent_instance.as_ref())
            .flatten();
        let owner = &self.plan.owners[work.owner.index()];
        let (mut record, prepared, node_slot) = match owner.kind {
            OwnerKind::Graph => (
                crate::journal::TransactionRecord {
                    owner_path: owner.path.clone(),
                    instance: work.instance.clone(),
                    parent_instance: parent_instance.cloned(),
                    close_scope: None,
                    session_exit: None,
                    trigger: node::Trigger::Lifecycle(work.lifecycle),
                    cause: Some(work.cause.meta().sys().id()),
                    consumed: vec![],
                    join_inputs: vec![],
                    state_delta: vec![],
                    emissions: vec![],
                    routes: vec![],
                    timers: vec![],
                },
                None,
                None,
            ),
            OwnerKind::Node(slot) => {
                let bundle =
                    self.instances
                        .get(&work.instance)
                        .ok_or_else(|| VmError::PlanInvariant {
                            node: self.plan.nodes[slot.index()].path.clone(),
                            reason: "lifecycle work lost its installed instance bundle",
                        })?;
                let state = self
                    .state
                    .cell_unchecked(self.plan.plan(), slot, &work.instance);
                let executor = TransactionExecutor::new(self.plan, self.graph_version);
                let outcome = executor
                    .fire_lifecycle(
                        slot,
                        work.lifecycle,
                        LifecycleFiring {
                            instance: &work.instance,
                            parent_instance,
                            cause: &work.cause,
                        },
                        &state,
                        &bundle.values,
                    )
                    .map_err(VmError::Fault)?;
                let prepared = self.validate_and_prepare_emissions(
                    journal,
                    slot,
                    &work.instance,
                    &outcome.record,
                )?;
                (outcome.record, Some(prepared), Some(slot))
            }
        };
        if let Some(prepared) = &prepared {
            record.routes = self.routed_deliveries(prepared);
        }
        if work.lifecycle == node::Lifecycle::Stop
            && let Some(bundle) = self.instances.get(&work.instance)
        {
            record
                .consumed
                .extend(
                    bundle
                        .residual
                        .iter()
                        .map(|delivery| crate::journal::ConsumedPosition {
                            edge: delivery.edge.clone(),
                            source: delivery.source_instance.clone(),
                            seq: delivery.seq,
                        }),
                );
        }
        if work.lifecycle == node::Lifecycle::Start
            && matches!(
                owner.instance.lifetime,
                InstanceLifetime::Idle(_)
                    | InstanceLifetime::Fixed(_)
                    | InstanceLifetime::Tumbling(_)
            )
        {
            record.timers.push((
                crate::journal::timer_kind::INSTANCE_LIFETIME,
                ids::instance_lifetime_ref(&work.instance),
            ));
        }
        journal.commit(record.clone()).map_err(VmError::Journal)?;
        self.lifecycle.pop_front();
        if work.lifecycle == node::Lifecycle::Start {
            if let Some(slot) = node_slot {
                self.state.apply_unchecked(
                    self.plan.plan(),
                    slot,
                    &work.instance,
                    &record.state_delta,
                );
            }
            self.arm_start_deadline(&work);
            if matches!(owner.instance.lifetime, InstanceLifetime::SinglePass) {
                self.request_close(journal, &work.instance, work.cause.clone())?;
            }
        }
        if let (Some(slot), Some(prepared)) = (node_slot, prepared) {
            self.route_emissions(slot, &work.instance, prepared);
        }
        if work.lifecycle == node::Lifecycle::Stop {
            if let Some(slot) = node_slot {
                self.state
                    .take_unchecked(self.plan.plan(), slot, &work.instance);
            }
            self.instances.remove(&work.instance);
            self.promote_waiting(journal)?;
        }
        self.progress_closures();
        Ok(())
    }

    fn promote_waiting(&mut self, journal: &dyn Journal) -> Result<(), VmError> {
        loop {
            let mut ready = Vec::new();
            for (reference, waiting) in &self.waiting {
                let predecessor_is_live = self
                    .instances
                    .keys()
                    .any(|live| live.address() == reference.address());
                let parent_is_ready = waiting
                    .candidate
                    .parent
                    .as_ref()
                    .is_none_or(|parent| self.instances.contains_key(parent));
                if predecessor_is_live || !parent_is_ready {
                    continue;
                }
                let expected = journal
                    .instance_ref_for_admission(reference.address())
                    .map_err(VmError::Journal)?;
                if expected == *reference {
                    ready.push((
                        self.plan.owners[waiting.candidate.owner.index()]
                            .path
                            .segments()
                            .len(),
                        reference.clone(),
                    ));
                }
            }
            ready.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
            let Some((_, reference)) = ready.into_iter().next() else {
                return Ok(());
            };
            let Some(waiting) = self.waiting.remove(&reference) else {
                return Ok(());
            };
            let owner = waiting.candidate.owner;
            self.instances.insert(
                reference.clone(),
                Instance {
                    owner,
                    parent: waiting.candidate.parent.clone(),
                    frames: waiting.candidate.frames,
                    values: waiting.candidate.values,
                    settings: waiting.candidate.settings,
                    created_by: waiting.created_by.clone(),
                    last_activity: waiting.last_activity.clone(),
                    residual: Vec::new(),
                    phase: InstancePhase::Open,
                },
            );
            self.lifecycle.push_back(LifecycleWork {
                owner,
                instance: reference.clone(),
                parent_instance: waiting.candidate.parent,
                lifecycle: node::Lifecycle::Start,
                cause: waiting.created_by,
            });
            if let InstanceLifetime::Idle(ttl) = self.plan.owners[owner.index()].instance.lifetime {
                self.scheduler.arm(
                    waiting.last_activity_at.saturating_add(ttl),
                    DatedPayload::InstanceLifetime {
                        owner,
                        instance: reference,
                        cause: waiting.last_activity.meta().sys().id(),
                        deadline: InstanceDeadline::Idle,
                    },
                );
            }
        }
    }

    fn arm_start_deadline(&mut self, work: &LifecycleWork) {
        let owner = &self.plan.owners[work.owner.index()];
        let (at, deadline) = match owner.instance.lifetime {
            InstanceLifetime::Fixed(after) => {
                (self.now.saturating_add(after), InstanceDeadline::Fixed)
            }
            InstanceLifetime::Tumbling(every) => {
                let Some(start) = self.instances.get(&work.instance).and_then(|instance| {
                    instance.frames.iter().rev().find_map(|frame| match frame {
                        InstanceFrame::ProcessingTimeBox {
                            owner: framed,
                            start_ns,
                        } if framed == &owner.path => Some(*start_ns),
                        _ => None,
                    })
                }) else {
                    return;
                };
                (start.saturating_add(every), InstanceDeadline::Tumbling)
            }
            InstanceLifetime::Persistent
            | InstanceLifetime::Idle(_)
            | InstanceLifetime::SinglePass => return,
        };
        self.scheduler.arm(
            at,
            DatedPayload::InstanceLifetime {
                owner: work.owner,
                instance: work.instance.clone(),
                cause: work.cause.meta().sys().id(),
                deadline,
            },
        );
    }

    /// One won closure becomes the connector's completion transaction.
    /// The event leaves the queue only after its commit; every
    /// integrity check happens before.
    fn settle_next(&mut self, journal: &mut dyn Journal, index: usize) -> Result<(), VmError> {
        let SettledEntry { event, trigger } = self
            .settled
            .get(index)
            .cloned()
            .unwrap_or_else(|| unreachable!("guarded by the caller"));
        let slot = self.slot_of(&event.connector_path);
        let Some(request) = self.pending.get(&event.call_ref).cloned() else {
            return Err(VmError::MissingPending(event.call_ref));
        };
        let executor = TransactionExecutor::new(self.plan, self.graph_version);
        // The dispatcher gate checked every materialized draft before the
        // closure became causal. This is the typed belt for machine-minted
        // completions (timeouts) and for any future in-memory caller.
        for output in event.completion.outputs() {
            let Data::Materialized(value) = &output.data else {
                continue;
            };
            let from = EndpointRef {
                node: Some(slot),
                port: output.port.clone(),
            };
            crate::runtime::crossing_pins_hold(self.plan, &from, value)
                .map_err(crate::runtime::PinViolation::into_fault)
                .map_err(VmError::Fault)?;
        }
        let mut outcome = executor
            .connector_completion(
                slot,
                &request.instance,
                &request.delivery,
                &trigger,
                &event.completion,
            )
            .map_err(VmError::Fault)?;
        let prepared =
            self.validate_and_prepare_emissions(journal, slot, &request.instance, &outcome.record)?;
        outcome.record.routes = self.routed_deliveries(&prepared);
        journal
            .commit(outcome.record.clone())
            .map_err(VmError::Journal)?;
        self.settled.remove(index);
        self.pending.remove(&event.call_ref);
        let instance_still_open = self
            .pending
            .values()
            .any(|pending| pending.slot == slot && pending.instance == request.instance);
        if !instance_still_open {
            self.scheduler.mailboxes.release(slot, &request.instance);
        }
        self.route_emissions(slot, &request.instance, prepared);
        self.progress_closures();
        Ok(())
    }

    /// Selects the earliest runnable fact in one Operator mailbox. Partial
    /// join arrivals are logically buffered and therefore do not block a
    /// later ready activation; ordering is by the arrival that made the work
    /// runnable, never by map or handler declaration order.
    fn operator_work(
        &self,
        slot: Slot,
        instance: &InstanceRef,
    ) -> Result<Option<OperatorWork>, VmError> {
        let Some(queue) = self.scheduler.mailboxes.queued(slot, instance) else {
            return Ok(None);
        };
        let handlers: Vec<_> = self
            .plan
            .handlers
            .iter()
            .filter(|handler| handler.node == slot)
            .collect();
        let join_owns = |port: &node::Port| {
            handlers.iter().any(|handler| {
                matches!(
                    &handler.activation,
                    node::HandlerActivation::Join { ports, .. } if ports.contains(port)
                )
            })
        };
        let mut best: Option<(usize, u8, OperatorWork)> = None;

        for (index, delivery) in queue.iter().enumerate() {
            let has_single = handlers.iter().any(|handler| {
                matches!(
                    &handler.activation,
                    node::HandlerActivation::Port(port) if port == &delivery.port
                )
            });
            if has_single || !join_owns(&delivery.port) {
                consider_operator_work(&mut best, index, 2, OperatorWork::Single(delivery.clone()));
                break;
            }
        }

        let executor = TransactionExecutor::new(self.plan, self.graph_version);
        let state = self.state.cell_unchecked(self.plan.plan(), slot, instance);
        let values = self
            .instances
            .get(instance)
            .map(|bundle| &bundle.values)
            .ok_or_else(|| VmError::PlanInvariant {
                node: self.plan.nodes[slot.index()].path.clone(),
                reason: "a mailbox references an instance without a bundle",
            })?;

        for handler in handlers {
            let node::HandlerActivation::Join { ports, last, .. } = &handler.activation else {
                continue;
            };

            if let Some(limit) = last {
                for port in ports {
                    let candidates: Vec<_> = queue
                        .iter()
                        .enumerate()
                        .filter(|(_, delivery)| &delivery.port == port)
                        .collect();
                    if candidates.len() > *limit as usize {
                        let overflow_at = candidates[*limit as usize].0;
                        consider_operator_work(
                            &mut best,
                            overflow_at,
                            0,
                            OperatorWork::Evict(candidates[0].1.clone()),
                        );
                    }
                }
            }

            for (trigger_index, trigger) in queue.iter().enumerate() {
                let Some(trigger_port) = ports.iter().position(|port| port == &trigger.port) else {
                    continue;
                };
                let mut options: Vec<Vec<Delivery>> = Vec::with_capacity(ports.len());
                for (port_index, port) in ports.iter().enumerate() {
                    let candidates = if port_index == trigger_port {
                        vec![trigger.clone()]
                    } else {
                        queue
                            .iter()
                            .take(trigger_index)
                            .filter(|delivery| &delivery.port == port)
                            .cloned()
                            .collect()
                    };
                    if candidates.is_empty() {
                        options.clear();
                        break;
                    }
                    options.push(candidates);
                }
                if options.len() != ports.len() {
                    continue;
                }

                // Declared port order is the tuple order. This mixed-radix
                // walk visits FIFO choices lexicographically and stops at the
                // first predicate match for this presence-completing arrival.
                let mut choice = vec![0usize; options.len()];
                loop {
                    let inputs: Vec<Delivery> = options
                        .iter()
                        .zip(&choice)
                        .map(|(candidates, index)| candidates[*index].clone())
                        .collect();
                    if executor
                        .join_matches(
                            slot,
                            JoinCandidate {
                                activation: &handler.activation,
                                trigger,
                                inputs: &inputs,
                            },
                            &state,
                            values,
                        )
                        .map_err(VmError::Fault)?
                    {
                        consider_operator_work(
                            &mut best,
                            trigger_index,
                            1,
                            OperatorWork::Join {
                                activation: handler.activation.clone(),
                                trigger: trigger.clone(),
                                inputs,
                            },
                        );
                        break;
                    }
                    let mut digit = choice.len();
                    while digit > 0 {
                        digit -= 1;
                        choice[digit] += 1;
                        if choice[digit] < options[digit].len() {
                            break;
                        }
                        choice[digit] = 0;
                    }
                    if digit == 0 && choice[0] == 0 {
                        break;
                    }
                }
                if best
                    .as_ref()
                    .is_some_and(|(at, rank, _)| (*at, *rank) <= (trigger_index, 1))
                {
                    break;
                }
            }
        }
        Ok(best.map(|(_, _, work)| work))
    }

    fn fire_operator(
        &mut self,
        journal: &mut dyn Journal,
        slot: Slot,
        instance: &InstanceRef,
        delivery: &Delivery,
    ) -> Result<(), VmError> {
        let executor = TransactionExecutor::new(self.plan, self.graph_version);
        let state = self.state.cell_unchecked(self.plan.plan(), slot, instance);
        let values = self
            .instances
            .get(instance)
            .map(|bundle| &bundle.values)
            .ok_or_else(|| VmError::PlanInvariant {
                node: self.plan.nodes[slot.index()].path.clone(),
                reason: "a mailbox references an instance without a bundle",
            })?;
        // An Operator's declared `err` is an ordinary domain-result port,
        // close to the E in `I -> Result<R, E>`. Engine/evaluation faults are
        // outside that contract: the transaction stays uncommitted and the
        // VM reports the fault to supervision.
        let mut outcome = executor
            .fire_with_bindings(slot, instance, delivery, &state, values)
            .map_err(VmError::Fault)?;
        let close_request = outcome.record.close_scope.clone();
        if let Some(scope) = &close_request {
            outcome.record.timers.extend(self.closing_timers(scope));
        }
        let prepared =
            self.validate_and_prepare_emissions(journal, slot, instance, &outcome.record)?;
        outcome.record.routes = self.routed_deliveries(&prepared);
        // The journal owns the full transaction after commit. Retain only the
        // small delta the in-memory state fold still needs; cloning the whole
        // record would duplicate every emitted envelope on the relay hot path.
        let state_delta = outcome.record.state_delta.clone();
        journal.commit(outcome.record).map_err(VmError::Journal)?;
        self.scheduler.mailboxes.pop(slot, instance);
        self.state
            .apply_unchecked(self.plan.plan(), slot, instance, &state_delta);
        if let Some(scope) = close_request {
            self.activate_close(&scope, delivery.envelope.clone());
            self.arm_closing_deadlines(&scope, delivery.envelope.meta().sys().id());
        }
        self.route_emissions(slot, instance, prepared);
        self.progress_closures();
        Ok(())
    }

    fn fire_join_operator(
        &mut self,
        journal: &mut dyn Journal,
        slot: Slot,
        instance: &InstanceRef,
        activation: &node::HandlerActivation,
        trigger: &Delivery,
        inputs: &[Delivery],
    ) -> Result<(), VmError> {
        let executor = TransactionExecutor::new(self.plan, self.graph_version);
        let state = self.state.cell_unchecked(self.plan.plan(), slot, instance);
        let values = self
            .instances
            .get(instance)
            .map(|bundle| &bundle.values)
            .ok_or_else(|| VmError::PlanInvariant {
                node: self.plan.nodes[slot.index()].path.clone(),
                reason: "a join mailbox references an instance without a bundle",
            })?;
        let mut outcome = executor
            .fire_join(
                slot,
                instance,
                JoinCandidate {
                    activation,
                    trigger,
                    inputs,
                },
                &state,
                values,
            )
            .map_err(VmError::Fault)?;
        let close_request = outcome.record.close_scope.clone();
        if let Some(scope) = &close_request {
            outcome.record.timers.extend(self.closing_timers(scope));
        }
        let prepared =
            self.validate_and_prepare_emissions(journal, slot, instance, &outcome.record)?;
        outcome.record.routes = self.routed_deliveries(&prepared);
        journal
            .commit(outcome.record.clone())
            .map_err(VmError::Journal)?;
        for delivery in inputs {
            if self
                .scheduler
                .mailboxes
                .remove_by_ref(
                    slot,
                    instance,
                    &delivery.edge,
                    delivery.envelope.meta().sys().id(),
                )
                .is_none()
            {
                return Err(VmError::PlanInvariant {
                    node: self.plan.nodes[slot.index()].path.clone(),
                    reason: "a committed join input disappeared before cursor removal",
                });
            }
        }
        self.state.apply_unchecked(
            self.plan.plan(),
            slot,
            instance,
            &outcome.record.state_delta,
        );
        if let Some(scope) = close_request {
            self.activate_close(&scope, trigger.envelope.clone());
            self.arm_closing_deadlines(&scope, trigger.envelope.meta().sys().id());
        }
        self.route_emissions(slot, instance, prepared);
        self.progress_closures();
        Ok(())
    }

    fn evict_join_delivery(
        &mut self,
        journal: &mut dyn Journal,
        slot: Slot,
        instance: &InstanceRef,
        delivery: &Delivery,
    ) -> Result<(), VmError> {
        let executor = TransactionExecutor::new(self.plan, self.graph_version);
        let mut outcome = executor
            .join_expired(slot, instance, delivery)
            .map_err(VmError::Fault)?;
        let prepared =
            self.validate_and_prepare_emissions(journal, slot, instance, &outcome.record)?;
        outcome.record.routes = self.routed_deliveries(&prepared);
        journal
            .commit(outcome.record.clone())
            .map_err(VmError::Journal)?;
        if self
            .scheduler
            .mailboxes
            .remove_by_ref(
                slot,
                instance,
                &delivery.edge,
                delivery.envelope.meta().sys().id(),
            )
            .is_none()
        {
            return Err(VmError::PlanInvariant {
                node: self.plan.nodes[slot.index()].path.clone(),
                reason: "a committed join eviction lost its buffered delivery",
            });
        }
        self.route_emissions(slot, instance, prepared);
        self.progress_closures();
        Ok(())
    }

    /// A delivery reaching a connector opens its outbound call:
    /// journal-before-effect — the open fact exists before any
    /// driver hears — and the request is held until the call
    /// settles, its mailbox claimed for the whole wait: the worker is
    /// free, the node is not.
    fn open_call<B: OutboundConnectorBindings + ?Sized>(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut B,
        slot: Slot,
        instance: &InstanceRef,
        delivery: &Delivery,
    ) -> Result<(), VmError> {
        let planned = &self.plan.nodes[slot.index()];
        let NodeKind::Connector(spec) = &planned.kind else {
            unreachable!("routed here by kind");
        };
        // The typed belt behind the validator's `EffectlessCallee`
        // door — never a panic.
        let node::Effect::Class(effect) = spec.effect else {
            return Err(VmError::PlanInvariant {
                node: planned.path.clone(),
                reason: "a delivery opened a call on a connector without an effect class",
            });
        };
        let emitting_id = delivery.envelope.meta().sys().id();
        // The machine prepares ONCE, in the boundary vocabulary: the
        // sealed digest and the bytes the driver will receive are
        // one value carried by the intent — nobody recomputes. An
        // unframeable request completes on `exit` before any open
        // fact exists: stderr is not a lifecycle channel.
        let resolved_settings =
            self.resolve_invocation_settings(instance, spec, &planned.path, &delivery.envelope)?;
        let prepared = match bindings
            .prepare_request(&planned.path, delivery.envelope.data(), &resolved_settings)
            .map_err(crate::boundary::DispatchError::Driver)
            .map_err(VmError::Dispatch)?
        {
            RequestPreparation::Prepared(prepared) => prepared,
            RequestPreparation::Rejected(completion) => {
                let executor = TransactionExecutor::new(self.plan, self.graph_version);
                let trigger = node::Trigger::Port(delivery.port.clone());
                let mut outcome = executor
                    .connector_completion(slot, instance, delivery, &trigger, &completion)
                    .map_err(VmError::Fault)?;
                let prepared =
                    self.validate_and_prepare_emissions(journal, slot, instance, &outcome.record)?;
                outcome.record.routes = self.routed_deliveries(&prepared);
                journal
                    .commit(outcome.record.clone())
                    .map_err(VmError::Journal)?;
                self.scheduler.mailboxes.pop(slot, instance);
                self.route_emissions(slot, instance, prepared);
                self.progress_closures();
                return Ok(());
            }
        };
        // The call identity: one delivery carrying one prepared
        // request (envelope.md §2) — the reference says by where,
        // the digest says what.
        let intent = Intent {
            call_ref: ids::call_ref(
                ids::delivery_ref(&delivery.edge, emitting_id),
                prepared.digest(),
            ),
            emitting_id,
            edge: delivery.edge.clone(),
            connector_path: planned.path.clone(),
            prepared,
            // The checker admits only `idempotent` in this subset; a
            // keyed irreversible intent arrives with its invariants.
            idempotency_key: None,
            effect,
            resolved_settings,
        };
        let call_ref = intent.call_ref;
        journal.open_outbound(intent).map_err(VmError::Journal)?;
        let delivery = self
            .scheduler
            .mailboxes
            .pop(slot, instance)
            .unwrap_or_else(|| unreachable!("the reserved head is still there"));
        self.pending.insert(
            call_ref,
            PendingCall {
                slot,
                delivery,
                instance: instance.clone(),
            },
        );
        self.scheduler.mailboxes.claim(slot, instance);
        Ok(())
    }

    fn slot_of(&self, path: &node::Path) -> Slot {
        Slot::new(
            u32::try_from(
                self.plan
                    .nodes
                    .iter()
                    .position(|node| &node.path == path)
                    .unwrap_or_else(|| unreachable!("planned paths resolve")),
            )
            .unwrap_or_else(|_| unreachable!("node count fits u32")),
        )
    }

    /// Validates every emission, selects each target instance once, and
    /// reserves room before the commit. A refusal is pure backpressure or a
    /// pure evaluation fault; the returned packet is infallible post-commit
    /// work.
    ///
    /// Pin conformance runs at TWO levels, deliberately: construction checks
    /// ordinary Operator emissions, while [`CompletionGate`] refuses an
    /// invalid driver draft before closure. THIS final, non-recursive level
    /// catches a violated plan/runtime invariant; it cannot manufacture
    /// another emission on the same bad wiring, so NOTHING commits.
    fn validate_and_prepare_emissions(
        &self,
        journal: &dyn Journal,
        slot: Slot,
        source_instance: &InstanceRef,
        record: &crate::runtime::TransactionRecord,
    ) -> Result<PreparedEmissions, VmError> {
        let mut prepared = Vec::with_capacity(record.emissions.len());
        // One producer transaction is one admission linearization point. If
        // several emissions select the same not-yet-live instance, the first
        // canonical route binds its immutable bundle and every sibling route
        // reuses it; later payloads cannot rebind the same generation.
        let mut candidates = BTreeMap::new();
        for emission in &record.emissions {
            if let Data::Materialized(value) = emission.envelope.data() {
                value
                    .validate()
                    .map_err(|error| VmError::Fault(crate::runtime::Fault::InvalidValue(error)))?;
                let from = EndpointRef {
                    node: Some(slot),
                    port: emission.port.clone(),
                };
                crate::runtime::crossing_pins_hold(self.plan, &from, value)
                    .map_err(|violation| VmError::Fault(violation.into_fault()))?;
            }
            let from = EndpointRef {
                node: Some(slot),
                port: emission.port.clone(),
            };
            prepared.push(PreparedEmission {
                emission: emission.clone(),
                routes: self.prepare_routes(
                    journal,
                    &from,
                    Some(source_instance),
                    &emission.envelope,
                    &mut candidates,
                )?,
            });
        }
        self.check_room_for_routes(prepared.iter().map(|emission| emission.routes.as_slice()))?;
        Ok(PreparedEmissions {
            emissions: prepared,
            candidates,
        })
    }

    /// Selects complete target references for every edge leaving one
    /// endpoint. The target chain is constructed outermost-to-innermost and
    /// remains a pure candidate until the producer record commits.
    fn prepare_routes(
        &self,
        journal: &dyn Journal,
        from: &EndpointRef,
        source_instance: Option<&InstanceRef>,
        envelope: &Envelope,
        candidates: &mut BTreeMap<InstanceRef, InstanceCandidate>,
    ) -> Result<Vec<PreparedEdge>, VmError> {
        let mut prepared = Vec::new();
        for edge in self
            .plan
            .edges
            .iter()
            .filter(|edge| edge.from == *from)
            .cloned()
        {
            let target = match edge.to.as_ref().and_then(|to| to.node) {
                Some(target) => Some(self.prepare_target(
                    journal,
                    edge.scope,
                    source_instance,
                    target,
                    envelope,
                    candidates,
                )?),
                None => None,
            };
            prepared.push(PreparedEdge { edge, target });
        }
        Ok(prepared)
    }

    /// Every route decision sealed by an inbound record. The envelope id is
    /// implicit because an inbound record carries exactly one envelope.
    fn inbound_route_targets(&self, prepared: &[PreparedEdge]) -> Vec<RouteTarget> {
        prepared
            .iter()
            .filter_map(|route| {
                route.target.as_ref().map(|target| RouteTarget {
                    edge: self.edge_text(&route.edge),
                    target: target.reference.clone(),
                })
            })
            .collect()
    }

    /// Every route decision sealed atomically with an emitting transaction.
    /// Repeated emissions on one edge remain distinct by id.
    fn routed_deliveries(&self, prepared: &PreparedEmissions) -> Vec<RoutedDelivery> {
        prepared
            .emissions
            .iter()
            .flat_map(|emission| {
                emission.routes.iter().filter_map(|route| {
                    route.target.as_ref().map(|target| RoutedDelivery {
                        delivery: emission.emission.envelope.meta().sys().id(),
                        edge: self.edge_text(&route.edge),
                        target: target.reference.clone(),
                    })
                })
            })
            .collect()
    }

    fn prepare_target(
        &self,
        journal: &dyn Journal,
        scope: OwnerSlot,
        source_instance: Option<&InstanceRef>,
        target: Slot,
        envelope: &Envelope,
        candidates: &mut BTreeMap<InstanceRef, InstanceCandidate>,
    ) -> Result<PreparedTarget, VmError> {
        let target_owner = self.plan.nodes[target.index()].owner;
        let chain = self.owner_chain(target_owner)?;
        let Some(scope_index) = chain.iter().position(|owner| *owner == scope) else {
            return Err(VmError::PlanInvariant {
                node: self.plan.nodes[target.index()].path.clone(),
                reason: "the edge scope is not an ancestor of its target owner",
            });
        };
        let mut selected = Vec::with_capacity(chain.len());
        let mut admitted = Vec::with_capacity(chain.len().saturating_sub(scope_index));
        for (index, owner_slot) in chain.iter().copied().enumerate() {
            if index <= scope_index
                && let Some(reference) =
                    self.source_instance_for_owner(source_instance, owner_slot)?
            {
                selected.push(reference);
            } else {
                let parent = selected.last();
                let reference = self.candidate_owner(
                    journal,
                    owner_slot,
                    parent,
                    source_instance,
                    envelope,
                    candidates,
                )?;
                admitted.push(reference.clone());
                selected.push(reference);
            }
        }
        let reference = selected
            .last()
            .cloned()
            .ok_or_else(|| VmError::PlanInvariant {
                node: self.plan.nodes[target.index()].path.clone(),
                reason: "an executable node has no owner chain",
            })?;
        let scope = selected
            .get(scope_index)
            .cloned()
            .ok_or_else(|| VmError::PlanInvariant {
                node: self.plan.nodes[target.index()].path.clone(),
                reason: "the selected owner chain lost its edge scope",
            })?;
        Ok(PreparedTarget {
            reference,
            scope,
            chain: selected,
            admitted,
        })
    }

    fn owner_chain(&self, owner: OwnerSlot) -> Result<&[OwnerSlot], VmError> {
        self.owner_chains
            .get(owner.index())
            .and_then(Option::as_deref)
            .ok_or_else(|| VmError::PlanInvariant {
                node: node::Path::new(["owner"])
                    .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                reason: "an owner reference escaped the validated owner table",
            })
    }

    fn source_instance_for_owner(
        &self,
        source: Option<&InstanceRef>,
        owner: OwnerSlot,
    ) -> Result<Option<InstanceRef>, VmError> {
        let mut cursor = source.cloned();
        while let Some(reference) = cursor {
            let Some(instance) = self.instances.get(&reference) else {
                return Err(VmError::PlanInvariant {
                    node: node::Path::new(["instance"])
                        .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                    reason: "a routed source instance has no installed bundle",
                });
            };
            if instance.owner == owner {
                return Ok(Some(reference));
            }
            cursor = instance.parent.clone();
        }
        Ok(None)
    }

    fn closing_generation_for_internal_route(
        &self,
        address: &InstanceAddress,
        source: Option<&InstanceRef>,
        lifetime: &InstanceLifetime,
    ) -> Option<InstanceRef> {
        if self.closing.is_empty() || matches!(lifetime, InstanceLifetime::SinglePass) {
            return None;
        }
        let first = InstanceRef::new(address.clone(), 0);
        let last = InstanceRef::new(address.clone(), u64::MAX);
        let reference = self.closing.range(first..=last).next_back()?;
        let instance = self.instances.get(reference)?;
        let InstancePhase::Closing { scope, .. } = &instance.phase else {
            return None;
        };
        source
            .is_some_and(|source| self.is_within_instance(source, scope))
            .then(|| reference.clone())
    }

    fn candidate_owner(
        &self,
        journal: &dyn Journal,
        owner_slot: OwnerSlot,
        parent: Option<&InstanceRef>,
        source: Option<&InstanceRef>,
        envelope: &Envelope,
        candidates: &mut BTreeMap<InstanceRef, InstanceCandidate>,
    ) -> Result<InstanceRef, VmError> {
        let owner = self
            .plan
            .owner(owner_slot)
            .ok_or_else(|| VmError::PlanInvariant {
                node: node::Path::new(["owner"])
                    .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                reason: "an owner reference escaped the validated owner table",
            })?;
        // An unkeyed non-tumbling owner keeps one address within the selected
        // parent generation. This is only a measured lookup fast path: the
        // reference still carries the canonical address and journal-derived
        // generation used everywhere else. Tumbling is excluded because
        // virtual time selects a distinct address; a missing live entry still
        // falls through to the journal-derived successor generation for
        // idle/fixed/single-pass.
        if owner.instance.key.is_none()
            && !matches!(owner.instance.lifetime, InstanceLifetime::Tumbling(_))
        {
            if let Some((reference, instance)) = self.instances.iter().find(|(_, instance)| {
                instance.owner == owner_slot && instance.parent.as_ref() == parent
            }) {
                let accepts = match &instance.phase {
                    InstancePhase::Open => {
                        !matches!(owner.instance.lifetime, InstanceLifetime::SinglePass)
                    }
                    InstancePhase::Closing { scope, .. } => {
                        source.is_some_and(|source| self.is_within_instance(source, scope))
                            && !matches!(owner.instance.lifetime, InstanceLifetime::SinglePass)
                    }
                    InstancePhase::Stopping { .. } => false,
                };
                if accepts {
                    return Ok(reference.clone());
                }
                // External admission after the close fact reserves the next
                // journal generation and waits in its ordinary mailbox.
            }
            if let Some(waiting) = self.waiting.values().find(|waiting| {
                waiting.candidate.owner == owner_slot && waiting.candidate.parent.as_ref() == parent
            }) {
                if matches!(owner.instance.lifetime, InstanceLifetime::SinglePass)
                    && waiting.candidate.single_pass_root != Some(envelope.meta().sys().id())
                {
                    return Err(VmError::InstanceClosing {
                        owner: owner.path.clone(),
                    });
                }
                return Ok(waiting.candidate.reference.clone());
            }
            if let Some(candidate) = candidates.values().find(|candidate| {
                candidate.owner == owner_slot && candidate.parent.as_ref() == parent
            }) {
                if matches!(owner.instance.lifetime, InstanceLifetime::SinglePass)
                    && candidate.single_pass_root != Some(envelope.meta().sys().id())
                {
                    return Err(VmError::InstanceClosing {
                        owner: owner.path.clone(),
                    });
                }
                return Ok(candidate.reference.clone());
            }
        }
        let (inherited, mut frames) = match parent {
            None => (BTreeMap::new(), Vec::new()),
            Some(reference) => {
                if let Some(instance) = self.instances.get(reference) {
                    (instance.values.clone(), instance.frames.clone())
                } else if let Some(waiting) = self.waiting.get(reference) {
                    (
                        waiting.candidate.values.clone(),
                        waiting.candidate.frames.clone(),
                    )
                } else if let Some(candidate) = candidates.get(reference) {
                    (candidate.values.clone(), candidate.frames.clone())
                } else {
                    return Err(VmError::PlanInvariant {
                        node: node::Path::new(["owner"])
                            .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                        reason: "a selected parent has no live or candidate instance bundle",
                    });
                }
            }
        };
        let selected_key = match owner.instance.key.as_ref() {
            None => None,
            Some(key) => {
                let mut bindings: BTreeMap<String, crate::expr::EvalValue> = inherited
                    .iter()
                    .map(|(name, value)| {
                        (name.clone(), crate::expr::EvalValue::from(value.clone()))
                    })
                    .collect();
                bindings.insert(
                    String::from("meta"),
                    crate::expr::EvalValue::from(meta_value(envelope.meta())),
                );
                let data = DataSlot::new(envelope.data());
                let value = resolve_payload(
                    eval_lazy(key, &bindings, &data, &self.plan.assets).map_err(VmError::Fault)?,
                    &data,
                )
                .map_err(VmError::Fault)?;
                frames.push(InstanceFrame::SelectedKey {
                    owner: owner.path.clone(),
                    ty: key.ty.clone(),
                    value: value.clone(),
                });
                Some(value)
            }
        };
        let time_box_start = match owner.instance.lifetime {
            InstanceLifetime::Tumbling(every) => {
                let start = self
                    .now
                    .div_euclid(every)
                    .checked_mul(every)
                    .ok_or_else(|| VmError::PlanInvariant {
                        node: node::Path::new(["instance"])
                            .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                        reason: "a validated tumbling duration overflowed its epoch box",
                    })?;
                frames.push(InstanceFrame::ProcessingTimeBox {
                    owner: owner.path.clone(),
                    start_ns: start,
                });
                Some(start)
            }
            _ => None,
        };
        let address = InstanceAddress::new(&owner.path, &frames)
            .map_err(|error| VmError::Fault(Fault::Unrepresentable(error.to_string())))?;
        // The journal normally selects the current generation. Only Closing
        // is different: external traffic belongs to the reserved successor,
        // while work already admitted inside the closing scope must retain
        // the old generation. The replay-derived closing index keeps this
        // exceptional lookup off ordinary and first-admission paths.
        if let Some(reference) =
            self.closing_generation_for_internal_route(&address, source, &owner.instance.lifetime)
        {
            return Ok(reference);
        }
        let reference = journal
            .instance_ref_for_admission(&address)
            .map_err(VmError::Journal)?;
        if let Some(start_ns) = time_box_start
            && reference.generation() != 0
        {
            return Err(VmError::ClosedTimeBox {
                owner: owner.path.clone(),
                start_ns,
            });
        }
        if let Some(instance) = self.instances.get(&reference) {
            let accepts = match &instance.phase {
                InstancePhase::Open => {
                    !matches!(owner.instance.lifetime, InstanceLifetime::SinglePass)
                }
                InstancePhase::Closing { scope, .. } => {
                    source.is_some_and(|source| self.is_within_instance(source, scope))
                        && !matches!(owner.instance.lifetime, InstanceLifetime::SinglePass)
                }
                InstancePhase::Stopping { .. } => false,
            };
            if accepts {
                return Ok(reference);
            }
            return Err(VmError::InstanceClosing {
                owner: owner.path.clone(),
            });
        }
        if let Some(waiting) = self.waiting.get(&reference) {
            if matches!(owner.instance.lifetime, InstanceLifetime::SinglePass)
                && waiting.candidate.single_pass_root != Some(envelope.meta().sys().id())
            {
                return Err(VmError::InstanceClosing {
                    owner: owner.path.clone(),
                });
            }
            return Ok(waiting.candidate.reference.clone());
        }
        if let Some(candidate) = candidates.get(&reference) {
            if matches!(owner.instance.lifetime, InstanceLifetime::SinglePass)
                && candidate.single_pass_root != Some(envelope.meta().sys().id())
            {
                return Err(VmError::InstanceClosing {
                    owner: owner.path.clone(),
                });
            }
            return Ok(candidate.reference.clone());
        }
        let mut values =
            self.evaluate_owner_values(owner_slot, &inherited, selected_key.as_ref(), envelope)?;
        if let Some(key) = &selected_key {
            values.insert(String::from("instance.key"), key.clone());
        }
        let settings =
            self.evaluate_instance_settings(owner_slot, &values, selected_key.as_ref(), envelope)?;
        let candidate = InstanceCandidate {
            owner: owner_slot,
            reference: reference.clone(),
            parent: parent.cloned(),
            frames,
            values,
            settings,
            single_pass_root: matches!(owner.instance.lifetime, InstanceLifetime::SinglePass)
                .then_some(envelope.meta().sys().id()),
        };
        candidates.insert(reference.clone(), candidate);
        Ok(reference)
    }

    fn evaluate_owner_values(
        &self,
        owner_slot: OwnerSlot,
        inherited: &BTreeMap<String, crate::value::Value>,
        selected_key: Option<&crate::value::Value>,
        envelope: &Envelope,
    ) -> Result<BTreeMap<String, crate::value::Value>, VmError> {
        let owner = self
            .plan
            .owner(owner_slot)
            .ok_or_else(|| VmError::PlanInvariant {
                node: node::Path::new(["owner"])
                    .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                reason: "an owner reference escaped the validated owner table",
            })?;
        let local: BTreeMap<&str, usize> = owner
            .vals
            .iter()
            .enumerate()
            .map(|(index, value)| (value.name.as_str(), index))
            .collect();
        let mut resolved: Vec<Option<crate::value::Value>> = vec![None; owner.vals.len()];
        let mut visiting = vec![false; owner.vals.len()];
        for index in 0..owner.vals.len() {
            self.evaluate_owner_value_at(
                owner_slot,
                index,
                inherited,
                selected_key,
                envelope,
                &local,
                &mut resolved,
                &mut visiting,
            )?;
        }
        let mut values = inherited.clone();
        for (declared, value) in owner.vals.iter().zip(resolved) {
            let value = value.ok_or_else(|| VmError::PlanInvariant {
                node: node::Path::new(["owner"])
                    .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                reason: "a validated immutable binding did not resolve",
            })?;
            values.insert(declared.name.clone(), value);
        }
        Ok(values)
    }

    #[allow(clippy::too_many_arguments)]
    fn evaluate_owner_value_at(
        &self,
        owner_slot: OwnerSlot,
        index: usize,
        inherited: &BTreeMap<String, crate::value::Value>,
        selected_key: Option<&crate::value::Value>,
        envelope: &Envelope,
        local: &BTreeMap<&str, usize>,
        resolved: &mut [Option<crate::value::Value>],
        visiting: &mut [bool],
    ) -> Result<(), VmError> {
        if resolved[index].is_some() {
            return Ok(());
        }
        if visiting[index] {
            return Err(VmError::PlanInvariant {
                node: node::Path::new(["owner"])
                    .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                reason: "a validated immutable-binding cycle reached evaluation",
            });
        }
        visiting[index] = true;
        let owner = self
            .plan
            .owner(owner_slot)
            .ok_or_else(|| VmError::PlanInvariant {
                node: node::Path::new(["owner"])
                    .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                reason: "an owner reference escaped the validated owner table",
            })?;
        let declared = &owner.vals[index];
        let mut roots = Vec::new();
        expression_roots(&declared.expr.root, &mut roots);
        for root in roots {
            if let Some(dependency) = local.get(root.as_str()) {
                self.evaluate_owner_value_at(
                    owner_slot,
                    *dependency,
                    inherited,
                    selected_key,
                    envelope,
                    local,
                    resolved,
                    visiting,
                )?;
            }
        }
        let value = match (&declared.site, &declared.resolved) {
            (EvaluationSite::Plan, Some(value)) => value.clone(),
            (EvaluationSite::Plan, None) | (EvaluationSite::Invocation, _) => {
                return Err(VmError::PlanInvariant {
                    node: node::Path::new(["owner"])
                        .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                    reason: "a validated immutable binding carries an impossible site",
                });
            }
            (EvaluationSite::Boot | EvaluationSite::Instance, _) => {
                let mut bindings: BTreeMap<String, crate::expr::EvalValue> = inherited
                    .iter()
                    .map(|(name, value)| {
                        (name.clone(), crate::expr::EvalValue::from(value.clone()))
                    })
                    .collect();
                for (dependency, value) in owner.vals.iter().zip(resolved.iter()) {
                    if let Some(value) = value {
                        bindings.insert(
                            dependency.name.clone(),
                            crate::expr::EvalValue::from(value.clone()),
                        );
                    }
                }
                if let Some(key) = selected_key {
                    bindings.insert(
                        String::from("instance.key"),
                        crate::expr::EvalValue::from(key.clone()),
                    );
                }
                let data = DataSlot::new(envelope.data());
                resolve_payload(
                    eval_lazy(&declared.expr, &bindings, &data, &self.plan.assets)
                        .map_err(VmError::Fault)?,
                    &data,
                )
                .map_err(VmError::Fault)?
            }
        };
        if let Some(violation) = declared.expr.ty.violation(&declared.name, &value) {
            return Err(VmError::Fault(Fault::Unrepresentable(
                violation.to_string(),
            )));
        }
        visiting[index] = false;
        resolved[index] = Some(value);
        Ok(())
    }

    fn evaluate_instance_settings(
        &self,
        owner_slot: OwnerSlot,
        values: &BTreeMap<String, crate::value::Value>,
        selected_key: Option<&crate::value::Value>,
        envelope: &Envelope,
    ) -> Result<BTreeMap<String, crate::value::Value>, VmError> {
        let Some(owner) = self.plan.owner(owner_slot) else {
            return Err(VmError::PlanInvariant {
                node: node::Path::new(["owner"])
                    .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                reason: "an owner reference escaped the validated owner table",
            });
        };
        let OwnerKind::Node(slot) = owner.kind else {
            return Ok(BTreeMap::new());
        };
        let Some(spec) = self.plan.nodes[slot.index()].connector() else {
            return Ok(BTreeMap::new());
        };
        let mut bindings: BTreeMap<String, crate::expr::EvalValue> = values
            .iter()
            .map(|(name, value)| (name.clone(), crate::expr::EvalValue::from(value.clone())))
            .collect();
        if let Some(key) = selected_key {
            bindings.insert(
                String::from("instance.key"),
                crate::expr::EvalValue::from(key.clone()),
            );
        }
        self.evaluate_connector_setting_expressions(
            spec,
            EvaluationSite::Instance,
            &bindings,
            envelope,
        )
    }

    fn resolve_invocation_settings(
        &self,
        instance: &InstanceRef,
        spec: &crate::plan::ConnectorSpec,
        node: &node::Path,
        envelope: &Envelope,
    ) -> Result<BTreeMap<String, crate::value::Value>, VmError> {
        let (values, mut settings) = self
            .instances
            .get(instance)
            .map(|bundle| (&bundle.values, bundle.settings.clone()))
            .or_else(|| {
                self.waiting.get(instance).map(|waiting| {
                    (
                        &waiting.candidate.values,
                        waiting.candidate.settings.clone(),
                    )
                })
            })
            .ok_or_else(|| VmError::PlanInvariant {
                node: node.clone(),
                reason: "an opening call lost its selected instance bundle",
            })?;
        if !spec.attributes.iter().any(|(_, setting)| {
            matches!(
                setting,
                AttrSetting::Expr {
                    site: EvaluationSite::Invocation,
                    ..
                }
            )
        }) {
            return Ok(settings);
        }
        let mut bindings: BTreeMap<String, crate::expr::EvalValue> = values
            .iter()
            .map(|(name, value)| (name.clone(), crate::expr::EvalValue::from(value.clone())))
            .collect();
        bindings.insert(
            String::from("meta"),
            crate::expr::EvalValue::from(meta_value(envelope.meta())),
        );
        settings.extend(self.evaluate_connector_setting_expressions(
            spec,
            EvaluationSite::Invocation,
            &bindings,
            envelope,
        )?);
        Ok(settings)
    }

    fn evaluate_connector_setting_expressions(
        &self,
        spec: &crate::plan::ConnectorSpec,
        target_site: EvaluationSite,
        bindings: &BTreeMap<String, crate::expr::EvalValue>,
        envelope: &Envelope,
    ) -> Result<BTreeMap<String, crate::value::Value>, VmError> {
        let data = DataSlot::new(envelope.data());
        let mut settings = BTreeMap::new();
        for (name, setting) in &spec.attributes {
            let AttrSetting::Expr { site, expr } = setting else {
                continue;
            };
            let selected = match target_site {
                EvaluationSite::Instance => *site != EvaluationSite::Invocation,
                EvaluationSite::Invocation => *site == EvaluationSite::Invocation,
                EvaluationSite::Plan | EvaluationSite::Boot => false,
            };
            if !selected {
                continue;
            }
            let value = resolve_payload(
                eval_lazy(expr, bindings, &data, &self.plan.assets).map_err(VmError::Fault)?,
                &data,
            )
            .map_err(VmError::Fault)?;
            if let Some(violation) = expr.ty.violation(name, &value) {
                return Err(VmError::Fault(Fault::Unrepresentable(
                    violation.to_string(),
                )));
            }
            settings.insert(name.clone(), value);
        }
        Ok(settings)
    }

    fn check_room_for_routes<'a>(
        &self,
        routes: impl IntoIterator<Item = &'a [PreparedEdge]>,
    ) -> Result<(), VmError>
    where
        PreparedEdge: 'a,
    {
        let mut needed: BTreeMap<(Slot, InstanceRef), usize> = BTreeMap::new();
        for routes in routes {
            for prepared in routes {
                let Some(to) = &prepared.edge.to else {
                    continue;
                };
                let Some(target) = to.node else {
                    continue;
                };
                let target_ref = prepared
                    .target
                    .as_ref()
                    .map(|target| target.reference.clone())
                    .ok_or_else(|| VmError::PlanInvariant {
                        node: self.plan.nodes[target.index()].path.clone(),
                        reason: "a node route has no selected target instance",
                    })?;
                *needed.entry((target, target_ref)).or_default() += 1;
            }
        }
        for ((target, key), count) in &needed {
            // `has_room` includes the route being admitted now. `needed`
            // counts every route in this prepared batch, so only its
            // additional siblings are passed as reservations.
            if !self
                .scheduler
                .mailboxes
                .has_room(*target, key, count.saturating_sub(1))
            {
                return Err(VmError::MailboxFull {
                    node: self.plan.nodes[target.index()].path.clone(),
                });
            }
        }
        Ok(())
    }

    /// Routes a committed record's emissions — infallible: capacity
    /// and every target key were reserved before the commit.
    fn route_emissions(
        &mut self,
        slot: Slot,
        source_instance: &InstanceRef,
        prepared: PreparedEmissions,
    ) {
        for PreparedEmission { emission, routes } in prepared.emissions {
            // The emission itself is observable at its source port —
            // `expect relay.out` watches departures, not arrivals.
            self.observed.push(Observed {
                node: Some(self.plan.nodes[slot.index()].path.clone()),
                port: emission.port.clone(),
                envelope: emission.envelope.clone(),
            });
            let root_sink = self.plan.nodes[slot.index()]
                .ports
                .iter()
                .find(|port| port.port == emission.port)
                .is_some_and(|port| port.unwired == crate::plan::UnwiredPolicy::RootSink);
            let claimed = self.route_prepared(
                routes,
                &prepared.candidates,
                &emission.envelope,
                Some(source_instance),
            );
            // The resolved port policy, never its spelling, decides whether
            // an unclaimed output reaches the root sink.
            if claimed == 0 && root_sink {
                if emission.port.name() == "err" {
                    self.errors.push(emission.envelope.clone());
                }
                self.observed.push(Observed {
                    node: None,
                    port: emission.port.clone(),
                    envelope: emission.envelope.clone(),
                });
            }
        }
    }

    /// Applies a preselected fan-out; returns how many edges claimed the
    /// envelope.
    fn route_prepared(
        &mut self,
        routes: Vec<PreparedEdge>,
        candidates: &BTreeMap<InstanceRef, InstanceCandidate>,
        envelope: &Envelope,
        source_instance: Option<&InstanceRef>,
    ) -> usize {
        let claimed = routes.len();
        for PreparedEdge {
            edge,
            target: selected,
        } in routes
        {
            match &edge.to {
                None => {} // `-> nothing`: the explicit, written discard
                Some(to) => match to.node {
                    None => {
                        self.outputs.push(BoundaryOutput {
                            port: to.port.clone(),
                            envelope: envelope.clone(),
                        });
                        self.observed.push(Observed {
                            node: None,
                            port: to.port.clone(),
                            envelope: envelope.clone(),
                        });
                    }
                    Some(target) => {
                        let prepared = selected
                            .as_ref()
                            .unwrap_or_else(|| unreachable!("a node route has a target bundle"));
                        let source = source_instance.unwrap_or(&prepared.scope).clone();
                        self.install_target(prepared, candidates, envelope);
                        let edge_text = self.edge_text(&edge);
                        self.scheduler
                            .mailboxes
                            .deliver(
                                target,
                                &prepared.reference,
                                &source,
                                to.port.clone(),
                                edge_text,
                                envelope.clone(),
                            )
                            .unwrap_or_else(|_| {
                                unreachable!("capacity was reserved before the commit")
                            });
                        let join_within = self.plan.handlers.iter().find_map(|handler| {
                            (handler.node == target)
                                .then_some(&handler.activation)
                                .and_then(|activation| match activation {
                                    node::HandlerActivation::Join {
                                        ports, within_ns, ..
                                    } if ports.contains(&to.port) => *within_ns,
                                    _ => None,
                                })
                        });
                        if let Some(within) = join_within {
                            self.scheduler.arm(
                                self.now.saturating_add(within),
                                DatedPayload::JoinWithin {
                                    operator: self.plan.nodes[target.index()].path.clone(),
                                    edge: self.plan.edge_text(&edge),
                                    delivery: envelope.meta().sys().id(),
                                    instance: prepared.reference.clone(),
                                },
                            );
                        }
                        self.observed.push(Observed {
                            node: Some(self.plan.nodes[target.index()].path.clone()),
                            port: to.port.clone(),
                            envelope: envelope.clone(),
                        });
                        // The budget runs from THIS moment — queue
                        // time counts (execution-model §8). What is
                        // armed here is exactly what the committed
                        // record sealed as its timers.
                        let dynamic_budget = prepared
                            .chain
                            .last()
                            .and_then(|reference| {
                                self.instances
                                    .get(reference)
                                    .map(|instance| &instance.settings)
                                    .or_else(|| {
                                        self.waiting
                                            .get(reference)
                                            .map(|waiting| &waiting.candidate.settings)
                                    })
                            })
                            .and_then(|settings| settings.get("timeout"))
                            .and_then(|value| match value {
                                crate::value::Value::Duration(budget) => Some(*budget),
                                _ => None,
                            });
                        let static_budget = self.plan.nodes[target.index()]
                            .connector()
                            .and_then(|spec| spec.attribute("timeout"))
                            .and_then(|setting| match setting {
                                crate::plan::AttrSetting::Value(crate::value::Value::Duration(
                                    budget,
                                )) => Some(*budget),
                                _ => None,
                            });
                        if let Some(budget) = dynamic_budget.or(static_budget) {
                            self.scheduler.arm(
                                self.now.saturating_add(budget),
                                DatedPayload::DeliveryBudget {
                                    connector: self.plan.nodes[target.index()].path.clone(),
                                    edge: self.plan.edge_text(&edge),
                                    delivery: envelope.meta().sys().id(),
                                    instance: prepared.reference.clone(),
                                },
                            );
                        }
                    }
                },
            }
        }
        claimed
    }

    fn install_target(
        &mut self,
        target: &PreparedTarget,
        candidates: &BTreeMap<InstanceRef, InstanceCandidate>,
        envelope: &Envelope,
    ) {
        for reference in &target.chain {
            let admitted = target.admitted.contains(reference);
            let mut arm_idle = false;
            let owner_slot = match self.instances.get_mut(reference) {
                Some(instance) => {
                    if admitted
                        && instance.last_activity.meta().sys().id() != envelope.meta().sys().id()
                    {
                        instance.last_activity = envelope.clone();
                        arm_idle = true;
                    }
                    instance.owner
                }
                None => {
                    if let Some(waiting) = self.waiting.get_mut(reference) {
                        if admitted
                            && waiting.last_activity.meta().sys().id() != envelope.meta().sys().id()
                        {
                            waiting.last_activity = envelope.clone();
                            waiting.last_activity_at = self.now;
                        }
                        continue;
                    }
                    let candidate = candidates.get(reference).unwrap_or_else(|| {
                        unreachable!("a new selected reference keeps its candidate bundle")
                    });
                    let predecessor_is_live = self
                        .instances
                        .keys()
                        .any(|live| live.address() == reference.address() && live != reference);
                    let parent_is_ready = candidate
                        .parent
                        .as_ref()
                        .is_none_or(|parent| self.instances.contains_key(parent));
                    if predecessor_is_live || !parent_is_ready {
                        self.waiting.insert(
                            reference.clone(),
                            WaitingInstance {
                                candidate: candidate.clone(),
                                created_by: envelope.clone(),
                                last_activity: envelope.clone(),
                                last_activity_at: self.now,
                            },
                        );
                        continue;
                    }
                    let inherited_close = candidate.parent.as_ref().and_then(|parent| {
                        self.instances
                            .get(parent)
                            .and_then(|instance| match &instance.phase {
                                InstancePhase::Open => None,
                                InstancePhase::Closing {
                                    cause,
                                    scope,
                                    stage,
                                } => Some((cause.clone(), scope.clone(), *stage)),
                                InstancePhase::Stopping { cause, scope } => {
                                    Some((cause.clone(), scope.clone(), ClosingStage::Settling))
                                }
                            })
                    });
                    let phase =
                        inherited_close.map_or(InstancePhase::Open, |(cause, scope, stage)| {
                            InstancePhase::Closing {
                                cause,
                                scope,
                                stage,
                            }
                        });
                    let is_closing = matches!(phase, InstancePhase::Closing { .. });
                    self.instances.insert(
                        reference.clone(),
                        Instance {
                            owner: candidate.owner,
                            parent: candidate.parent.clone(),
                            frames: candidate.frames.clone(),
                            values: candidate.values.clone(),
                            settings: candidate.settings.clone(),
                            created_by: envelope.clone(),
                            last_activity: envelope.clone(),
                            residual: Vec::new(),
                            phase,
                        },
                    );
                    if is_closing {
                        self.closing.insert(reference.clone());
                    }
                    self.lifecycle.push_back(LifecycleWork {
                        owner: candidate.owner,
                        instance: reference.clone(),
                        parent_instance: candidate.parent.clone(),
                        lifecycle: node::Lifecycle::Start,
                        cause: envelope.clone(),
                    });
                    arm_idle = admitted;
                    candidate.owner
                }
            };
            if arm_idle
                && let InstanceLifetime::Idle(ttl) =
                    self.plan.owners[owner_slot.index()].instance.lifetime
            {
                self.scheduler.arm(
                    self.now.saturating_add(ttl),
                    DatedPayload::InstanceLifetime {
                        owner: owner_slot,
                        instance: reference.clone(),
                        cause: envelope.meta().sys().id(),
                        deadline: InstanceDeadline::Idle,
                    },
                );
            }
        }
    }

    /// The canonical edge rendering — the same text the consumed
    /// cursor seals.
    fn edge_text(&self, edge: &Edge) -> String {
        self.plan.edge_text(edge)
    }
}
