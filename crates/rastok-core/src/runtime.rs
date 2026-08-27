//! Deterministic execution machinery — the M0 single-worker core.
//!
//! Governing rule: [`execution-model.md`] — as-if-run-to-completion,
//! one node transaction at a time. This stage owns the transaction
//! machinery: keyed state cells, bounded per-key mailboxes, per-port
//! emission indices in text order, and the atomic executor — a
//! handler runs against scratch state, and either everything lands
//! (consumed input, state delta, emissions, the commit packet) or
//! nothing does. The scheduler and the VM assemble these parts at the
//! next stage.
//!
//! [`execution-model.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/execution-model.md

pub mod vm;

use crate::boundary::{Completion, InboundCallRef, SessionRef};
use crate::datatype::DataType;
use crate::envelope::{self, Data, Envelope, Meta, MetaSys, MetaValue, Scalar};
use crate::expr;
use crate::identity::InstanceRef;
use crate::journal::{ConsumedPosition, Emission, TransactionRecord, ids};
use crate::node;
use crate::plan::{
    Emission as PlanEmission, ExecutionPlan, GraphVersion, Handler, Slot, Step,
    ValidatedExecutionPlan,
};
use crate::value::{Value, ValueError};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::{format, vec};
use core::fmt;

pub use crate::datatype::{ConformanceReason, ConformanceViolation};
pub use crate::value::ValueKind;

/// Pure, deterministic, budgeted guest execution — the contract the
/// jq and SQL engines will implement; no engine ships in M0.
pub trait GuestExecutor {
    /// Runs an opaque guest program over a value within a budget.
    ///
    /// # Errors
    ///
    /// A typed guest fault; never an observation of the host.
    fn run(&self, program: &[u8], input: Value, budget: u64) -> Result<Value, Fault>;
}

/// A runtime fault of one transaction. Atomicity holds: a faulted
/// transaction changed nothing. This is supervision output from the VM,
/// never a payload emitted on an Operator port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// A Rastok expression evaluation fault.
    Eval(expr::Error),
    /// A value that envelope data cannot represent yet.
    Unrepresentable(String),
    /// A materialized value violates the core value algebra.
    InvalidValue(ValueError),
    /// The plan has no handler for this node and trigger.
    MissingHandler,
    /// The payload could not be materialized for the handler.
    Opaque(String),
    /// A caller-supplied slot the plan does not know. `Slot` is
    /// freely constructible; the validated plan proves only its OWN
    /// references, so the public doors verify and refuse — never
    /// index on faith.
    UnknownSlot(Slot),
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Eval(error) => error.fmt(f),
            Self::Unrepresentable(what) => {
                write!(f, "`{what}` is not representable in envelope data yet")
            }
            Self::InvalidValue(error) => error.fmt(f),
            Self::MissingHandler => f.write_str("no handler for this trigger"),
            Self::Opaque(what) => write!(f, "the payload is opaque here: {what}"),
            Self::UnknownSlot(slot) => {
                write!(f, "the plan has no node at slot {}", slot.index())
            }
        }
    }
}

impl core::error::Error for Fault {}

/// State cells indexed by complete instance reference: successive
/// generations at one logical address never share mutable state.
#[derive(Debug, Default)]
pub struct InstanceState {
    cells: BTreeMap<(Slot, InstanceRef), BTreeMap<String, Value>>,
}

impl InstanceState {
    /// Empty state; cells materialize from plan defaults on first
    /// access.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The current cell values for a node under a key, defaults
    /// applied from the plan on first access. The plan is validated,
    /// which proves the plan's OWN references — not the caller's
    /// `slot`, which is freely constructible and verified here.
    ///
    /// # Errors
    ///
    /// [`Fault::UnknownSlot`] when the plan has no such node.
    pub fn cell(
        &self,
        plan: &ValidatedExecutionPlan,
        slot: Slot,
        instance: &InstanceRef,
    ) -> Result<BTreeMap<String, Value>, Fault> {
        if plan.nodes.get(slot.index()).is_none() {
            return Err(Fault::UnknownSlot(slot));
        }
        Ok(self.cell_unchecked(plan.plan(), slot, instance))
    }

    /// The unchecked read the machine uses on its own plan-derived
    /// slots — crate-private: the public door above verifies.
    pub(crate) fn cell_unchecked(
        &self,
        plan: &ExecutionPlan,
        slot: Slot,
        instance: &InstanceRef,
    ) -> BTreeMap<String, Value> {
        if let Some(cell) = self.cells.get(&(slot, instance.clone())) {
            return cell.clone();
        }
        plan.nodes[slot.index()]
            .vars
            .iter()
            .filter_map(|var| var.init.clone().map(|value| (var.name.clone(), value)))
            .collect()
    }

    /// Applies a committed delta to a cell. Verified like
    /// [`Self::cell`].
    ///
    /// # Errors
    ///
    /// [`Fault::UnknownSlot`] when the plan has no such node.
    pub fn apply(
        &mut self,
        plan: &ValidatedExecutionPlan,
        slot: Slot,
        instance: &InstanceRef,
        delta: &[(String, Value)],
    ) -> Result<(), Fault> {
        if plan.nodes.get(slot.index()).is_none() {
            return Err(Fault::UnknownSlot(slot));
        }
        self.apply_unchecked(plan.plan(), slot, instance, delta);
        Ok(())
    }

    /// The unchecked write behind [`Self::apply`] — crate-private.
    pub(crate) fn apply_unchecked(
        &mut self,
        plan: &ExecutionPlan,
        slot: Slot,
        instance: &InstanceRef,
        delta: &[(String, Value)],
    ) {
        let mut cell = self.cell_unchecked(plan, slot, instance);
        for (name, value) in delta {
            cell.insert(name.clone(), value.clone());
        }
        self.cells.insert((slot, instance.clone()), cell);
    }

    /// Detaches one generation's state so a following generation of the same
    /// logical address can start while `on stop` still commits against the
    /// old snapshot.
    pub(crate) fn take_unchecked(
        &mut self,
        plan: &ExecutionPlan,
        slot: Slot,
        instance: &InstanceRef,
    ) -> BTreeMap<String, Value> {
        self.cells
            .remove(&(slot, instance.clone()))
            .unwrap_or_else(|| self.cell_unchecked(plan, slot, instance))
    }
}

/// One delivery waiting in a mailbox.
#[derive(Debug, Clone, PartialEq)]
pub struct Delivery {
    /// The arrival port — part of the delivery, because one node
    /// mailbox carries every port in one FIFO.
    pub port: node::Port,
    /// The canonical edge text the envelope traveled.
    pub edge: String,
    /// The position on that edge — the consumed cursor.
    pub seq: u64,
    /// The complete upstream instance reference that owns this cursor.
    pub source_instance: InstanceRef,
    /// The delivered envelope.
    pub envelope: Envelope,
}

/// Bounded per-`(node, instance-ref)` FIFO queues — one mailbox per node
/// instance, all ports in arrival order, per the execution model.
/// A claimed mailbox stays claimed while its node awaits an external
/// call: the worker is free, the mailbox is not. A full mailbox
/// refuses — backpressure is a signal, never a silent drop.
#[derive(Debug)]
pub struct Mailboxes {
    capacity: usize,
    queues: BTreeMap<(Slot, InstanceRef), VecDeque<Delivery>>,
    claimed: alloc::collections::BTreeSet<(Slot, InstanceRef)>,
    edge_seq: BTreeMap<(String, InstanceRef), u64>,
}

