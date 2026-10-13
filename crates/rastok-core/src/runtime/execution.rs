//! Live execution of a validated plan, for both real runs and simulation.
//!
//! The host creates an [`Execution`], supplies inputs and logical time, and
//! advances ready work with [`Execution::step`] or [`Execution::run_ready`].
//! [`Execution::poll_drivers`] submits committed effect requests to the
//! supplied Drivers and collects their observations. Real and simulated
//! Drivers use the same interfaces; this module reads no ambient clock.
//!
//! # The delivery protocol
//!
//! A transaction is atomic in [`TransactionPreparer`]; this module
//! preserves that atomicity across routing. A transaction follows the same
//! discipline: *reserve* the head of a mailbox without removing it,
//! *evaluate* on isolated state, *reserve capacity* in every target
//! mailbox, *commit* to the journal — and only then remove the
//! delivery, apply the state, and route. A fault or refusal anywhere
//! before commit leaves the delivery in its mailbox. Applying the committed
//! transaction to live state introduces no further admission decision.

use crate::boundary::{
    AdmittedIngress, BoundaryObservation, CallId, CallOpen, ClosingDeadline, Completion,
    CompletionCheck, DriverPoller, InboundCallId, InboundCallObservation, InboundCallResponse,
    InboundCallTimeout, InboundSession, IngressAdmission, IngressProposal, IngressRejection,
    InstanceDeadline, Obligation, OutboundConnectorBindings, RequestPreparation, Response,
    RetainedProposal, SchedulerCompletion, SessionId, SessionObservation, SessionOpen,
    SessionStart, SessionWrite, SessionWriteId,
};
use crate::connector::{ConnectorExchange, SessionOpening, session_opening};
use crate::envelope;
use crate::envelope::{Data, Envelope, Id, Meta, MetaSys};
use crate::instance;

use crate::journal::admission::InstanceAdmissions;
use crate::journal::{Emission, Journal, LoopRunId, RouteTarget, RoutedDelivery, ids};
use crate::node;
use crate::plan::{
    AttrSetting, Edge, EdgeEnd, EvaluationSite, GraphVersion, NodeExecution, NodeIndex, OwnerKind,
    ValidatedExecutionPlan,
};
use crate::runtime::{
    DataSlot, Fault, JoinCandidate, LifecycleFiring, Mailboxes, State, TransactionPreparer,
    eval_lazy, meta_value, resolve_payload,
};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

mod admission;
mod advance;
pub use advance::TimeAdvance;
mod call_completion;
mod closing;
mod graph_loop;
mod instance_stop;
mod loop_manager;
mod recovery;
mod retry_manager;
mod timing;
use core::fmt;

/// Failure of an execution operation: invalid admission, transaction preparation,
/// journal access, Driver binding, recovery or an internal invariant.
/// This is returned to the host, never emitted as a business payload.
/// It does not by itself imply that the operation committed no record;
/// the journal remains authoritative when determining what completed.
#[derive(Debug, Clone, PartialEq)]
pub enum ExecutionFault {
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
        owner: node::OwnerPath,
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
        owner: node::OwnerPath,
        /// Missing, ambiguous, or inconsistent instance authority.
        reason: crate::journal::instance_stop::Error,
    },
    /// Instance command capacity is checked with the other producer products.
    InstanceCommandQueueFull,
    /// The Driver poller raised an integrity fault.
    Dispatch(crate::boundary::PollFault),
    /// A target mailbox has no room for a transaction's deliveries —
    /// backpressure surfaced before the commit.
    MailboxFull {
        /// The refusing node.
        node: node::Path,
    },
    /// The shared recurrence budget cannot admit another Loop run or Retry operation.
    RecurrenceQueueFull {
        /// Owner whose opening cannot be reserved.
        owner: node::OwnerPath,
    },
    /// Runtime defense for a control decision the validated L3 proof says is
    /// impossible.
    InvalidLoopDecision {
        /// Loop owner whose traversal contradicted the proof.
        owner: node::OwnerPath,
        /// Stable invariant description.
        reason: &'static str,
    },
    /// A route reached an owner generation whose finite lifetime has already
    /// won. The producer must retain normal backpressure; it cannot journal a
    /// route into the closing generation.
    InstanceClosing {
        /// Owner refusing new external admission.
        owner: node::OwnerPath,
    },
    /// A processing-time box has already closed. Its address has no second
    /// forward generation, so retry cannot reopen or spill it.
    ClosedTimeBox {
        /// Tumbling owner whose box is closed.
        owner: node::OwnerPath,
        /// Canonical start of the closed box.
        start_ns: i64,
    },
    /// A causal closure arrived for a call this machine holds no
    /// pending request for — an integrity breach, never a silence.
    MissingPending(CallId),
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
        rejected_at: EdgeEnd,
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

impl fmt::Display for ExecutionFault {
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
            Self::RecurrenceQueueFull { owner } => {
                write!(
                    f,
                    "the shared recurrence admission budget is full at `{owner}`"
                )
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

impl core::error::Error for ExecutionFault {}

/// Result of polling outbound Drivers: the number of settled Call completions
/// queued for a Connector transaction, plus discarded, uncorrelated and rejected
/// Call or Session observations for the host to report.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PollReport {
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
    pub discarded_sessions: Vec<SessionObservation>,
    /// Session observations matching no live logical Session.
    pub uncorrelated_sessions: Vec<SessionObservation>,
    /// Session terminal observations refused by the resolved output table.
    pub rejected_sessions: Vec<(SessionObservation, String)>,
}

/// Result of polling inbound-Call Drivers: terminal observations queued for
/// a Connector transaction, plus observations which were not accepted.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InboundCallPollReport {
    /// Valid terminal events retained for a Connector transaction.
    pub queued: usize,
    /// Duplicate terminal events observed after one was already retained.
    pub discarded: Vec<InboundCallObservation>,
    /// Events matching no admitted Call in this VM.
    pub uncorrelated: Vec<InboundCallObservation>,
    /// Events refused by the resolved Connector output table. The Call stays
    /// open so the Driver or a later timeout may still terminate it.
    pub rejected: Vec<(InboundCallObservation, String)>,
}

impl PollReport {
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
    slot: NodeIndex,
    delivery: envelope::Delivery,
    instance: instance::Generation,
}

/// One accepted inbound Call held until its Driver reports the sole terminal
/// completion. Unlike an outbound call, no input cursor is claimed: the root
/// itself opened this obligation.
#[derive(Debug, Clone, PartialEq)]
struct PendingInboundCall {
    slot: NodeIndex,
    request: Envelope,
    instance: instance::Generation,
    response: Option<PendingInboundCallResponse>,
    timeout: Option<PendingInboundCallTimeout>,
}

/// One graph response held behind the source Connector's claimed mailbox
/// until its protocol terminal observation commits `l:reply`.
#[derive(Debug, Clone, PartialEq)]
struct PendingInboundCallResponse {
    delivery: envelope::Delivery,
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
    slot: NodeIndex,
    instance: instance::Generation,
    opening: Envelope,
    /// Present only for an outbound opening whose exact prepared bytes may
    /// need to be re-offered after bounded Driver pressure.
    open_intent: Option<SessionOpen>,
    /// Outbound client openings retain their graph cursor through the sole
    /// terminal Session transaction. Inbound peer openings are roots and
    /// therefore carry no consumed cursor.
    opening_delivery: Option<envelope::Delivery>,
    opened: bool,
    opening_started: bool,
    opening_claimed: bool,
    close_started: bool,
}

/// One graph-to-world write retained until the Driver accepts its bounded
/// action and later reports local settlement.
#[derive(Debug, Clone, PartialEq)]
struct PendingSessionWrite {
    intent: SessionWrite,
    slot: NodeIndex,
    instance: instance::Generation,
    delivery: envelope::Delivery,
    started: bool,
}

/// One mailbox delivery being dispatched to an already-selected Session.
/// Keeping its three routing facts together prevents a write from being
/// prepared for one instance and retained from another.
struct SessionDispatch<'a> {
    slot: NodeIndex,
    instance: &'a instance::Generation,
    delivery: &'a envelope::Delivery,
}

/// One accepted terminal observation waiting for its atomic Connector
/// transaction.
#[derive(Debug, Clone, PartialEq)]
struct SettledSession {
    session: SessionId,
    completion: Completion,
    /// A preparation refusal can terminate a Session before a write intent
    /// opens. Its causing graph cursor is consumed by the terminal
    /// transaction together with the opening cursor.
    terminal_delivery: Option<envelope::Delivery>,
}

/// Pure instance construction. Installation happens only after the producer
/// route commits.
#[derive(Debug, Clone)]
struct InstanceCandidate {
    owner: node::OwnerSlot,
    reference: instance::Generation,
    parent: Option<instance::Generation>,
    frames: Vec<instance::Segment>,
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
    reference: instance::Generation,
    scope: instance::Generation,
    chain: Vec<instance::Generation>,
    /// Owners whose boundary this route crosses. Sticky ancestors are not
    /// re-admitted and therefore do not move an idle deadline.
    admitted: Vec<instance::Generation>,
}

/// Lifecycle work owns its causal envelope and names the lexical owner. The
/// live bundle remains installed through `l:stop` so stop emissions retain
/// their complete source chain; reclamation follows the successful commit.
#[derive(Debug, Clone)]
struct LifecycleWork {
    owner: node::OwnerSlot,
    instance: instance::Generation,
    parent_instance: Option<instance::Generation>,
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
    candidates: BTreeMap<instance::Generation, InstanceCandidate>,
    contexts: Vec<crate::journal::loop_protocol::RoutedContext>,
    commands: Vec<crate::journal::instance_stop::Command>,
}

/// One Operator activation selected without removing mailbox work. Join
/// inputs remain ordinary durable routed deliveries until their transaction
/// commits; an eviction follows the same reserve/commit/remove discipline.
#[derive(Debug, Clone)]
enum OperatorWork {
    Single(envelope::Delivery),
    Join {
        activation: node::HandlerActivation,
        trigger: envelope::Delivery,
        inputs: Vec<envelope::Delivery>,
    },
    Evict(envelope::Delivery),
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

/// Validates a Call completion against the Connector's resolved ports and
/// output types in the validated plan, before its closure is committed.
struct PlanCompletionCheck<'plan> {
    plan: &'plan ValidatedExecutionPlan,
}

