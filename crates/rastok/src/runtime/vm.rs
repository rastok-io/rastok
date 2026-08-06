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
use crate::plan::{Edge, EndpointRef, ExecutionPlan, GraphVersion, NodeKind, Slot};
use crate::runtime::{Delivery, Fault, Key, Mailboxes, PartitionState, TransactionExecutor};
use alloc::collections::{BTreeMap, VecDeque};
use core::fmt;

/// A machine fault; the step that raised it committed nothing, and
/// the work that triggered it stays reserved — a retry sees it again.
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
    plan: &'plan ExecutionPlan,
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
    plan: &'plan ExecutionPlan,
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
    outputs: Vec<Envelope>,
    errors: Vec<Envelope>,
    observed: Vec<Observed>,
}

impl<'plan> Vm<'plan> {
    /// Boots the machine over a compiled plan — once. This is a fresh
    /// boot; recovering obligations from an existing journal is the
    /// recovery fold's job and is not in this build.
    #[must_use]
    pub fn boot(plan: &'plan ExecutionPlan, graph_version: GraphVersion) -> Self {
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
    /// boundary `in` edges. Capacity is reserved before the journal
    /// hears: a refused injection commits nothing, and backpressure
    /// reaches the ingress instead of stranding a committed birth.
    ///
    /// # Errors
    ///
    /// Journal refusals and full target mailboxes; both leave the
    /// journal untouched by this call.
    pub fn inject(
        &mut self,
        journal: &mut dyn Journal,
        submission: InboundSubmission,
    ) -> Result<Id, VmError> {
        let boundary_in =
            node::Port::new("in").unwrap_or_else(|_| unreachable!("`in` is a valid port"));
        let from = EndpointRef {
            node: None,
            port: boundary_in,
        };
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
            let effect = match spec.effect {
                node::Effect::Class(class) => class,
                node::Effect::None => unreachable!("an effectless connector opens no calls"),
            };
            match crate::connectors::prepare(spec, queued.envelope.data()) {
                Err(_) => None,
                Ok(bytes) => {
                    let mut hasher = blake3::Hasher::new();
                    hasher.update(&bytes);
                    let mut digest = [0u8; 32];
                    hasher.finalize_xof().fill(&mut digest);
                    let emitting_id = queued.envelope.meta().sys().id();
                    Some(Intent {
                        call_ref: ids::call_ref(emitting_id, &planned.path, &digest),
                        emitting_id,
                        connector_path: planned.path.clone(),
                        request_digest: digest,
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
                self.reserve_emissions(slot, &outcome.record)?;
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

    /// The envelopes that left the graph through its boundary.
    #[must_use]
    pub fn outputs(&self) -> &[Envelope] {
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
        let outcome = executor.connector_outcome(
            &event,
            &trigger,
            request.envelope.meta().sys(),
            ConsumedPosition {
                edge: request.edge.clone(),
                seq: request.seq,
            },
        );
        self.reserve_emissions(slot, &outcome.record)?;
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
        let state = self.state.cell(self.plan, slot, key);
        let outcome = match executor.fire(slot, delivery, &state) {
            Ok(outcome) => outcome,
            // A missing handler is a wiring integrity breach, not a
            // value fault — nothing can consume this delivery.
            Err(fault @ Fault::MissingHandler) => return Err(VmError::Fault(fault)),
            // A value fault consumes the input causally and leaves as
            // a typed error — the machine never swallows an envelope.
            Err(fault) => executor.fault_transaction(slot, delivery, &fault),
        };
        self.reserve_emissions(slot, &outcome.record)?;
        journal
            .commit(outcome.record.clone())
            .map_err(VmError::Journal)?;
        self.scheduler.mailboxes.pop(slot, key);
        self.state
            .apply(self.plan, slot, key, &outcome.record.state_delta);
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
        let effect = match spec.effect {
            node::Effect::Class(class) => class,
            node::Effect::None => {
                unreachable!("an effectless connector opens no calls")
            }
        };
        let emitting_id = delivery.envelope.meta().sys().id();
        // The digest seals the EXACT protocol bytes: preparation is
        // the same pure function the adapter uses, so what is sealed
        // and what is sent cannot diverge. An unframeable request is
        // a typed decode error before any open fact exists.
        let request_digest = match crate::connectors::prepare(spec, delivery.envelope.data()) {
            Ok(bytes) => {
                let mut hasher = blake3::Hasher::new();
                hasher.update(&bytes);
                let mut digest = [0u8; 32];
                hasher.finalize_xof().fill(&mut digest);
                digest
            }
            Err(message) => {
                let executor = TransactionExecutor::new(self.plan, self.graph_version);
                let outcome =
                    executor.typed_error_transaction(slot, delivery, "DecodeError", &message);
                self.reserve_emissions(slot, &outcome.record)?;
                journal
                    .commit(outcome.record.clone())
                    .map_err(VmError::Journal)?;
                self.scheduler.mailboxes.pop(slot, key);
                self.route_emissions(slot, &outcome.record);
                return Ok(());
            }
        };
        let intent = Intent {
            call_ref: ids::call_ref(emitting_id, &planned.path, &request_digest),
            emitting_id,
            connector_path: planned.path.clone(),
            request_digest,

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
    fn reserve_emissions(
        &self,
        slot: Slot,
        record: &crate::runtime::TransactionRecord,
    ) -> Result<(), VmError> {
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
                        self.outputs.push(envelope.clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::{
        CallOutcome, Draft, IngressAuthority, Response, SubmissionSequence, TypedError,
    };
    use crate::compiler::compile;
    use crate::envelope::{Encoding, Namespace, Scalar, Value};
    use crate::journal::{MemJournal, OutboundStatus, Record};
    use crate::stdlib::Registry;

    const M0_PROBE: &str = r#"
graph m0_probe : App {
  connector echo : Process {
    mode = job ; command = "cat"
    codec = lines
    effect = idempotent ; behavior = deterministic
  }

  operator shout {
    var count : int = 0
    on in where data.text != "" {
      count = count + 1
      out = data.text
    }
  }

  . -> shout -> echo -> .
}
"#;

    /// The world scripted: echoes every request back, like `cat`.
    #[derive(Default)]
    struct EchoWorld {
        seen: Vec<Intent>,
        fail_next: bool,
    }

    impl Partner for EchoWorld {
        fn start(&mut self, intent: &Intent, request: &Envelope) {
            let _ = request;
            self.seen.push(intent.clone());
        }

        fn events(&mut self) -> Vec<Response> {
            self.seen
                .drain(..)
                .map(|intent| Response {
                    call_ref: intent.call_ref,
                    outcome: if self.fail_next {
                        CallOutcome::Failure(TypedError {
                            kind: String::from("ExecFailed"),
                            data: Data::encoded(Encoding::Raw, *b"exit 1"),
                        })
                    } else {
                        CallOutcome::Success(Data::encoded(Encoding::Raw, *b"salut"))
                    },
                })
                .collect()
        }
    }

    fn submission(text: &str) -> InboundSubmission {
        let mut data = std::collections::BTreeMap::new();
        data.insert(
            String::from("text"),
            Value::Scalar(Scalar::Text(String::from(text))),
        );
        InboundSubmission {
            authority: IngressAuthority::single(node::Path::new(["cli"]).unwrap()),
            draft: Draft {
                data: Data::materialized(Value::Map(data)),
                mime_type: None,
                app: Namespace::new(),
            },
            sequence: SubmissionSequence::Next,
        }
    }

    fn run_probe() -> (MemJournal, Vec<Envelope>) {
        let compilation = compile(&[M0_PROBE], &Registry::m0()).expect("a valid plan");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        let mut world = EchoWorld::default();
        vm.inject(&mut journal, submission("salut")).unwrap();
        vm.drain(&mut journal).unwrap();
        vm.pump(&mut journal, &mut world).unwrap();
        vm.drain(&mut journal).unwrap();
        assert!(vm.is_stable());
        (journal, vm.outputs().to_vec())
    }

    #[test]
    fn the_m0_probe_runs_end_to_end() {
        let (journal, outputs) = run_probe();
        // The whole mechanical chain, in causal order: birth, the
        // operator's transaction, the call's open and closure facts,
        // the connector's outcome transaction.
        let kinds: Vec<&str> = journal
            .records()
            .iter()
            .map(|record| match record {
                Record::Inbound(_) => "inbound",
                Record::Transaction(t) => {
                    if t.node_path.to_string() == "shout" {
                        "tx:shout"
                    } else {
                        "tx:echo"
                    }
                }
                Record::Outbound(o) => match o.status {
                    OutboundStatus::Open => "outbound:open",
                    OutboundStatus::Closed(_) => "outbound:closed",
                    OutboundStatus::Abandoned => "outbound:abandoned",
                },
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "inbound",
                "tx:shout",
                "outbound:open",
                "outbound:closed",
                "tx:echo"
            ],
            "the normative record sequence"
        );
        // One envelope left the graph, with the root's origin.
        assert_eq!(outputs.len(), 1);
        let Record::Inbound(root) = &journal.records()[0] else {
            panic!("expected the birth");
        };
        assert_eq!(
            outputs[0].meta().sys().origin_id(),
            root.envelope.meta().sys().id(),
            "the lineage crosses the world boundary"
        );
    }

    #[test]
    fn two_runs_produce_identical_journals() {
        let (once, first_outputs) = run_probe();
        let (twice, second_outputs) = run_probe();
        assert_eq!(once.records(), twice.records(), "replay determinism");
        assert_eq!(first_outputs, second_outputs);
    }

    #[test]
    fn a_typed_failure_leaves_by_the_err_edge_or_stops() {
        let compilation = compile(&[M0_PROBE], &Registry::m0()).expect("a valid plan");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        let mut world = EchoWorld {
            fail_next: true,
            ..EchoWorld::default()
        };
        vm.inject(&mut journal, submission("salut")).unwrap();
        vm.drain(&mut journal).unwrap();
        vm.pump(&mut journal, &mut world).unwrap();
        vm.drain(&mut journal).unwrap();
        // The failure became the connector's err emission; no edge
        // routes err in the probe, so it bubbles to the root error
        // sink instead of vanishing — with its kind intact.
        assert!(vm.outputs().is_empty());
        let last = journal.records().last().unwrap();
        let Record::Transaction(transaction) = last else {
            panic!("expected the outcome transaction");
        };
        assert_eq!(transaction.emissions[0].port.name(), "err");
        assert_eq!(vm.errors().len(), 1);
        let Data::Materialized(Value::Map(payload)) = vm.errors()[0].data() else {
            panic!("expected the typed error payload");
        };
        assert_eq!(
            payload.get("kind"),
            Some(&Value::Scalar(Scalar::Text(String::from("ExecFailed")))),
            "the error kind survives into the graph"
        );
    }

    #[test]
    fn an_invented_error_kind_is_refused_loudly() {
        let compilation = compile(&[M0_PROBE], &Registry::m0()).expect("a valid plan");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        vm.inject(&mut journal, submission("salut")).unwrap();
        vm.drain(&mut journal).unwrap();
        let open = journal.open_calls();
        let mut world = ScriptedWorld {
            answers: vec![Response {
                call_ref: open[0].call_ref,
                outcome: CallOutcome::Failure(TypedError {
                    kind: String::from("MadeUp"),
                    data: Data::encoded(Encoding::Raw, *b""),
                }),
            }],
        };
        let report = vm.pump(&mut journal, &mut world).unwrap();
        // Refused BEFORE the causal fact: the reply is rejected with
        // its reason, and the call stays open — the timeout machinery
        // can still settle it honestly.
        assert_eq!(report.settled, 0);
        assert_eq!(report.rejected.len(), 1);
        assert!(report.rejected[0].1.contains("MadeUp"));
        assert_eq!(journal.open_calls().len(), 1, "no garbage closure");
        vm.drain(&mut journal).unwrap();
        assert!(!vm.is_stable(), "the obligation is still open");
    }

    /// A world answering from a script, for integrity tests.
    struct ScriptedWorld {
        answers: Vec<Response>,
    }

    impl Partner for ScriptedWorld {
        fn start(&mut self, _: &Intent, _: &Envelope) {}
        fn events(&mut self) -> Vec<Response> {
            std::mem::take(&mut self.answers)
        }
    }

    #[test]
    fn a_value_fault_becomes_a_typed_error_not_a_lost_envelope() {
        let source = r#"
graph g : App {
  operator pick {
    on in {
      out = data.missing
    }
  }
  . -> pick -> .
}
"#;
        let compilation = compile(&[source], &Registry::m0()).expect("a valid plan");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        vm.inject(&mut journal, submission("salut")).unwrap();
        vm.drain(&mut journal).unwrap();
        // The input is consumed causally — a transaction exists — and
        // the fault left as a typed error on the root sink.
        assert!(vm.is_stable(), "the machine did not wedge");
        let last = journal.records().last().unwrap();
        let Record::Transaction(transaction) = last else {
            panic!("expected the fault transaction");
        };
        assert_eq!(transaction.emissions[0].port.name(), "err");
        assert_eq!(vm.errors().len(), 1);
        let Data::Materialized(Value::Map(payload)) = vm.errors()[0].data() else {
            panic!("expected the typed error payload");
        };
        assert_eq!(
            payload.get("kind"),
            Some(&Value::Scalar(Scalar::Text(String::from("RuntimeFault"))))
        );
    }

    /// A journal that refuses commits on demand — the durable-store
    /// failure, scripted.
    struct FlakyJournal {
        inner: MemJournal,
        refuse_commits: bool,
        refuse_opens: bool,
        refuse_closes: bool,
    }

    impl FlakyJournal {
        fn healthy(version: GraphVersion) -> Self {
            Self {
                inner: MemJournal::new(version),
                refuse_commits: false,
                refuse_opens: false,
                refuse_closes: false,
            }
        }
    }

    impl Journal for FlakyJournal {
        fn append_inbound(
            &mut self,
            submission: InboundSubmission,
        ) -> Result<crate::journal::CommittedInbound, crate::journal::Error> {
            self.inner.append_inbound(submission)
        }
        fn commit(
            &mut self,
            transaction: crate::runtime::TransactionRecord,
        ) -> Result<(), crate::journal::Error> {
            if self.refuse_commits {
                return Err(crate::journal::Error::Storage(String::from("scripted")));
            }
            self.inner.commit(transaction)
        }
        fn open_outbound(&mut self, intent: Intent) -> Result<(), crate::journal::Error> {
            if self.refuse_opens {
                return Err(crate::journal::Error::Storage(String::from("scripted")));
            }
            self.inner.open_outbound(intent)
        }
        fn close_outbound(
            &mut self,
            call_ref: &crate::boundary::CallRef,
            closure: crate::journal::Closure,
        ) -> Result<crate::journal::CloseVerdict, crate::journal::Error> {
            if self.refuse_closes {
                return Err(crate::journal::Error::Storage(String::from("scripted")));
            }
            self.inner.close_outbound(call_ref, closure)
        }
        fn find_envelope(&self, id: crate::envelope::Id) -> Option<Envelope> {
            self.inner.find_envelope(id)
        }
        fn find_call(&self, call_ref: &crate::boundary::CallRef) -> Option<Intent> {
            self.inner.find_call(call_ref)
        }
        fn open_calls(&self) -> Vec<Intent> {
            self.inner.open_calls()
        }
    }

    #[test]
    fn a_refused_commit_leaves_the_delivery_replayable() {
        let compilation = compile(&[M0_PROBE], &Registry::m0()).expect("a valid plan");
        let mut journal = FlakyJournal {
            refuse_commits: true,
            ..FlakyJournal::healthy(GraphVersion(1))
        };
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        vm.inject(&mut journal, submission("salut")).unwrap();
        let error = vm.step(&mut journal).unwrap_err();
        assert!(matches!(error, VmError::Journal(_)));
        // The store recovers; the same delivery replays and commits —
        // nothing was consumed by the refusal.
        journal.refuse_commits = false;
        assert!(
            vm.step(&mut journal).unwrap(),
            "the delivery was still there"
        );
        assert!(matches!(
            journal.inner.records().last().unwrap(),
            Record::Transaction(_)
        ));
    }

    const TIMED_PROBE: &str = r#"
graph timed : App {
  connector echo : Process {
    mode = job ; command = "cat"
    codec = lines
    effect = idempotent ; behavior = deterministic
    timeout = duration("5s")
  }

  operator shout {
    var count : int = 0
    on in where data.text != "" {
      count = count + 1
      out = data.text
    }
  }

  . -> shout -> echo -> .
}
"#;

    #[test]
    fn a_silent_partner_expires_into_a_typed_timeout() {
        let compilation = compile(&[TIMED_PROBE], &Registry::m0()).expect("a valid plan");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        vm.inject(&mut journal, submission("salut")).unwrap();
        vm.drain(&mut journal).unwrap();
        assert!(vm.has_open_obligations());
        // Nobody answers; the budget expires at the discrete advance.
        vm.advance(&mut journal, &mut EchoWorld::default(), 5_000_000_000)
            .unwrap();
        assert!(vm.is_stable(), "the expiry released the machine");
        assert_eq!(vm.now(), 5_000_000_000);
        // The closure is a Timeout fact and the outcome transaction
        // fired under l:timeout, a TimeoutExpired on err.
        let last = journal.records().last().unwrap();
        let Record::Transaction(transaction) = last else {
            panic!("expected the timeout transaction");
        };
        assert_eq!(
            transaction.trigger,
            node::Trigger::Lifecycle(node::Lifecycle::Timeout)
        );
        assert_eq!(transaction.emissions[0].port.name(), "err");
        assert_eq!(vm.errors().len(), 1);
        let Data::Materialized(Value::Map(payload)) = vm.errors()[0].data() else {
            panic!("expected the typed error payload");
        };
        assert_eq!(
            payload.get("kind"),
            Some(&Value::Scalar(Scalar::Text(String::from("TimeoutExpired"))))
        );
        // The claim lifted: a second submission opens a second call.
        vm.inject(&mut journal, submission("encore")).unwrap();
        vm.drain(&mut journal).unwrap();
        assert_eq!(journal.open_calls().len(), 1);
    }

    #[test]
    fn a_budget_runs_from_delivery_and_covers_queue_time() {
        // The claim serializes the connector: the second request
        // waits in the mailbox behind the open call. Its budget runs
        // anyway — queue time counts (execution-model §8) — and the
        // call-closed rule applies even to a call that expires
        // before it started: the queued request's call opens and
        // closes at the expiry, back to back.
        let compilation = compile(&[TIMED_PROBE], &Registry::m0()).expect("a valid plan");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        let mut world = EchoWorld::default();
        vm.inject(&mut journal, submission("un")).unwrap();
        vm.inject(&mut journal, submission("deux")).unwrap();
        vm.drain(&mut journal).unwrap();
        assert_eq!(journal.open_calls().len(), 1, "one open, one queued");
        // Nobody answers; both budgets expire at the same sweep.
        vm.advance(&mut journal, &mut world, 5_000_000_000).unwrap();
        assert!(vm.is_stable());
        assert_eq!(vm.errors().len(), 2, "both requests expired, typed");
        let statuses: Vec<&OutboundStatus> = journal
            .records()
            .iter()
            .filter_map(|record| match record {
                Record::Outbound(o) => Some(&o.status),
                _ => None,
            })
            .collect();
        assert_eq!(
            statuses
                .iter()
                .filter(|status| matches!(status, OutboundStatus::Open))
                .count(),
            2,
            "both calls opened — the queued one at its expiry"
        );
        assert_eq!(
            statuses
                .iter()
                .filter(|status| matches!(
                    status,
                    OutboundStatus::Closed(crate::journal::Closure::Timeout)
                ))
                .count(),
            2,
            "both closed by the machine's Timeout fact"
        );
        // Both outcome transactions fired under the timeout
        // lifecycle, never a port trigger.
        let timeout_transactions = journal
            .records()
            .iter()
            .filter(|record| {
                matches!(
                    record,
                    Record::Transaction(t)
                        if t.trigger == node::Trigger::Lifecycle(node::Lifecycle::Timeout)
                )
            })
            .count();
        assert_eq!(timeout_transactions, 2);
        // The sealed timers name the obligations the transactions
        // armed — one per budgeted delivery.
        let sealed: usize = journal
            .records()
            .iter()
            .filter_map(|record| match record {
                Record::Transaction(t) => Some(t.timers.len()),
                _ => None,
            })
            .sum();
        assert_eq!(sealed, 2);
    }

    #[test]
    fn a_fan_out_expires_its_own_call_not_a_siblings() {
        // The seventh review's counter-example: one envelope fans out
        // to two budgeted connectors; the shorter budget must expire
        // ITS call — the obligation is (connector, delivery), never
        // the delivery alone.
        let source = r#"
graph fanout : App {
  connector alpha : Process {
    mode = job ; command = "cat"
    codec = lines
    effect = idempotent ; behavior = deterministic
    timeout = duration("500ms")
  }

  connector beta : Process {
    mode = job ; command = "cat"
    codec = lines
    effect = idempotent ; behavior = deterministic
    timeout = duration("100ms")
  }

  operator shout {
    on in { out = data.text }
  }

  . -> shout
  shout -> alpha -> nothing
  shout -> beta -> nothing
}
"#;
        let compilation = compile(&[source], &Registry::m0()).expect("a valid plan");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        let mut world = EchoWorld::default();
        vm.inject(&mut journal, submission("salut")).unwrap();
        vm.drain(&mut journal).unwrap();
        assert_eq!(journal.open_calls().len(), 2, "both calls opened");
        // Only beta's budget is due at 100ms.
        vm.advance(&mut journal, &mut world, 100_000_000).unwrap();
        let open = journal.open_calls();
        assert_eq!(open.len(), 1);
        assert_eq!(
            open[0].connector_path.to_string(),
            "alpha",
            "alpha's call survived its sibling's expiry"
        );
    }

    #[test]
    fn an_answer_and_an_expiry_race_to_a_single_closure() {
        let compilation = compile(&[TIMED_PROBE], &Registry::m0()).expect("a valid plan");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        let mut world = EchoWorld::default();
        vm.inject(&mut journal, submission("salut")).unwrap();
        vm.drain(&mut journal).unwrap();
        // The partner answers first; the expiry then loses the race —
        // one closure fact, one outcome transaction, no duplicate.
        vm.pump(&mut journal, &mut world).unwrap();
        vm.drain(&mut journal).unwrap();
        vm.advance(&mut journal, &mut EchoWorld::default(), 5_000_000_000)
            .unwrap();
        assert!(vm.is_stable());
        let closures = journal
            .records()
            .iter()
            .filter(|record| {
                matches!(
                    record,
                    Record::Outbound(o) if matches!(o.status, OutboundStatus::Closed(_))
                )
            })
            .count();
        assert_eq!(closures, 1, "close-once");
        assert_eq!(vm.outputs().len(), 1);
        assert!(vm.errors().is_empty(), "no phantom timeout");
    }

    #[test]
    fn a_late_answer_is_a_reported_observation_not_a_silence() {
        let compilation = compile(&[TIMED_PROBE], &Registry::m0()).expect("a valid plan");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        vm.inject(&mut journal, submission("salut")).unwrap();
        vm.drain(&mut journal).unwrap();
        let call_ref = journal.open_calls()[0].call_ref;
        // The budget expires first…
        vm.advance(&mut journal, &mut EchoWorld::default(), 5_000_000_000)
            .unwrap();
        // …then the world answers a closed call.
        let mut late = ScriptedWorld {
            answers: vec![Response {
                call_ref,
                outcome: CallOutcome::Success(Data::encoded(Encoding::Raw, *b"tard")),
            }],
        };
        let report = vm.pump(&mut journal, &mut late).unwrap();
        assert_eq!(report.settled, 0);
        assert_eq!(report.discarded.len(), 1, "the late answer is visible");
        assert!(report.uncorrelated.is_empty());
    }

    #[test]
    fn a_refused_expiry_stays_armed_and_replays() {
        // The eighth review's invariant: after any refusal, the
        // obligation and its delivery are either fully resolved or
        // replayable — never half-consumed, never duplicated.
        // Case: the pending call's Timeout closure is refused.
        let compilation = compile(&[TIMED_PROBE], &Registry::m0()).expect("a valid plan");
        let mut journal = FlakyJournal {
            refuse_closes: true,
            ..FlakyJournal::healthy(GraphVersion(1))
        };
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        let mut world = EchoWorld::default();
        vm.inject(&mut journal, submission("salut")).unwrap();
        vm.drain(&mut journal).unwrap();
        assert!(
            vm.advance(&mut journal, &mut world, 5_000_000_000).is_err(),
            "the refused closure surfaces"
        );
        // Heal and replay: the timer was never acked.
        journal.refuse_closes = false;
        vm.advance(&mut journal, &mut world, 5_000_000_000).unwrap();
        assert!(vm.is_stable());
        assert_eq!(
            vm.errors().len(),
            1,
            "exactly one typed timeout — no duplicate fact"
        );
    }

    #[test]
    fn a_refused_open_on_a_queued_expiry_replays_without_duplicates() {
        // Case: the queued delivery's open fact is refused at expiry.
        let compilation = compile(&[TIMED_PROBE], &Registry::m0()).expect("a valid plan");
        let mut journal = FlakyJournal::healthy(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        let mut world = EchoWorld::default();
        vm.inject(&mut journal, submission("un")).unwrap();
        vm.inject(&mut journal, submission("deux")).unwrap();
        vm.drain(&mut journal).unwrap();
        assert_eq!(journal.inner.open_calls().len(), 1, "one open, one queued");
        journal.refuse_opens = true;
        assert!(vm.advance(&mut journal, &mut world, 5_000_000_000).is_err());
        journal.refuse_opens = false;
        vm.advance(&mut journal, &mut world, 5_000_000_000).unwrap();
        assert!(vm.is_stable());
        assert_eq!(vm.errors().len(), 2, "both expired exactly once");
    }

    #[test]
    fn a_refused_commit_on_an_unframeable_expiry_loses_nothing() {
        // Case: the queued delivery cannot be framed and the decode
        // transaction's commit is refused — the delivery must stay.
        let compilation = compile(
            &["graph g : App {\n  connector echo : Process {\n    mode = job ; command = \"cat\"\n    codec = lines\n    effect = idempotent ; behavior = deterministic\n    timeout = duration(\"5s\")\n  }\n  operator wrap {\n    on in { out = envelope }\n  }\n  . -> wrap -> echo -> nothing\n}\n"],
            &Registry::m0(),
        )
        .expect("a valid plan");
        let mut journal = FlakyJournal::healthy(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        let mut world = EchoWorld::default();
        vm.inject(&mut journal, submission("salut")).unwrap();
        // wrap re-emits the MAP; the delivery to echo is unframeable,
        // so it refuses at the head with a typed DecodeError… unless
        // it is still QUEUED behind nothing — drive only the wrap
        // transaction, then expire while queued.
        vm.step(&mut journal).unwrap();
        let facts_before = journal.inner.records().len();
        journal.refuse_commits = true;
        assert!(vm.advance(&mut journal, &mut world, 5_000_000_000).is_err());
        assert_eq!(
            journal.inner.records().len(),
            facts_before,
            "a refused commit adds no fact"
        );
        // Heal and replay: the delivery was never removed, the timer
        // never acked — exactly one typed error results.
        journal.refuse_commits = false;
        vm.advance(&mut journal, &mut world, 5_000_000_000).unwrap();
        vm.drain(&mut journal).unwrap();
        assert!(vm.is_stable());
        assert_eq!(vm.errors().len(), 1, "one typed error, no duplicate");
        let Data::Materialized(crate::envelope::Value::Map(payload)) = vm.errors()[0].data() else {
            panic!("expected the typed error payload");
        };
        assert_eq!(
            payload.get("kind"),
            Some(&crate::envelope::Value::Scalar(
                crate::envelope::Scalar::Text(String::from("DecodeError"))
            )),
            "the kind survived the replay"
        );
        let decode_transactions: Vec<_> = journal
            .inner
            .records()
            .iter()
            .filter_map(|record| match record {
                Record::Transaction(t) if t.node_path.to_string() == "echo" => Some(t),
                _ => None,
            })
            .collect();
        assert_eq!(
            decode_transactions.len(),
            1,
            "exactly one transaction consumed the delivery"
        );
        assert_eq!(decode_transactions[0].consumed.len(), 1);
    }

    #[test]
    fn a_suspended_call_keeps_its_mailbox_claimed() {
        let compilation = compile(&[M0_PROBE], &Registry::m0()).expect("a valid plan");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        let mut world = EchoWorld::default();
        vm.inject(&mut journal, submission("un")).unwrap();
        vm.inject(&mut journal, submission("deux")).unwrap();
        vm.drain(&mut journal).unwrap();
        // Two requests reached the connector, but only one call is
        // open: the second waits for the mailbox claim to lift.
        assert_eq!(journal.open_calls().len(), 1);
        assert!(vm.has_open_obligations());
        // Settle, then the claim lifts and the second call opens.
        vm.pump(&mut journal, &mut world).unwrap();
        vm.drain(&mut journal).unwrap();
        assert_eq!(journal.open_calls().len(), 1, "the second call is now open");
        vm.pump(&mut journal, &mut world).unwrap();
        vm.drain(&mut journal).unwrap();
        assert!(vm.is_stable());
        assert_eq!(vm.outputs().len(), 2);
    }

    #[test]
    fn a_filtered_input_reaches_stability_without_output() {
        let compilation = compile(&[M0_PROBE], &Registry::m0()).expect("a valid plan");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        vm.inject(&mut journal, submission("")).unwrap();
        vm.drain(&mut journal).unwrap();
        assert!(vm.is_stable(), "the guard filtered; nothing is pending");
        assert!(vm.outputs().is_empty());
        // The consumption is still a causal fact.
        assert!(matches!(
            journal.records()[1],
            Record::Transaction(ref t) if t.emissions.is_empty()
        ));
    }
}
