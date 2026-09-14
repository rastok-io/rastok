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
//! A transaction is atomic in [`HandlerExecutor`]; this module
//! keeps it atomic across the machine. Every step follows the same
//! discipline: *reserve* the head of a mailbox without removing it,
//! *evaluate* on isolated state, *reserve capacity* in every target
//! mailbox, *commit* to the journal — and only then remove the
//! delivery, apply the state, and route. A fault or refusal anywhere
//! before the commit leaves the delivery exactly where it was; after
//! the commit, nothing can fail.

use crate::boundary::{
    BoundaryEvent, CallRef, ClosingDeadline, Completion, CompletionGate, DatedPayload, Dispatcher,
    InboundAcceptance, InboundAdmission, InboundCallEvent, InboundCallRef, InboundCallResponse,
    InboundCallTimeout, InboundProposal, InboundRejection, InboundSession, InboundSubmission,
    InstanceDeadline, Intent, OutboundConnectorBindings, RequestPreparation, Response,
    SchedulerCompletion, SessionDriverEvent, SessionOpenIntent, SessionRef, SessionStart,
    SessionWriteIntent, SessionWriteRef,
};
use crate::connector::ConnectorExchange;
use crate::envelope::{Data, Envelope, Id, Meta, MetaSys};
use crate::identity::{InstanceAddress, InstanceFrame, InstanceRef};
use crate::journal::admission::InstanceAdmissions;
use crate::journal::{Emission, Journal, LoopRunRef, RouteTarget, RoutedDelivery, ids};
use crate::node;
use crate::plan::{
    AttrSetting, Edge, EndpointRef, EvaluationSite, GraphVersion, InstanceLifetime, NodeExecution,
    OwnerKind, OwnerPath, OwnerSlot, Slot, ValidatedExecutionPlan,
};
use crate::runtime::{
    DataSlot, Delivery, Fault, HandlerExecutor, InstanceState, JoinCandidate, LifecycleFiring,
    Mailboxes, eval_lazy, meta_value, resolve_payload,
};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

mod advance;
pub use advance::TimeAdvance;
mod call_completion;
mod closing;
mod graph_loop;
mod instance_stop;
mod loop_manager;
mod recovery;
mod timing;
use core::fmt;

/// A machine fault; the step that raised it committed nothing, and
/// the work that triggered it stays reserved — a retry sees it again.
/// A machine-integrity refusal — never a business error.
#[derive(Debug, Clone, PartialEq)]
pub enum VmError {
    /// Recovery cannot reconstruct this profile from the retained facts.
    RecoveryUnavailable {
        /// Missing reconstruction obligation, never a guessed default.
        reason: &'static str,
    },
    /// A committed record contradicts its plan, routes or consumed cursors.
    InvalidRecovery {
        /// Zero-based position in the committed prefix.
        record: usize,
        /// The violated replay invariant.
        reason: &'static str,
    },
    /// Classified payload metadata reached execution without its required
    /// deployment protocol. Raw secret payloads remain fail-closed; personal
    /// data requires A4's complete attested privacy profile.
    UnsupportedDataPolicy {
        /// First deterministic unsupported classified schema site.
        location: String,
    },
    /// The journal run header does not carry the exact deployment facts with
    /// which this VM was booted. No causal mutation may cross that split.
    DeploymentAttestationMismatch,
    /// One categorical boot reference has no compatible resolved binding.
    /// The VM cannot defer this failure until a later Driver effect.
    MissingDeploymentBinding {
        /// Exact Connector setting that requires the binding.
        location: String,
    },
    /// The runtime image cannot execute this Extension on its resolved host.
    /// It must refuse before any effect, never ignore the Extension.
    UnsupportedExtension {
        /// Node carrying the Extension.
        owner: OwnerPath,
        /// Unsupported Extension.
        extension: node::Extension,
    },
    /// An evaluation or engine fault. It is outside every graph port
    /// contract: the transaction commits nothing and supervision decides.
    Fault(Fault),
    /// A typed Driver outcome contradicts its Contract's production guarantees.
    ContractViolation {
        /// Connector which supplied the contradictory outcome.
        node: node::Path,
        /// Stable explanation, without payload data.
        reason: &'static str,
    },
    /// The journal refused an operation.
    Journal(crate::journal::Error),
    /// Exact-instance closure cannot select a unique retained target.
    InstanceCommand {
        /// Static target which refused; no payload values enter the error.
        owner: OwnerPath,
        /// Missing, ambiguous, or inconsistent instance authority.
        reason: crate::journal::instance_stop::Error,
    },
    /// Instance command capacity is checked with the other producer products.
    InstanceCommandQueueFull,
    /// The dispatcher raised an integrity fault.
    Dispatch(crate::boundary::DispatchError),
    /// A target mailbox has no room for a transaction's deliveries —
    /// backpressure surfaced before the commit.
    MailboxFull {
        /// The refusing node.
        node: node::Path,
    },
    /// The bounded Loop-manager queue cannot admit another external root.
    LoopQueueFull {
        /// Loop owner refusing the root.
        owner: OwnerPath,
    },
    /// Runtime defense for a control decision the validated L3 proof says is
    /// impossible.
    InvalidLoopDecision {
        /// Loop owner whose traversal contradicted the proof.
        owner: OwnerPath,
        /// Stable invariant description.
        reason: &'static str,
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
            Self::RecoveryUnavailable { reason } => write!(f, "recovery unavailable: {reason}"),
            Self::InvalidRecovery { record, reason } => {
                write!(f, "recovery rejected record {record}: {reason}")
            }
            Self::UnsupportedDataPolicy { location } => write!(
                f,
                "data policy at {location} requires the M1.8 A4 deployment boundary"
            ),
            Self::DeploymentAttestationMismatch => {
                f.write_str("the journal deployment attestation does not match the booted bindings")
            }
            Self::MissingDeploymentBinding { location } => {
                write!(f, "deployment binding required by {location} is missing")
            }
            Self::UnsupportedExtension { owner, extension } => write!(
                f,
                "Node `{owner}` carries `{extension}`, which this runtime cannot execute"
            ),
            Self::Fault(fault) => fault.fmt(f),
            Self::Journal(error) => error.fmt(f),
            Self::InstanceCommand { owner, reason } => {
                write!(f, "instance command at `{owner}`: {reason}")
            }
            Self::InstanceCommandQueueFull => {
                f.write_str("the bounded instance-command queue has no room before producer commit")
            }
            Self::Dispatch(error) => error.fmt(f),
            Self::MailboxFull { node } => {
                write!(f, "the mailbox of `{node}` is full")
            }
            Self::LoopQueueFull { owner } => {
                write!(f, "the Loop work queue of `{owner}` is full")
            }
            Self::InvalidLoopDecision { owner, reason } => {
                write!(
                    f,
                    "Loop `{owner}` made an invalid progress decision: {reason}"
                )
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
            Self::ContractViolation { node, reason } => {
                write!(f, "ContractViolation at `{node}`: {reason}")
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
    /// Duplicate or post-terminal Session observations.
    pub discarded_sessions: Vec<SessionDriverEvent>,
    /// Session observations matching no live logical Session.
    pub uncorrelated_sessions: Vec<SessionDriverEvent>,
    /// Session terminal observations refused by the resolved output table.
    pub rejected_sessions: Vec<(SessionDriverEvent, String)>,
}

/// Non-causal observations from one inbound-Call Driver drain.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InboundCallPumpReport {
    /// Valid terminal events retained for a Connector transaction.
    pub queued: usize,
    /// Duplicate terminal events observed after one was already retained.
    pub discarded: Vec<InboundCallEvent>,
    /// Events matching no admitted Call in this VM.
    pub uncorrelated: Vec<InboundCallEvent>,
    /// Events refused by the resolved Connector output table. The Call stays
    /// open so the Driver or a later timeout may still terminate it.
    pub rejected: Vec<(InboundCallEvent, String)>,
}

impl PumpReport {
    fn append(&mut self, mut other: Self) {
        self.settled = self.settled.saturating_add(other.settled);
        self.discarded.append(&mut other.discarded);
        self.uncorrelated.append(&mut other.uncorrelated);
        self.rejected.append(&mut other.rejected);
        self.discarded_sessions
            .append(&mut other.discarded_sessions);
        self.uncorrelated_sessions
            .append(&mut other.uncorrelated_sessions);
        self.rejected_sessions.append(&mut other.rejected_sessions);
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

/// One accepted inbound Call held until its Driver reports the sole terminal
/// completion. Unlike an outbound call, no input cursor is claimed: the root
/// itself opened this obligation.
#[derive(Debug, Clone, PartialEq)]
struct PendingInboundCall {
    slot: Slot,
    request: Envelope,
    instance: InstanceRef,
    response: Option<PendingInboundCallResponse>,
    timeout: Option<PendingInboundCallTimeout>,
}

/// One graph response held behind the source Connector's claimed mailbox
/// until its protocol terminal observation commits `l:reply`.
#[derive(Debug, Clone, PartialEq)]
struct PendingInboundCallResponse {
    delivery: Delivery,
    started: bool,
}

/// Scheduler-to-Driver handoff for one request deadline. `requested` changes
/// only when the dated event wins; `started` changes only after the Driver
/// accepts the bounded protocol attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingInboundCallTimeout {
    requested: bool,
    started: bool,
}

/// One journaled logical Session owned by a Connector instance.
#[derive(Debug, Clone, PartialEq)]
struct PendingSession {
    slot: Slot,
    instance: InstanceRef,
    opening: Envelope,
    /// Present only for an outbound opening whose exact prepared bytes may
    /// need to be re-offered after bounded Driver pressure.
    open_intent: Option<SessionOpenIntent>,
    /// Outbound client openings retain their graph cursor through the sole
    /// terminal Session transaction. Inbound peer openings are roots and
    /// therefore carry no consumed cursor.
    opening_delivery: Option<Delivery>,
    opened: bool,
    opening_started: bool,
    opening_claimed: bool,
    close_started: bool,
}

/// One graph-to-world write retained until the Driver accepts its bounded
/// action and later reports local settlement.
#[derive(Debug, Clone, PartialEq)]
struct PendingSessionWrite {
    intent: SessionWriteIntent,
    slot: Slot,
    instance: InstanceRef,
    delivery: Delivery,
    started: bool,
}

/// One mailbox delivery being dispatched to an already-selected Session.
/// Keeping its three routing facts together prevents a write from being
/// prepared for one instance and retained from another.
struct SessionDispatch<'a> {
    slot: Slot,
    instance: &'a InstanceRef,
    delivery: &'a Delivery,
}

/// One accepted terminal observation waiting for its atomic Connector
/// transaction.
#[derive(Debug, Clone, PartialEq)]
struct SettledSession {
    session: SessionRef,
    completion: Completion,
    /// A preparation refusal can terminate a Session before a write intent
    /// opens. Its causing graph cursor is consumed by the terminal
    /// transaction together with the opening cursor.
    terminal_delivery: Option<Delivery>,
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
/// prevents a replay-sensitive second evaluation of `instance.key`.
#[derive(Debug, Clone)]
struct PreparedEdge {
    edge: Edge,
    target: Option<PreparedTarget>,
    /// A retained Graph boundary is a routing stage, not an executable Node.
    boundary: Option<Box<graph_loop::BoundaryRoute>>,
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
    contexts: Vec<crate::journal::loop_protocol::RoutedContext>,
    commands: Vec<crate::journal::instance_stop::Command>,
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

/// Read-only lifecycle state exposed to hosted debuggers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstancePhaseInspection {
    /// The instance accepts ordinary work.
    Open,
    /// External admission is closed while admitted work drains.
    Draining,
    /// Open effects are being settled under the closing budget.
    Settling,
    /// The terminal lifecycle transaction is waiting to commit.
    Stopping,
}

/// One live logical instance and its Node-private state, if it owns a Node.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceInspection {
    /// Canonical logical address plus journal-derived generation.
    pub reference: InstanceRef,
    /// Static lexical owner.
    pub owner: OwnerPath,
    /// Current replay-derivable lifecycle phase.
    pub phase: InstancePhaseInspection,
    /// Executable Node at this owner, absent for a Graph owner.
    pub node: Option<node::Path>,
    /// Current Node-private cells with defaults applied.
    pub state: BTreeMap<String, crate::value::Value>,
}

/// One live per-instance mailbox exposed to hosted debuggers.
#[derive(Debug, Clone, PartialEq)]
pub struct MailboxInspection {
    /// Executable Node that owns this FIFO.
    pub node: node::Path,
    /// Exact logical instance generation owning the FIFO.
    pub instance: InstanceRef,
    /// Whether execution has reserved the mailbox head for an open protocol.
    pub claimed: bool,
    /// Current FIFO contents in delivery order.
    pub deliveries: Vec<Delivery>,
}

/// Direction of one retained Call obligation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallDirectionInspection {
    /// Graph-to-world Call.
    Outbound,
    /// World-to-graph Call.
    Inbound,
}

/// Typed identity of one retained Call obligation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CallReferenceInspection {
    /// Graph-to-world Call identity.
    Outbound(CallRef),
    /// World-to-graph Call identity.
    Inbound(InboundCallRef),
}

/// One Call held by the machine at the inspection boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallInspection {
    /// Typed Call identity.
    pub reference: CallReferenceInspection,
    /// Direction relative to the graph.
    pub direction: CallDirectionInspection,
    /// Connector owning the protocol.
    pub connector: node::Path,
    /// Exact logical instance generation.
    pub instance: InstanceRef,
    /// Whether the terminal observation is retained but not yet consumed.
    pub completion_retained: bool,
}

/// Direction in which one Session was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionDirectionInspection {
    /// Graph opened the Session toward the world.
    Outbound,
    /// World opened the Session toward the graph.
    Inbound,
}

/// One long-lived Session held by the machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInspection {
    /// Logical Session identity.
    pub reference: SessionRef,
    /// Direction of the opening.
    pub direction: SessionDirectionInspection,
    /// Connector owning the protocol.
    pub connector: node::Path,
    /// Exact logical instance generation.
    pub instance: InstanceRef,
    /// Whether the opening handshake completed.
    pub opened: bool,
    /// Journaled writes not yet locally settled.
    pub pending_writes: usize,
    /// Whether a terminal observation is retained but not yet consumed.
    pub completion_retained: bool,
}

/// Disposable read model over VM-owned mutable state.
///
/// It is deliberately a clone: a debugger can retain and format it without
/// borrowing the scheduler or becoming an execution authority.
#[derive(Debug, Clone, PartialEq)]
pub struct VmInspection {
    /// Capacity shared by all incoming routes of one instance mailbox.
    pub mailbox_capacity: usize,
    /// Live logical instances in canonical reference order.
    pub instances: Vec<InstanceInspection>,
    /// Current bounded mailbox contents in canonical owner/instance order.
    pub mailboxes: Vec<MailboxInspection>,
    /// Retained Call obligations in canonical identity order.
    pub calls: Vec<CallInspection>,
    /// Retained Sessions in canonical identity order.
    pub sessions: Vec<SessionInspection>,
}

/// One won closure and the trigger its completion transaction fires
/// under: `p:in` for a driver answer, `l:timeout` for an expired
/// budget.
#[derive(Debug, Clone, PartialEq)]
struct SettledEntry {
    event: BoundaryEvent,
    trigger: node::Trigger,
}

