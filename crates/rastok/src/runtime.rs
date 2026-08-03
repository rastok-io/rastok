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
use crate::plan::{Emission as PlanEmission, ExecutionPlan, GraphVersion, Handler, Slot, Step};
use std::collections::{BTreeMap, VecDeque};
use std::fmt;

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
        }
    }
}

impl std::error::Error for Fault {}

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
    /// applied from the plan on first access.
    #[must_use]
    pub fn cell(
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

    /// Applies a committed delta to a cell.
    pub fn apply(
        &mut self,
        plan: &ExecutionPlan,
        slot: Slot,
        key: &str,
        delta: &[(String, cel::Value)],
    ) {
        let mut cell = self.cell(plan, slot, key);
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
    claimed: std::collections::BTreeSet<(Slot, Key)>,
    edge_seq: BTreeMap<String, u64>,
}

impl Mailboxes {
    /// Mailboxes bounded at `capacity` deliveries per queue.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            queues: BTreeMap::new(),
            claimed: std::collections::BTreeSet::new(),
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
    /// An executor over a compiled plan.
    #[must_use]
    pub fn new(plan: &'plan ExecutionPlan, graph_version: GraphVersion) -> Self {
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
        state: &BTreeMap<String, cel::Value>,
    ) -> Result<TransactionOutcome, Fault> {
        let node = &self.plan.nodes[slot.index()];
        let trigger = &node::Trigger::Port(delivery.port.clone());
        let handler = self
            .plan
            .handlers
            .iter()
            .find(|handler| handler.node == slot && handler.trigger == *trigger)
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
                    scratch.insert(name.clone(), value);
                }
                Step::Emit { port, value } => {
                    let env = bindings(&scratch, &lets)?;
                    if let Some(data_and_app) = emitted_content(value, input, &env)? {
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
                        let (data, app) = data_and_app;
                        // The identity emission re-ships the input's
                        // content, so its content-type hint still
                        // holds; recomputed data drops it.
                        if matches!(value, PlanEmission::Identity)
                            && let Some(mime) = input.meta().sys().mime_type()
                        {
                            sys = sys.with_mime_type(mime);
                        }
                        // Writable namespaces are inherited whole; a
                        // single-input handler never silently drops
                        // what the ancestry attached.
                        let mut meta = Meta::new(sys).with_app(app);
                        for (name, namespace) in input.meta().customs() {
                            meta = meta.with_custom(name.clone(), namespace.clone());
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
        Ok(TransactionOutcome {
            record: TransactionRecord {
                node_path: node.path.clone(),
                trigger: trigger.clone(),
                consumed: vec![consumed],
                state_delta,
                emissions: buffer.into_emissions(),
            },
        })
    }

    /// The typed-error transaction for a faulted handler: the input
    /// is consumed causally and the fault leaves on `err` as a typed
    /// payload — a machine never swallows an envelope.
    #[must_use]
    pub fn fault_transaction(
        &self,
        slot: Slot,
        delivery: &Delivery,
        fault: &Fault,
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
            envelope::Value::Scalar(envelope::Scalar::Text(String::from("RuntimeFault"))),
        );
        payload.insert(
            String::from("message"),
            envelope::Value::Scalar(envelope::Scalar::Text(fault.to_string())),
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
                emissions: vec![Emission {
                    port: event.port.clone(),
                    index: 0,
                    envelope,
                }],
            },
        }
    }

    /// The handler registered for a trigger, if any — the scheduler
    /// asks before consuming a delivery.
    #[must_use]
    pub fn handler_for(&self, slot: Slot, trigger: &node::Trigger) -> Option<&'plan Handler> {
        self.plan
            .handlers
            .iter()
            .find(|handler| handler.node == slot && handler.trigger == *trigger)
    }
}

/// What one emission builds: the data and the outgoing `meta.app`
/// namespace. `None` means `nothing` — no emission on this port.
type EmittedContent = Option<(Data, envelope::Namespace)>;

