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
use crate::envelope::{Data, Envelope, Id, Meta, MetaSys};
use crate::journal::{Journal, RouteTarget, RoutedDelivery, ids};
use crate::node;
use crate::plan::{Edge, EndpointRef, GraphVersion, NodeKind, Slot, ValidatedExecutionPlan};
use crate::runtime::{
    DataSlot, Delivery, Fault, Mailboxes, PartitionKey, PartitionState, TransactionExecutor,
    eval_lazy, meta_value, resolve_payload,
};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::{String, ToString};
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
    partition_key: PartitionKey,
}

/// One live incarnation of `(node, partition_key)`. The generation changes
/// on every admitted envelope, so dated idle events never need deletion.
#[derive(Debug, Clone)]
struct Instance {
    generation: u64,
    last_activity: Envelope,
    expiry_pending: bool,
}

/// Lifecycle work owns its causal envelope. Stop work additionally owns the
/// detached state snapshot of the closing incarnation, allowing a following
/// incarnation of the same key to start without sharing cells.
#[derive(Debug, Clone)]
struct LifecycleWork {
    slot: Slot,
    partition_key: PartitionKey,
    lifecycle: node::Lifecycle,
    cause: Envelope,
    detached_state: Option<BTreeMap<String, crate::value::Value>>,
}