impl Mailboxes {
    /// Mailboxes bounded at `capacity` deliveries per queue.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            queues: BTreeMap::new(),
            claimed: alloc::collections::BTreeSet::new(),
            edge_seq: BTreeMap::new(),
        }
    }

    /// Whether one more delivery fits the `(slot, key)` queue after
    /// `already` reservations against it — the pre-commit capacity
    /// check.
    #[must_use]
    pub fn has_room(&self, slot: Slot, instance: &InstanceRef, already: usize) -> bool {
        let queued = self
            .queues
            .get(&(slot, instance.clone()))
            .map_or(0, VecDeque::len);
        queued + already < self.capacity
    }

    /// Delivers into a node's single FIFO, assigning the next
    /// per-edge sequence. Callers pre-check capacity; a full queue
    /// still refuses.
    ///
    /// # Errors
    ///
    /// The delivery's envelope back, boxed, when the queue is full.
    pub fn deliver(
        &mut self,
        slot: Slot,
        instance: &InstanceRef,
        source_instance: &InstanceRef,
        port: node::Port,
        edge: String,
        envelope: Envelope,
    ) -> Result<(), Box<Envelope>> {
        let queue = self.queues.entry((slot, instance.clone())).or_default();
        if queue.len() >= self.capacity {
            return Err(Box::new(envelope));
        }
        let seq = self
            .edge_seq
            .entry((edge.clone(), source_instance.clone()))
            .or_insert(0);
        *seq += 1;
        queue.push_back(Delivery {
            port,
            edge,
            seq: *seq,
            source_instance: source_instance.clone(),
            envelope,
        });
        Ok(())
    }

    /// The first unclaimed non-empty mailbox and a copy of its head —
    /// the reservation: nothing is removed until [`Self::pop`].
    #[must_use]
    pub fn peek_next(&self) -> Option<(Slot, InstanceRef, Delivery)> {
        self.peek_next_where(|_, _| true)
    }

    /// The first unclaimed non-empty mailbox whose instance is currently
    /// runnable. Successor-generation deliveries may already be committed
    /// while their predecessor closes; they remain in these same bounded
    /// queues but are not executable until their bundle is installed.
    #[must_use]
    pub fn peek_next_where(
        &self,
        ready: impl Fn(Slot, &InstanceRef) -> bool,
    ) -> Option<(Slot, InstanceRef, Delivery)> {
        self.queues
            .iter()
            .find(|((slot, key), queue)| {
                !queue.is_empty()
                    && !self.claimed.contains(&(*slot, key.clone()))
                    && ready(*slot, key)
            })
            .and_then(|((slot, key), queue)| {
                queue.front().map(|head| (*slot, key.clone(), head.clone()))
            })
    }

    /// The next canonically ordered unclaimed mailbox identity after
    /// `after`, when its instance is runnable. Returning one key at a time
    /// lets the VM skip an incomplete join without allocating a snapshot of
    /// every mailbox on the ordinary one-delivery hot path.
    pub(crate) fn next_runnable_key(
        &self,
        after: Option<&(Slot, InstanceRef)>,
        ready: impl Fn(Slot, &InstanceRef) -> bool,
    ) -> Option<(Slot, InstanceRef, Delivery)> {
        let accepted = |(key, queue): (&(Slot, InstanceRef), &VecDeque<Delivery>)| {
            let (slot, instance) = key;
            !queue.is_empty()
                && !self.claimed.contains(&(*slot, instance.clone()))
                && ready(*slot, instance)
        };
        let found = match after {
            Some(after) => self
                .queues
                .range((
                    core::ops::Bound::Excluded(after.clone()),
                    core::ops::Bound::Unbounded,
                ))
                .find(|entry| accepted(*entry)),
            None => self.queues.iter().find(|entry| accepted(*entry)),
        };
        found.and_then(|((slot, instance), queue)| {
            queue
                .front()
                .map(|delivery| (*slot, instance.clone(), delivery.clone()))
        })
    }

    /// Immutable FIFO contents for activation selection. The returned queue
    /// is only a reservation view; commit paths remove by full cursor/id.
    pub(crate) fn queued(&self, slot: Slot, instance: &InstanceRef) -> Option<&VecDeque<Delivery>> {
        self.queues.get(&(slot, instance.clone()))
    }

    /// Finds one specific queued delivery by its full reference —
    /// the canonical edge it traveled AND its envelope id — without
    /// removing it: the reservation half of the expired-in-queue
    /// path. Two ports of one connector are two references.
    #[must_use]
    pub fn find_by_ref(
        &self,
        slot: Slot,
        instance: &InstanceRef,
        edge: &str,
        id: crate::envelope::Id,
    ) -> Option<&Delivery> {
        self.queues
            .get(&(slot, instance.clone()))?
            .iter()
            .find(|delivery| delivery.edge == edge && delivery.envelope.meta().sys().id() == id)
    }

    /// Finds a delivery reference across every instance of one node. The
    /// delivery reference is globally injective for that fan-out; the key is
    /// returned because timeout obligations intentionally store the causal
    /// reference, not a duplicate routing decision.
    #[must_use]
    pub fn find_by_ref_any(
        &self,
        slot: Slot,
        edge: &str,
        id: crate::envelope::Id,
    ) -> Option<(InstanceRef, &Delivery)> {
        self.queues
            .iter()
            .filter(|((candidate, _), _)| *candidate == slot)
            .find_map(|((_, key), queue)| {
                queue
                    .iter()
                    .find(|delivery| {
                        delivery.edge == edge && delivery.envelope.meta().sys().id() == id
                    })
                    .map(|delivery| (key.clone(), delivery))
            })
    }

    /// Removes one specific queued delivery by its full reference —
    /// only after the facts consuming it hold; FIFO order among the
    /// survivors holds.
    pub fn remove_by_ref(
        &mut self,
        slot: Slot,
        instance: &InstanceRef,
        edge: &str,
        id: crate::envelope::Id,
    ) -> Option<Delivery> {
        let queue = self.queues.get_mut(&(slot, instance.clone()))?;
        let position = queue.iter().position(|delivery| {
            delivery.edge == edge && delivery.envelope.meta().sys().id() == id
        })?;
        queue.remove(position)
    }

    /// Removes a delivery reference without a duplicated target key, returning
    /// the instance it occupied.
    pub fn remove_by_ref_any(
        &mut self,
        slot: Slot,
        edge: &str,
        id: crate::envelope::Id,
    ) -> Option<(InstanceRef, Delivery)> {
        let key = self
            .queues
            .iter()
            .filter(|((candidate, _), _)| *candidate == slot)
            .find_map(|((_, key), queue)| {
                queue
                    .iter()
                    .any(|delivery| {
                        delivery.edge == edge && delivery.envelope.meta().sys().id() == id
                    })
                    .then(|| key.clone())
            })?;
        self.remove_by_ref(slot, &key, edge, id)
            .map(|delivery| (key, delivery))
    }

    /// Removes the head of a mailbox — only after the transaction
    /// that consumes it is committed.
    pub fn pop(&mut self, slot: Slot, instance: &InstanceRef) -> Option<Delivery> {
        self.queues
            .get_mut(&(slot, instance.clone()))
            .and_then(VecDeque::pop_front)
    }

    /// Claims a mailbox for the duration of an external call.
    pub fn claim(&mut self, slot: Slot, instance: &InstanceRef) {
        self.claimed.insert((slot, instance.clone()));
    }

    /// Releases a claim after the call settled.
    pub fn release(&mut self, slot: Slot, instance: &InstanceRef) {
        self.claimed.remove(&(slot, instance.clone()));
    }

    /// Removes every queued delivery for one mailbox in FIFO order. Bounded
    /// Closing records these positions on `l:stop`; this is not a silent drop.
    pub fn take_all(&mut self, slot: Slot, instance: &InstanceRef) -> Vec<Delivery> {
        self.queues
            .remove(&(slot, instance.clone()))
            .map_or_else(Vec::new, |queue| queue.into_iter().collect())
    }

    /// Whether any delivery is waiting anywhere.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queues.values().all(VecDeque::is_empty)
    }

    /// Whether any unclaimed mailbox has work.
    #[must_use]
    pub fn has_ready(&self) -> bool {
        self.peek_next().is_some()
    }

    /// Whether this exact instance still owns queued or claimed work.
    #[must_use]
    pub fn has_work(&self, slot: Slot, instance: &InstanceRef) -> bool {
        self.queues
            .get(&(slot, instance.clone()))
            .is_some_and(|queue| !queue.is_empty())
            || self.claimed.contains(&(slot, instance.clone()))
    }
}

/// Ordered per-port emissions of one transaction: text order fixes
/// the per-port emission indices the id recipe seals.
#[derive(Debug, Default)]
pub struct EmissionBuffer {
    next_index: BTreeMap<node::Port, u32>,
    emissions: Vec<Emission>,
}

impl EmissionBuffer {
    /// An empty buffer for one transaction.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The next emission index for a port, then advances it.
    pub fn next_index(&mut self, port: &node::Port) -> u32 {
        let index = self.next_index.entry(port.clone()).or_insert(0);
        let current = *index;
        *index += 1;
        current
    }

    /// Records one emission, in text order.
    pub fn push(&mut self, emission: Emission) {
        self.emissions.push(emission);
    }

    /// The emissions, in text order.
    #[must_use]
    pub fn into_emissions(self) -> Vec<Emission> {
        self.emissions
    }
}

/// The result of one atomic transaction: the commit packet and
/// nothing else — its `state_delta` is the single source of truth
/// the caller journals, applies, and routes from, in that order.
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionOutcome {
    /// The commit packet for the journal.
    pub record: TransactionRecord,
}

/// The causal identity and envelope of one lifecycle firing. Grouping these
/// fields prevents a caller from accidentally pairing a child reference with
/// another generation's parent or cause.
pub(crate) struct LifecycleFiring<'a> {
    pub(crate) instance: &'a InstanceRef,
    pub(crate) parent_instance: Option<&'a InstanceRef>,
    pub(crate) cause: &'a Envelope,
}

/// One complete join candidate in declared-port order. The trigger stays
/// with the tuple because it owns causal identity even when it is not the
/// first declared input.
pub(crate) struct JoinCandidate<'a> {
    pub(crate) activation: &'a node::HandlerActivation,
    pub(crate) trigger: &'a Delivery,
    pub(crate) inputs: &'a [Delivery],
}

/// Atomic node-transaction executor: scratch isolation while a
/// handler runs, everything or nothing after.
#[derive(Debug)]
pub struct TransactionExecutor<'plan> {
    plan: &'plan ValidatedExecutionPlan,
    graph_version: GraphVersion,
}

impl<'plan> TransactionExecutor<'plan> {
    /// An executor over a validated plan. The executor dereferences
    /// the plan without checking, so the proof enters here — the
    /// internal reference stays raw, but no public path constructs
    /// an executor around an unvalidated plan.
    #[must_use]
    pub fn new(plan: &'plan ValidatedExecutionPlan, graph_version: GraphVersion) -> Self {
        Self {
            plan,
            graph_version,
        }
    }

    /// Fires the handler of `slot` for a port arrival, atomically.
    /// A false guard still consumes the input — the cursor advance is
    /// a causal fact — but emits nothing and changes nothing.
    ///
    /// # Errors
    ///
    /// A [`Fault`]; the transaction then changed nothing at all.
    pub fn fire(
        &self,
        slot: Slot,
        instance: &InstanceRef,
        delivery: &Delivery,
        state: &BTreeMap<String, Value>,
    ) -> Result<TransactionOutcome, Fault> {
        self.fire_with_bindings(slot, instance, delivery, state, &BTreeMap::new())
    }

    /// Fires with the immutable bindings carried by the selected instance
    /// bundle. The VM uses this path; the public four-argument form remains a
    /// useful low-level probe for plans without instance values.
    pub(crate) fn fire_with_bindings(
        &self,
        slot: Slot,
        instance: &InstanceRef,
        delivery: &Delivery,
        state: &BTreeMap<String, Value>,
        instance_values: &BTreeMap<String, Value>,
    ) -> Result<TransactionOutcome, Fault> {
        // Two distinct facts share one port here: the handler is
        // *selected* by its activation pattern, while the committed
        // record carries the *causal* trigger. They coincide for a
        // single-port activation and will not for a join.
        let activation = node::HandlerActivation::Port(delivery.port.clone());
        let trigger = node::Trigger::Port(delivery.port.clone());
        let consumed = vec![ConsumedPosition {
            edge: delivery.edge.clone(),
            source: delivery.source_instance.clone(),
            seq: delivery.seq,
        }];
        self.fire_activation(
            slot,
            instance,
            &activation,
            &trigger,
            &delivery.envelope,
            None,
            None,
            consumed,
            state,
            instance_values,
            &BTreeMap::new(),
            &[],
        )
    }

