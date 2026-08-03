//! The engine — run and simulation alike.
//!
//! The debugger-shaped surface of the blueprint: [`Vm::boot`] once,
//! [`Vm::step`] for exactly one node transaction, [`Vm::drain`] to
//! the stable state. No I/O, no clock, no randomness live here: the
//! world enters through the journal and the boundary traits, and the
//! same machine serves run and simulation.

use crate::boundary::{Dispatcher, InboundSubmission, Intent, Partner};
use crate::envelope::{Data, Envelope, Id};
use crate::journal::{ConsumedPosition, Journal, ids};
use crate::node;
use crate::plan::{Edge, EndpointRef, ExecutionPlan, GraphVersion, NodeKind, Slot};
use crate::runtime::{Delivery, Fault, Mailboxes, PartitionState, TransactionExecutor};
use std::collections::BTreeMap;
use std::fmt;

/// A machine fault; the step that raised it committed nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum VmError {
    /// A transaction fault.
    Fault(Fault),
    /// The journal refused an operation.
    Journal(crate::journal::Error),
    /// The dispatcher raised an integrity fault.
    Dispatch(crate::boundary::DispatchError),
    /// A mailbox refused a delivery — backpressure reached the top.
    MailboxFull {
        /// The refusing node.
        node: node::Path,
        /// The refusing port.
        port: node::Port,
    },
}

impl fmt::Display for VmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fault(fault) => fault.fmt(f),
            Self::Journal(error) => error.fmt(f),
            Self::Dispatch(error) => error.fmt(f),
            Self::MailboxFull { node, port } => {
                write!(f, "the mailbox of `{node}.{port}` is full")
            }
        }
    }
}

impl std::error::Error for VmError {}

/// The run-to-completion scheduler: it owns the mailboxes and picks
/// the next work deterministically — the same journal always replays
/// to the same order.
#[derive(Debug)]
pub struct Scheduler {
    mailboxes: Mailboxes,
}

impl Scheduler {
    fn new(capacity: usize) -> Self {
        Self {
            mailboxes: Mailboxes::new(capacity),
        }
    }

    /// The next delivery to process, deterministically: the first
    /// non-empty queue in `(slot, port)` order.
    fn next(&mut self) -> Option<(Slot, node::Port, Delivery)> {
        let (slot, port) = self.mailboxes.ready().into_iter().next()?;
        let delivery = self.mailboxes.take(slot, &port)?;
        Some((slot, port, delivery))
    }