/// The explicit no-world binding used by the pure `step`/`run_ready` helpers.
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
        _: &CallId,
    ) -> Result<(), crate::boundary::DriverRouteError> {
        Err(crate::boundary::DriverRouteError::UnboundOutbound(
            connector.clone(),
        ))
    }

    fn start(
        &mut self,
        intent: &CallOpen,
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

impl crate::boundary::CompletionCheck for PlanCompletionCheck<'_> {
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
        let slot =
            NodeIndex::new(u32::try_from(index).map_err(|_| {
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
            if resolved.direction() != node::PortDirection::Output {
                return Err(format!(
                    "`{connector}.{}` is not an output port",
                    output.port.name()
                ));
            }
            // Validation has already proved the transported contract row.
            // The resolved table is therefore the sole admission authority
            // here too, independently of the concrete Connector family.
            let expected = resolved.ty();
            if spec.contract.as_str() == crate::connector::HTTP_CLIENT_CONNECTOR_ID
                && matches!(spec.attribute("codec"), Some(crate::plan::AttrSetting::Word(word)) if word == "json")
                && output.port.name() == "out"
                && expected != &crate::datatype::DataType::Variant
                && !matches!(&output.data, Data::Materialized(_))
            {
                return Err(format!(
                    "`{connector}.out` requires a deserialized JSON value before retention"
                ));
            }
            if output.port.name() == "exit" && !matches!(&output.data, Data::Materialized(_)) {
                return Err(format!(
                    "`{connector}.exit` must carry a materialized terminal union"
                ));
            }
            if let Data::Materialized(value) = &output.data
                && let Err(violation) = expected.check_value_type(output.port.name(), value)
            {
                return Err(format!(
                    "`{connector}.{}` refuses the completion value: {violation}",
                    output.port.name()
                ));
            }
            if let Some(call) = spec.call_semantics()
                && output.port.name() == call.terminal()
                && !output
                    .data
                    .materialized_value()
                    .is_some_and(|value| call.terminal_case(value).is_some())
            {
                return Err(format!(
                    "`{connector}` returned a terminal outside its Contract"
                ));
            }
            if let Data::Materialized(value) = &output.data {
                let from = EdgeEnd {
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

/// An envelope seen at a port, leaving its producer or arriving at a target.
/// This is the debugger's live log, not another journal record. Readers and
/// Lab expectations inspect windows of it without consuming any delivery.
#[derive(Debug, Clone, PartialEq)]
pub struct EnvelopeView {
    /// The observed node, or `None` at the App boundary.
    pub node: Option<node::Path>,
    /// The source or destination port where the envelope was seen.
    pub port: node::Port,
    /// The observed envelope.
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
    pub reference: instance::Generation,
    /// Static lexical owner.
    pub owner: node::OwnerPath,
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
    pub instance: instance::Generation,
    /// Whether execution has reserved the mailbox head for an open protocol.
    pub claimed: bool,
    /// Current FIFO contents in delivery order.
    pub deliveries: Vec<envelope::Delivery>,
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
    Outbound(CallId),
    /// World-to-graph Call identity.
    Inbound(InboundCallId),
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
    pub instance: instance::Generation,
    /// Whether the terminal observation is retained but not yet consumed.
    pub completion_recorded: bool,
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
    pub reference: SessionId,
    /// Direction of the opening.
    pub direction: SessionDirectionInspection,
    /// Connector owning the protocol.
    pub connector: node::Path,
    /// Exact logical instance generation.
    pub instance: instance::Generation,
    /// Whether the opening handshake completed.
    pub opened: bool,
    /// Journaled writes not yet locally settled.
    pub pending_writes: usize,
    /// Whether a terminal observation is retained but not yet consumed.
    pub completion_recorded: bool,
}

/// Read-only snapshot of the current execution state.
///
/// It is deliberately a clone: a debugger can retain and format it without
/// borrowing the scheduler or becoming an execution authority.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionInspection {
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
    event: BoundaryObservation,
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
    /// creation sequence breaks ties deterministically. Typed on the
    /// boundary vocabulary so every future obligation (join windows,
    /// lifetimes) crosses the same frontier as call budgets.
    dated: BTreeMap<(i64, u64), Obligation>,
    creation_seq: u64,
}

impl Scheduler {
    fn new(capacity: usize) -> Self {
        Self {
            mailboxes: Mailboxes::new(capacity),
            dated: BTreeMap::new(),
            creation_seq: 0,
        }
    }

    fn schedule(&mut self, at: i64, payload: Obligation) {
        self.dated.insert((at, self.creation_seq), payload);
        self.creation_seq = self.creation_seq.wrapping_add(1);
    }

    /// The next dated event due at or before `target`, WITHOUT
    /// removing it — the reservation: an expiry that fails to commit
    /// stays scheduled and replays.
    fn peek_due(
        &self,
        target: i64,
        eligible: impl Fn(&Obligation) -> bool,
    ) -> Option<((i64, u64), Obligation)> {
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

/// Live instances, mailboxes, exchanges and obligations for one validated plan.
///
/// The caller owns the journal and supplies it to each operation. Preparation
/// checks a proposed transition; the journal commits the decision; execution
/// then applies it to memory. Recovery reconstructs that state from committed
/// records without executing their producers again.
///
/// The host drives progress and supplies logical time and Driver bindings.
/// This type does not own a background run loop or a physical clock.
#[derive(Debug)]
pub struct Execution<'plan> {
    plan: &'plan ValidatedExecutionPlan,
    original_hash: Option<crate::plan::SemanticHash>,
    retry_manager: Option<Box<crate::journal::retry_protocol::RetryView>>,
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
    owner_chains: Vec<Option<Vec<node::OwnerSlot>>>,
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
    state: State,
    poller: DriverPoller,
    /// Requests delivered to a connector, held until their call
    /// settles: the completion transaction consumes this cursor, and the
    /// connector's mailbox stays claimed the whole time.
    pending: BTreeMap<CallId, PendingCall>,
    /// Accepted inbound Calls, bounded per Connector instance by the same
    /// Worker capacity as ordinary mailboxes.
    inbound_calls: BTreeMap<InboundCallId, PendingInboundCall>,
    /// Driver terminal observations waiting for their atomic Connector
    /// transaction. A reference appears at most once.
    settled_inbound_calls: VecDeque<InboundCallObservation>,
    /// Open inbound and outbound Sessions, keyed only by the journal-derived
    /// logical reference.
    sessions: BTreeMap<SessionId, PendingSession>,
    /// Session writes whose journal-before-effect intent is already durable.
    session_writes: BTreeMap<SessionWriteId, PendingSessionWrite>,
    /// Session terminal observations waiting for their causal transaction.
    settled_sessions: VecDeque<SettledSession>,
    /// Live keyed/singleton instances whose lifecycle is observable.
    instances: BTreeMap<instance::Generation, instance::LiveInstance>,
    /// Selected successor generations not yet allowed to start.
    waiting: BTreeMap<instance::Generation, WaitingInstance>,
    /// Rebuildable index of instances that have won a close request but have
    /// not yet queued `l:stop`. `instance::Phase` remains the authority; this
    /// set keeps open-only traffic off the closure scan.
    closing: BTreeSet<instance::Generation>,
    /// Journal-before-next-work lifecycle transitions. This queue outranks
    /// ordinary deliveries so `l:start` precedes the creating arrival and a
    /// detached `l:stop` precedes the following generation.
    lifecycle: VecDeque<LifecycleWork>,
    /// Disposable index reconstructed from the journal's command protocol.
    loop_manager: Option<Box<crate::journal::loop_protocol::LoopView>>,
    /// Won closures awaiting their completion transaction — replayable:
    /// an entry leaves this queue only when its transaction commits.
    settled: VecDeque<SettledEntry>,
    /// Non-causal observations collected by internal boundary polls between
    /// drain and settlement deadlines. The next public poll returns them.
    buffered_poll: PollReport,
    /// Current logical instant in nanoseconds, supplied by the host or Lab.
    /// The semantic core never reads an ambient clock.
    now: i64,
    outputs: Vec<BoundaryOutput>,
    observed: Vec<EnvelopeView>,
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
        let slot = NodeIndex::new(u32::try_from(index).unwrap_or(u32::MAX));
        if let Some(ports) = plan.resolved_ports().node(slot) {
            for port in ports {
                if contains(port.annotations()) {
                    return Some(format!("port `{}.{}`", planned.path, port.port()));
                }
            }
        }
        for var in planned.vars() {
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
    for (handler_index, (_, handler)) in plan.handlers().enumerate() {
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
        let slot = NodeIndex::new(u32::try_from(index).unwrap_or(u32::MAX));
        if let Some(ports) = plan.resolved_ports().node(slot) {
            for port in ports {
                inspect(port.annotations());
                if planned.connector().is_some()
                    && port.direction() == node::PortDirection::Input
                    && crate::data_policy::contains_personal(port.annotations())
                {
                    requirements.external_disclosure = true;
                }
            }
        }
        for var in planned.vars() {
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
    for (_, handler) in plan.handlers() {
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
                    .is_some_and(|value| expr.ty.check_value_type(name, value).is_ok()),
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

impl<'plan> Execution<'plan> {
    fn preflight(&self, journal: &dyn Journal) -> Result<(), ExecutionFault> {
        if let Some(error) = self.unsupported_feature() {
            return Err(error);
        }
        if self.requires_personal_retention
            && journal.privacy_retention() != Some(crate::deployment::PrivacyRetention::RunScoped)
        {
            return Err(ExecutionFault::UnsupportedDataPolicy {
                location: String::from("personal-data journal retention capability"),
            });
        }
        if journal.deployment_attestation() != self.expected_deployment.as_ref() {
            return Err(ExecutionFault::DeploymentAttestationMismatch);
        }
        Ok(())
    }

    fn loop_delay_for(
        &self,
        owner: node::OwnerSlot,
        iteration: u64,
    ) -> Result<Option<i64>, ExecutionFault> {
        let path = self.plan.owners[owner.index()].path.clone();
        let Some(spec) = self.plan.owners[owner.index()].loop_spec() else {
            return Err(ExecutionFault::InvalidLoopDecision {
                owner: path,
                reason: "the Loop manager lost its validated Extension specification",
            });
        };
        let Some(delay) = &spec.delay else {
            return Ok(None);
        };
        let Some(duration) = delay.duration_after(iteration) else {
            return Err(ExecutionFault::InvalidLoopDecision {
                owner: path,
                reason: "the Loop delay contradicts its positive bounds or application ordinal",
            });
        };
        Ok(Some(duration))
    }

    fn unsupported_feature(&self) -> Option<ExecutionFault> {
        if let Some(location) = &self.missing_deployment_binding {
            return Some(ExecutionFault::MissingDeploymentBinding {
                location: location.clone(),
            });
        }
        if let Some(location) = &self.unsupported_data_policy {
            return Some(ExecutionFault::UnsupportedDataPolicy {
                location: location.clone(),
            });
        }
        self.plan.owners.iter().find_map(|owner| {
            if owner.retry_spec().is_some() && self.original_hash.is_none() {
                return Some(ExecutionFault::UnsupportedExtension { owner: owner.path.clone(), extension: node::Extension::Retry });
            }
            let spec = owner.loop_spec()?;
            let supported = (owner.kind == OwnerKind::Graph || matches!(owner.kind, crate::plan::OwnerKind::Node(slot)
                    if matches!(self.plan.nodes[slot.index()].execution, NodeExecution::Operator { .. })
                        || self.plan.nodes[slot.index()].connector().and_then(crate::plan::ConnectorSpec::call_semantics).is_some()))
                && (spec.fuel.is_some() || spec.proof.is_some());
            (!supported).then(|| ExecutionFault::UnsupportedExtension {
                owner: owner.path.clone(),
                extension: node::Extension::Loop,
            })
        })
    }

    /// Creates fresh execution state for a validated plan at logical time zero.
    /// No handler or Driver runs during construction. The validated plan
    /// guarantees that its table indexes can be used without further checks.
    /// Use [`Self::recover_from_journal`] to reconstruct a recorded execution.
    #[must_use]
    pub fn new(plan: &'plan ValidatedExecutionPlan, graph_version: GraphVersion) -> Self {
        Self::new_at(plan, graph_version, 0)
    }

    /// Creates fresh execution state with resolved deployment inputs.
    /// Personal-data policy requires a privacy declaration covering every
    /// legal regime in the plan. Secret bindings resolve named references;
    /// secret-classified input payloads are not admitted.
    #[must_use]
    pub fn new_with_deployment(
        plan: &'plan ValidatedExecutionPlan,
        graph_version: GraphVersion,
        boot: &crate::deployment::BootBindings,
    ) -> Self {
        Self::new_at_with_deployment(plan, graph_version, 0, boot)
    }

    /// Creates fresh execution state at an explicit logical instant, in
    /// nanoseconds since the Unix epoch. Hosted code maps its time source
    /// here; Lab and bare
    /// metal may keep the deterministic epoch default or supply another
    /// origin. The semantic core never reads an ambient clock.
    #[must_use]
    pub fn new_at(
        plan: &'plan ValidatedExecutionPlan,
        graph_version: GraphVersion,
        logical_time: i64,
    ) -> Self {
        Self::new_at_inner(plan, graph_version, logical_time, None)
    }

    /// Creates fresh execution state at the supplied logical instant with
    /// fixed deployment inputs.
    #[must_use]
    pub fn new_at_with_deployment(
        plan: &'plan ValidatedExecutionPlan,
        graph_version: GraphVersion,
        logical_time: i64,
        boot: &crate::deployment::BootBindings,
    ) -> Self {
        Self::new_at_inner(plan, graph_version, logical_time, Some(boot))
    }

    /// Creates execution state for the exact validated program, including its
    /// separately transported semantic identity. Retry openings bind to it; a
    /// plan body alone cannot reconstruct it.
    #[must_use]
    pub fn new_program(
        plan: &'plan ValidatedExecutionPlan,
        hash: crate::plan::SemanticHash,
        graph_version: GraphVersion,
        logical_time: i64,
        boot: Option<&crate::deployment::BootBindings>,
    ) -> Self {
        let mut vm = Self::new_at_inner(plan, graph_version, logical_time, boot);
        vm.original_hash = Some(hash);
        vm
    }

    fn new_at_inner(
        plan: &'plan ValidatedExecutionPlan,
        graph_version: GraphVersion,
        logical_time: i64,
        boot: Option<&crate::deployment::BootBindings>,
    ) -> Self {
        let mut owner_chains: Vec<Option<Vec<node::OwnerSlot>>> =
            Vec::with_capacity(plan.owners.len());
        for (index, owner) in plan.owners.iter().enumerate() {
            let slot = node::OwnerSlot::new(u32::try_from(index).unwrap_or(u32::MAX));
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
            original_hash: None,
            retry_manager: plan
                .owners
                .iter()
                .any(|owner| owner.retry_spec().is_some())
                .then(|| Box::new(crate::journal::retry_protocol::RetryView::default())),
            unsupported_data_policy: first_unsupported_data_policy_location(plan, boot),
            missing_deployment_binding: first_missing_deployment_binding(plan, boot),
            requires_personal_retention: personal_data_requirements(plan).personal,
            expected_deployment: boot.map(|boot| boot.attestation().clone()),
            owner_chains,
            has_join_activations: plan.handlers().any(|(_, handler)| {
                matches!(&handler.activation, node::HandlerActivation::Join { .. })
            }),
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
                    instance::Lifetime::Idle(_)
                        | instance::Lifetime::Fixed(_)
                        | instance::Lifetime::Tumbling(_)
                )
            }),
            has_single_pass: plan
                .owners
                .iter()
                .any(|owner| matches!(owner.instance.lifetime, instance::Lifetime::SinglePass)),
            graph_version,
            scheduler: Scheduler::new(WORKER_MAILBOX_CAPACITY),
            state: State::new(),
            poller: DriverPoller::new(),
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
            buffered_poll: PollReport::default(),
            now: logical_time,
            outputs: Vec::new(),
            observed: Vec::new(),
        }
    }

    /// Accepts an App input proposal and routes its identified envelope.
    /// Types, instances and destination capacity are checked before the
    /// journal commits the INGRESS record and its routes. Only then are
    /// the admitted instances and mailbox deliveries applied to live state.
    ///
    /// # Errors
    ///
    /// [`ExecutionFault::UnknownIngress`] when the port is not a graph
    /// input; [`ExecutionFault::IngressPinViolation`] when a materialized
    /// draft violates the ingress pin or a destination's; journal
    /// refusals and full target mailboxes. Every refusal leaves the
    /// journal untouched by this call.
    pub fn accept_input(
        &mut self,
        journal: &mut dyn Journal,
        submission: IngressProposal,
    ) -> Result<Id, ExecutionFault> {
        self.admit_proposal(journal, submission, None)
            .map(|accepted| accepted.envelope)
    }

    /// Accepts a world-originated proposal from a source Connector's output.
    /// The Driver keeps ownership of a pending proposal. This is an
    /// alternative to [`Self::accept_input`], not a second admission phase.
    /// Results of outbound Calls settle their existing exchanges through
    /// [`Self::poll_drivers`] instead of entering here as new roots.
    ///
    /// `Accepted` is returned only after the INGRESS record and every route
    /// committed. `Pending` and `Rejected` leave the journal untouched.
    ///
    /// # Errors
    ///
    /// Storage failures and machine-integrity faults remain fail-stop errors;
    /// portable peer/data refusals are represented by [`IngressAdmission`].
    pub fn accept_connector_output(
        &mut self,
        journal: &mut dyn Journal,
        proposal: &RetainedProposal,
    ) -> Result<IngressAdmission, ExecutionFault> {
        let connector_path = proposal.proposal.provenance.connector_path().clone();
        match self.admit_proposal(journal, proposal.proposal.clone(), Some(connector_path)) {
            Ok(accepted) => Ok(IngressAdmission::Accepted(accepted)),
            Err(ExecutionFault::MailboxFull { .. } | ExecutionFault::InstanceClosing { .. }) => {
                Ok(IngressAdmission::Pending)
            }
            Err(
                ExecutionFault::UnknownIngress(_) | ExecutionFault::UnknownInboundSource { .. },
            ) => Ok(IngressAdmission::Rejected(IngressRejection::UnknownPort)),
            Err(ExecutionFault::IngressPinViolation { .. }) => {
                Ok(IngressAdmission::Rejected(IngressRejection::InvalidPayload))
            }
            Err(ExecutionFault::IngressMetadataViolation { .. }) => Ok(IngressAdmission::Rejected(
                IngressRejection::InvalidMetadata,
            )),
            Err(ExecutionFault::Journal(crate::journal::Error::UnknownSession)) => Ok(
                IngressAdmission::Rejected(IngressRejection::SessionUnavailable),
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
        events: impl IntoIterator<Item = InboundCallObservation>,
    ) -> InboundCallPollReport {
        let gate = PlanCompletionCheck { plan: self.plan };
        let mut report = InboundCallPollReport::default();
        for event in events {
            let Some(pending) = self.inbound_calls.get(&event.call_id) else {
                report.uncorrelated.push(event);
                continue;
            };
            if self
                .settled_inbound_calls
                .iter()
                .any(|queued| queued.call_id == event.call_id)
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
            .filter_map(|(call_id, pending)| {
                if pending.timeout.is_some_and(|timeout| timeout.requested) {
                    return None;
                }
                let response = pending
                    .response
                    .as_ref()
                    .filter(|response| !response.started)?;
                Some(InboundCallResponse {
                    call_id: *call_id,
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
        let Some(pending) = self.inbound_calls.get_mut(&response.call_id) else {
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
    /// their Driver. Stable Call-id order makes repeated bounded polling
    /// rounds deterministic.
    #[must_use]
    pub fn ready_inbound_call_timeouts(&self) -> Vec<InboundCallTimeout> {
        self.inbound_calls
            .iter()
            .filter_map(|(call_id, pending)| {
                pending
                    .timeout
                    .filter(|timeout| timeout.requested && !timeout.started)?;
                Some(InboundCallTimeout {
                    call_id: *call_id,
                    connector_path: self.plan.nodes[pending.slot.index()].path.clone(),
                })
            })
            .collect()
    }

    /// Confirms that the exact scheduler expiry has transferred to its
    /// Driver. Stale and crossed acknowledgements are harmless refusals.
    pub fn mark_inbound_call_timeout_started(&mut self, timeout: &InboundCallTimeout) -> bool {
        let Some(pending) = self.inbound_calls.get_mut(&timeout.call_id) else {
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
        source: NodeIndex,
        instance: &instance::Generation,
        candidates: &BTreeMap<instance::Generation, InstanceCandidate>,
    ) -> Result<Option<i64>, ExecutionFault> {
        let planned = &self.plan.nodes[source.index()];
        let spec = planned
            .connector()
            .ok_or_else(|| ExecutionFault::PlanInvariant {
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
                    .ok_or_else(|| ExecutionFault::PlanInvariant {
                        node: planned.path.clone(),
                        reason: "a respond-mode inbound Call has no positive resolved timeout",
                    })
            }
            _ => Err(ExecutionFault::PlanInvariant {
                node: planned.path.clone(),
                reason: "an inbound Call source has no executable mode",
            }),
        }
    }

    /// Advances ready work once without waiting for external input.
    /// Work includes lifecycle and manager transitions, exchange settlements
    /// and Executor transactions. Returns whether ready work progressed.
    ///
    /// # Errors
    ///
    /// A refused prepared transition changes neither its input cursor nor
    /// its private state. An operation may already have committed earlier
    /// transitions; the journal determines which changes took effect.
    pub fn step(&mut self, journal: &mut dyn Journal) -> Result<bool, ExecutionFault> {
        self.step_with_bindings(journal, &mut NoOutboundBindings)
    }

    /// Advances ready work with verified outbound bindings available for
    /// Connector request preparation.
    ///
    /// # Errors
    ///
    /// The same transaction or binding-integrity failures as [`Self::step`].
    pub fn step_with_bindings(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut dyn OutboundConnectorBindings,
    ) -> Result<bool, ExecutionFault> {
        self.preflight(journal)?;
        self.synchronize_retry_closures(journal)?;
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
        let recurrence = self.next_dated_recurrence(journal)?;
        if self.has_loops && self.step_loop_manager(journal, recurrence)? {
            return Ok(true);
        }
        if self.step_retry_manager(journal, recurrence)? {
            return Ok(true);
        }
        let inbound_ready =
            self.settled_inbound_calls
                .iter()
                .enumerate()
                .find_map(|(index, event)| {
                    let pending = self.inbound_calls.get(&event.call_id)?;
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
                        .any(|write| write.intent.session_id == settled.session)
            })
        }) {
            self.settle_session_next(journal, bindings, index)?;
            return Ok(true);
        }
        if let Some(index) = self.settled.iter().position(|entry| {
            self.pending
                .get(&entry.event.call_id)
                .is_some_and(|pending| self.instances.contains_key(&pending.instance))
        }) {
            self.settle_next(journal, index)?;
            return Ok(true);
        }
        // Preserve the direct FIFO delivery path for the common
        // plan with no joins. R6's wider scan exists solely because a partial
        // join head may let later independent work run; paying for it on
        // every ordinary relay was a measured Cortex-M33 regression.
        if !self.has_join_activations {
            let instances = &self.instances;
            let Some((slot, instance, delivery)) =
                self.scheduler.mailboxes.peek_next_where(|_, reference| {
                    matches!(
                        instances.get(reference),
                        Some(instance::LiveInstance {
                            phase: instance::Phase::Open,
                            ..
                        }) | Some(instance::LiveInstance {
                            phase: instance::Phase::Closing {
                                stage: instance::ClosingStage::Draining,
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
                NodeExecution::Operator { .. } => {
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
    fn step_join_mailboxes(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut dyn OutboundConnectorBindings,
    ) -> Result<bool, ExecutionFault> {
        let mut after = None;
        while let Some((slot, instance, delivery)) =
            self.scheduler
                .mailboxes
                .next_runnable_key(after.as_ref(), |_, reference| {
                    matches!(
                        self.instances.get(reference),
                        Some(instance::LiveInstance {
                            phase: instance::Phase::Open,
                            ..
                        }) | Some(instance::LiveInstance {
                            phase: instance::Phase::Closing {
                                stage: instance::ClosingStage::Draining,
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
                NodeExecution::Operator { .. } => {
                    // An ordinary FIFO head necessarily precedes any work a
                    // later delivery could make runnable. Only a join-owned
                    // head can be partial and require the wider candidate
                    // scan, so the common single-port path stays allocation
                    // free.
                    let join_owned =
                        self.plan.nodes[slot.index()]
                            .handlers()
                            .iter()
                            .any(|handler| {
                                matches!(
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
    pub fn run_ready(&mut self, journal: &mut dyn Journal) -> Result<(), ExecutionFault> {
        self.run_ready_with_bindings(journal, &mut NoOutboundBindings)
    }

    /// Runs ready work with the verified outbound bindings available.
    ///
    /// # Errors
    ///
    /// The first transaction or binding-integrity failure.
    pub fn run_ready_with_bindings(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut dyn OutboundConnectorBindings,
    ) -> Result<(), ExecutionFault> {
        while self.step_with_bindings(journal, bindings)? {}
        Ok(())
    }

    /// One boundary round: the Driver poller starts journaled Calls and
    /// settles answers. Won closures queue as pending outcome
    /// transactions — [`Self::step`] commits them — and every
    /// non-causal observation is returned, never dropped.
    ///
    /// # Errors
    ///
    /// Driver-polling integrity faults propagate.
    pub fn poll_drivers(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut dyn OutboundConnectorBindings,
    ) -> Result<PollReport, ExecutionFault> {
        self.preflight(journal)?;
        let mut report = core::mem::take(&mut self.buffered_poll);
        report.append(self.poll_outbound_driver(journal, bindings)?);
        Ok(report)
    }

    fn poll_outbound_driver(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut dyn OutboundConnectorBindings,
    ) -> Result<PollReport, ExecutionFault> {
        let gate = PlanCompletionCheck { plan: self.plan };
        let retry = self.retry_manager.as_deref();
        let now = self.now;
        let batch = self
            .poller
            .poll_drivers_with_timing(journal, bindings, &gate, now, &mut |intent, completion| {
                retry_manager::closure_schedule(retry, intent, completion, now).map_err(|error| {
                    crate::boundary::PollFault::Journal(
                        crate::journal::Error::InvalidRetryProtocol(error),
                    )
                })
            })
            .map_err(ExecutionFault::Dispatch)?;
        self.synchronize_retry_closures(journal)?;
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
        let mut report = PollReport {
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
            .map_err(crate::boundary::PollFault::Driver)
            .map_err(ExecutionFault::Dispatch)?;
        for event in session_events {
            match event.clone() {
                SessionObservation::Opened(session_id) => {
                    let Some(session) = self.sessions.get_mut(&session_id) else {
                        report.uncorrelated_sessions.push(event);
                        continue;
                    };
                    if session.opened {
                        report.discarded_sessions.push(event);
                        continue;
                    }
                    match journal
                        .confirm_session_open(&session_id)
                        .map_err(ExecutionFault::Journal)?
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
                SessionObservation::WriteSettled {
                    session: session_id,
                    write,
                } => {
                    if !self.sessions.contains_key(&session_id) {
                        report.uncorrelated_sessions.push(event);
                        continue;
                    }
                    let Some(pending) = self.session_writes.get(&write) else {
                        report.discarded_sessions.push(event);
                        continue;
                    };
                    if pending.intent.session_id != session_id || !pending.started {
                        report.discarded_sessions.push(event);
                        continue;
                    }
                    let slot = pending.slot;
                    let instance = pending.instance.clone();
                    match journal
                        .settle_session_write(&write, false)
                        .map_err(ExecutionFault::Journal)?
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
                SessionObservation::Exited {
                    session: session_id,
                    mut exit,
                } => {
                    let Some(session) = self.sessions.get(&session_id) else {
                        report.uncorrelated_sessions.push(event);
                        continue;
                    };
                    if self
                        .settled_sessions
                        .iter()
                        .any(|settled| settled.session == session_id)
                    {
                        report.discarded_sessions.push(event);
                        continue;
                    }
                    exit.ext.insert(
                        String::from("session_id"),
                        crate::envelope::MetaValue::Scalar(crate::envelope::Scalar::Text(
                            session_id.to_string(),
                        )),
                    );
                    let completion = Completion::try_new(vec![exit]).map_err(|error| {
                        ExecutionFault::PlanInvariant {
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
                            (write.intent.session_id == session_id).then_some(*reference)
                        })
                        .collect::<Vec<_>>();
                    for write in writes {
                        match journal
                            .settle_session_write(&write, true)
                            .map_err(ExecutionFault::Journal)?
                        {
                            crate::journal::CloseVerdict::Won
                            | crate::journal::CloseVerdict::AlreadyClosed => {
                                self.session_writes.remove(&write);
                            }
                        }
                    }
                    self.settled_sessions.push_back(SettledSession {
                        session: session_id,
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

    /// Re-offers bounded Session actions only when polling Drivers. A
    /// `Pending` result therefore cannot leave the same mailbox head runnable
    /// and spin the deterministic transaction drain.
    fn retry_session_actions(
        &mut self,
        bindings: &mut dyn OutboundConnectorBindings,
    ) -> Result<(), ExecutionFault> {
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
                                instance::Phase::Open
                                    | instance::Phase::Closing {
                                        stage: instance::ClosingStage::Draining,
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
            let session = self.sessions.get(&reference).cloned().ok_or_else(|| {
                ExecutionFault::PlanInvariant {
                    node: node::Path::new(["session"])
                        .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                    reason: "a pending Session opening disappeared before retry",
                }
            })?;
            let intent =
                session
                    .open_intent
                    .as_ref()
                    .ok_or_else(|| ExecutionFault::PlanInvariant {
                        node: self.plan.nodes[session.slot.index()].path.clone(),
                        reason: "a retryable Session opening lost its sealed intent",
                    })?;
            if bindings
                .open_session(intent, &session.opening)
                .map_err(crate::boundary::PollFault::Driver)
                .map_err(ExecutionFault::Dispatch)?
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
                            instance::Phase::Open
                                | instance::Phase::Closing {
                                    stage: instance::ClosingStage::Draining,
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
                .ok_or_else(|| ExecutionFault::PlanInvariant {
                    node: node::Path::new(["session"])
                        .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                    reason: "a pending Session write disappeared before retry",
                })?;
            if bindings
                .write_session(&write.intent, &write.delivery.envelope)
                .map_err(crate::boundary::PollFault::Driver)
                .map_err(ExecutionFault::Dispatch)?
                == SessionStart::Started
                && let Some(write) = self.session_writes.get_mut(&reference)
            {
                write.started = true;
            }
        }
        Ok(())
    }

    fn retry_session_closes(
        &mut self,
        bindings: &mut dyn OutboundConnectorBindings,
    ) -> Result<(), ExecutionFault> {
        let pending: Vec<SessionId> = self
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
                                instance::Phase::Closing {
                                    stage: instance::ClosingStage::Settling,
                                    ..
                                }
                            )
                        })
            })
            .map(|(reference, _)| *reference)
            .collect();
        for reference in pending {
            let session = self.sessions.get(&reference).cloned().ok_or_else(|| {
                ExecutionFault::PlanInvariant {
                    node: node::Path::new(["session"])
                        .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                    reason: "a pending Session close lost its logical session",
                }
            })?;
            let connector = self.plan.nodes[session.slot.index()].path.clone();
            if bindings
                .close_session(&connector, reference)
                .map_err(crate::boundary::PollFault::Driver)
                .map_err(ExecutionFault::Dispatch)?
                == SessionStart::Started
                && let Some(session) = self.sessions.get_mut(&reference)
            {
                session.close_started = true;
            }
        }
        Ok(())
    }

    /// Advances logical time to the absolute `target` instant in nanoseconds.
    /// Processes scheduled obligations in deadline order and runs ready work
    /// between deadlines. This may commit multiple transitions.
    ///
    /// Call budgets start at delivery, including time spent in the mailbox.
    /// At expiry, an open Call closes with a timeout and its Driver is asked
    /// to cancel. A queued Call opens and immediately closes with a timeout;
    /// an already settled obligation produces no additional record.
    ///
    /// # Errors
    ///
    /// Journal refusals and faulting steps, as in [`Self::run_ready`].
    pub fn advance_to(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut dyn OutboundConnectorBindings,
        target: i64,
    ) -> Result<(), ExecutionFault> {
        let mut advance = TimeAdvance::new(target);
        while advance.advance_once(self, journal, bindings)? {}
        Ok(())
    }

    fn peek_due(&self, target: i64) -> Option<((i64, u64), Obligation)> {
        self.scheduler.peek_due(target, |payload| {
            !matches!(payload, Obligation::InstanceLifetime { instance, .. }
                if self.waiting.contains_key(instance)
                    || self.lifecycle.iter().any(|work|
                        work.instance == *instance && work.lifecycle == node::Lifecycle::Start))
        })
    }

    /// Resolves one expired budget wherever the obligation lives.
    fn expire(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut dyn OutboundConnectorBindings,
        payload: Obligation,
    ) -> Result<(), ExecutionFault> {
        let (connector, edge, delivery, instance) = match payload {
            Obligation::DeliveryBudget {
                connector,
                edge,
                delivery,
                instance,
            } => (connector, edge, delivery, instance),
            Obligation::InboundCallBudget { call_id, instance } => {
                if self
                    .settled_inbound_calls
                    .iter()
                    .any(|event| event.call_id == call_id)
                {
                    return Ok(());
                }
                let Some(pending) = self.inbound_calls.get_mut(&call_id) else {
                    return Ok(());
                };
                if pending.instance != instance {
                    return Ok(());
                }
                let Some(timeout) = &mut pending.timeout else {
                    return Err(ExecutionFault::PlanInvariant {
                        node: self.plan.nodes[pending.slot.index()].path.clone(),
                        reason: "an ack-only inbound Call carried a request deadline",
                    });
                };
                timeout.requested = true;
                return Ok(());
            }
            Obligation::JoinWithin {
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
            Obligation::LoopDelay { .. } | Obligation::RetryDelay { .. } => {
                // The wakeup only dates eligibility. The manager rechecks the
                // committed run, generation and deadline before any admission.
                return Ok(());
            }
            Obligation::InstanceLifetime {
                owner,
                instance,
                cause,
                deadline,
            } => {
                self.expire_instance(journal, owner, instance, cause, deadline)?;
                return Ok(());
            }
            Obligation::InstanceClosing {
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
        // envelope both: a fan-out of one envelope creates one budget
        // per receiving edge, and each expires its own call.
        if let Some(call_id) = self
            .pending
            .iter()
            .find(|(_, held)| {
                held.instance == instance
                    && held.delivery.edge == edge
                    && held.delivery.envelope.meta().sys().id() == delivery
            })
            .map(|(call_id, _)| *call_id)
        {
            // A retained closure can still await its completion transaction.
            // Its old deadline is inert, including after recovery: neither a
            // second terminal projection nor another Driver cancel is due.
            let mut closed = false;
            journal
                .visit_records(&mut |record| {
                    if let crate::journal::Record::Egress(crate::journal::EgressRecord::Call(
                        call,
                    )) = record
                        && call.intent.call_id == call_id
                        && call.status != crate::journal::CallStatus::Open
                    {
                        closed = true;
                    }
                })
                .map_err(ExecutionFault::Journal)?;
            if closed {
                return Ok(());
            }
            // Contract projection is pure, but it can still expose a broken
            // binding. Prove it before the close-once mutation so an
            // integrity refusal cannot strand a journaled obligation.
            let completion = bindings
                .scheduler_completion(&connector, SchedulerCompletion::TimeoutExpired)
                .map_err(crate::boundary::PollFault::Driver)
                .map_err(ExecutionFault::Dispatch)?;
            // Cancellation is a fallible Driver boundary. Prove it before
            // close-once so a refusal leaves both the call and its deadline
            // replayable; no terminal fact can strand a claimed mailbox.
            bindings
                .cancel(&connector, &call_id)
                .map_err(crate::boundary::PollFault::Driver)
                .map_err(ExecutionFault::Dispatch)?;
            let intent = journal
                .find_call(&call_id)
                .ok_or(ExecutionFault::MissingPending(call_id))?;
            let schedule = retry_manager::closure_schedule(
                self.retry_manager.as_deref(),
                &intent,
                &completion,
                self.now,
            )
            .map_err(retry_manager::error)?;
            match journal
                .close_egress(
                    &call_id,
                    crate::journal::Closure::Timeout,
                    self.now,
                    schedule,
                )
                .map_err(ExecutionFault::Journal)?
            {
                crate::journal::CloseVerdict::AlreadyClosed => return Ok(()),
                crate::journal::CloseVerdict::Won => {}
            }
            self.synchronize_retry_closures(journal)?;
            let intent = journal
                .find_call(&call_id)
                .unwrap_or_else(|| unreachable!("a closed call is journaled"));
            self.settled.push_back(SettledEntry {
                event: BoundaryObservation {
                    call_id,
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
                unreachable!("budgets apply to connectors only")
            };
            if !spec.roles.has_outbound() || spec.exchange != ConnectorExchange::Call {
                return Err(ExecutionFault::PlanInvariant {
                    node: planned.path.clone(),
                    reason: "a request requires an outbound Call Contract",
                });
            }
            if let Some(intent) = self.pinned_retry_intent(&instance, queued) {
                Ok(intent)
            } else {
                let resolved_settings = self.resolve_invocation_settings(
                    &instance,
                    spec,
                    &planned.path,
                    &queued.envelope,
                )?;
                match bindings
                    .prepare_request(&planned.path, queued.envelope.data(), &resolved_settings)
                    .map_err(crate::boundary::PollFault::Driver)
                    .map_err(ExecutionFault::Dispatch)?
                {
                    RequestPreparation::Rejected(completion) => Err(completion),
                    RequestPreparation::Prepared(prepared) => {
                        let emitting_id = queued.envelope.meta().sys().id();
                        Ok(CallOpen {
                            call_id: ids::call_id(
                                ids::delivery_id(&queued.edge, emitting_id),
                                prepared.digest(),
                            ),
                            emitting_id,
                            edge: queued.edge.clone(),
                            connector_path: planned.path.clone(),
                            prepared,
                            idempotency_key: None,
                            resolved_settings,
                            logical_time: self.now,
                        })
                    }
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
                    let executor = TransactionPreparer::at(self.plan, self.graph_version, self.now);
                    let trigger = node::Trigger::Port(queued.port.clone());
                    let mut outcome = executor
                        .connector_completion(slot, &instance, queued, &trigger, &completion)
                        .map_err(ExecutionFault::Fault)?;
                    self.prepare_call_completion(slot, &instance, queued, &mut outcome.record)?;
                    outcome
                };
                let prepared = self.validate_and_prepare_emissions(
                    &&*journal,
                    slot,
                    &instance,
                    &outcome.record,
                )?;
                self.retain_emission_routes(&mut outcome.record, &prepared)?;
                journal
                    .commit(outcome.record.clone())
                    .map_err(ExecutionFault::Journal)?;
                if outcome.record.loop_protocol().is_some() {
                    self.refresh_loop_manager(journal)?;
                }
                let Some(_) = self
                    .scheduler
                    .mailboxes
                    .remove_by_ref(slot, &instance, &edge, delivery)
                else {
                    return Err(ExecutionFault::PlanInvariant {
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
                    .map_err(crate::boundary::PollFault::Driver)
                    .map_err(ExecutionFault::Dispatch)?;
                self.admit_expiring_call_loop(journal, slot, &instance, &edge, delivery)?;
                let queued = self
                    .scheduler
                    .mailboxes
                    .find_by_ref(slot, &instance, &edge, delivery)
                    .cloned()
                    .ok_or_else(|| {
                        retry_manager::error(crate::journal::retry_protocol::Error::OpeningIdentity)
                    })?;
                self.open_retry_operation(journal, slot, &instance, &queued, &intent)?;
                let attempt = self.retry_attempt(&instance, &queued);
                let call_id = intent.call_id;
                let connector_path = intent.connector_path.clone();
                let emitting_id = intent.emitting_id;
                // Idempotent open: a replay after a refused close
                // finds its own open fact and proceeds to close it.
                if journal.find_call(&call_id).is_none() {
                    let mut next = self.retry_manager.clone();
                    if let (Some(attempt), Some(fold)) = (attempt, next.as_deref_mut()) {
                        fold.open_call(attempt, &intent)
                            .map_err(retry_manager::error)?;
                    }
                    journal
                        .open_egress(intent.clone(), attempt)
                        .map_err(ExecutionFault::Journal)?;
                    self.retry_manager = next;
                }
                let schedule = retry_manager::closure_schedule(
                    self.retry_manager.as_deref(),
                    &intent,
                    &completion,
                    self.now,
                )
                .map_err(retry_manager::error)?;
                match journal
                    .close_egress(
                        &call_id,
                        crate::journal::Closure::Timeout,
                        self.now,
                        schedule,
                    )
                    .map_err(ExecutionFault::Journal)?
                {
                    crate::journal::CloseVerdict::Won => {}
                    // A replay can find its own earlier closure.
                    crate::journal::CloseVerdict::AlreadyClosed => {}
                }
                self.synchronize_retry_closures(journal)?;
                // The facts hold; only now does the delivery leave
                // the queue and join the settled path.
                let Some(queued) = self
                    .scheduler
                    .mailboxes
                    .remove_by_ref(slot, &instance, &edge, delivery)
                else {
                    return Err(ExecutionFault::PlanInvariant {
                        node: self.plan.nodes[slot.index()].path.clone(),
                        reason: "a reserved timeout delivery disappeared before settlement",
                    });
                };
                self.pending.insert(
                    call_id,
                    PendingCall {
                        slot,
                        delivery: queued,
                        instance,
                    },
                );
                self.settled.push_back(SettledEntry {
                    event: BoundaryObservation {
                        call_id,
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

    fn expire_session_handshake(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut dyn OutboundConnectorBindings,
        slot: NodeIndex,
        instance: instance::Generation,
        edge: &str,
        delivery: Id,
    ) -> Result<(), ExecutionFault> {
        let connector = self.plan.nodes[slot.index()].path.clone();
        if let Some(session_id) = self
            .sessions
            .iter()
            .find(|(_, session)| {
                session.slot == slot
                    && session.instance == instance
                    && session.opening.meta().sys().id() == delivery
            })
            .map(|(reference, _)| *reference)
        {
            let session = self.sessions.get(&session_id).cloned().ok_or_else(|| {
                ExecutionFault::PlanInvariant {
                    node: connector.clone(),
                    reason: "a due Session handshake lost its logical session",
                }
            })?;
            if session.opened
                || self
                    .settled_sessions
                    .iter()
                    .any(|settled| settled.session == session_id)
            {
                return Ok(());
            }
            let completion = bindings
                .scheduler_session_completion(&connector, SchedulerCompletion::TimeoutExpired)
                .map_err(crate::boundary::PollFault::Driver)
                .map_err(ExecutionFault::Dispatch)?;
            if session.opening_started {
                let _ = bindings
                    .close_session(&connector, session_id)
                    .map_err(crate::boundary::PollFault::Driver)
                    .map_err(ExecutionFault::Dispatch)?;
            } else if !session.opening_claimed {
                let retained = self
                    .scheduler
                    .mailboxes
                    .remove_by_ref(slot, &instance, edge, delivery)
                    .ok_or_else(|| ExecutionFault::PlanInvariant {
                        node: connector.clone(),
                        reason: "a due pending Session opening lost its mailbox delivery",
                    })?;
                if let Some(session) = self.sessions.get_mut(&session_id) {
                    session.opening_delivery = Some(retained);
                }
            }
            self.settled_sessions.push_back(SettledSession {
                session: session_id,
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
            // handshake deadline. Once this instance owns an opening, later
            // message deadlines are intentionally stale rather than a second
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
        let spec = self.plan.nodes[slot.index()].connector().ok_or_else(|| {
            ExecutionFault::PlanInvariant {
                node: connector.clone(),
                reason: "a Session handshake deadline names a non-connector",
            }
        })?;
        let settings =
            self.resolve_invocation_settings(&instance, spec, &connector, &queued.envelope)?;
        let preparation = bindings
            .prepare_session_open(&connector, queued.envelope.data(), &settings)
            .map_err(crate::boundary::PollFault::Driver)
            .map_err(ExecutionFault::Dispatch)?;
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
        let session_id = ids::outbound_session_id(
            &connector,
            queued.envelope.meta().sys().id(),
            prepared.digest(),
        );
        let intent = SessionOpen {
            session_id,
            emitting_id: queued.envelope.meta().sys().id(),
            connector_path: connector.clone(),
            prepared,
        };
        journal
            .open_outbound_session(intent)
            .map_err(ExecutionFault::Journal)?;
        let retained = self
            .scheduler
            .mailboxes
            .remove_by_ref(slot, &instance, edge, delivery)
            .ok_or_else(|| ExecutionFault::PlanInvariant {
                node: connector.clone(),
                reason: "a due queued Session opening disappeared after its WAL fact",
            })?;
        self.sessions.insert(
            session_id,
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
            .map_err(crate::boundary::PollFault::Driver)
            .map_err(ExecutionFault::Dispatch)?;
        self.settled_sessions.push_back(SettledSession {
            session: session_id,
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
        owner: node::OwnerSlot,
        reference: instance::Generation,
        cause: Id,
        deadline: InstanceDeadline,
    ) -> Result<(), ExecutionFault> {
        let current = self.instances.get(&reference).filter(|instance| {
            instance.owner == owner
                && matches!(instance.phase, instance::Phase::Open)
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

    /// Returns the current logical instant in nanoseconds since the Unix epoch.
    #[must_use]
    pub fn logical_time(&self) -> i64 {
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

    /// Read-only envelope views in observation order: emissions at their
    /// source ports and deliveries at their destinations.
    #[must_use]
    pub fn envelope_views(&self) -> &[EnvelopeView] {
        &self.observed
    }

    /// Read-only operations from the accepted journal fold. Inspection borrows
    /// these states without copying pinned requests or altering scheduling.
    pub fn retry_operations(
        &self,
    ) -> impl Iterator<
        Item = (
            &crate::journal::retry_protocol::OperationId,
            &crate::journal::retry_protocol::Operation,
        ),
    > {
        self.retry_manager.iter().flat_map(|fold| fold.operations())
    }

    /// Clones one read-only debugger view without changing scheduler state.
    #[must_use]
    pub fn inspection(&self) -> ExecutionInspection {
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
                    instance::Phase::Open => InstancePhaseInspection::Open,
                    instance::Phase::Closing {
                        stage: instance::ClosingStage::Draining,
                        ..
                    } => InstancePhaseInspection::Draining,
                    instance::Phase::Closing {
                        stage: instance::ClosingStage::Settling,
                        ..
                    } => InstancePhaseInspection::Settling,
                    instance::Phase::Stopping { .. } => InstancePhaseInspection::Stopping,
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
                completion_recorded: self
                    .settled
                    .iter()
                    .any(|entry| entry.event.call_id == *reference),
            }
        }));
        calls.extend(self.inbound_calls.iter().map(|(reference, pending)| {
            CallInspection {
                reference: CallReferenceInspection::Inbound(*reference),
                direction: CallDirectionInspection::Inbound,
                connector: self.plan.nodes[pending.slot.index()].path.clone(),
                instance: pending.instance.clone(),
                completion_recorded: self
                    .settled_inbound_calls
                    .iter()
                    .any(|event| event.call_id == *reference),
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
                    .filter(|write| write.intent.session_id == *reference)
                    .count(),
                completion_recorded: self
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
        ExecutionInspection {
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
                    .any(|entry| entry.event.call_id == **reference)
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
            || self.retry_has_work(None)
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
                        Some(instance::LiveInstance {
                            phase: instance::Phase::Open,
                            ..
                        }) | Some(instance::LiveInstance {
                            phase: instance::Phase::Closing {
                                stage: instance::ClosingStage::Draining,
                                ..
                            },
                            ..
                        })
                    )
                })
        {
            if !matches!(
                self.plan.nodes[slot.index()].execution,
                NodeExecution::Operator { .. }
            ) {
                return false;
            }
            let join_owned = self.plan.nodes[slot.index()]
                .handlers()
                .iter()
                .any(|handler| {
                    matches!(
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
    /// use this distinction to pause when only external input or a future
    /// deadline can make further work ready.
    #[must_use]
    pub fn has_ready_work(&self) -> bool {
        if !self.lifecycle.is_empty()
            || self.instance_command_ready()
            || self.single_pass_close().is_some()
            || self.loop_manager_ready()
            || self.retry_manager_ready()
            || self.settled_inbound_calls.iter().any(|event| {
                self.inbound_calls
                    .get(&event.call_id)
                    .is_some_and(|pending| self.instances.contains_key(&pending.instance))
            })
            || self.settled_sessions.iter().any(|settled| {
                self.sessions.get(&settled.session).is_some_and(|session| {
                    self.instances.contains_key(&session.instance)
                        && !self
                            .session_writes
                            .values()
                            .any(|write| write.intent.session_id == settled.session)
                })
            })
            || self.settled.iter().any(|entry| {
                self.pending
                    .get(&entry.event.call_id)
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
                        Some(instance::LiveInstance {
                            phase: instance::Phase::Open,
                            ..
                        }) | Some(instance::LiveInstance {
                            phase: instance::Phase::Closing {
                                stage: instance::ClosingStage::Draining,
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
                        Some(instance::LiveInstance {
                            phase: instance::Phase::Open,
                            ..
                        }) | Some(instance::LiveInstance {
                            phase: instance::Phase::Closing {
                                stage: instance::ClosingStage::Draining,
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
            let join_owned = self.plan.nodes[slot.index()]
                .handlers()
                .iter()
                .any(|handler| {
                    matches!(
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

    /// Whether calls are open against the world — work [`Self::run_ready`]
    /// cannot finish alone.
    #[must_use]
    pub fn has_open_obligations(&self) -> bool {
        !self.pending.is_empty()
            || !self.inbound_calls.is_empty()
            || !self.sessions.is_empty()
            || !self.session_writes.is_empty()
    }

    fn instance_has_direct_work(&self, reference: &instance::Generation) -> bool {
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
            || self.retry_has_work(Some(reference))
            || self
                .lifecycle
                .iter()
                .any(|work| work.instance == *reference)
    }

    fn single_pass_close(&self) -> Option<(&instance::Generation, &Envelope)> {
        if !self.has_single_pass {
            return None;
        }
        self.instances.iter().find_map(|(reference, instance)| {
            (matches!(instance.phase, instance::Phase::Open)
                && matches!(
                    self.plan.owners[instance.owner.index()].instance.lifetime,
                    instance::Lifetime::SinglePass
                )
                && !self.lifecycle.iter().any(|work| {
                    work.instance == *reference && work.lifecycle == node::Lifecycle::Start
                }))
            .then_some((reference, &instance.created_by))
        })
    }

    fn is_descendant_instance(
        &self,
        child: &instance::Generation,
        ancestor: &instance::Generation,
    ) -> bool {
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

    fn is_within_instance(
        &self,
        candidate: &instance::Generation,
        scope: &instance::Generation,
    ) -> bool {
        candidate == scope || self.is_descendant_instance(candidate, scope)
    }

    fn instance_has_descendants(&self, reference: &instance::Generation) -> bool {
        self.instances.keys().any(|candidate| {
            candidate != reference && self.is_descendant_instance(candidate, reference)
        })
    }

    fn closing_deadline_ids(
        &self,
        reference: &instance::Generation,
    ) -> Vec<(crate::journal::DeadlineKind, Id)> {
        let Some(instance) = self.instances.get(reference) else {
            return Vec::new();
        };
        let Some(_) = self.plan.owners[instance.owner.index()].instance.closing else {
            return Vec::new();
        };
        vec![
            (
                crate::journal::DeadlineKind::InstanceDrain,
                ids::instance_closing_id(reference, crate::journal::DeadlineKind::InstanceDrain),
            ),
            (
                crate::journal::DeadlineKind::InstanceSettlement,
                ids::instance_closing_id(
                    reference,
                    crate::journal::DeadlineKind::InstanceSettlement,
                ),
            ),
        ]
    }

    fn schedule_closing_deadlines(&mut self, reference: &instance::Generation, cause: Id) {
        let Some(instance) = self.instances.get(reference) else {
            return;
        };
        let Some(policy) = self.plan.owners[instance.owner.index()].instance.closing else {
            return;
        };
        let drain_at = self.now.saturating_add(policy.drain_for);
        self.scheduler.schedule(
            drain_at,
            Obligation::InstanceClosing {
                instance: reference.clone(),
                cause,
                deadline: ClosingDeadline::Drain,
            },
        );
        self.scheduler.schedule(
            drain_at.saturating_add(policy.settle_for),
            Obligation::InstanceClosing {
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
        reference: &instance::Generation,
        cause: Envelope,
        command: Option<Id>,
    ) -> Result<(), ExecutionFault> {
        if !self
            .instances
            .get(reference)
            .is_some_and(|instance| matches!(instance.phase, instance::Phase::Open))
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
            retry_record: None,
            loop_record: None,
            logical_time: None,
            deadlines: None,
            instance_commands: command.map(|id| {
                Box::new(crate::journal::instance_stop::InstanceCommands {
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
            deadline_ids: self.closing_deadline_ids(reference),
        };
        self.retain_deadlines(&mut record, self.closing_deadlines(reference))?;
        journal.commit(record).map_err(ExecutionFault::Journal)?;
        self.activate_close(reference, cause.clone());
        self.schedule_closing_deadlines(reference, cause.meta().sys().id());
        Ok(())
    }

    /// The first close request wins causally. A Graph request marks every live
    /// descendant, but reclamation remains inner-first: the parent cannot
    /// queue `l:stop` while a child bundle or child stop fact is outstanding.
    fn activate_close(&mut self, reference: &instance::Generation, cause: Envelope) {
        let closes_subtree = self.instances.get(reference).is_some_and(|instance| {
            self.plan.owners[instance.owner.index()].kind == OwnerKind::Graph
        });
        let targets: Vec<instance::Generation> = self
            .instances
            .keys()
            .filter(|candidate| {
                *candidate == reference
                    || (closes_subtree && self.is_descendant_instance(candidate, reference))
            })
            .cloned()
            .collect();
        for target in targets {
            if let Some(fold) = self.retry_manager.as_deref_mut() {
                fold.begin_closing(&target);
            }
            let widen = self
                .instances
                .get(&target)
                .and_then(|instance| match &instance.phase {
                    instance::Phase::Closing { scope, .. }
                        if self.is_within_instance(scope, reference) =>
                    {
                        Some(true)
                    }
                    _ => None,
                });
            if let Some(instance) = self.instances.get_mut(&target) {
                match &mut instance.phase {
                    instance::Phase::Open => {
                        instance.phase = instance::Phase::Closing {
                            cause: cause.clone(),
                            scope: reference.clone(),
                            stage: instance::ClosingStage::Draining,
                        };
                        self.closing.insert(target);
                    }
                    instance::Phase::Closing { scope, .. } if widen == Some(true) => {
                        *scope = reference.clone();
                    }
                    instance::Phase::Closing { .. } | instance::Phase::Stopping { .. } => {}
                }
            }
        }
        if let Some(queue) = self.instance_commands.as_mut() {
            queue.retain(|command| {
                self.instances
                    .get(&command.target)
                    .is_some_and(|instance| matches!(instance.phase, instance::Phase::Open))
            });
        }
    }

    fn progress_closures(&mut self) {
        if self.closing.is_empty() {
            return;
        }
        let mut stop_receivers = BTreeMap::new();
        let mut ready: Vec<(usize, instance::Generation)> = self
            .closing
            .iter()
            .filter_map(|reference| {
                self.instances
                    .get(reference)
                    .map(|instance| (reference, instance))
            })
            .filter(|(reference, _)| {
                let scope = match &self.instances[*reference].phase {
                    instance::Phase::Closing { scope, .. } => scope,
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

    fn scope_has_work(&self, scope: &instance::Generation) -> bool {
        self.instances.keys().any(|reference| {
            self.is_within_instance(reference, scope) && self.instance_has_direct_work(reference)
        })
    }

    fn residualize_scope(&mut self, scope: &instance::Generation) {
        let targets: Vec<(NodeIndex, instance::Generation)> = self
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

    fn begin_settlement(&mut self, scope: &instance::Generation) {
        self.residualize_scope(scope);
        let targets: Vec<_> = self
            .instances
            .keys()
            .filter(|candidate| self.is_within_instance(candidate, scope))
            .cloned()
            .collect();
        for target in targets {
            if let Some(instance) = self.instances.get_mut(&target)
                && let instance::Phase::Closing { stage, .. } = &mut instance.phase
            {
                *stage = instance::ClosingStage::Settling;
            }
        }
    }

    fn expire_closing(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut dyn OutboundConnectorBindings,
        reference: &instance::Generation,
        cause: Id,
        deadline: ClosingDeadline,
    ) -> Result<(), ExecutionFault> {
        let current = self
            .instances
            .get(reference)
            .and_then(|instance| match &instance.phase {
                instance::Phase::Closing {
                    cause: current_cause,
                    scope,
                    stage,
                } if scope == reference && current_cause.meta().sys().id() == cause => Some(*stage),
                instance::Phase::Open
                | instance::Phase::Closing { .. }
                | instance::Phase::Stopping { .. } => None,
            });
        let Some(stage) = current else {
            return Ok(());
        };
        match deadline {
            ClosingDeadline::Drain if stage == instance::ClosingStage::Draining => {
                self.begin_settlement(reference);
                let calls: Vec<CallId> = self
                    .pending
                    .iter()
                    .filter(|(_, pending)| self.is_within_instance(&pending.instance, reference))
                    .map(|(call_id, _)| *call_id)
                    .collect();
                for call_id in calls {
                    let pending = self
                        .pending
                        .get(&call_id)
                        .ok_or(ExecutionFault::MissingPending(call_id))?;
                    let connector = &self.plan.nodes[pending.slot.index()].path;
                    bindings
                        .cancel(connector, &call_id)
                        .map_err(crate::boundary::PollFault::Driver)
                        .map_err(ExecutionFault::Dispatch)?;
                }
                // Session closure is requested in canonical logical-reference
                // order. A bounded Driver may defer the handoff; the next
                // Driver poll retries the same already-live Session.
                let sessions: Vec<SessionId> = self
                    .sessions
                    .iter()
                    .filter(|(_, session)| self.is_within_instance(&session.instance, reference))
                    .map(|(session, _)| *session)
                    .collect();
                for session_id in sessions {
                    let session = self.sessions.get(&session_id).cloned().ok_or_else(|| {
                        ExecutionFault::PlanInvariant {
                            node: node::Path::new(["session"])
                                .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                            reason: "a Closing Session vanished before close handoff",
                        }
                    })?;
                    let connector = self.plan.nodes[session.slot.index()].path.clone();
                    if bindings
                        .close_session(&connector, session_id)
                        .map_err(crate::boundary::PollFault::Driver)
                        .map_err(ExecutionFault::Dispatch)?
                        == SessionStart::Started
                        && let Some(session) = self.sessions.get_mut(&session_id)
                    {
                        session.close_started = true;
                    }
                }
            }
            ClosingDeadline::Settlement => {
                self.residualize_scope(reference);
                let calls: Vec<CallId> = self
                    .pending
                    .iter()
                    .filter(|(_, pending)| self.is_within_instance(&pending.instance, reference))
                    .map(|(call_id, _)| *call_id)
                    .collect();
                for call_id in calls {
                    let pending = self
                        .pending
                        .get(&call_id)
                        .ok_or(ExecutionFault::MissingPending(call_id))?;
                    let connector = self.plan.nodes[pending.slot.index()].path.clone();
                    let completion = bindings
                        .scheduler_completion(&connector, SchedulerCompletion::OutcomeUnknown)
                        .map_err(crate::boundary::PollFault::Driver)
                        .map_err(ExecutionFault::Dispatch)?;
                    match journal
                        .abandon_egress(&call_id, self.now)
                        .map_err(ExecutionFault::Journal)?
                    {
                        crate::journal::CloseVerdict::AlreadyClosed => {}
                        crate::journal::CloseVerdict::Won => {
                            self.settled.push_back(SettledEntry {
                                event: BoundaryObservation {
                                    call_id,
                                    connector_path: connector.clone(),
                                    emitting_id: pending.delivery.envelope.meta().sys().id(),
                                    completion,
                                },
                                trigger: node::Trigger::Lifecycle(node::Lifecycle::Recover),
                            });
                        }
                    }
                }
                let writes: Vec<SessionWriteId> = self
                    .session_writes
                    .iter()
                    .filter(|(_, write)| self.is_within_instance(&write.instance, reference))
                    .map(|(write, _)| *write)
                    .collect();
                for write in writes {
                    match journal
                        .settle_session_write(&write, true)
                        .map_err(ExecutionFault::Journal)?
                    {
                        crate::journal::CloseVerdict::AlreadyClosed => {}
                        crate::journal::CloseVerdict::Won => {
                            self.session_writes.remove(&write);
                        }
                    }
                }
                let sessions: Vec<SessionId> = self
                    .sessions
                    .iter()
                    .filter(|(_, session)| self.is_within_instance(&session.instance, reference))
                    .map(|(session, _)| *session)
                    .collect();
                for session_id in sessions {
                    if self
                        .settled_sessions
                        .iter()
                        .any(|settled| settled.session == session_id)
                    {
                        continue;
                    }
                    let session = self.sessions.get(&session_id).ok_or_else(|| {
                        ExecutionFault::PlanInvariant {
                            node: node::Path::new(["session"])
                                .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                            reason: "a settling Session lost its logical record",
                        }
                    })?;
                    let connector = self.plan.nodes[session.slot.index()].path.clone();
                    let completion = bindings
                        .scheduler_session_completion(
                            &connector,
                            SchedulerCompletion::OutcomeUnknown,
                        )
                        .map_err(crate::boundary::PollFault::Driver)
                        .map_err(ExecutionFault::Dispatch)?;
                    // The committed Closing request sealed this deadline.
                    // Forced cleanup must not wait for exit-route capacity or
                    // terminal storage; it proves no successful outcome.
                    bindings
                        .abort_session(&connector, session_id)
                        .map_err(crate::boundary::PollFault::Driver)
                        .map_err(ExecutionFault::Dispatch)?;
                    self.settled_sessions.push_back(SettledSession {
                        session: session_id,
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

    fn begin_stop(&mut self, reference: &instance::Generation) {
        let Some(bundle) = self.instances.get_mut(reference) else {
            return;
        };
        let instance::Phase::Closing { cause, scope, .. } = &bundle.phase else {
            return;
        };
        let cause = cause.clone();
        let scope = scope.clone();
        self.closing.remove(reference);
        bundle.phase = instance::Phase::Stopping {
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

    fn fire_lifecycle_next(&mut self, journal: &mut dyn Journal) -> Result<(), ExecutionFault> {
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
                    retry_record: None,
                    loop_record: None,
                    logical_time: None,
                    deadlines: None,
                    instance_commands: None,
                    trigger: node::Trigger::Lifecycle(work.lifecycle),
                    cause: Some(work.cause.meta().sys().id()),
                    consumed: vec![],
                    join_inputs: vec![],
                    state_delta: vec![],
                    emissions: vec![],
                    routes: vec![],
                    deadline_ids: vec![],
                },
                None,
                None,
            ),
            OwnerKind::Node(slot) => {
                let bundle = self.instances.get(&work.instance).ok_or_else(|| {
                    ExecutionFault::PlanInvariant {
                        node: self.plan.nodes[slot.index()].path.clone(),
                        reason: "lifecycle work lost its installed instance bundle",
                    }
                })?;
                let state = self
                    .state
                    .cell_unchecked(self.plan.plan(), slot, &work.instance);
                let executor = TransactionPreparer::at(self.plan, self.graph_version, self.now);
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
                    .map_err(ExecutionFault::Fault)?;
                let prepared = self.validate_and_prepare_emissions(
                    &&*journal,
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
        journal
            .commit(record.clone())
            .map_err(ExecutionFault::Journal)?;
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
            if matches!(owner.instance.lifetime, instance::Lifetime::SinglePass) {
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
            self.promote_waiting(&&*journal)?;
        }
        self.progress_closures();
        Ok(())
    }

    fn promote_waiting(&mut self, journal: &dyn InstanceAdmissions) -> Result<(), ExecutionFault> {
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
                    .instance_generation_for_admission(reference.address())
                    .map_err(ExecutionFault::Journal)?;
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
                instance::LiveInstance {
                    owner,
                    parent: waiting.candidate.parent.clone(),
                    segments: waiting.candidate.frames,
                    values: waiting.candidate.values,
                    settings: waiting.candidate.settings,
                    created_by: waiting.created_by.clone(),
                    last_activity: waiting.last_activity.clone(),
                    residual: Vec::new(),
                    phase: instance::Phase::Open,
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
    fn settle_next(
        &mut self,
        journal: &mut dyn Journal,
        index: usize,
    ) -> Result<(), ExecutionFault> {
        let SettledEntry { event, trigger } = self
            .settled
            .get(index)
            .cloned()
            .unwrap_or_else(|| unreachable!("guarded by the caller"));
        let slot = self.slot_of(&event.connector_path);
        let Some(request) = self.pending.get(&event.call_id).cloned() else {
            return Err(ExecutionFault::MissingPending(event.call_id));
        };
        let executor = TransactionPreparer::at(self.plan, self.graph_version, self.now);
        // The completion check validated every materialized draft before the
        // closure committed. This also checks runtime-constructed
        // completions (timeouts) and completions supplied by in-memory callers.
        for output in event.completion.outputs() {
            let Data::Materialized(value) = &output.data else {
                continue;
            };
            let from = EdgeEnd {
                node: Some(slot),
                port: output.port.clone(),
            };
            crate::runtime::crossing_pins_hold(self.plan, &from, value)
                .map_err(crate::runtime::PinViolation::into_fault)
                .map_err(ExecutionFault::Fault)?;
        }
        let mut outcome = executor
            .connector_completion(
                slot,
                &request.instance,
                &request.delivery,
                &trigger,
                &event.completion,
            )
            .map_err(ExecutionFault::Fault)?;
        self.validate_call_completion(slot, &outcome.record)?;
        let retry = self.prepare_retry_completion(
            &request.instance,
            &request.delivery,
            event.call_id,
            &mut outcome.record,
        )?;
        let held = retry.as_ref().is_some_and(|(_, held)| *held);
        let prepared = if held {
            let context = self.graph_context(&request.delivery);
            if !context.is_empty() {
                outcome.record.loop_record =
                    Some(Box::new(crate::journal::loop_protocol::LoopRecord {
                        context: context.to_vec(),
                        ..Default::default()
                    }));
            }
            retry_manager::hold_emissions(&outcome.record)
        } else {
            self.prepare_call_completion(
                slot,
                &request.instance,
                &request.delivery,
                &mut outcome.record,
            )?;
            self.validate_and_prepare_emissions(
                &&*journal,
                slot,
                &request.instance,
                &outcome.record,
            )?
        };
        self.retain_emission_routes(&mut outcome.record, &prepared)?;
        let has_loop_context = outcome.record.loop_protocol().is_some();
        journal
            .commit(outcome.record)
            .map_err(ExecutionFault::Journal)?;
        if let Some((fold, _)) = retry {
            self.retry_manager = Some(fold);
        }
        if has_loop_context {
            self.refresh_loop_manager(journal)?;
        }
        self.settled.remove(index);
        self.pending.remove(&event.call_id);
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
    /// an EGRESS record.
    fn settle_inbound_call_next(
        &mut self,
        journal: &mut dyn Journal,
        index: usize,
        event: InboundCallObservation,
        pending: PendingInboundCall,
    ) -> Result<(), ExecutionFault> {
        let executor = TransactionPreparer::at(self.plan, self.graph_version, self.now);
        let mut outcome = match pending.response.as_ref() {
            Some(response) => executor.inbound_call_response_completion(
                pending.slot,
                &pending.instance,
                &pending.request,
                &response.delivery,
                event.call_id,
                &event.completion,
            ),
            None => executor.inbound_call_completion(
                pending.slot,
                &pending.instance,
                &pending.request,
                event.call_id,
                &event.completion,
            ),
        }
        .map_err(ExecutionFault::Fault)?;
        let prepared = self.validate_and_prepare_emissions(
            &&*journal,
            pending.slot,
            &pending.instance,
            &outcome.record,
        )?;
        self.retain_emission_routes(&mut outcome.record, &prepared)?;
        journal
            .commit(outcome.record)
            .map_err(ExecutionFault::Journal)?;
        self.settled_inbound_calls.remove(index);
        self.inbound_calls.remove(&event.call_id);
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
    fn settle_session_next(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut dyn OutboundConnectorBindings,
        index: usize,
    ) -> Result<(), ExecutionFault> {
        let settled = self
            .settled_sessions
            .get(index)
            .cloned()
            .unwrap_or_else(|| unreachable!("guarded by the caller"));
        let Some(session) = self.sessions.get(&settled.session).cloned() else {
            return Err(ExecutionFault::PlanInvariant {
                node: node::Path::new(["session"])
                    .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                reason: "a terminal Session observation lost its logical session",
            });
        };
        let executor = TransactionPreparer::at(self.plan, self.graph_version, self.now);
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
            .map_err(ExecutionFault::Fault)?;
        let prepared = self.validate_and_prepare_emissions(
            &&*journal,
            session.slot,
            &session.instance,
            &outcome.record,
        )?;
        self.retain_emission_routes(&mut outcome.record, &prepared)?;
        journal
            .commit(outcome.record)
            .map_err(ExecutionFault::Journal)?;
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
            .map_err(crate::boundary::PollFault::Driver)
            .map_err(ExecutionFault::Dispatch)?;
        Ok(())
    }

    /// Selects the earliest runnable fact in one Operator mailbox. Partial
    /// join arrivals are logically buffered and therefore do not block a
    /// later ready activation; ordering is by the arrival that made the work
    /// runnable, never by map or handler declaration order.
    fn operator_work(
        &self,
        slot: NodeIndex,
        instance: &instance::Generation,
    ) -> Result<Option<OperatorWork>, ExecutionFault> {
        let Some(queue) = self.scheduler.mailboxes.queued(slot, instance) else {
            return Ok(None);
        };
        let handlers = self.plan.nodes[slot.index()].handlers();
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

        let executor = TransactionPreparer::at(self.plan, self.graph_version, self.now);
        let state = self.state.cell_unchecked(self.plan.plan(), slot, instance);
        let values = self
            .instances
            .get(instance)
            .map(|bundle| &bundle.values)
            .ok_or_else(|| ExecutionFault::PlanInvariant {
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
                let mut options: Vec<Vec<envelope::Delivery>> = Vec::with_capacity(ports.len());
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
                    let inputs: Vec<envelope::Delivery> = options
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
                        .map_err(ExecutionFault::Fault)?
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
        slot: NodeIndex,
        instance: &instance::Generation,
        delivery: &envelope::Delivery,
    ) -> Result<(), ExecutionFault> {
        let executor = TransactionPreparer::at(self.plan, self.graph_version, self.now);
        let state = self.state.cell_unchecked(self.plan.plan(), slot, instance);
        let values = self
            .instances
            .get(instance)
            .map(|bundle| &bundle.values)
            .ok_or_else(|| ExecutionFault::PlanInvariant {
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
        .map_err(ExecutionFault::Fault)?;
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
            outcome
                .record
                .deadline_ids
                .extend(self.closing_deadline_ids(scope));
        }
        let prepared =
            self.validate_and_prepare_emissions(&&*journal, slot, instance, &outcome.record)?;
        self.retain_emission_routes(&mut outcome.record, &prepared)?;
        // The journal owns the full transaction after commit. Retain only the
        // small delta the in-memory state fold still needs; cloning the whole
        // record would duplicate every emitted envelope on the relay hot path.
        let state_delta = outcome.record.state_delta.clone();
        journal
            .commit(outcome.record)
            .map_err(ExecutionFault::Journal)?;
        if application.is_some() {
            self.refresh_loop_manager(journal)?;
        }
        self.scheduler.mailboxes.pop(slot, instance);
        self.state
            .apply_unchecked(self.plan.plan(), slot, instance, &state_delta);
        if let Some(scope) = close_request {
            self.activate_close(&scope, delivery.envelope.clone());
            self.schedule_closing_deadlines(&scope, delivery.envelope.meta().sys().id());
        }
        self.route_emissions(slot, instance, prepared);
        self.progress_closures();
        Ok(())
    }

    /// Routes a Connector delivery by its validated exchange cardinality.
    /// Roles decide direction; the exchange decides whether the retained
    /// obligation is one Call or one long-lived Session.
    fn fire_connector(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut dyn OutboundConnectorBindings,
        slot: NodeIndex,
        instance: &instance::Generation,
        delivery: &envelope::Delivery,
    ) -> Result<(), ExecutionFault> {
        let planned = &self.plan.nodes[slot.index()];
        let NodeExecution::Connector(spec) = &planned.execution else {
            return Err(ExecutionFault::PlanInvariant {
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
            ConnectorExchange::Event | ConnectorExchange::Call => {
                Err(ExecutionFault::PlanInvariant {
                    node: planned.path.clone(),
                    reason: "a delivery reached a Connector without a compatible world role",
                })
            }
        }
    }

    fn fire_join_operator(
        &mut self,
        journal: &mut dyn Journal,
        slot: NodeIndex,
        instance: &instance::Generation,
        activation: &node::HandlerActivation,
        trigger: &envelope::Delivery,
        inputs: &[envelope::Delivery],
    ) -> Result<(), ExecutionFault> {
        let context = self.graph_context(trigger);
        if inputs
            .iter()
            .any(|input| self.graph_context(input) != context)
        {
            return Err(ExecutionFault::InvalidLoopDecision {
                owner: instance.address().owner().clone(),
                reason: "E-LOOP-CONTEXT: join candidates belong to different applications",
            });
        }
        let application = self.loop_application(instance, trigger);
        let executor = TransactionPreparer::at(self.plan, self.graph_version, self.now);
        let state = self.state.cell_unchecked(self.plan.plan(), slot, instance);
        let values = self
            .instances
            .get(instance)
            .map(|bundle| &bundle.values)
            .ok_or_else(|| ExecutionFault::PlanInvariant {
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
            .map_err(ExecutionFault::Fault)?;
        if let Some(packet) = outcome.record.loop_record.as_deref_mut() {
            packet.context = context.to_vec();
        }
        self.retain_graph_verdicts(&mut outcome.record)?;
        let close_request = outcome.record.close_scope.clone();
        if let Some(scope) = &close_request {
            outcome
                .record
                .deadline_ids
                .extend(self.closing_deadline_ids(scope));
        }
        let prepared =
            self.validate_and_prepare_emissions(&&*journal, slot, instance, &outcome.record)?;
        self.retain_emission_routes(&mut outcome.record, &prepared)?;
        journal
            .commit(outcome.record.clone())
            .map_err(ExecutionFault::Journal)?;
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
                return Err(ExecutionFault::PlanInvariant {
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
            self.schedule_closing_deadlines(&scope, trigger.envelope.meta().sys().id());
        }
        self.route_emissions(slot, instance, prepared);
        self.progress_closures();
        Ok(())
    }

    fn evict_join_delivery(
        &mut self,
        journal: &mut dyn Journal,
        slot: NodeIndex,
        instance: &instance::Generation,
        delivery: &envelope::Delivery,
    ) -> Result<(), ExecutionFault> {
        let executor = TransactionPreparer::at(self.plan, self.graph_version, self.now);
        let mut outcome = executor
            .join_expired(slot, instance, delivery)
            .map_err(ExecutionFault::Fault)?;
        let prepared =
            self.validate_and_prepare_emissions(&&*journal, slot, instance, &outcome.record)?;
        self.retain_emission_routes(&mut outcome.record, &prepared)?;
        journal
            .commit(outcome.record.clone())
            .map_err(ExecutionFault::Journal)?;
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
            return Err(ExecutionFault::PlanInvariant {
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
        slot: NodeIndex,
        instance: &instance::Generation,
        delivery: &envelope::Delivery,
    ) -> Result<(), ExecutionFault> {
        let planned = &self.plan.nodes[slot.index()];
        let NodeExecution::Connector(spec) = &planned.execution else {
            return Err(ExecutionFault::PlanInvariant {
                node: planned.path.clone(),
                reason: "an inbound-Call response was routed to a non-connector",
            });
        };
        if !spec.roles.has_inbound()
            || spec.roles.has_outbound()
            || spec.exchange != ConnectorExchange::Call
        {
            return Err(ExecutionFault::PlanInvariant {
                node: planned.path.clone(),
                reason: "a delivery reached a connector with neither an outbound Call nor an inbound-Call response role",
            });
        }

        let Some(call_id) = self.select_inbound_call_ref(slot, spec, &delivery.envelope)? else {
            return self.discard_inbound_call_response(journal, slot, instance, delivery);
        };
        let matches_open_call = self.inbound_calls.get(&call_id).is_some_and(|call| {
            call.slot == slot
                && call.instance == *instance
                && call.response.is_none()
                && !self
                    .settled_inbound_calls
                    .iter()
                    .any(|event| event.call_id == call_id)
        });
        if !matches_open_call {
            return self.discard_inbound_call_response(journal, slot, instance, delivery);
        }
        let response = self
            .scheduler
            .mailboxes
            .pop(slot, instance)
            .ok_or_else(|| ExecutionFault::PlanInvariant {
                node: planned.path.clone(),
                reason: "an inbound-Call response disappeared before reservation",
            })?;
        let Some(call) = self.inbound_calls.get_mut(&call_id) else {
            return Err(ExecutionFault::PlanInvariant {
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
        slot: NodeIndex,
        instance: &instance::Generation,
        delivery: &envelope::Delivery,
    ) -> Result<(), ExecutionFault> {
        let planned = &self.plan.nodes[slot.index()];
        let record = crate::journal::TransactionRecord {
            owner_path: node::OwnerPath::from(&planned.path),
            instance: instance.clone(),
            parent_instance: None,
            close_scope: None,
            trigger: node::Trigger::Port(delivery.port.clone()),
            cause: Some(delivery.envelope.meta().sys().id()),
            exchange_exit: None,
            retry_record: None,
            loop_record: None,
            logical_time: None,
            deadlines: None,
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
            deadline_ids: vec![],
        };
        journal.commit(record).map_err(ExecutionFault::Journal)?;
        if self.scheduler.mailboxes.pop(slot, instance).is_none() {
            return Err(ExecutionFault::PlanInvariant {
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
    fn progress_session(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut dyn OutboundConnectorBindings,
        slot: NodeIndex,
        instance: &instance::Generation,
        delivery: &envelope::Delivery,
    ) -> Result<(), ExecutionFault> {
        let planned = &self.plan.nodes[slot.index()];
        let NodeExecution::Connector(spec) = &planned.execution else {
            return Err(ExecutionFault::PlanInvariant {
                node: planned.path.clone(),
                reason: "Session dispatch reached a non-connector",
            });
        };
        if spec.exchange != ConnectorExchange::Session || !spec.roles.has_outbound() {
            return Err(ExecutionFault::PlanInvariant {
                node: planned.path.clone(),
                reason: "a Session delivery reached a Connector without an outbound Session role",
            });
        }
        let opening =
            session_opening(&spec.contract, spec.roles, spec.exchange).ok_or_else(|| {
                ExecutionFault::PlanInvariant {
                    node: planned.path.clone(),
                    reason: "the Contract has no Session opening rule",
                }
            })?;
        let settings =
            self.resolve_invocation_settings(instance, spec, &planned.path, &delivery.envelope)?;
        let selected = self.select_session_id(spec, &delivery.envelope, &settings)?;
        if let Some(session_id) = selected {
            return if self.sessions.get(&session_id).is_some_and(|session| {
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
                    session_id,
                    &settings,
                )
            } else {
                self.discard_session_write(journal, slot, instance, delivery)
            };
        }

        let live: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, session)| session.slot == slot && session.instance == *instance)
            .map(|(reference, _)| *reference)
            .collect();
        if opening == SessionOpening::Peer {
            // A server never guesses which peer to address. The ordinary
            // consume-only discard is observable and prevents an orphan from
            // becoming a new client opening by accident.
            return self.discard_session_write(journal, slot, instance, delivery);
        }
        match live.as_slice() {
            [] => self.open_session(journal, bindings, slot, instance, delivery, &settings),
            [session_id] => {
                let pending_open = self.sessions.get(session_id).is_some_and(|session| {
                    !session.opened
                        && !session.opening_claimed
                        && session.opening.meta().sys().id() == delivery.envelope.meta().sys().id()
                });
                if pending_open {
                    self.open_session(journal, bindings, slot, instance, delivery, &settings)
                } else if self.sessions[session_id].opened {
                    self.write_session(
                        journal,
                        bindings,
                        SessionDispatch {
                            slot,
                            instance,
                            delivery,
                        },
                        *session_id,
                        &settings,
                    )
                } else {
                    Err(ExecutionFault::PlanInvariant {
                        node: planned.path.clone(),
                        reason: "a client Session opening lost its claimed mailbox",
                    })
                }
            }
            _ => Err(ExecutionFault::PlanInvariant {
                node: planned.path.clone(),
                reason: "a client Connector owns more than one live Session in one instance",
            }),
        }
    }

    fn select_session_id(
        &self,
        spec: &crate::plan::ConnectorSpec,
        envelope: &Envelope,
        settings: &BTreeMap<String, crate::value::Value>,
    ) -> Result<Option<SessionId>, ExecutionFault> {
        // The resident child belongs to this exact Node instance. An upstream
        // socket's session_id is provenance, not an address for this resource.
        if spec.contract.as_str() == crate::connector::PROCESS_SERVICE_CONNECTOR_ID {
            return Ok(None);
        }
        if spec.attribute("correlate").is_some() {
            let Some(crate::value::Value::Text(value)) = settings.get("correlate") else {
                return Err(ExecutionFault::PlanInvariant {
                    node: node::Path::new(["session"])
                        .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                    reason: "a Session correlator did not resolve to text",
                });
            };
            return Ok(SessionId::parse_projection(value));
        }
        Ok(envelope
            .meta()
            .ext()
            .get("session_id")
            .and_then(|value| match value {
                crate::envelope::MetaValue::Scalar(crate::envelope::Scalar::Text(value)) => {
                    SessionId::parse_projection(value)
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

    fn open_session(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut dyn OutboundConnectorBindings,
        slot: NodeIndex,
        instance: &instance::Generation,
        delivery: &envelope::Delivery,
        settings: &BTreeMap<String, crate::value::Value>,
    ) -> Result<(), ExecutionFault> {
        let planned = &self.plan.nodes[slot.index()];
        let preparation = bindings
            .prepare_session_open(&planned.path, delivery.envelope.data(), settings)
            .map_err(crate::boundary::PollFault::Driver)
            .map_err(ExecutionFault::Dispatch)?;
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
        let session_id = ids::outbound_session_id(
            &planned.path,
            delivery.envelope.meta().sys().id(),
            prepared.digest(),
        );
        let intent = SessionOpen {
            session_id,
            emitting_id: delivery.envelope.meta().sys().id(),
            connector_path: planned.path.clone(),
            prepared,
        };
        if let alloc::collections::btree_map::Entry::Vacant(entry) = self.sessions.entry(session_id)
        {
            journal
                .open_outbound_session(intent.clone())
                .map_err(ExecutionFault::Journal)?;
            let retained = self
                .scheduler
                .mailboxes
                .pop(slot, instance)
                .ok_or_else(|| ExecutionFault::PlanInvariant {
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
            .map_err(crate::boundary::PollFault::Driver)
            .map_err(ExecutionFault::Dispatch)?
        {
            SessionStart::Pending => Ok(()),
            SessionStart::Started => {
                let session = self.sessions.get_mut(&session_id).ok_or_else(|| {
                    ExecutionFault::PlanInvariant {
                        node: planned.path.clone(),
                        reason: "a started Session opening lost its logical record",
                    }
                })?;
                session.opening_started = true;
                Ok(())
            }
        }
    }

    fn write_session(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut dyn OutboundConnectorBindings,
        dispatch: SessionDispatch<'_>,
        session_id: SessionId,
        settings: &BTreeMap<String, crate::value::Value>,
    ) -> Result<(), ExecutionFault> {
        let SessionDispatch {
            slot,
            instance,
            delivery,
        } = dispatch;
        let planned = &self.plan.nodes[slot.index()];
        let preparation = bindings
            .prepare_session_write(
                &planned.path,
                session_id,
                delivery.envelope.data(),
                settings,
            )
            .map_err(crate::boundary::PollFault::Driver)
            .map_err(ExecutionFault::Dispatch)?;
        let prepared = match preparation {
            RequestPreparation::Prepared(prepared) => prepared,
            RequestPreparation::Rejected(completion) => {
                let terminal = self
                    .scheduler
                    .mailboxes
                    .pop(slot, instance)
                    .ok_or_else(|| ExecutionFault::PlanInvariant {
                        node: planned.path.clone(),
                        reason: "a rejected Session write lost its mailbox delivery",
                    })?;
                self.scheduler.mailboxes.claim(slot, instance);
                self.settled_sessions.push_back(SettledSession {
                    session: session_id,
                    completion,
                    terminal_delivery: Some(terminal),
                });
                return Ok(());
            }
        };
        let write_ref = ids::session_write_id(
            session_id,
            delivery.envelope.meta().sys().id(),
            prepared.digest(),
        );
        let intent = SessionWrite {
            write_ref,
            session_id,
            emitting_id: delivery.envelope.meta().sys().id(),
            connector_path: planned.path.clone(),
            prepared,
        };
        if let alloc::collections::btree_map::Entry::Vacant(entry) =
            self.session_writes.entry(write_ref)
        {
            journal
                .open_session_write(intent.clone())
                .map_err(ExecutionFault::Journal)?;
            let retained = self
                .scheduler
                .mailboxes
                .pop(slot, instance)
                .ok_or_else(|| ExecutionFault::PlanInvariant {
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
            .map_err(crate::boundary::PollFault::Driver)
            .map_err(ExecutionFault::Dispatch)?
        {
            SessionStart::Pending => Ok(()),
            SessionStart::Started => {
                let write = self.session_writes.get_mut(&write_ref).ok_or_else(|| {
                    ExecutionFault::PlanInvariant {
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
        slot: NodeIndex,
        instance: &instance::Generation,
        delivery: &envelope::Delivery,
        completion: &Completion,
    ) -> Result<(), ExecutionFault> {
        let executor = TransactionPreparer::at(self.plan, self.graph_version, self.now);
        let trigger = node::Trigger::Port(delivery.port.clone());
        let mut outcome = executor
            .connector_completion(slot, instance, delivery, &trigger, completion)
            .map_err(ExecutionFault::Fault)?;
        let prepared =
            self.validate_and_prepare_emissions(&&*journal, slot, instance, &outcome.record)?;
        self.retain_emission_routes(&mut outcome.record, &prepared)?;
        journal
            .commit(outcome.record)
            .map_err(ExecutionFault::Journal)?;
        self.scheduler.mailboxes.pop(slot, instance);
        self.route_emissions(slot, instance, prepared);
        self.progress_closures();
        Ok(())
    }

    fn discard_session_write(
        &mut self,
        journal: &mut dyn Journal,
        slot: NodeIndex,
        instance: &instance::Generation,
        delivery: &envelope::Delivery,
    ) -> Result<(), ExecutionFault> {
        self.discard_inbound_call_response(journal, slot, instance, delivery)
    }

    /// A delivery reaching a connector opens its outbound call:
    /// journal-before-effect — the open fact exists before any
    /// driver hears — and the request is held until the call
    /// settles, its mailbox claimed for the whole wait: the worker is
    /// free, the node is not.
    fn open_call(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut dyn OutboundConnectorBindings,
        slot: NodeIndex,
        instance: &instance::Generation,
        delivery: &envelope::Delivery,
    ) -> Result<(), ExecutionFault> {
        // A recovered mailbox can be ready after its retained budget is
        // already due. Settle that exact delivery before handing any intent
        // to the Driver, even when the caller drains before `advance`.
        let overdue = self.scheduler.dated.range(..=(self.now, u64::MAX)).find(|(_, payload)| {
            matches!(payload, Obligation::DeliveryBudget { edge, delivery: id, instance: target, .. }
                if edge == &delivery.edge && *id == delivery.envelope.meta().sys().id() && target == instance)
        }).map(|(key, payload)| (*key, payload.clone()));
        if let Some((key, payload)) = overdue {
            self.expire(journal, bindings, payload)?;
            self.scheduler.ack(key);
            return Ok(());
        }
        if let Some(intent) = self.pinned_retry_intent(instance, delivery) {
            return self.retain_call_open(journal, slot, instance, delivery, intent);
        }
        let planned = &self.plan.nodes[slot.index()];
        let NodeExecution::Connector(spec) = &planned.execution else {
            unreachable!("routed here by kind");
        };
        if !spec.roles.has_outbound() || spec.exchange != ConnectorExchange::Call {
            return Err(ExecutionFault::PlanInvariant {
                node: planned.path.clone(),
                reason: "a request requires an outbound Call Contract",
            });
        }
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
            .map_err(crate::boundary::PollFault::Driver)
            .map_err(ExecutionFault::Dispatch)?
        {
            RequestPreparation::Prepared(prepared) => prepared,
            RequestPreparation::Rejected(completion) => {
                let executor = TransactionPreparer::at(self.plan, self.graph_version, self.now);
                let trigger = node::Trigger::Port(delivery.port.clone());
                let mut outcome = executor
                    .connector_completion(slot, instance, delivery, &trigger, &completion)
                    .map_err(ExecutionFault::Fault)?;
                self.prepare_call_completion(slot, instance, delivery, &mut outcome.record)?;
                let prepared = self.validate_and_prepare_emissions(
                    &&*journal,
                    slot,
                    instance,
                    &outcome.record,
                )?;
                self.retain_emission_routes(&mut outcome.record, &prepared)?;
                journal
                    .commit(outcome.record.clone())
                    .map_err(ExecutionFault::Journal)?;
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
        let intent = CallOpen {
            call_id: ids::call_id(
                ids::delivery_id(&delivery.edge, emitting_id),
                prepared.digest(),
            ),
            emitting_id,
            edge: delivery.edge.clone(),
            connector_path: planned.path.clone(),
            prepared,
            idempotency_key: None,
            resolved_settings,
            logical_time: self.now,
        };
        if self.open_retry_operation(journal, slot, instance, delivery, &intent)? {
            return Ok(());
        }
        self.retain_call_open(journal, slot, instance, delivery, intent)
    }

    fn retain_call_open(
        &mut self,
        journal: &mut dyn Journal,
        slot: NodeIndex,
        instance: &instance::Generation,
        delivery: &envelope::Delivery,
        intent: CallOpen,
    ) -> Result<(), ExecutionFault> {
        let attempt = self.retry_attempt(instance, delivery);
        let mut retry = self.retry_manager.clone();
        if let (Some(attempt), Some(fold)) = (attempt, retry.as_deref_mut()) {
            fold.open_call(attempt, &intent)
                .map_err(retry_manager::error)?;
        }
        let call_id = intent.call_id;
        journal
            .open_egress(intent, attempt)
            .map_err(ExecutionFault::Journal)?;
        self.retry_manager = retry;
        let delivery = self
            .scheduler
            .mailboxes
            .pop(slot, instance)
            .unwrap_or_else(|| unreachable!("the reserved head is still there"));
        self.pending.insert(
            call_id,
            PendingCall {
                slot,
                delivery,
                instance: instance.clone(),
            },
        );
        self.scheduler.mailboxes.claim(slot, instance);
        Ok(())
    }

    fn slot_of(&self, path: &node::Path) -> NodeIndex {
        NodeIndex::new(
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
    /// ordinary Operator emissions, while [`CompletionCheck`] refuses an
    /// invalid driver draft before closure. THIS final, non-recursive level
    /// catches a violated plan/runtime invariant; it cannot manufacture
    /// another emission on the same bad wiring, so NOTHING commits.
    fn validate_and_prepare_emissions(
        &self,
        journal: &dyn InstanceAdmissions,
        slot: NodeIndex,
        source_instance: &instance::Generation,
        record: &crate::runtime::TransactionRecord,
    ) -> Result<PreparedEmissions, ExecutionFault> {
        let mut prepared = Vec::with_capacity(record.emissions.len());
        let mut commands = Vec::new();
        // One producer transaction is one admission linearization point. If
        // several emissions select the same not-yet-live instance, the first
        // canonical route binds its immutable bundle and every sibling route
        // reuses it; later payloads cannot rebind the same generation.
        let mut candidates = BTreeMap::new();
        for emission in &record.emissions {
            if let Data::Materialized(value) = emission.envelope.data() {
                value.check_consistency().map_err(|error| {
                    ExecutionFault::Fault(crate::runtime::Fault::InvalidValue(error))
                })?;
                let from = EdgeEnd {
                    node: Some(slot),
                    port: emission.port.clone(),
                };
                crate::runtime::crossing_pins_hold(self.plan, &from, value)
                    .map_err(|violation| ExecutionFault::Fault(violation.into_fault()))?;
            }
            let from = EdgeEnd {
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
        journal: &dyn InstanceAdmissions,
        from: &EdgeEnd,
        source_instance: Option<&instance::Generation>,
        envelope: &Envelope,
        candidates: &mut BTreeMap<instance::Generation, InstanceCandidate>,
    ) -> Result<Vec<PreparedEdge>, ExecutionFault> {
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
            .map_err(ExecutionFault::Fault)?;
        if let Some(value) = &selected_value {
            crate::runtime::crossing_pins_hold(self.plan, from, value)
                .map_err(|violation| ExecutionFault::Fault(violation.into_fault()))?;
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
                        edge.owner,
                        source_instance,
                        target,
                        envelope,
                    )? {
                        target
                    } else {
                        self.prepare_target(
                            journal,
                            edge.owner,
                            source_instance,
                            edge.to
                                .as_ref()
                                .ok_or_else(|| ExecutionFault::PlanInvariant {
                                    node: self.plan.nodes[target.index()].path.clone(),
                                    reason: "a node route has no destination port",
                                })?,
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
        edge_scope: node::OwnerSlot,
        source_instance: Option<&instance::Generation>,
        target: NodeIndex,
        envelope: &Envelope,
    ) -> Result<Option<PreparedTarget>, ExecutionFault> {
        let Some(spec) = self.plan.nodes[target.index()].connector() else {
            return Ok(None);
        };
        if !spec.roles.has_inbound()
            || spec.roles.has_outbound()
            || spec.exchange != ConnectorExchange::Call
        {
            return Ok(None);
        }

        let Some(call_id) = self.select_inbound_call_ref(target, spec, envelope)? else {
            return Ok(None);
        };
        let Some(pending) = self.inbound_calls.get(&call_id) else {
            return Ok(None);
        };
        if pending.slot != target
            || pending.response.is_some()
            || self
                .settled_inbound_calls
                .iter()
                .any(|event| event.call_id == call_id)
        {
            return Ok(None);
        }

        let owner_chain = self.owner_chain(self.plan.nodes[target.index()].owner)?;
        let Some(scope_index) = owner_chain.iter().position(|owner| *owner == edge_scope) else {
            return Err(ExecutionFault::PlanInvariant {
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
                    .ok_or_else(|| ExecutionFault::PlanInvariant {
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
            return Err(ExecutionFault::PlanInvariant {
                node: self.plan.nodes[target.index()].path.clone(),
                reason: "an open inbound Call has an instance chain inconsistent with its owner",
            });
        }
        let chain: Vec<instance::Generation> = selected
            .into_iter()
            .map(|(_, reference)| reference)
            .collect();
        let reference = chain
            .last()
            .cloned()
            .ok_or_else(|| ExecutionFault::PlanInvariant {
                node: self.plan.nodes[target.index()].path.clone(),
                reason: "an inbound Call target has no instance",
            })?;
        let scope =
            chain
                .get(scope_index)
                .cloned()
                .ok_or_else(|| ExecutionFault::PlanInvariant {
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
        slot: NodeIndex,
        spec: &crate::plan::ConnectorSpec,
        envelope: &Envelope,
    ) -> Result<Option<InboundCallId>, ExecutionFault> {
        let Some(setting) = spec.attribute("correlate") else {
            return Ok(Some(InboundCallId::from_root(
                envelope.meta().sys().origin_id(),
            )));
        };
        let AttrSetting::Expr {
            site: EvaluationSite::Invocation,
            expr,
        } = setting
        else {
            return Err(ExecutionFault::PlanInvariant {
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
        let data = DataSlot {
            ty: self.plan.resolved_ports().node(slot).and_then(|ports| {
                ports
                    .iter()
                    .find(|port| port.direction() == node::PortDirection::Input)
                    .map(|port| port.ty())
            }),
            ..DataSlot::new(envelope.data())
        };
        let selected = resolve_payload(
            eval_lazy(expr, &bindings, &data, &self.plan.assets).map_err(ExecutionFault::Fault)?,
            &data,
        )
        .map_err(ExecutionFault::Fault)?;
        let crate::value::Value::Text(selected) = selected else {
            return Err(ExecutionFault::PlanInvariant {
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

    /// Every route decision sealed by an INGRESS record. The envelope id is
    /// implicit because an INGRESS record carries exactly one envelope.
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
        journal: &dyn InstanceAdmissions,
        scope: node::OwnerSlot,
        source_instance: Option<&instance::Generation>,
        target: &EdgeEnd,
        envelope: &Envelope,
        candidates: &mut BTreeMap<instance::Generation, InstanceCandidate>,
    ) -> Result<PreparedTarget, ExecutionFault> {
        let slot = target.node.ok_or_else(|| ExecutionFault::PlanInvariant {
            node: node::Path::new(["admission"])
                .unwrap_or_else(|_| unreachable!("a literal path is valid")),
            reason: "an Executor admission has no node",
        })?;
        let target_owner = self.plan.nodes[slot.index()].owner;
        let data = DataSlot {
            ty: self.plan.resolved_ports().endpoint_type(target),
            ..DataSlot::new(envelope.data())
        };
        self.prepare_owner_target(
            journal,
            scope,
            source_instance,
            target_owner,
            envelope,
            &data,
            candidates,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_owner_target(
        &self,
        journal: &dyn InstanceAdmissions,
        scope: node::OwnerSlot,
        source_instance: Option<&instance::Generation>,
        target_owner: node::OwnerSlot,
        envelope: &Envelope,
        data: &DataSlot<'_>,
        candidates: &mut BTreeMap<instance::Generation, InstanceCandidate>,
    ) -> Result<PreparedTarget, ExecutionFault> {
        let refusal = |reason| match self.plan.owners[target_owner.index()].kind {
            OwnerKind::Node(slot) => ExecutionFault::PlanInvariant {
                node: self.plan.nodes[slot.index()].path.clone(),
                reason,
            },
            OwnerKind::Graph => ExecutionFault::InvalidLoopDecision {
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
                    data,
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

    fn owner_chain(&self, owner: node::OwnerSlot) -> Result<&[node::OwnerSlot], ExecutionFault> {
        self.owner_chains
            .get(owner.index())
            .and_then(Option::as_deref)
            .ok_or_else(|| ExecutionFault::PlanInvariant {
                node: node::Path::new(["owner"])
                    .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                reason: "an owner reference escaped the validated owner table",
            })
    }

    fn source_instance_for_owner(
        &self,
        source: Option<&instance::Generation>,
        owner: node::OwnerSlot,
    ) -> Result<Option<instance::Generation>, ExecutionFault> {
        let mut cursor = source.cloned();
        while let Some(reference) = cursor {
            let Some(instance) = self.instances.get(&reference) else {
                return Err(ExecutionFault::PlanInvariant {
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
        address: &instance::InstanceAddress,
        source: Option<&instance::Generation>,
        lifetime: &instance::Lifetime,
    ) -> Option<instance::Generation> {
        if self.closing.is_empty() || matches!(lifetime, instance::Lifetime::SinglePass) {
            return None;
        }
        let first = instance::Generation::new(address.clone(), 0);
        let last = instance::Generation::new(address.clone(), u64::MAX);
        let reference = self.closing.range(first..=last).next_back()?;
        let instance = self.instances.get(reference)?;
        let instance::Phase::Closing { scope, .. } = &instance.phase else {
            return None;
        };
        source
            .is_some_and(|source| self.is_within_instance(source, scope))
            .then(|| reference.clone())
    }

    #[allow(clippy::too_many_arguments)]
    fn candidate_owner(
        &self,
        journal: &dyn InstanceAdmissions,
        owner_slot: node::OwnerSlot,
        parent: Option<&instance::Generation>,
        source: Option<&instance::Generation>,
        envelope: &Envelope,
        data: &DataSlot<'_>,
        candidates: &mut BTreeMap<instance::Generation, InstanceCandidate>,
    ) -> Result<instance::Generation, ExecutionFault> {
        let owner = self
            .plan
            .owner(owner_slot)
            .ok_or_else(|| ExecutionFault::PlanInvariant {
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
            && !matches!(owner.instance.lifetime, instance::Lifetime::Tumbling(_))
        {
            if let Some((reference, instance)) = self.instances.iter().find(|(_, instance)| {
                instance.owner == owner_slot && instance.parent.as_ref() == parent
            }) {
                let accepts = match &instance.phase {
                    instance::Phase::Open => {
                        !matches!(owner.instance.lifetime, instance::Lifetime::SinglePass)
                    }
                    instance::Phase::Closing { scope, .. } => {
                        source.is_some_and(|source| self.is_within_instance(source, scope))
                            && !matches!(owner.instance.lifetime, instance::Lifetime::SinglePass)
                    }
                    instance::Phase::Stopping { .. } => false,
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
                if matches!(owner.instance.lifetime, instance::Lifetime::SinglePass)
                    && waiting.candidate.single_pass_root != Some(envelope.meta().sys().id())
                {
                    return Err(ExecutionFault::InstanceClosing {
                        owner: owner.path.clone(),
                    });
                }
                return Ok(waiting.candidate.reference.clone());
            }
            if let Some(candidate) = candidates.values().find(|candidate| {
                candidate.owner == owner_slot && candidate.parent.as_ref() == parent
            }) {
                if matches!(owner.instance.lifetime, instance::Lifetime::SinglePass)
                    && candidate.single_pass_root != Some(envelope.meta().sys().id())
                {
                    return Err(ExecutionFault::InstanceClosing {
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
                    (instance.values.clone(), instance.segments.clone())
                } else if let Some(waiting) = self.waiting.get(reference) {
                    (
                        waiting.candidate.values.clone(),
                        waiting.candidate.frames.clone(),
                    )
                } else if let Some(candidate) = candidates.get(reference) {
                    (candidate.values.clone(), candidate.frames.clone())
                } else {
                    return Err(ExecutionFault::PlanInvariant {
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
                let mut bindings = crate::expr::value_bindings(&inherited);
                bindings.insert(
                    String::from("meta"),
                    crate::expr::EvalValue::from(meta_value(envelope.meta())),
                );
                let value = resolve_payload(
                    eval_lazy(key, &bindings, data, &self.plan.assets)
                        .map_err(ExecutionFault::Fault)?,
                    data,
                )
                .map_err(ExecutionFault::Fault)?;
                frames.push(instance::Segment::SelectedKey {
                    scope: owner.path.clone(),
                    ty: key.ty.clone(),
                    value: value.clone(),
                });
                Some(value)
            }
        };
        let time_box_start = match owner.instance.lifetime {
            instance::Lifetime::Tumbling(every) => {
                let start = self
                    .now
                    .div_euclid(every)
                    .checked_mul(every)
                    .ok_or_else(|| ExecutionFault::PlanInvariant {
                        node: node::Path::new(["instance"])
                            .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                        reason: "a validated tumbling duration overflowed its epoch box",
                    })?;
                frames.push(instance::Segment::ProcessingTimeBox {
                    scope: owner.path.clone(),
                    start_ns: start,
                });
                Some(start)
            }
            _ => None,
        };
        let address = instance::InstanceAddress::new(&owner.path, &frames)
            .map_err(|error| ExecutionFault::Fault(Fault::Unrepresentable(error.to_string())))?;
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
            .instance_generation_for_admission(&address)
            .map_err(ExecutionFault::Journal)?;
        if let Some(start_ns) = time_box_start
            && reference.generation() != 0
        {
            return Err(ExecutionFault::ClosedTimeBox {
                owner: owner.path.clone(),
                start_ns,
            });
        }
        if let Some(instance) = self.instances.get(&reference) {
            let accepts = match &instance.phase {
                instance::Phase::Open => {
                    !matches!(owner.instance.lifetime, instance::Lifetime::SinglePass)
                }
                instance::Phase::Closing { scope, .. } => {
                    source.is_some_and(|source| self.is_within_instance(source, scope))
                        && !matches!(owner.instance.lifetime, instance::Lifetime::SinglePass)
                }
                instance::Phase::Stopping { .. } => false,
            };
            if accepts {
                return Ok(reference);
            }
            return Err(ExecutionFault::InstanceClosing {
                owner: owner.path.clone(),
            });
        }
        if let Some(waiting) = self.waiting.get(&reference) {
            if matches!(owner.instance.lifetime, instance::Lifetime::SinglePass)
                && waiting.candidate.single_pass_root != Some(envelope.meta().sys().id())
            {
                return Err(ExecutionFault::InstanceClosing {
                    owner: owner.path.clone(),
                });
            }
            return Ok(waiting.candidate.reference.clone());
        }
        if let Some(candidate) = candidates.get(&reference) {
            if matches!(owner.instance.lifetime, instance::Lifetime::SinglePass)
                && candidate.single_pass_root != Some(envelope.meta().sys().id())
            {
                return Err(ExecutionFault::InstanceClosing {
                    owner: owner.path.clone(),
                });
            }
            return Ok(candidate.reference.clone());
        }
        let mut values =
            self.evaluate_owner_values(owner_slot, &inherited, selected_key.as_ref(), data)?;
        if let Some(key) = &selected_key {
            values.insert(String::from("instance.key"), key.clone());
        }
        let settings =
            self.evaluate_instance_settings(owner_slot, &values, selected_key.as_ref(), data)?;
        let candidate = InstanceCandidate {
            owner: owner_slot,
            reference: reference.clone(),
            parent: parent.cloned(),
            frames,
            values,
            settings,
            single_pass_root: matches!(owner.instance.lifetime, instance::Lifetime::SinglePass)
                .then_some(envelope.meta().sys().id()),
        };
        candidates.insert(reference.clone(), candidate);
        Ok(reference)
    }

    fn evaluate_owner_values(
        &self,
        owner_slot: node::OwnerSlot,
        inherited: &BTreeMap<String, crate::value::Value>,
        selected_key: Option<&crate::value::Value>,
        data: &DataSlot<'_>,
    ) -> Result<BTreeMap<String, crate::value::Value>, ExecutionFault> {
        let owner = self
            .plan
            .owner(owner_slot)
            .ok_or_else(|| ExecutionFault::PlanInvariant {
                node: node::Path::new(["owner"])
                    .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                reason: "an owner reference escaped the validated owner table",
            })?;
        let mut local = BTreeMap::new();
        for (index, value) in owner.vals.iter().enumerate() {
            local.insert(value.name.as_str(), index);
        }
        let mut resolved: Vec<Option<crate::value::Value>> = vec![None; owner.vals.len()];
        let mut visiting = vec![false; owner.vals.len()];
        for index in 0..owner.vals.len() {
            self.evaluate_owner_value_at(
                owner_slot,
                index,
                inherited,
                selected_key,
                data,
                &local,
                &mut resolved,
                &mut visiting,
            )?;
        }
        let mut values = inherited.clone();
        for (declared, value) in owner.vals.iter().zip(resolved) {
            let value = value.ok_or_else(|| ExecutionFault::PlanInvariant {
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
        owner_slot: node::OwnerSlot,
        index: usize,
        inherited: &BTreeMap<String, crate::value::Value>,
        selected_key: Option<&crate::value::Value>,
        data: &DataSlot<'_>,
        local: &BTreeMap<&str, usize>,
        resolved: &mut [Option<crate::value::Value>],
        visiting: &mut [bool],
    ) -> Result<(), ExecutionFault> {
        if resolved[index].is_some() {
            return Ok(());
        }
        if visiting[index] {
            return Err(ExecutionFault::PlanInvariant {
                node: node::Path::new(["owner"])
                    .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                reason: "a validated immutable-binding cycle reached evaluation",
            });
        }
        visiting[index] = true;
        let owner = self
            .plan
            .owner(owner_slot)
            .ok_or_else(|| ExecutionFault::PlanInvariant {
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
                    data,
                    local,
                    resolved,
                    visiting,
                )?;
            }
        }
        let value = match (&declared.site, &declared.resolved) {
            (EvaluationSite::Plan, Some(value)) => value.clone(),
            (EvaluationSite::Plan, None) | (EvaluationSite::Invocation, _) => {
                return Err(ExecutionFault::PlanInvariant {
                    node: node::Path::new(["owner"])
                        .unwrap_or_else(|_| unreachable!("a literal path is valid")),
                    reason: "a validated immutable binding carries an impossible site",
                });
            }
            (EvaluationSite::Boot | EvaluationSite::Instance, _) => {
                let mut bindings = crate::expr::value_bindings(inherited);
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
                resolve_payload(
                    eval_lazy(&declared.expr, &bindings, data, &self.plan.assets)
                        .map_err(ExecutionFault::Fault)?,
                    data,
                )
                .map_err(ExecutionFault::Fault)?
            }
        };
        if let Err(violation) = declared.expr.ty.check_value_type(&declared.name, &value) {
            return Err(ExecutionFault::Fault(Fault::Unrepresentable(
                violation.to_string(),
            )));
        }
        visiting[index] = false;
        resolved[index] = Some(value);
        Ok(())
    }

    fn evaluate_instance_settings(
        &self,
        owner_slot: node::OwnerSlot,
        values: &BTreeMap<String, crate::value::Value>,
        selected_key: Option<&crate::value::Value>,
        data: &DataSlot<'_>,
    ) -> Result<BTreeMap<String, crate::value::Value>, ExecutionFault> {
        let Some(owner) = self.plan.owner(owner_slot) else {
            return Err(ExecutionFault::PlanInvariant {
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
        let mut bindings = crate::expr::value_bindings(values);
        if let Some(key) = selected_key {
            bindings.insert(
                String::from("instance.key"),
                crate::expr::EvalValue::from(key.clone()),
            );
        }
        self.evaluate_connector_setting_expressions(spec, EvaluationSite::Instance, &bindings, data)
    }

    fn resolve_invocation_settings(
        &self,
        instance: &instance::Generation,
        spec: &crate::plan::ConnectorSpec,
        node: &node::Path,
        envelope: &Envelope,
    ) -> Result<BTreeMap<String, crate::value::Value>, ExecutionFault> {
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
            .ok_or_else(|| ExecutionFault::PlanInvariant {
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
        let mut bindings = crate::expr::value_bindings(values);
        bindings.insert(
            String::from("meta"),
            crate::expr::EvalValue::from(meta_value(envelope.meta())),
        );
        let ty = self
            .plan
            .nodes
            .iter()
            .position(|entry| entry.path == *node)
            .and_then(|index| {
                self.plan
                    .resolved_ports()
                    .node(NodeIndex::new(index as u32))
            })
            .and_then(|ports| {
                ports
                    .iter()
                    .find(|port| port.direction() == node::PortDirection::Input)
            })
            .map(|port| port.ty());
        let data = DataSlot {
            ty,
            ..DataSlot::new(envelope.data())
        };
        settings.extend(self.evaluate_connector_setting_expressions(
            spec,
            EvaluationSite::Invocation,
            &bindings,
            &data,
        )?);
        Ok(settings)
    }

    fn evaluate_connector_setting_expressions(
        &self,
        spec: &crate::plan::ConnectorSpec,
        target_site: EvaluationSite,
        bindings: &BTreeMap<String, crate::expr::EvalValue>,
        data: &DataSlot<'_>,
    ) -> Result<BTreeMap<String, crate::value::Value>, ExecutionFault> {
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
                eval_lazy(expr, bindings, data, &self.plan.assets)
                    .map_err(ExecutionFault::Fault)?,
                data,
            )
            .map_err(ExecutionFault::Fault)?;
            if let Err(violation) = expr.ty.check_value_type(name, &value) {
                return Err(ExecutionFault::Fault(Fault::Unrepresentable(
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
    ) -> Result<(), ExecutionFault>
    where
        PreparedEdge: 'a,
    {
        let mut needed: BTreeMap<(NodeIndex, instance::Generation), usize> = BTreeMap::new();
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
                if self.has_loops || self.retry_manager.is_some() {
                    let owner = self.plan.nodes[target.index()].owner;
                    if self.plan.owners[owner.index()]
                        .loop_spec()
                        .is_some_and(|spec| spec.entry == to.port)
                        || self.plan.owners[owner.index()]
                            .retry_spec()
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
                    .ok_or_else(|| ExecutionFault::PlanInvariant {
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
                return Err(ExecutionFault::MailboxFull {
                    node: self.plan.nodes[target.index()].path.clone(),
                });
            }
        }
        if let Some(owner) = loop_owner
            && self
                .recurrence_admission_load()
                .saturating_add(loop_admissions)
                > WORKER_MAILBOX_CAPACITY
        {
            return Err(ExecutionFault::RecurrenceQueueFull {
                owner: self.plan.owners[owner.index()].path.clone(),
            });
        }
        Ok(())
    }

    /// Routes a committed record's emissions — infallible: capacity
    /// and every target key were reserved before the commit.
    fn route_emissions(
        &mut self,
        slot: NodeIndex,
        source_instance: &instance::Generation,
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
            self.observed.push(EnvelopeView {
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
            self.update_graph_loop_contexts(contexts);
        }
    }

    /// Applies a preselected fan-out; returns how many edges claimed the
    /// envelope.
    fn route_prepared(
        &mut self,
        routes: Vec<PreparedEdge>,
        candidates: &BTreeMap<instance::Generation, InstanceCandidate>,
        envelope: &Envelope,
        source_instance: Option<&instance::Generation>,
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
                        self.apply_instance_admissions(prepared, candidates, envelope);
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
                        for (deadline, payload) in
                            self.delivery_deadlines(&route, prepared, candidates, envelope)
                        {
                            self.scheduler.schedule(deadline.due_at, payload);
                        }
                        self.observed.push(EnvelopeView {
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

    /// Apply a committed target's instance chain, including lifecycle work
    /// and deadlines. Instance selection and capacity checks already ran.
    fn apply_instance_admissions(
        &mut self,
        target: &PreparedTarget,
        candidates: &BTreeMap<instance::Generation, InstanceCandidate>,
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
                                instance::Phase::Open => None,
                                instance::Phase::Closing {
                                    cause,
                                    scope,
                                    stage,
                                } => Some((cause.clone(), scope.clone(), *stage)),
                                instance::Phase::Stopping { cause, scope } => Some((
                                    cause.clone(),
                                    scope.clone(),
                                    instance::ClosingStage::Settling,
                                )),
                            })
                    });
                    let phase =
                        inherited_close.map_or(instance::Phase::Open, |(cause, scope, stage)| {
                            instance::Phase::Closing {
                                cause,
                                scope,
                                stage,
                            }
                        });
                    let is_closing = matches!(phase, instance::Phase::Closing { .. });
                    self.instances.insert(
                        reference.clone(),
                        instance::LiveInstance {
                            owner: candidate.owner,
                            parent: candidate.parent.clone(),
                            segments: candidate.frames.clone(),
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
        for (deadline, payload) in deadlines {
            self.scheduler.schedule(deadline.due_at, payload);
        }
    }

    /// The canonical edge rendering — the same text the consumed
    /// cursor seals.
    fn edge_text(&self, edge: &Edge) -> String {
        self.plan.edge_text(edge)
    }
}
