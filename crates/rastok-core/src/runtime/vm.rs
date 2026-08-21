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
    BoundaryEvent, CallRef, Completion, DatedPayload, Dispatcher, InboundSubmission, Intent,
    Partner, ProcessExit, Response,
};
use crate::datatype::DataType;
use crate::envelope::{Data, Envelope, Id};
use crate::identity::{InstanceAddress, InstanceFrame, InstanceRef};
use crate::journal::{Journal, RouteTarget, RoutedDelivery, ids};
use crate::node;
use crate::plan::{
    AttrSetting, Edge, EndpointRef, EvaluationSite, GraphVersion, NodeKind, OwnerKind, OwnerSlot,
    Slot, ValidatedExecutionPlan,
};
use crate::runtime::{
    DataSlot, Delivery, Fault, InstanceState, LifecycleFiring, Mailboxes, TransactionExecutor,
    eval_lazy, meta_value, resolve_payload,
};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
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
            Self::MissingPending(_) => {
                f.write_str("a closure settled a call with no pending request")
            }
            Self::PlanInvariant { node, reason } => {
                write!(f, "`{node}` violates a validated-plan invariant: {reason}")
            }
            Self::UnknownIngress(port) => {
                write!(f, "`{port}` is not an input of this graph")
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
    last_activity: Envelope,
    expiry_pending: bool,
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
}

#[derive(Debug, Clone)]
struct PreparedTarget {
    reference: InstanceRef,
    scope: InstanceRef,
    chain: Vec<InstanceRef>,
}