    /// Whether every queue is drained.
    fn is_stable(&self) -> bool {
        self.mailboxes.is_empty()
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
    /// settles: the outcome transaction consumes this cursor.
    pending: BTreeMap<crate::boundary::CallRef, Delivery>,
    outputs: Vec<Envelope>,
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
            outputs: Vec::new(),
        }
    }

    /// Accepts one submission — the journal's one accepting
    /// transaction — and delivers the born envelope along the
    /// boundary `in` edges.
    ///
    /// # Errors
    ///
    /// Journal refusals and full mailboxes.
    pub fn inject(
        &mut self,
        journal: &mut dyn Journal,
        submission: InboundSubmission,
    ) -> Result<Id, VmError> {
        let committed = journal
            .append_inbound(submission)
            .map_err(VmError::Journal)?;
        let id = committed.envelope.meta().sys().id();
        let boundary_in =
            node::Port::new("in").unwrap_or_else(|_| unreachable!("`in` is a valid port"));
        self.route(
            &EndpointRef {
                node: None,
                port: boundary_in,
            },
            &committed.envelope,
        )?;
        Ok(id)
    }

    /// Executes exactly one node transaction, if any work is ready.
    /// Returns whether a transaction ran.
    ///
    /// # Errors
    ///
    /// A fault leaves the machine exactly as it was — the failed
    /// step committed nothing.
    pub fn step(&mut self, journal: &mut dyn Journal) -> Result<bool, VmError> {
        let Some((slot, port, delivery)) = self.scheduler.next() else {
            return Ok(false);
        };
        match &self.plan.nodes[slot.index()].kind {
            NodeKind::Operator => self.fire_operator(journal, slot, &port, delivery)?,
            NodeKind::Connector(_) => self.open_call(journal, slot, delivery)?,
        }
        Ok(true)
    }

    /// Runs to the stable state: no delivery left anywhere.
    ///
    /// # Errors
    ///
    /// The first faulting step; everything before it committed.
    pub fn drain(&mut self, journal: &mut dyn Journal) -> Result<(), VmError> {
        while self.step(journal)? {}
        Ok(())
    }

    /// One boundary round: the dispatcher starts journaled calls and
    /// settles answers; each won closure becomes the connector's own
    /// outcome transaction, committed and routed.
    ///
    /// # Errors
    ///
    /// Dispatcher integrity faults and journal refusals propagate.
    pub fn pump(
        &mut self,
        journal: &mut dyn Journal,
        partner: &mut dyn Partner,
    ) -> Result<(), VmError> {
        let executor = TransactionExecutor::new(self.plan, self.graph_version);
        let batch = self
            .dispatcher
            .pump(journal, partner)
            .map_err(VmError::Dispatch)?;
        for event in batch.events {
            let Some(request) = self.pending.remove(&event.call_ref) else {
                continue;
            };
            let outcome = executor.connector_outcome(
                &event,
                request.envelope.meta().sys(),
                ConsumedPosition {
                    edge: request.edge.clone(),
                    seq: request.seq,
                },
            );
            journal
                .commit(outcome.record.clone())
                .map_err(VmError::Journal)?;
            let slot = self.slot_of(&event.connector_path);
            for emission in &outcome.record.emissions {
                self.route_from(slot, &emission.port, &emission.envelope)?;
            }
        }
        Ok(())
    }

    /// The envelopes that left the graph through its boundary.
    #[must_use]
    pub fn outputs(&self) -> &[Envelope] {
        &self.outputs
    }

    /// Whether the machine is stable: nothing queued, no open call
    /// held.
    #[must_use]
    pub fn is_stable(&self) -> bool {
        self.scheduler.is_stable() && self.pending.is_empty()
    }

    fn fire_operator(
        &mut self,
        journal: &mut dyn Journal,
        slot: Slot,
        port: &node::Port,
        delivery: Delivery,
    ) -> Result<(), VmError> {
        let executor = TransactionExecutor::new(self.plan, self.graph_version);
        let trigger = node::Trigger::Port(port.clone());
        let state = self.state.cell(self.plan, slot, "");
        let outcome = executor
            .fire(slot, &trigger, &delivery, &state)
            .map_err(VmError::Fault)?;
        journal
            .commit(outcome.record.clone())
            .map_err(VmError::Journal)?;
        self.state.apply(self.plan, slot, "", &outcome.state);
        for emission in &outcome.record.emissions {
            self.route_from(slot, &emission.port, &emission.envelope)?;
        }
        Ok(())
    }

    /// A delivery reaching a connector opens its outbound call:
    /// journal-before-effect — the open fact exists before any
    /// partner hears — and the request is held until the call
    /// settles.
    fn open_call(
        &mut self,
        journal: &mut dyn Journal,
        slot: Slot,
        delivery: Delivery,
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
        let request_digest = request_digest(delivery.envelope.data());
        let intent = Intent {
            call_ref: ids::call_ref(emitting_id, &planned.path, &request_digest),
            emitting_id,
            connector_path: planned.path.clone(),
            request_digest,
            idempotency_key: None,
            effect,
        };
        let call_ref = intent.call_ref;
        journal.open_outbound(intent).map_err(VmError::Journal)?;
        self.pending.insert(call_ref, delivery);
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

    /// Routes one emission along every edge leaving `(slot, port)`.
    fn route_from(
        &mut self,
        slot: Slot,
        port: &node::Port,
        envelope: &Envelope,
    ) -> Result<(), VmError> {
        self.route(
            &EndpointRef {
                node: Some(slot),
                port: port.clone(),
            },
            envelope,
        )
    }

    fn route(&mut self, from: &EndpointRef, envelope: &Envelope) -> Result<(), VmError> {
        let edges: Vec<Edge> = self
            .plan
            .edges
            .iter()
            .filter(|edge| edge.from == *from)
            .cloned()
            .collect();
        for edge in edges {
            match &edge.to {
                None => {} // `-> nothing`: the explicit, written discard
                Some(to) => match to.node {
                    None => self.outputs.push(envelope.clone()),
                    Some(target) => {
                        let edge_text = self.edge_text(&edge);
                        self.scheduler
                            .mailboxes
                            .deliver(target, to.port.clone(), edge_text, envelope.clone())
                            .map_err(|_| VmError::MailboxFull {
                                node: self.plan.nodes[target.index()].path.clone(),
                                port: to.port.clone(),
                            })?;
                    }
                },
            }
        }
        Ok(())
    }

    /// The canonical edge rendering — the same text the consumed
    /// cursor seals.
    fn edge_text(&self, edge: &Edge) -> String {
        let end = |endpoint: &EndpointRef| match endpoint.node {
            Some(slot) => format!("{}.{}", self.plan.nodes[slot.index()].path, endpoint.port),
            None => format!(".{}", endpoint.port),
        };
        let to = match &edge.to {
            Some(endpoint) => end(endpoint),
            None => String::from("!discard"),
        };
        format!("{}>{to}", end(&edge.from))
    }
}