    /// Tests a join candidate set without consuming it. `where` refines the
    /// retained candidates, so a false verdict must not manufacture a cursor
    /// advance. Evaluation is pure and sees the same merged metadata and
    /// named-envelope bindings as the eventual firing.
    pub(crate) fn join_matches(
        &self,
        slot: Slot,
        candidate: JoinCandidate<'_>,
        state: &BTreeMap<String, Value>,
        instance_values: &BTreeMap<String, Value>,
    ) -> Result<bool, Fault> {
        let Some(handler) =
            self.plan.handlers.iter().find(|handler| {
                handler.node == slot && &handler.activation == candidate.activation
            })
        else {
            return Err(Fault::MissingHandler);
        };
        let Some(guard) = &handler.guard else {
            return Ok(true);
        };
        let (input, extra) = join_context(self.plan.plan(), candidate.trigger, candidate.inputs);
        let mut base: BTreeMap<String, expr::EvalValue> = state
            .iter()
            .map(|(name, value)| (name.clone(), expr::EvalValue::from(value.clone())))
            .collect();
        for (name, value) in instance_values {
            base.insert(name.clone(), expr::EvalValue::from(value.clone()));
        }
        base.extend(extra);
        base.insert(
            String::from("meta"),
            expr::EvalValue::from(meta_value(input.meta())),
        );
        let payloads = PayloadSlots::new(&input, candidate.inputs);
        let verdict = resolve_payload_with_slots(
            eval_with_slots(guard, &base, &payloads, &self.plan.assets)?,
            &payloads,
        )?;
        match verdict {
            Value::Bool(value) => Ok(value),
            other => Err(Fault::Eval(expr::Error {
                kind: expr::ErrorKind::Evaluation,
                message: format!("the guard evaluated to {other:?}"),
            })),
        }
    }

    /// Fires one presence-completing join. Inputs are in the handler's
    /// declared port order; the trigger is the member that arrived last in
    /// the owning mailbox. Every cursor and contribution id commits in the
    /// same transaction.
    pub(crate) fn fire_join(
        &self,
        slot: Slot,
        instance: &InstanceRef,
        candidate: JoinCandidate<'_>,
        state: &BTreeMap<String, Value>,
        instance_values: &BTreeMap<String, Value>,
    ) -> Result<TransactionOutcome, Fault> {
        let (input, extra) = join_context(self.plan.plan(), candidate.trigger, candidate.inputs);
        let consumed = candidate
            .inputs
            .iter()
            .map(|delivery| ConsumedPosition {
                edge: delivery.edge.clone(),
                source: delivery.source_instance.clone(),
                seq: delivery.seq,
            })
            .collect();
        let mut outcome = self.fire_activation(
            slot,
            instance,
            candidate.activation,
            &node::Trigger::Port(candidate.trigger.port.clone()),
            &input,
            None,
            None,
            consumed,
            state,
            instance_values,
            &extra,
            candidate.inputs,
        )?;
        outcome.record.join_inputs = candidate
            .inputs
            .iter()
            .map(|delivery| delivery.envelope.meta().sys().id())
            .collect();
        Ok(outcome)
    }

    /// Constructs the explicit typed outcome of one retained join candidate
    /// being evicted. It is an ordinary emission on the declared `expired`
    /// port and consumes the candidate cursor atomically.
    pub(crate) fn join_expired(
        &self,
        slot: Slot,
        instance: &InstanceRef,
        delivery: &Delivery,
    ) -> Result<TransactionOutcome, Fault> {
        let Some(node) = self.plan.nodes.get(slot.index()) else {
            return Err(Fault::UnknownSlot(slot));
        };
        let port = node::Port::new("expired")
            .map_err(|error| Fault::Unrepresentable(error.to_string()))?;
        let trigger = node::Trigger::Lifecycle(node::Lifecycle::Evict);
        let parent = delivery.envelope.meta().sys();
        let id = ids::emission_id(&node.path, &trigger, parent.id(), &port, 0);
        let value = Value::TaggedUnion(Box::new(crate::value::TaggedUnionValue {
            case: String::from("JoinExpired"),
            payload: vec![],
        }));
        let from = crate::plan::EndpointRef {
            node: Some(slot),
            port: port.clone(),
        };
        crossing_pins_hold(self.plan, &from, &value).map_err(PinViolation::into_fault)?;
        let mut ext = delivery.envelope.meta().ext().clone();
        ext.remove("content_type");
        let meta = Meta::new(MetaSys::emission(id, parent, self.graph_version.0))
            .with_app(delivery.envelope.meta().app().clone())
            .with_ext(ext);
        Ok(TransactionOutcome {
            record: TransactionRecord {
                owner_path: crate::plan::OwnerPath::from(&node.path),
                instance: instance.clone(),
                parent_instance: None,
                close_scope: None,
                exchange_exit: None,
                trigger,
                cause: Some(delivery.envelope.meta().sys().id()),
                consumed: vec![ConsumedPosition {
                    edge: delivery.edge.clone(),
                    source: delivery.source_instance.clone(),
                    seq: delivery.seq,
                }],
                join_inputs: vec![],
                state_delta: vec![],
                emissions: vec![Emission {
                    port,
                    index: 0,
                    envelope: Envelope::new(meta, Data::materialized(value)),
                }],
                routes: vec![],
                timers: vec![],
            },
        })
    }