/// One edge whose target instance was selected before the causing record
/// committed. Reusing this value after the commit makes routing infallible and
/// prevents a replay-sensitive second evaluation of `per`.
#[derive(Debug, Clone)]
struct PreparedEdge {
    edge: Edge,
    target_key: PartitionKey,
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
struct PreparedEmissions(Vec<PreparedEmission>);

/// The plan-backed gate for one atomic boundary completion.
///
/// Format v8 retains the exact `ProcessExit` union in the resolved port
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
    graph_version: GraphVersion,
    scheduler: Scheduler,
    state: PartitionState,
    dispatcher: Dispatcher,
    /// Requests delivered to a connector, held until their call
    /// settles: the completion transaction consumes this cursor, and the
    /// connector's mailbox stays claimed the whole time.
    pending: BTreeMap<CallRef, PendingCall>,
    /// Live keyed/singleton incarnations whose lifecycle is observable.
    instances: BTreeMap<(Slot, PartitionKey), Instance>,
    /// Journal-before-next-work lifecycle transitions. This queue outranks
    /// ordinary deliveries so `l:start` precedes the creating arrival and a
    /// detached `l:stop` precedes the following incarnation.
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
        Self {
            plan,
            graph_version,
            scheduler: Scheduler::new(1024),
            state: PartitionState::new(),
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
        // `sys` is minted by the journal commit. Partition expressions are
        // checked never to depend on it, so this provisional envelope is
        // exact for the admitted routing vocabulary (`data`, `app`, `ext`).
        // The selected keys are retained across the append — no semantic
        // expression is evaluated after a causal record exists.
        let provisional = Envelope::new(
            Meta::new(MetaSys::root(Id::from_bytes([0; 16]), self.graph_version.0))
                .with_ext(submission.draft.ext.clone()),
            submission.draft.data.clone(),
        );
        let prepared = self.prepare_routes(&from, &provisional)?;
        self.check_room_for_routes(core::iter::once(prepared.as_slice()))?;
        let recorded_routes = self.inbound_route_targets(&prepared);
        let committed = journal
            .append_inbound_routed(submission, recorded_routes)
            .map_err(VmError::Journal)?;
        let id = committed.envelope.meta().sys().id();
        self.route_prepared(prepared, &committed.envelope, &PartitionKey::singleton());
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
        let (connector, edge, delivery) = match payload {
            DatedPayload::DeliveryBudget {
                connector,
                edge,
                delivery,
            } => (connector, edge, delivery),
            DatedPayload::PartitionIdle {
                node,
                partition_key,
                generation,
                cause,
            } => {
                self.expire_partition(node, partition_key, generation, cause);
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
                held.delivery.edge == edge && held.delivery.envelope.meta().sys().id() == delivery
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
                .find_by_ref_any(slot, &edge, delivery)
                .map(|(_, delivery)| delivery)
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
                    let Some((partition_key, queued)) = self
                        .scheduler
                        .mailboxes
                        .find_by_ref_any(slot, &edge, delivery)
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
                        .connector_completion(slot, &partition_key, queued, &trigger, &completion)
                        .map_err(VmError::Fault)?
                };
                let prepared = self.validate_and_prepare_emissions(slot, &outcome.record)?;
                outcome.record.routes = self.routed_deliveries(&prepared);
                journal
                    .commit(outcome.record.clone())
                    .map_err(VmError::Journal)?;
                let Some((partition_key, _)) = self
                    .scheduler
                    .mailboxes
                    .remove_by_ref_any(slot, &edge, delivery)
                else {
                    return Err(VmError::PlanInvariant {
                        node: self.plan.nodes[slot.index()].path.clone(),
                        reason: "a reserved timeout delivery disappeared after commit",
                    });
                };
                self.route_emissions(slot, &partition_key, prepared);
                self.maybe_begin_pending_stop(slot, &partition_key);
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
                let Some((partition_key, queued)) = self
                    .scheduler
                    .mailboxes
                    .remove_by_ref_any(slot, &edge, delivery)
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
                        partition_key,
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

    /// Resolves one idle deadline. A superseded generation is a harmless
    /// clock observation. An open call postpones closure without losing the
    /// request's terminal obligation; settlement queues the stop afterwards.
    fn expire_partition(
        &mut self,
        path: node::Path,
        partition_key: Vec<u8>,
        generation: u64,
        cause: Id,
    ) {
        let slot = self.slot_of(&path);
        let key = PartitionKey::from_recorded(partition_key);
        let current = self
            .instances
            .get(&(slot, key.clone()))
            .filter(|instance| {
                instance.generation == generation
                    && instance.last_activity.meta().sys().id() == cause
            })
            .map(|instance| instance.last_activity.clone());
        let Some(cause) = current else {
            return;
        };
        if self.instance_has_work(slot, &key) {
            if let Some(instance) = self.instances.get_mut(&(slot, key.clone())) {
                instance.expiry_pending = true;
            }
            return;
        }
        self.begin_stop(slot, &key, cause);
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
        self.plan.nodes[slot.index()].partition.is_some()
            || self.plan.handlers.iter().any(|handler| {
                handler.node == slot
                    && matches!(
                        handler.activation,
                        node::HandlerActivation::Lifecycle(
                            node::Lifecycle::Start | node::Lifecycle::Stop
                        )
                    )
            })
    }

    /// Records one admitted envelope as instance activity. This runs only
    /// after the producer fact committed; every choice it makes was already
    /// sealed in that producer's route table.
    fn admit_instance(&mut self, slot: Slot, key: &PartitionKey, envelope: &Envelope) {
        if !self.lifecycle_managed(slot) {
            return;
        }
        let instance_key = (slot, key.clone());
        let generation = match self.instances.get_mut(&instance_key) {
            Some(instance) => {
                instance.generation = instance.generation.wrapping_add(1);
                instance.last_activity = envelope.clone();
                instance.expiry_pending = false;
                instance.generation
            }
            None => {
                self.instances.insert(
                    instance_key,
                    Instance {
                        generation: 0,
                        last_activity: envelope.clone(),
                        expiry_pending: false,
                    },
                );
                self.lifecycle.push_back(LifecycleWork {
                    slot,
                    partition_key: key.clone(),
                    lifecycle: node::Lifecycle::Start,
                    cause: envelope.clone(),
                    detached_state: None,
                });
                0
            }
        };
        if let Some(crate::plan::PartitionLifetime::IdleTtl(ttl)) = self.plan.nodes[slot.index()]
            .partition
            .as_ref()
            .map(|partition| &partition.lifetime)
        {
            self.scheduler.arm(
                self.now.saturating_add(*ttl),
                DatedPayload::PartitionIdle {
                    node: self.plan.nodes[slot.index()].path.clone(),
                    partition_key: key.as_bytes().to_vec(),
                    generation,
                    cause: envelope.meta().sys().id(),
                },
            );
        }
    }

    fn instance_has_work(&self, slot: Slot, key: &PartitionKey) -> bool {
        self.scheduler.mailboxes.has_work(slot, key)
            || self
                .pending
                .values()
                .any(|pending| pending.slot == slot && pending.partition_key == *key)
    }

    /// Detaches a closing incarnation before its stop transaction. New work
    /// for the same logical key can then create a fresh state cell, while the
    /// queued stop owns the old snapshot and remains retryable on refusal.
    fn begin_stop(&mut self, slot: Slot, key: &PartitionKey, cause: Envelope) {
        if self.instances.remove(&(slot, key.clone())).is_none() {
            return;
        }
        let detached_state = self.state.take_unchecked(self.plan.plan(), slot, key);
        self.lifecycle.push_back(LifecycleWork {
            slot,
            partition_key: key.clone(),
            lifecycle: node::Lifecycle::Stop,
            cause,
            detached_state: Some(detached_state),
        });
    }

    fn maybe_begin_pending_stop(&mut self, slot: Slot, key: &PartitionKey) {
        let cause = self
            .instances
            .get(&(slot, key.clone()))
            .filter(|instance| instance.expiry_pending)
            .map(|instance| instance.last_activity.clone());
        if let Some(cause) = cause
            && !self.instance_has_work(slot, key)
        {
            self.begin_stop(slot, key, cause);
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
                .cell_unchecked(self.plan.plan(), work.slot, &work.partition_key)
        });
        let executor = TransactionExecutor::new(self.plan, self.graph_version);
        let mut outcome = executor
            .fire_lifecycle(
                work.slot,
                &work.partition_key,
                work.lifecycle,
                &work.cause,
                &state,
            )
            .map_err(VmError::Fault)?;
        let prepared = self.validate_and_prepare_emissions(work.slot, &outcome.record)?;
        outcome.record.routes = self.routed_deliveries(&prepared);
        journal
            .commit(outcome.record.clone())
            .map_err(VmError::Journal)?;
        self.lifecycle.pop_front();
        if work.lifecycle == node::Lifecycle::Start {
            self.state.apply_unchecked(
                self.plan.plan(),
                work.slot,
                &work.partition_key,
                &outcome.record.state_delta,
            );
        }
        self.route_emissions(work.slot, &work.partition_key, prepared);
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
                &request.partition_key,
                &request.delivery,
                &trigger,
                &event.completion,
            )
            .map_err(VmError::Fault)?;
        let prepared = self.validate_and_prepare_emissions(slot, &outcome.record)?;
        outcome.record.routes = self.routed_deliveries(&prepared);
        journal
            .commit(outcome.record.clone())
            .map_err(VmError::Journal)?;
        self.settled.pop_front();
        self.pending.remove(&event.call_ref);
        let instance_still_open = self
            .pending
            .values()
            .any(|pending| pending.slot == slot && pending.partition_key == request.partition_key);
        if !instance_still_open {
            self.scheduler
                .mailboxes
                .release(slot, &request.partition_key);
        }
        self.route_emissions(slot, &request.partition_key, prepared);
        self.maybe_begin_pending_stop(slot, &request.partition_key);
        Ok(())
    }