/// Lifecycle work owns its causal envelope. Stop work additionally owns the
/// detached state snapshot of the closing generation, allowing a following
/// generation of the same key to start without sharing cells.
#[derive(Debug, Clone)]
struct LifecycleWork {
    slot: Slot,
    instance: InstanceRef,
    parent_instance: Option<InstanceRef>,
    lifecycle: node::Lifecycle,
    cause: Envelope,
    values: BTreeMap<String, crate::value::Value>,
    detached_state: Option<BTreeMap<String, crate::value::Value>>,
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

/// The plan-backed gate for one atomic boundary completion.
///
/// Format v9 retains the exact `ProcessExit` union in the resolved port
/// table. This gate judges that same type, so no malformed terminal fact can
/// become causal.
struct PlanGate<'plan> {
    plan: &'plan ValidatedExecutionPlan,
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
        let Some(spec) = planned.connector() else {
            return Err(format!("`{connector}` is not a connector"));
        };
        if spec.contract != "Process" {
            return Err(format!(
                "the active Partner does not serve `{}` completions",
                spec.contract
            ));
        }
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
            // Validation has already proved that the transported Process
            // contract row is the canonical ProcessExit type. The resolved
            // table is therefore the sole admission authority here too.
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
/// under: `p:in` for a partner answer, `l:timeout` for an expired
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
    /// Journal-before-next-work lifecycle transitions. This queue outranks
    /// ordinary deliveries so `l:start` precedes the creating arrival and a
    /// detached `l:stop` precedes the following generation.
    lifecycle: VecDeque<LifecycleWork>,
    /// Won closures awaiting their completion transaction — replayable:
    /// an entry leaves this queue only when its transaction commits.
    settled: VecDeque<SettledEntry>,
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
            graph_version,
            scheduler: Scheduler::new(1024),
            state: InstanceState::new(),
            dispatcher: Dispatcher::new(),
            pending: BTreeMap::new(),
            instances: BTreeMap::new(),
            lifecycle: VecDeque::new(),
            settled: VecDeque::new(),
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
        // The submission's port is verified against the materialized graph
        // table. Source sugar may spell `.`, but no runtime string creates an
        // implicit ingress.
        let ingress_ok =
            self.plan.boundary.iter().any(|decl| {
                decl.port == submission.port && decl.direction == node::Direction::Input
            });
        if !ingress_ok {
            return Err(VmError::UnknownIngress(submission.port.clone()));
        }
        let from = EndpointRef {
            node: None,
            port: submission.port.clone(),
        };
        // The ext zone, held to the travelling schema BEFORE any
        // append (D-2026-08-10-1): an external caller cannot seed a
        // field no sheet declares — the same grid the checker
        // enforces on sources, applied to the API.
        for (field, value) in &submission.draft.ext {
            // Name AND type, one shared rule (D-2026-08-10-1): the
            // standard field is implicitly `text`, sheet fields
            // answer to their declared types.
            let ty = if field == "content_type" {
                Some(DataType::Text)
            } else {
                self.plan
                    .plan()
                    .ext_schema
                    .iter()
                    .find(|(known, _)| known == field)
                    .map(|(_, ty)| ty.clone())
            };
            let Some(ty) = ty else {
                return Err(VmError::Fault(crate::runtime::Fault::Unrepresentable(
                    alloc::format!("`meta.ext.{field}` is outside the declared schema"),
                )));
            };
            if !crate::runtime::meta_conforms(value, &ty) {
                return Err(VmError::Fault(crate::runtime::Fault::Unrepresentable(
                    alloc::format!("`meta.ext.{field}` is declared {ty:?}; this value is not"),
                )));
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
        let inbound = journal
            .prepare_inbound(submission)
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
        self.route_prepared(prepared, &candidates, &committed.envelope, None);
        Ok(id)
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
        if !self.lifecycle.is_empty() {
            self.fire_lifecycle_next(journal)?;
            return Ok(true);
        }
        if !self.settled.is_empty() {
            self.settle_next(journal)?;
            return Ok(true);
        }
        let Some((slot, key, delivery)) = self.scheduler.mailboxes.peek_next() else {
            return Ok(false);
        };
        match &self.plan.nodes[slot.index()].kind {
            NodeKind::Operator => self.fire_operator(journal, slot, &key, &delivery)?,
            NodeKind::Connector(_) => self.open_call(journal, slot, &key, &delivery)?,
        }
        Ok(true)
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
        while self.step(journal)? {}
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
    pub fn pump(
        &mut self,
        journal: &mut dyn Journal,
        partner: &mut dyn Partner,
    ) -> Result<PumpReport, VmError> {
        let gate = PlanGate { plan: self.plan };
        let batch = self
            .dispatcher
            .pump(journal, partner, &gate)
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
    /// mints a timeout — a partner answers or stays silent, never
    /// times itself out. A budget runs from the DELIVERY, so an
    /// expiry resolves wherever the obligation lives: a call still
    /// open closes with a Timeout fact (and the partner is told to
    /// cancel the in-flight work); a delivery still queued follows
    /// the call-closed rule — its call opens and closes at the
    /// expiry, back to back; an obligation already settled loses the
    /// race and fires nothing.
    ///
    /// # Errors
    ///
    /// Journal refusals and faulting steps, as in [`Self::drain`].
    pub fn advance(
        &mut self,
        journal: &mut dyn Journal,
        partner: &mut dyn Partner,
        target: i64,
    ) -> Result<(), VmError> {
        // Ready work at the current logical instant precedes any future
        // expiry. In particular, a just-created instance commits `l:start`
        // before its first idle deadline can close it.
        self.drain(journal)?;
        while let Some((key, payload)) = self.scheduler.peek_due(target) {
            self.now = key.0;
            // Every expiry OF THIS INSTANT fires before the machine
            // drains: the drain happens between jumps of the clock,
            // never between simultaneous events — a same-instant
            // budget must not see the world advanced by its sibling.
            // A failed expiry stays armed: reserve, resolve, ack.
            self.expire(journal, partner, payload)?;
            self.scheduler.ack(key);
            while let Some((sibling_key, sibling)) = self.scheduler.peek_due(key.0) {
                self.expire(journal, partner, sibling)?;
                self.scheduler.ack(sibling_key);
            }
            self.drain(journal)?;
        }
        self.now = self.now.max(target);
        Ok(())
    }

    /// Resolves one expired budget wherever the obligation lives.
    fn expire(
        &mut self,
        journal: &mut dyn Journal,
        partner: &mut dyn Partner,
        payload: DatedPayload,
    ) -> Result<(), VmError> {
        let (connector, edge, delivery, instance) = match payload {
            DatedPayload::DeliveryBudget {
                connector,
                edge,
                delivery,
                instance,
            } => (connector, edge, delivery, instance),
            DatedPayload::InstanceIdle {
                node,
                instance,
                cause,
            } => {
                self.expire_instance(node, instance, cause);
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
            match journal
                .close_outbound(&call_ref, crate::journal::Closure::Timeout)
                .map_err(VmError::Journal)?
            {
                crate::journal::CloseVerdict::AlreadyClosed => return Ok(()),
                crate::journal::CloseVerdict::Won => {}
            }
            partner.cancel(&call_ref);
            let intent = journal
                .find_call(&call_ref)
                .unwrap_or_else(|| unreachable!("a closed call is journaled"));
            self.settled.push_back(SettledEntry {
                event: BoundaryEvent {
                    call_ref,
                    connector_path: intent.connector_path,
                    emitting_id: intent.emitting_id,
                    completion: Completion::process_exit(ProcessExit::TimeoutExpired),
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
        let queued_id = {
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
            match crate::boundary::prepare_request(
                crate::boundary::RequestCodec::of(spec),
                queued.envelope.data(),
            ) {
                Err(_) => None,
                Ok(prepared) => {
                    let instance_settings = self
                        .instances
                        .get(&instance)
                        .ok_or_else(|| VmError::PlanInvariant {
                            node: planned.path.clone(),
                            reason: "a queued call lost its installed instance bundle",
                        })?
                        .settings
                        .clone();
                    let emitting_id = queued.envelope.meta().sys().id();
                    Some(Intent {
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
                        instance_settings,
                    })
                }
            }
        };
        match queued_id {
            None => {
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
                    let completion = Completion::process_exit(ProcessExit::DecodeFailed {
                        message: String::from("the request cannot be framed"),
                        line: None,
                    });
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
                self.maybe_begin_pending_stop(slot, &instance);
            }
            Some(intent) => {
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
                        completion: Completion::process_exit(ProcessExit::TimeoutExpired),
                    },
                    trigger: node::Trigger::Lifecycle(node::Lifecycle::Timeout),
                });
            }
        }
        Ok(())
    }

    /// Resolves one idle deadline. An event whose causal activity has been
    /// superseded is a harmless clock observation. An open call postpones
    /// closure without losing the request's terminal obligation; settlement
    /// queues the stop afterwards.
    fn expire_instance(&mut self, path: node::Path, reference: InstanceRef, cause: Id) {
        let slot = self.slot_of(&path);
        let current = self
            .instances
            .get(&reference)
            .filter(|instance| instance.last_activity.meta().sys().id() == cause)
            .map(|instance| instance.last_activity.clone());
        let Some(cause) = current else {
            return;
        };
        if self.instance_has_work(slot, &reference) {
            if let Some(instance) = self.instances.get_mut(&reference) {
                instance.expiry_pending = true;
            }
            return;
        }
        self.begin_stop(slot, &reference, cause);
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

    /// Whether the machine is stable: nothing queued, nothing
    /// settled-but-uncommitted, no open call held.
    #[must_use]
    pub fn is_stable(&self) -> bool {
        self.scheduler.mailboxes.is_empty()
            && self.lifecycle.is_empty()
            && self.settled.is_empty()
            && self.pending.is_empty()
    }

    /// Whether calls are open against the world — work [`Self::drain`]
    /// cannot finish alone.
    #[must_use]
    pub fn has_open_obligations(&self) -> bool {
        !self.pending.is_empty()
    }

    fn lifecycle_managed(&self, slot: Slot) -> bool {
        self.plan.node_instance(slot).is_some_and(|instance| {
            instance.key.is_some() || instance.lifetime != crate::plan::InstanceLifetime::Persistent
        }) || self.plan.handlers.iter().any(|handler| {
            handler.node == slot
                && matches!(
                    handler.activation,
                    node::HandlerActivation::Lifecycle(
                        node::Lifecycle::Start | node::Lifecycle::Stop
                    )
                )
        })
    }

    fn instance_has_work(&self, slot: Slot, instance: &InstanceRef) -> bool {
        self.scheduler.mailboxes.has_work(slot, instance)
            || self
                .pending
                .values()
                .any(|pending| pending.slot == slot && pending.instance == *instance)
    }

    /// Detaches a closing generation before its stop transaction. New work
    /// for the same logical key can then create a fresh state cell, while the
    /// queued stop owns the old snapshot and remains retryable on refusal.
    fn begin_stop(&mut self, slot: Slot, instance: &InstanceRef, cause: Envelope) {
        let Some(bundle) = self.instances.remove(instance) else {
            return;
        };
        let detached_state = self.state.take_unchecked(self.plan.plan(), slot, instance);
        self.lifecycle.push_back(LifecycleWork {
            slot,
            instance: instance.clone(),
            parent_instance: bundle.parent,
            lifecycle: node::Lifecycle::Stop,
            cause,
            values: bundle.values,
            detached_state: Some(detached_state),
        });
    }

    fn maybe_begin_pending_stop(&mut self, slot: Slot, instance_ref: &InstanceRef) {
        let cause = self
            .instances
            .get(instance_ref)
            .filter(|instance| instance.expiry_pending)
            .map(|instance| instance.last_activity.clone());
        if let Some(cause) = cause
            && !self.instance_has_work(slot, instance_ref)
        {
            self.begin_stop(slot, instance_ref, cause);
        }
    }

    /// Commits exactly one queued lifecycle transition. Start state remains
    /// live; stop state was detached when admission closed and is reclaimed
    /// only by this successful commit.
    fn fire_lifecycle_next(&mut self, journal: &mut dyn Journal) -> Result<(), VmError> {
        let Some(work) = self.lifecycle.front().cloned() else {
            return Ok(());
        };
        let state = work.detached_state.clone().unwrap_or_else(|| {
            self.state
                .cell_unchecked(self.plan.plan(), work.slot, &work.instance)
        });
        let executor = TransactionExecutor::new(self.plan, self.graph_version);
        let parent_instance = (work.lifecycle == node::Lifecycle::Start)
            .then_some(work.parent_instance.as_ref())
            .flatten();
        let mut outcome = executor
            .fire_lifecycle(
                work.slot,
                work.lifecycle,
                LifecycleFiring {
                    instance: &work.instance,
                    parent_instance,
                    cause: &work.cause,
                },
                &state,
                &work.values,
            )
            .map_err(VmError::Fault)?;
        let prepared = self.validate_and_prepare_emissions(
            journal,
            work.slot,
            &work.instance,
            &outcome.record,
        )?;
        outcome.record.routes = self.routed_deliveries(&prepared);
        journal
            .commit(outcome.record.clone())
            .map_err(VmError::Journal)?;
        self.lifecycle.pop_front();
        if work.lifecycle == node::Lifecycle::Start {
            self.state.apply_unchecked(
                self.plan.plan(),
                work.slot,
                &work.instance,
                &outcome.record.state_delta,
            );
        }
        self.route_emissions(work.slot, &work.instance, prepared);
        Ok(())
    }

    /// One won closure becomes the connector's completion transaction.
    /// The event leaves the queue only after its commit; every
    /// integrity check happens before.
    fn settle_next(&mut self, journal: &mut dyn Journal) -> Result<(), VmError> {
        let SettledEntry { event, trigger } = self
            .settled
            .front()
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
        self.settled.pop_front();
        self.pending.remove(&event.call_ref);
        let instance_still_open = self
            .pending
            .values()
            .any(|pending| pending.slot == slot && pending.instance == request.instance);
        if !instance_still_open {
            self.scheduler.mailboxes.release(slot, &request.instance);
        }
        self.route_emissions(slot, &request.instance, prepared);
        self.maybe_begin_pending_stop(slot, &request.instance);
        Ok(())
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
        let prepared =
            self.validate_and_prepare_emissions(journal, slot, instance, &outcome.record)?;
        outcome.record.routes = self.routed_deliveries(&prepared);
        journal
            .commit(outcome.record.clone())
            .map_err(VmError::Journal)?;
        self.scheduler.mailboxes.pop(slot, instance);
        self.state.apply_unchecked(
            self.plan.plan(),
            slot,
            instance,
            &outcome.record.state_delta,
        );
        self.route_emissions(slot, instance, prepared);
        self.maybe_begin_pending_stop(slot, instance);
        Ok(())
    }

    /// A delivery reaching a connector opens its outbound call:
    /// journal-before-effect — the open fact exists before any
    /// partner hears — and the request is held until the call
    /// settles, its mailbox claimed for the whole wait: the worker is
    /// free, the node is not.
    fn open_call(
        &mut self,
        journal: &mut dyn Journal,
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
        // sealed digest and the bytes the partner will receive are
        // one value carried by the intent — nobody recomputes. An
        // unframeable request completes on `exit` before any open
        // fact exists: stderr is not a lifecycle channel.
        let prepared = match crate::boundary::prepare_request(
            crate::boundary::RequestCodec::of(spec),
            delivery.envelope.data(),
        ) {
            Ok(prepared) => prepared,
            Err(message) => {
                let executor = TransactionExecutor::new(self.plan, self.graph_version);
                let completion = Completion::process_exit(ProcessExit::DecodeFailed {
                    message,
                    line: None,
                });
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
                self.maybe_begin_pending_stop(slot, instance);
                return Ok(());
            }
        };
        // The call identity: one delivery carrying one prepared
        // request (envelope.md §2) — the reference says by where,
        // the digest says what.
        let instance_settings = self
            .instances
            .get(instance)
            .ok_or_else(|| VmError::PlanInvariant {
                node: planned.path.clone(),
                reason: "an opening call lost its installed instance bundle",
            })?
            .settings
            .clone();
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
            instance_settings,
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
    /// invalid partner draft before closure. THIS final, non-recursive level
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
        for (index, owner_slot) in chain.iter().copied().enumerate() {
            if index <= scope_index
                && let Some(reference) =
                    self.source_instance_for_owner(source_instance, owner_slot)?
            {
                selected.push(reference);
            } else {
                let parent = selected.last();
                selected
                    .push(self.candidate_owner(journal, owner_slot, parent, envelope, candidates)?);
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

    fn candidate_owner(
        &self,
        journal: &dyn Journal,
        owner_slot: OwnerSlot,
        parent: Option<&InstanceRef>,
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
        if owner.instance.key.is_none() {
            if let Some((reference, _)) = self.instances.iter().find(|(_, instance)| {
                instance.owner == owner_slot && instance.parent.as_ref() == parent
            }) {
                return Ok(reference.clone());
            }
            if let Some(candidate) = candidates.values().find(|candidate| {
                candidate.owner == owner_slot && candidate.parent.as_ref() == parent
            }) {
                return Ok(candidate.reference.clone());
            }
        }
        let (inherited, mut frames) = match parent {
            None => (BTreeMap::new(), Vec::new()),
            Some(reference) => {
                if let Some(instance) = self.instances.get(reference) {
                    (instance.values.clone(), instance.frames.clone())
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
        let address = InstanceAddress::new(&owner.path, &frames)
            .map_err(|error| VmError::Fault(Fault::Unrepresentable(error.to_string())))?;
        let reference = journal
            .instance_ref_for_admission(&address)
            .map_err(VmError::Journal)?;
        if self.instances.contains_key(&reference) {
            return Ok(reference);
        }
        if let Some(candidate) = candidates.get(&reference) {
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
        let data = DataSlot::new(envelope.data());
        let mut settings = BTreeMap::new();
        for (name, setting) in &spec.attributes {
            let AttrSetting::Expr { site, expr } = setting else {
                continue;
            };
            if *site == EvaluationSite::Invocation {
                continue;
            }
            let value = resolve_payload(
                eval_lazy(expr, &bindings, &data, &self.plan.assets).map_err(VmError::Fault)?,
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
                            .and_then(|reference| self.instances.get(reference))
                            .and_then(|instance| instance.settings.get("timeout"))
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
            let owner_slot = match self.instances.get_mut(reference) {
                Some(instance) => {
                    instance.last_activity = envelope.clone();
                    instance.expiry_pending = false;
                    instance.owner
                }
                None => {
                    let candidate = candidates.get(reference).unwrap_or_else(|| {
                        unreachable!("a new selected reference keeps its candidate bundle")
                    });
                    self.instances.insert(
                        reference.clone(),
                        Instance {
                            owner: candidate.owner,
                            parent: candidate.parent.clone(),
                            frames: candidate.frames.clone(),
                            values: candidate.values.clone(),
                            settings: candidate.settings.clone(),
                            last_activity: envelope.clone(),
                            expiry_pending: false,
                        },
                    );
                    if let OwnerKind::Node(slot) = self.plan.owners[candidate.owner.index()].kind
                        && self.lifecycle_managed(slot)
                    {
                        self.lifecycle.push_back(LifecycleWork {
                            slot,
                            instance: reference.clone(),
                            parent_instance: candidate.parent.clone(),
                            lifecycle: node::Lifecycle::Start,
                            cause: envelope.clone(),
                            values: candidate.values.clone(),
                            detached_state: None,
                        });
                    }
                    candidate.owner
                }
            };
            if let crate::plan::InstanceLifetime::Idle(ttl) =
                self.plan.owners[owner_slot.index()].instance.lifetime
                && let OwnerKind::Node(slot) = self.plan.owners[owner_slot.index()].kind
            {
                self.scheduler.arm(
                    self.now.saturating_add(ttl),
                    DatedPayload::InstanceIdle {
                        node: self.plan.nodes[slot.index()].path.clone(),
                        instance: reference.clone(),
                        cause: envelope.meta().sys().id(),
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