    /// Fires one instance lifecycle transaction. The causal envelope is
    /// explicit because no mailbox cursor is consumed; an absent lifecycle
    /// handler still commits the transition as an empty causal record.
    ///
    /// # Errors
    ///
    /// A [`Fault`]; the lifecycle transition then commits nothing.
    pub(crate) fn fire_lifecycle(
        &self,
        slot: Slot,
        lifecycle: node::Lifecycle,
        firing: LifecycleFiring<'_>,
        state: &BTreeMap<String, Value>,
        instance_values: &BTreeMap<String, Value>,
    ) -> Result<TransactionOutcome, Fault> {
        let activation = node::HandlerActivation::Lifecycle(lifecycle);
        let trigger = node::Trigger::Lifecycle(lifecycle);
        self.fire_activation(
            slot,
            firing.instance,
            &activation,
            &trigger,
            firing.cause,
            Some(firing.cause.meta().sys().id()),
            firing.parent_instance,
            vec![],
            state,
            instance_values,
            &BTreeMap::new(),
            &[],
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn fire_activation(
        &self,
        slot: Slot,
        instance: &InstanceRef,
        activation: &node::HandlerActivation,
        trigger: &node::Trigger,
        input: &Envelope,
        cause: Option<envelope::Id>,
        parent_instance: Option<&InstanceRef>,
        consumed: Vec<ConsumedPosition>,
        state: &BTreeMap<String, Value>,
        instance_values: &BTreeMap<String, Value>,
        extra_bindings: &BTreeMap<String, expr::EvalValue>,
        named_inputs: &[Delivery],
    ) -> Result<TransactionOutcome, Fault> {
        // The validated plan proves its own references, not the caller's
        // slot — verified, refused typed.
        let Some(node) = self.plan.nodes.get(slot.index()) else {
            return Err(Fault::UnknownSlot(slot));
        };
        let handler = self
            .plan
            .handlers
            .iter()
            .find(|handler| handler.node == slot && &handler.activation == activation);
        let Some(handler) = handler else {
            if matches!(
                activation,
                node::HandlerActivation::Port(_) | node::HandlerActivation::Join { .. }
            ) {
                return Err(Fault::MissingHandler);
            }
            return Ok(TransactionOutcome {
                record: TransactionRecord {
                    owner_path: crate::plan::OwnerPath::from(&node.path),
                    instance: instance.clone(),
                    parent_instance: parent_instance.cloned(),
                    close_scope: None,
                    exchange_exit: None,
                    trigger: trigger.clone(),
                    cause,
                    consumed,
                    join_inputs: vec![],
                    state_delta: vec![],
                    emissions: vec![],
                    routes: vec![],
                    timers: vec![],
                },
            });
        };
        // Scratch isolation: the handler runs against copies; nothing
        // below touches the caller's state.
        let mut scratch = state.clone();
        let mut lets: BTreeMap<String, expr::EvalValue> = BTreeMap::new();
        let mut buffer = EmissionBuffer::new();
        let base = |scratch: &BTreeMap<String, Value>,
                    lets: &BTreeMap<String, expr::EvalValue>|
         -> BTreeMap<String, expr::EvalValue> {
            let mut map: BTreeMap<String, expr::EvalValue> = scratch
                .iter()
                .map(|(name, value)| (name.clone(), expr::EvalValue::from(value.clone())))
                .collect();
            for (name, value) in instance_values {
                map.insert(name.clone(), expr::EvalValue::from(value.clone()));
            }
            for (name, value) in extra_bindings {
                map.insert(name.clone(), value.clone());
            }
            for (name, value) in lets {
                map.insert(name.clone(), value.clone());
            }
            map.insert(
                String::from("meta"),
                expr::EvalValue::from(meta_value(input.meta())),
            );
            map
        };
        // `data` materializes on the EVALUATOR'S demand, cached for
        // the whole transaction: a guard on `meta`, a state write,
        // an identity emission, or a short-circuited branch never
        // forces an opaque payload to decode (envelope.md §5:
        // opacity is a property, not an ambush).
        let payloads = PayloadSlots::new(input, named_inputs);
        if let Some(guard) = &handler.guard {
            let verdict = resolve_payload_with_slots(
                eval_with_slots(guard, &base(&scratch, &lets), &payloads, &self.plan.assets)?,
                &payloads,
            )?;
            match verdict {
                Value::Bool(true) => {}
                Value::Bool(false) => {
                    // Filtered: consumed with no effect.
                    return Ok(TransactionOutcome {
                        record: TransactionRecord {
                            owner_path: crate::plan::OwnerPath::from(&node.path),
                            instance: instance.clone(),
                            parent_instance: parent_instance.cloned(),
                            close_scope: None,
                            exchange_exit: None,
                            trigger: trigger.clone(),
                            cause,
                            consumed,
                            join_inputs: vec![],
                            state_delta: vec![],
                            emissions: vec![],
                            routes: vec![],
                            timers: vec![],
                        },
                    });
                }
                other => {
                    return Err(Fault::Eval(expr::Error {
                        kind: expr::ErrorKind::Evaluation,
                        message: format!("the guard evaluated to {other:?}"),
                    }));
                }
            }
        }
        let mut close_requested = false;
        for step in &handler.body {
            match step {
                Step::Let { name, expr } => {
                    let value = eval_with_slots(
                        expr,
                        &base(&scratch, &lets),
                        &payloads,
                        &self.plan.assets,
                    )?;
                    lets.insert(name.clone(), value);
                }
                Step::SetVar { name, expr } => {
                    // A state write consumes the value — the store
                    // outlives the transaction, so the witness
                    // resolves here.
                    let value = resolve_payload_with_slots(
                        eval_with_slots(
                            expr,
                            &base(&scratch, &lets),
                            &payloads,
                            &self.plan.assets,
                        )?,
                        &payloads,
                    )?;
                    value.validate().map_err(Fault::InvalidValue)?;
                    // A state cell holds its DECLARED type at every
                    // write, not just at the initializer: a dynamic
                    // value of the wrong shape — or an int leaving
                    // the 32-bit range — is a typed error, never a
                    // silent coercion.
                    if let Some(cell) = node.vars.iter().find(|cell| &cell.name == name)
                        && let Some(violation) = cell.ty.violation(name, &value)
                    {
                        return Err(Fault::Eval(expr::Error {
                            kind: expr::ErrorKind::Evaluation,
                            message: alloc::string::ToString::to_string(&violation),
                        }));
                    }
                    scratch.insert(name.clone(), value);
                }
                Step::Emit { port, value } => {
                    let built = emitted_content(
                        value,
                        input,
                        &base(&scratch, &lets),
                        &payloads,
                        self.plan,
                    )?;
                    if let Some(built) = built {
                        let index = buffer.next_index(port);
                        let id = ids::emission_id(
                            &node.path,
                            trigger,
                            input.meta().sys().id(),
                            port,
                            index,
                        );
                        let sys = MetaSys::emission(id, input.meta().sys(), self.graph_version.0);
                        // The content-type hint: an explicit write wins; a
                        // carried payload keeps that source's hint; rebuilt
                        // content carries none.
                        // Every pin the content must cross — the
                        // emitting port's own and each fan-out
                        // target's — is judged HERE, before the
                        // commit: the same judge as a state cell, so
                        // runtime arithmetic cannot carry an
                        // out-of-range int (or any wrong shape) into
                        // a pin, pass-through included. Encoded
                        // bytes are the codecs' door.
                        let from = crate::plan::EndpointRef {
                            node: Some(slot),
                            port: port.clone(),
                        };
                        let data = match built.data {
                            BuiltData::Carried(data) => {
                                if let Data::Materialized(current) = &data {
                                    crossing_pins_hold(self.plan, &from, current)
                                        .map_err(PinViolation::into_fault)?;
                                }
                                data
                            }
                            BuiltData::Value(value) => {
                                crossing_pins_hold(self.plan, &from, &value)
                                    .map_err(PinViolation::into_fault)?;
                                Data::materialized(value)
                            }
                        };
                        // The writable zones travel whole — the
                        // builder's overlays are already folded in.
                        let meta = Meta::new(sys).with_app(built.app).with_ext(built.ext);
                        buffer.push(Emission {
                            port: port.clone(),
                            index,
                            envelope: Envelope::new(meta, data),
                        });
                    }
                }
                Step::Stop => close_requested = true,
            }
        }
        // The delta is what actually changed.
        let state_delta: Vec<(String, Value)> = scratch
            .iter()
            .filter(|(name, value)| state.get(*name) != Some(value))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        let emissions = buffer.into_emissions();
        let timers = self.armed_budgets(slot, &emissions);
        Ok(TransactionOutcome {
            record: TransactionRecord {
                owner_path: crate::plan::OwnerPath::from(&node.path),
                instance: instance.clone(),
                parent_instance: parent_instance.cloned(),
                close_scope: close_requested.then(|| instance.clone()),
                exchange_exit: None,
                trigger: trigger.clone(),
                cause,
                consumed,
                join_inputs: vec![],
                state_delta,
                emissions,
                routes: vec![],
                timers,
            },
        })
    }

    /// The dated obligations this record's emissions arm: a delivery
    /// reaching a connector with a declared `timeout` starts its
    /// budget AT DELIVERY — queue time counts (execution-model §8).
    /// Statically derivable from the plan, so the record can seal
    /// what the machine then arms, identically.
    fn armed_budgets(&self, slot: Slot, emissions: &[Emission]) -> Vec<(u8, envelope::Id)> {
        let mut timers = Vec::new();
        for emission in emissions {
            for edge in self
                .plan
                .edges
                .iter()
                .filter(|edge| edge.from.node == Some(slot) && edge.from.port == emission.port)
            {
                let Some(to) = &edge.to else { continue };
                let Some(target) = to.node else { continue };
                let budget_kind = self.plan.nodes[target.index()]
                    .connector()
                    .and_then(|spec| {
                        if spec.attribute("timeout").is_some() {
                            Some(crate::journal::timer_kind::CALL_TIMEOUT)
                        } else if spec.attribute("handshake_timeout").is_some() {
                            Some(crate::journal::timer_kind::SESSION_HANDSHAKE)
                        } else {
                            None
                        }
                    });
                if let Some(kind) = budget_kind {
                    timers.push((
                        kind,
                        ids::delivery_ref(
                            &self.plan.edge_text(edge),
                            emission.envelope.meta().sys().id(),
                        ),
                    ));
                }
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
                if join_within.is_some() {
                    timers.push((
                        crate::journal::timer_kind::JOIN_WITHIN,
                        ids::delivery_ref(
                            &self.plan.edge_text(edge),
                            emission.envelope.meta().sys().id(),
                        ),
                    ));
                }
            }
        }
        timers.sort();
        timers
    }

    /// The connector's completion transaction for a settled call: every
    /// ordered draft, including the terminal `exit`, becomes an ordinary
    /// emission in one atomic record. The dispatcher correlates; only this
    /// VM boundary mints envelopes and identities.
    ///
    /// # Errors
    ///
    /// [`Fault::UnknownSlot`] when the caller supplies a slot outside the
    /// validated plan. No handler lookup is involved: a connector completion
    /// is already the settled result of one delivered request.
    pub fn connector_completion(
        &self,
        slot: Slot,
        instance: &InstanceRef,
        delivery: &Delivery,
        trigger: &node::Trigger,
        completion: &Completion,
    ) -> Result<TransactionOutcome, Fault> {
        let Some(planned) = self.plan.nodes.get(slot.index()) else {
            return Err(Fault::UnknownSlot(slot));
        };
        let connector_path = &planned.path;
        let emissions =
            self.completion_emissions(connector_path, &delivery.envelope, trigger, completion);
        Ok(TransactionOutcome {
            record: TransactionRecord {
                owner_path: crate::plan::OwnerPath::from(connector_path),
                instance: instance.clone(),
                parent_instance: None,
                close_scope: None,
                exchange_exit: None,
                trigger: trigger.clone(),
                cause: None,
                consumed: vec![ConsumedPosition {
                    edge: delivery.edge.clone(),
                    source: delivery.source_instance.clone(),
                    seq: delivery.seq,
                }],
                join_inputs: vec![],
                state_delta: vec![],
                timers: vec![],
                emissions,
                routes: vec![],
            },
        })
    }

    /// Builds the terminal Connector transaction of one admitted inbound
    /// Call. The root opened the obligation, so no mailbox cursor is consumed;
    /// the transaction instead closes the Call reference carried by INBOUND.
    ///
    /// # Errors
    ///
    /// [`Fault::UnknownSlot`] when `slot` is outside the validated plan.
    pub fn inbound_call_completion(
        &self,
        slot: Slot,
        instance: &InstanceRef,
        request: &Envelope,
        call_ref: InboundCallRef,
        completion: &Completion,
    ) -> Result<TransactionOutcome, Fault> {
        let Some(planned) = self.plan.nodes.get(slot.index()) else {
            return Err(Fault::UnknownSlot(slot));
        };
        let trigger = node::Trigger::Lifecycle(node::Lifecycle::Reply);
        let emissions = self.completion_emissions(&planned.path, request, &trigger, completion);
        Ok(TransactionOutcome {
            record: TransactionRecord {
                owner_path: crate::plan::OwnerPath::from(&planned.path),
                instance: instance.clone(),
                parent_instance: None,
                close_scope: None,
                exchange_exit: Some(Box::new(crate::journal::InboundExchangeExitRecord::Call(
                    call_ref,
                ))),
                trigger,
                cause: Some(request.meta().sys().id()),
                consumed: vec![],
                join_inputs: vec![],
                state_delta: vec![],
                timers: vec![],
                emissions,
                routes: vec![],
            },
        })
    }

    /// Builds the terminal transaction of an inbound Call whose response was
    /// produced by the graph. Unlike fixed acknowledgement, this form closes
    /// the Call and consumes the real response delivery atomically; it still
    /// creates no OUTBOUND record or synthetic cursor.
    ///
    /// # Errors
    ///
    /// [`Fault::UnknownSlot`] when `slot` is outside the validated plan.
    pub fn inbound_call_response_completion(
        &self,
        slot: Slot,
        instance: &InstanceRef,
        request: &Envelope,
        response: &Delivery,
        call_ref: InboundCallRef,
        completion: &Completion,
    ) -> Result<TransactionOutcome, Fault> {
        let mut outcome =
            self.inbound_call_completion(slot, instance, request, call_ref, completion)?;
        outcome.record.consumed.push(ConsumedPosition {
            edge: response.edge.clone(),
            source: response.source_instance.clone(),
            seq: response.seq,
        });
        Ok(outcome)
    }

    /// Builds the sole terminal transaction of one committed Session.
    ///
    /// The runtime, not the Driver, projects the opaque logical reference on
    /// the terminal envelope. The same transaction closes the journal fold,
    /// so a socket identity can neither select nor forge this correlation.
    ///
    /// # Errors
    ///
    /// [`Fault::UnknownSlot`] when `slot` is outside the validated plan, or a
    /// malformed completion contradicts the terminal barrier.
    pub fn session_completion(
        &self,
        slot: Slot,
        instance: &InstanceRef,
        opening: &Envelope,
        session_ref: SessionRef,
        completion: &Completion,
        consumed: &[&Delivery],
    ) -> Result<TransactionOutcome, Fault> {
        let Some(planned) = self.plan.nodes.get(slot.index()) else {
            return Err(Fault::UnknownSlot(slot));
        };
        let trigger = node::Trigger::Lifecycle(node::Lifecycle::Exit);
        let mut outputs = completion.outputs().to_vec();
        for output in &mut outputs {
            output.ext.insert(
                String::from("session_id"),
                MetaValue::Scalar(Scalar::Text(session_ref.to_string())),
            );
        }
        let completion = Completion::try_new(outputs)
            .map_err(|error| Fault::Unrepresentable(error.to_string()))?;
        let emissions = self.completion_emissions(&planned.path, opening, &trigger, &completion);
        Ok(TransactionOutcome {
            record: TransactionRecord {
                owner_path: crate::plan::OwnerPath::from(&planned.path),
                instance: instance.clone(),
                parent_instance: None,
                close_scope: None,
                exchange_exit: Some(Box::new(
                    crate::journal::InboundExchangeExitRecord::Session(session_ref),
                )),
                trigger,
                cause: Some(opening.meta().sys().id()),
                consumed: consumed
                    .iter()
                    .map(|delivery| crate::journal::ConsumedPosition {
                        edge: delivery.edge.clone(),
                        source: delivery.source_instance.clone(),
                        seq: delivery.seq,
                    })
                    .collect(),
                join_inputs: vec![],
                state_delta: vec![],
                timers: vec![],
                emissions,
                routes: vec![],
            },
        })
    }

    fn completion_emissions(
        &self,
        connector_path: &node::Path,
        request: &Envelope,
        trigger: &node::Trigger,
        completion: &Completion,
    ) -> Vec<Emission> {
        let emitting_id = request.meta().sys().id();
        let request_meta = request.meta();
        let request_sys = request_meta.sys();
        let mut next_index: BTreeMap<node::Port, u32> = BTreeMap::new();
        completion
            .outputs()
            .iter()
            .map(|output| {
                let index = next_index.entry(output.port.clone()).or_default();
                let emission_index = *index;
                *index = index.saturating_add(1);
                let id = ids::emission_id(
                    connector_path,
                    trigger,
                    emitting_id,
                    &output.port,
                    emission_index,
                );
                let sys = MetaSys::emission(id, request_sys, self.graph_version.0);
                // Uniform propagation (D-2026-08-10-2): every completion
                // output inherits the REQUEST zones. Each payload came from
                // the world, so stale content_type is struck.
                let mut meta = inherit_zones(Meta::new(sys), request_meta);
                if !output.ext.is_empty() {
                    let mut ext = meta.ext().clone();
                    ext.extend(output.ext.clone());
                    meta = meta.with_ext(ext);
                }
                Emission {
                    port: output.port.clone(),
                    index: emission_index,
                    envelope: Envelope::new(meta, output.data.clone()),
                }
            })
            .collect()
    }

    /// The handler registered for an activation pattern, if any — the
    /// scheduler asks before consuming a delivery.
    #[must_use]
    pub fn handler_for(
        &self,
        slot: Slot,
        activation: &node::HandlerActivation,
    ) -> Option<&'plan Handler> {
        self.plan
            .handlers
            .iter()
            .find(|handler| handler.node == slot && handler.activation == *activation)
    }
}

/// What one emission carries out of its builder. A carried payload keeps its
/// original representation (encoded bytes included) and content-type hint,
/// per the "no structural read, no transcode" rule.
pub(crate) struct Built {
    /// The built payload.
    pub(crate) data: BuiltData,
    /// The outgoing application zone.
    pub(crate) app: envelope::Namespace,
    /// The outgoing external-context zone — `content_type` already
    /// derived from the payload decision, explicit writes folded.
    pub(crate) ext: envelope::Namespace,
}

/// A builder's payload verdict.
pub(crate) enum BuiltData {
    /// One input's physical representation ships as-is.
    Carried(Data),
    /// The payload was (re)built as one materialized core value.
    Value(Value),
}

/// Folds the input envelope's zones onto a fresh emission meta —
/// the uniform-propagation rule (D-2026-08-10-2) for envelopes the
/// MACHINE mints (connector completions and operator faults): `app` carries
/// whole, `ext` carries minus `content_type` (the minted payload is
/// always fresh). Builder emissions spell the same rule through
/// [`Built`].
fn inherit_zones(meta: Meta, input: &Meta) -> Meta {
    let mut ext = input.ext().clone();
    ext.remove("content_type");
    meta.with_app(input.app().clone()).with_ext(ext)
}

fn emitted_content(
    emission: &PlanEmission,
    input: &Envelope,
    base: &BTreeMap<String, expr::EvalValue>,
    slots: &PayloadSlots<'_>,
    plan: &crate::plan::ExecutionPlan,
) -> Result<Option<Built>, Fault> {
    let assets: &[crate::plan::AssetBlob] = &plan.assets;
    // The atomic order of D-2026-08-10-4: copy the zones, decide
    // the payload, DERIVE `content_type` from that decision (a kept
    // witness keeps it, fresh data strikes it), then let explicit
    // writes win.
    let fresh = |data: Value| {
        let mut ext = input.meta().ext().clone();
        ext.remove("content_type");
        Built {
            data: BuiltData::Value(data),
            app: input.meta().app().clone(),
            ext,
        }
    };
    let carried = |reference: &expr::PayloadRef| -> Result<Built, Fault> {
        let source = slots.source(reference).ok_or_else(|| {
            Fault::Eval(expr::Error {
                kind: expr::ErrorKind::Evaluation,
                message: format!("payload {reference:?} is not bound"),
            })
        })?;
        let ext = match reference {
            // The ordinary single-input path already has exactly the source
            // metadata. Preserve it without a remove/reinsert cycle on every
            // identity relay.
            expr::PayloadRef::Input => input.meta().ext().clone(),
            expr::PayloadRef::Named(_) => {
                let mut ext = input.meta().ext().clone();
                ext.remove("content_type");
                if let Some(content_type) =
                    source.meta.and_then(|meta| meta.ext().get("content_type"))
                {
                    ext.insert(String::from("content_type"), content_type.clone());
                }
                ext
            }
        };
        Ok(Built {
            data: BuiltData::Carried(source.data.data.clone()),
            app: input.meta().app().clone(),
            ext,
        })
    };
    match emission {
        PlanEmission::Nothing => Ok(None),
        PlanEmission::Identity => Ok(Some(carried(&expr::PayloadRef::Input)?)),
        PlanEmission::Data(expr) => {
            // No spelling-dependent special case, in EITHER
            // direction: the evaluator preserves the struct and
            // laziness witnesses. A whole payload
            // arriving uninspected — `out = data`, `out = copy`, the
            // taken arm of a ternary — ships the REPRESENTATION,
            // content type included; anything else is FRESH data and
            // strikes it (D-2026-08-10-4).
            // `data` is the dominant relay form. Its canonical tree already
            // denotes the input witness, so avoid entering the general
            // evaluator unless a (defensively supported) local binding
            // shadows that name. This preserves the physical representation
            // and keeps the ordinary embedded relay inside its measured
            // budget even though joins add independently lazy witnesses.
            let direct_input = matches!(
                &expr.root,
                expr::Expr::Path(path)
                    if path.len() == 1 && path[0] == "data" && !base.contains_key("data")
            );
            let value = if direct_input {
                expr::EvalValue::Payload(expr::PayloadRef::Input)
            } else {
                eval_with_slots(expr, base, slots, assets)?
            };
            if let expr::EvalValue::Payload(reference) = value {
                return Ok(Some(carried(&reference)?));
            }
            let value = resolve_payload_with_slots(value, slots)?;
            Ok(Some(fresh(value)))
        }
        PlanEmission::Patch(writes) | PlanEmission::Replace(writes) => {
            let replace = matches!(emission, PlanEmission::Replace(_));
            // The whole-data CARRY: a builder whose only payload
            // write is the bare witness (`data = data`,
            // `data = a.data`) keeps the representation — the
            // decision follows the VALUE, not the spelling.
            let data_writes: alloc::vec::Vec<usize> = writes
                .iter()
                .enumerate()
                .filter(|(_, write)| matches!(&write.target, envelope::WriteTarget::Data(_)))
                .map(|(index, _)| index)
                .collect();
            // The decision follows the FINAL fold result, not the
            // write count: a whole-data write replaces everything
            // before it, so the LAST data write decides — if it is
            // whole-data and its value is the bare witness, every
            // earlier data write is dead and the representation
            // carries (`data = "temp"` then `data = data` keeps).
            let carry = if let Some(last) = data_writes.last() {
                let write = &writes[*last];
                if matches!(&write.target, envelope::WriteTarget::Data(path) if path.is_empty()) {
                    match eval_with_slots(&write.expr, base, slots, assets)? {
                        expr::EvalValue::Payload(reference) => Some(reference),
                        _ => None,
                    }
                } else {
                    None
                }
            } else {
                None
            };
            let mut built = if let Some(reference) = &carry {
                carried(reference)?
            } else if replace || !data_writes.is_empty() {
                fresh(envelope::Value::Struct(Vec::new()))
            } else {
                carried(&expr::PayloadRef::Input)?
            };
            let mut data: Option<envelope::Value> = if replace && carry.is_none() {
                // A fresh STRUCT — the physical form of a braced
                // record (D-2026-08-09-1).
                Some(envelope::Value::Struct(Vec::new()))
            } else {
                None
            };
            for write in writes {
                if carry.is_some() && matches!(&write.target, envelope::WriteTarget::Data(_)) {
                    // The carried representation IS the payload.
                    continue;
                }
                let value = resolve_payload_with_slots(
                    eval_with_slots(&write.expr, base, slots, assets)?,
                    slots,
                )?;
                match &write.target {
                    envelope::WriteTarget::Data(path) => {
                        // The fold base is a DIRECT demand — a
                        // data-path patch needs the current value —
                        // through the same cached slot; a meta-only
                        // patch never touches the payload bytes.
                        let current = match &mut data {
                            Some(current) => current,
                            None => data.insert(match input.data() {
                                Data::Materialized(value) => value.clone(),
                                _ => slots.materialize(&expr::PayloadRef::Input)?,
                            }),
                        };
                        apply_struct_write(current, path, value);
                    }
                    envelope::WriteTarget::App(field) => {
                        // The travelling schema judges the VALUE — a
                        // A `variant` expression cannot smuggle text into
                        // a sealed `int` leaf (D-2026-08-10-1).
                        let converted = to_meta_value(&value)?;
                        if let Some((_, ty)) =
                            plan.app_schema.iter().find(|(known, _)| known == field)
                            && !meta_conforms(&converted, ty)
                        {
                            return Err(Fault::Unrepresentable(format!(
                                "`meta.app.{field}` is sealed as {ty:?}; this value is not"
                            )));
                        }
                        built.app.insert(field.clone(), converted);
                    }
                    envelope::WriteTarget::Ext(field) => {
                        // Explicit writes come LAST in the atomic
                        // order — they win over the derivation. The
                        // standard fields keep their implicit `text` contract
                        // even through a `variant` expression; sheet fields
                        // answer to their declared types.
                        let converted = to_meta_value(&value)?;
                        let ty = crate::plan::standard_ext_field_type(field).or_else(|| {
                            plan.ext_schema
                                .iter()
                                .find(|(known, _)| known == field)
                                .map(|(_, ty)| ty.clone())
                        });
                        if let Some(ty) = ty
                            && !meta_conforms(&converted, &ty)
                        {
                            return Err(Fault::Unrepresentable(format!(
                                "`meta.ext.{field}` is declared {ty:?}; this value is not"
                            )));
                        }
                        built.ext.insert(field.clone(), converted);
                    }
                }
            }
            if let Some(value) = data {
                built.data = BuiltData::Value(value);
            }
            Ok(Some(built))
        }
    }
}

/// One refused crossing: WHERE the pin refused — the emitting end
/// itself or a precise fan-out destination — and the judgement.
/// Structure survives to the public APIs; callers that speak Fault
/// flatten it themselves.
#[derive(Debug, Clone, PartialEq)]
pub struct PinViolation {
    /// The endpoint whose pin refused.
    pub at: crate::plan::EndpointRef,
    /// The conformance judgement — SEMANTIC: expected type, found
    /// kind, and the reason; text is `Display`'s business.
    pub violation: Box<ConformanceViolation>,
}

impl PinViolation {
    /// The flattening callers on the Fault path use.
    #[must_use]
    pub fn into_fault(self) -> Fault {
        Fault::Eval(expr::Error {
            kind: expr::ErrorKind::Evaluation,
            message: alloc::string::ToString::to_string(&self.violation),
        })
    }
}

/// Judges materialized content against every pin it must cross —
/// the emitting end's own pin and each fan-out target's — BEFORE any
/// commit: routing is post-commit and must never fail. Encoded bytes
/// never reach this judge (the codecs' door); an explicit `variant`
/// destination admits any valid materialized value.
pub(crate) fn crossing_pins_hold(
    plan: &crate::plan::ValidatedExecutionPlan,
    from: &crate::plan::EndpointRef,
    value: &Value,
) -> Result<(), PinViolation> {
    let from_type = plan.resolved_ports().endpoint_type(from);
    debug_assert!(
        from_type.is_some(),
        "a validated emitting endpoint must have a resolved type: {from:?}"
    );
    if let Some(pinned) = from_type
        && let Some(violation) = pinned.violation(from.port.name(), value)
    {
        return Err(PinViolation {
            at: from.clone(),
            violation: Box::new(violation),
        });
    }
    for edge in plan.edges.iter().filter(|edge| edge.from == *from) {
        let Some(to) = &edge.to else { continue };
        let pinned = plan.resolved_ports().endpoint_type(to);
        debug_assert!(
            pinned.is_some(),
            "a validated fan-out endpoint must have a resolved type: {to:?}"
        );
        if let Some(pinned) = pinned
            && let Some(violation) = pinned.violation(to.port.name(), value)
        {
            return Err(PinViolation {
                at: to.clone(),
                violation: Box::new(violation),
            });
        }
    }
    Ok(())
}

/// Applies one struct-building write along its path — a braced
/// record builds a STRUCT (D-2026-08-09-1), so a
/// missing or non-composite level becomes a `Struct` whose fields
/// keep write order; an existing `Struct` or `Map` level keeps its
/// own kind. The empty path replaces the whole value.
pub fn apply_struct_write(data: &mut envelope::Value, path: &[String], leaf: envelope::Value) {
    let Some((first, rest)) = path.split_first() else {
        *data = leaf;
        return;
    };
    match data {
        envelope::Value::Struct(fields) => {
            if let Some((_, existing)) = fields.iter_mut().find(|(name, _)| name == first) {
                apply_struct_write(existing, rest, leaf);
            } else {
                let mut fresh = envelope::Value::Struct(Vec::new());
                apply_struct_write(&mut fresh, rest, leaf);
                fields.push((first.clone(), fresh));
            }
        }
        envelope::Value::Map(entries) => {
            let entry = entries
                .entry(first.clone())
                .or_insert_with(|| envelope::Value::Struct(Vec::new()));
            apply_struct_write(entry, rest, leaf);
        }
        _ => {
            *data = envelope::Value::Struct(Vec::new());
            apply_struct_write(data, path, leaf);
        }
    }
}

/// The `data` binding, materialized on the EVALUATOR'S first demand
/// and cached for the rest of the transaction. The demand is the
/// evaluation itself — a short-circuited branch (`false && data.x`,
/// the untaken arm of a ternary, a statement never reached) never
/// asks, so an opaque payload survives every path that does not
/// structurally read it (envelope.md §5.1: no structural access, no
/// transcode — ever). A static pre-scan of the tree cannot express
/// that; only the evaluator knows which paths ran.
#[derive(Debug)]
pub struct DataSlot<'a> {
    data: &'a Data,
    cache: core::cell::RefCell<Option<Value>>,
    fault: core::cell::RefCell<Option<Fault>>,
}

