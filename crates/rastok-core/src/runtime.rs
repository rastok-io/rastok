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
        let bindings = |scratch: &BTreeMap<String, cel::Value>,
                        lets: &BTreeMap<String, cel::Value>|
         -> Result<BTreeMap<String, cel::Value>, Fault> {
            let mut map = scratch.clone();
            for (name, value) in lets {
                map.insert(name.clone(), value.clone());
            }
            map.insert(String::from("data"), data_value(input.data())?);
            map.insert(String::from("meta"), meta_value(input.meta()));
            Ok(map)
        };
        if let Some(guard) = &handler.guard {
            let env = bindings(&scratch, &lets)?;
            match cel::eval(guard, &env).map_err(Fault::Eval)? {
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
                    let env = bindings(&scratch, &lets)?;
                    let value = cel::eval(expr, &env).map_err(Fault::Eval)?;
                    lets.insert(name.clone(), value);
                }
                Step::SetVar { name, expr } => {
                    let env = bindings(&scratch, &lets)?;
                    let value = cel::eval(expr, &env).map_err(Fault::Eval)?;
                    // A state cell holds its DECLARED type at every
                    // write, not just at the initializer: a dynamic
                    // value of the wrong shape — or an int leaving
                    // the 32-bit range — is a typed error, never a
                    // silent coercion.
                    if let Some(cell) = node.vars.iter().find(|cell| &cell.name == name)
                        && let Some(message) = conformance_fault(name, &cell.ty, &value)
                    {
                        return Err(Fault::Eval(cel::Error { message }));
                    }
                    scratch.insert(name.clone(), value);
                }
                Step::Emit { port, value } => {
                    let env = bindings(&scratch, &lets)?;
                    if let Some(built) = emitted_content(value, input, &env)? {
                        let index = buffer.next_index(port);
                        let id = ids::emission_id(
                            &node.path,
                            trigger,
                            input.meta().sys().id(),
                            port,
                            index,
                        );
                        let mut sys =
                            MetaSys::emission(id, input.meta().sys(), self.graph_version.0);
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
                                    )?;
                                }
                                if let Some(mime) = built
                                    .mime
                                    .as_deref()
                                    .or_else(|| input.meta().sys().mime_type())
                                {
                                    sys = sys.with_mime_type(mime);
                                }
                                input.data().clone()
                            }
                            BuiltData::Value(value) => {
                                crossing_pins_hold(self.plan, &from, own_pin, &value)?;
                                if let Some(mime) = &built.mime {
                                    sys = sys.with_mime_type(mime.clone());
                                }
                                Data::materialized(to_envelope_value(&value)?)
                            }
                            BuiltData::Envelope(value) => {
                                crossing_pins_hold(
                                    self.plan,
                                    &from,
                                    own_pin,
                                    &from_envelope_value(&value),
                                )?;
                                if let Some(mime) = &built.mime {
                                    sys = sys.with_mime_type(mime.clone());
                                }
                                Data::materialized(value)
                            }
                        };
                        // Writable namespaces travel whole — the
                        // builder's overlays are already folded in.
                        let mut meta = Meta::new(sys).with_app(built.app);
                        for (name, namespace) in built.customs {
                            meta = meta.with_custom(name, namespace);
                        }
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
    /// The outgoing application namespace.
    pub(crate) app: envelope::Namespace,
    /// The outgoing custom namespaces.
    pub(crate) customs: BTreeMap<String, envelope::Namespace>,
    /// An explicit `mime_type =` write, when present.
    pub(crate) mime: Option<String>,
}

/// A builder's payload verdict.
pub(crate) enum BuiltData {
    /// No payload write ran: the input's representation ships as-is.
    Unchanged,
    /// The payload was (re)built as an evaluated CEL value.
    Value(cel::Value),
    /// The payload was built in the ENVELOPE domain — the builders
    /// build `Struct`s, a physical form CEL does not carry.
    Envelope(envelope::Value),
}