/// The integrity seal over the request content: blake3 of the tagged
/// payload bytes. Encoded data seals its original bytes; materialized
/// data seals a canonical framing of its value.
fn request_digest(data: &Data) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    match data {
        Data::Encoded {
            encoding, bytes, ..
        } => {
            hasher.update(&[1, encoding_tag(*encoding)]);
            hasher.update(bytes);
        }
        Data::Materialized(value) => {
            hasher.update(&[2]);
            hash_value(&mut hasher, value);
        }
    }
    let mut digest = [0u8; 32];
    hasher.finalize_xof().fill(&mut digest);
    digest
}

fn encoding_tag(encoding: crate::envelope::Encoding) -> u8 {
    match encoding {
        crate::envelope::Encoding::Raw => 1,
        crate::envelope::Encoding::Avro => 2,
        crate::envelope::Encoding::ArrowIpc => 3,
    }
}

/// A minimal injective framing of a materialized value — tags and
/// length prefixes, the same discipline as the id recipes.
fn hash_value(hasher: &mut blake3::Hasher, value: &crate::envelope::Value) {
    use crate::envelope::{Scalar, Value};
    let write_len = |hasher: &mut blake3::Hasher, len: usize| {
        hasher.update(&u32::try_from(len).unwrap_or(u32::MAX).to_le_bytes());
    };
    match value {
        Value::Scalar(Scalar::Bool(v)) => {
            hasher.update(&[1, u8::from(*v)]);
        }
        Value::Scalar(Scalar::Int(v)) => {
            hasher.update(&[2]);
            hasher.update(&v.to_le_bytes());
        }
        Value::Scalar(Scalar::Double(v)) => {
            hasher.update(&[3]);
            hasher.update(&v.to_bits().to_le_bytes());
        }
        Value::Scalar(Scalar::Text(v)) => {
            hasher.update(&[4]);
            write_len(hasher, v.len());
            hasher.update(v.as_bytes());
        }
        Value::Scalar(Scalar::Bytes(v)) => {
            hasher.update(&[5]);
            write_len(hasher, v.len());
            hasher.update(v);
        }
        Value::Struct(fields) => {
            hasher.update(&[6]);
            write_len(hasher, fields.len());
            for (name, field) in fields {
                write_len(hasher, name.len());
                hasher.update(name.as_bytes());
                hash_value(hasher, field);
            }
        }
        Value::Array(items) => {
            hasher.update(&[7]);
            write_len(hasher, items.len());
            for item in items {
                hash_value(hasher, item);
            }
        }
        Value::Map(map) => {
            hasher.update(&[8]);
            write_len(hasher, map.len());
            for (key, item) in map {
                write_len(hasher, key.len());
                hasher.update(key.as_bytes());
                hash_value(hasher, item);
            }
        }
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
      out = envelope { data.text = data.text }
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
        // routes err in the probe, so nothing reaches the boundary —
        // but the transaction is journaled with the typed payload.
        assert!(vm.outputs().is_empty());
        let last = journal.records().last().unwrap();
        let Record::Transaction(transaction) = last else {
            panic!("expected the outcome transaction");
        };
        assert_eq!(transaction.emissions[0].port.name(), "err");
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