impl<'a> DataSlot<'a> {
    /// Wraps the input's data; nothing is read yet.
    #[must_use]
    pub fn new(data: &'a Data) -> Self {
        Self {
            data,
            cache: core::cell::RefCell::new(None),
            fault: core::cell::RefCell::new(None),
        }
    }

    /// The direct demand — a structural fold that NEEDS the value
    /// (a data-path patch base). Cached like the evaluator's own
    /// reads.
    ///
    /// # Errors
    /// The materialization fault, if the payload cannot decode.
    pub fn materialize(&self) -> Result<Value, Fault> {
        // A materialized payload is already the cache. Cloning it into the
        // transaction-local cell before cloning it back out only duplicates
        // allocations on the ordinary typed relay; the cell is needed solely
        // to memoize an encoded decode.
        if let Data::Materialized(value) = self.data {
            return Ok(value.clone());
        }
        if let Some(value) = self.cache.borrow().as_ref() {
            return Ok(value.clone());
        }
        let value = data_value(self.data)?;
        *self.cache.borrow_mut() = Some(value.clone());
        Ok(value)
    }

    /// The evaluator's demand: a failed materialization parks its
    /// fault for [`eval_lazy`] to surface and answers "unbound".
    fn demand(&self) -> Option<expr::EvalValue> {
        match self.materialize() {
            Ok(value) => Some(expr::EvalValue::from(value)),
            Err(fault) => {
                *self.fault.borrow_mut() = Some(fault);
                None
            }
        }
    }