const WORKER_MAILBOX_CAPACITY: usize = 1024;

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
    fn peek_due(
        &self,
        target: i64,
        eligible: impl Fn(&DatedPayload) -> bool,
    ) -> Option<((i64, u64), DatedPayload)> {
        self.dated
            .range(..=(target, u64::MAX))
            .find(|(_, payload)| eligible(payload))
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
    /// First still-unsupported classified schema, computed once at boot. Raw
    /// secret payloads remain closed; personal data opens only under the
    /// attested run-scoped privacy profile.
    unsupported_data_policy: Option<String>,
    /// First categorical boot setting without a type-compatible resolved
    /// binding. This closes the library API as well as the hosted CLI path.
    missing_deployment_binding: Option<String>,
    /// Whether any executable schema position contains personal data. A
    /// matching profile header is insufficient unless the Journal separately
    /// advertises an admitted retention capability.
    requires_personal_retention: bool,
    /// Exact run-header facts expected before any causal mutation. Physical
    /// secret bytes are absent; only provider coordinates and versions live
    /// here.
    expected_deployment: Option<crate::deployment::DeploymentAttestation>,
    /// Plan-derived owner chains, indexed once so route selection allocates
    /// only the dynamic reference chain it must commit.
    owner_chains: Vec<Option<Vec<OwnerSlot>>>,
    /// Whether any activation can retain a partial presence set. The common
    /// no-join plan can keep FIFO dispatch on its original direct path.
    has_join_activations: bool,
    /// Whether command-based Loop work can exist. Non-Loop plans do not
    /// build or scan a manager fold.
    has_loops: bool,
    instance_commands: Option<VecDeque<crate::journal::instance_stop::Command>>,
    graph_loops: Option<Box<graph_loop::GraphLoops>>,
    /// Persistent-only plans do not inspect owner lifetimes on each route.
    has_lifetime_deadlines: bool,
    /// A committed single-pass start creates a recoverable close obligation.
    has_single_pass: bool,
    graph_version: GraphVersion,
    scheduler: Scheduler,
    state: InstanceState,
    dispatcher: Dispatcher,
    /// Requests delivered to a connector, held until their call
    /// settles: the completion transaction consumes this cursor, and the
    /// connector's mailbox stays claimed the whole time.
    pending: BTreeMap<CallRef, PendingCall>,
    /// Accepted inbound Calls, bounded per Connector instance by the same
    /// Worker capacity as ordinary mailboxes.
    inbound_calls: BTreeMap<InboundCallRef, PendingInboundCall>,
    /// Driver terminal observations waiting for their atomic Connector
    /// transaction. A reference appears at most once.
    settled_inbound_calls: VecDeque<InboundCallEvent>,
    /// Open inbound and outbound Sessions, keyed only by the journal-derived
    /// logical reference.
    sessions: BTreeMap<SessionRef, PendingSession>,
    /// Session writes whose journal-before-effect intent is already durable.
    session_writes: BTreeMap<SessionWriteRef, PendingSessionWrite>,
    /// Session terminal observations waiting for their causal transaction.
    settled_sessions: VecDeque<SettledSession>,
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
    /// Disposable index reconstructed from the journal's command protocol.
    loop_manager: Option<Box<crate::journal::loop_protocol::Fold>>,
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
    observed: Vec<Observed>,
}

fn first_annotation_location(
    plan: &ValidatedExecutionPlan,
    contains: fn(&crate::schema_annotation::AnnotationTable) -> bool,
) -> Option<String> {
    for port in plan.resolved_ports().boundary() {
        if contains(port.annotations()) {
            return Some(format!("boundary port `{}`", port.port()));
        }
    }
    for (index, planned) in plan.nodes.iter().enumerate() {
        let slot = Slot::new(u32::try_from(index).unwrap_or(u32::MAX));
        if let Some(ports) = plan.resolved_ports().node(slot) {
            for port in ports {
                if contains(port.annotations()) {
                    return Some(format!("port `{}.{}`", planned.path, port.port()));
                }
            }
        }
        for var in &planned.vars {
            if contains(&var.annotations) {
                return Some(format!("var `{}.{}`", planned.path, var.name));
            }
        }
    }
    for owner in &plan.owners {
        for port in &owner.ports {
            if contains(&port.annotations) {
                return Some(format!("Graph port `{}.{}`", owner.path, port.port));
            }
        }
        for value in &owner.vals {
            if contains(&value.annotations) {
                return Some(format!("value `{}.{}`", owner.path, value.name));
            }
        }
    }
    for (handler_index, handler) in plan.handlers.iter().enumerate() {
        for step in handler.dispatch.steps() {
            if let crate::plan::DispatchStep::Let {
                name, annotations, ..
            } = step
                && contains(annotations)
            {
                return Some(format!("handler {handler_index} let `{name}`"));
            }
        }
    }
    None
}

fn first_unsupported_data_policy_location(
    plan: &ValidatedExecutionPlan,
    boot: Option<&crate::deployment::BootBindings>,
) -> Option<String> {
    first_annotation_location(plan, crate::data_policy::contains_secret)
        .map(|location| format!("secret-classified {location}"))
        .or_else(|| {
            let location = first_annotation_location(plan, crate::data_policy::contains_personal)?;
            let requirements = personal_data_requirements(plan);
            (!boot
                .and_then(crate::deployment::BootBindings::privacy)
                .is_some_and(|profile| profile.covers(requirements)))
            .then(|| format!("personal-data {location}"))
        })
}

fn personal_data_requirements(
    plan: &ValidatedExecutionPlan,
) -> crate::deployment::PrivacyRequirements {
    let mut requirements = crate::deployment::PrivacyRequirements::default();
    let mut inspect = |table: &crate::schema_annotation::AnnotationTable| {
        for set in table.entries().values() {
            for annotation in set.annotations() {
                if let crate::schema_annotation::DataAnnotation::Personal(class) = annotation {
                    requirements.personal = true;
                    requirements.special_category |= class.is_special_category();
                    requirements.criminal_offence |= class.is_criminal_offence();
                }
            }
        }
    };
    for port in plan.resolved_ports().boundary() {
        inspect(port.annotations());
    }
    for (index, planned) in plan.nodes.iter().enumerate() {
        let slot = Slot::new(u32::try_from(index).unwrap_or(u32::MAX));
        if let Some(ports) = plan.resolved_ports().node(slot) {
            for port in ports {
                inspect(port.annotations());
                if planned.connector().is_some()
                    && port.direction() == node::Direction::Input
                    && crate::data_policy::contains_personal(port.annotations())
                {
                    requirements.external_disclosure = true;
                }
            }
        }
        for var in &planned.vars {
            inspect(&var.annotations);
        }
    }
    for owner in &plan.owners {
        for port in &owner.ports {
            inspect(&port.annotations);
        }
        for value in &owner.vals {
            inspect(&value.annotations);
        }
    }
    for handler in &plan.handlers {
        for step in handler.dispatch.steps() {
            if let crate::plan::DispatchStep::Let { annotations, .. } = step {
                inspect(annotations);
            }
        }
    }
    requirements
}

fn first_missing_deployment_binding(
    plan: &ValidatedExecutionPlan,
    boot: Option<&crate::deployment::BootBindings>,
) -> Option<String> {
    for planned in &plan.plan().nodes {
        let Some(spec) = planned.connector() else {
            continue;
        };
        for (name, setting) in &spec.attributes {
            let crate::plan::AttrSetting::Expr { expr, .. } = setting else {
                continue;
            };
            let Ok(Some(reference)) = crate::expr::boot_reference(&expr.root) else {
                continue;
            };
            let resolved = match (&reference, boot) {
                (crate::deployment::BootReference::Environment(environment), Some(boot)) => boot
                    .environment(environment)
                    .is_some_and(|value| expr.ty.violation(name, value).is_none()),
                (crate::deployment::BootReference::Secret(_), Some(boot)) => {
                    boot.resolves(&reference)
                }
                (_, None) => false,
            };
            if !resolved {
                return Some(format!("`{}.{name}`", planned.path));
            }
        }
    }
    None
}

impl<'plan> Vm<'plan> {
    fn preflight(&self, journal: &dyn Journal) -> Result<(), VmError> {
        if let Some(error) = self.unsupported_feature() {
            return Err(error);
        }
        if self.requires_personal_retention
            && journal.privacy_retention() != Some(crate::deployment::PrivacyRetention::RunScoped)
        {
            return Err(VmError::UnsupportedDataPolicy {
                location: String::from("personal-data journal retention capability"),
            });
        }
        if journal.deployment_attestation() != self.expected_deployment.as_ref() {
            return Err(VmError::DeploymentAttestationMismatch);
        }
        Ok(())
    }

    fn loop_delay_for(&self, owner: OwnerSlot, iteration: u64) -> Result<Option<i64>, VmError> {
        let path = self.plan.owners[owner.index()].path.clone();
        let Some(spec) = self.plan.owners[owner.index()].loop_spec() else {
            return Err(VmError::InvalidLoopDecision {
                owner: path,
                reason: "the Loop manager lost its validated Extension specification",
            });
        };
        let duration = |constant: &crate::plan::PlanConstant| match constant.resolved {
            crate::value::Value::Duration(value) if value > 0 => Some(value),
            _ => None,
        };
        match &spec.delay {
            None => Ok(None),
            Some(crate::plan::LoopDelay::Fixed { after }) => {
                duration(after)
                    .map(Some)
                    .ok_or(VmError::InvalidLoopDecision {
                        owner: path,
                        reason: "the fixed Loop delay is not a positive duration",
                    })
            }
            Some(crate::plan::LoopDelay::Exponential { first, ceiling }) => {
                let Some(first) = duration(first) else {
                    return Err(VmError::InvalidLoopDecision {
                        owner: path,
                        reason: "the first exponential Loop delay is not positive",
                    });
                };
                let Some(ceiling) = duration(ceiling) else {
                    return Err(VmError::InvalidLoopDecision {
                        owner: path,
                        reason: "the exponential Loop delay ceiling is not positive",
                    });
                };
                if first > ceiling || iteration == 0 {
                    return Err(VmError::InvalidLoopDecision {
                        owner: path,
                        reason: "the exponential Loop delay contradicts its validated bounds",
                    });
                }
                let shift = iteration - 1;
                let scaled = u32::try_from(shift)
                    .ok()
                    .filter(|shift| *shift < i64::BITS - 1)
                    .and_then(|shift| 1_i64.checked_shl(shift))
                    .and_then(|factor| first.checked_mul(factor))
                    .unwrap_or(ceiling);
                Ok(Some(scaled.min(ceiling)))
            }
        }
    }

    fn unsupported_feature(&self) -> Option<VmError> {
        if let Some(location) = &self.missing_deployment_binding {
            return Some(VmError::MissingDeploymentBinding {
                location: location.clone(),
            });
        }
        if let Some(location) = &self.unsupported_data_policy {
            return Some(VmError::UnsupportedDataPolicy {
                location: location.clone(),
            });
        }
        self.plan.owners.iter().find_map(|owner| {
            let spec = owner.loop_spec()?;
            let supported = (owner.kind == OwnerKind::Graph || matches!(owner.kind, crate::plan::OwnerKind::Node(slot)
                    if matches!(self.plan.nodes[slot.index()].execution, NodeExecution::Operator)
                        || self.plan.nodes[slot.index()].connector().and_then(crate::plan::ConnectorSpec::call_semantics).is_some()))
                && (spec.fuel.is_some() || spec.proof.is_some());
            (!supported).then(|| VmError::UnsupportedExtension {
                owner: owner.path.clone(),
                extension: node::Extension::Loop,
            })
        })
    }

    /// Boots the machine over a VALIDATED plan — once. The machine
    /// dereferences the plan's tables without checking; the type is
    /// what makes that safe, so no raw plan is accepted here. This
    /// is a fresh boot; [`Self::recover_from_journal`] reconstructs the
    /// admitted recovery profiles without executing a producer.
    #[must_use]
    pub fn boot(plan: &'plan ValidatedExecutionPlan, graph_version: GraphVersion) -> Self {
        Self::boot_at(plan, graph_version, 0)
    }

    /// Boots with already resolved deployment inputs. Personal-data policy is
    /// admitted only when the supplied privacy profile covers every legal
    /// regime carried by the plan; raw secret-classified payloads remain
    /// fail-closed because A4 resolves only reference-origin secrets.
    #[must_use]
    pub fn boot_with_deployment(
        plan: &'plan ValidatedExecutionPlan,
        graph_version: GraphVersion,
        boot: &crate::deployment::BootBindings,
    ) -> Self {
        Self::boot_at_with_deployment(plan, graph_version, 0, boot)
    }

    /// Boots at an explicit logical instant, in nanoseconds since the Unix
    /// epoch. Hosted code maps its monotonic time source here; Lab and bare
    /// metal may keep the deterministic epoch default or supply another
    /// origin. The semantic core never reads an ambient clock.
    #[must_use]
    pub fn boot_at(
        plan: &'plan ValidatedExecutionPlan,
        graph_version: GraphVersion,
        logical_time: i64,
    ) -> Self {
        Self::boot_at_inner(plan, graph_version, logical_time, None)
    }

    /// Boots at an explicit logical instant with fixed deployment inputs.
    #[must_use]
    pub fn boot_at_with_deployment(
        plan: &'plan ValidatedExecutionPlan,
        graph_version: GraphVersion,
        logical_time: i64,
        boot: &crate::deployment::BootBindings,
    ) -> Self {
        Self::boot_at_inner(plan, graph_version, logical_time, Some(boot))
    }

