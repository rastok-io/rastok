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
use crate::cel;
use crate::envelope::{self, Data, Envelope, Meta, MetaSys, MetaValue};
use crate::journal::{ConsumedPosition, Emission, TransactionRecord, ids};
use crate::node;
use crate::plan::{
    Emission as PlanEmission, ExecutionPlan, GraphVersion, Handler, Slot, Step,
    ValidatedExecutionPlan,
};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::{format, vec};
use core::fmt;

/// Pure, deterministic, budgeted guest execution — the contract the
/// jq and SQL engines will implement; no engine ships in M0.
pub trait GuestExecutor {
    /// Runs an opaque guest program over a value within a budget.
    ///
    /// # Errors
    ///
    /// A typed guest fault; never an observation of the host.
    fn run(&self, program: &[u8], input: cel::Value, budget: u64) -> Result<cel::Value, Fault>;
}

/// A runtime fault of one transaction. Atomicity holds: a faulted
/// transaction changed nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// A CEL evaluation fault.
    Eval(cel::Error),
    /// A value that envelope data cannot represent yet.
    Unrepresentable(String),
    /// The plan has no handler for this node and trigger.
    MissingHandler,
    /// The payload could not be materialized for the handler.
    Opaque(String),
    /// A struct carrying one field name twice — ill-formed data the
    /// canonical seal would silently collapse; refused at the doors,
    /// never resolved by "first found".
    DuplicateField(String),
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
            Self::MissingHandler => f.write_str("no handler for this trigger"),
            Self::Opaque(what) => write!(f, "the payload is opaque here: {what}"),
            Self::DuplicateField(name) => {
                write!(f, "the struct field `{name}` appears twice")
            }
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
    cells: BTreeMap<(Slot, Key), BTreeMap<String, cel::Value>>,
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
    ) -> Result<BTreeMap<String, cel::Value>, Fault> {
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
    ) -> BTreeMap<String, cel::Value> {
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
        delta: &[(String, cel::Value)],
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
        delta: &[(String, cel::Value)],
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
    plan: &'plan ExecutionPlan,
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
            plan: plan.plan(),
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
        state: &BTreeMap<String, cel::Value>,
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
        let mut lets: BTreeMap<String, cel::Value> = BTreeMap::new();
        let mut buffer = EmissionBuffer::new();
        let input = &delivery.envelope;
        let base = |scratch: &BTreeMap<String, cel::Value>,
                    lets: &BTreeMap<String, cel::Value>|
         -> BTreeMap<String, cel::Value> {
            let mut map = scratch.clone();
            for (name, value) in lets {
                map.insert(name.clone(), value.clone());
            }
            map.insert(String::from("meta"), meta_value(input.meta()));
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
                cel::Value::Bool(true) => {}
                cel::Value::Bool(false) => {
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
                    return Err(Fault::Eval(cel::Error {
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
                    // A state cell holds its DECLARED type at every
                    // write, not just at the initializer: a dynamic
                    // value of the wrong shape — or an int leaving
                    // the 32-bit range — is a typed error, never a
                    // silent coercion.
                    if let Some(cell) = node.vars.iter().find(|cell| &cell.name == name)
                        && let Some(violation) = conformance_fault(name, &cell.ty, &value)
                    {
                        return Err(Fault::Eval(cel::Error {
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
                        let own_pin = node
                            .ports
                            .iter()
                            .find(|decl| &decl.port == port)
                            .and_then(|decl| decl.ty.as_ref());
                        let from = crate::plan::EndpointRef {
                            node: Some(slot),
                            port: port.clone(),
                        };
                        let data = match built.data {
                            BuiltData::Unchanged => {
                                if let Data::Materialized(current) = input.data() {
                                    crossing_pins_hold(
                                        self.plan,
                                        &from,
                                        own_pin,
                                        &from_envelope_value(current),
                                    )
                                    .map_err(PinViolation::into_fault)?;
                                }
                                input.data().clone()
                            }
                            BuiltData::Value(value) => {
                                crossing_pins_hold(self.plan, &from, own_pin, &value)
                                    .map_err(PinViolation::into_fault)?;
                                Data::materialized(to_envelope_value(&value)?)
                            }
                            BuiltData::Envelope(value) => {
                                crossing_pins_hold(
                                    self.plan,
                                    &from,
                                    own_pin,
                                    &from_envelope_value(&value),
                                )
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
        let state_delta: Vec<(String, cel::Value)> = scratch
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
            envelope::Value::Scalar(envelope::Scalar::Text(String::from(kind))),
        );
        payload.insert(
            String::from("message"),
            envelope::Value::Scalar(envelope::Scalar::Text(String::from(message))),
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
                        Meta::new(sys),
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
        request_sys: &MetaSys,
        consumed: ConsumedPosition,
    ) -> TransactionOutcome {
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
                    envelope::Value::Scalar(envelope::Scalar::Text(error.kind.clone())),
                );
                if let Data::Encoded {
                    encoding: envelope::Encoding::Raw,
                    bytes,
                    ..
                } = &error.data
                {
                    payload.insert(
                        String::from("detail"),
                        envelope::Value::Scalar(envelope::Scalar::Bytes(bytes.clone())),
                    );
                } else if let Data::Materialized(value) = &error.data {
                    payload.insert(String::from("detail"), value.clone());
                }
                Data::materialized(envelope::Value::Map(payload))
            }
        };
        let envelope = Envelope::new(Meta::new(sys), data);
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
    /// The payload was (re)built as an evaluated CEL value.
    Value(cel::Value),
    /// The payload was built directly in the ENVELOPE domain by a
    /// builder's write fold. (CEL VALUES carry the struct witness
    /// too, since the bridge preserves it — this variant simply
    /// skips a domain round trip the fold never needed.)
    Envelope(envelope::Value),
}

fn emitted_content(
    emission: &PlanEmission,
    input: &Envelope,
    base: &BTreeMap<String, cel::Value>,
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
            // direction: the struct witness rides `cel::Value` and
            // the laziness witness rides it too. A whole payload
            // arriving uninspected — `out = data`, `out = copy`, the
            // taken arm of a ternary — ships the REPRESENTATION,
            // content type included; anything else is FRESH data and
            // strikes it (D-2026-08-10-4).
            let value = eval_lazy(expr, base, slot, assets)?;
            if matches!(value, cel::Value::Payload) {
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
                        cel::Value::Payload
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
                                _ => to_envelope_value(&slot.materialize()?)?,
                            }),
                        };
                        apply_struct_write(current, path, to_envelope_value(&value)?);
                    }
                    envelope::WriteTarget::App(field) => {
                        // The travelling schema judges the VALUE — a
                        // Dyn expression cannot smuggle a text into
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
                        // contract even through a Dyn expression;
                        // sheet fields answer to their declared
                        // types.
                        let converted = to_meta_value(&value)?;
                        let ty = if field == "content_type" {
                            Some(cel::Type::Text)
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
                built.data = BuiltData::Envelope(value);
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
    pub violation: ConformanceViolation,
}

impl PinViolation {
    /// The flattening callers on the Fault path use.
    #[must_use]
    pub fn into_fault(self) -> Fault {
        Fault::Eval(cel::Error {
            message: alloc::string::ToString::to_string(&self.violation),
        })
    }
}

/// Holds materialized content to WELL-FORMEDNESS: a struct may not
/// carry one field name twice — the canonical seal would silently
/// collapse the pair, so the doors refuse it typed instead of
/// resolving by "first found".
pub(crate) fn value_well_formed(value: &envelope::Value) -> Result<(), Fault> {
    match value {
        envelope::Value::Scalar(_) => Ok(()),
        envelope::Value::Struct(fields) => {
            let mut seen = alloc::collections::BTreeSet::new();
            for (name, value) in fields {
                if !seen.insert(name.as_str()) {
                    return Err(Fault::DuplicateField(name.clone()));
                }
                value_well_formed(value)?;
            }
            Ok(())
        }
        envelope::Value::Array(items) => items.iter().try_for_each(value_well_formed),
        envelope::Value::Map(entries) => entries.values().try_for_each(value_well_formed),
    }
}

/// Judges materialized content against every pin it must cross —
/// the emitting end's own pin and each fan-out target's — BEFORE any
/// commit: routing is post-commit and must never fail. Encoded bytes
/// never reach this judge (the codecs' door); `dyn` pins admit
/// anything.
pub(crate) fn crossing_pins_hold(
    plan: &crate::plan::ExecutionPlan,
    from: &crate::plan::EndpointRef,
    own_pin: Option<&cel::Type>,
    value: &cel::Value,
) -> Result<(), PinViolation> {
    if let Some(pinned) = own_pin
        && let Some(violation) = conformance_fault(from.port.name(), pinned, value)
    {
        return Err(PinViolation {
            at: from.clone(),
            violation,
        });
    }
    for edge in plan.edges.iter().filter(|edge| edge.from == *from) {
        let Some(to) = &edge.to else { continue };
        let pinned = match to.node {
            Some(slot) => plan
                .nodes
                .get(slot.index())
                .and_then(|node| node.ports.iter().find(|decl| decl.port == to.port))
                .and_then(|decl| decl.ty.as_ref()),
            None => plan
                .boundary
                .iter()
                .find(|decl| decl.port == to.port)
                .and_then(|decl| decl.ty.as_ref()),
        };
        if let Some(pinned) = pinned
            && let Some(violation) = conformance_fault(to.port.name(), pinned, value)
        {
            return Err(PinViolation {
                at: to.clone(),
                violation,
            });
        }
    }
    Ok(())
}

/// What kind of value stood at a refused pin — the machine's
/// vocabulary for "found", never a rendered sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    /// A boolean.
    Bool,
    /// A signed or unsigned integer.
    Number,
    /// A binary64.
    Double,
    /// Text.
    Text,
    /// Bytes.
    Bytes,
    /// A duration.
    Duration,
    /// A timestamp.
    Timestamp,
    /// The null value.
    Null,
    /// A list.
    List,
    /// A map.
    Map,
    /// A struct.
    Struct,
    /// An unmaterialized payload witness — judged values are
    /// resolved first, so a judge never meets one; total anyway.
    Opaque,
    /// An exact dynamic decimal (D-2026-08-10-7).
    Decimal,
}

impl ValueKind {
    fn of(value: &cel::Value) -> Self {
        match value {
            cel::Value::Bool(_) => Self::Bool,
            cel::Value::Int(_) | cel::Value::Uint(_) => Self::Number,
            cel::Value::Decimal { .. } => Self::Decimal,
            cel::Value::Double(_) => Self::Double,
            cel::Value::Text(_) => Self::Text,
            cel::Value::Bytes(_) => Self::Bytes,
            cel::Value::Duration(_) => Self::Duration,
            cel::Value::Timestamp(_) => Self::Timestamp,
            cel::Value::Null => Self::Null,
            cel::Value::List(_) => Self::List,
            cel::Value::Map(_) => Self::Map,
            cel::Value::Struct(_) => Self::Struct,
            cel::Value::Payload => Self::Opaque,
        }
    }
}

impl fmt::Display for ValueKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Bool => "bool",
            Self::Number => "number",
            Self::Double => "double",
            Self::Text => "text",
            Self::Bytes => "bytes",
            Self::Duration => "duration",
            Self::Timestamp => "timestamp",
            Self::Null => "null",
            Self::List => "list",
            Self::Map => "map",
            Self::Struct => "struct",
            Self::Opaque => "opaque",
            Self::Decimal => "decimal",
        })
    }
}

/// Why a pin refused — semantic, matchable; `Display` renders the
/// sentence at the frontier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConformanceReason {
    /// The value's kind cannot inhabit the declared type.
    TypeMismatch,
    /// The right kind, outside the physical range.
    IntegerOutOfRange {
        /// The offending value.
        value: i64,
    },
}

/// One refused conformance: the declared expectation, what stood
/// there, and why — structure first, text at `Display`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConformanceViolation {
    /// The declaring name (a cell or a port).
    pub name: String,
    /// The declared type.
    pub expected: cel::Type,
    /// The kind of value that stood there.
    pub found: ValueKind,
    /// The semantic reason.
    pub reason: ConformanceReason,
}