    /// Takes a parked materialization fault, if one happened.
    pub fn take_fault(&self) -> Option<Fault> {
        self.fault.borrow_mut().take()
    }
}

#[derive(Debug)]
struct PayloadSource<'a> {
    data: DataSlot<'a>,
    meta: Option<&'a Meta>,
}

/// All lazy payloads visible during one transaction. An ordinary handler has
/// only [`expr::PayloadRef::Input`]; a join adds one independently cached slot
/// per declared input port. This is the runtime half of the rule that reading
/// one join leg must not decode every other leg.
#[derive(Debug)]
struct PayloadSlots<'a> {
    input: PayloadSource<'a>,
    named: BTreeMap<String, PayloadSource<'a>>,
}

impl<'a> PayloadSlots<'a> {
    fn new(input: &'a Envelope, named: &'a [Delivery]) -> Self {
        let named = named
            .iter()
            .map(|delivery| {
                (
                    delivery.port.name().to_string(),
                    PayloadSource {
                        data: DataSlot::new(delivery.envelope.data()),
                        meta: Some(delivery.envelope.meta()),
                    },
                )
            })
            .collect();
        Self {
            input: PayloadSource {
                data: DataSlot::new(input.data()),
                meta: Some(input.meta()),
            },
            named,
        }
    }

    fn source(&self, reference: &expr::PayloadRef) -> Option<&PayloadSource<'a>> {
        match reference {
            expr::PayloadRef::Input => Some(&self.input),
            expr::PayloadRef::Named(name) => self.named.get(name),
        }
    }

    fn materialize(&self, reference: &expr::PayloadRef) -> Result<Value, Fault> {
        self.source(reference)
            .ok_or_else(|| {
                Fault::Eval(expr::Error {
                    kind: expr::ErrorKind::Evaluation,
                    message: format!("payload {reference:?} is not bound"),
                })
            })?
            .data
            .materialize()
    }

    fn demand(&self, reference: &expr::PayloadRef) -> Option<expr::EvalValue> {
        self.source(reference)?.data.demand()
    }

    fn take_fault(&self) -> Option<Fault> {
        self.input.data.take_fault().or_else(|| {
            self.named
                .values()
                .find_map(|source| source.data.take_fault())
        })
    }
}

/// Bindings backed by a base map plus the lazy `data` slot. Exact
/// name only: dotted lookups (`data.x`) fall through to the root,
/// which is where the demand lands.
#[derive(Debug)]
pub struct LazyBindings<'a> {
    /// Everything already materialized: state scratch, locals, meta.
    pub base: &'a BTreeMap<String, expr::EvalValue>,
    /// The on-demand payload.
    pub data: &'a DataSlot<'a>,
    /// The plan's build-time asset table (D-2026-08-10-5).
    pub assets: &'a [crate::plan::AssetBlob],
}

impl expr::Bindings for LazyBindings<'_> {
    fn get(&self, name: &str) -> Option<expr::EvalValue> {
        if let Some(value) = self.base.get(name) {
            return Some(value.clone());
        }
        // Resolving the NAME is not yet inspecting the structure:
        // the witness carries; `Bindings::payload` materializes.
        (name == "data").then_some(expr::EvalValue::Payload(expr::PayloadRef::Input))
    }

    fn payload(&self, reference: &expr::PayloadRef) -> Result<expr::EvalValue, expr::Error> {
        if !matches!(reference, expr::PayloadRef::Input) {
            return Err(expr::Error {
                kind: expr::ErrorKind::Evaluation,
                message: format!("payload {reference:?} is not bound"),
            });
        }
        self.data.demand().ok_or_else(|| expr::Error {
            kind: expr::ErrorKind::Evaluation,
            message: String::from("the payload could not materialize"),
        })
    }

    fn asset(&self, index: u32) -> Result<expr::EvalValue, expr::Error> {
        self.assets
            .get(index as usize)
            .map(|blob| expr::EvalValue::Bytes(blob.bytes.clone()))
            .ok_or_else(|| expr::Error {
                kind: expr::ErrorKind::Evaluation,
                message: format!("asset {index} is outside the validated table"),
            })
    }
}

#[derive(Debug)]
struct MultiLazyBindings<'a> {
    base: &'a BTreeMap<String, expr::EvalValue>,
    payloads: &'a PayloadSlots<'a>,
    assets: &'a [crate::plan::AssetBlob],
}

impl expr::Bindings for MultiLazyBindings<'_> {
    fn get(&self, name: &str) -> Option<expr::EvalValue> {
        self.base.get(name).cloned().or_else(|| {
            (name == "data").then_some(expr::EvalValue::Payload(expr::PayloadRef::Input))
        })
    }

    fn payload(&self, reference: &expr::PayloadRef) -> Result<expr::EvalValue, expr::Error> {
        self.payloads.demand(reference).ok_or_else(|| expr::Error {
            kind: expr::ErrorKind::Evaluation,
            message: format!("payload {reference:?} could not materialize"),
        })
    }

    fn asset(&self, index: u32) -> Result<expr::EvalValue, expr::Error> {
        self.assets
            .get(index as usize)
            .map(|blob| expr::EvalValue::Bytes(blob.bytes.clone()))
            .ok_or_else(|| expr::Error {
                kind: expr::ErrorKind::Evaluation,
                message: format!("asset {index} is outside the validated table"),
            })
    }
}

