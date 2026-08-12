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
    BoundaryEvent, CallOutcome, CallRef, DatedPayload, Dispatcher, InboundSubmission, Intent,
    Partner, Response,
};
use crate::envelope::{Data, Envelope, Id};
use crate::journal::{ConsumedPosition, Journal, ids};
use crate::node;
use crate::plan::{Edge, EndpointRef, GraphVersion, NodeKind, Slot, ValidatedExecutionPlan};
use crate::runtime::{Delivery, Fault, Key, Mailboxes, PartitionState, TransactionExecutor};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// A machine fault; the step that raised it committed nothing, and
/// the work that triggered it stays reserved — a retry sees it again.
/// A machine-integrity refusal — never a business error.
#[derive(Debug, Clone, PartialEq)]
pub enum VmError {
    /// A transaction fault the machine cannot convert into a typed
    /// error — a plan/configuration integrity breach.
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
    /// An injection at a port that is not a graph input — neither the
    /// implicit `in` nor a declared boundary input. Checked sources
    /// never build one; the raw API is verified, not trusted.
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
        violation: crate::runtime::ConformanceViolation,
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
    /// Closures that settled into pending outcome transactions.
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

/// One held request: the delivery whose cursor the outcome
/// transaction will consume. A budget obligation resolves against
/// its reference — the canonical edge and the envelope id, both on
/// the delivery itself.
#[derive(Debug, Clone, PartialEq)]
struct PendingCall {
    delivery: Delivery,
}

/// The plan-backed outcome gate: a failure kind must belong to the
/// calling connector's sealed error set. Timeout-minted failures
/// never pass here — the machine, not a partner, makes those.
struct PlanGate<'plan> {
    plan: &'plan crate::plan::ExecutionPlan,
}