impl fmt::Display for ConformanceViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.reason {
            ConformanceReason::TypeMismatch => write!(
                f,
                "`{}` is declared {:?}; a {} cannot enter it",
                self.name, self.expected, self.found
            ),
            ConformanceReason::IntegerOutOfRange { value } => write!(
                f,
                "`{}` is an int; {value} leaves the 32-bit range",
                self.name
            ),
        }
    }
}

/// Judges a runtime value against a declared cell type; `None` means
/// it conforms. `Dyn` admits anything — the dynamic escape is chosen
/// in the declaration, not discovered at the write.
pub(crate) fn conformance_fault(
    name: &str,
    declared: &cel::Type,
    value: &cel::Value,
) -> Option<ConformanceViolation> {
    let mismatch = || {
        Some(ConformanceViolation {
            name: String::from(name),
            expected: declared.clone(),
            found: ValueKind::of(value),
            reason: ConformanceReason::TypeMismatch,
        })
    };
    match (declared, value) {
        (cel::Type::Dyn, _) => None,
        // A dynamic decimal conforms to no precise M0 type — it is
        // narrowed before use (D-2026-08-10-7).
        (_, cel::Value::Decimal { .. }) => mismatch(),
        // Judged values are resolved first; an unmaterialized
        // witness meeting a precise judge is a mismatch, total.
        (_, cel::Value::Payload) => mismatch(),
        (cel::Type::Int, cel::Value::Int(candidate)) => {
            if i32::try_from(*candidate).is_err() {
                Some(ConformanceViolation {
                    name: String::from(name),
                    expected: cel::Type::Int,
                    found: ValueKind::Number,
                    reason: ConformanceReason::IntegerOutOfRange { value: *candidate },
                })
            } else {
                None
            }
        }
        (cel::Type::Bool, cel::Value::Bool(_))
        | (cel::Type::Text, cel::Value::Text(_))
        | (cel::Type::Bytes, cel::Value::Bytes(_))
        | (cel::Type::Double, cel::Value::Double(_))
        | (cel::Type::Duration, cel::Value::Duration(_))
        | (cel::Type::Timestamp, cel::Value::Timestamp(_)) => None,
        (_, cel::Value::Bool(_)) => mismatch(),
        (_, cel::Value::Int(_) | cel::Value::Uint(_)) => mismatch(),
        (_, cel::Value::Double(_)) => mismatch(),
        (_, cel::Value::Text(_)) => mismatch(),
        (_, cel::Value::Bytes(_)) => mismatch(),
        (_, cel::Value::Struct(_)) => mismatch(),
        (_, cel::Value::Duration(_)) => mismatch(),
        (_, cel::Value::Timestamp(_)) => mismatch(),
        (_, cel::Value::Null) => mismatch(),
        (_, cel::Value::List(_)) => mismatch(),
        (_, cel::Value::Map(_)) => mismatch(),
    }
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
    cache: core::cell::RefCell<Option<cel::Value>>,
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
    pub fn materialize(&self) -> Result<cel::Value, Fault> {
        if let Some(value) = self.cache.borrow().as_ref() {
            return Ok(value.clone());
        }
        let value = data_value(self.data)?;
        *self.cache.borrow_mut() = Some(value.clone());
        Ok(value)
    }

    /// The evaluator's demand: a failed materialization parks its
    /// fault for [`eval_lazy`] to surface and answers "unbound".
    fn demand(&self) -> Option<cel::Value> {
        match self.materialize() {
            Ok(value) => Some(value),
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
    pub base: &'a BTreeMap<String, cel::Value>,
    /// The on-demand payload.
    pub data: &'a DataSlot<'a>,
    /// The plan's build-time asset table (D-2026-08-10-5).
    pub assets: &'a [crate::plan::AssetBlob],
}

impl cel::Bindings for LazyBindings<'_> {
    fn get(&self, name: &str) -> Option<cel::Value> {
        if let Some(value) = self.base.get(name) {
            return Some(value.clone());
        }
        // Resolving the NAME is not yet inspecting the structure:
        // the witness carries; `Bindings::payload` materializes.
        (name == "data").then_some(cel::Value::Payload)
    }

    fn payload(&self) -> Result<cel::Value, cel::Error> {
        self.data.demand().ok_or_else(|| cel::Error {
            message: String::from("the payload could not materialize"),
        })
    }

    fn asset(&self, index: u32) -> Result<cel::Value, cel::Error> {
        self.assets
            .get(index as usize)
            .map(|blob| cel::Value::Bytes(blob.bytes.clone()))
            .ok_or_else(|| cel::Error {
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
pub fn resolve_payload(value: cel::Value, slot: &DataSlot<'_>) -> Result<cel::Value, Fault> {
    Ok(match value {
        cel::Value::Payload => slot.materialize()?,
        cel::Value::List(items) => cel::Value::List(
            items
                .into_iter()
                .map(|item| resolve_payload(item, slot))
                .collect::<Result<_, _>>()?,
        ),
        cel::Value::Map(map) => {
            let mut deep = BTreeMap::new();
            for (key, value) in map {
                deep.insert(key, resolve_payload(value, slot)?);
            }
            cel::Value::Map(deep)
        }
        cel::Value::Struct(fields) => cel::Value::Struct(
            fields
                .into_iter()
                .map(|(name, value)| Ok((name, resolve_payload(value, slot)?)))
                .collect::<Result<_, _>>()?,
        ),
        other => other,
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
    tree: &cel::Tree,
    base: &BTreeMap<String, cel::Value>,
    slot: &DataSlot<'_>,
    assets: &[crate::plan::AssetBlob],
) -> Result<cel::Value, Fault> {
    let env = LazyBindings {
        base,
        data: slot,
        assets,
    };
    let result = cel::eval(tree, &env).map_err(Fault::Eval);
    if let Some(fault) = slot.take_fault() {
        return Err(fault);
    }
    result
}

/// The handler-facing CEL view of an envelope's data — the bridge a
/// host layer (lab, adapters, tooling) uses to read what a handler
/// reads.
pub fn data_value(data: &Data) -> Result<cel::Value, Fault> {
    // `Data::materialize_with` is the CACHE AUTHORITY: the decode
    // lands in the envelope's own cell, so an identity re-emission
    // ships the filled cell and the next node never decodes again —
    // once per ENVELOPE, not once per transaction (envelope.md §5.1).
    Ok(from_envelope_value(data.materialize_with(decode_encoded)?))
}

/// Decodes encoded bytes into the envelope domain — the closure
/// `Data::materialize_with` runs on first structural access.
fn decode_encoded(encoding: envelope::Encoding, bytes: &[u8]) -> Result<envelope::Value, Fault> {
    match encoding {
        envelope::Encoding::Raw => Ok(envelope::Value::Scalar(match core::str::from_utf8(bytes) {
            Ok(text) => envelope::Scalar::Text(String::from(text)),
            Err(_) => envelope::Scalar::Bytes(bytes.to_vec()),
        })),
        other => Err(Fault::Opaque(format!(
            "{other} decoding is not in this build"
        ))),
    }
}

/// The handler-facing CEL view of an envelope's metadata: the full
/// `meta.sys` spine and the two writable zones, flat dotted keys
/// nested for reading.
pub fn meta_value(meta: &Meta) -> cel::Value {
    let mut app = BTreeMap::new();
    for (field, value) in meta.app() {
        app.insert(field.clone(), from_meta_value(value));
    }
    // The whole five-field spine — a handler reading any `meta.sys`
    // field the spec names must find it, not a hole.
    let mut sys = BTreeMap::new();
    sys.insert(
        String::from("id"),
        cel::Value::Text(meta.sys().id().to_string()),
    );
    sys.insert(
        String::from("parent_id"),
        match meta.sys().parent_id() {
            Some(parent) => cel::Value::Text(parent.to_string()),
            None => cel::Value::Null,
        },
    );
    sys.insert(
        String::from("origin_id"),
        cel::Value::Text(meta.sys().origin_id().to_string()),
    );
    sys.insert(
        String::from("role"),
        cel::Value::Text(meta.sys().role().to_string()),
    );
    sys.insert(
        String::from("graph_version"),
        cel::Value::Int(i64::try_from(meta.sys().graph_version()).unwrap_or(i64::MAX)),
    );
    let mut root = BTreeMap::new();
    // Exactly three zones, no alias, no root fields
    // (D-2026-08-10-1): storage is FLAT dotted keys, the projection
    // NESTS by splitting on dots so `meta.app.audit.origin` reads
    // naturally.
    root.insert(String::from("app"), nested_zone(&app));
    root.insert(String::from("ext"), nested_zone_meta(meta.ext()));
    root.insert(String::from("sys"), cel::Value::Map(sys));
    cel::Value::Map(root)
}

/// Nests a zone's flat dotted keys into the CEL map tree the
/// projection exposes — one storage form, nested views derived.
fn nested_from_pairs(pairs: impl Iterator<Item = (String, cel::Value)>) -> cel::Value {
    let mut root: BTreeMap<String, cel::Value> = BTreeMap::new();
    for (key, value) in pairs {
        let segments: alloc::vec::Vec<&str> = key.split('.').collect();
        let mut cursor = &mut root;
        for segment in &segments[..segments.len() - 1] {
            let entry = cursor
                .entry(String::from(*segment))
                .or_insert_with(|| cel::Value::Map(BTreeMap::new()));
            match entry {
                cel::Value::Map(inner) => cursor = inner,
                // Prefix-freedom is a checker invariant; a leaf met
                // on the way is unreachable from validated plans —
                // keep the projection total by shadowing.
                other => {
                    *other = cel::Value::Map(BTreeMap::new());
                    let cel::Value::Map(inner) = other else {
                        unreachable!("just inserted a map")
                    };
                    cursor = inner;
                }
            }
        }
        cursor.insert(String::from(segments[segments.len() - 1]), value);
    }
    cel::Value::Map(root)
}

fn nested_zone(zone: &BTreeMap<String, cel::Value>) -> cel::Value {
    nested_from_pairs(zone.iter().map(|(k, v)| (k.clone(), v.clone())))
}

fn nested_zone_meta(zone: &envelope::Namespace) -> cel::Value {
    nested_from_pairs(zone.iter().map(|(k, v)| (k.clone(), from_meta_value(v))))
}

pub(crate) fn from_envelope_value(value: &envelope::Value) -> cel::Value {
    match value {
        envelope::Value::Scalar(scalar) => from_scalar(scalar),
        // The logical type CROSSES the bridge: a struct stays a
        // struct through bindings, locals, and ternaries — tag 17
        // never silently becomes tag 18 on the way through CEL.
        envelope::Value::Struct(fields) => cel::Value::Struct(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), from_envelope_value(value)))
                .collect(),
        ),
        envelope::Value::Array(items) => {
            cel::Value::List(items.iter().map(from_envelope_value).collect())
        }
        envelope::Value::Map(map) => cel::Value::Map(
            map.iter()
                .map(|(key, value)| (key.clone(), from_envelope_value(value)))
                .collect(),
        ),
    }
}

fn from_scalar(scalar: &envelope::Scalar) -> cel::Value {
    match scalar {
        envelope::Scalar::Bool(value) => cel::Value::Bool(*value),
        envelope::Scalar::Int(value) => cel::Value::Int(*value),
        envelope::Scalar::Double(value) => cel::Value::Double(*value),
        envelope::Scalar::Text(value) => cel::Value::Text(value.clone()),
        envelope::Scalar::Bytes(value) => cel::Value::Bytes(value.clone()),
        envelope::Scalar::Decimal { unscaled, scale } => cel::Value::Decimal {
            unscaled: *unscaled,
            scale: *scale,
        },
    }
}

fn from_meta_value(value: &MetaValue) -> cel::Value {
    match value {
        MetaValue::Scalar(scalar) => from_scalar(scalar),
        MetaValue::Array(items) => cel::Value::List(items.iter().map(from_scalar).collect()),
    }
}

/// Converts a CEL value into the envelope data domain.
pub fn to_envelope_value(value: &cel::Value) -> Result<envelope::Value, Fault> {
    Ok(match value {
        cel::Value::Decimal { unscaled, scale } => {
            envelope::Value::Scalar(envelope::Scalar::Decimal {
                unscaled: *unscaled,
                scale: *scale,
            })
        }
        // Emission builders resolve the witness before converting;
        // an unresolved one is a caller defect, refused typed.
        cel::Value::Payload => {
            return Err(Fault::Unrepresentable(String::from(
                "an unmaterialized payload cannot enter the envelope domain",
            )));
        }
        cel::Value::Bool(v) => envelope::Value::Scalar(envelope::Scalar::Bool(*v)),
        cel::Value::Int(v) => envelope::Value::Scalar(envelope::Scalar::Int(*v)),
        cel::Value::Uint(v) => envelope::Value::Scalar(envelope::Scalar::Int(
            i64::try_from(*v).map_err(|_| Fault::Unrepresentable(String::from("uint > i64")))?,
        )),
        cel::Value::Double(v) => envelope::Value::Scalar(envelope::Scalar::Double(*v)),
        cel::Value::Text(v) => envelope::Value::Scalar(envelope::Scalar::Text(v.clone())),
        cel::Value::Bytes(v) => envelope::Value::Scalar(envelope::Scalar::Bytes(v.clone())),
        cel::Value::List(items) => envelope::Value::Array(
            items
                .iter()
                .map(to_envelope_value)
                .collect::<Result<_, _>>()?,
        ),
        cel::Value::Map(map) => envelope::Value::Map(
            map.iter()
                .map(|(key, item)| Ok((key.clone(), to_envelope_value(item)?)))
                .collect::<Result<_, Fault>>()?,
        ),
        cel::Value::Struct(fields) => envelope::Value::Struct(
            fields
                .iter()
                .map(|(name, item)| Ok((name.clone(), to_envelope_value(item)?)))
                .collect::<Result<_, Fault>>()?,
        ),
        other @ (cel::Value::Duration(_) | cel::Value::Timestamp(_) | cel::Value::Null) => {
            return Err(Fault::Unrepresentable(format!("{other:?}")));
        }
    })
}

/// Converts a CEL value into a writable-metadata leaf.
/// The ONE conformance rule between a metadata value and a schema
/// type (D-2026-08-10-1) — used at runtime zone writes, at the VM
/// ingress, and mirrored by the checker's static list. `Dyn` admits
/// every representable leaf.
#[must_use]
pub fn meta_conforms(value: &MetaValue, ty: &cel::Type) -> bool {
    match ty {
        cel::Type::Dyn => true,
        cel::Type::Bool => matches!(value, MetaValue::Scalar(envelope::Scalar::Bool(_))),
        cel::Type::Int => matches!(value, MetaValue::Scalar(envelope::Scalar::Int(_))),
        cel::Type::Double => matches!(value, MetaValue::Scalar(envelope::Scalar::Double(_))),
        cel::Type::Text => matches!(value, MetaValue::Scalar(envelope::Scalar::Text(_))),
        cel::Type::Bytes => matches!(value, MetaValue::Scalar(envelope::Scalar::Bytes(_))),
        cel::Type::List(inner) => match value {
            MetaValue::Array(items) => items
                .iter()
                .all(|scalar| meta_conforms(&MetaValue::Scalar(scalar.clone()), inner)),
            MetaValue::Scalar(_) => false,
        },
        _ => false,
    }
}

/// Converts a CEL value into a metadata leaf — the physical half of
/// [`meta_conforms`]'s rule.
///
/// # Errors
/// A typed refusal when the value has no flat-leaf representation.
pub fn to_meta_value(value: &cel::Value) -> Result<MetaValue, Fault> {
    Ok(match value {
        cel::Value::Bool(v) => MetaValue::Scalar(envelope::Scalar::Bool(*v)),
        cel::Value::Int(v) => MetaValue::Scalar(envelope::Scalar::Int(*v)),
        cel::Value::Double(v) => MetaValue::Scalar(envelope::Scalar::Double(*v)),
        cel::Value::Text(v) => MetaValue::Scalar(envelope::Scalar::Text(v.clone())),
        cel::Value::Bytes(v) => MetaValue::Scalar(envelope::Scalar::Bytes(v.clone())),
        // `array<scalar>` is spec-admitted meta (envelope.md §0).
        cel::Value::List(items) => {
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