/// Resolves every payload witness in a value — the host-side twin of
/// the evaluator's structural demand, for the points where a value
/// LEAVES the evaluation: a state write, an emission conversion, a
/// mock reply. Composites resolve deep.
///
/// # Errors
/// The materialization fault, if the payload cannot decode.
fn resolve_payload_with_slots(
    value: expr::EvalValue,
    slots: &PayloadSlots<'_>,
) -> Result<Value, Fault> {
    Ok(match value {
        expr::EvalValue::Payload(reference) => slots.materialize(&reference)?,
        expr::EvalValue::List(items) => Value::Array(
            items
                .into_iter()
                .map(|item| resolve_payload_with_slots(item, slots))
                .collect::<Result<_, _>>()?,
        ),
        expr::EvalValue::Map(map) => {
            let mut deep = BTreeMap::new();
            for (key, value) in map {
                deep.insert(key, resolve_payload_with_slots(value, slots)?);
            }
            Value::Map(deep)
        }
        expr::EvalValue::KeyedMap(entries) => Value::KeyedMap(
            entries
                .into_iter()
                .map(|(key, value)| {
                    Ok((
                        resolve_payload_with_slots(key, slots)?,
                        resolve_payload_with_slots(value, slots)?,
                    ))
                })
                .collect::<Result<_, Fault>>()?,
        ),
        expr::EvalValue::Struct(fields) => Value::Struct(
            fields
                .into_iter()
                .map(|(name, value)| Ok((name, resolve_payload_with_slots(value, slots)?)))
                .collect::<Result<_, _>>()?,
        ),
        expr::EvalValue::StructuralUnion { branch, value } => Value::StructuralUnion {
            branch,
            value: Box::new(resolve_payload_with_slots(*value, slots)?),
        },
        expr::EvalValue::TaggedUnion { case, payload } => {
            Value::TaggedUnion(Box::new(crate::value::TaggedUnionValue {
                case,
                payload: payload
                    .into_iter()
                    .map(|(name, value)| Ok((name, resolve_payload_with_slots(value, slots)?)))
                    .collect::<Result<_, Fault>>()?,
            }))
        }
        other => other.into_value().map_err(Fault::Eval)?,
    })
}

/// Evaluates against the lazy binding. A materialization fault takes
/// precedence over whatever the evaluator made of the missing
/// binding: a demanded decode that failed is the transaction's
/// fault, even if the expression could have absorbed an unbound
/// name.
///
/// # Errors
/// The materialization fault, or the evaluation's own.
fn eval_with_slots(
    tree: &expr::Tree,
    base: &BTreeMap<String, expr::EvalValue>,
    slots: &PayloadSlots<'_>,
    assets: &[crate::plan::AssetBlob],
) -> Result<expr::EvalValue, Fault> {
    if slots.named.is_empty() {
        return eval_lazy(tree, base, &slots.input.data, assets);
    }
    let env = MultiLazyBindings {
        base,
        payloads: slots,
        assets,
    };
    let result = expr::eval(tree, &env).map_err(Fault::Eval);
    if let Some(fault) = slots.take_fault() {
        return Err(fault);
    }
    result
}

/// Resolves the ordinary single-input payload witness in a value. Join
/// execution uses the internal multi-slot twin so named inputs remain lazy.
///
/// # Errors
/// The materialization fault, if the payload cannot decode.
pub fn resolve_payload(value: expr::EvalValue, slot: &DataSlot<'_>) -> Result<Value, Fault> {
    Ok(match value {
        expr::EvalValue::Payload(expr::PayloadRef::Input) => slot.materialize()?,
        expr::EvalValue::Payload(reference) => {
            return Err(Fault::Eval(expr::Error {
                kind: expr::ErrorKind::Evaluation,
                message: format!("payload {reference:?} is not bound"),
            }));
        }
        expr::EvalValue::List(items) => Value::Array(
            items
                .into_iter()
                .map(|item| resolve_payload(item, slot))
                .collect::<Result<_, _>>()?,
        ),
        expr::EvalValue::Map(map) => {
            let mut deep = BTreeMap::new();
            for (key, value) in map {
                deep.insert(key, resolve_payload(value, slot)?);
            }
            Value::Map(deep)
        }
        expr::EvalValue::KeyedMap(entries) => Value::KeyedMap(
            entries
                .into_iter()
                .map(|(key, value)| {
                    Ok((resolve_payload(key, slot)?, resolve_payload(value, slot)?))
                })
                .collect::<Result<_, Fault>>()?,
        ),
        expr::EvalValue::Struct(fields) => Value::Struct(
            fields
                .into_iter()
                .map(|(name, value)| Ok((name, resolve_payload(value, slot)?)))
                .collect::<Result<_, _>>()?,
        ),
        expr::EvalValue::StructuralUnion { branch, value } => Value::StructuralUnion {
            branch,
            value: Box::new(resolve_payload(*value, slot)?),
        },
        expr::EvalValue::TaggedUnion { case, payload } => {
            Value::TaggedUnion(Box::new(crate::value::TaggedUnionValue {
                case,
                payload: payload
                    .into_iter()
                    .map(|(name, value)| Ok((name, resolve_payload(value, slot)?)))
                    .collect::<Result<_, Fault>>()?,
            }))
        }
        other => other.into_value().map_err(Fault::Eval)?,
    })
}

/// Evaluates against one ordinary lazy `data` binding.
///
/// # Errors
/// The materialization fault, or the expression evaluator's own error.
pub fn eval_lazy(
    tree: &expr::Tree,
    base: &BTreeMap<String, expr::EvalValue>,
    slot: &DataSlot<'_>,
    assets: &[crate::plan::AssetBlob],
) -> Result<expr::EvalValue, Fault> {
    let env = LazyBindings {
        base,
        data: slot,
        assets,
    };
    let result = expr::eval(tree, &env).map_err(Fault::Eval);
    if let Some(fault) = slot.take_fault() {
        return Err(fault);
    }
    result
}

/// The handler-facing materialized view of an envelope's data — the bridge a
/// host layer (lab, Drivers, tooling) uses to read what a handler
/// reads.
pub fn data_value(data: &Data) -> Result<Value, Fault> {
    // `Data::materialize_with` is the CACHE AUTHORITY: the decode
    // lands in the envelope's own cell, so an identity re-emission
    // ships the filled cell and the next node never decodes again —
    // once per ENVELOPE, not once per transaction (envelope.md §5.1).
    Ok(data.materialize_with(decode_encoded)?.clone())
}

/// Decodes encoded bytes into the envelope domain — the closure
/// `Data::materialize_with` runs on first structural access.
fn decode_encoded(encoding: envelope::Encoding, bytes: &[u8]) -> Result<envelope::Value, Fault> {
    match encoding {
        envelope::Encoding::Raw => Ok(match core::str::from_utf8(bytes) {
            Ok(text) => envelope::Value::Text(String::from(text)),
            Err(_) => envelope::Value::Bytes(bytes.to_vec()),
        }),
        other => Err(Fault::Opaque(format!(
            "{other} decoding is not in this build"
        ))),
    }
}

/// Builds the one join evaluation context. Writable metadata uses
/// CommonOrAbsent across every contribution; the triggering envelope keeps
/// the runtime spine and payload solely as the causal base. Each declared
/// port also receives a complete named envelope binding.
fn join_context(
    plan: &crate::plan::ExecutionPlan,
    trigger: &Delivery,
    inputs: &[Delivery],
) -> (Envelope, BTreeMap<String, expr::EvalValue>) {
    let common = |zone: fn(&Meta) -> &envelope::Namespace, fields: &[(String, DataType)]| {
        let mut values = envelope::Namespace::new();
        for (field, _) in fields {
            let mut present = inputs
                .iter()
                .filter_map(|delivery| zone(delivery.envelope.meta()).get(field));
            let Some(value) = present.next() else {
                continue;
            };
            if present.all(|candidate| candidate == value) {
                values.insert(field.clone(), value.clone());
            }
        }
        values
    };
    let meta = Meta::new(trigger.envelope.meta().sys().clone())
        .with_app(common(Meta::app, &plan.app_schema))
        // `content_type` is deliberately implicit in the ext vocabulary and
        // excluded from join fusion: a later payload decision derives it.
        .with_ext(common(Meta::ext, &plan.ext_schema));
    let input = Envelope::new(meta, trigger.envelope.data().clone());
    let mut bindings = BTreeMap::new();
    for delivery in inputs {
        let name = delivery.port.name();
        let meta = meta_value(delivery.envelope.meta());
        let payload = expr::EvalValue::Payload(expr::PayloadRef::Named(name.to_string()));
        bindings.insert(format!("{name}.data"), payload.clone());
        bindings.insert(format!("{name}.meta"), expr::EvalValue::from(meta.clone()));
        bindings.insert(
            name.to_string(),
            expr::EvalValue::Struct(alloc::vec![
                (String::from("data"), payload),
                (String::from("meta"), expr::EvalValue::from(meta)),
            ]),
        );
    }
    (input, bindings)
}

/// The handler-facing logical view of an envelope's metadata: the full
/// `meta.sys` spine and the two writable zones, flat dotted keys
/// nested for reading.
pub fn meta_value(meta: &Meta) -> Value {
    let mut app = BTreeMap::new();
    for (field, value) in meta.app() {
        app.insert(field.clone(), from_meta_value(value));
    }
    // The whole five-field spine — a handler reading any `meta.sys`
    // field the spec names must find it, not a hole.
    let mut sys = BTreeMap::new();
    sys.insert(String::from("id"), Value::Text(meta.sys().id().to_string()));
    sys.insert(
        String::from("parent_id"),
        match meta.sys().parent_id() {
            Some(parent) => Value::Text(parent.to_string()),
            None => Value::Null,
        },
    );
    sys.insert(
        String::from("origin_id"),
        Value::Text(meta.sys().origin_id().to_string()),
    );
    sys.insert(
        String::from("role"),
        Value::Text(meta.sys().role().to_string()),
    );
    sys.insert(
        String::from("graph_version"),
        Value::Int(i64::try_from(meta.sys().graph_version()).unwrap_or(i64::MAX)),
    );
    let mut root = BTreeMap::new();
    // Exactly three zones, no alias, no root fields
    // (D-2026-08-10-1): storage is FLAT dotted keys, the projection
    // NESTS by splitting on dots so `meta.app.audit.origin` reads
    // naturally.
    root.insert(String::from("app"), nested_zone(&app));
    root.insert(String::from("ext"), nested_zone_meta(meta.ext()));
    root.insert(String::from("sys"), Value::Map(sys));
    Value::Map(root)
}