fn emitted_content(
    emission: &PlanEmission,
    input: &Envelope,
    env: &BTreeMap<String, cel::Value>,
) -> Result<EmittedContent, Fault> {
    match emission {
        PlanEmission::Nothing => Ok(None),
        PlanEmission::Identity => Ok(Some((input.data().clone(), input.meta().app().clone()))),
        PlanEmission::Data(expr) => {
            let value = cel::eval(expr, env).map_err(Fault::Eval)?;
            Ok(Some((
                Data::materialized(to_envelope_value(&value)?),
                input.meta().app().clone(),
            )))
        }
        PlanEmission::Patch(writes) => {
            let mut data = data_value(input.data())?;
            let mut app = input.meta().app().clone();
            for write in writes {
                let value = cel::eval(&write.expr, env).map_err(Fault::Eval)?;
                apply_write(&mut data, &mut app, &write.path, value)?;
            }
            Ok(Some((Data::materialized(to_envelope_value(&data)?), app)))
        }
        PlanEmission::Replace(writes) => {
            let mut data = cel::Value::Map(BTreeMap::new());
            let mut app = input.meta().app().clone();
            for write in writes {
                let value = cel::eval(&write.expr, env).map_err(Fault::Eval)?;
                apply_write(&mut data, &mut app, &write.path, value)?;
            }
            Ok(Some((Data::materialized(to_envelope_value(&data)?), app)))
        }
    }
}

/// Applies one field write. `data.x` and bare `x` land in the data
/// value; `meta.app.x` and `meta.<custom>.x` land in metadata —
/// custom namespaces are deferred with the checker, so only `app`
/// arrives here.
fn apply_write(
    data: &mut cel::Value,
    app: &mut envelope::Namespace,
    path: &[String],
    value: cel::Value,
) -> Result<(), Fault> {
    let (root, rest): (&str, &[String]) = match path.first().map(String::as_str) {
        Some("data") => ("data", &path[1..]),
        Some("meta") => ("meta", &path[1..]),
        _ => ("data", path),
    };
    if root == "meta" {
        // The checker admits meta.app.* only.
        let field = rest
            .get(1)
            .ok_or_else(|| Fault::Unrepresentable(String::from("a bare meta write")))?;
        app.insert(field.clone(), to_meta_value(&value)?);
        return Ok(());
    }
    if rest.is_empty() {
        *data = value;
        return Ok(());
    }
    let mut current = data;
    for segment in &rest[..rest.len() - 1] {
        if !matches!(current, cel::Value::Map(_)) {
            *current = cel::Value::Map(BTreeMap::new());
        }
        let cel::Value::Map(map) = current else {
            unreachable!("just ensured a map");
        };
        current = map
            .entry(segment.clone())
            .or_insert_with(|| cel::Value::Map(BTreeMap::new()));
    }
    if !matches!(current, cel::Value::Map(_)) {
        *current = cel::Value::Map(BTreeMap::new());
    }
    let cel::Value::Map(map) = current else {
        unreachable!("just ensured a map");
    };
    map.insert(rest[rest.len() - 1].clone(), value);
    Ok(())
}

/// The handler-facing view of an envelope's data.
fn data_value(data: &Data) -> Result<cel::Value, Fault> {
    match data {
        Data::Materialized(value) => Ok(from_envelope_value(value)),
        Data::Encoded {
            encoding: envelope::Encoding::Raw,
            bytes,
            ..
        } => match std::str::from_utf8(bytes) {
            Ok(text) => Ok(cel::Value::Text(String::from(text))),
            Err(_) => Ok(cel::Value::Bytes(bytes.clone())),
        },
        Data::Encoded { encoding, .. } => Err(Fault::Opaque(format!(
            "{encoding} decoding is not in this build"
        ))),
    }
}

