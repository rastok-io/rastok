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

use crate::boundary::{BoundaryEvent, CallOutcome};
use crate::datatype::DataType;
use crate::envelope::{self, Data, Envelope, Meta, MetaSys, MetaValue, Scalar};
use crate::expr;
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
/// transaction changed nothing.
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

/// The partition key of a state cell; M0 runs everything under the
/// default key, but isolation is keyed from day one.
pub type Key = String;

/// State cells partitioned by key: reads under one key never see
/// another key's writes.
#[derive(Debug, Default)]
pub struct PartitionState {
    cells: BTreeMap<(Slot, Key), BTreeMap<String, Value>>,
}

impl PartitionState {
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
        key: &str,
    ) -> Result<BTreeMap<String, Value>, Fault> {
        if plan.nodes.get(slot.index()).is_none() {
            return Err(Fault::UnknownSlot(slot));
        }
        Ok(self.cell_unchecked(plan.plan(), slot, key))
    }

    /// The unchecked read the machine uses on its own plan-derived
    /// slots — crate-private: the public door above verifies.
    pub(crate) fn cell_unchecked(
        &self,
        plan: &ExecutionPlan,
        slot: Slot,
        key: &str,
    ) -> BTreeMap<String, Value> {
        if let Some(cell) = self.cells.get(&(slot, Key::from(key))) {
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
        key: &str,
        delta: &[(String, Value)],
    ) -> Result<(), Fault> {
        if plan.nodes.get(slot.index()).is_none() {
            return Err(Fault::UnknownSlot(slot));
        }
        self.apply_unchecked(plan.plan(), slot, key, delta);
        Ok(())
    }

    /// The unchecked write behind [`Self::apply`] — crate-private.
    pub(crate) fn apply_unchecked(
        &mut self,
        plan: &ExecutionPlan,
        slot: Slot,
        key: &str,
        delta: &[(String, Value)],
    ) {
        let mut cell = self.cell_unchecked(plan, slot, key);
        for (name, value) in delta {
            cell.insert(name.clone(), value.clone());
        }
        self.cells.insert((slot, Key::from(key)), cell);
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
    /// The delivered envelope.
    pub envelope: Envelope,
}

/// Bounded per-`(node, key)` FIFO queues — one mailbox per node
/// instance, all ports in arrival order, per the execution model.
/// A claimed mailbox stays claimed while its node awaits an external
/// call: the worker is free, the mailbox is not. A full mailbox
/// refuses — backpressure is a signal, never a silent drop.
#[derive(Debug)]
pub struct Mailboxes {
    capacity: usize,
    queues: BTreeMap<(Slot, Key), VecDeque<Delivery>>,
    claimed: alloc::collections::BTreeSet<(Slot, Key)>,
    edge_seq: BTreeMap<String, u64>,
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
    pub fn has_room(&self, slot: Slot, key: &str, already: usize) -> bool {
        let queued = self
            .queues
            .get(&(slot, Key::from(key)))
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
        key: &str,
        port: node::Port,
        edge: String,
        envelope: Envelope,
    ) -> Result<(), Box<Envelope>> {
        let queue = self.queues.entry((slot, Key::from(key))).or_default();
        if queue.len() >= self.capacity {
            return Err(Box::new(envelope));
        }
        let seq = self.edge_seq.entry(edge.clone()).or_insert(0);
        *seq += 1;
        queue.push_back(Delivery {
            port,
            edge,
            seq: *seq,
            envelope,
        });
        Ok(())
    }

    /// The first unclaimed non-empty mailbox and a copy of its head —
    /// the reservation: nothing is removed until [`Self::pop`].
    #[must_use]
    pub fn peek_next(&self) -> Option<(Slot, Key, Delivery)> {
        self.queues
            .iter()
            .find(|((slot, key), queue)| {
                !queue.is_empty() && !self.claimed.contains(&(*slot, key.clone()))
            })
            .and_then(|((slot, key), queue)| {
                queue.front().map(|head| (*slot, key.clone(), head.clone()))
            })
    }

    /// Finds one specific queued delivery by its full reference —
    /// the canonical edge it traveled AND its envelope id — without
    /// removing it: the reservation half of the expired-in-queue
    /// path. Two ports of one connector are two references.
    #[must_use]
    pub fn find_by_ref(
        &self,
        slot: Slot,
        key: &str,
        edge: &str,
        id: crate::envelope::Id,
    ) -> Option<&Delivery> {
        self.queues
            .get(&(slot, Key::from(key)))?
            .iter()
            .find(|delivery| delivery.edge == edge && delivery.envelope.meta().sys().id() == id)
    }

    /// Removes one specific queued delivery by its full reference —
    /// only after the facts consuming it hold; FIFO order among the
    /// survivors holds.
    pub fn remove_by_ref(
        &mut self,
        slot: Slot,
        key: &str,
        edge: &str,
        id: crate::envelope::Id,
    ) -> Option<Delivery> {
        let queue = self.queues.get_mut(&(slot, Key::from(key)))?;
        let position = queue.iter().position(|delivery| {
            delivery.edge == edge && delivery.envelope.meta().sys().id() == id
        })?;
        queue.remove(position)
    }

    /// Removes the head of a mailbox — only after the transaction
    /// that consumes it is committed.
    pub fn pop(&mut self, slot: Slot, key: &str) -> Option<Delivery> {
        self.queues
            .get_mut(&(slot, Key::from(key)))
            .and_then(VecDeque::pop_front)
    }

    /// Claims a mailbox for the duration of an external call.
    pub fn claim(&mut self, slot: Slot, key: &str) {
        self.claimed.insert((slot, Key::from(key)));
    }

    /// Releases a claim after the call settled.
    pub fn release(&mut self, slot: Slot, key: &str) {
        self.claimed.remove(&(slot, Key::from(key)));
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
        delivery: &Delivery,
        state: &BTreeMap<String, Value>,
    ) -> Result<TransactionOutcome, Fault> {
        // The validated plan proves its own references, not the
        // caller's slot — verified, refused typed.
        let Some(node) = self.plan.nodes.get(slot.index()) else {
            return Err(Fault::UnknownSlot(slot));
        };
        // Two distinct facts share one port here: the handler is
        // *selected* by its activation pattern, while the committed
        // record carries the *causal* trigger. They coincide for a
        // single-port activation and will not for a join.
        let activation = node::HandlerActivation::Port(delivery.port.clone());
        let trigger = &node::Trigger::Port(delivery.port.clone());
        let handler = self
            .plan
            .handlers
            .iter()
            .find(|handler| handler.node == slot && handler.activation == activation)
            .ok_or(Fault::MissingHandler)?;
        let consumed = ConsumedPosition {
            edge: delivery.edge.clone(),
            seq: delivery.seq,
        };
        // Scratch isolation: the handler runs against copies; nothing
        // below touches the caller's state.
        let mut scratch = state.clone();
        let mut lets: BTreeMap<String, expr::EvalValue> = BTreeMap::new();
        let mut buffer = EmissionBuffer::new();
        let input = &delivery.envelope;
        let base = |scratch: &BTreeMap<String, Value>,
                    lets: &BTreeMap<String, expr::EvalValue>|
         -> BTreeMap<String, expr::EvalValue> {
            let mut map: BTreeMap<String, expr::EvalValue> = scratch
                .iter()
                .map(|(name, value)| (name.clone(), expr::EvalValue::from(value.clone())))
                .collect();
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
        let data_slot = DataSlot::new(input.data());
        if let Some(guard) = &handler.guard {
            let verdict = resolve_payload(
                eval_lazy(guard, &base(&scratch, &lets), &data_slot, &self.plan.assets)?,
                &data_slot,
            )?;
            match verdict {
                Value::Bool(true) => {}
                Value::Bool(false) => {
                    // Filtered: consumed with no effect.
                    return Ok(TransactionOutcome {
                        record: TransactionRecord {
                            node_path: node.path.clone(),
                            trigger: trigger.clone(),
                            consumed: vec![consumed],
                            state_delta: vec![],
                            emissions: vec![],
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
        for step in &handler.body {
            match step {
                Step::Let { name, expr } => {
                    let value =
                        eval_lazy(expr, &base(&scratch, &lets), &data_slot, &self.plan.assets)?;
                    lets.insert(name.clone(), value);
                }
                Step::SetVar { name, expr } => {
                    // A state write consumes the value — the store
                    // outlives the transaction, so the witness
                    // resolves here.
                    let value = resolve_payload(
                        eval_lazy(expr, &base(&scratch, &lets), &data_slot, &self.plan.assets)?,
                        &data_slot,
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
                        &data_slot,
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
                        // The content-type hint: an explicit write
                        // wins; unchanged content keeps the input's;
                        // rebuilt content carries none.
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
                            BuiltData::Unchanged => {
                                if let Data::Materialized(current) = input.data() {
                                    crossing_pins_hold(self.plan, &from, current)
                                        .map_err(PinViolation::into_fault)?;
                                }
                                input.data().clone()
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
                node_path: node.path.clone(),
                trigger: trigger.clone(),
                consumed: vec![consumed],
                state_delta,
                emissions,
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
                let budgeted = self.plan.nodes[target.index()]
                    .connector()
                    .and_then(|spec| spec.attribute("timeout"))
                    .is_some();
                if budgeted {
                    timers.push((
                        1,
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

    /// The typed-error transaction for a faulted handler: the input
    /// is consumed causally and the fault leaves on `err` as a typed
    /// payload — a machine never swallows an envelope. Crate-private:
    /// it indexes the slot unchecked, which is the VM's invariant to
    /// hold (its slots come from the plan) — the verified public
    /// door is [`Self::fire`].
    #[must_use]
    pub(crate) fn fault_transaction(
        &self,
        slot: Slot,
        delivery: &Delivery,
        fault: &Fault,
    ) -> TransactionOutcome {
        self.typed_error_transaction(slot, delivery, "RuntimeFault", &fault.to_string())
    }

    /// A transaction consuming `delivery` with a typed error on
    /// `err` — the shape every non-happy consumption shares.
    /// Crate-private for the same reason as
    /// [`Self::fault_transaction`].
    #[must_use]
    pub(crate) fn typed_error_transaction(
        &self,
        slot: Slot,
        delivery: &Delivery,
        kind: &str,
        message: &str,
    ) -> TransactionOutcome {
        let node = &self.plan.nodes[slot.index()];
        let trigger = node::Trigger::Port(delivery.port.clone());
        let err = node::Port::new("err").unwrap_or_else(|_| unreachable!("`err` is a valid port"));
        let id = ids::emission_id(
            &node.path,
            &trigger,
            delivery.envelope.meta().sys().id(),
            &err,
            0,
        );
        let sys = MetaSys::emission(id, delivery.envelope.meta().sys(), self.graph_version.0);
        let mut payload = BTreeMap::new();
        payload.insert(
            String::from("kind"),
            envelope::Value::Text(String::from(kind)),
        );
        payload.insert(
            String::from("message"),
            envelope::Value::Text(String::from(message)),
        );
        TransactionOutcome {
            record: TransactionRecord {
                node_path: node.path.clone(),
                trigger,
                consumed: vec![ConsumedPosition {
                    edge: delivery.edge.clone(),
                    seq: delivery.seq,
                }],
                state_delta: vec![],
                timers: vec![],
                emissions: vec![Emission {
                    port: err,
                    index: 0,
                    envelope: Envelope::new(
                        // Uniform propagation (D-2026-08-10-2): the
                        // error envelope inherits the input's zones;
                        // its payload is fresh, so the carried
                        // content_type is struck.
                        inherit_zones(Meta::new(sys), delivery.envelope.meta()),
                        Data::materialized(envelope::Value::Map(payload)),
                    ),
                }],
            },
        }
    }

    /// The connector's outcome transaction for a settled call: one
    /// emission of the outcome envelope on the settled port, trigger
    /// `p:in`, parented on the request — exactly the reference
    /// trace's shape. The dispatcher routed; this builds.
    ///
    /// # Errors
    ///
    /// [`Fault::MissingHandler`]-free by construction; faults only if
    /// the request spine is unavailable.
    pub fn connector_outcome(
        &self,
        event: &BoundaryEvent,
        trigger: &node::Trigger,
        request_meta: &Meta,
        consumed: ConsumedPosition,
    ) -> TransactionOutcome {
        let request_sys = request_meta.sys();
        let id = ids::emission_id(
            &event.connector_path,
            trigger,
            event.emitting_id,
            &event.port,
            0,
        );
        let sys = MetaSys::emission(id, request_sys, self.graph_version.0);
        let data = match &event.outcome {
            CallOutcome::Success(data) => data.clone(),
            CallOutcome::Failure(error) => {
                // The kind survives into the graph: err handlers test
                // `err.kind`, so a failure envelope carries it beside
                // the payload.
                let mut payload = BTreeMap::new();
                payload.insert(
                    String::from("kind"),
                    envelope::Value::Text(error.kind.clone()),
                );
                if let Data::Encoded {
                    encoding: envelope::Encoding::Raw,
                    bytes,
                    ..
                } = &error.data
                {
                    payload.insert(
                        String::from("detail"),
                        envelope::Value::Bytes(bytes.clone()),
                    );
                } else if let Data::Materialized(value) = &error.data {
                    payload.insert(String::from("detail"), value.clone());
                }
                Data::materialized(envelope::Value::Map(payload))
            }
        };
        // Uniform propagation (D-2026-08-10-2): the outcome inherits
        // the REQUEST's zones — an app token explicitly kept before
        // the call survives the reply. The reply's payload is always
        // fresh (the world answered), so the carried content_type is
        // struck; minting one, and folding connector-written meta
        // fields over these zones so they win, is the sheets'
        // `Populates` business (nothing writes meta in v1).
        let envelope = Envelope::new(inherit_zones(Meta::new(sys), request_meta), data);
        TransactionOutcome {
            record: TransactionRecord {
                node_path: event.connector_path.clone(),
                trigger: trigger.clone(),
                consumed: vec![consumed],
                state_delta: vec![],
                timers: vec![],
                emissions: vec![Emission {
                    port: event.port.clone(),
                    index: 0,
                    envelope,
                }],
            },
        }
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

/// What one emission carries out of its builder. `data` stays
/// [`BuiltData::Unchanged`] when no payload write touched it — the
/// original representation (encoded bytes included) and its
/// content-type hint survive, per the "no structural read, no
/// transcode" rule.
pub(crate) struct Built {
    /// The payload outcome.
    pub(crate) data: BuiltData,
    /// The outgoing application zone.
    pub(crate) app: envelope::Namespace,
    /// The outgoing external-context zone — `content_type` already
    /// derived from the payload decision, explicit writes folded.
    pub(crate) ext: envelope::Namespace,
}

/// A builder's payload verdict.
pub(crate) enum BuiltData {
    /// No payload write ran: the input's representation ships as-is.
    Unchanged,
    /// The payload was (re)built as one materialized core value.
    Value(Value),
}

/// Folds the input envelope's zones onto a fresh emission meta —
/// the uniform-propagation rule (D-2026-08-10-2) for envelopes the
/// MACHINE mints (connector outcomes, typed errors): `app` carries
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
    slot: &DataSlot<'_>,
    plan: &crate::plan::ExecutionPlan,
) -> Result<Option<Built>, Fault> {
    let assets: &[crate::plan::AssetBlob] = &plan.assets;
    // The atomic order of D-2026-08-10-4: copy the zones, decide
    // the payload, DERIVE `content_type` from that decision (a kept
    // witness keeps it, fresh data strikes it), then let explicit
    // writes win.
    let inherited = |data: BuiltData, fresh: bool| {
        let mut ext = input.meta().ext().clone();
        if fresh {
            ext.remove("content_type");
        }
        Built {
            data,
            app: input.meta().app().clone(),
            ext,
        }
    };
    match emission {
        PlanEmission::Nothing => Ok(None),
        PlanEmission::Identity => Ok(Some(inherited(BuiltData::Unchanged, false))),
        PlanEmission::Data(expr) => {
            // No spelling-dependent special case, in EITHER
            // direction: the evaluator preserves the struct and
            // laziness witnesses. A whole payload
            // arriving uninspected — `out = data`, `out = copy`, the
            // taken arm of a ternary — ships the REPRESENTATION,
            // content type included; anything else is FRESH data and
            // strikes it (D-2026-08-10-4).
            let value = eval_lazy(expr, base, slot, assets)?;
            if matches!(value, expr::EvalValue::Payload) {
                return Ok(Some(inherited(BuiltData::Unchanged, false)));
            }
            let value = resolve_payload(value, slot)?;
            Ok(Some(inherited(BuiltData::Value(value), true)))
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
                matches!(&write.target, envelope::WriteTarget::Data(path) if path.is_empty())
                    && matches!(
                        eval_lazy(&write.expr, base, slot, assets)?,
                        expr::EvalValue::Payload
                    )
            } else {
                false
            };
            // Fresh iff the payload changes: any data write (patch)
            // or the fresh-build form (replace) — except the carry.
            let fresh = !carry && (replace || !data_writes.is_empty());
            let mut built = inherited(BuiltData::Unchanged, fresh);
            let mut data: Option<envelope::Value> = if replace && !carry {
                // A fresh STRUCT — the physical form of a braced
                // record (D-2026-08-09-1).
                Some(envelope::Value::Struct(Vec::new()))
            } else {
                None
            };
            for write in writes {
                if carry && matches!(&write.target, envelope::WriteTarget::Data(_)) {
                    // The carried representation IS the payload.
                    continue;
                }
                let value = resolve_payload(eval_lazy(&write.expr, base, slot, assets)?, slot)?;
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
                                _ => slot.materialize()?,
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
                        // standard field keeps its implicit `text`
                        // contract even through a `variant` expression;
                        // sheet fields answer to their declared
                        // types.
                        let converted = to_meta_value(&value)?;
                        let ty = if field == "content_type" {
                            Some(DataType::Text)
                        } else {
                            plan.ext_schema
                                .iter()
                                .find(|(known, _)| known == field)
                                .map(|(_, ty)| ty.clone())
                        };
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
    if let Some(pinned) = plan.resolved_ports().endpoint_type(from)
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
        (name == "data").then_some(expr::EvalValue::Payload)
    }

    fn payload(&self) -> Result<expr::EvalValue, expr::Error> {
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

/// Resolves every payload witness in a value — the host-side twin of
/// the evaluator's structural demand, for the points where a value
/// LEAVES the evaluation: a state write, an emission conversion, a
/// mock reply. Composites resolve deep.
///
/// # Errors
/// The materialization fault, if the payload cannot decode.
pub fn resolve_payload(value: expr::EvalValue, slot: &DataSlot<'_>) -> Result<Value, Fault> {
    Ok(match value {
        expr::EvalValue::Payload => slot.materialize()?,
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

/// Evaluates against the lazy binding. A materialization fault takes
/// precedence over whatever the evaluator made of the missing
/// binding: a demanded decode that failed is the transaction's
/// fault, even if the expression could have absorbed an unbound
/// name.
///
/// # Errors
/// The materialization fault, or the evaluation's own.
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
/// host layer (lab, adapters, tooling) uses to read what a handler
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
                    MetaValue::Array(_) => Err(Fault::Unrepresentable(String::from(
                        "a nested array in a flat meta zone",
                    ))),
                })
                .collect::<Result<alloc::vec::Vec<_>, _>>()?;
            MetaValue::Array(scalars)
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
            payload: vec![(String::from("value"), expr::EvalValue::Payload)],
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