/// Nests a zone's flat dotted keys into the Rastok expression map tree the
/// projection exposes — one storage form, nested views derived.
fn nested_from_pairs(pairs: impl Iterator<Item = (String, Value)>) -> Value {
    let mut root: BTreeMap<String, Value> = BTreeMap::new();
    for (key, value) in pairs {
        let segments: alloc::vec::Vec<&str> = key.split('.').collect();
        let mut cursor = &mut root;
        for segment in &segments[..segments.len() - 1] {
            let entry = cursor
                .entry(String::from(*segment))
                .or_insert_with(|| Value::Map(BTreeMap::new()));
            match entry {
                Value::Map(inner) => cursor = inner,
                // Prefix-freedom is a checker invariant; a leaf met
                // on the way is unreachable from validated plans —
                // keep the projection total by shadowing.
                other => {
                    *other = Value::Map(BTreeMap::new());
                    let Value::Map(inner) = other else {
                        unreachable!("just inserted a map")
                    };
                    cursor = inner;
                }
            }
        }
        cursor.insert(String::from(segments[segments.len() - 1]), value);
    }
    Value::Map(root)
}

fn nested_zone(zone: &BTreeMap<String, Value>) -> Value {
    nested_from_pairs(zone.iter().map(|(k, v)| (k.clone(), v.clone())))
}

fn nested_zone_meta(zone: &envelope::Namespace) -> Value {
    nested_from_pairs(zone.iter().map(|(k, v)| (k.clone(), from_meta_value(v))))
}

fn from_scalar(scalar: &envelope::Scalar) -> Value {
    match scalar {
        envelope::Scalar::Bool(value) => Value::Bool(*value),
        envelope::Scalar::Int(value) => Value::Int(*value),
        envelope::Scalar::Double(value) => Value::Double(*value),
        envelope::Scalar::Text(value) => Value::Text(value.clone()),
        envelope::Scalar::Bytes(value) => Value::Bytes(value.clone()),
        envelope::Scalar::Decimal { unscaled, scale } => Value::Decimal {
            unscaled: *unscaled,
            scale: *scale,
        },
    }
}

fn from_meta_value(value: &MetaValue) -> Value {
    match value {
        MetaValue::Scalar(scalar) => from_scalar(scalar),
        MetaValue::Array(items) => Value::Array(items.iter().map(from_scalar).collect()),
        MetaValue::Map(entries) => Value::Map(
            entries
                .iter()
                .map(|(key, values)| {
                    (
                        key.clone(),
                        Value::Array(values.iter().map(from_scalar).collect()),
                    )
                })
                .collect(),
        ),
    }
}

/// The ONE conformance rule between a metadata value and a schema
/// type (D-2026-08-10-1) — used at runtime zone writes, at the VM
/// ingress, and mirrored by the checker's static list. `variant` admits
/// every representable leaf.
#[must_use]
pub fn meta_conforms(value: &MetaValue, ty: &DataType) -> bool {
    let materialized = from_meta_value(value);
    materialized.validate().is_ok() && ty.accepts(&materialized)
}

/// Converts a core value into a metadata leaf — the physical half of
/// [`meta_conforms`]'s rule.
///
/// # Errors
/// A typed refusal when the value has no flat-leaf representation.
pub fn to_meta_value(value: &Value) -> Result<MetaValue, Fault> {
    Ok(match value {
        Value::Bool(v) => MetaValue::Scalar(Scalar::Bool(*v)),
        Value::Int(v) => MetaValue::Scalar(Scalar::Int(*v)),
        Value::Double(v) => MetaValue::Scalar(Scalar::Double(*v)),
        Value::Text(v) => MetaValue::Scalar(Scalar::Text(v.clone())),
        Value::Bytes(v) => MetaValue::Scalar(Scalar::Bytes(v.clone())),
        Value::Decimal { unscaled, scale } => MetaValue::Scalar(Scalar::Decimal {
            unscaled: *unscaled,
            scale: *scale,
        }),
        // `array<scalar>` is spec-admitted meta (envelope.md §0).
        Value::Array(items) => {
            let scalars = items
                .iter()
                .map(|item| match to_meta_value(item)? {
                    MetaValue::Scalar(scalar) => Ok(scalar),
                    MetaValue::Array(_) | MetaValue::Map(_) => Err(Fault::Unrepresentable(
                        String::from("a nested array in a flat meta zone"),
                    )),
                })
                .collect::<Result<alloc::vec::Vec<_>, _>>()?;
            MetaValue::Array(scalars)
        }
        Value::Map(entries) => {
            let entries = entries
                .iter()
                .map(|(key, value)| {
                    let Value::Array(values) = value else {
                        return Err(Fault::Unrepresentable(String::from(
                            "a metadata map value is not an array",
                        )));
                    };
                    let values = values
                        .iter()
                        .map(|value| match to_meta_value(value)? {
                            MetaValue::Scalar(scalar) => Ok(scalar),
                            MetaValue::Array(_) | MetaValue::Map(_) => Err(Fault::Unrepresentable(
                                String::from("a metadata map contains a nested value"),
                            )),
                        })
                        .collect::<Result<alloc::vec::Vec<_>, _>>()?;
                    Ok((key.clone(), values))
                })
                .collect::<Result<alloc::collections::BTreeMap<_, _>, _>>()?;
            MetaValue::Map(entries)
        }
        other => {
            return Err(Fault::Unrepresentable(format!(
                "{other:?} in a flat meta namespace"
            )));
        }
    })
}

#[cfg(test)]
mod metadata_conformance_tests {
    use super::*;

    #[test]
    fn metadata_uses_the_core_value_and_type_authorities() {
        let largest_int = MetaValue::Scalar(Scalar::Int(i64::from(i32::MAX)));
        let oversized_int = MetaValue::Scalar(Scalar::Int(i64::from(i32::MAX) + 1));
        assert!(meta_conforms(&largest_int, &DataType::INT));
        assert!(!meta_conforms(&oversized_int, &DataType::INT));

        let malformed_decimal = MetaValue::Scalar(Scalar::Decimal {
            unscaled: 1,
            scale: crate::datatype::DECIMAL_MAX_SCALE + 1,
        });
        assert!(
            !meta_conforms(&malformed_decimal, &DataType::Variant),
            "variant is dynamic, not an escape from value well-formedness"
        );

        let headers = MetaValue::Map(alloc::collections::BTreeMap::from([(
            String::from("set-cookie"),
            alloc::vec![
                Scalar::Bytes(alloc::vec![0xff]),
                Scalar::Bytes(alloc::vec![0x00]),
            ],
        )]));
        assert!(meta_conforms(&headers, &crate::plan::http_headers_type()));
        assert_eq!(
            to_meta_value(&from_meta_value(&headers)).expect("the closed carrier round-trips"),
            headers,
        );
    }
}

#[cfg(test)]
mod instance_address_tests {
    use super::*;
    use crate::datatype::{DateUnit, EnumType, StructType, TemporalType, TypeField};
    use crate::identity::{InstanceAddress, InstanceFrame};
    use crate::plan::OwnerPath;

    fn address(ty: DataType, value: Value) -> InstanceAddress {
        let owner = OwnerPath::new(["node"]).expect("valid owner");
        InstanceAddress::new(
            &owner,
            &[InstanceFrame::SelectedKey {
                owner: owner.clone(),
                ty,
                value,
            }],
        )
        .expect("the fixture inhabits the key type")
    }

    #[test]
    fn nested_struct_keys_use_one_field_order_independent_identity() {
        let scope = StructType::new(vec![
            TypeField::new("region", DataType::Text),
            TypeField::new("sequence", DataType::UINT),
        ])
        .expect("the nested scope type is closed");
        let ty = DataType::Struct(
            StructType::new(vec![
                TypeField::new("scope", DataType::Struct(scope)),
                TypeField::new("tenant", DataType::Uuid),
            ])
            .expect("the outer key type is closed"),
        );
        let tenant = Value::Uuid([0xA5; 16]);
        let scope = Value::Struct(vec![
            (String::from("sequence"), Value::Uint(7)),
            (String::from("region"), Value::Text(String::from("eu"))),
        ]);
        let first = Value::Struct(vec![
            (String::from("tenant"), tenant.clone()),
            (String::from("scope"), scope.clone()),
        ]);
        let reordered = Value::Struct(vec![
            (String::from("scope"), scope),
            (String::from("tenant"), tenant),
        ]);

        assert_eq!(address(ty.clone(), first), address(ty, reordered));
    }

    #[test]
    fn every_non_text_key_family_uses_the_shared_canonical_identity() {
        let enumeration = DataType::Enum(
            EnumType::new(vec![String::from("pending"), String::from("ready")])
                .expect("the enum fixture is valid"),
        );
        let cases = [
            (DataType::BYTES, Value::Bytes(vec![0, 1, 2])),
            (
                DataType::Temporal(TemporalType::Date(DateUnit::Day)),
                Value::Date(0),
            ),
            (enumeration, Value::Enum(String::from("ready"))),
        ];

        let keys = cases
            .iter()
            .map(|(ty, value)| {
                let first = address(ty.clone(), value.clone());
                let second = address(ty.clone(), value.clone());
                assert_eq!(first, second);
                assert!(!first.as_bytes().is_empty());
                first
            })
            .collect::<Vec<_>>();

        assert_ne!(keys[0], keys[1]);
        assert_ne!(keys[1], keys[2]);
        assert_ne!(keys[0], keys[2]);
    }
}

#[cfg(test)]
mod payload_resolution_tests {
    use super::*;

    #[test]
    fn union_payloads_resolve_lazy_witnesses_recursively() {
        let data = Data::materialized(Value::Text(String::from("carried")));
        let slot = DataSlot::new(&data);
        let value = expr::EvalValue::TaggedUnion {
            case: String::from("Done"),
            payload: vec![(
                String::from("value"),
                expr::EvalValue::Payload(expr::PayloadRef::Input),
            )],
        };

        assert_eq!(
            resolve_payload(value, &slot).expect("the nested witness materializes"),
            Value::TaggedUnion(Box::new(crate::value::TaggedUnionValue {
                case: String::from("Done"),
                payload: vec![(String::from("value"), Value::Text(String::from("carried")))],
            }))
        );
    }
}