    fn fire_operator(
        &mut self,
        journal: &mut dyn Journal,
        slot: Slot,
        key: &PartitionKey,
        delivery: &Delivery,
    ) -> Result<(), VmError> {
        let executor = TransactionExecutor::new(self.plan, self.graph_version);
        let state = self.state.cell_unchecked(self.plan.plan(), slot, key);
        // An Operator's declared `err` is an ordinary domain-result port,
        // close to the E in `I -> Result<R, E>`. Engine/evaluation faults are
        // outside that contract: the transaction stays uncommitted and the
        // VM reports the fault to supervision.
        let mut outcome = executor
            .fire(slot, key, delivery, &state)
            .map_err(VmError::Fault)?;
        let prepared = self.validate_and_prepare_emissions(slot, &outcome.record)?;
        outcome.record.routes = self.routed_deliveries(&prepared);
        journal
            .commit(outcome.record.clone())
            .map_err(VmError::Journal)?;
        self.scheduler.mailboxes.pop(slot, key);
        self.state
            .apply_unchecked(self.plan.plan(), slot, key, &outcome.record.state_delta);
        self.route_emissions(slot, key, prepared);
        self.maybe_begin_pending_stop(slot, key);
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
        key: &PartitionKey,
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
                    .connector_completion(slot, key, delivery, &trigger, &completion)
                    .map_err(VmError::Fault)?;
                let prepared = self.validate_and_prepare_emissions(slot, &outcome.record)?;
                outcome.record.routes = self.routed_deliveries(&prepared);
                journal
                    .commit(outcome.record.clone())
                    .map_err(VmError::Journal)?;
                self.scheduler.mailboxes.pop(slot, key);
                self.route_emissions(slot, key, prepared);
                self.maybe_begin_pending_stop(slot, key);
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
        };
        let call_ref = intent.call_ref;
        journal.open_outbound(intent).map_err(VmError::Journal)?;
        let delivery = self
            .scheduler
            .mailboxes
            .pop(slot, key)
            .unwrap_or_else(|| unreachable!("the reserved head is still there"));
        self.pending.insert(
            call_ref,
            PendingCall {
                slot,
                delivery,
                partition_key: key.clone(),
            },
        );
        self.scheduler.mailboxes.claim(slot, key);
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
        slot: Slot,
        record: &crate::runtime::TransactionRecord,
    ) -> Result<PreparedEmissions, VmError> {
        let mut prepared = Vec::with_capacity(record.emissions.len());
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
                routes: self.prepare_routes(&from, &emission.envelope)?,
            });
        }
        self.check_room_for_routes(prepared.iter().map(|emission| emission.routes.as_slice()))?;
        Ok(PreparedEmissions(prepared))
    }

    /// Selects target keys for every edge leaving one endpoint. The target
    /// expression sees the envelope as it will be delivered, not the source
    /// node's state or scheduler context.
    fn prepare_routes(
        &self,
        from: &EndpointRef,
        envelope: &Envelope,
    ) -> Result<Vec<PreparedEdge>, VmError> {
        self.plan
            .edges
            .iter()
            .filter(|edge| edge.from == *from)
            .cloned()
            .map(|edge| {
                let target_key = match edge.to.as_ref().and_then(|to| to.node) {
                    Some(target) => self.partition_key(target, envelope)?,
                    None => PartitionKey::singleton(),
                };
                Ok(PreparedEdge { edge, target_key })
            })
            .collect()
    }

    /// The non-singleton route decisions sealed by an inbound record. The
    /// envelope id is implicit because an inbound record carries exactly one
    /// envelope.
    fn inbound_route_targets(&self, prepared: &[PreparedEdge]) -> Vec<RouteTarget> {
        prepared
            .iter()
            .filter(|route| !route.target_key.as_bytes().is_empty())
            .map(|route| RouteTarget {
                edge: self.edge_text(&route.edge),
                target_partition: route.target_key.as_bytes().to_vec(),
            })
            .collect()
    }

    /// The non-singleton route decisions sealed atomically with an emitting
    /// transaction. Repeated emissions on one edge remain distinct by id.
    fn routed_deliveries(&self, prepared: &PreparedEmissions) -> Vec<RoutedDelivery> {
        prepared
            .0
            .iter()
            .flat_map(|emission| {
                emission
                    .routes
                    .iter()
                    .filter(|route| !route.target_key.as_bytes().is_empty())
                    .map(|route| RoutedDelivery {
                        delivery: emission.emission.envelope.meta().sys().id(),
                        edge: self.edge_text(&route.edge),
                        target_partition: route.target_key.as_bytes().to_vec(),
                    })
            })
            .collect()
    }

    fn partition_key(&self, target: Slot, envelope: &Envelope) -> Result<PartitionKey, VmError> {
        let Some(partition) = &self.plan.nodes[target.index()].partition else {
            return Ok(PartitionKey::singleton());
        };
        let mut bindings = BTreeMap::new();
        bindings.insert(
            String::from("meta"),
            crate::expr::EvalValue::from(meta_value(envelope.meta())),
        );
        let data = DataSlot::new(envelope.data());
        let value = resolve_payload(
            eval_lazy(&partition.key, &bindings, &data, &self.plan.assets)
                .map_err(VmError::Fault)?,
            &data,
        )
        .map_err(VmError::Fault)?;
        PartitionKey::from_value(&partition.key.ty, &value).map_err(VmError::Fault)
    }

    fn check_room_for_routes<'a>(
        &self,
        routes: impl IntoIterator<Item = &'a [PreparedEdge]>,
    ) -> Result<(), VmError>
    where
        PreparedEdge: 'a,
    {
        let mut needed: BTreeMap<(Slot, PartitionKey), usize> = BTreeMap::new();
        for routes in routes {
            for prepared in routes {
                let Some(to) = &prepared.edge.to else {
                    continue;
                };
                let Some(target) = to.node else {
                    continue;
                };
                *needed
                    .entry((target, prepared.target_key.clone()))
                    .or_default() += 1;
            }
        }
        for ((target, key), count) in &needed {
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
        source_partition: &PartitionKey,
        prepared: PreparedEmissions,
    ) {
        for PreparedEmission { emission, routes } in prepared.0 {
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
            let claimed = self.route_prepared(routes, &emission.envelope, source_partition);
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
        envelope: &Envelope,
        source_partition: &PartitionKey,
    ) -> usize {
        let claimed = routes.len();
        for PreparedEdge { edge, target_key } in routes {
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
                        let edge_text = self.edge_text(&edge);
                        self.scheduler
                            .mailboxes
                            .deliver(
                                target,
                                &target_key,
                                source_partition,
                                to.port.clone(),
                                edge_text,
                                envelope.clone(),
                            )
                            .unwrap_or_else(|_| {
                                unreachable!("capacity was reserved before the commit")
                            });
                        self.admit_instance(target, &target_key, envelope);
                        self.observed.push(Observed {
                            node: Some(self.plan.nodes[target.index()].path.clone()),
                            port: to.port.clone(),
                            envelope: envelope.clone(),
                        });
                        // The budget runs from THIS moment — queue
                        // time counts (execution-model §8). What is
                        // armed here is exactly what the committed
                        // record sealed as its timers.
                        if let Some(crate::plan::AttrSetting::Value(
                            crate::value::Value::Duration(budget),
                        )) = self.plan.nodes[target.index()]
                            .connector()
                            .and_then(|spec| spec.attribute("timeout"))
                        {
                            self.scheduler.arm(
                                self.now.saturating_add(*budget),
                                DatedPayload::DeliveryBudget {
                                    connector: self.plan.nodes[target.index()].path.clone(),
                                    edge: self.plan.edge_text(&edge),
                                    delivery: envelope.meta().sys().id(),
                                },
                            );
                        }
                    }
                },
            }
        }
        claimed
    }

    /// The canonical edge rendering — the same text the consumed
    /// cursor seals.
    fn edge_text(&self, edge: &Edge) -> String {
        self.plan.edge_text(edge)
    }
}