impl crate::boundary::OutcomeGate for PlanGate<'_> {
    fn admit(
        &self,
        connector: &node::Path,
        outcome: &crate::boundary::CallOutcome,
    ) -> Result<(), String> {
        let CallOutcome::Failure(error) = outcome else {
            return Ok(());
        };
        let known = self
            .plan
            .nodes
            .iter()
            .find(|node| &node.path == connector)
            .and_then(crate::plan::Node::connector)
            .is_some_and(|spec| spec.error_set.iter().any(|kind| kind == &error.kind));
        if known {
            Ok(())
        } else {
            Err(format!(
                "`{}` is not in the error set of `{connector}`",
                error.kind
            ))
        }
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

/// One won closure and the trigger its outcome transaction fires
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
    /// settles: the outcome transaction consumes this cursor, and the
    /// connector's mailbox stays claimed the whole time.
    pending: BTreeMap<CallRef, PendingCall>,
    /// Won closures awaiting their outcome transaction — replayable:
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
        // The submission's port is verified against the graph, not
        // trusted: the implicit `in` or a DECLARED boundary input —
        // the same union the plan validator holds edges to.
        let ingress_ok = submission.port.name() == "in"
            || self.plan.boundary.iter().any(|decl| {
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
                Some(crate::cel::Type::Text)
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
            crate::runtime::value_well_formed(value).map_err(VmError::Fault)?;
            let own_pin = self
                .plan
                .boundary
                .iter()
                .find(|decl| decl.port == submission.port)
                .and_then(|decl| decl.ty.as_ref());
            crate::runtime::crossing_pins_hold(
                self.plan.plan(),
                &from,
                own_pin,
                &crate::runtime::from_envelope_value(value),
            )
            .map_err(|refused| VmError::IngressPinViolation {
                ingress: submission.port.clone(),
                rejected_at: refused.at,
                violation: refused.violation,
            })?;
        }
        self.reserve_targets(&from)?;
        let committed = journal
            .append_inbound(submission)
            .map_err(VmError::Journal)?;
        let id = committed.envelope.meta().sys().id();
        self.route(&from, &committed.envelope);
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
        let gate = PlanGate {
            plan: self.plan.plan(),
        };
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
        let DatedPayload::DeliveryBudget {
            connector,
            edge,
            delivery,
        } = payload;
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
                    port: node::Port::new("err")
                        .unwrap_or_else(|_| unreachable!("`err` is a valid port")),
                    outcome: CallOutcome::Failure(crate::boundary::TypedError {
                        kind: String::from("TimeoutExpired"),
                        data: Data::materialized(crate::envelope::Value::Map(BTreeMap::new())),
                    }),
                },
                trigger: node::Trigger::Lifecycle(node::Lifecycle::Timeout),
            });
            return Ok(());
        }
        // Still queued: the queue time consumed the whole budget.
        // The call-closed rule applies even to a call that expires
        // before it started (execution-model §8): the call OPENS and
        // CLOSES with a Timeout fact, and the outcome transaction
        // fires under `l:timeout` — the same causality as any expiry.
        // Journal facts come first; the mailbox is touched only after
        // they hold, so a refusal loses nothing.
        let slot = self.slot_of(&connector);
        let queued_id = {
            let Some(queued) = self
                .scheduler
                .mailboxes
                .find_by_ref(slot, "", &edge, delivery)
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
                let outcome = {
                    let Some(queued) = self
                        .scheduler
                        .mailboxes
                        .find_by_ref(slot, "", &edge, delivery)
                    else {
                        return Ok(());
                    };
                    let executor = TransactionExecutor::new(self.plan, self.graph_version);
                    executor.typed_error_transaction(
                        slot,
                        queued,
                        "DecodeError",
                        "the request cannot be framed",
                    )
                };
                self.validate_and_reserve_emissions(slot, &outcome.record)?;
                journal
                    .commit(outcome.record.clone())
                    .map_err(VmError::Journal)?;
                self.scheduler
                    .mailboxes
                    .remove_by_ref(slot, "", &edge, delivery);
                self.route_emissions(slot, &outcome.record);
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
                let queued = self
                    .scheduler
                    .mailboxes
                    .remove_by_ref(slot, "", &edge, delivery)
                    .unwrap_or_else(|| unreachable!("found above; nothing ran since"));
                self.pending
                    .insert(call_ref, PendingCall { delivery: queued });
                self.settled.push_back(SettledEntry {
                    event: BoundaryEvent {
                        call_ref,
                        connector_path,
                        emitting_id,
                        port: node::Port::new("err")
                            .unwrap_or_else(|_| unreachable!("`err` is a valid port")),
                        outcome: CallOutcome::Failure(crate::boundary::TypedError {
                            kind: String::from("TimeoutExpired"),
                            data: Data::materialized(crate::envelope::Value::Map(BTreeMap::new())),
                        }),
                    },
                    trigger: node::Trigger::Lifecycle(node::Lifecycle::Timeout),
                });
            }
        }
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

    /// The root error sink: `err` emissions no edge claimed. The spec
    /// bubbles unwired errors to the boundary — they surface here
    /// instead of vanishing.
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
        self.scheduler.mailboxes.is_empty() && self.settled.is_empty() && self.pending.is_empty()
    }

    /// Whether calls are open against the world — work [`Self::drain`]
    /// cannot finish alone.
    #[must_use]
    pub fn has_open_obligations(&self) -> bool {
        !self.pending.is_empty()
    }

    /// One won closure becomes the connector's outcome transaction.
    /// The event leaves the queue only after its commit; every
    /// integrity check happens before.
    fn settle_next(&mut self, journal: &mut dyn Journal) -> Result<(), VmError> {
        let SettledEntry { event, trigger } = self
            .settled
            .front()
            .cloned()
            .unwrap_or_else(|| unreachable!("guarded by the caller"));
        let slot = self.slot_of(&event.connector_path);
        let Some(request) = self.pending.get(&event.call_ref).map(|held| &held.delivery) else {
            return Err(VmError::MissingPending(event.call_ref));
        };
        let executor = TransactionExecutor::new(self.plan, self.graph_version);
        // A materialized partner answer crosses the connector's
        // fan-out like any content: every destination pin is judged
        // BEFORE the outcome commits, and a breach leaves as the
        // typed error the machine already speaks — never a silent
        // crossing.
        if let CallOutcome::Success(Data::Materialized(value)) = &event.outcome {
            let out = node::Port::new("out").unwrap_or_else(|_| unreachable!("a valid port"));
            let from = EndpointRef {
                node: Some(slot),
                port: out,
            };
            // The connector's own pin is the CONTRACT's reply type —
            // a mock answering a record over jsonl is judged exactly
            // like a real child would be shaped: array<variant> or
            // the typed refusal. `connector_out_pin` is the ONE
            // effective-pin authority (D-2026-08-11-1, amended).
            let contract_pin = match &self.plan.plan().nodes[slot.index()].kind {
                NodeKind::Connector(spec) => crate::boundary::connector_out_pin(spec),
                NodeKind::Operator => None,
            };
            if let Err(fault) = crate::runtime::crossing_pins_hold(
                self.plan.plan(),
                &from,
                contract_pin.as_ref(),
                &crate::runtime::from_envelope_value(value),
            )
            .map_err(crate::runtime::PinViolation::into_fault)
            {
                let outcome = executor.typed_error_transaction(
                    slot,
                    request,
                    "RuntimeFault",
                    &alloc::string::ToString::to_string(&fault),
                );
                self.validate_and_reserve_emissions(slot, &outcome.record)?;
                journal
                    .commit(outcome.record.clone())
                    .map_err(VmError::Journal)?;
                self.settled.pop_front();
                self.pending.remove(&event.call_ref);
                self.scheduler.mailboxes.release(slot, "");
                self.route_emissions(slot, &outcome.record);
                return Ok(());
            }
        }
        let outcome = executor.connector_outcome(
            &event,
            &trigger,
            request.envelope.meta(),
            ConsumedPosition {
                edge: request.edge.clone(),
                seq: request.seq,
            },
        );
        self.validate_and_reserve_emissions(slot, &outcome.record)?;
        journal
            .commit(outcome.record.clone())
            .map_err(VmError::Journal)?;
        self.settled.pop_front();
        self.pending.remove(&event.call_ref);
        self.scheduler.mailboxes.release(slot, "");
        self.route_emissions(slot, &outcome.record);
        Ok(())
    }

    fn fire_operator(
        &mut self,
        journal: &mut dyn Journal,
        slot: Slot,
        key: &Key,
        delivery: &Delivery,
    ) -> Result<(), VmError> {
        let executor = TransactionExecutor::new(self.plan, self.graph_version);
        let state = self.state.cell_unchecked(self.plan.plan(), slot, key);
        let outcome = match executor.fire(slot, delivery, &state) {
            Ok(outcome) => outcome,
            // A missing handler is a wiring integrity breach, not a
            // value fault — nothing can consume this delivery.
            Err(fault @ Fault::MissingHandler) => return Err(VmError::Fault(fault)),
            // A value fault consumes the input causally and leaves as
            // a typed error — the machine never swallows an envelope.
            Err(fault) => executor.fault_transaction(slot, delivery, &fault),
        };
        self.validate_and_reserve_emissions(slot, &outcome.record)?;
        journal
            .commit(outcome.record.clone())
            .map_err(VmError::Journal)?;
        self.scheduler.mailboxes.pop(slot, key);
        self.state
            .apply_unchecked(self.plan.plan(), slot, key, &outcome.record.state_delta);
        self.route_emissions(slot, &outcome.record);
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
        key: &Key,
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
        // unframeable request is a typed decode error before any
        // open fact exists.
        let prepared = match crate::boundary::prepare_request(
            crate::boundary::RequestCodec::of(spec),
            delivery.envelope.data(),
        ) {
            Ok(prepared) => prepared,
            Err(message) => {
                let executor = TransactionExecutor::new(self.plan, self.graph_version);
                let outcome =
                    executor.typed_error_transaction(slot, delivery, "DecodeError", &message);
                self.validate_and_reserve_emissions(slot, &outcome.record)?;
                journal
                    .commit(outcome.record.clone())
                    .map_err(VmError::Journal)?;
                self.scheduler.mailboxes.pop(slot, key);
                self.route_emissions(slot, &outcome.record);
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
        self.pending.insert(call_ref, PendingCall { delivery });
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

    /// Reserves room in every mailbox a record's emissions will reach.
    /// Called before the commit: a refusal is pure backpressure.
    /// The FINAL gate every commit candidate passes — validation
    /// and reservation, in that order, both named by this function.
    ///
    /// Pin conformance runs at TWO levels, deliberately: the
    /// RECOVERABLE level at construction (an operator's or a
    /// partner's value breach converts into a typed error on `err`)
    /// and THIS final, non-recursive level — a synthesized error
    /// that itself violates its wiring cannot become another typed
    /// error on the same wiring, so it surfaces as the machine
    /// fault it is and NOTHING commits.
    fn validate_and_reserve_emissions(
        &self,
        slot: Slot,
        record: &crate::runtime::TransactionRecord,
    ) -> Result<(), VmError> {
        for emission in &record.emissions {
            if let Data::Materialized(value) = emission.envelope.data() {
                crate::runtime::value_well_formed(value).map_err(VmError::Fault)?;
                let own_pin = self.plan.nodes[slot.index()]
                    .ports
                    .iter()
                    .find(|decl| decl.port == emission.port)
                    .and_then(|decl| decl.ty.as_ref());
                let from = EndpointRef {
                    node: Some(slot),
                    port: emission.port.clone(),
                };
                // Convert only when a pin actually watches this
                // fan-out — an untyped graph pays nothing here.
                let any_pin = own_pin.is_some()
                    || self.plan.edges.iter().any(|edge| {
                        edge.from == from
                            && edge.to.as_ref().is_some_and(|to| match to.node {
                                Some(target) => self.plan.nodes[target.index()]
                                    .ports
                                    .iter()
                                    .any(|decl| decl.port == to.port && decl.ty.is_some()),
                                None => self
                                    .plan
                                    .boundary
                                    .iter()
                                    .any(|decl| decl.port == to.port && decl.ty.is_some()),
                            })
                    });
                if any_pin {
                    crate::runtime::crossing_pins_hold(
                        self.plan.plan(),
                        &from,
                        own_pin,
                        &crate::runtime::from_envelope_value(value),
                    )
                    .map_err(|violation| VmError::Fault(violation.into_fault()))?;
                }
            }
        }
        let mut needed: BTreeMap<Slot, usize> = BTreeMap::new();
        for emission in &record.emissions {
            let from = EndpointRef {
                node: Some(slot),
                port: emission.port.clone(),
            };
            for edge in self.plan.edges.iter().filter(|edge| edge.from == from) {
                if let Some(to) = &edge.to
                    && let Some(target) = to.node
                {
                    *needed.entry(target).or_default() += 1;
                }
            }
        }
        self.check_room(&needed)
    }

    /// Reserves room for one envelope leaving a boundary endpoint.
    fn reserve_targets(&self, from: &EndpointRef) -> Result<(), VmError> {
        let mut needed: BTreeMap<Slot, usize> = BTreeMap::new();
        for edge in self.plan.edges.iter().filter(|edge| edge.from == *from) {
            if let Some(to) = &edge.to
                && let Some(target) = to.node
            {
                *needed.entry(target).or_default() += 1;
            }
        }
        self.check_room(&needed)
    }

    fn check_room(&self, needed: &BTreeMap<Slot, usize>) -> Result<(), VmError> {
        for (target, count) in needed {
            if !self
                .scheduler
                .mailboxes
                .has_room(*target, "", count.saturating_sub(1))
            {
                return Err(VmError::MailboxFull {
                    node: self.plan.nodes[target.index()].path.clone(),
                });
            }
        }
        Ok(())
    }

    /// Routes a committed record's emissions — infallible: capacity
    /// was reserved before the commit.
    fn route_emissions(&mut self, slot: Slot, record: &crate::runtime::TransactionRecord) {
        for emission in &record.emissions {
            // The emission itself is observable at its source port —
            // `expect relay.out` watches departures, not arrivals.
            self.observed.push(Observed {
                node: Some(self.plan.nodes[slot.index()].path.clone()),
                port: emission.port.clone(),
                envelope: emission.envelope.clone(),
            });
            let from = EndpointRef {
                node: Some(slot),
                port: emission.port.clone(),
            };
            let claimed = self.route(&from, &emission.envelope);
            // An `err` no edge claims bubbles to the root error sink
            // instead of vanishing.
            if claimed == 0 && emission.port.name() == "err" {
                self.errors.push(emission.envelope.clone());
                self.observed.push(Observed {
                    node: None,
                    port: emission.port.clone(),
                    envelope: emission.envelope.clone(),
                });
            }
        }
    }

    /// Delivers along every edge leaving `from`; returns how many
    /// edges claimed the envelope.
    fn route(&mut self, from: &EndpointRef, envelope: &Envelope) -> usize {
        let edges: Vec<Edge> = self
            .plan
            .edges
            .iter()
            .filter(|edge| edge.from == *from)
            .cloned()
            .collect();
        let claimed = edges.len();
        for edge in edges {
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
                            .deliver(target, "", to.port.clone(), edge_text, envelope.clone())
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
                        if let Some(crate::plan::AttrSetting::Value(crate::cel::Value::Duration(
                            budget,
                        ))) = self.plan.nodes[target.index()]
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