/// The handler-facing view of an envelope's metadata.
fn meta_value(meta: &Meta) -> cel::Value {
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

fn from_envelope_value(value: &envelope::Value) -> cel::Value {
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

fn to_envelope_value(value: &cel::Value) -> Result<envelope::Value, Fault> {
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

fn to_meta_value(value: &cel::Value) -> Result<MetaValue, Fault> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::TypedError;
    use crate::compiler::compile;
    use crate::envelope::Encoding;
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
      out = envelope { data.text = data.text }
    }
  }

  . -> shout -> echo -> .
}
"#;

    fn probe() -> (ExecutionPlan, Slot) {
        let compilation = compile(&[M0_PROBE], &Registry::m0()).expect("a valid plan");
        let slot = Slot::new(
            u32::try_from(
                compilation
                    .symbols
                    .nodes
                    .iter()
                    .position(|name| name == "shout")
                    .unwrap(),
            )
            .unwrap(),
        );
        (compilation.plan, slot)
    }

    fn input(text: &str) -> Delivery {
        let mut data = BTreeMap::new();
        data.insert(
            String::from("text"),
            envelope::Value::Scalar(envelope::Scalar::Text(String::from(text))),
        );
        let id = envelope::Id::derive(0x01, &[b"inbound", &0u64.to_le_bytes()]);
        Delivery {
            port: node::Port::new("in").unwrap(),
            edge: String::from(".in>shout.in"),
            seq: 1,
            envelope: Envelope::new(
                Meta::new(MetaSys::root(id, 1)),
                Data::materialized(envelope::Value::Map(data)),
            ),
        }
    }

    fn trigger_in() -> node::Trigger {
        node::Trigger::Port(node::Port::new("in").unwrap())
    }

    #[test]
    fn an_identity_emission_inherits_everything_but_its_spine() {
        let source = "graph g : App {\n  operator relay {\n    on in { out = envelope }\n  }\n  . -> relay -> .\n}\n";
        let compilation = compile(&[source], &Registry::m0()).expect("a valid plan");
        let executor = TransactionExecutor::new(&compilation.plan, GraphVersion(1));
        let slot = Slot::new(0);
        let state = PartitionState::new().cell(&compilation.plan, slot, "");
        // An ancestry-rich input: app fields, a custom namespace, a
        // content-type hint on unchanged encoded data.
        let id = envelope::Id::derive(0x01, &[b"inbound", &0u64.to_le_bytes()]);
        let mut app = envelope::Namespace::new();
        app.insert(
            String::from("label"),
            envelope::MetaValue::Scalar(envelope::Scalar::Text(String::from("greeting"))),
        );
        let mut tenant = envelope::Namespace::new();
        tenant.insert(
            String::from("org"),
            envelope::MetaValue::Scalar(envelope::Scalar::Text(String::from("acme"))),
        );
        let meta = Meta::new(MetaSys::root(id, 1).with_mime_type("text/plain"))
            .with_app(app.clone())
            .with_custom("tenant", tenant.clone());
        let delivery = Delivery {
            port: node::Port::new("in").unwrap(),
            edge: String::from(".in>relay.in"),
            seq: 1,
            envelope: Envelope::new(meta, Data::encoded(Encoding::Raw, *b"salut")),
        };
        let outcome = executor.fire(slot, &delivery, &state).unwrap();
        let emitted = &outcome.record.emissions[0].envelope;
        let sys = emitted.meta().sys();
        let parent = delivery.envelope.meta().sys();
        // A fresh spine…
        assert_ne!(sys.id(), parent.id());
        assert_eq!(sys.parent_id(), Some(parent.id()));
        assert_eq!(sys.origin_id(), parent.origin_id());
        // …carrying everything the ancestry attached.
        assert_eq!(emitted.meta().app(), &app);
        assert_eq!(emitted.meta().custom("tenant"), Some(&tenant));
        assert_eq!(
            sys.mime_type(),
            Some("text/plain"),
            "unchanged content keeps its content-type hint"
        );
        assert_eq!(emitted.data(), delivery.envelope.data());
    }

    #[test]
    fn the_meta_projection_carries_the_whole_spine() {
        let id = envelope::Id::derive(0x01, &[b"inbound", &0u64.to_le_bytes()]);
        let mut tenant = envelope::Namespace::new();
        tenant.insert(
            String::from("org"),
            envelope::MetaValue::Scalar(envelope::Scalar::Text(String::from("acme"))),
        );
        let meta = Meta::new(MetaSys::root(id, 3).with_mime_type("text/plain"))
            .with_custom("tenant", tenant);
        let cel::Value::Map(root) = meta_value(&meta) else {
            panic!("expected a map projection");
        };
        let cel::Value::Map(sys) = &root["sys"] else {
            panic!("expected a sys map");
        };
        assert_eq!(sys["id"], cel::Value::Text(id.to_string()));
        assert_eq!(sys["parent_id"], cel::Value::Null, "a root has no parent");
        assert_eq!(sys["role"], cel::Value::Text(String::from("default")));
        assert_eq!(sys["graph_version"], cel::Value::Int(3));
        assert_eq!(
            sys["mime_type"],
            cel::Value::Text(String::from("text/plain"))
        );
        let cel::Value::Map(tenant) = &root["tenant"] else {
            panic!("custom namespaces project");
        };
        assert_eq!(tenant["org"], cel::Value::Text(String::from("acme")));
    }

    #[test]
    fn a_transaction_commits_everything_together() {
        let (plan, slot) = probe();
        let executor = TransactionExecutor::new(&plan, GraphVersion(1));
        let state = PartitionState::new().cell(&plan, slot, "");
        assert_eq!(state.get("count"), Some(&cel::Value::Int(0)));
        let delivery = input("salut");
        let outcome = executor.fire(slot, &delivery, &state).unwrap();
        // The commit packet: consumed cursor, state delta, emission.
        assert_eq!(outcome.record.consumed[0].edge, ".in>shout.in");
        assert_eq!(outcome.record.consumed[0].seq, 1);
        assert_eq!(
            outcome.record.state_delta,
            vec![(String::from("count"), cel::Value::Int(1))]
        );
        assert_eq!(outcome.record.emissions.len(), 1);
        let emission = &outcome.record.emissions[0];
        assert_eq!(emission.port.name(), "out");
        assert_eq!(emission.index, 0);
        // The emitted envelope follows the recipes: derived id,
        // parented on the input, origin inherited.
        let parent = delivery.envelope.meta().sys();
        let expected = ids::emission_id(
            &node::Path::new(["shout"]).unwrap(),
            &trigger_in(),
            parent.id(),
            &node::Port::new("out").unwrap(),
            0,
        );
        let sys = emission.envelope.meta().sys();
        assert_eq!(sys.id(), expected);
        assert_eq!(sys.parent_id(), Some(parent.id()));
        assert_eq!(sys.origin_id(), parent.origin_id());
    }

    #[test]
    fn execution_is_deterministic() {
        let (plan, slot) = probe();
        let executor = TransactionExecutor::new(&plan, GraphVersion(1));
        let state = PartitionState::new().cell(&plan, slot, "");
        let delivery = input("salut");
        let once = executor.fire(slot, &delivery, &state).unwrap();
        let twice = executor.fire(slot, &delivery, &state).unwrap();
        assert_eq!(once, twice, "same inputs, same transaction, same ids");
    }

    #[test]
    fn a_false_guard_consumes_without_effect() {
        let (plan, slot) = probe();
        let executor = TransactionExecutor::new(&plan, GraphVersion(1));
        let state = PartitionState::new().cell(&plan, slot, "");
        let outcome = executor.fire(slot, &input(""), &state).unwrap();
        assert!(outcome.record.emissions.is_empty());
        assert!(outcome.record.state_delta.is_empty());
        assert_eq!(
            outcome.record.consumed.len(),
            1,
            "the cursor still advances"
        );
    }

    #[test]
    fn a_faulted_transaction_changes_nothing() {
        let (plan, slot) = probe();
        let executor = TransactionExecutor::new(&plan, GraphVersion(1));
        let state = PartitionState::new().cell(&plan, slot, "");
        // Avro payloads cannot be materialized in this build: the
        // handler faults, and the caller applies nothing.
        let delivery = Delivery {
            port: node::Port::new("in").unwrap(),
            edge: String::from(".in>shout.in"),
            seq: 1,
            envelope: Envelope::new(
                Meta::new(MetaSys::root(envelope::Id::from_bytes([1; 16]), 1)),
                Data::encoded(Encoding::Avro, *b"opaque"),
            ),
        };
        let fault = executor.fire(slot, &delivery, &state).unwrap_err();
        assert!(matches!(fault, Fault::Opaque(_)));
        assert_eq!(state.get("count"), Some(&cel::Value::Int(0)), "untouched");
    }

    #[test]
    fn state_is_isolated_between_keys() {
        let (plan, slot) = probe();
        let mut partitions = PartitionState::new();
        partitions.apply(
            &plan,
            slot,
            "alice",
            &[(String::from("count"), cel::Value::Int(7))],
        );
        assert_eq!(
            partitions.cell(&plan, slot, "alice").get("count"),
            Some(&cel::Value::Int(7))
        );
        assert_eq!(
            partitions.cell(&plan, slot, "bob").get("count"),
            Some(&cel::Value::Int(0)),
            "bob's cell starts from the plan defaults"
        );
    }

    #[test]
    fn mailboxes_are_fifo_bounded_and_edge_sequenced() {
        let (plan, slot) = probe();
        let _ = &plan;
        let mut mailboxes = Mailboxes::new(2);
        let port = node::Port::new("in").unwrap();
        let envelope = input("a").envelope;
        mailboxes
            .deliver(slot, "", port.clone(), String::from("e"), envelope.clone())
            .unwrap();
        mailboxes
            .deliver(slot, "", port.clone(), String::from("e"), envelope.clone())
            .unwrap();
        // Bounded: the third delivery is refused, not dropped.
        assert!(!mailboxes.has_room(slot, "", 0));
        assert!(
            mailboxes
                .deliver(slot, "", port.clone(), String::from("e"), envelope.clone())
                .is_err()
        );
        // Peek reserves without removing; pop removes.
        let head = mailboxes.peek_next().unwrap();
        assert_eq!(head.2.seq, 1);
        let first = mailboxes.pop(slot, "").unwrap();
        let second = mailboxes.pop(slot, "").unwrap();
        assert_eq!(first.seq, 1);
        assert_eq!(second.seq, 2, "per-edge sequence, FIFO order");
        assert!(mailboxes.pop(slot, "").is_none());
        assert!(mailboxes.is_empty());
    }

    #[test]
    fn one_node_mailbox_keeps_arrival_order_across_ports() {
        let (plan, slot) = probe();
        let _ = &plan;
        let mut mailboxes = Mailboxes::new(8);
        let z = node::Port::new("z").unwrap();
        let a = node::Port::new("a").unwrap();
        let envelope = input("x").envelope;
        mailboxes
            .deliver(slot, "", z.clone(), String::from("ez"), envelope.clone())
            .unwrap();
        mailboxes
            .deliver(slot, "", a.clone(), String::from("ea"), envelope.clone())
            .unwrap();
        // One FIFO per node instance: `z` arrived first, `z` fires
        // first — the port never reorders shared-state work.
        assert_eq!(mailboxes.pop(slot, "").unwrap().port, z);
        assert_eq!(mailboxes.pop(slot, "").unwrap().port, a);
    }

    #[test]
    fn a_claimed_mailbox_is_skipped_until_released() {
        let (plan, slot) = probe();
        let _ = &plan;
        let mut mailboxes = Mailboxes::new(8);
        let port = node::Port::new("in").unwrap();
        mailboxes
            .deliver(slot, "", port, String::from("e"), input("x").envelope)
            .unwrap();
        mailboxes.claim(slot, "");
        assert!(
            mailboxes.peek_next().is_none(),
            "a suspended call keeps its mailbox claimed"
        );
        mailboxes.release(slot, "");
        assert!(mailboxes.peek_next().is_some());
    }

    #[test]
    fn the_connector_outcome_matches_the_reference_trace() {
        let (plan, _) = probe();
        let executor = TransactionExecutor::new(&plan, GraphVersion(1));
        let request = input("salut").envelope;
        let connector_path = node::Path::new(["echo"]).unwrap();
        let event = BoundaryEvent {
            call_ref: ids::call_ref(request.meta().sys().id(), &connector_path, &[7; 32]),
            connector_path: connector_path.clone(),
            emitting_id: request.meta().sys().id(),
            port: node::Port::new("err").unwrap(),
            outcome: CallOutcome::Failure(TypedError {
                kind: String::from("ExecFailed"),
                data: Data::encoded(Encoding::Raw, *b"exit 1"),
            }),
        };
        let outcome = executor.connector_outcome(
            &event,
            &node::Trigger::Port(node::Port::new("in").unwrap()),
            request.meta().sys(),
            ConsumedPosition {
                edge: String::from("shout.out>echo.in"),
                seq: 1,
            },
        );
        assert_eq!(outcome.record.node_path, connector_path);
        assert_eq!(outcome.record.trigger.to_string(), "p:in");
        let emission = &outcome.record.emissions[0];
        assert_eq!(emission.port.name(), "err");
        let expected = ids::emission_id(
            &connector_path,
            &outcome.record.trigger,
            request.meta().sys().id(),
            &node::Port::new("err").unwrap(),
            0,
        );
        assert_eq!(emission.envelope.meta().sys().id(), expected);
        assert_eq!(
            emission.envelope.meta().sys().origin_id(),
            request.meta().sys().origin_id()
        );
    }
}