fn emitted_content(
    emission: &PlanEmission,
    input: &Envelope,
    env: &BTreeMap<String, cel::Value>,
) -> Result<Option<Built>, Fault> {
    let inherited = |data: BuiltData| Built {
        data,
        app: input.meta().app().clone(),
        customs: input
            .meta()
            .customs()
            .map(|(name, namespace)| (name.clone(), namespace.clone()))
            .collect(),
        mime: None,
    };
    match emission {
        PlanEmission::Nothing => Ok(None),
        PlanEmission::Identity => Ok(Some(inherited(BuiltData::Unchanged))),
        PlanEmission::Data(expr) => {
            let value = cel::eval(expr, env).map_err(Fault::Eval)?;
            Ok(Some(inherited(BuiltData::Value(value))))
        }
        PlanEmission::Patch(writes) | PlanEmission::Replace(writes) => {
            let replace = matches!(emission, PlanEmission::Replace(_));
            let mut built = inherited(BuiltData::Unchanged);
            let mut data: Option<envelope::Value> = if replace {
                // A fresh STRUCT — the physical form the grammar
                // defines: `{ … } ≡ envelope { data = struct { … } }`.
                Some(envelope::Value::Struct(Vec::new()))
            } else {
                None
            };
            for write in writes {
                let value = cel::eval(&write.expr, env).map_err(Fault::Eval)?;
                match &write.target {
                    envelope::WriteTarget::Data(path) => {
                        // Materialize lazily: a metadata-only patch
                        // never touches the payload bytes.
                        let current = match &mut data {
                            Some(current) => current,
                            None => data.insert(match input.data() {
                                Data::Materialized(value) => value.clone(),
                                encoded => to_envelope_value(&data_value(encoded)?)?,
                            }),
                        };
                        apply_struct_write(current, path, to_envelope_value(&value)?);
                    }
                    envelope::WriteTarget::AppMeta(field) => {
                        built.app.insert(field.clone(), to_meta_value(&value)?);
                    }
                    envelope::WriteTarget::CustomMeta(namespace, field) => {
                        built
                            .customs
                            .entry(namespace.clone())
                            .or_default()
                            .insert(field.clone(), to_meta_value(&value)?);
                    }
                    envelope::WriteTarget::MimeType => match value {
                        cel::Value::Text(mime) => built.mime = Some(mime),
                        other => {
                            return Err(Fault::Unrepresentable(format!(
                                "mime_type takes text, got {other:?}"
                            )));
                        }
                    },
                }
            }
            if let Some(value) = data {
                built.data = BuiltData::Envelope(value);
            }
            Ok(Some(built))
        }
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
) -> Result<(), Fault> {
    if let Some(pinned) = own_pin
        && let Some(message) = conformance_fault(from.port.name(), pinned, value)
    {
        return Err(Fault::Eval(cel::Error { message }));
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
            && let Some(message) = conformance_fault(to.port.name(), pinned, value)
        {
            return Err(Fault::Eval(cel::Error { message }));
        }
    }
    Ok(())
}

/// Judges a runtime value against a declared cell type; `None` means
/// it conforms. `Dyn` admits anything — the dynamic escape is chosen
/// in the declaration, not discovered at the write.
pub(crate) fn conformance_fault(
    name: &str,
    declared: &cel::Type,
    value: &cel::Value,
) -> Option<String> {
    let mismatch = |got: &str| {
        Some(format!(
            "`{name}` is declared {declared:?}; a {got} cannot enter it"
        ))
    };
    match (declared, value) {
        (cel::Type::Dyn, _) => None,
        (cel::Type::Int, cel::Value::Int(candidate)) => {
            if i32::try_from(*candidate).is_err() {
                Some(format!(
                    "`{name}` is an int; {candidate} leaves the 32-bit range"
                ))
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
        (_, cel::Value::Bool(_)) => mismatch("bool"),
        (_, cel::Value::Int(_) | cel::Value::Uint(_)) => mismatch("number"),
        (_, cel::Value::Double(_)) => mismatch("double"),
        (_, cel::Value::Text(_)) => mismatch("text"),
        (_, cel::Value::Bytes(_)) => mismatch("bytes"),
        (_, cel::Value::Duration(_)) => mismatch("duration"),
        (_, cel::Value::Timestamp(_)) => mismatch("timestamp"),
        (_, cel::Value::Null) => mismatch("null"),
        (_, cel::Value::List(_)) => mismatch("list"),
        (_, cel::Value::Map(_)) => mismatch("map"),
    }
}

/// Applies one struct-building write along its path — the builders
/// build STRUCTS (`{ … } ≡ envelope { data = struct { … } }`), so a
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

/// The handler-facing CEL view of an envelope's data — the bridge a
/// host layer (lab, adapters, tooling) uses to read what a handler
/// reads.
pub fn data_value(data: &Data) -> Result<cel::Value, Fault> {
    match data {
        Data::Materialized(value) => Ok(from_envelope_value(value)),
        Data::Encoded {
            encoding: envelope::Encoding::Raw,
            bytes,
            ..
        } => match core::str::from_utf8(bytes) {
            Ok(text) => Ok(cel::Value::Text(String::from(text))),
            Err(_) => Ok(cel::Value::Bytes(bytes.clone())),
        },
        Data::Encoded { encoding, .. } => Err(Fault::Opaque(format!(
            "{encoding} decoding is not in this build"
        ))),
    }
}

/// The handler-facing CEL view of an envelope's metadata: the full
/// `meta.sys` spine, the app namespace (aliased at the root), and
/// every custom namespace.
pub fn meta_value(meta: &Meta) -> cel::Value {
    let mut app = BTreeMap::new();
    for (field, value) in meta.app() {
        app.insert(field.clone(), from_meta_value(value));
    }
    // The whole six-field spine — a handler reading any `meta.sys`
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
    sys.insert(
        String::from("mime_type"),
        match meta.sys().mime_type() {
            Some(mime) => cel::Value::Text(String::from(mime)),
            None => cel::Value::Null,
        },
    );
    let mut root = BTreeMap::new();
    // The read alias mirrors the write alias: `meta.x` reads
    // `meta.app.x` — app fields surface at the meta root. Precedence
    // on a name collision: `sys`, `app`, and custom NAMESPACES shadow
    // the alias (they are inserted after), and the shadowed app field
    // stays reachable at its unambiguous `meta.app.x` address.
    for (field, value) in &app {
        root.insert(field.clone(), value.clone());
    }
    root.insert(String::from("app"), cel::Value::Map(app));
    root.insert(String::from("sys"), cel::Value::Map(sys));
    for (name, namespace) in meta.customs() {
        let mut fields = BTreeMap::new();
        for (field, value) in namespace {
            fields.insert(field.clone(), from_meta_value(value));
        }
        root.insert(name.clone(), cel::Value::Map(fields));
    }
    cel::Value::Map(root)
}

pub(crate) fn from_envelope_value(value: &envelope::Value) -> cel::Value {
    match value {
        envelope::Value::Scalar(scalar) => from_scalar(scalar),
        envelope::Value::Struct(fields) => cel::Value::Map(
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
        other @ (cel::Value::Duration(_) | cel::Value::Timestamp(_) | cel::Value::Null) => {
            return Err(Fault::Unrepresentable(format!("{other:?}")));
        }
    })
}

/// Converts a CEL value into a writable-metadata leaf.
pub fn to_meta_value(value: &cel::Value) -> Result<MetaValue, Fault> {
    Ok(match value {
        cel::Value::Bool(v) => MetaValue::Scalar(envelope::Scalar::Bool(*v)),
        cel::Value::Int(v) => MetaValue::Scalar(envelope::Scalar::Int(*v)),
        cel::Value::Double(v) => MetaValue::Scalar(envelope::Scalar::Double(*v)),
        cel::Value::Text(v) => MetaValue::Scalar(envelope::Scalar::Text(v.clone())),
        cel::Value::Bytes(v) => MetaValue::Scalar(envelope::Scalar::Bytes(v.clone())),
        other => {
            return Err(Fault::Unrepresentable(format!(
                "{other:?} in a flat meta namespace"
            )));
        }
    })
}