    fn boot_at_inner(
        plan: &'plan ValidatedExecutionPlan,
        graph_version: GraphVersion,
        logical_time: i64,
        boot: Option<&crate::deployment::BootBindings>,
    ) -> Self {
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
            unsupported_data_policy: first_unsupported_data_policy_location(plan, boot),
            missing_deployment_binding: first_missing_deployment_binding(plan, boot),
            requires_personal_retention: personal_data_requirements(plan).personal,
            expected_deployment: boot.map(|boot| boot.attestation().clone()),
            owner_chains,
            has_join_activations: plan
                .handlers
                .iter()
                .any(|handler| matches!(&handler.activation, node::HandlerActivation::Join { .. })),
            has_loops: plan.owners.iter().any(|owner| {
                owner
                    .extensions
                    .iter()
                    .any(|aspect| aspect.extension() == node::Extension::Loop)
            }),
            instance_commands: plan
                .routes
                .iter()
                .any(|rule| match &rule.selection {
                    crate::plan::RouteSelection::All(set) => set.iter().any(|destination| {
                        matches!(destination, crate::plan::RouteDestination::InstanceStop(_))
                    }),
                    crate::plan::RouteSelection::Cases(arms) => arms
                        .iter()
                        .flat_map(|arm| &arm.destinations)
                        .any(|destination| {
                            matches!(destination, crate::plan::RouteDestination::InstanceStop(_))
                        }),
                })
                .then(VecDeque::new),
            graph_loops: plan
                .owners
                .iter()
                .any(|owner| owner.kind == OwnerKind::Graph && owner.loop_spec().is_some())
                .then(|| Box::new(graph_loop::GraphLoops::default())),
            has_lifetime_deadlines: plan.owners.iter().any(|owner| {
                matches!(
                    owner.instance.lifetime,
                    InstanceLifetime::Idle(_)
                        | InstanceLifetime::Fixed(_)
                        | InstanceLifetime::Tumbling(_)
                )
            }),
            has_single_pass: plan
                .owners
                .iter()
                .any(|owner| matches!(owner.instance.lifetime, InstanceLifetime::SinglePass)),
            graph_version,
            scheduler: Scheduler::new(WORKER_MAILBOX_CAPACITY),
            state: InstanceState::new(),
            dispatcher: Dispatcher::new(),
            pending: BTreeMap::new(),
            inbound_calls: BTreeMap::new(),
            settled_inbound_calls: VecDeque::new(),
            sessions: BTreeMap::new(),
            session_writes: BTreeMap::new(),
            settled_sessions: VecDeque::new(),
            instances: BTreeMap::new(),
            waiting: BTreeMap::new(),
            closing: BTreeSet::new(),
            lifecycle: VecDeque::new(),
            loop_manager: None,
            settled: VecDeque::new(),
            buffered_pump: PumpReport::default(),
            now: logical_time,
            outputs: Vec::new(),
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

    /// Retains terminal observations for admitted inbound Calls after judging
    /// them against the resolved Connector output table. No journal mutation
    /// occurs here; [`Self::step`] commits each accepted completion exactly
    /// once as an ordinary Connector transaction.
    #[must_use]
    pub fn queue_inbound_call_events(
        &mut self,
        events: impl IntoIterator<Item = InboundCallEvent>,
    ) -> InboundCallPumpReport {
        let gate = PlanGate { plan: self.plan };
        let mut report = InboundCallPumpReport::default();
        for event in events {
            let Some(pending) = self.inbound_calls.get(&event.call_ref) else {
                report.uncorrelated.push(event);
                continue;
            };
            if self
                .settled_inbound_calls
                .iter()
                .any(|queued| queued.call_ref == event.call_ref)
            {
                report.discarded.push(event);
                continue;
            }
            if pending
                .response
                .as_ref()
                .is_some_and(|response| !response.started)
                && !pending.timeout.is_some_and(|timeout| timeout.started)
            {
                report.rejected.push((
                    event,
                    String::from(
                        "the inbound Call Driver reported a response terminal before accepting the journaled response",
                    ),
                ));
                continue;
            }
            let connector = &self.plan.nodes[pending.slot.index()].path;
            if let Err(reason) = gate.admit(connector, &event.completion) {
                report.rejected.push((event, reason));
                continue;
            }
            self.settled_inbound_calls.push_back(event);
            report.queued = report.queued.saturating_add(1);
        }
        report
    }

    /// Returns graph responses that have committed and are waiting for their
    /// owning inbound Call Driver. The order is the canonical Call-reference
    /// order; repeated reads return the same envelopes until the Driver
    /// confirms ownership with [`Self::mark_inbound_call_response_started`].
    #[must_use]
    pub fn ready_inbound_call_responses(&self) -> Vec<InboundCallResponse> {
        self.inbound_calls
            .iter()
            .filter_map(|(call_ref, pending)| {
                if pending.timeout.is_some_and(|timeout| timeout.requested) {
                    return None;
                }
                let response = pending
                    .response
                    .as_ref()
                    .filter(|response| !response.started)?;
                Some(InboundCallResponse {
                    call_ref: *call_ref,
                    connector_path: self.plan.nodes[pending.slot.index()].path.clone(),
                    response: response.delivery.envelope.clone(),
                })
            })
            .collect()
    }

    /// Marks the exact retained response as Driver-owned after a successful
    /// non-blocking start. A stale or crossed acknowledgement changes
    /// nothing and returns `false`.
    pub fn mark_inbound_call_response_started(&mut self, response: &InboundCallResponse) -> bool {
        let Some(pending) = self.inbound_calls.get_mut(&response.call_ref) else {
            return false;
        };
        if self.plan.nodes[pending.slot.index()].path != response.connector_path {
            return false;
        }
        let Some(retained) = &mut pending.response else {
            return false;
        };
        if retained.started
            || retained.delivery.envelope.meta().sys().id() != response.response.meta().sys().id()
        {
            return false;
        }
        retained.started = true;
        true
    }

    /// Returns scheduler-owned inbound request expiries not yet accepted by
    /// their Driver. Stable Call-reference order makes repeated bounded pump
    /// rounds deterministic.
    #[must_use]
    pub fn ready_inbound_call_timeouts(&self) -> Vec<InboundCallTimeout> {
        self.inbound_calls
            .iter()
            .filter_map(|(call_ref, pending)| {
                pending
                    .timeout
                    .filter(|timeout| timeout.requested && !timeout.started)?;
                Some(InboundCallTimeout {
                    call_ref: *call_ref,
                    connector_path: self.plan.nodes[pending.slot.index()].path.clone(),
                })
            })
            .collect()
    }

    /// Confirms that the exact scheduler expiry has transferred to its
    /// Driver. Stale and crossed acknowledgements are harmless refusals.
    pub fn mark_inbound_call_timeout_started(&mut self, timeout: &InboundCallTimeout) -> bool {
        let Some(pending) = self.inbound_calls.get_mut(&timeout.call_ref) else {
            return false;
        };
        if self.plan.nodes[pending.slot.index()].path != timeout.connector_path {
            return false;
        }
        let Some(retained) = &mut pending.timeout else {
            return false;
        };
        if !retained.requested || retained.started {
            return false;
        }
        retained.started = true;
        true
    }

    fn inbound_call_timeout(
        &self,
        source: Slot,
        instance: &InstanceRef,
        candidates: &BTreeMap<InstanceRef, InstanceCandidate>,
    ) -> Result<Option<i64>, VmError> {
        let planned = &self.plan.nodes[source.index()];
        let spec = planned.connector().ok_or_else(|| VmError::PlanInvariant {
            node: planned.path.clone(),
            reason: "an inbound Call source lost its Contract",
        })?;
        match spec.attribute("mode") {
            Some(AttrSetting::Word(mode)) if mode == "ack" => Ok(None),
            Some(AttrSetting::Word(mode)) if mode == "respond" => {
                let resolved = spec.attribute("timeout").and_then(|setting| match setting {
                    AttrSetting::Value(crate::value::Value::Duration(value)) => Some(*value),
                    AttrSetting::Expr { .. } => candidates
                        .get(instance)
                        .map(|candidate| &candidate.settings)
                        .or_else(|| {
                            self.instances
                                .get(instance)
                                .map(|instance| &instance.settings)
                        })
                        .or_else(|| {
                            self.waiting
                                .get(instance)
                                .map(|waiting| &waiting.candidate.settings)
                        })
                        .and_then(|settings| settings.get("timeout"))
                        .and_then(|value| match value {
                            crate::value::Value::Duration(value) => Some(*value),
                            _ => None,
                        }),
                    AttrSetting::Value(_) | AttrSetting::Word(_) => None,
                });
                resolved
                    .filter(|value| *value > 0)
                    .map(Some)
                    .ok_or_else(|| VmError::PlanInvariant {
                        node: planned.path.clone(),
                        reason: "a respond-mode inbound Call has no positive resolved timeout",
                    })
            }
            _ => Err(VmError::PlanInvariant {
                node: planned.path.clone(),
                reason: "an inbound Call source has no executable mode",
            }),
        }
    }

    fn inject_submission(
        &mut self,
        journal: &mut dyn Journal,
        submission: InboundSubmission,
        session: Option<InboundSession>,
        connector_path: Option<node::Path>,
    ) -> Result<InboundAcceptance, VmError> {
        self.preflight(journal)?;
        let session_admission = session;
        // Boundary injection and Connector-origin roots are distinct causal
        // sources. The caller chooses the category; the validated plan proves
        // the exact endpoint before the journal reserves a sequence.
        let mut inbound_source = None;
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
                    (spec.roles.has_inbound() && output).then_some((index, spec.exchange))
                });
            let Some((source, exchange)) = source else {
                return Err(VmError::UnknownInboundSource {
                    connector: connector.clone(),
                    port: submission.port.clone(),
                });
            };
            let source =
                Slot::new(
                    u32::try_from(source).map_err(|_| VmError::UnknownInboundSource {
                        connector: connector.clone(),
                        port: submission.port.clone(),
                    })?,
                );
            inbound_source = Some((source, exchange));
            EndpointRef {
                node: Some(source),
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
        let mut inbound = match (connector_path, session_admission) {
            (Some(_), session) => journal.prepare_connector_inbound(submission, session),
            (None, Some(session)) => journal.prepare_session_inbound(submission, session),
            (None, None) => journal.prepare_inbound(submission),
        }
        .map_err(VmError::Journal)?;
        let mut candidates = BTreeMap::new();
        // A Call root opens an obligation on the exact source Connector
        // generation. Select that instance in the same pure preparation as
        // downstream routing; the accepting INBOUND record seals both before
        // either bundle or delivery becomes visible.
        let source_target = match inbound_source {
            Some((source, ConnectorExchange::Call)) => {
                let owner = self.plan.nodes[source.index()].owner;
                let scope = self.owner_chain(owner)?.first().copied().ok_or_else(|| {
                    VmError::PlanInvariant {
                        node: self.plan.nodes[source.index()].path.clone(),
                        reason: "an inbound Call source has no owner chain",
                    }
                })?;
                let target = self.prepare_target(
                    journal,
                    scope,
                    None,
                    source,
                    inbound.envelope(),
                    &mut candidates,
                )?;
                let held = self
                    .inbound_calls
                    .values()
                    .filter(|pending| {
                        pending.slot == source && pending.instance == target.reference
                    })
                    .count();
                if held >= WORKER_MAILBOX_CAPACITY {
                    return Err(VmError::MailboxFull {
                        node: self.plan.nodes[source.index()].path.clone(),
                    });
                }
                let timeout = self.inbound_call_timeout(source, &target.reference, &candidates)?;
                let call_ref = InboundCallRef::from_root(inbound.envelope().meta().sys().id());
                let timers = timeout
                    .map(|_| {
                        vec![(
                            crate::journal::timer_kind::INBOUND_CALL_TIMEOUT,
                            call_ref.id(),
                        )]
                    })
                    .unwrap_or_default();
                inbound = inbound
                    .into_call(target.reference.clone(), timers)
                    .map_err(VmError::Journal)?;
                Some((source, target, timeout))
            }
            Some((_, ConnectorExchange::Event | ConnectorExchange::Session)) | None => None,
        };
        // Session roots obey the same source-instance rule as Calls. The
        // opening selects and installs the Connector owner exactly once;
        // every later message reuses that journal-derived logical Session
        // instead of re-evaluating a key from mutable metadata.
        let session_target = match (inbound_source, session_admission) {
            (Some((source, ConnectorExchange::Session)), Some(InboundSession::Open)) => {
                if !matches!(
                    self.plan.nodes[source.index()]
                        .connector()
                        .map(|spec| spec.effect),
                    Some(node::Effect::None)
                ) {
                    return Err(VmError::PlanInvariant {
                        node: self.plan.nodes[source.index()].path.clone(),
                        reason: "an effectful Session Connector cannot accept a peer opening",
                    });
                }
                let owner = self.plan.nodes[source.index()].owner;
                let scope = self.owner_chain(owner)?.first().copied().ok_or_else(|| {
                    VmError::PlanInvariant {
                        node: self.plan.nodes[source.index()].path.clone(),
                        reason: "an inbound Session source has no owner chain",
                    }
                })?;
                let target = self.prepare_target(
                    journal,
                    scope,
                    None,
                    source,
                    inbound.envelope(),
                    &mut candidates,
                )?;
                let held = self
                    .sessions
                    .values()
                    .filter(|pending| {
                        pending.slot == source && pending.instance == target.reference
                    })
                    .count();
                if held >= WORKER_MAILBOX_CAPACITY {
                    return Err(VmError::MailboxFull {
                        node: self.plan.nodes[source.index()].path.clone(),
                    });
                }
                Some((source, target, true))
            }
            (
                Some((source, ConnectorExchange::Session)),
                Some(InboundSession::Message(session_ref)),
            ) => {
                let Some(pending) = self.sessions.get(&session_ref) else {
                    return Err(VmError::PlanInvariant {
                        node: self.plan.nodes[source.index()].path.clone(),
                        reason: "an inbound Session message has no installed logical session",
                    });
                };
                if pending.slot != source || !pending.opened {
                    return Err(VmError::PlanInvariant {
                        node: self.plan.nodes[source.index()].path.clone(),
                        reason: "an inbound Session message crossed its bound source",
                    });
                }
                Some((
                    source,
                    PreparedTarget {
                        reference: pending.instance.clone(),
                        scope: pending.instance.clone(),
                        chain: Vec::new(),
                        admitted: Vec::new(),
                    },
                    false,
                ))
            }
            (Some((source, ConnectorExchange::Session)), None) => {
                return Err(VmError::PlanInvariant {
                    node: self.plan.nodes[source.index()].path.clone(),
                    reason: "a Session source arrived without Session framing",
                });
            }
            (Some((source, ConnectorExchange::Event | ConnectorExchange::Call)), Some(_)) => {
                return Err(VmError::PlanInvariant {
                    node: self.plan.nodes[source.index()].path.clone(),
                    reason: "Session framing reached a non-Session source",
                });
            }
            (Some((_, ConnectorExchange::Event | ConnectorExchange::Call)), None) => None,
            (None, _) => None,
        };
        let source_instance = source_target
            .as_ref()
            .map(|(_, target, _)| &target.reference)
            .or_else(|| {
                session_target
                    .as_ref()
                    .map(|(_, target, _)| &target.reference)
            });
        let opening_only = matches!(
            (inbound_source, session_admission),
            (
                Some((_, ConnectorExchange::Session)),
                Some(InboundSession::Open)
            )
        );
        let prepared = if opening_only {
            // The upgrade commit creates the logical Session but is not a
            // peer message. WebSocket sheets expose complete messages on
            // `out`; routing the handshake's causal seed there would invent
            // an empty application payload.
            Vec::new()
        } else {
            self.prepare_routes(
                journal,
                &from,
                // A Session message already owns an installed source instance.
                // Preserve its ancestor scope: its new external metadata is
                // not another admission into a keyed containing Graph. Other
                // source bundles are still pre-commit candidates and share the
                // candidate map rather than consulting live state.
                matches!(session_admission, Some(InboundSession::Message(_)))
                    .then_some(source_instance)
                    .flatten(),
                inbound.envelope(),
                &mut candidates,
            )?
        };
        self.check_room_for_routes(core::iter::once(prepared.as_slice()))?;
        let commands = if opening_only {
            Vec::new()
        } else {
            self.prepare_instance_commands(
                journal,
                &self.graph_endpoint(&from),
                source_instance,
                inbound.envelope(),
            )?
        };
        self.check_instance_command_room(journal, &commands)?;
        if !commands.is_empty() {
            inbound = inbound.with_instance_commands(crate::journal::instance_stop::Packet {
                produced: commands.clone(),
                consumed: None,
            });
        }
        let recorded_routes = self.inbound_route_targets(&prepared);
        let mut activity = BTreeMap::new();
        let mut arms = Vec::new();
        for target in source_target
            .as_ref()
            .map(|(_, target, _)| target)
            .into_iter()
            .chain(
                session_target
                    .as_ref()
                    .filter(|(_, _, opening)| *opening)
                    .map(|(_, target, _)| target),
            )
        {
            arms.extend(
                self.admission_deadlines(target, &candidates, inbound.envelope(), &mut activity)
                    .into_iter()
                    .map(|(arm, _)| arm),
            );
        }
        if let Some((_, _, Some(timeout))) = &source_target {
            arms.push(crate::journal::timing::TimerArm {
                kind: crate::journal::timer_kind::INBOUND_CALL_TIMEOUT,
                reference: InboundCallRef::from_root(inbound.envelope().meta().sys().id()).id(),
                due_at: self.now.saturating_add(*timeout),
            });
        }
        arms.extend(self.route_timer_arms_with_activity(
            &prepared,
            &candidates,
            inbound.envelope(),
            &mut activity,
        ));
        if !arms.is_empty() {
            inbound = inbound.with_timer_schedule(Self::timer_schedule(self.now, arms)?);
        }
        let committed = journal
            .commit_inbound(inbound, recorded_routes)
            .map_err(VmError::Journal)?;
        self.enqueue_instance_commands(commands);
        let id = committed.envelope.meta().sys().id();
        let exchange = committed
            .record
            .exchange
            .as_deref()
            .map(|exchange| match exchange {
                crate::journal::InboundExchangeRecord::Call(call) => {
                    crate::boundary::InboundExchangeRef::Call(call.reference)
                }
                crate::journal::InboundExchangeRecord::Session(session) => {
                    let reference = match session {
                        crate::journal::SessionIngressRecord::Open(reference)
                        | crate::journal::SessionIngressRecord::Message(reference) => *reference,
                    };
                    crate::boundary::InboundExchangeRef::Session(reference)
                }
            });
        if let Some((source, target, timeout)) = &source_target {
            self.install_target(target, &candidates, &committed.envelope);
            let call_ref = InboundCallRef::from_root(id);
            if let Some(timeout) = timeout {
                self.scheduler.arm(
                    self.now.saturating_add(*timeout),
                    DatedPayload::InboundCallBudget {
                        call_ref,
                        instance: target.reference.clone(),
                    },
                );
            }
            self.inbound_calls.insert(
                call_ref,
                PendingInboundCall {
                    slot: *source,
                    request: committed.envelope.clone(),
                    instance: target.reference.clone(),
                    response: None,
                    timeout: timeout.map(|_| PendingInboundCallTimeout {
                        requested: false,
                        started: false,
                    }),
                },
            );
        }
        if let Some((source, target, opening)) = &session_target
            && *opening
        {
            self.install_target(target, &candidates, &committed.envelope);
            let Some(crate::boundary::InboundExchangeRef::Session(session_ref)) = exchange else {
                return Err(VmError::PlanInvariant {
                    node: self.plan.nodes[source.index()].path.clone(),
                    reason: "a committed Session opening lost its logical reference",
                });
            };
            self.sessions.insert(
                session_ref,
                PendingSession {
                    slot: *source,
                    instance: target.reference.clone(),
                    opening: committed.envelope.clone(),
                    open_intent: None,
                    opening_delivery: None,
                    opened: true,
                    opening_started: true,
                    opening_claimed: false,
                    close_started: false,
                },
            );
        }
        self.route_prepared(prepared, &candidates, &committed.envelope, source_instance);
        Ok(InboundAcceptance {
            envelope: id,
            exchange,
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
        self.preflight(journal)?;
        if self.step_instance_commands(journal)? {
            return Ok(true);
        }
        if let Some((instance, cause)) = self
            .single_pass_close()
            .map(|(reference, cause)| (reference.clone(), cause.clone()))
        {
            self.request_close(journal, &instance, cause, None)?;
            return Ok(true);
        }
        self.progress_closures();
        if !self.lifecycle.is_empty() {
            self.fire_lifecycle_next(journal)?;
            return Ok(true);
        }
        if self.graph_loops.is_some() && self.open_graph_loop(journal)? {
            return Ok(true);
        }
        if self.has_loops && self.step_loop_manager(journal)? {
            return Ok(true);
        }
        let inbound_ready =
            self.settled_inbound_calls
                .iter()
                .enumerate()
                .find_map(|(index, event)| {
                    let pending = self.inbound_calls.get(&event.call_ref)?;
                    self.instances
                        .contains_key(&pending.instance)
                        .then(|| (index, event.clone(), pending.clone()))
                });
        if let Some((index, event, pending)) = inbound_ready {
            self.settle_inbound_call_next(journal, index, event, pending)?;
            return Ok(true);
        }
        if let Some(index) = self.settled_sessions.iter().position(|settled| {
            self.sessions.get(&settled.session).is_some_and(|session| {
                self.instances.contains_key(&session.instance)
                    && !self
                        .session_writes
                        .values()
                        .any(|write| write.intent.session_ref == settled.session)
            })
        }) {
            self.settle_session_next(journal, bindings, index)?;
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
            if self.has_loops && self.open_loop_application(journal, slot, &instance, &delivery)? {
                return Ok(true);
            }
            match &self.plan.nodes[slot.index()].execution {
                NodeExecution::Operator => {
                    self.fire_operator(journal, slot, &instance, &delivery)?
                }
                NodeExecution::Connector(_) => {
                    self.fire_connector(journal, bindings, slot, &instance, &delivery)?
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
            match &self.plan.nodes[slot.index()].execution {
                NodeExecution::Connector(_) => {
                    if self.has_loops
                        && self.open_loop_application(journal, slot, &instance, &delivery)?
                    {
                        return Ok(true);
                    }
                    self.fire_connector(journal, bindings, slot, &instance, &delivery)?;
                    return Ok(true);
                }
                NodeExecution::Operator => {
                    // An ordinary FIFO head necessarily precedes any work a
                    // later delivery could make runnable. Only a join-owned
                    // head can be partial and require the wider candidate
                    // scan, so the common single-port path stays allocation
                    // free.
                    let join_owned = self.plan.handlers.iter().any(|handler| {
                        handler.owner == slot
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
                            if self.has_loops
                                && self
                                    .open_loop_application(journal, slot, &instance, &delivery)?
                            {
                                return Ok(true);
                            }
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
        self.preflight(journal)?;
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
        self.retry_session_actions(bindings)?;
        self.retry_session_closes(bindings)?;
        let settled = batch.events.len();
        let in_port =
            node::Port::new("in").unwrap_or_else(|_| unreachable!("`in` is a valid port"));
        self.settled
            .extend(batch.events.into_iter().map(|event| SettledEntry {
                event,
                trigger: node::Trigger::Port(in_port.clone()),
            }));
        let mut report = PumpReport {
            settled,
            discarded: batch.discarded,
            uncorrelated: batch.uncorrelated,
            rejected: batch.rejected,
            discarded_sessions: Vec::new(),
            uncorrelated_sessions: Vec::new(),
            rejected_sessions: Vec::new(),
        };
        let session_events = bindings
            .session_events()
            .map_err(crate::boundary::DispatchError::Driver)
            .map_err(VmError::Dispatch)?;
        for event in session_events {
            match event.clone() {
                SessionDriverEvent::Opened(session_ref) => {
                    let Some(session) = self.sessions.get_mut(&session_ref) else {
                        report.uncorrelated_sessions.push(event);
                        continue;
                    };
                    if session.opened {
                        report.discarded_sessions.push(event);
                        continue;
                    }
                    match journal
                        .confirm_session_open(&session_ref)
                        .map_err(VmError::Journal)?
                    {
                        crate::journal::CloseVerdict::AlreadyClosed => {
                            report.discarded_sessions.push(event);
                        }
                        crate::journal::CloseVerdict::Won => {
                            session.opened = true;
                            if session.opening_claimed {
                                self.scheduler
                                    .mailboxes
                                    .release(session.slot, &session.instance);
                                session.opening_claimed = false;
                            }
                            report.settled = report.settled.saturating_add(1);
                        }
                    }
                }
                SessionDriverEvent::WriteSettled {
                    session: session_ref,
                    write,
                } => {
                    if !self.sessions.contains_key(&session_ref) {
                        report.uncorrelated_sessions.push(event);
                        continue;
                    }
                    let Some(pending) = self.session_writes.get(&write) else {
                        report.discarded_sessions.push(event);
                        continue;
                    };
                    if pending.intent.session_ref != session_ref || !pending.started {
                        report.discarded_sessions.push(event);
                        continue;
                    }
                    let slot = pending.slot;
                    let instance = pending.instance.clone();
                    match journal
                        .settle_session_write(&write, false)
                        .map_err(VmError::Journal)?
                    {
                        crate::journal::CloseVerdict::AlreadyClosed => {
                            report.discarded_sessions.push(event);
                        }
                        crate::journal::CloseVerdict::Won => {
                            self.session_writes.remove(&write);
                            let held = self
                                .session_writes
                                .values()
                                .filter(|pending| {
                                    pending.slot == slot && pending.instance == instance
                                })
                                .count();
                            if held < WORKER_MAILBOX_CAPACITY {
                                self.scheduler.mailboxes.release(slot, &instance);
                            }
                            report.settled = report.settled.saturating_add(1);
                        }
                    }
                }
                SessionDriverEvent::Exited {
                    session: session_ref,
                    mut exit,
                } => {
                    let Some(session) = self.sessions.get(&session_ref) else {
                        report.uncorrelated_sessions.push(event);
                        continue;
                    };
                    if self
                        .settled_sessions
                        .iter()
                        .any(|settled| settled.session == session_ref)
                    {
                        report.discarded_sessions.push(event);
                        continue;
                    }
                    exit.ext.insert(
                        String::from("session_id"),
                        crate::envelope::MetaValue::Scalar(crate::envelope::Scalar::Text(
                            session_ref.to_string(),
                        )),
                    );
                    let completion = Completion::try_new(vec![exit]).map_err(|error| {
                        VmError::PlanInvariant {
                            node: self.plan.nodes[session.slot.index()].path.clone(),
                            reason: match error {
                                crate::boundary::CompletionError::MissingExit => {
                                    "a Session Driver omitted its exit"
                                }
                                crate::boundary::CompletionError::MultipleExits(_) => {
                                    "a Session Driver repeated its exit"
                                }
                                crate::boundary::CompletionError::OutputAfterExit => {
                                    "a Session Driver emitted after exit"
                                }
                            },
                        }
                    })?;
                    let connector = &self.plan.nodes[session.slot.index()].path;
                    if let Err(reason) = gate.admit(connector, &completion) {
                        report.rejected_sessions.push((event, reason));
                        continue;
                    }
                    let writes = self
                        .session_writes
                        .iter()
                        .filter_map(|(reference, write)| {
                            (write.intent.session_ref == session_ref).then_some(*reference)
                        })
                        .collect::<Vec<_>>();
                    for write in writes {
                        match journal
                            .settle_session_write(&write, true)
                            .map_err(VmError::Journal)?
                        {
                            crate::journal::CloseVerdict::Won
                            | crate::journal::CloseVerdict::AlreadyClosed => {
                                self.session_writes.remove(&write);
                            }
                        }
                    }
                    self.settled_sessions.push_back(SettledSession {
                        session: session_ref,
                        completion,
                        terminal_delivery: None,
                    });
                    report.settled = report.settled.saturating_add(1);
                }
            }
        }
        self.progress_closures();
        Ok(report)
    }

    /// Re-offers bounded Session actions only from the boundary pump. A
    /// `Pending` result therefore cannot leave the same mailbox head runnable
    /// and spin the deterministic transaction drain.
    fn retry_session_actions<B: OutboundConnectorBindings + ?Sized>(
        &mut self,
        bindings: &mut B,
    ) -> Result<(), VmError> {
        let openings = self
            .sessions
            .iter()
            .filter_map(|(reference, session)| {
                (!session.opening_started
                    && session.open_intent.is_some()
                    && self
                        .instances
                        .get(&session.instance)
                        .is_some_and(|instance| {
                            matches!(
                                instance.phase,
                                InstancePhase::Open
                                    | InstancePhase::Closing {
                                        stage: ClosingStage::Draining,
                                        ..
                                    }
                            )
                        })
                    && !self
                        .settled_sessions
                        .iter()
                        .any(|settled| settled.session == *reference))
                .then_some(*reference)
            })
            .collect::<Vec<_>>();
        for reference in openings {
            let session =
                self.sessions
                    .get(&reference)
                    .cloned()
                    .ok_or_else(|| VmError::PlanInvariant {
                        node: node::Path::new(["session"])
                            .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                        reason: "a pending Session opening disappeared before retry",
                    })?;
            let intent = session
                .open_intent
                .as_ref()
                .ok_or_else(|| VmError::PlanInvariant {
                    node: self.plan.nodes[session.slot.index()].path.clone(),
                    reason: "a retryable Session opening lost its sealed intent",
                })?;
            if bindings
                .open_session(intent, &session.opening)
                .map_err(crate::boundary::DispatchError::Driver)
                .map_err(VmError::Dispatch)?
                == SessionStart::Started
                && let Some(session) = self.sessions.get_mut(&reference)
            {
                session.opening_started = true;
            }
        }

        let writes = self
            .session_writes
            .iter()
            .filter_map(|(reference, write)| {
                (!write.started
                    && self.instances.get(&write.instance).is_some_and(|instance| {
                        matches!(
                            instance.phase,
                            InstancePhase::Open
                                | InstancePhase::Closing {
                                    stage: ClosingStage::Draining,
                                    ..
                                }
                        )
                    }))
                .then_some(*reference)
            })
            .collect::<Vec<_>>();
        for reference in writes {
            let write = self
                .session_writes
                .get(&reference)
                .cloned()
                .ok_or_else(|| VmError::PlanInvariant {
                    node: node::Path::new(["session"])
                        .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                    reason: "a pending Session write disappeared before retry",
                })?;
            if bindings
                .write_session(&write.intent, &write.delivery.envelope)
                .map_err(crate::boundary::DispatchError::Driver)
                .map_err(VmError::Dispatch)?
                == SessionStart::Started
                && let Some(write) = self.session_writes.get_mut(&reference)
            {
                write.started = true;
            }
        }
        Ok(())
    }

    fn retry_session_closes<B: OutboundConnectorBindings + ?Sized>(
        &mut self,
        bindings: &mut B,
    ) -> Result<(), VmError> {
        let pending: Vec<SessionRef> = self
            .sessions
            .iter()
            .filter(|(_, session)| {
                !session.close_started
                    && self
                        .instances
                        .get(&session.instance)
                        .is_some_and(|instance| {
                            matches!(
                                instance.phase,
                                InstancePhase::Closing {
                                    stage: ClosingStage::Settling,
                                    ..
                                }
                            )
                        })
            })
            .map(|(reference, _)| *reference)
            .collect();
        for reference in pending {
            let session =
                self.sessions
                    .get(&reference)
                    .cloned()
                    .ok_or_else(|| VmError::PlanInvariant {
                        node: node::Path::new(["session"])
                            .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                        reason: "a pending Session close lost its logical session",
                    })?;
            let connector = self.plan.nodes[session.slot.index()].path.clone();
            if bindings
                .close_session(&connector, reference)
                .map_err(crate::boundary::DispatchError::Driver)
                .map_err(VmError::Dispatch)?
                == SessionStart::Started
                && let Some(session) = self.sessions.get_mut(&reference)
            {
                session.close_started = true;
            }
        }
        Ok(())
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
        let mut advance = TimeAdvance::new(target);
        while advance.step(self, journal, bindings)? {}
        Ok(())
    }

    fn peek_due(&self, target: i64) -> Option<((i64, u64), DatedPayload)> {
        self.scheduler.peek_due(target, |payload| {
            !matches!(payload, DatedPayload::InstanceLifetime { instance, .. } if self.waiting.contains_key(instance))
        })
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
            DatedPayload::InboundCallBudget { call_ref, instance } => {
                if self
                    .settled_inbound_calls
                    .iter()
                    .any(|event| event.call_ref == call_ref)
                {
                    return Ok(());
                }
                let Some(pending) = self.inbound_calls.get_mut(&call_ref) else {
                    return Ok(());
                };
                if pending.instance != instance {
                    return Ok(());
                }
                let Some(timeout) = &mut pending.timeout else {
                    return Err(VmError::PlanInvariant {
                        node: self.plan.nodes[pending.slot.index()].path.clone(),
                        reason: "an ack-only inbound Call carried a request deadline",
                    });
                };
                timeout.requested = true;
                return Ok(());
            }
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
            DatedPayload::LoopDelay { .. } => {
                // The wakeup only dates eligibility. The manager rechecks the
                // committed run, generation and deadline before any admission.
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
        let slot = self.slot_of(&connector);
        if self.plan.nodes[slot.index()]
            .connector()
            .is_some_and(|spec| spec.exchange == ConnectorExchange::Session)
        {
            return self
                .expire_session_handshake(journal, bindings, slot, instance, &edge, delivery);
        }
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
            let NodeExecution::Connector(spec) = &planned.execution else {
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
                        logical_time: self.now,
                    })
                }
            }
        };
        match queued_intent {
            Err(completion) => {
                self.admit_expiring_call_loop(journal, slot, &instance, &edge, delivery)?;
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
                    let executor = HandlerExecutor::at(self.plan, self.graph_version, self.now);
                    let trigger = node::Trigger::Port(queued.port.clone());
                    let mut outcome = executor
                        .connector_completion(slot, &instance, queued, &trigger, &completion)
                        .map_err(VmError::Fault)?;
                    self.prepare_call_completion(slot, &instance, queued, &mut outcome.record)?;
                    outcome
                };
                let prepared =
                    self.validate_and_prepare_emissions(journal, slot, &instance, &outcome.record)?;
                self.retain_emission_routes(&mut outcome.record, &prepared)?;
                journal
                    .commit(outcome.record.clone())
                    .map_err(VmError::Journal)?;
                if outcome.record.loop_protocol().is_some() {
                    self.refresh_loop_manager(journal)?;
                }
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
                self.admit_expiring_call_loop(journal, slot, &instance, &edge, delivery)?;
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

    fn expire_session_handshake<B: OutboundConnectorBindings + ?Sized>(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut B,
        slot: Slot,
        instance: InstanceRef,
        edge: &str,
        delivery: Id,
    ) -> Result<(), VmError> {
        let connector = self.plan.nodes[slot.index()].path.clone();
        if let Some(session_ref) = self
            .sessions
            .iter()
            .find(|(_, session)| {
                session.slot == slot
                    && session.instance == instance
                    && session.opening.meta().sys().id() == delivery
            })
            .map(|(reference, _)| *reference)
        {
            let session =
                self.sessions
                    .get(&session_ref)
                    .cloned()
                    .ok_or_else(|| VmError::PlanInvariant {
                        node: connector.clone(),
                        reason: "a due Session handshake lost its logical session",
                    })?;
            if session.opened
                || self
                    .settled_sessions
                    .iter()
                    .any(|settled| settled.session == session_ref)
            {
                return Ok(());
            }
            let completion = bindings
                .scheduler_session_completion(&connector, SchedulerCompletion::TimeoutExpired)
                .map_err(crate::boundary::DispatchError::Driver)
                .map_err(VmError::Dispatch)?;
            if session.opening_started {
                let _ = bindings
                    .close_session(&connector, session_ref)
                    .map_err(crate::boundary::DispatchError::Driver)
                    .map_err(VmError::Dispatch)?;
            } else if !session.opening_claimed {
                let retained = self
                    .scheduler
                    .mailboxes
                    .remove_by_ref(slot, &instance, edge, delivery)
                    .ok_or_else(|| VmError::PlanInvariant {
                        node: connector.clone(),
                        reason: "a due pending Session opening lost its mailbox delivery",
                    })?;
                if let Some(session) = self.sessions.get_mut(&session_ref) {
                    session.opening_delivery = Some(retained);
                }
            }
            self.settled_sessions.push_back(SettledSession {
                session: session_ref,
                completion,
                terminal_delivery: None,
            });
            return Ok(());
        }
        if self
            .sessions
            .values()
            .any(|session| session.slot == slot && session.instance == instance)
        {
            // Every routed client delivery carries the statically derivable
            // handshake timer. Once this instance owns an opening, later
            // message timers are intentionally stale rather than a second
            // opening authority.
            return Ok(());
        }

        let Some(queued) = self
            .scheduler
            .mailboxes
            .find_by_ref(slot, &instance, edge, delivery)
            .cloned()
        else {
            return Ok(());
        };
        let spec =
            self.plan.nodes[slot.index()]
                .connector()
                .ok_or_else(|| VmError::PlanInvariant {
                    node: connector.clone(),
                    reason: "a Session handshake deadline names a non-connector",
                })?;
        let settings =
            self.resolve_invocation_settings(&instance, spec, &connector, &queued.envelope)?;
        let preparation = bindings
            .prepare_session_open(&connector, queued.envelope.data(), &settings)
            .map_err(crate::boundary::DispatchError::Driver)
            .map_err(VmError::Dispatch)?;
        let prepared = match preparation {
            RequestPreparation::Rejected(completion) => {
                return self.commit_pre_session_refusal(
                    journal,
                    slot,
                    &instance,
                    &queued,
                    &completion,
                );
            }
            RequestPreparation::Prepared(prepared) => prepared,
        };
        let session_ref = ids::outbound_session_ref(
            &connector,
            queued.envelope.meta().sys().id(),
            prepared.digest(),
        );
        let intent = SessionOpenIntent {
            session_ref,
            emitting_id: queued.envelope.meta().sys().id(),
            connector_path: connector.clone(),
            prepared,
        };
        journal
            .open_outbound_session(intent)
            .map_err(VmError::Journal)?;
        let retained = self
            .scheduler
            .mailboxes
            .remove_by_ref(slot, &instance, edge, delivery)
            .ok_or_else(|| VmError::PlanInvariant {
                node: connector.clone(),
                reason: "a due queued Session opening disappeared after its WAL fact",
            })?;
        self.sessions.insert(
            session_ref,
            PendingSession {
                slot,
                instance,
                opening: retained.envelope.clone(),
                open_intent: None,
                opening_delivery: Some(retained),
                opened: false,
                opening_started: false,
                opening_claimed: false,
                close_started: false,
            },
        );
        let completion = bindings
            .scheduler_session_completion(&connector, SchedulerCompletion::TimeoutExpired)
            .map_err(crate::boundary::DispatchError::Driver)
            .map_err(VmError::Dispatch)?;
        self.settled_sessions.push_back(SettledSession {
            session: session_ref,
            completion,
            terminal_delivery: None,
        });
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
        self.request_close(journal, &reference, cause, None)?;
        self.progress_closures();
        Ok(())
    }

    /// The machine's logical instant, in nanoseconds since the Unix epoch.
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

    /// Resolved data-policy table of one graph-boundary port.
    ///
    /// Presentation layers use this verdict before formatting an output. An
    /// unknown port returns `None`; callers must not recover policy from the
    /// spelling or the payload shape.
    #[must_use]
    pub fn boundary_annotations(
        &self,
        port: &node::Port,
    ) -> Option<&crate::schema_annotation::AnnotationTable> {
        self.plan
            .resolved_ports()
            .boundary()
            .iter()
            .find(|resolved| resolved.port() == port)
            .map(crate::plan::ResolvedPort::annotations)
    }

    /// Every delivery the machine made, in arrival order — the
    /// observation log expectations read windows of.
    #[must_use]
    pub fn observations(&self) -> &[Observed] {
        &self.observed
    }

    /// Clones one read-only debugger view without changing scheduler state.
    #[must_use]
    pub fn inspection(&self) -> VmInspection {
        let instances = self
            .instances
            .iter()
            .map(|(reference, instance)| {
                let owner = &self.plan.owners[instance.owner.index()];
                let (node, state) = match owner.kind {
                    OwnerKind::Graph => (None, BTreeMap::new()),
                    OwnerKind::Node(slot) => (
                        Some(self.plan.nodes[slot.index()].path.clone()),
                        self.state.cell_unchecked(self.plan.plan(), slot, reference),
                    ),
                };
                let phase = match instance.phase {
                    InstancePhase::Open => InstancePhaseInspection::Open,
                    InstancePhase::Closing {
                        stage: ClosingStage::Draining,
                        ..
                    } => InstancePhaseInspection::Draining,
                    InstancePhase::Closing {
                        stage: ClosingStage::Settling,
                        ..
                    } => InstancePhaseInspection::Settling,
                    InstancePhase::Stopping { .. } => InstancePhaseInspection::Stopping,
                };
                InstanceInspection {
                    reference: reference.clone(),
                    owner: owner.path.clone(),
                    phase,
                    node,
                    state,
                }
            })
            .collect();
        let mut calls =
            Vec::with_capacity(self.pending.len().saturating_add(self.inbound_calls.len()));
        calls.extend(self.pending.iter().map(|(reference, pending)| {
            CallInspection {
                reference: CallReferenceInspection::Outbound(*reference),
                direction: CallDirectionInspection::Outbound,
                connector: self.plan.nodes[pending.slot.index()].path.clone(),
                instance: pending.instance.clone(),
                completion_retained: self
                    .settled
                    .iter()
                    .any(|entry| entry.event.call_ref == *reference),
            }
        }));
        calls.extend(self.inbound_calls.iter().map(|(reference, pending)| {
            CallInspection {
                reference: CallReferenceInspection::Inbound(*reference),
                direction: CallDirectionInspection::Inbound,
                connector: self.plan.nodes[pending.slot.index()].path.clone(),
                instance: pending.instance.clone(),
                completion_retained: self
                    .settled_inbound_calls
                    .iter()
                    .any(|event| event.call_ref == *reference),
            }
        }));
        calls.sort_by_key(|call| call.reference);
        let sessions = self
            .sessions
            .iter()
            .map(|(reference, session)| SessionInspection {
                reference: *reference,
                direction: if session.open_intent.is_some() {
                    SessionDirectionInspection::Outbound
                } else {
                    SessionDirectionInspection::Inbound
                },
                connector: self.plan.nodes[session.slot.index()].path.clone(),
                instance: session.instance.clone(),
                opened: session.opened,
                pending_writes: self
                    .session_writes
                    .values()
                    .filter(|write| write.intent.session_ref == *reference)
                    .count(),
                completion_retained: self
                    .settled_sessions
                    .iter()
                    .any(|settled| settled.session == *reference),
            })
            .collect();
        let mailboxes = self
            .scheduler
            .mailboxes
            .inspection()
            .into_iter()
            .map(|(slot, instance, claimed, deliveries)| MailboxInspection {
                node: self.plan.nodes[slot.index()].path.clone(),
                instance,
                claimed,
                deliveries,
            })
            .collect();
        VmInspection {
            mailbox_capacity: self.scheduler.mailboxes.capacity,
            instances,
            mailboxes,
            calls,
            sessions,
        }
    }

    /// Outbound Calls whose terminal observation is not retained yet.
    ///
    /// A hosted pause controller uses this distinction to keep polling the
    /// world after an open, then stop before [`Self::step_with_bindings`]
    /// consumes the retained completion.
    #[must_use]
    pub fn awaiting_outbound_calls(&self) -> usize {
        self.pending
            .keys()
            .filter(|reference| {
                !self
                    .settled
                    .iter()
                    .any(|entry| entry.event.call_ref == **reference)
            })
            .count()
    }

    /// Whether the machine is stable: no activation can run, no completion
    /// waits to commit, and no call is open. A partial join is retained
    /// mailbox state, not runnable work; it is therefore quiescent until a
    /// matching arrival or its dated/count eviction makes progress possible.
    #[must_use]
    pub fn is_stable(&self) -> bool {
        if !self.lifecycle.is_empty()
            || self.instance_command_ready()
            || self.single_pass_close().is_some()
            || self.loop_manager_ready()
            || !self.settled.is_empty()
            || !self.pending.is_empty()
            || !self.settled_inbound_calls.is_empty()
            || !self.inbound_calls.is_empty()
            || !self.sessions.is_empty()
            || !self.session_writes.is_empty()
            || !self.settled_sessions.is_empty()
        {
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
            if !matches!(
                self.plan.nodes[slot.index()].execution,
                NodeExecution::Operator
            ) {
                return false;
            }
            let join_owned = self.plan.handlers.iter().any(|handler| {
                handler.owner == slot
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

    /// Whether one call to [`Self::step_with_bindings`] can select semantic
    /// work without waiting for the world or advancing logical time.
    ///
    /// Unlike [`Self::is_stable`], an open Call, Session or partial join does
    /// not by itself make work ready. Hosted stepping and the interactive Lab
    /// use this distinction to stop at a genuine transaction boundary.
    #[must_use]
    pub fn has_ready_transaction(&self) -> bool {
        if !self.lifecycle.is_empty()
            || self.instance_command_ready()
            || self.single_pass_close().is_some()
            || self.loop_manager_ready()
            || self.settled_inbound_calls.iter().any(|event| {
                self.inbound_calls
                    .get(&event.call_ref)
                    .is_some_and(|pending| self.instances.contains_key(&pending.instance))
            })
            || self.settled_sessions.iter().any(|settled| {
                self.sessions.get(&settled.session).is_some_and(|session| {
                    self.instances.contains_key(&session.instance)
                        && !self
                            .session_writes
                            .values()
                            .any(|write| write.intent.session_ref == settled.session)
                })
            })
            || self.settled.iter().any(|entry| {
                self.pending
                    .get(&entry.event.call_ref)
                    .is_some_and(|pending| self.instances.contains_key(&pending.instance))
            })
        {
            return true;
        }
        if !self.has_join_activations {
            return self
                .scheduler
                .mailboxes
                .peek_next_where(|_, reference| {
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
                .is_some();
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
            after = Some((slot, instance.clone()));
            if matches!(
                self.plan.nodes[slot.index()].execution,
                NodeExecution::Connector(_)
            ) {
                return true;
            }
            let join_owned = self.plan.handlers.iter().any(|handler| {
                handler.owner == slot
                    && matches!(
                        &handler.activation,
                        node::HandlerActivation::Join { ports, .. }
                            if ports.contains(&delivery.port)
                    )
            });
            if !join_owned || !matches!(self.operator_work(slot, &instance), Ok(None)) {
                return true;
            }
        }
        false
    }

    /// Whether calls are open against the world — work [`Self::drain`]
    /// cannot finish alone.
    #[must_use]
    pub fn has_open_obligations(&self) -> bool {
        !self.pending.is_empty()
            || !self.inbound_calls.is_empty()
            || !self.sessions.is_empty()
            || !self.session_writes.is_empty()
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
                    || self
                        .inbound_calls
                        .values()
                        .any(|pending| pending.slot == slot && pending.instance == *reference)
                    || self
                        .sessions
                        .values()
                        .any(|pending| pending.slot == slot && pending.instance == *reference)
                    || self
                        .session_writes
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

    fn single_pass_close(&self) -> Option<(&InstanceRef, &Envelope)> {
        if !self.has_single_pass {
            return None;
        }
        self.instances.iter().find_map(|(reference, instance)| {
            (matches!(instance.phase, InstancePhase::Open)
                && matches!(
                    self.plan.owners[instance.owner.index()].instance.lifetime,
                    InstanceLifetime::SinglePass
                )
                && !self.lifecycle.iter().any(|work| {
                    work.instance == *reference && work.lifecycle == node::Lifecycle::Start
                }))
            .then_some((reference, &instance.created_by))
        })
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
        command: Option<Id>,
    ) -> Result<(), VmError> {
        if !self
            .instances
            .get(reference)
            .is_some_and(|instance| matches!(instance.phase, InstancePhase::Open))
        {
            return Ok(());
        }
        let owner = self.instances[reference].owner;
        let mut record = crate::journal::TransactionRecord {
            owner_path: self.plan.owners[owner.index()].path.clone(),
            instance: reference.clone(),
            parent_instance: None,
            close_scope: Some(reference.clone()),
            exchange_exit: None,
            loop_record: None,
            logical_time: None,
            timer_schedule: None,
            instance_commands: command.map(|id| {
                Box::new(crate::journal::instance_stop::Packet {
                    produced: Vec::new(),
                    consumed: Some(id),
                })
            }),
            trigger: node::Trigger::Lifecycle(node::Lifecycle::Closing),
            cause: Some(cause.meta().sys().id()),
            consumed: vec![],
            join_inputs: vec![],
            state_delta: vec![],
            emissions: vec![],
            routes: vec![],
            timers: self.closing_timers(reference),
        };
        self.retain_timer_arms(&mut record, self.closing_timer_arms(reference))?;
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
        if let Some(queue) = self.instance_commands.as_mut() {
            queue.retain(|command| {
                self.instances
                    .get(&command.target)
                    .is_some_and(|instance| matches!(instance.phase, InstancePhase::Open))
            });
        }
    }

    fn progress_closures(&mut self) {
        if self.closing.is_empty() {
            return;
        }
        let mut stop_receivers = BTreeMap::new();
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
                    && match self.plan.owners[self.instances[*reference].owner.index()].kind {
                        OwnerKind::Graph => true,
                        OwnerKind::Node(slot) => !stop_receivers
                            .entry(scope.clone())
                            .or_insert_with(|| self.pending_stop_receivers(scope))
                            .contains(&slot),
                    }
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

    fn begin_settlement(&mut self, scope: &InstanceRef) {
        self.residualize_scope(scope);
        let targets: Vec<_> = self
            .instances
            .keys()
            .filter(|candidate| self.is_within_instance(candidate, scope))
            .cloned()
            .collect();
        for target in targets {
            if let Some(instance) = self.instances.get_mut(&target)
                && let InstancePhase::Closing { stage, .. } = &mut instance.phase
            {
                *stage = ClosingStage::Settling;
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
                self.begin_settlement(reference);
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
                // Session closure is requested in canonical logical-reference
                // order. A bounded Driver may defer the handoff; the next
                // boundary pump retries the same already-live session.
                let sessions: Vec<SessionRef> = self
                    .sessions
                    .iter()
                    .filter(|(_, session)| self.is_within_instance(&session.instance, reference))
                    .map(|(session, _)| *session)
                    .collect();
                for session_ref in sessions {
                    let session = self.sessions.get(&session_ref).cloned().ok_or_else(|| {
                        VmError::PlanInvariant {
                            node: node::Path::new(["session"])
                                .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                            reason: "a Closing Session vanished before close handoff",
                        }
                    })?;
                    let connector = self.plan.nodes[session.slot.index()].path.clone();
                    if bindings
                        .close_session(&connector, session_ref)
                        .map_err(crate::boundary::DispatchError::Driver)
                        .map_err(VmError::Dispatch)?
                        == SessionStart::Started
                        && let Some(session) = self.sessions.get_mut(&session_ref)
                    {
                        session.close_started = true;
                    }
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
                let writes: Vec<SessionWriteRef> = self
                    .session_writes
                    .iter()
                    .filter(|(_, write)| self.is_within_instance(&write.instance, reference))
                    .map(|(write, _)| *write)
                    .collect();
                for write in writes {
                    match journal
                        .settle_session_write(&write, true)
                        .map_err(VmError::Journal)?
                    {
                        crate::journal::CloseVerdict::AlreadyClosed => {}
                        crate::journal::CloseVerdict::Won => {
                            self.session_writes.remove(&write);
                        }
                    }
                }
                let sessions: Vec<SessionRef> = self
                    .sessions
                    .iter()
                    .filter(|(_, session)| self.is_within_instance(&session.instance, reference))
                    .map(|(session, _)| *session)
                    .collect();
                for session_ref in sessions {
                    if self
                        .settled_sessions
                        .iter()
                        .any(|settled| settled.session == session_ref)
                    {
                        continue;
                    }
                    let session =
                        self.sessions
                            .get(&session_ref)
                            .ok_or_else(|| VmError::PlanInvariant {
                                node: node::Path::new(["session"])
                                    .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                                reason: "a settling Session lost its logical record",
                            })?;
                    let connector = self.plan.nodes[session.slot.index()].path.clone();
                    let completion = bindings
                        .scheduler_session_completion(
                            &connector,
                            SchedulerCompletion::OutcomeUnknown,
                        )
                        .map_err(crate::boundary::DispatchError::Driver)
                        .map_err(VmError::Dispatch)?;
                    // The committed Closing request sealed this deadline.
                    // Forced cleanup must not wait for exit-route capacity or
                    // terminal storage; it proves no successful outcome.
                    bindings
                        .abort_session(&connector, session_ref)
                        .map_err(crate::boundary::DispatchError::Driver)
                        .map_err(VmError::Dispatch)?;
                    self.settled_sessions.push_back(SettledSession {
                        session: session_ref,
                        completion,
                        terminal_delivery: None,
                    });
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
                    exchange_exit: None,
                    loop_record: None,
                    logical_time: None,
                    timer_schedule: None,
                    instance_commands: None,
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
                let executor = HandlerExecutor::at(self.plan, self.graph_version, self.now);
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
            self.retain_emission_routes(&mut record, prepared)?;
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
        if work.lifecycle == node::Lifecycle::Stop {
            record
                .consumed
                .extend(self.graph_stop_cursors(&work.instance));
            self.close_instance_loops(&work.instance, &mut record)?;
        }
        journal.commit(record.clone()).map_err(VmError::Journal)?;
        if record.loop_protocol().is_some() {
            self.refresh_loop_manager(journal)?;
        }
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
            if matches!(owner.instance.lifetime, InstanceLifetime::SinglePass) {
                self.request_close(journal, &work.instance, work.cause.clone(), None)?;
            }
        }
        if let (Some(slot), Some(prepared)) = (node_slot, prepared) {
            self.route_emissions(slot, &work.instance, prepared);
        }
        if work.lifecycle == node::Lifecycle::Stop {
            self.discard_graph_admissions(&work.instance);
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

    fn promote_waiting(
        &mut self,
        journal: &(impl InstanceAdmissions + ?Sized),
    ) -> Result<(), VmError> {
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
        }
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
        let executor = HandlerExecutor::at(self.plan, self.graph_version, self.now);
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
        self.prepare_call_completion(
            slot,
            &request.instance,
            &request.delivery,
            &mut outcome.record,
        )?;
        let prepared =
            self.validate_and_prepare_emissions(journal, slot, &request.instance, &outcome.record)?;
        self.retain_emission_routes(&mut outcome.record, &prepared)?;
        journal
            .commit(outcome.record.clone())
            .map_err(VmError::Journal)?;
        if outcome.record.loop_protocol().is_some() {
            self.refresh_loop_manager(journal)?;
        }
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

    /// Commits one retained inbound-Call terminal observation. Fixed
    /// acknowledgements consume no mailbox cursor; a graph-driven response
    /// atomically consumes its real response delivery. Neither form creates
    /// an outbound record.
    fn settle_inbound_call_next(
        &mut self,
        journal: &mut dyn Journal,
        index: usize,
        event: InboundCallEvent,
        pending: PendingInboundCall,
    ) -> Result<(), VmError> {
        let executor = HandlerExecutor::at(self.plan, self.graph_version, self.now);
        let mut outcome = match pending.response.as_ref() {
            Some(response) => executor.inbound_call_response_completion(
                pending.slot,
                &pending.instance,
                &pending.request,
                &response.delivery,
                event.call_ref,
                &event.completion,
            ),
            None => executor.inbound_call_completion(
                pending.slot,
                &pending.instance,
                &pending.request,
                event.call_ref,
                &event.completion,
            ),
        }
        .map_err(VmError::Fault)?;
        let prepared = self.validate_and_prepare_emissions(
            journal,
            pending.slot,
            &pending.instance,
            &outcome.record,
        )?;
        self.retain_emission_routes(&mut outcome.record, &prepared)?;
        journal.commit(outcome.record).map_err(VmError::Journal)?;
        self.settled_inbound_calls.remove(index);
        self.inbound_calls.remove(&event.call_ref);
        if pending.response.is_some() {
            self.scheduler
                .mailboxes
                .release(pending.slot, &pending.instance);
        }
        self.route_emissions(pending.slot, &pending.instance, prepared);
        self.progress_closures();
        Ok(())
    }

    /// Commits the sole terminal transaction of one logical Session. The
    /// opening cursor (client) and an optional locally rejected write cursor
    /// are consumed atomically with the typed `exit`; inbound peer openings
    /// are roots and therefore consume neither.
    fn settle_session_next<B: OutboundConnectorBindings + ?Sized>(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut B,
        index: usize,
    ) -> Result<(), VmError> {
        let settled = self
            .settled_sessions
            .get(index)
            .cloned()
            .unwrap_or_else(|| unreachable!("guarded by the caller"));
        let Some(session) = self.sessions.get(&settled.session).cloned() else {
            return Err(VmError::PlanInvariant {
                node: node::Path::new(["session"])
                    .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                reason: "a terminal Session observation lost its logical session",
            });
        };
        let executor = HandlerExecutor::at(self.plan, self.graph_version, self.now);
        let consumed = session
            .opening_delivery
            .iter()
            .chain(settled.terminal_delivery.iter())
            .collect::<Vec<_>>();
        let mut outcome = executor
            .session_completion(
                session.slot,
                &session.instance,
                &session.opening,
                settled.session,
                &settled.completion,
                &consumed,
            )
            .map_err(VmError::Fault)?;
        let prepared = self.validate_and_prepare_emissions(
            journal,
            session.slot,
            &session.instance,
            &outcome.record,
        )?;
        self.retain_emission_routes(&mut outcome.record, &prepared)?;
        journal.commit(outcome.record).map_err(VmError::Journal)?;
        self.settled_sessions.remove(index);
        self.sessions.remove(&settled.session);
        self.scheduler
            .mailboxes
            .release(session.slot, &session.instance);
        self.route_emissions(session.slot, &session.instance, prepared);
        self.progress_closures();
        // Local cleanup follows the retained winner. Even a broken binding
        // cannot leave this committed transaction's deliveries unapplied.
        bindings
            .forget_session(settled.session)
            .map_err(crate::boundary::DispatchError::Driver)
            .map_err(VmError::Dispatch)?;
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
            .filter(|handler| handler.owner == slot)
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

        let executor = HandlerExecutor::at(self.plan, self.graph_version, self.now);
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
                            .filter(|delivery| {
                                &delivery.port == port
                                    && self.graph_context(delivery) == self.graph_context(trigger)
                            })
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
        let executor = HandlerExecutor::at(self.plan, self.graph_version, self.now);
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
        let application = self.loop_application(instance, delivery);
        let mut outcome = match &application {
            None => executor.fire_with_bindings(slot, instance, delivery, &state, values),
            Some((run, application)) => executor.fire_in_loop(
                slot,
                instance,
                delivery,
                &state,
                values,
                crate::runtime::LoopFiring {
                    run: *run,
                    iteration: application.iteration,
                    state: &application.state,
                },
            ),
        }
        .map_err(VmError::Fault)?;
        if self.plan.owners[self.plan.nodes[slot.index()].owner.index()]
            .loop_spec()
            .is_some()
        {
            self.prepare_operator_loop_verdict(slot, &mut outcome.record)?;
        }
        if let Some(packet) = outcome.record.loop_record.as_deref_mut() {
            packet.context = self.graph_context(delivery).to_vec();
            if let Some((run, _)) = application
                && packet.context.last() != Some(&run)
            {
                packet.context.push(run);
            }
        }
        self.retain_graph_verdicts(&mut outcome.record)?;
        let close_request = outcome.record.close_scope.clone();
        if let Some(scope) = &close_request {
            outcome.record.timers.extend(self.closing_timers(scope));
        }
        let prepared =
            self.validate_and_prepare_emissions(journal, slot, instance, &outcome.record)?;
        self.retain_emission_routes(&mut outcome.record, &prepared)?;
        // The journal owns the full transaction after commit. Retain only the
        // small delta the in-memory state fold still needs; cloning the whole
        // record would duplicate every emitted envelope on the relay hot path.
        let state_delta = outcome.record.state_delta.clone();
        journal.commit(outcome.record).map_err(VmError::Journal)?;
        if application.is_some() {
            self.refresh_loop_manager(journal)?;
        }
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

    /// Routes a Connector delivery by its validated exchange cardinality.
    /// Roles decide direction; the exchange decides whether the retained
    /// obligation is one Call or one long-lived Session.
    fn fire_connector<B: OutboundConnectorBindings + ?Sized>(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut B,
        slot: Slot,
        instance: &InstanceRef,
        delivery: &Delivery,
    ) -> Result<(), VmError> {
        let planned = &self.plan.nodes[slot.index()];
        let NodeExecution::Connector(spec) = &planned.execution else {
            return Err(VmError::PlanInvariant {
                node: planned.path.clone(),
                reason: "connector dispatch reached a non-connector node",
            });
        };
        match spec.exchange {
            ConnectorExchange::Call if spec.roles.has_outbound() => {
                self.open_call(journal, bindings, slot, instance, delivery)
            }
            ConnectorExchange::Call if spec.roles.has_inbound() => {
                self.open_inbound_call_response(journal, slot, instance, delivery)
            }
            ConnectorExchange::Session => {
                self.progress_session(journal, bindings, slot, instance, delivery)
            }
            ConnectorExchange::Event | ConnectorExchange::Call => Err(VmError::PlanInvariant {
                node: planned.path.clone(),
                reason: "a delivery reached a Connector without a compatible world role",
            }),
        }
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
        let context = self.graph_context(trigger);
        if inputs
            .iter()
            .any(|input| self.graph_context(input) != context)
        {
            return Err(VmError::InvalidLoopDecision {
                owner: instance.address().owner().clone(),
                reason: "E-LOOP-CONTEXT: join candidates belong to different applications",
            });
        }
        let application = self.loop_application(instance, trigger);
        let executor = HandlerExecutor::at(self.plan, self.graph_version, self.now);
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
                application.map(|(run, state)| crate::runtime::LoopFiring {
                    run,
                    iteration: state.iteration,
                    state: &state.state,
                }),
            )
            .map_err(VmError::Fault)?;
        if let Some(packet) = outcome.record.loop_record.as_deref_mut() {
            packet.context = context.to_vec();
        }
        self.retain_graph_verdicts(&mut outcome.record)?;
        let close_request = outcome.record.close_scope.clone();
        if let Some(scope) = &close_request {
            outcome.record.timers.extend(self.closing_timers(scope));
        }
        let prepared =
            self.validate_and_prepare_emissions(journal, slot, instance, &outcome.record)?;
        self.retain_emission_routes(&mut outcome.record, &prepared)?;
        journal
            .commit(outcome.record.clone())
            .map_err(VmError::Journal)?;
        if application.is_some() {
            self.refresh_loop_manager(journal)?;
        }
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
        let executor = HandlerExecutor::at(self.plan, self.graph_version, self.now);
        let mut outcome = executor
            .join_expired(slot, instance, delivery)
            .map_err(VmError::Fault)?;
        let prepared =
            self.validate_and_prepare_emissions(journal, slot, instance, &outcome.record)?;
        self.retain_emission_routes(&mut outcome.record, &prepared)?;
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

    /// Reserves one graph-produced response for the admitted inbound Call it
    /// names. The producing transaction already journaled the response
    /// envelope and route, so this step performs no external effect. The
    /// Driver receives the retained response through the registry and the
    /// eventual `l:reply` transaction consumes this real mailbox cursor.
    fn open_inbound_call_response(
        &mut self,
        journal: &mut dyn Journal,
        slot: Slot,
        instance: &InstanceRef,
        delivery: &Delivery,
    ) -> Result<(), VmError> {
        let planned = &self.plan.nodes[slot.index()];
        let NodeExecution::Connector(spec) = &planned.execution else {
            return Err(VmError::PlanInvariant {
                node: planned.path.clone(),
                reason: "an inbound-Call response was routed to a non-connector",
            });
        };
        if !spec.roles.has_inbound()
            || spec.roles.has_outbound()
            || spec.exchange != ConnectorExchange::Call
        {
            return Err(VmError::PlanInvariant {
                node: planned.path.clone(),
                reason: "a delivery reached a connector with neither an outbound Call nor an inbound-Call response role",
            });
        }

        let Some(call_ref) = self.select_inbound_call_ref(spec, &delivery.envelope)? else {
            return self.discard_inbound_call_response(journal, slot, instance, delivery);
        };
        let matches_open_call = self.inbound_calls.get(&call_ref).is_some_and(|call| {
            call.slot == slot
                && call.instance == *instance
                && call.response.is_none()
                && !self
                    .settled_inbound_calls
                    .iter()
                    .any(|event| event.call_ref == call_ref)
        });
        if !matches_open_call {
            return self.discard_inbound_call_response(journal, slot, instance, delivery);
        }
        let response = self
            .scheduler
            .mailboxes
            .pop(slot, instance)
            .ok_or_else(|| VmError::PlanInvariant {
                node: planned.path.clone(),
                reason: "an inbound-Call response disappeared before reservation",
            })?;
        let Some(call) = self.inbound_calls.get_mut(&call_ref) else {
            return Err(VmError::PlanInvariant {
                node: planned.path.clone(),
                reason: "an inbound-Call response lost its admitted request",
            });
        };
        call.response = Some(PendingInboundCallResponse {
            delivery: response,
            started: false,
        });
        self.scheduler.mailboxes.claim(slot, instance);
        Ok(())
    }

    /// Commits the observable orphan-response discard. A late or malformed
    /// correlation consumes exactly the already-journaled delivery and can
    /// never create protocol bytes or a second Call exit.
    fn discard_inbound_call_response(
        &mut self,
        journal: &mut dyn Journal,
        slot: Slot,
        instance: &InstanceRef,
        delivery: &Delivery,
    ) -> Result<(), VmError> {
        let planned = &self.plan.nodes[slot.index()];
        let record = crate::journal::TransactionRecord {
            owner_path: OwnerPath::from(&planned.path),
            instance: instance.clone(),
            parent_instance: None,
            close_scope: None,
            trigger: node::Trigger::Port(delivery.port.clone()),
            cause: Some(delivery.envelope.meta().sys().id()),
            exchange_exit: None,
            loop_record: None,
            logical_time: None,
            timer_schedule: None,
            instance_commands: None,
            consumed: vec![crate::journal::ConsumedPosition {
                edge: delivery.edge.clone(),
                source: delivery.source_instance.clone(),
                seq: delivery.seq,
            }],
            join_inputs: vec![],
            state_delta: vec![],
            emissions: vec![],
            routes: vec![],
            timers: vec![],
        };
        journal.commit(record).map_err(VmError::Journal)?;
        if self.scheduler.mailboxes.pop(slot, instance).is_none() {
            return Err(VmError::PlanInvariant {
                node: planned.path.clone(),
                reason: "a committed orphan response lost its mailbox delivery",
            });
        }
        self.progress_closures();
        Ok(())
    }

    /// Starts or writes one Session without ever using arrival order as a
    /// correlator. Server-like Connectors require a logical reference from
    /// lineage or their explicit `correlate` expression; client-like
    /// Connectors may open one session and thereafter reuse that sole live
    /// session for their instance.
    fn progress_session<B: OutboundConnectorBindings + ?Sized>(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut B,
        slot: Slot,
        instance: &InstanceRef,
        delivery: &Delivery,
    ) -> Result<(), VmError> {
        let planned = &self.plan.nodes[slot.index()];
        let NodeExecution::Connector(spec) = &planned.execution else {
            return Err(VmError::PlanInvariant {
                node: planned.path.clone(),
                reason: "Session dispatch reached a non-connector",
            });
        };
        if spec.exchange != ConnectorExchange::Session || !spec.roles.has_outbound() {
            return Err(VmError::PlanInvariant {
                node: planned.path.clone(),
                reason: "a Session delivery reached a Connector without an outbound Session role",
            });
        }
        let settings =
            self.resolve_invocation_settings(instance, spec, &planned.path, &delivery.envelope)?;
        let selected = self.select_session_ref(spec, &delivery.envelope, &settings)?;
        if let Some(session_ref) = selected {
            return if self.sessions.get(&session_ref).is_some_and(|session| {
                session.slot == slot && session.instance == *instance && session.opened
            }) {
                self.write_session(
                    journal,
                    bindings,
                    SessionDispatch {
                        slot,
                        instance,
                        delivery,
                    },
                    session_ref,
                    &settings,
                )
            } else {
                self.discard_session_write(journal, slot, instance, delivery)
            };
        }

        let live: Vec<SessionRef> = self
            .sessions
            .iter()
            .filter(|(_, session)| session.slot == slot && session.instance == *instance)
            .map(|(reference, _)| *reference)
            .collect();
        if matches!(spec.effect, node::Effect::None) {
            // A server never guesses which peer to address. The ordinary
            // consume-only discard is observable and prevents an orphan from
            // becoming a new client opening by accident.
            return self.discard_session_write(journal, slot, instance, delivery);
        }
        match live.as_slice() {
            [] => self.open_session(journal, bindings, slot, instance, delivery, &settings),
            [session_ref] => {
                let pending_open = self.sessions.get(session_ref).is_some_and(|session| {
                    !session.opened
                        && !session.opening_claimed
                        && session.opening.meta().sys().id() == delivery.envelope.meta().sys().id()
                });
                if pending_open {
                    self.open_session(journal, bindings, slot, instance, delivery, &settings)
                } else if self.sessions[session_ref].opened {
                    self.write_session(
                        journal,
                        bindings,
                        SessionDispatch {
                            slot,
                            instance,
                            delivery,
                        },
                        *session_ref,
                        &settings,
                    )
                } else {
                    Err(VmError::PlanInvariant {
                        node: planned.path.clone(),
                        reason: "a client Session opening lost its claimed mailbox",
                    })
                }
            }
            _ => Err(VmError::PlanInvariant {
                node: planned.path.clone(),
                reason: "a client Connector owns more than one live Session in one instance",
            }),
        }
    }

    fn select_session_ref(
        &self,
        spec: &crate::plan::ConnectorSpec,
        envelope: &Envelope,
        settings: &BTreeMap<String, crate::value::Value>,
    ) -> Result<Option<SessionRef>, VmError> {
        // The resident child belongs to this exact Node instance. An upstream
        // socket's session_id is provenance, not an address for this resource.
        if spec.contract.as_str() == crate::connector::PROCESS_SERVICE_CONNECTOR_ID {
            return Ok(None);
        }
        if spec.attribute("correlate").is_some() {
            let Some(crate::value::Value::Text(value)) = settings.get("correlate") else {
                return Err(VmError::PlanInvariant {
                    node: node::Path::new(["session"])
                        .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                    reason: "a Session correlator did not resolve to text",
                });
            };
            return Ok(SessionRef::parse_projection(value));
        }
        Ok(envelope
            .meta()
            .ext()
            .get("session_id")
            .and_then(|value| match value {
                crate::envelope::MetaValue::Scalar(crate::envelope::Scalar::Text(value)) => {
                    SessionRef::parse_projection(value)
                }
                crate::envelope::MetaValue::Scalar(
                    crate::envelope::Scalar::Bool(_)
                    | crate::envelope::Scalar::Int(_)
                    | crate::envelope::Scalar::Double(_)
                    | crate::envelope::Scalar::Bytes(_)
                    | crate::envelope::Scalar::Decimal { .. },
                )
                | crate::envelope::MetaValue::Array(_)
                | crate::envelope::MetaValue::Map(_) => None,
            }))
    }

    fn open_session<B: OutboundConnectorBindings + ?Sized>(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut B,
        slot: Slot,
        instance: &InstanceRef,
        delivery: &Delivery,
        settings: &BTreeMap<String, crate::value::Value>,
    ) -> Result<(), VmError> {
        let planned = &self.plan.nodes[slot.index()];
        let preparation = bindings
            .prepare_session_open(&planned.path, delivery.envelope.data(), settings)
            .map_err(crate::boundary::DispatchError::Driver)
            .map_err(VmError::Dispatch)?;
        let prepared = match preparation {
            RequestPreparation::Prepared(prepared) => prepared,
            RequestPreparation::Rejected(completion) => {
                return self.commit_pre_session_refusal(
                    journal,
                    slot,
                    instance,
                    delivery,
                    &completion,
                );
            }
        };
        let session_ref = ids::outbound_session_ref(
            &planned.path,
            delivery.envelope.meta().sys().id(),
            prepared.digest(),
        );
        let intent = SessionOpenIntent {
            session_ref,
            emitting_id: delivery.envelope.meta().sys().id(),
            connector_path: planned.path.clone(),
            prepared,
        };
        if let alloc::collections::btree_map::Entry::Vacant(entry) =
            self.sessions.entry(session_ref)
        {
            journal
                .open_outbound_session(intent.clone())
                .map_err(VmError::Journal)?;
            let retained = self
                .scheduler
                .mailboxes
                .pop(slot, instance)
                .ok_or_else(|| VmError::PlanInvariant {
                    node: planned.path.clone(),
                    reason: "a journaled Session opening lost its mailbox delivery",
                })?;
            entry.insert(PendingSession {
                slot,
                instance: instance.clone(),
                opening: delivery.envelope.clone(),
                open_intent: Some(intent.clone()),
                opening_delivery: Some(retained),
                opened: false,
                opening_started: false,
                opening_claimed: true,
                close_started: false,
            });
            self.scheduler.mailboxes.claim(slot, instance);
        }
        match bindings
            .open_session(&intent, &delivery.envelope)
            .map_err(crate::boundary::DispatchError::Driver)
            .map_err(VmError::Dispatch)?
        {
            SessionStart::Pending => Ok(()),
            SessionStart::Started => {
                let session =
                    self.sessions
                        .get_mut(&session_ref)
                        .ok_or_else(|| VmError::PlanInvariant {
                            node: planned.path.clone(),
                            reason: "a started Session opening lost its logical record",
                        })?;
                session.opening_started = true;
                Ok(())
            }
        }
    }

    fn write_session<B: OutboundConnectorBindings + ?Sized>(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut B,
        dispatch: SessionDispatch<'_>,
        session_ref: SessionRef,
        settings: &BTreeMap<String, crate::value::Value>,
    ) -> Result<(), VmError> {
        let SessionDispatch {
            slot,
            instance,
            delivery,
        } = dispatch;
        let planned = &self.plan.nodes[slot.index()];
        let preparation = bindings
            .prepare_session_write(
                &planned.path,
                session_ref,
                delivery.envelope.data(),
                settings,
            )
            .map_err(crate::boundary::DispatchError::Driver)
            .map_err(VmError::Dispatch)?;
        let prepared = match preparation {
            RequestPreparation::Prepared(prepared) => prepared,
            RequestPreparation::Rejected(completion) => {
                let terminal = self
                    .scheduler
                    .mailboxes
                    .pop(slot, instance)
                    .ok_or_else(|| VmError::PlanInvariant {
                        node: planned.path.clone(),
                        reason: "a rejected Session write lost its mailbox delivery",
                    })?;
                self.scheduler.mailboxes.claim(slot, instance);
                self.settled_sessions.push_back(SettledSession {
                    session: session_ref,
                    completion,
                    terminal_delivery: Some(terminal),
                });
                return Ok(());
            }
        };
        let write_ref = ids::session_write_ref(
            session_ref,
            delivery.envelope.meta().sys().id(),
            prepared.digest(),
        );
        let intent = SessionWriteIntent {
            write_ref,
            session_ref,
            emitting_id: delivery.envelope.meta().sys().id(),
            connector_path: planned.path.clone(),
            prepared,
        };
        if let alloc::collections::btree_map::Entry::Vacant(entry) =
            self.session_writes.entry(write_ref)
        {
            journal
                .open_session_write(intent.clone())
                .map_err(VmError::Journal)?;
            let retained = self
                .scheduler
                .mailboxes
                .pop(slot, instance)
                .ok_or_else(|| VmError::PlanInvariant {
                    node: planned.path.clone(),
                    reason: "a journaled Session write lost its mailbox delivery",
                })?;
            entry.insert(PendingSessionWrite {
                intent: intent.clone(),
                slot,
                instance: instance.clone(),
                delivery: retained,
                started: false,
            });
            let held = self
                .session_writes
                .values()
                .filter(|pending| pending.slot == slot && pending.instance == *instance)
                .count();
            if held >= WORKER_MAILBOX_CAPACITY {
                // Moving an effect from the bounded mailbox into the durable
                // Session-write fold must not create an unbounded side queue.
                // At the shared Worker bound, keep this mailbox claimed until
                // a local write settlement releases one slot.
                self.scheduler.mailboxes.claim(slot, instance);
            }
        }
        match bindings
            .write_session(&intent, &delivery.envelope)
            .map_err(crate::boundary::DispatchError::Driver)
            .map_err(VmError::Dispatch)?
        {
            SessionStart::Pending => Ok(()),
            SessionStart::Started => {
                let write = self.session_writes.get_mut(&write_ref).ok_or_else(|| {
                    VmError::PlanInvariant {
                        node: planned.path.clone(),
                        reason: "a started Session write lost its journaled intent",
                    }
                })?;
                write.started = true;
                Ok(())
            }
        }
    }

    fn commit_pre_session_refusal(
        &mut self,
        journal: &mut dyn Journal,
        slot: Slot,
        instance: &InstanceRef,
        delivery: &Delivery,
        completion: &Completion,
    ) -> Result<(), VmError> {
        let executor = HandlerExecutor::at(self.plan, self.graph_version, self.now);
        let trigger = node::Trigger::Port(delivery.port.clone());
        let mut outcome = executor
            .connector_completion(slot, instance, delivery, &trigger, completion)
            .map_err(VmError::Fault)?;
        let prepared =
            self.validate_and_prepare_emissions(journal, slot, instance, &outcome.record)?;
        self.retain_emission_routes(&mut outcome.record, &prepared)?;
        journal.commit(outcome.record).map_err(VmError::Journal)?;
        self.scheduler.mailboxes.pop(slot, instance);
        self.route_emissions(slot, instance, prepared);
        self.progress_closures();
        Ok(())
    }

    fn discard_session_write(
        &mut self,
        journal: &mut dyn Journal,
        slot: Slot,
        instance: &InstanceRef,
        delivery: &Delivery,
    ) -> Result<(), VmError> {
        self.discard_inbound_call_response(journal, slot, instance, delivery)
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
        // A recovered mailbox can be ready after its retained budget is
        // already due. Settle that exact delivery before handing any intent
        // to the Driver, even when the caller drains before `advance`.
        let overdue = self.scheduler.dated.range(..=(self.now, u64::MAX)).find(|(_, payload)| {
            matches!(payload, DatedPayload::DeliveryBudget { edge, delivery: id, instance: target, .. }
                if edge == &delivery.edge && *id == delivery.envelope.meta().sys().id() && target == instance)
        }).map(|(key, payload)| (*key, payload.clone()));
        if let Some((key, payload)) = overdue {
            self.expire(journal, bindings, payload)?;
            self.scheduler.ack(key);
            return Ok(());
        }
        let planned = &self.plan.nodes[slot.index()];
        let NodeExecution::Connector(spec) = &planned.execution else {
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
                let executor = HandlerExecutor::at(self.plan, self.graph_version, self.now);
                let trigger = node::Trigger::Port(delivery.port.clone());
                let mut outcome = executor
                    .connector_completion(slot, instance, delivery, &trigger, &completion)
                    .map_err(VmError::Fault)?;
                self.prepare_call_completion(slot, instance, delivery, &mut outcome.record)?;
                let prepared =
                    self.validate_and_prepare_emissions(journal, slot, instance, &outcome.record)?;
                self.retain_emission_routes(&mut outcome.record, &prepared)?;
                journal
                    .commit(outcome.record.clone())
                    .map_err(VmError::Journal)?;
                if outcome.record.loop_protocol().is_some() {
                    self.refresh_loop_manager(journal)?;
                }
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
            logical_time: self.now,
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
        journal: &(impl InstanceAdmissions + ?Sized),
        slot: Slot,
        source_instance: &InstanceRef,
        record: &crate::runtime::TransactionRecord,
    ) -> Result<PreparedEmissions, VmError> {
        let mut prepared = Vec::with_capacity(record.emissions.len());
        let mut commands = Vec::new();
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
            commands.extend(self.prepare_instance_commands(
                journal,
                &self.graph_endpoint(&from),
                Some(source_instance),
                &emission.envelope,
            )?);
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
        commands.sort_by_key(|command| command.id);
        self.check_instance_command_room(journal, &commands)?;
        Ok(PreparedEmissions {
            commands,
            contexts: if self.graph_loops.is_some() {
                self.graph_emission_contexts(record, &prepared)
            } else {
                Vec::new()
            },
            emissions: prepared,
            candidates,
        })
    }

    /// Selects complete target references for every edge leaving one
    /// endpoint. The target chain is constructed outermost-to-innermost and
    /// remains a pure candidate until the producer record commits.
    fn prepare_routes(
        &self,
        journal: &(impl InstanceAdmissions + ?Sized),
        from: &EndpointRef,
        source_instance: Option<&InstanceRef>,
        envelope: &Envelope,
        candidates: &mut BTreeMap<InstanceRef, InstanceCandidate>,
    ) -> Result<Vec<PreparedEdge>, VmError> {
        if self.graph_loops.is_some() {
            return self.prepare_graph_routes(
                journal,
                &self.graph_endpoint(from),
                source_instance,
                envelope,
                candidates,
                false,
            );
        }
        let mut prepared = Vec::new();
        let selected_value = self
            .plan
            .has_case_routes(from)
            .then(|| DataSlot::new(envelope.data()).materialize())
            .transpose()
            .map_err(VmError::Fault)?;
        if let Some(value) = &selected_value {
            crate::runtime::crossing_pins_hold(self.plan, from, value)
                .map_err(|violation| VmError::Fault(violation.into_fault()))?;
        }
        for edge in self
            .plan
            .edges
            .iter()
            .filter(|edge| edge.from == *from)
            .filter(|edge| {
                selected_value
                    .as_ref()
                    .is_none_or(|value| self.plan.edge_is_selected(edge, value))
            })
            .cloned()
        {
            let target = match edge.to.as_ref().and_then(|to| to.node) {
                Some(target) => Some(
                    if let Some(target) = self.prepare_direct_inbound_call_response_target(
                        edge.scope,
                        source_instance,
                        target,
                        envelope,
                    )? {
                        target
                    } else {
                        self.prepare_target(
                            journal,
                            edge.scope,
                            source_instance,
                            target,
                            envelope,
                            candidates,
                        )?
                    },
                ),
                None => None,
            };
            prepared.push(PreparedEdge {
                edge,
                target,
                boundary: None,
            });
        }
        Ok(prepared)
    }

    /// A direct response returns to the exact Connector generation that
    /// admitted its root Call. Selecting that destination here, before the
    /// producer commits, is essential: re-evaluating a keyed Connector from
    /// transformed response data could otherwise migrate the response to a
    /// different instance and make the recorded route a lie.
    fn prepare_direct_inbound_call_response_target(
        &self,
        edge_scope: OwnerSlot,
        source_instance: Option<&InstanceRef>,
        target: Slot,
        envelope: &Envelope,
    ) -> Result<Option<PreparedTarget>, VmError> {
        let Some(spec) = self.plan.nodes[target.index()].connector() else {
            return Ok(None);
        };
        if !spec.roles.has_inbound()
            || spec.roles.has_outbound()
            || spec.exchange != ConnectorExchange::Call
        {
            return Ok(None);
        }

        let Some(call_ref) = self.select_inbound_call_ref(spec, envelope)? else {
            return Ok(None);
        };
        let Some(pending) = self.inbound_calls.get(&call_ref) else {
            return Ok(None);
        };
        if pending.slot != target
            || pending.response.is_some()
            || self
                .settled_inbound_calls
                .iter()
                .any(|event| event.call_ref == call_ref)
        {
            return Ok(None);
        }

        let owner_chain = self.owner_chain(self.plan.nodes[target.index()].owner)?;
        let Some(scope_index) = owner_chain.iter().position(|owner| *owner == edge_scope) else {
            return Err(VmError::PlanInvariant {
                node: self.plan.nodes[target.index()].path.clone(),
                reason: "the response edge scope is not an ancestor of its inbound Call target",
            });
        };
        let mut selected = Vec::with_capacity(owner_chain.len());
        let mut cursor = Some(pending.instance.clone());
        while let Some(reference) = cursor {
            let instance =
                self.instances
                    .get(&reference)
                    .ok_or_else(|| VmError::PlanInvariant {
                        node: self.plan.nodes[target.index()].path.clone(),
                        reason: "an open inbound Call lost its installed instance chain",
                    })?;
            selected.push((instance.owner, reference));
            cursor = instance.parent.clone();
        }
        selected.reverse();
        if selected.len() != owner_chain.len()
            || selected
                .iter()
                .zip(owner_chain)
                .any(|((actual, _), expected)| actual != expected)
        {
            return Err(VmError::PlanInvariant {
                node: self.plan.nodes[target.index()].path.clone(),
                reason: "an open inbound Call has an instance chain inconsistent with its owner",
            });
        }
        let chain: Vec<InstanceRef> = selected
            .into_iter()
            .map(|(_, reference)| reference)
            .collect();
        let reference = chain
            .last()
            .cloned()
            .ok_or_else(|| VmError::PlanInvariant {
                node: self.plan.nodes[target.index()].path.clone(),
                reason: "an inbound Call target has no instance",
            })?;
        let scope = chain
            .get(scope_index)
            .cloned()
            .ok_or_else(|| VmError::PlanInvariant {
                node: self.plan.nodes[target.index()].path.clone(),
                reason: "the direct response route lost its edge scope instance",
            })?;
        let Some(source_scope) = self.source_instance_for_owner(source_instance, edge_scope)?
        else {
            return Ok(None);
        };
        if source_scope != scope {
            return Ok(None);
        }
        let admitted = chain.iter().skip(scope_index + 1).cloned().collect();
        Ok(Some(PreparedTarget {
            reference,
            scope,
            chain,
            admitted,
        }))
    }

    /// Selects one admitted inbound Call before a response route commits.
    /// Default lineage uses the immutable origin id. An explicit correlator
    /// is a checked direct `data`/`meta` path and must yield the textual root
    /// reference carried by the application through a root-mixing path.
    fn select_inbound_call_ref(
        &self,
        spec: &crate::plan::ConnectorSpec,
        envelope: &Envelope,
    ) -> Result<Option<InboundCallRef>, VmError> {
        let Some(setting) = spec.attribute("correlate") else {
            return Ok(Some(InboundCallRef::from_root(
                envelope.meta().sys().origin_id(),
            )));
        };
        let AttrSetting::Expr {
            site: EvaluationSite::Invocation,
            expr,
        } = setting
        else {
            return Err(VmError::PlanInvariant {
                node: node::Path::new(["inbound-call"])
                    .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                reason: "an inbound Call correlator is not one invocation expression",
            });
        };
        let mut bindings = BTreeMap::new();
        bindings.insert(
            String::from("meta"),
            crate::expr::EvalValue::from(meta_value(envelope.meta())),
        );
        let data = DataSlot::new(envelope.data());
        let selected = resolve_payload(
            eval_lazy(expr, &bindings, &data, &self.plan.assets).map_err(VmError::Fault)?,
            &data,
        )
        .map_err(VmError::Fault)?;
        let crate::value::Value::Text(selected) = selected else {
            return Err(VmError::PlanInvariant {
                node: node::Path::new(["inbound-call"])
                    .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                reason: "an inbound Call correlator did not yield text",
            });
        };
        Ok(self
            .inbound_calls
            .keys()
            .copied()
            .find(|reference| reference.to_string() == selected))
    }

    /// Every route decision sealed by an inbound record. The envelope id is
    /// implicit because an inbound record carries exactly one envelope.
    fn inbound_route_targets(&self, prepared: &[PreparedEdge]) -> Vec<RouteTarget> {
        prepared
            .iter()
            .filter_map(|route| {
                route.target.as_ref().map(|target| RouteTarget {
                    edge: self.prepared_edge_text(route),
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
                        edge: self.prepared_edge_text(route),
                        target: target.reference.clone(),
                    })
                })
            })
            .collect()
    }

    fn prepare_target(
        &self,
        journal: &(impl InstanceAdmissions + ?Sized),
        scope: OwnerSlot,
        source_instance: Option<&InstanceRef>,
        target: Slot,
        envelope: &Envelope,
        candidates: &mut BTreeMap<InstanceRef, InstanceCandidate>,
    ) -> Result<PreparedTarget, VmError> {
        let target_owner = self.plan.nodes[target.index()].owner;
        self.prepare_owner_target(
            journal,
            scope,
            source_instance,
            target_owner,
            envelope,
            candidates,
        )
    }

    fn prepare_owner_target(
        &self,
        journal: &(impl InstanceAdmissions + ?Sized),
        scope: OwnerSlot,
        source_instance: Option<&InstanceRef>,
        target_owner: OwnerSlot,
        envelope: &Envelope,
        candidates: &mut BTreeMap<InstanceRef, InstanceCandidate>,
    ) -> Result<PreparedTarget, VmError> {
        let refusal = |reason| match self.plan.owners[target_owner.index()].kind {
            OwnerKind::Node(slot) => VmError::PlanInvariant {
                node: self.plan.nodes[slot.index()].path.clone(),
                reason,
            },
            OwnerKind::Graph => VmError::InvalidLoopDecision {
                owner: self.plan.owners[target_owner.index()].path.clone(),
                reason,
            },
        };
        let chain = self.owner_chain(target_owner)?;
        let Some(scope_index) = chain.iter().position(|owner| *owner == scope) else {
            return Err(refusal(
                "the edge scope is not an ancestor of its target owner",
            ));
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
            .ok_or_else(|| refusal("a routed owner has no owner chain"))?;
        let scope = selected
            .get(scope_index)
            .cloned()
            .ok_or_else(|| refusal("the selected owner chain lost its edge scope"))?;
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
        journal: &(impl InstanceAdmissions + ?Sized),
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
                // Boot settings are consumed once by the bound Driver. They
                // never become per-instance Values inside the semantic VM.
                EvaluationSite::Instance => *site == EvaluationSite::Instance,
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
        let mut loop_admissions = 0usize;
        let mut loop_owner = None;
        for routes in routes {
            for prepared in routes {
                if let Some(owner) = self.prepared_graph_entry(prepared) {
                    loop_admissions = loop_admissions.saturating_add(1);
                    loop_owner.get_or_insert(owner);
                    continue;
                }
                let Some(to) = &prepared.edge.to else {
                    continue;
                };
                let Some(target) = to.node else {
                    continue;
                };
                if self.has_loops {
                    let owner = self.plan.nodes[target.index()].owner;
                    if self.plan.owners[owner.index()]
                        .loop_spec()
                        .is_some_and(|spec| spec.entry == to.port)
                    {
                        loop_admissions = loop_admissions.saturating_add(1);
                        loop_owner.get_or_insert(owner);
                    }
                }
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
        if let Some(owner) = loop_owner
            && self
                .loop_admission_load()
                .saturating_add(self.graph_admission_load())
                .saturating_add(loop_admissions)
                > WORKER_MAILBOX_CAPACITY
        {
            return Err(VmError::LoopQueueFull {
                owner: self.plan.owners[owner.index()].path.clone(),
            });
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
        let PreparedEmissions {
            emissions,
            candidates,
            contexts,
            commands,
        } = prepared;
        self.enqueue_instance_commands(commands);
        for PreparedEmission { emission, routes } in emissions {
            // The emission itself is observable at its source port —
            // `expect relay.out` watches departures, not arrivals.
            self.observed.push(Observed {
                node: Some(self.plan.nodes[slot.index()].path.clone()),
                port: emission.port.clone(),
                envelope: emission.envelope.clone(),
            });
            self.route_prepared(
                routes,
                &candidates,
                &emission.envelope,
                Some(source_instance),
            );
        }
        if self.graph_loops.is_some() {
            self.install_graph_contexts(contexts);
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
        for route in routes {
            if self.deliver_graph_admission(&route, candidates, envelope, source_instance) {
                continue;
            }
            let edge_text = self.prepared_edge_text(&route);
            let PreparedEdge {
                edge,
                target: selected,
                ..
            } = &route;
            match &edge.to {
                None => {} // `-> nothing`: the explicit, written discard
                Some(to) => match to.node {
                    None => {
                        self.outputs.push(BoundaryOutput {
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
                        for (arm, payload) in
                            self.delivery_deadlines(&route, prepared, candidates, envelope)
                        {
                            self.scheduler.arm(arm.due_at, payload);
                        }
                        self.observed.push(Observed {
                            node: Some(self.plan.nodes[target.index()].path.clone()),
                            port: to.port.clone(),
                            envelope: envelope.clone(),
                        });
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
        let deadlines =
            self.admission_deadlines(target, candidates, envelope, &mut BTreeMap::new());
        for reference in &target.chain {
            let admitted = target.admitted.contains(reference);
            match self.instances.get_mut(reference) {
                Some(instance) => {
                    if admitted
                        && instance.last_activity.meta().sys().id() != envelope.meta().sys().id()
                    {
                        instance.last_activity = envelope.clone();
                    }
                }
                None => {
                    if let Some(waiting) = self.waiting.get_mut(reference) {
                        if admitted
                            && waiting.last_activity.meta().sys().id() != envelope.meta().sys().id()
                        {
                            waiting.last_activity = envelope.clone();
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
                }
            };
        }
        for (arm, payload) in deadlines {
            self.scheduler.arm(arm.due_at, payload);
        }
    }

    /// The canonical edge rendering — the same text the consumed
    /// cursor seals.
    fn edge_text(&self, edge: &Edge) -> String {
        self.plan.edge_text(edge)
    }
}
