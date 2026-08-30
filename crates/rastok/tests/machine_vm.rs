//! The Vm end to end, driven through compiled probes — integration
//! tests of the core machine, living in the std crate because they
//! need the compiler.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
// Harness helpers live outside `#[test]` bodies, where the
// allow-unwrap-in-tests toggle does not reach; a broken fixture's
// correct report is a panic.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use rastok::boundary::{
    CallConnectorDriver, CallStart, DriverRouteError, InboundAdmission, InboundExchangeRef,
    InboundProposal, InboundProposalRef, InboundSession, InboundSubmission, Intent,
    OutboundConnectorBindings, PreparedRequest, SessionDriverEvent, SessionOpenIntent, SessionRef,
    SessionStart, SessionWriteIntent, SessionWriteRef, WebSocketExit,
};
use rastok::envelope::{Data, Envelope};
use rastok::identity::{InstanceAddress, InstanceFrame, InstanceRef};
use rastok::journal::Journal;
use rastok::node;
use rastok::plan::{
    AttrSetting, EvaluationSite, GraphVersion, OwnerPath, OwnerValue, ValidatedExecutionPlan,
};
use rastok::runtime::vm::{Vm, VmError};

use rastok::boundary::{
    Completion, Draft, InboundEnvelopeProvenance, PortOutput, ProcessExit, RequestPreparation,
    Response, SchedulerCompletion, SubmissionSequence,
};
use rastok::compiler::compile;
use rastok::contract::ContractCatalog;
use rastok::envelope::{Encoding, Namespace, Value};
use rastok::journal::{CloseVerdict, MemJournal, OutboundRecord, OutboundStatus, Record};

fn int_address(owner: &str, value: i64) -> InstanceAddress {
    let owner = OwnerPath::new([owner]).expect("valid owner path");
    InstanceAddress::new(
        &owner,
        &[InstanceFrame::SelectedKey {
            owner: owner.clone(),
            ty: rastok::datatype::DataType::INT,
            value: Value::Int(value),
        }],
    )
    .expect("int is a canonical instance key")
}

const M0_PROBE: &str = r#"
graph m0_probe : App {
  connector echo : Process {
mode = job ; command = "cat"
codec = lines
effect = idempotent ; behavior = deterministic
  }

  operator shout {
input in: struct { text: text }
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

struct ProjectionRefusal {
    inner: EchoWorld,
    refuse_scheduler_completion: bool,
    refuse_cancel: bool,
}

impl OutboundConnectorBindings for ProjectionRefusal {
    fn prepare_request(
        &mut self,
        connector: &node::Path,
        data: &Data,
        resolved_settings: &std::collections::BTreeMap<String, rastok::value::Value>,
    ) -> Result<RequestPreparation, DriverRouteError> {
        Ok(CallConnectorDriver::prepare_request(
            &mut self.inner,
            connector,
            data,
            resolved_settings,
        ))
    }

    fn scheduler_completion(
        &mut self,
        connector: &node::Path,
        completion: SchedulerCompletion,
    ) -> Result<Completion, DriverRouteError> {
        if self.refuse_scheduler_completion {
            return Err(DriverRouteError::UnavailableOutbound(connector.clone()));
        }
        Ok(CallConnectorDriver::scheduler_completion(
            &mut self.inner,
            connector,
            completion,
        ))
    }

    fn cancel(
        &mut self,
        connector: &node::Path,
        call_ref: &rastok::boundary::CallRef,
    ) -> Result<(), DriverRouteError> {
        if self.refuse_cancel {
            return Err(DriverRouteError::UnavailableOutbound(connector.clone()));
        }
        CallConnectorDriver::cancel(&mut self.inner, call_ref);
        Ok(())
    }

    fn start(
        &mut self,
        intent: &Intent,
        request: &Envelope,
    ) -> Result<CallStart, DriverRouteError> {
        Ok(CallConnectorDriver::start(&mut self.inner, intent, request))
    }

    fn events(&mut self) -> Result<Vec<Response>, DriverRouteError> {
        Ok(CallConnectorDriver::events(&mut self.inner))
    }
}

/// A boundary seam that records deterministic cancellation order. It can
/// either confirm each cancellation with an observed normal exit or remain
/// silent so bounded settlement has to mint `OutcomeUnknown`.
#[derive(Default)]
struct ClosingWorld {
    started: Vec<rastok::boundary::CallRef>,
    cancelled: Vec<rastok::boundary::CallRef>,
    answers: Vec<Response>,
    confirm_cancel: bool,
}

impl CallConnectorDriver for ClosingWorld {
    fn prepare_request(
        &mut self,
        _: &node::Path,
        data: &Data,
        _: &std::collections::BTreeMap<String, rastok::value::Value>,
    ) -> RequestPreparation {
        rastok::boundary::prepare_process_request(rastok::boundary::RequestCodec::Lines, data)
    }

    fn scheduler_completion(
        &mut self,
        _: &node::Path,
        completion: SchedulerCompletion,
    ) -> Completion {
        rastok::boundary::process_scheduler_completion(completion)
    }

    fn start(&mut self, intent: &Intent, _: &Envelope) -> CallStart {
        self.started.push(intent.call_ref);
        CallStart::Started
    }

    fn cancel(&mut self, call_ref: &rastok::boundary::CallRef) {
        self.cancelled.push(*call_ref);
        if self.confirm_cancel {
            self.answers.push(Response {
                call_ref: *call_ref,
                completion: Completion::process(
                    vec![],
                    vec![],
                    ProcessExit::Exited {
                        exit_code: -1,
                        signal: Some(15),
                    },
                ),
            });
        }
    }

    fn events(&mut self) -> Vec<Response> {
        core::mem::take(&mut self.answers)
    }
}

const CLOSING_PROCESS: &str = r#"
graph bounded_closing : App {
  input in: text
  output exit
  lifetime = { kind = fixed ; after = duration("1s") }
  closing = { drain_for = duration("1s") ; settle_for = duration("1s") }

  connector shell : Process {
    mode = job
    command = "cat"
    codec = lines
    effect = idempotent
    behavior = deterministic
  }

  . -> shell
  shell.out -> nothing
  shell.err -> nothing
  shell.exit -> .exit
}
"#;

impl CallConnectorDriver for EchoWorld {
    fn prepare_request(
        &mut self,
        _: &node::Path,
        data: &Data,
        _: &std::collections::BTreeMap<String, rastok::value::Value>,
    ) -> RequestPreparation {
        rastok::boundary::prepare_process_request(rastok::boundary::RequestCodec::Lines, data)
    }

    fn scheduler_completion(
        &mut self,
        _: &node::Path,
        completion: SchedulerCompletion,
    ) -> Completion {
        rastok::boundary::process_scheduler_completion(completion)
    }

    fn start(&mut self, intent: &Intent, request: &Envelope) -> CallStart {
        let _ = request;
        self.seen.push(intent.clone());
        CallStart::Started
    }

    fn events(&mut self) -> Vec<Response> {
        self.seen
            .drain(..)
            .map(|intent| Response {
                call_ref: intent.call_ref,
                completion: if self.fail_next {
                    Completion::process(
                        vec![],
                        vec![Data::materialized(Value::Text(String::from("exit 1")))],
                        ProcessExit::Exited {
                            exit_code: 1,
                            signal: None,
                        },
                    )
                } else {
                    Completion::process(
                        vec![Data::encoded(Encoding::Raw, *b"salut")],
                        vec![],
                        ProcessExit::Exited {
                            exit_code: 0,
                            signal: None,
                        },
                    )
                },
            })
            .collect()
    }
}

#[derive(Default)]
struct FileSettingsWorld {
    prepared: Vec<std::collections::BTreeMap<String, Value>>,
}

impl CallConnectorDriver for FileSettingsWorld {
    fn prepare_request(
        &mut self,
        _: &node::Path,
        _: &Data,
        settings: &std::collections::BTreeMap<String, Value>,
    ) -> RequestPreparation {
        self.prepared.push(settings.clone());
        RequestPreparation::Prepared(rastok::boundary::PreparedRequest::seal(Vec::new()))
    }

    fn scheduler_completion(&mut self, _: &node::Path, _: SchedulerCompletion) -> Completion {
        Completion::try_new(vec![PortOutput {
            port: node::Port::conventional_exit(),
            data: Data::materialized(Value::TaggedUnion(Box::new(
                rastok::value::TaggedUnionValue {
                    case: String::from("Completed"),
                    payload: Vec::new(),
                },
            ))),
            ext: rastok::envelope::Namespace::new(),
        }])
        .expect("the File terminal fact is structurally complete")
    }

    fn start(&mut self, _: &Intent, _: &Envelope) -> CallStart {
        CallStart::Started
    }

    fn events(&mut self) -> Vec<Response> {
        Vec::new()
    }
}

fn submission(text: &str) -> InboundSubmission {
    let data = vec![(String::from("text"), Value::Text(String::from(text)))];
    InboundSubmission {
        provenance: InboundEnvelopeProvenance::single(node::Path::new(["cli"]).unwrap()),
        port: rastok::node::Port::new("in").unwrap(),
        draft: Draft {
            data: Data::materialized(Value::Struct(data)),
            ext: Namespace::new(),
        },
        sequence: SubmissionSequence::Next,
    }
}

fn text_submission(text: &str) -> InboundSubmission {
    InboundSubmission {
        provenance: InboundEnvelopeProvenance::single(node::Path::new(["cli"]).unwrap()),
        port: rastok::node::Port::new("in").unwrap(),
        draft: Draft {
            data: Data::materialized(Value::Text(String::from(text))),
            ext: Namespace::new(),
        },
        sequence: SubmissionSequence::Next,
    }
}

fn websocket_proposal(reference: u64, text: &str, session: InboundSession) -> InboundProposal {
    InboundProposal {
        reference: InboundProposalRef::new(reference),
        submission: InboundSubmission {
            provenance: InboundEnvelopeProvenance::single(
                node::Path::new(["socket"]).expect("the fixture path is valid"),
            ),
            port: node::Port::conventional_out(),
            draft: Draft {
                data: Data::materialized(Value::Text(String::from(text))),
                ext: Namespace::new(),
            },
            sequence: SubmissionSequence::Next,
        },
        session: Some(session),
    }
}

fn int_submission(value: i64) -> InboundSubmission {
    InboundSubmission {
        provenance: InboundEnvelopeProvenance::single(node::Path::new(["cli"]).unwrap()),
        port: rastok::node::Port::new("in").unwrap(),
        draft: Draft {
            data: Data::materialized(Value::Int(value)),
            ext: Namespace::new(),
        },
        sequence: SubmissionSequence::Next,
    }
}

fn port_int_submission(port: &str, value: i64) -> InboundSubmission {
    InboundSubmission {
        provenance: InboundEnvelopeProvenance::single(node::Path::new(["cli"]).unwrap()),
        port: rastok::node::Port::new(port).unwrap(),
        draft: Draft {
            data: Data::materialized(Value::Int(value)),
            ext: Namespace::new(),
        },
        sequence: SubmissionSequence::Next,
    }
}

fn encoded_submission(
    port: &str,
    encoding: Encoding,
    bytes: &'static [u8],
    content_type: &str,
) -> InboundSubmission {
    let mut ext = Namespace::new();
    ext.insert(
        String::from("content_type"),
        rastok::envelope::MetaValue::Scalar(rastok::envelope::Scalar::Text(String::from(
            content_type,
        ))),
    );
    InboundSubmission {
        provenance: InboundEnvelopeProvenance::single(node::Path::new(["cli"]).unwrap()),
        port: rastok::node::Port::new(port).unwrap(),
        draft: Draft {
            data: Data::encoded(encoding, bytes),
            ext,
        },
        sequence: SubmissionSequence::Next,
    }
}

fn tenant_submission(tenant: &str, user: i64) -> InboundSubmission {
    InboundSubmission {
        provenance: InboundEnvelopeProvenance::single(node::Path::new(["cli"]).unwrap()),
        port: rastok::node::Port::new("in").unwrap(),
        draft: Draft {
            data: Data::materialized(Value::Struct(vec![
                (String::from("tenant"), Value::Text(String::from(tenant))),
                (String::from("user"), Value::Int(user)),
            ])),
            ext: Namespace::new(),
        },
        sequence: SubmissionSequence::Next,
    }
}

fn root_exit_cases<'a>(vm: &'a Vm<'_>) -> Vec<&'a str> {
    vm.observations()
        .iter()
        .filter_map(|observed| {
            if observed.node.is_some() || observed.port.name() != "exit" {
                return None;
            }
            let Data::Materialized(Value::TaggedUnion(tagged)) = observed.envelope.data() else {
                return None;
            };
            Some(tagged.case.as_str())
        })
        .collect()
}

fn run_probe() -> (MemJournal, Vec<rastok::runtime::vm::BoundaryOutput>) {
    let compilation = compile(
        &[M0_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    vm.inject(&mut journal, submission("salut")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    assert!(vm.is_stable());
    (journal, vm.outputs().to_vec())
}

#[test]
fn invocation_settings_are_values_before_the_file_driver_prepares() {
    let source = r#"
graph file_path : App {
  input in: text

  connector file : File {
    mode = read
    path = data as text
  }

  . -> file
  file.out -> nothing
  file.exit -> nothing
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the invocation-bound File path checks");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = FileSettingsWorld::default();

    vm.inject(&mut journal, text_submission("objects/report.bin"))
        .expect("the text path is admitted");
    vm.drain_with_bindings(&mut journal, &mut world)
        .expect("resolution and call opening succeed");

    let expected = std::collections::BTreeMap::from([(
        String::from("path"),
        Value::Text(String::from("objects/report.bin")),
    )]);
    assert_eq!(world.prepared.as_slice(), std::slice::from_ref(&expected));
    assert_eq!(
        journal.open_calls()[0].resolved_settings,
        expected,
        "the intent keeps the exact values the Driver prepared"
    );
}

#[test]
fn the_m0_probe_runs_end_to_end() {
    let (journal, outputs) = run_probe();
    // The whole mechanical chain, in causal order: birth, the
    // operator's transaction, the call's open and closure facts,
    // the connector's completion transaction.
    let kinds: Vec<String> = journal
        .records()
        .iter()
        .map(|record| match record {
            Record::Inbound(_) => String::from("inbound"),
            Record::Transaction(t) => {
                let owner = if t.owner_path.is_root() {
                    String::from("root")
                } else {
                    t.owner_path.to_string()
                };
                if t.trigger == node::Trigger::Lifecycle(node::Lifecycle::Start) {
                    format!("start:{owner}")
                } else {
                    format!("tx:{owner}")
                }
            }
            Record::Outbound(OutboundRecord::Call(o)) => match o.status {
                OutboundStatus::Open => String::from("outbound:open"),
                OutboundStatus::Closed(_) => String::from("outbound:closed"),
                OutboundStatus::Abandoned => String::from("outbound:abandoned"),
            },
            Record::Outbound(OutboundRecord::Session(_)) => String::from("session"),
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "inbound",
            "start:root",
            "start:shout",
            "tx:shout",
            "start:echo",
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
        outputs[0].envelope.meta().sys().origin_id(),
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
fn a_graph_key_is_selected_once_and_metadata_rewrites_cannot_migrate_it() {
    let source = r#"
graph rooted : App {
  input in: int
  output out: int
  instance = { key = meta.sys.id }

  operator first {
    input in: int
    output out: int
    on in { out = . }
  }
  operator second {
    input in: int
    output out: int
    on in { out = . }
  }

  . -> first -> second -> .
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("R3 admits a persistent Graph-owned key");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(&mut journal, int_submission(7)).unwrap();
    vm.drain(&mut journal).unwrap();

    let Record::Inbound(inbound) = &journal.records()[0] else {
        panic!("the first fact is ingress");
    };
    let root_id = inbound.envelope.meta().sys().id();
    let frame = InstanceFrame::SelectedKey {
        owner: OwnerPath::root(),
        ty: rastok::datatype::DataType::Text,
        value: Value::Text(root_id.to_string()),
    };
    let expected = |node: &str| {
        InstanceRef::new(
            InstanceAddress::new(
                &OwnerPath::new([node]).expect("valid owner path"),
                core::slice::from_ref(&frame),
            )
            .expect("the root frame belongs to each child"),
            0,
        )
    };
    assert_eq!(inbound.routes[0].target, expected("first"));

    let transactions: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction)
                if matches!(transaction.trigger, node::Trigger::Port(_)) =>
            {
                Some(transaction)
            }
            _ => None,
        })
        .collect();
    assert_eq!(transactions[0].instance, expected("first"));
    assert_eq!(transactions[0].routes[0].target, expected("second"));
    assert_eq!(transactions[1].instance, expected("second"));
    assert_ne!(
        transactions[0].emissions[0].envelope.meta().sys().id(),
        root_id,
        "the first hop really minted different meta.sys"
    );
}

#[test]
fn nested_graph_and_node_keys_form_one_state_address() {
    let source = r#"
graph nested : App {
  input in: struct { tenant: text, user: int }
  output out: int
  instance = { key = data.tenant }

  operator counter {
    instance = { key = data.user }
    input in: struct { tenant: text, user: int }
    output out: int
    var count: int = 0
    on in {
      count = count + 1
      out = count
    }
  }

  . -> counter -> .
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the two key dimensions are type-closed");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    for (tenant, user) in [("acme", 7), ("acme", 7), ("other", 7)] {
        vm.inject(&mut journal, tenant_submission(tenant, user))
            .unwrap();
        vm.drain(&mut journal).unwrap();
    }
    assert_eq!(
        vm.outputs()
            .iter()
            .map(|output| match output.envelope.data() {
                Data::Materialized(Value::Int(value)) => *value,
                other => panic!("the counter emits ints, got {other:?}"),
            })
            .collect::<Vec<_>>(),
        [1, 2, 1],
        "the inner key is namespaced by the enclosing Graph key"
    );
}

#[test]
fn nested_graph_source_routes_through_its_owner_before_the_child_node() {
    let source = r#"
graph nested : App {
  input in: text
  output out: text
  val prefix = "hello "

  graph sessions {
    input in: text
    output out: text
    instance = { key = data }

    operator greet {
      input in: text
      output out: text
      on in { out = prefix + data }
    }
    . -> greet -> .
  }

  . -> sessions -> .
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the public nested Graph source compiles");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    for _ in 0..2 {
        vm.inject(&mut journal, text_submission("alice")).unwrap();
        vm.drain(&mut journal).unwrap();
    }

    assert_eq!(vm.outputs().len(), 2);
    assert!(vm.outputs().iter().all(|output| {
        output.envelope.data() == &Data::materialized(Value::Text(String::from("hello alice")))
    }));
    assert!(journal.records().iter().any(|record| matches!(
        record,
        Record::Transaction(transaction)
            if transaction.owner_path.to_string() == "sessions"
                && transaction.trigger == node::Trigger::Lifecycle(node::Lifecycle::Start)
    )));
    let transactions: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction)
                if transaction.owner_path.to_string() == "sessions.greet"
                    && matches!(transaction.trigger, node::Trigger::Port(_)) =>
            {
                Some(transaction)
            }
            _ => None,
        })
        .collect();
    assert_eq!(transactions.len(), 2, "the child handler fires twice");
    let session = OwnerPath::new(["sessions"]).expect("valid Graph owner");
    let expected = InstanceAddress::new(
        &OwnerPath::new(["sessions", "greet"]).expect("valid child owner"),
        &[InstanceFrame::SelectedKey {
            owner: session,
            ty: rastok::datatype::DataType::Text,
            value: Value::Text(String::from("alice")),
        }],
    )
    .expect("the selected text key is canonical");
    assert!(
        transactions
            .iter()
            .all(|transaction| transaction.instance.address() == &expected),
        "the second route's fast lookup retains the reconstructed canonical address"
    );
}

const JOIN_PROBE: &str = r#"
type JoinExpiry = union { JoinExpired {} }
graph joined : App {
  input left: int
  input right: int
  output out: struct { left: int, right: int }
  output expired: JoinExpiry

  operator pair {
    input left: int
    input right: int
    output out: struct { left: int, right: int }
    output expired: JoinExpiry
    on (left, right) last 2 {
      out = { left = left.data ; right = right.data }
    }
  }

  .left -> pair.left
  .right -> pair.right
  pair.out -> .out
  pair.expired -> .expired
}
"#;

#[test]
fn a_public_join_consumes_one_presence_set_in_declared_port_order() {
    let compilation = compile(
        &[JOIN_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the public join source compiles");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));

    vm.inject(&mut journal, port_int_submission("left", 10))
        .unwrap();
    vm.drain(&mut journal).unwrap();
    assert!(
        vm.outputs().is_empty(),
        "an incomplete presence emits nothing"
    );
    vm.inject(&mut journal, port_int_submission("right", 20))
        .unwrap();
    vm.drain(&mut journal).unwrap();

    let output = vm
        .outputs()
        .iter()
        .find(|output| output.port.name() == "out")
        .expect("presence completion emits once");
    assert_eq!(
        output.envelope.data(),
        &Data::materialized(Value::Struct(vec![
            (String::from("left"), Value::Int(10)),
            (String::from("right"), Value::Int(20)),
        ]))
    );
    let transaction = journal
        .records()
        .iter()
        .find_map(|record| match record {
            Record::Transaction(transaction) if transaction.join_inputs.len() == 2 => {
                Some(transaction)
            }
            _ => None,
        })
        .expect("the join transaction carries both contribution roots");
    assert_eq!(transaction.trigger.to_string(), "p:right");
    assert_eq!(transaction.consumed.len(), 2);
    assert_eq!(
        transaction.join_inputs,
        vec![
            journal
                .records()
                .iter()
                .find_map(|record| match record {
                    Record::Inbound(inbound) if inbound.port.name() == "left" => {
                        Some(inbound.envelope.meta().sys().id())
                    }
                    _ => None,
                })
                .unwrap(),
            journal
                .records()
                .iter()
                .find_map(|record| match record {
                    Record::Inbound(inbound) if inbound.port.name() == "right" => {
                        Some(inbound.envelope.meta().sys().id())
                    }
                    _ => None,
                })
                .unwrap(),
        ],
        "contributions follow source-declared port order, not arrival sorting"
    );
}

#[test]
fn a_presence_join_uses_arrival_only_for_the_trigger_not_tuple_order() {
    let compilation = compile(
        &[JOIN_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the public join source compiles");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));

    vm.inject(&mut journal, port_int_submission("right", 20))
        .unwrap();
    vm.drain(&mut journal).unwrap();
    assert!(vm.is_stable(), "retained partial presence is quiescent");
    vm.inject(&mut journal, port_int_submission("left", 10))
        .unwrap();
    vm.drain(&mut journal).unwrap();

    let transaction = journal
        .records()
        .iter()
        .find_map(|record| match record {
            Record::Transaction(transaction) if transaction.join_inputs.len() == 2 => {
                Some(transaction)
            }
            _ => None,
        })
        .expect("the join fires");
    let inbound_id = |port: &str| {
        journal
            .records()
            .iter()
            .find_map(|record| match record {
                Record::Inbound(inbound) if inbound.port.name() == port => {
                    Some(inbound.envelope.meta().sys().id())
                }
                _ => None,
            })
            .expect("the named ingress exists")
    };
    assert_eq!(transaction.trigger.to_string(), "p:left");
    assert_eq!(
        transaction.join_inputs,
        [inbound_id("left"), inbound_id("right")],
        "the declaration fixes tuple identity even when right arrived first"
    );
}

const THETA_JOIN_PROBE: &str = r#"
type JoinExpiry = union { JoinExpired {} }
graph selected : App {
  input left: int
  input right: int
  output out: struct { left: int, right: int }
  output expired: JoinExpiry

  operator pair {
    input left: int
    input right: int
    output out: struct { left: int, right: int }
    output expired: JoinExpiry
    on (left, right) where left.data == right.data last 2 {
      out = { left = left.data ; right = right.data }
    }
  }

  .left -> pair.left
  .right -> pair.right
  pair.out -> .out
  pair.expired -> .expired
}
"#;

#[test]
fn a_theta_join_refines_fifo_candidates_without_consuming_a_false_pair() {
    let compilation = compile(
        &[THETA_JOIN_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the bounded theta join compiles");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    for (port, value) in [("left", 1), ("left", 2), ("right", 2)] {
        vm.inject(&mut journal, port_int_submission(port, value))
            .unwrap();
        vm.drain(&mut journal).unwrap();
    }

    let output = vm
        .outputs()
        .iter()
        .find(|output| output.port.name() == "out")
        .expect("the second left candidate matches right");
    assert_eq!(
        output.envelope.data(),
        &Data::materialized(Value::Struct(vec![
            (String::from("left"), Value::Int(2)),
            (String::from("right"), Value::Int(2)),
        ]))
    );
    assert!(
        vm.is_stable(),
        "left=1 remains as a retained partial presence, not runnable work"
    );
}

#[test]
fn last_evicts_the_oldest_candidate_before_correlating_new_work() {
    let compilation = compile(
        &[JOIN_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the last-bounded join compiles");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    for value in [1, 2, 3] {
        vm.inject(&mut journal, port_int_submission("left", value))
            .unwrap();
        vm.drain(&mut journal).unwrap();
    }

    let expiry = vm
        .outputs()
        .iter()
        .find(|output| output.port.name() == "expired")
        .expect("overflow is an explicit typed outcome");
    assert!(matches!(
        expiry.envelope.data(),
        Data::Materialized(Value::TaggedUnion(value)) if value.case == "JoinExpired"
    ));
    vm.inject(&mut journal, port_int_submission("right", 9))
        .unwrap();
    vm.drain(&mut journal).unwrap();
    let joined = vm
        .outputs()
        .iter()
        .find(|output| output.port.name() == "out")
        .expect("the next FIFO candidate remains available");
    assert_eq!(
        joined.envelope.data(),
        &Data::materialized(Value::Struct(vec![
            (String::from("left"), Value::Int(2)),
            (String::from("right"), Value::Int(9)),
        ]))
    );
    assert!(journal.records().iter().any(|record| matches!(
        record,
        Record::Transaction(transaction)
            if transaction.trigger == node::Trigger::Lifecycle(node::Lifecycle::Evict)
                && transaction.cause.is_some()
    )));
}

const WITHIN_JOIN_PROBE: &str = r#"
type JoinExpiry = union { JoinExpired {} }
graph timed : App {
  input left: int
  input right: int
  output out: int
  output expired: JoinExpiry

  operator pair {
    input left: int
    input right: int
    output out: int
    output expired: JoinExpiry
    on (left, right) within duration("1s") { out = left.data + right.data }
  }

  .left -> pair.left
  .right -> pair.right
  pair.out -> .out
  pair.expired -> .expired
}
"#;

#[test]
fn within_evicts_only_a_still_buffered_candidate() {
    let compilation = compile(
        &[WITHIN_JOIN_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the within-bounded join compiles");

    let mut expired_journal = MemJournal::new(GraphVersion(1));
    let mut expired = Vm::boot(&compilation.plan, GraphVersion(1));
    expired
        .inject(&mut expired_journal, port_int_submission("left", 4))
        .unwrap();
    expired.drain(&mut expired_journal).unwrap();
    expired
        .advance(
            &mut expired_journal,
            &mut EchoWorld::default(),
            1_000_000_000,
        )
        .unwrap();
    assert!(
        expired
            .outputs()
            .iter()
            .any(|output| output.port.name() == "expired")
    );

    let mut joined_journal = MemJournal::new(GraphVersion(1));
    let mut joined = Vm::boot(&compilation.plan, GraphVersion(1));
    joined
        .inject(&mut joined_journal, port_int_submission("left", 4))
        .unwrap();
    joined.drain(&mut joined_journal).unwrap();
    joined
        .inject(&mut joined_journal, port_int_submission("right", 5))
        .unwrap();
    joined.drain(&mut joined_journal).unwrap();
    joined
        .advance(
            &mut joined_journal,
            &mut EchoWorld::default(),
            1_000_000_000,
        )
        .unwrap();
    assert_eq!(
        joined
            .outputs()
            .iter()
            .filter(|output| output.port.name() == "out")
            .count(),
        1
    );
    assert!(
        joined
            .outputs()
            .iter()
            .all(|output| output.port.name() != "expired"),
        "the stale timer cannot evict a contribution already committed"
    );
}

const KEYED_JOIN_PROBE: &str = r#"
type JoinExpiry = union { JoinExpired {} }
graph keyed_join : App {
  input left: int
  input right: int
  output out: int
  output expired: JoinExpiry

  operator pair {
    instance = { key = data }
    input left: int
    input right: int
    output out: int
    output expired: JoinExpiry
    on (left, right) last 2 { out = left.data }
  }

  .left -> pair.left
  .right -> pair.right
  pair.out -> .out
  pair.expired -> .expired
}
"#;

#[test]
fn a_join_correlates_only_inside_its_owning_operator_instance() {
    let compilation = compile(
        &[KEYED_JOIN_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the keyed join compiles");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));

    for (port, value) in [("left", 1), ("right", 2)] {
        vm.inject(&mut journal, port_int_submission(port, value))
            .unwrap();
        vm.drain(&mut journal).unwrap();
    }
    assert!(
        vm.outputs().is_empty(),
        "different instance addresses never form one presence set"
    );
    vm.inject(&mut journal, port_int_submission("right", 1))
        .unwrap();
    vm.drain(&mut journal).unwrap();

    let transaction = journal
        .records()
        .iter()
        .find_map(|record| match record {
            Record::Transaction(transaction) if transaction.join_inputs.len() == 2 => {
                Some(transaction)
            }
            _ => None,
        })
        .expect("equal keys complete one presence set");
    assert_eq!(transaction.instance.address(), &int_address("pair", 1));
    assert_eq!(
        vm.outputs()
            .iter()
            .filter(|output| output.port.name() == "out")
            .count(),
        1
    );
}

#[test]
fn a_join_can_pick_one_named_input_metadata_value_explicitly() {
    let source = r#"
type JoinExpiry = union { JoinExpired {} }
graph attributed : App {
  input left: int
  input right: int
  output out: variant

  operator mark_left {
    input in: int
    output out: int
    on in { out = { meta.app.token = data } }
  }
  operator mark_right {
    input in: int
    output out: int
    on in { out = { meta.app.token = data } }
  }
  operator pair {
    input left: int
    input right: int
    output out: int
    output expired: JoinExpiry
    on (left, right) last 2 {
      out = { data = left.data ; meta.app.token = right.meta.app.token }
    }
  }
  operator read_pick {
    input in: int
    output out: variant
    on in { out = meta.app.token }
  }

  .left -> mark_left -> pair.left
  .right -> mark_right -> pair.right
  pair.out -> read_pick -> .out
  pair.expired -> nothing
}

"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("named join metadata follows the same sealed grid as bare meta");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(&mut journal, port_int_submission("left", 1))
        .unwrap();
    vm.drain(&mut journal).unwrap();
    vm.inject(&mut journal, port_int_submission("right", 2))
        .unwrap();
    vm.drain(&mut journal).unwrap();

    assert_eq!(
        vm.outputs()
            .iter()
            .find(|output| output.port.name() == "out")
            .expect("the explicit attribution survives the join")
            .envelope
            .data(),
        &Data::materialized(Value::Int(2))
    );
}

#[test]
fn common_or_absent_ignores_missing_contributions_and_not_input_order() {
    let source = r#"
graph attributed : App {
  input left: int
  input right: int
  output out: int

  operator mark_right {
    input in: int
    output out: int
    on in { out = { meta.app.token = data } }
  }
  operator pair {
    input left: int
    input right: int
    output out: int
    on (left, right) { out = left.data }
  }

  .left -> pair.left
  .right -> mark_right -> pair.right
  pair.out -> .out
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("an unused optional join field is legal");

    for arrivals in [[("left", 1), ("right", 2)], [("right", 2), ("left", 1)]] {
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        for (port, value) in arrivals {
            vm.inject(&mut journal, port_int_submission(port, value))
                .unwrap();
            vm.drain(&mut journal).unwrap();
        }
        assert_eq!(
            vm.outputs()[0].envelope.meta().app().get("token"),
            Some(&rastok::envelope::MetaValue::Scalar(
                rastok::envelope::Scalar::Int(2)
            )),
            "one present value survives regardless of which input triggers"
        );
    }
}

#[test]
fn join_payload_laziness_is_keyed_by_declared_input() {
    let compile_join = |body: &str, output_type: &str| {
        let source = format!(
            r#"
graph lazy_join : App {{
  input left: variant
  input right: variant
  output out: {output_type}
  operator pair {{
    input left: variant
    input right: variant
    output out: {output_type}
    on (left, right) {{ out = {body} }}
  }}
  .left -> pair.left
  .right -> pair.right
  pair.out -> .out
}}
"#
        );
        compile(
            &[source.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("the lazy join checks")
    };

    let carried = compile_join("left.data", "variant");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&carried.plan, GraphVersion(1));
    vm.inject(
        &mut journal,
        encoded_submission(
            "left",
            Encoding::Avro,
            b"left-opaque",
            "application/avro-left",
        ),
    )
    .unwrap();
    vm.drain(&mut journal).unwrap();
    vm.inject(
        &mut journal,
        encoded_submission(
            "right",
            Encoding::Avro,
            b"right-opaque",
            "application/avro-right",
        ),
    )
    .unwrap();
    vm.drain(&mut journal)
        .expect("carrying left never asks either opaque decoder");
    let output = &vm.outputs()[0].envelope;
    assert!(matches!(
        output.data(),
        Data::Encoded { encoding: Encoding::Avro, bytes, .. }
            if bytes.as_slice() == b"left-opaque"
    ));
    assert_eq!(
        output.meta().ext().get("content_type"),
        Some(&rastok::envelope::MetaValue::Scalar(
            rastok::envelope::Scalar::Text(String::from("application/avro-left"))
        )),
        "the selected payload, not the causal trigger, decides content_type"
    );

    let inspected = compile_join("size(left.data)", "int");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&inspected.plan, GraphVersion(1));
    vm.inject(
        &mut journal,
        encoded_submission("right", Encoding::Avro, b"never-read", "application/avro"),
    )
    .unwrap();
    vm.drain(&mut journal).unwrap();
    vm.inject(
        &mut journal,
        encoded_submission("left", Encoding::Raw, b"abc", "text/plain"),
    )
    .unwrap();
    vm.drain(&mut journal)
        .expect("only the structurally inspected left payload decodes");
    assert_eq!(
        vm.outputs()[0].envelope.data(),
        &Data::materialized(Value::Int(3))
    );
    assert!(
        vm.outputs()[0]
            .envelope
            .meta()
            .ext()
            .get("content_type")
            .is_none(),
        "a computed payload is fresh"
    );
}

#[test]
fn a_graph_instance_value_binds_process_before_the_first_effect() {
    let source = r#"
graph dynamic_binding : App {
  input in: text
  output out: text

  connector shell : Process {
    mode = job
    command = "cat"
    codec = lines
    effect = idempotent
    behavior = deterministic
  }

  . -> shell
  shell.out -> .out
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the static spelling supplies a valid plan skeleton");
    let mut raw = compilation.plan.into_inner();
    raw.owners[0].instance.key = Some(rastok::expr::Tree {
        root: rastok::expr::Expr::Literal(Value::Text(String::from("cat"))),
        ty: rastok::datatype::DataType::Text,
    });
    raw.owners[0].vals.push(OwnerValue {
        name: String::from("selected_command"),
        site: EvaluationSite::Instance,
        expr: rastok::expr::Tree {
            root: rastok::expr::Expr::Path(vec![String::from("instance"), String::from("key")]),
            ty: rastok::datatype::DataType::Text,
        },
        resolved: None,
        annotations: Default::default(),
    });
    let shell = raw
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "shell")
        .expect("the Process node exists");
    let rastok::plan::NodeExecution::Connector(spec) = &mut shell.execution else {
        panic!("shell is a connector")
    };
    let (_, command) = spec
        .attributes
        .iter_mut()
        .find(|(name, _)| name == "command")
        .expect("Process declares command");
    *command = AttrSetting::Expr {
        site: EvaluationSite::Instance,
        expr: rastok::expr::Tree {
            root: rastok::expr::Expr::Path(vec![String::from("selected_command")]),
            ty: rastok::datatype::DataType::Text,
        },
    };
    let plan = ValidatedExecutionPlan::try_new(raw)
        .expect("the Graph value and Process setting close at instance entry");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&plan, GraphVersion(1));
    let mut process = rastok::connectors::Process::try_new(&plan)
        .expect("the Driver accepts a validated instance-bound sheet");
    vm.inject(&mut journal, text_submission("salut")).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        vm.drain_with_bindings(&mut journal, &mut process).unwrap();
        vm.pump(&mut journal, &mut process).unwrap();
        vm.drain_with_bindings(&mut journal, &mut process).unwrap();
        if vm.is_stable() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the instance-bound Process did not settle"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        vm.outputs()
            .iter()
            .filter(|output| output.port.name() == "out")
            .map(|output| output.envelope.data().clone())
            .collect::<Vec<_>>(),
        [Data::materialized(Value::Text(String::from("salut")))],
        "the Driver ran the command selected by the parent Graph instance"
    );
}

#[test]
fn a_failed_instance_bundle_commits_no_ingress_start_or_effect() {
    let source = r#"
graph failing_bundle : App {
  input in: int
  output out: int
  instance = { key = data }

  operator relay {
    input in: int
    output out: int
    on in { out = . }
  }

  . -> relay -> .
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the keyed plan checks");
    let mut raw = compilation.plan.into_inner();
    raw.owners[0].vals.push(OwnerValue {
        name: String::from("broken"),
        site: EvaluationSite::Instance,
        expr: rastok::expr::Tree {
            root: rastok::expr::Expr::Binary {
                op: rastok::expr::BinaryOp::Div,
                left: Box::new(rastok::expr::Expr::Path(vec![
                    String::from("instance"),
                    String::from("key"),
                ])),
                right: Box::new(rastok::expr::Expr::Literal(Value::Int(0))),
            },
            ty: rastok::datatype::DataType::INT,
        },
        resolved: None,
        annotations: Default::default(),
    });
    let plan = ValidatedExecutionPlan::try_new(raw)
        .expect("division is a well-typed instance expression even when this value faults");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&plan, GraphVersion(1));

    assert!(matches!(
        vm.inject(&mut journal, int_submission(7)),
        Err(VmError::Fault(_))
    ));
    assert!(
        journal.records().is_empty(),
        "candidate evaluation precedes ingress, lifecycle, and Driver effects"
    );
    assert!(vm.is_stable(), "the failed candidate installed no work");
}

#[test]
fn keyed_instances_isolate_state_and_own_their_edge_cursors() {
    let source = r#"
graph keyed : App {
  input in: int
  output out: int

  operator counter {
    instance = { key = data }
    input in: int
    output out: int
    var count: int = 0
    on in {
      count = count + 1
      out = count
    }
  }

  operator sink {
    instance = { key = data }
    input in: int
    output out: int
    on in { out = . }
  }

  . -> counter -> sink -> .
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the keyed graph checks");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));

    for value in [7, 9, 7] {
        vm.inject(&mut journal, int_submission(value)).unwrap();
        vm.drain(&mut journal).unwrap();
    }

    let output_values: Vec<i64> = vm
        .outputs()
        .iter()
        .map(|output| match output.envelope.data() {
            Data::Materialized(Value::Int(value)) => *value,
            other => panic!("the counter emits ints, got {other:?}"),
        })
        .collect();
    assert_eq!(output_values, [1, 1, 2], "each key owns its state cell");

    let key7 = int_address("counter", 7);
    let key9 = int_address("counter", 9);
    let key1 = int_address("sink", 1);
    let key2 = int_address("sink", 2);
    let inbound_routes: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Inbound(inbound) => Some(inbound.routes[0].target.address().clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        inbound_routes,
        [key7.clone(), key9.clone(), key7.clone()],
        "the ingress fact seals the selected target before delivery"
    );
    let transactions: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction) => Some(transaction),
            _ => None,
        })
        .collect();
    let counters: Vec<_> = transactions
        .iter()
        .copied()
        .filter(|transaction| {
            transaction.owner_path.to_string() == "counter"
                && matches!(transaction.trigger, node::Trigger::Port(_))
        })
        .collect();
    assert_eq!(
        counters
            .iter()
            .map(|transaction| transaction.instance.address().clone())
            .collect::<Vec<_>>(),
        [key7.clone(), key9.clone(), key7.clone()],
        "the chosen target key is a causal transaction field"
    );
    let sinks: Vec<_> = transactions
        .iter()
        .copied()
        .filter(|transaction| {
            transaction.owner_path.to_string() == "sink"
                && matches!(transaction.trigger, node::Trigger::Port(_))
        })
        .collect();
    assert_eq!(
        sinks
            .iter()
            .map(|transaction| transaction.instance.address().clone())
            .collect::<Vec<_>>(),
        [key1.clone(), key1.clone(), key2.clone()]
    );
    assert_eq!(
        counters
            .iter()
            .map(|transaction| transaction.routes[0].target.address().clone())
            .collect::<Vec<_>>(),
        [key1.clone(), key1.clone(), key2.clone()],
        "the emitting fact seals each downstream target before delivery"
    );
    assert_eq!(
        sinks
            .iter()
            .map(|transaction| (
                transaction.consumed[0].source.address().clone(),
                transaction.consumed[0].seq,
            ))
            .collect::<Vec<_>>(),
        [(key7.clone(), 1), (key9, 1), (key7, 2)],
        "the cursor advances per edge and source instance reference"
    );
}

#[test]
fn idle_lifecycle_starts_once_stops_from_last_activity_and_restarts_clean() {
    let source = r#"
graph lifecycle : App {
  input in: int
  output out: int
  output stopped: int

  operator counter {
    instance = { key = data }
    lifetime = { kind = idle ; after = duration("1s") }
    input in: int
    output out: int
    output stopped: int
    var count: int = 0
    on start { count = 100 }
    on in {
      count = count + 1
      out = count
    }
    on stop { stopped = count }
  }

  . -> counter
  counter.out -> .out
  counter.stopped -> .stopped
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the lifecycle graph checks");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));

    let first = vm.inject(&mut journal, int_submission(7)).unwrap();
    vm.drain(&mut journal).unwrap();
    let second = vm.inject(&mut journal, int_submission(7)).unwrap();
    vm.drain(&mut journal).unwrap();
    let other = vm.inject(&mut journal, int_submission(9)).unwrap();
    vm.drain(&mut journal).unwrap();
    assert_eq!(
        vm.outputs()
            .iter()
            .filter(|output| output.port.name() == "out")
            .map(|output| match output.envelope.data() {
                Data::Materialized(Value::Int(value)) => *value,
                other => panic!("the counter emits ints, got {other:?}"),
            })
            .collect::<Vec<_>>(),
        [101, 102, 101]
    );

    let mut world = EchoWorld::default();
    vm.advance(&mut journal, &mut world, 1_000_000_000).unwrap();
    assert_eq!(
        vm.outputs()
            .iter()
            .filter(|output| output.port.name() == "stopped")
            .map(|output| match output.envelope.data() {
                Data::Materialized(Value::Int(value)) => *value,
                other => panic!("the stop hook emits ints, got {other:?}"),
            })
            .collect::<Vec<_>>(),
        [102, 101],
        "same-time expiries retain arm order after stale generations disappear"
    );

    let lifecycle: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction)
                if transaction.owner_path.to_string() == "counter"
                    && matches!(transaction.trigger, node::Trigger::Lifecycle(_)) =>
            {
                Some((
                    transaction.trigger.clone(),
                    transaction.cause,
                    transaction.instance.clone(),
                    transaction.parent_instance.clone(),
                ))
            }
            _ => None,
        })
        .collect();
    let root = InstanceRef::new(InstanceAddress::singleton(&OwnerPath::root()), 0);
    let seven = InstanceRef::new(int_address("counter", 7), 0);
    let nine = InstanceRef::new(int_address("counter", 9), 0);
    assert_eq!(
        lifecycle,
        [
            (
                node::Trigger::Lifecycle(node::Lifecycle::Start),
                Some(first),
                seven.clone(),
                Some(root.clone()),
            ),
            (
                node::Trigger::Lifecycle(node::Lifecycle::Start),
                Some(other),
                nine.clone(),
                Some(root),
            ),
            (
                node::Trigger::Lifecycle(node::Lifecycle::Closing),
                Some(second),
                seven.clone(),
                None,
            ),
            (
                node::Trigger::Lifecycle(node::Lifecycle::Closing),
                Some(other),
                nine.clone(),
                None,
            ),
            (
                node::Trigger::Lifecycle(node::Lifecycle::Stop),
                Some(second),
                seven,
                None,
            ),
            (
                node::Trigger::Lifecycle(node::Lifecycle::Stop),
                Some(other),
                nine,
                None,
            ),
        ]
    );

    let next = vm.inject(&mut journal, int_submission(7)).unwrap();
    vm.drain(&mut journal).unwrap();
    let last = vm
        .outputs()
        .iter()
        .rev()
        .find(|output| output.port.name() == "out")
        .expect("the new generation emits");
    assert_eq!(last.envelope.data(), &Data::materialized(Value::Int(101)));
    let restarted = journal
        .records()
        .iter()
        .find_map(|record| match record {
            Record::Transaction(transaction)
                if transaction.cause == Some(next)
                    && transaction.trigger == node::Trigger::Lifecycle(node::Lifecycle::Start) =>
            {
                Some(transaction)
            }
            _ => None,
        })
        .expect("the repeated key starts a fresh generation");
    assert_eq!(restarted.instance.generation(), 1);
    assert_eq!(
        journal.records().iter().find_map(|record| match record {
            Record::Inbound(inbound) if inbound.envelope.meta().sys().id() == next => {
                Some(inbound.routes[0].target.generation())
            }
            _ => None,
        }),
        Some(1),
        "routing seals the journal-derived generation before delivery"
    );
}

#[test]
fn idle_instance_stop_waits_for_each_connector_invocation_exit() {
    let source = r#"
graph nested_lifetimes : App {
  input in: text
  output out: text
  output exit

  connector shell : Process {
    instance = { key = data }
    lifetime = { kind = idle ; after = duration("1s") }
    closing = { drain_for = duration("1s") ; settle_for = duration("1s") }
    mode = job
    command = "cat"
    codec = lines
    timeout = duration("10s")
    effect = idempotent
    behavior = deterministic
  }

  . -> shell
  shell.out -> .out
  shell.exit -> .exit
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the keyed Process graph checks");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let cause = vm.inject(&mut journal, text_submission("salut")).unwrap();
    let mut world = EchoWorld::default();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    assert!(vm.has_open_obligations());

    vm.advance(&mut journal, &mut world, 1_000_000_000).unwrap();
    assert!(
        journal.records().iter().all(|record| !matches!(
            record,
            Record::Transaction(transaction)
                if transaction.trigger
                    == node::Trigger::Lifecycle(node::Lifecycle::Stop)
        )),
        "instance expiry cannot replace an open call's terminal fact"
    );

    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain(&mut journal).unwrap();
    let shell_transactions: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction) if transaction.owner_path.to_string() == "shell" => {
                Some(transaction)
            }
            _ => None,
        })
        .collect();
    assert_eq!(shell_transactions.len(), 4);
    assert_eq!(
        shell_transactions[0].trigger,
        node::Trigger::Lifecycle(node::Lifecycle::Start)
    );
    assert_eq!(
        shell_transactions[1].trigger,
        node::Trigger::Lifecycle(node::Lifecycle::Closing)
    );
    assert!(
        shell_transactions[2]
            .emissions
            .iter()
            .any(|emission| emission.port.name() == "exit")
    );
    assert_eq!(
        shell_transactions[3].trigger,
        node::Trigger::Lifecycle(node::Lifecycle::Stop)
    );
    assert_eq!(shell_transactions[3].cause, Some(cause));
}

#[test]
fn a_graph_close_drains_a_previously_admitted_call_through_an_existing_descendant() {
    let source = r#"
graph draining : App {
  input in: text
  output out: text
  lifetime = { kind = fixed ; after = duration("1s") }
  closing = { drain_for = duration("1s") ; settle_for = duration("1s") }

  operator seed { on in { out = . } }
  connector shell : Process {
    mode = job
    command = "cat"
    codec = lines
    timeout = duration("10s")
    effect = idempotent
    behavior = deterministic
  }
  operator collect {
    instance = { key = data }
    on in { out = . }
  }

  . -> seed -> collect -> .
  . -> shell
  shell.out -> collect
  shell.exit -> nothing
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a bounded call may live inside a finite Graph");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();

    vm.inject(&mut journal, text_submission("salut")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    assert!(vm.has_open_obligations());
    assert_eq!(vm.outputs().len(), 1, "the seed creates collect first");

    vm.advance(&mut journal, &mut world, 1_000_000_000).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();

    assert_eq!(vm.outputs().len(), 2);
    assert!(journal.records().iter().any(|record| matches!(
        record,
        Record::Transaction(transaction)
            if transaction.owner_path.to_string() == "collect"
                && transaction.trigger == node::Trigger::Lifecycle(node::Lifecycle::Stop)
    )));
    assert!(journal.records().iter().any(|record| matches!(
        record,
        Record::Transaction(transaction)
            if transaction.owner_path.is_root()
                && transaction.trigger == node::Trigger::Lifecycle(node::Lifecycle::Stop)
    )));
}

#[test]
fn a_confirmed_cancel_keeps_the_observed_exit_and_never_invents_unknown() {
    let compilation = compile(
        &[CLOSING_PROCESS],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the explicit closing policy bounds the open Process call");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = ClosingWorld {
        confirm_cancel: true,
        ..ClosingWorld::default()
    };

    vm.inject(&mut journal, text_submission("salut")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    assert_eq!(world.started.len(), 1);

    vm.advance(&mut journal, &mut world, 1_000_000_000).unwrap();
    assert!(
        world.cancelled.is_empty(),
        "Closing first offers the drain budget"
    );
    vm.advance(&mut journal, &mut world, 2_000_000_000).unwrap();

    assert_eq!(world.cancelled, world.started);
    assert_eq!(root_exit_cases(&vm), ["Exited"]);
    assert!(journal.records().iter().any(|record| matches!(
        record,
        Record::Outbound(OutboundRecord::Call(outbound))
            if matches!(outbound.status, OutboundStatus::Closed(_))
    )));
    assert!(journal.records().iter().all(|record| !matches!(
        record,
        Record::Outbound(OutboundRecord::Call(outbound))
            if outbound.status == OutboundStatus::Abandoned
    )));
}

#[test]
fn silent_cancel_becomes_one_unknown_only_after_settlement() {
    let compilation = compile(
        &[CLOSING_PROCESS],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the explicit closing policy bounds the open Process call");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = ClosingWorld::default();

    vm.inject(&mut journal, text_submission("salut")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    vm.advance(&mut journal, &mut world, 2_000_000_000).unwrap();
    assert_eq!(world.cancelled.len(), 1);
    assert!(root_exit_cases(&vm).is_empty());
    assert!(
        vm.has_open_obligations(),
        "settlement is a real grace interval"
    );

    vm.advance(&mut journal, &mut world, 3_000_000_000).unwrap();
    assert_eq!(root_exit_cases(&vm), ["OutcomeUnknown"]);
    assert_eq!(
        journal
            .records()
            .iter()
            .filter(|record| matches!(
                record,
                Record::Outbound(OutboundRecord::Call(outbound))
                    if outbound.status == OutboundStatus::Abandoned
            ))
            .count(),
        1,
        "the close-once winner appends one abandonment fact"
    );
    assert!(journal.open_calls().is_empty());
    assert!(
        journal.records().iter().any(|record| matches!(
            record,
            Record::Transaction(transaction)
                if transaction.trigger == node::Trigger::Lifecycle(node::Lifecycle::Stop)
        )),
        "the invocation exit precedes lifecycle reclamation"
    );
    world.answers.push(Response {
        call_ref: world.started[0],
        completion: Completion::process(
            vec![],
            vec![],
            ProcessExit::Exited {
                exit_code: 0,
                signal: None,
            },
        ),
    });
    let report = vm.pump(&mut journal, &mut world).unwrap();
    assert_eq!(
        report.discarded.len(),
        1,
        "a late reply is observable but non-causal"
    );
    assert_eq!(root_exit_cases(&vm), ["OutcomeUnknown"]);
}

#[test]
fn bounded_closing_interleavings_are_model_checked() {
    #[derive(Clone, Copy, Debug)]
    enum ReplyAt {
        BeforeDrain,
        DuringSettlement,
        OnCancel,
        AfterSettlement,
        Never,
    }

    let compilation = compile(
        &[CLOSING_PROCESS],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the explicit policy closes every schedule in bounded time");
    for reply_at in [
        ReplyAt::BeforeDrain,
        ReplyAt::DuringSettlement,
        ReplyAt::OnCancel,
        ReplyAt::AfterSettlement,
        ReplyAt::Never,
    ] {
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        let mut world = ClosingWorld {
            confirm_cancel: matches!(reply_at, ReplyAt::OnCancel),
            ..ClosingWorld::default()
        };
        vm.inject(&mut journal, text_submission("one")).unwrap();
        vm.drain_with_bindings(&mut journal, &mut world).unwrap();
        vm.pump(&mut journal, &mut world).unwrap();
        let call = world.started[0];
        let reply = || Response {
            call_ref: call,
            completion: Completion::process(
                vec![],
                vec![],
                ProcessExit::Exited {
                    exit_code: 0,
                    signal: None,
                },
            ),
        };

        // t=1 requests Closing; t=2 ends drain and requests cancel;
        // t=3 ends settlement. Reply placement is the model dimension.
        vm.advance(&mut journal, &mut world, 1_000_000_000).unwrap();
        if matches!(reply_at, ReplyAt::BeforeDrain) {
            world.answers.push(reply());
            vm.pump(&mut journal, &mut world).unwrap();
            vm.drain_with_bindings(&mut journal, &mut world).unwrap();
        }
        vm.advance(&mut journal, &mut world, 2_000_000_000).unwrap();
        if matches!(reply_at, ReplyAt::DuringSettlement) {
            world.answers.push(reply());
            vm.pump(&mut journal, &mut world).unwrap();
            vm.drain(&mut journal).unwrap();
        }
        vm.advance(&mut journal, &mut world, 3_000_000_000).unwrap();
        let late = if matches!(reply_at, ReplyAt::AfterSettlement) {
            world.answers.push(reply());
            vm.pump(&mut journal, &mut world).unwrap()
        } else {
            Default::default()
        };
        vm.drain(&mut journal).unwrap();

        let expected = if matches!(
            reply_at,
            ReplyAt::BeforeDrain | ReplyAt::DuringSettlement | ReplyAt::OnCancel
        ) {
            "Exited"
        } else {
            "OutcomeUnknown"
        };
        assert_eq!(root_exit_cases(&vm), [expected], "schedule {reply_at:?}");
        assert_eq!(
            journal
                .records()
                .iter()
                .filter(|record| matches!(
                    record,
                    Record::Outbound(OutboundRecord::Call(outbound))
                        if outbound.intent.call_ref == call
                ))
                .count(),
            2,
            "open plus exactly one terminal journal fact for {reply_at:?}"
        );
        let exit_at = journal
            .records()
            .iter()
            .position(|record| matches!(
                record,
                Record::Transaction(transaction)
                    if transaction.emissions.iter().any(|emission| emission.port.name() == "exit")
            ))
            .expect("one call-terminal transaction");
        let stop_at = journal
            .records()
            .iter()
            .position(|record| {
                matches!(
                    record,
                    Record::Transaction(transaction)
                        if transaction.owner_path.is_root()
                            && transaction.trigger
                                == node::Trigger::Lifecycle(node::Lifecycle::Stop)
                )
            })
            .expect("bounded Closing reaches stop");
        assert!(
            exit_at < stop_at,
            "exit precedes reclamation for {reply_at:?}"
        );
        assert!(vm.is_stable(), "schedule {reply_at:?} reaches rest");
        if matches!(reply_at, ReplyAt::AfterSettlement) {
            assert_eq!(late.discarded.len(), 1, "the late reply is observable");
        }
    }
}

#[test]
fn external_arrival_waits_for_the_clean_successor_generation() {
    let compilation = compile(
        &[CLOSING_PROCESS],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the explicit closing policy bounds the open Process call");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = ClosingWorld {
        confirm_cancel: true,
        ..ClosingWorld::default()
    };

    vm.inject(&mut journal, text_submission("first")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    vm.advance(&mut journal, &mut world, 1_000_000_000).unwrap();
    let second = vm
        .inject(&mut journal, text_submission("second"))
        .expect("Closing reserves a successor instead of refusing the arrival");
    assert!(
        journal.records().iter().all(|record| !matches!(
            record,
            Record::Transaction(transaction) if transaction.cause == Some(second)
        )),
        "the committed arrival has not executed in the closing generation"
    );

    vm.advance(&mut journal, &mut world, 2_000_000_000).unwrap();
    let root_lifecycle: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction)
                if transaction.owner_path.is_root()
                    && matches!(transaction.trigger, node::Trigger::Lifecycle(_)) =>
            {
                Some((
                    transaction.trigger.clone(),
                    transaction.instance.generation(),
                ))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        root_lifecycle,
        [
            (node::Trigger::Lifecycle(node::Lifecycle::Start), 0),
            (node::Trigger::Lifecycle(node::Lifecycle::Closing), 0),
            (node::Trigger::Lifecycle(node::Lifecycle::Stop), 0),
            (node::Trigger::Lifecycle(node::Lifecycle::Start), 1),
        ],
        "the old stop is durable before the clean successor starts"
    );
    assert!(
        journal.records().iter().any(|record| matches!(
            record,
            Record::Transaction(transaction) if transaction.cause == Some(second)
        )),
        "the waiting delivery runs only after generation one starts"
    );
}

#[test]
fn drain_residuals_are_sealed_by_stop_and_cancels_are_call_ref_ordered() {
    let source = r#"
graph two_calls : App {
  input in: text
  lifetime = { kind = fixed ; after = duration("1s") }
  closing = { drain_for = duration("1s") ; settle_for = duration("1s") }

  connector a : Process {
    mode = job ; command = "cat" ; codec = lines
    effect = idempotent ; behavior = deterministic
  }
  connector b : Process {
    mode = job ; command = "cat" ; codec = lines
    effect = idempotent ; behavior = deterministic
  }

  . -> a
  . -> b
  a.out -> nothing ; a.err -> nothing ; a.exit -> nothing
  b.out -> nothing ; b.err -> nothing ; b.exit -> nothing
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("one root policy bounds both Process descendants");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = ClosingWorld::default();

    vm.inject(&mut journal, text_submission("first")).unwrap();
    vm.inject(&mut journal, text_submission("queued")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    assert_eq!(
        world.started.len(),
        2,
        "one call per claimed connector starts"
    );
    vm.advance(&mut journal, &mut world, 2_000_000_000).unwrap();
    let mut sorted = world.cancelled.clone();
    sorted.sort();
    assert_eq!(
        world.cancelled, sorted,
        "cancels follow canonical CallRef order"
    );
    vm.advance(&mut journal, &mut world, 3_000_000_000).unwrap();

    let residual_count: usize = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction)
                if transaction.trigger == node::Trigger::Lifecycle(node::Lifecycle::Stop) =>
            {
                Some(transaction.consumed.len())
            }
            _ => None,
        })
        .sum();
    assert_eq!(
        residual_count, 2,
        "the second ingress left one admitted-but-unconsumed delivery per connector"
    );
    assert!(root_exit_cases(&vm).is_empty());
}

#[test]
fn fixed_and_single_pass_reenter_with_clean_state_and_the_next_generation() {
    for (lifetime, needs_advance) in [
        (
            "lifetime = { kind = fixed ; after = duration(\"1s\") }",
            true,
        ),
        ("lifetime = { kind = single_pass }", false),
    ] {
        let source = format!(
            r#"
graph finite : App {{
  input in: int
  output out: int

  operator counter {{
    {lifetime}
    input in: int
    output out: int
    var count: int = 0
    on in {{ count = count + 1 ; out = count }}
  }}

  . -> counter -> .
}}
"#
        );
        let compilation = compile(
            &[&source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("R4 admits the finite profile");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        let mut world = EchoWorld::default();

        vm.inject(&mut journal, int_submission(1)).unwrap();
        vm.drain(&mut journal).unwrap();
        if needs_advance {
            vm.advance(&mut journal, &mut world, 1_000_000_000).unwrap();
        }
        vm.inject(&mut journal, int_submission(2)).unwrap();
        vm.drain(&mut journal).unwrap();

        let values: Vec<i64> = vm
            .outputs()
            .iter()
            .map(|output| match output.envelope.data() {
                Data::Materialized(Value::Int(value)) => *value,
                other => panic!("the counter emits ints, got {other:?}"),
            })
            .collect();
        assert_eq!(values, [1, 1], "a successor never inherits state");
        let lifecycle: Vec<_> = journal
            .records()
            .iter()
            .filter_map(|record| match record {
                Record::Transaction(transaction)
                    if transaction.owner_path.to_string() == "counter"
                        && matches!(
                            transaction.trigger,
                            node::Trigger::Lifecycle(
                                node::Lifecycle::Start | node::Lifecycle::Stop
                            )
                        ) =>
                {
                    Some((
                        transaction.trigger.clone(),
                        transaction.instance.generation(),
                    ))
                }
                _ => None,
            })
            .collect();
        let mut expected = vec![
            (node::Trigger::Lifecycle(node::Lifecycle::Start), 0),
            (node::Trigger::Lifecycle(node::Lifecycle::Stop), 0),
            (node::Trigger::Lifecycle(node::Lifecycle::Start), 1),
        ];
        if !needs_advance {
            expected.push((node::Trigger::Lifecycle(node::Lifecycle::Stop), 1));
        }
        assert_eq!(lifecycle, expected);
    }
}

#[test]
fn persistent_owners_start_once_and_arm_no_lifetime_deadline() {
    let source = r#"
graph persistent : App {
  input in: int
  output out: int
  operator relay { on in { out = . } }
  . -> relay -> .
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("persistent is the executable omission default");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();

    vm.inject(&mut journal, int_submission(1)).unwrap();
    vm.advance(&mut journal, &mut world, i64::MAX).unwrap();

    let lifecycle: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction)
                if matches!(
                    transaction.trigger,
                    node::Trigger::Lifecycle(node::Lifecycle::Start | node::Lifecycle::Stop)
                ) =>
            {
                Some((transaction.trigger.clone(), transaction.timers.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(lifecycle.len(), 2, "root and relay start lazily once");
    assert!(lifecycle.iter().all(|(trigger, timers)| {
        *trigger == node::Trigger::Lifecycle(node::Lifecycle::Start) && timers.is_empty()
    }));
}

#[test]
fn keyed_fixed_instances_close_on_their_own_start_dates() {
    let source = r#"
graph keyed_fixed : App {
  input in: int
  output out: int
  operator relay {
    instance = { key = data }
    lifetime = { kind = fixed ; after = duration("1s") }
    on in { out = . }
  }
  . -> relay -> .
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("key and fixed lifetime are orthogonal");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();

    vm.inject(&mut journal, int_submission(1)).unwrap();
    vm.advance(&mut journal, &mut world, 500_000_000).unwrap();
    vm.inject(&mut journal, int_submission(2)).unwrap();
    vm.advance(&mut journal, &mut world, 1_000_000_000).unwrap();

    let stopped: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction)
                if transaction.owner_path.to_string() == "relay"
                    && transaction.trigger == node::Trigger::Lifecycle(node::Lifecycle::Stop) =>
            {
                Some(transaction.instance.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(stopped.len(), 1, "only the t=0 key reaches its deadline");

    vm.advance(&mut journal, &mut world, 1_500_000_000).unwrap();
    let stopped = journal
        .records()
        .iter()
        .filter(|record| {
            matches!(
                record,
                Record::Transaction(transaction)
                    if transaction.owner_path.to_string() == "relay"
                        && transaction.trigger == node::Trigger::Lifecycle(node::Lifecycle::Stop)
            )
        })
        .count();
    assert_eq!(stopped, 2);
}

#[test]
fn a_single_pass_successor_waits_until_its_first_generation_stops() {
    let source = r#"
graph one : App {
  input in: int
  output out: int
  operator relay {
    lifetime = { kind = single_pass }
    on in { out = . }
  }
  . -> relay -> .
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("single-pass is executable");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));

    vm.inject(&mut journal, int_submission(1)).unwrap();
    while !journal.records().iter().any(|record| {
        matches!(
            record,
            Record::Transaction(transaction) if transaction.close_scope.is_some()
        )
    }) {
        assert!(
            vm.step(&mut journal).unwrap(),
            "the first root reaches Closing"
        );
    }
    let second = vm.inject(&mut journal, int_submission(2)).unwrap();
    assert!(
        matches!(
            vm.inject(&mut journal, int_submission(3)),
            Err(VmError::InstanceClosing { owner }) if owner.to_string() == "relay"
        ),
        "one exact successor generation is already reserved"
    );
    assert!(
        journal.records().iter().all(|record| !matches!(
            record,
            Record::Transaction(transaction) if transaction.cause == Some(second)
        )),
        "the successor delivery waits in its ordinary mailbox"
    );
    vm.drain(&mut journal).unwrap();
    assert_eq!(vm.outputs().len(), 2);
    let generations: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction)
                if transaction.owner_path.to_string() == "relay"
                    && transaction.trigger == node::Trigger::Lifecycle(node::Lifecycle::Start) =>
            {
                Some(transaction.instance.generation())
            }
            _ => None,
        })
        .collect();
    assert_eq!(generations, [0, 1]);
}

#[test]
fn bare_stop_closes_exactly_the_current_owner_generation() {
    let source = r#"
graph explicit_stop : App {
  input in: int
  output out: int
  operator relay {
    input in: int
    output out: int
    on in { out = . ; stop }
  }
  . -> relay -> .
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("bare stop names the current owner without a target language");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(&mut journal, int_submission(1)).unwrap();
    vm.drain(&mut journal).unwrap();
    vm.inject(&mut journal, int_submission(2)).unwrap();
    vm.drain(&mut journal).unwrap();

    assert_eq!(
        vm.outputs()
            .iter()
            .map(|output| output.envelope.data().clone())
            .collect::<Vec<_>>(),
        [
            Data::materialized(Value::Int(1)),
            Data::materialized(Value::Int(2)),
        ]
    );
    let close_requests: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction) if transaction.close_scope.is_some() => {
                Some(transaction.instance.generation())
            }
            _ => None,
        })
        .collect();
    assert_eq!(close_requests, [0, 1]);
}

#[test]
fn a_tumbling_graph_is_a_real_global_window_and_closes_children_first() {
    let source = r#"
graph windows : App {
  input in: int
  output out: int
  lifetime = { kind = tumbling ; every = duration("5s") }

  operator counter {
    var count: int = 0
    on in { count = count + 1 ; out = count }
  }

  . -> counter -> .
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("an unkeyed Graph can own a temporal window");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();

    vm.inject(&mut journal, int_submission(1)).unwrap();
    vm.inject(&mut journal, int_submission(2)).unwrap();
    vm.advance(&mut journal, &mut world, 5_000_000_000).unwrap();
    vm.inject(&mut journal, int_submission(3)).unwrap();
    vm.drain(&mut journal).unwrap();

    let values: Vec<i64> = vm
        .outputs()
        .iter()
        .map(|output| match output.envelope.data() {
            Data::Materialized(Value::Int(value)) => *value,
            other => panic!("the counter emits ints, got {other:?}"),
        })
        .collect();
    assert_eq!(values, [1, 2, 1]);

    let lifecycle: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction)
                if matches!(
                    transaction.trigger,
                    node::Trigger::Lifecycle(node::Lifecycle::Start | node::Lifecycle::Stop)
                ) =>
            {
                Some((
                    transaction.owner_path.to_string(),
                    transaction.trigger.clone(),
                    transaction.instance.clone(),
                ))
            }
            _ => None,
        })
        .collect();
    let root_stops = lifecycle
        .iter()
        .position(|(owner, trigger, _)| {
            owner.is_empty() && *trigger == node::Trigger::Lifecycle(node::Lifecycle::Stop)
        })
        .expect("the first box closes");
    let child_stops = lifecycle
        .iter()
        .position(|(owner, trigger, _)| {
            owner == "counter" && *trigger == node::Trigger::Lifecycle(node::Lifecycle::Stop)
        })
        .expect("the child closes");
    assert!(child_stops < root_stops, "containment closes inner-first");

    let root_starts: Vec<_> = lifecycle
        .iter()
        .filter(|(owner, trigger, _)| {
            owner.is_empty() && *trigger == node::Trigger::Lifecycle(node::Lifecycle::Start)
        })
        .map(|(_, _, reference)| reference)
        .collect();
    assert_eq!(root_starts.len(), 2);
    assert_eq!(root_starts[0].generation(), 0);
    assert_eq!(root_starts[1].generation(), 0);
    assert_ne!(root_starts[0].address(), root_starts[1].address());
}

#[test]
fn a_closed_tumbling_box_never_reopens_as_a_second_generation() {
    let source = r#"
graph clipped : App {
  input in: int
  output out: int
  lifetime = { kind = fixed ; after = duration("1s") }

  operator window {
    lifetime = { kind = tumbling ; every = duration("5s") }
    on in { out = . }
  }

  . -> window -> .
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("nested finite profiles are executable");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();

    vm.inject(&mut journal, int_submission(1)).unwrap();
    vm.advance(&mut journal, &mut world, 1_000_000_000).unwrap();
    assert!(matches!(
        vm.inject(&mut journal, int_submission(2)),
        Err(VmError::ClosedTimeBox { owner, start_ns: 0 }) if owner.to_string() == "window"
    ));
    vm.advance(&mut journal, &mut world, 5_000_000_000).unwrap();
    vm.inject(&mut journal, int_submission(2)).unwrap();
    vm.drain(&mut journal).unwrap();
    assert_eq!(vm.outputs().len(), 2);
}

#[test]
fn a_stale_fixed_deadline_cannot_close_the_successor_generation() {
    let source = r#"
graph parent : App {
  input in: int
  output out: int
  lifetime = { kind = fixed ; after = duration("1s") }

  operator child {
    lifetime = { kind = fixed ; after = duration("5s") }
    on in { out = . }
  }

  . -> child -> .
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("nested fixed generations are executable");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();

    vm.inject(&mut journal, int_submission(1)).unwrap();
    vm.advance(&mut journal, &mut world, 1_000_000_000).unwrap();
    vm.inject(&mut journal, int_submission(2)).unwrap();
    vm.drain(&mut journal).unwrap();
    vm.advance(&mut journal, &mut world, 2_000_000_000).unwrap();

    let records_after_successor = journal.records().len();
    vm.advance(&mut journal, &mut world, 5_000_000_000).unwrap();

    let child_stops: Vec<u64> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction)
                if transaction.owner_path.to_string() == "child"
                    && transaction.trigger == node::Trigger::Lifecycle(node::Lifecycle::Stop) =>
            {
                Some(transaction.instance.generation())
            }
            _ => None,
        })
        .collect();
    assert_eq!(child_stops, [0, 1]);
    assert_eq!(
        journal.records().len(),
        records_after_successor,
        "the stale generation-0 deadline cannot append a fact after generation 1 closed"
    );
}

#[test]
fn equal_date_lifetimes_close_in_one_deterministic_order() {
    let source = r#"
graph ties : App {
  input in: int
  operator alpha {
    lifetime = { kind = fixed ; after = duration("1s") }
    on in { out = . }
  }
  operator beta {
    lifetime = { kind = fixed ; after = duration("1s") }
    on in { out = . }
  }
  . -> alpha -> nothing
  . -> beta -> nothing
}
"#;
    let run = || {
        let compilation = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("equal fixed deadlines are executable");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        vm.inject(&mut journal, int_submission(1)).unwrap();
        vm.advance(&mut journal, &mut EchoWorld::default(), 1_000_000_000)
            .unwrap();
        journal
    };
    let once = run();
    let twice = run();
    assert_eq!(once.records(), twice.records());
    let stops: Vec<String> = once
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction)
                if transaction.trigger == node::Trigger::Lifecycle(node::Lifecycle::Stop) =>
            {
                Some(transaction.owner_path.to_string())
            }
            _ => None,
        })
        .collect();
    assert_eq!(stops, [String::from("alpha"), String::from("beta")]);
}

#[test]
fn stderr_and_terminal_status_leave_on_independent_ports() {
    let compilation = compile(
        &[M0_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld {
        fail_next: true,
        ..EchoWorld::default()
    };
    vm.inject(&mut journal, submission("salut")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    // stderr is ordinary data on `err`; the non-zero child status is an
    // independent, typed `Exited` terminal fact.
    assert!(vm.outputs().is_empty());
    let last = journal.records().last().unwrap();
    let Record::Transaction(transaction) = last else {
        panic!("expected the completion transaction");
    };
    assert_eq!(transaction.emissions[0].port.name(), "err");
    assert_eq!(transaction.emissions[1].port.name(), "exit");
    assert_eq!(vm.errors().len(), 1);
    assert_eq!(
        vm.errors()[0].data(),
        &Data::materialized(Value::Text(String::from("exit 1")))
    );
    assert_eq!(root_exit_cases(&vm), ["Exited"]);
}

#[test]
fn malformed_terminal_facts_are_refused_loudly() {
    let compilation = compile(
        &[M0_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut opener = EchoWorld::default();
    vm.inject(&mut journal, submission("salut")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut opener).unwrap();
    let open = journal.open_calls();
    let mut world = ScriptedWorld {
        answers: vec![
            Response {
                call_ref: open[0].call_ref,
                completion: Completion::try_new(vec![PortOutput {
                    port: node::Port::conventional_exit(),
                    data: Data::materialized(Value::TaggedUnion(Box::new(
                        rastok::value::TaggedUnionValue {
                            case: String::from("MadeUp"),
                            payload: vec![],
                        },
                    ))),
                    ext: rastok::envelope::Namespace::new(),
                }])
                .unwrap(),
            },
            Response {
                call_ref: open[0].call_ref,
                completion: Completion::try_new(vec![PortOutput {
                    port: node::Port::conventional_exit(),
                    data: Data::encoded(rastok::envelope::Encoding::Raw, b"not a union"),
                    ext: rastok::envelope::Namespace::new(),
                }])
                .unwrap(),
            },
        ],
    };
    let report = vm.pump(&mut journal, &mut world).unwrap();
    // Refused BEFORE the causal fact: the reply is rejected with
    // its reason, and the call stays open — the timeout machinery
    // can still settle it honestly.
    assert_eq!(report.settled, 0);
    assert_eq!(report.rejected.len(), 2);
    assert!(report.rejected[0].1.contains("MadeUp"));
    assert!(report.rejected[1].1.contains("materialized"));
    assert_eq!(journal.open_calls().len(), 1, "no garbage closure");
    vm.drain(&mut journal).unwrap();
    assert!(!vm.is_stable(), "the obligation is still open");
}

/// A world answering from a script, for integrity tests.
struct ScriptedWorld {
    answers: Vec<Response>,
}

impl CallConnectorDriver for ScriptedWorld {
    fn prepare_request(
        &mut self,
        _: &node::Path,
        data: &Data,
        _: &std::collections::BTreeMap<String, rastok::value::Value>,
    ) -> RequestPreparation {
        rastok::boundary::prepare_process_request(rastok::boundary::RequestCodec::Lines, data)
    }

    fn scheduler_completion(
        &mut self,
        _: &node::Path,
        completion: SchedulerCompletion,
    ) -> Completion {
        rastok::boundary::process_scheduler_completion(completion)
    }

    fn start(&mut self, _: &Intent, _: &Envelope) -> CallStart {
        CallStart::Started
    }
    fn events(&mut self) -> Vec<Response> {
        std::mem::take(&mut self.answers)
    }
}

#[derive(Default)]
struct SessionWorld {
    events: Vec<SessionDriverEvent>,
    opened: Vec<SessionRef>,
    pending_opens: bool,
    hold_writes: bool,
    writes: Vec<(SessionRef, SessionWriteRef)>,
}

impl SessionWorld {
    fn close(&mut self, session: SessionRef) {
        self.events.push(SessionDriverEvent::Exited {
            session,
            exit: Completion::websocket_exit(WebSocketExit::Closed {
                code: 1000,
                reason: String::from("done"),
            })
            .outputs()[0]
                .clone(),
        });
    }

    fn settle_write(&mut self, index: usize) {
        let (session, write) = self.writes[index];
        self.events
            .push(SessionDriverEvent::WriteSettled { session, write });
    }
}

impl OutboundConnectorBindings for SessionWorld {
    fn prepare_request(
        &mut self,
        connector: &node::Path,
        _: &Data,
        _: &std::collections::BTreeMap<String, rastok::value::Value>,
    ) -> Result<RequestPreparation, DriverRouteError> {
        Err(DriverRouteError::UnboundOutbound(connector.clone()))
    }

    fn scheduler_completion(
        &mut self,
        connector: &node::Path,
        _: SchedulerCompletion,
    ) -> Result<Completion, DriverRouteError> {
        Err(DriverRouteError::UnboundOutbound(connector.clone()))
    }

    fn cancel(
        &mut self,
        connector: &node::Path,
        _: &rastok::boundary::CallRef,
    ) -> Result<(), DriverRouteError> {
        Err(DriverRouteError::UnboundOutbound(connector.clone()))
    }

    fn start(&mut self, intent: &Intent, _: &Envelope) -> Result<CallStart, DriverRouteError> {
        Err(DriverRouteError::UnboundOutbound(
            intent.connector_path.clone(),
        ))
    }

    fn events(&mut self) -> Result<Vec<Response>, DriverRouteError> {
        Ok(Vec::new())
    }

    fn prepare_session_open(
        &mut self,
        _: &node::Path,
        data: &Data,
        _: &std::collections::BTreeMap<String, rastok::value::Value>,
    ) -> Result<RequestPreparation, DriverRouteError> {
        let bytes = match data {
            Data::Materialized(Value::Text(value)) => value.as_bytes().to_vec(),
            Data::Encoded { bytes, .. } => bytes.clone(),
            Data::Materialized(_) => Vec::new(),
        };
        Ok(RequestPreparation::Prepared(PreparedRequest::seal(bytes)))
    }

    fn scheduler_session_completion(
        &mut self,
        _: &node::Path,
        completion: SchedulerCompletion,
    ) -> Result<Completion, DriverRouteError> {
        Ok(Completion::websocket_exit(match completion {
            SchedulerCompletion::TimeoutExpired => WebSocketExit::TimeoutExpired,
            SchedulerCompletion::OutcomeUnknown => WebSocketExit::OutcomeUnknown,
        }))
    }

    fn open_session(
        &mut self,
        intent: &SessionOpenIntent,
        _: &Envelope,
    ) -> Result<SessionStart, DriverRouteError> {
        self.opened.push(intent.session_ref);
        if self.pending_opens {
            Ok(SessionStart::Pending)
        } else {
            self.events
                .push(SessionDriverEvent::Opened(intent.session_ref));
            Ok(SessionStart::Started)
        }
    }

    fn prepare_session_write(
        &mut self,
        _: &node::Path,
        _: SessionRef,
        data: &Data,
        _: &std::collections::BTreeMap<String, rastok::value::Value>,
    ) -> Result<RequestPreparation, DriverRouteError> {
        self.prepare_session_open(
            &node::Path::new(["session"]).unwrap(),
            data,
            &std::collections::BTreeMap::new(),
        )
    }

    fn write_session(
        &mut self,
        intent: &SessionWriteIntent,
        _: &Envelope,
    ) -> Result<SessionStart, DriverRouteError> {
        self.writes.push((intent.session_ref, intent.write_ref));
        if !self.hold_writes {
            self.events.push(SessionDriverEvent::WriteSettled {
                session: intent.session_ref,
                write: intent.write_ref,
            });
        }
        Ok(SessionStart::Started)
    }

    fn close_session(
        &mut self,
        _: &node::Path,
        session: SessionRef,
    ) -> Result<SessionStart, DriverRouteError> {
        self.close(session);
        Ok(SessionStart::Started)
    }

    fn session_events(&mut self) -> Result<Vec<SessionDriverEvent>, DriverRouteError> {
        Ok(std::mem::take(&mut self.events))
    }
}

#[test]
fn a_websocket_client_session_opens_writes_and_exits_once() {
    let source = r#"
graph websocket : App {
  input in: text
  connector socket : WebSocketClient {
    url = "ws://example.test/socket"
    handshake_timeout = duration("5s")
    codec = text
    effect = idempotent ; behavior = deterministic
  }
  . -> socket -> .
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the WebSocket session probe checks");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = SessionWorld::default();

    vm.inject(&mut journal, text_submission("hello")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    assert_eq!(world.opened.len(), 1);
    assert!(
        journal.live_sessions().is_empty(),
        "the opening is not confirmed yet"
    );
    vm.pump(&mut journal, &mut world).unwrap();
    assert_eq!(journal.live_sessions().len(), 1);

    vm.inject(&mut journal, text_submission("again")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    assert_eq!(world.writes.len(), 1);
    assert_eq!(world.writes[0].0, world.opened[0]);
    vm.pump(&mut journal, &mut world).unwrap();
    assert!(journal.open_session_writes().is_empty());

    world.close(world.opened[0]);
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    assert!(journal.live_sessions().is_empty());
    assert!(vm.is_stable());
    assert_eq!(root_exit_cases(&vm), ["Closed"]);
    let exits = journal
        .records()
        .iter()
        .filter(|record| {
            matches!(
                record,
                Record::Transaction(transaction)
                    if transaction.trigger
                        == node::Trigger::Lifecycle(node::Lifecycle::Exit)
            )
        })
        .count();
    assert_eq!(exits, 1);
}

#[test]
fn a_slow_session_write_cannot_escape_the_worker_mailbox_bound() {
    let source = r#"
graph websocket : App {
  input in: text
  connector socket : WebSocketClient {
    url = "ws://example.test/socket"
    handshake_timeout = duration("5s")
    codec = text
    effect = idempotent ; behavior = deterministic
  }
  . -> socket -> .
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the bounded Session probe checks");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = SessionWorld {
        hold_writes: true,
        ..SessionWorld::default()
    };

    vm.inject(&mut journal, text_submission("open")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();

    for _ in 0..1_024 {
        vm.inject(&mut journal, text_submission("held")).unwrap();
        vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    }
    assert_eq!(world.writes.len(), 1_024);

    vm.inject(&mut journal, text_submission("still-bounded"))
        .unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    assert_eq!(
        world.writes.len(),
        1_024,
        "the durable write table cannot become an unbounded side queue"
    );

    world.settle_write(0);
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    assert_eq!(world.writes.len(), 1_025);
}

#[test]
fn a_pending_websocket_handshake_times_out_without_spinning_or_replaying() {
    let source = r#"
graph websocket : App {
  input in: text
  connector socket : WebSocketClient {
    url = "ws://example.test/socket"
    handshake_timeout = duration("5s")
    codec = text
    effect = idempotent ; behavior = deterministic
  }
  . -> socket -> .
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the WebSocket timeout probe checks");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = SessionWorld {
        pending_opens: true,
        ..SessionWorld::default()
    };

    vm.inject(&mut journal, text_submission("hello")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    assert_eq!(world.opened.len(), 1);
    assert_eq!(journal.open_session_writes().len(), 0);
    vm.pump(&mut journal, &mut world).unwrap();
    assert_eq!(world.opened.len(), 2, "one pump makes one bounded retry");

    vm.advance(&mut journal, &mut world, 5_000_000_000).unwrap();
    assert!(journal.live_sessions().is_empty());
    assert!(vm.is_stable());
    assert_eq!(root_exit_cases(&vm), ["TimeoutExpired"]);
}

#[test]
fn concurrent_inbound_websocket_sessions_never_cross_messages_or_exits() {
    let source = r#"
graph websocket : App {
  input in: text
  connector socket : WebSocketServer {
    listen = "127.0.0.1:0"
    path = "/socket"
    handshake_timeout = duration("5s")
    codec = text
  }
  . -> socket -> .
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the inbound WebSocket session probe checks");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = SessionWorld::default();

    let open = |vm: &mut Vm<'_>, journal: &mut MemJournal, reference| {
        let InboundAdmission::Accepted(accepted) = vm
            .admit_inbound(
                journal,
                &websocket_proposal(reference, "", InboundSession::Open),
            )
            .expect("the bounded opening admits")
        else {
            panic!("the uncongested opening must commit")
        };
        let Some(InboundExchangeRef::Session(session)) = accepted.exchange else {
            panic!("an opening must mint a SessionRef")
        };
        session
    };
    let first = open(&mut vm, &mut journal, 1);
    let second = open(&mut vm, &mut journal, 2);
    assert!(
        vm.outputs().is_empty(),
        "a handshake is causal session state, not an empty peer message"
    );

    for (reference, text, session) in [(3, "alice", first), (4, "bob", second)] {
        assert!(matches!(
            vm.admit_inbound(
                &mut journal,
                &websocket_proposal(reference, text, InboundSession::Message(session)),
            )
            .expect("the message admission is healthy"),
            InboundAdmission::Accepted(_)
        ));
    }
    assert_eq!(vm.outputs().len(), 2);
    for (output, (text, expected)) in vm.outputs().iter().zip([("alice", first), ("bob", second)]) {
        assert_eq!(
            output.envelope.data(),
            &Data::materialized(Value::Text(String::from(text)))
        );
        assert_eq!(
            output.envelope.meta().ext().get("session_id"),
            Some(&rastok::envelope::MetaValue::Scalar(
                rastok::envelope::Scalar::Text(expected.to_string())
            ))
        );
    }

    world.close(second);
    world.close(first);
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    assert!(vm.is_stable());
    assert_eq!(root_exit_cases(&vm), ["Closed", "Closed"]);
    let exit_sessions = vm
        .observations()
        .iter()
        .filter(|observed| observed.node.is_none() && observed.port.name() == "exit")
        .filter_map(
            |observed| match observed.envelope.meta().ext().get("session_id") {
                Some(rastok::envelope::MetaValue::Scalar(rastok::envelope::Scalar::Text(
                    value,
                ))) => Some(value.clone()),
                _ => None,
            },
        )
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        exit_sessions,
        [first.to_string(), second.to_string()]
            .into_iter()
            .collect()
    );
}

#[test]
fn a_value_fault_aborts_the_operator_transaction_and_reaches_supervision() {
    let source = r#"
graph g : App {
  input in: variant
  operator pick {
on in {
  out = data as text
}
  }
  . -> pick -> .
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(&mut journal, submission("salut")).unwrap();
    let error = vm
        .drain(&mut journal)
        .expect_err("a failed narrowing is a VM fault, not a domain result");
    assert!(matches!(error, VmError::Fault(_)), "{error:?}");
    assert!(
        journal.records().iter().all(|record| !matches!(
            record,
            Record::Transaction(transaction)
                if matches!(transaction.trigger, node::Trigger::Port(_))
        )),
        "the faulted transaction must not consume or emit"
    );
    assert!(vm.errors().is_empty(), "no implicit err output exists");
    assert!(
        !vm.is_stable(),
        "the triggering delivery stays reserved for supervision/recovery"
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
    fn prepare_inbound(
        &self,
        submission: InboundSubmission,
    ) -> Result<rastok::journal::PreparedInbound, rastok::journal::Error> {
        self.inner.prepare_inbound(submission)
    }
    fn prepare_connector_inbound(
        &self,
        submission: InboundSubmission,
        session: Option<InboundSession>,
    ) -> Result<rastok::journal::PreparedInbound, rastok::journal::Error> {
        self.inner.prepare_connector_inbound(submission, session)
    }
    fn prepare_session_inbound(
        &self,
        submission: InboundSubmission,
        session: InboundSession,
    ) -> Result<rastok::journal::PreparedInbound, rastok::journal::Error> {
        self.inner.prepare_session_inbound(submission, session)
    }
    fn commit_inbound(
        &mut self,
        prepared: rastok::journal::PreparedInbound,
        routes: Vec<rastok::journal::RouteTarget>,
    ) -> Result<rastok::journal::CommittedInbound, rastok::journal::Error> {
        self.inner.commit_inbound(prepared, routes)
    }
    fn instance_ref_for_admission(
        &self,
        address: &InstanceAddress,
    ) -> Result<InstanceRef, rastok::journal::Error> {
        self.inner.instance_ref_for_admission(address)
    }
    fn loop_run(
        &self,
        run: &rastok::journal::LoopRunRef,
    ) -> Result<Option<rastok::journal::LoopRunState>, rastok::journal::Error> {
        self.inner.loop_run(run)
    }
    fn open_loop_runs(
        &self,
    ) -> Result<
        Vec<(rastok::journal::LoopRunRef, rastok::journal::LoopRunState)>,
        rastok::journal::Error,
    > {
        self.inner.open_loop_runs()
    }
    fn commit(
        &mut self,
        transaction: rastok::journal::TransactionRecord,
    ) -> Result<(), rastok::journal::Error> {
        if self.refuse_commits {
            return Err(rastok::journal::Error::Storage(String::from("scripted")));
        }
        self.inner.commit(transaction)
    }
    fn open_outbound(&mut self, intent: Intent) -> Result<(), rastok::journal::Error> {
        if self.refuse_opens {
            return Err(rastok::journal::Error::Storage(String::from("scripted")));
        }
        self.inner.open_outbound(intent)
    }
    fn open_outbound_session(
        &mut self,
        intent: SessionOpenIntent,
    ) -> Result<(), rastok::journal::Error> {
        self.inner.open_outbound_session(intent)
    }
    fn confirm_session_open(
        &mut self,
        session: &SessionRef,
    ) -> Result<CloseVerdict, rastok::journal::Error> {
        self.inner.confirm_session_open(session)
    }
    fn open_session_write(
        &mut self,
        intent: SessionWriteIntent,
    ) -> Result<(), rastok::journal::Error> {
        self.inner.open_session_write(intent)
    }
    fn settle_session_write(
        &mut self,
        write_ref: &SessionWriteRef,
        abandoned: bool,
    ) -> Result<CloseVerdict, rastok::journal::Error> {
        self.inner.settle_session_write(write_ref, abandoned)
    }
    fn close_outbound(
        &mut self,
        call_ref: &rastok::boundary::CallRef,
        closure: rastok::journal::Closure,
    ) -> Result<rastok::journal::CloseVerdict, rastok::journal::Error> {
        if self.refuse_closes {
            return Err(rastok::journal::Error::Storage(String::from("scripted")));
        }
        self.inner.close_outbound(call_ref, closure)
    }
    fn abandon_outbound(
        &mut self,
        call_ref: &rastok::boundary::CallRef,
    ) -> Result<rastok::journal::CloseVerdict, rastok::journal::Error> {
        if self.refuse_closes {
            return Err(rastok::journal::Error::Storage(String::from("scripted")));
        }
        self.inner.abandon_outbound(call_ref)
    }
    fn find_envelope(&self, id: rastok::envelope::Id) -> Option<Envelope> {
        self.inner.find_envelope(id)
    }
    fn find_call(&self, call_ref: &rastok::boundary::CallRef) -> Option<Intent> {
        self.inner.find_call(call_ref)
    }
    fn open_calls(&self) -> Vec<Intent> {
        self.inner.open_calls()
    }
    fn open_inbound_calls(&self) -> Vec<rastok::journal::InboundCallIngressRecord> {
        self.inner.open_inbound_calls()
    }
    fn live_sessions(&self) -> Vec<SessionRef> {
        self.inner.live_sessions()
    }
    fn open_session_writes(&self) -> Vec<SessionWriteIntent> {
        self.inner.open_session_writes()
    }
}

#[test]
fn a_refused_commit_leaves_the_delivery_replayable() {
    let compilation = compile(
        &[M0_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
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
input in: struct { text: text }
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
fn now_is_fixed_for_one_transaction_and_moves_only_with_logical_time() {
    let source = r#"
graph logical_time : App {
  input in: variant
  output out: struct { first: timestamp, second: timestamp }

  operator stamp {
    input in: variant
    output out: struct { first: timestamp, second: timestamp }
    on in { out = { first = now(); second = now() } }
  }

  . -> stamp -> .
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a handler may observe its logical instant");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot_at(&compilation.plan, GraphVersion(1), 1_000);
    let mut world = EchoWorld::default();

    vm.inject(&mut journal, submission("first")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.advance(&mut journal, &mut world, 2_000).unwrap();
    vm.inject(&mut journal, submission("second")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();

    let instants: Vec<_> = vm
        .outputs()
        .iter()
        .map(|output| match output.envelope.data() {
            Data::Materialized(Value::Struct(fields)) => fields
                .iter()
                .map(|(_, value)| match value {
                    Value::Timestamp(value) => *value,
                    other => panic!("expected a timestamp, got {other:?}"),
                })
                .collect::<Vec<_>>(),
            other => panic!("expected a struct, got {other:?}"),
        })
        .collect();
    assert_eq!(instants, [vec![1_000, 1_000], vec![2_000, 2_000]]);

    let observed: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction) => transaction.logical_time,
            Record::Inbound(_) | Record::Outbound(_) => None,
        })
        .collect();
    assert_eq!(observed, [1_000, 2_000]);
}

#[test]
fn a_filtered_guard_seals_the_logical_time_it_observed() {
    let source = r#"
graph logical_time_guard : App {
  input in: variant
  output out: variant

  operator filter {
    input in: variant
    output out: variant
    on in where now() < timestamp("1970-01-01T00:00:00Z") { out = . }
  }

  . -> filter -> .
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a guard may observe its logical instant");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot_at(&compilation.plan, GraphVersion(1), 1_000);
    let mut world = EchoWorld::default();

    vm.inject(&mut journal, submission("filtered")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();

    assert!(vm.outputs().is_empty(), "the false guard emits nothing");
    let observed: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction) => transaction.logical_time,
            Record::Inbound(_) | Record::Outbound(_) => None,
        })
        .collect();
    assert_eq!(observed, [1_000]);
}

#[test]
fn a_silent_driver_expires_into_a_typed_timeout() {
    let compilation = compile(
        &[TIMED_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    vm.inject(&mut journal, submission("salut")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    assert!(vm.has_open_obligations());
    // Nobody answers; the budget expires at the discrete advance.
    vm.advance(&mut journal, &mut world, 5_000_000_000).unwrap();
    assert!(vm.is_stable(), "the expiry released the machine");
    assert_eq!(vm.now(), 5_000_000_000);
    // The closure is a Timeout fact and the completion transaction
    // fired under l:timeout, with TimeoutExpired on `exit`.
    let last = journal.records().last().unwrap();
    let Record::Transaction(transaction) = last else {
        panic!("expected the timeout transaction");
    };
    assert_eq!(
        transaction.trigger,
        node::Trigger::Lifecycle(node::Lifecycle::Timeout)
    );
    assert_eq!(transaction.emissions[0].port.name(), "exit");
    assert!(vm.errors().is_empty());
    assert_eq!(root_exit_cases(&vm), ["TimeoutExpired"]);
    // The claim lifted: a second submission opens a second call.
    vm.inject(&mut journal, submission("encore")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    assert_eq!(journal.open_calls().len(), 1);
}

#[test]
fn a_scheduler_projection_refusal_precedes_the_timeout_close() {
    let compilation = compile(
        &[TIMED_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut bindings = ProjectionRefusal {
        inner: EchoWorld::default(),
        refuse_scheduler_completion: false,
        refuse_cancel: false,
    };
    vm.inject(&mut journal, submission("salut")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut bindings).unwrap();
    let facts_before = journal.records().len();
    assert_eq!(journal.open_calls().len(), 1);

    bindings.refuse_scheduler_completion = true;
    assert!(matches!(
        vm.advance(&mut journal, &mut bindings, 5_000_000_000),
        Err(VmError::Dispatch(rastok::boundary::DispatchError::Driver(
            DriverRouteError::UnavailableOutbound(_)
        )))
    ));
    assert_eq!(journal.records().len(), facts_before);
    assert_eq!(journal.open_calls().len(), 1, "the call remains replayable");

    bindings.refuse_scheduler_completion = false;
    vm.advance(&mut journal, &mut bindings, 5_000_000_000)
        .unwrap();
    assert_eq!(root_exit_cases(&vm), ["TimeoutExpired"]);
}

#[test]
fn a_cancel_refusal_cannot_strand_an_expiring_call() {
    let compilation = compile(
        &[TIMED_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut bindings = ProjectionRefusal {
        inner: EchoWorld::default(),
        refuse_scheduler_completion: false,
        refuse_cancel: true,
    };
    vm.inject(&mut journal, submission("salut")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut bindings).unwrap();

    assert!(matches!(
        vm.advance(&mut journal, &mut bindings, 5_000_000_000),
        Err(VmError::Dispatch(rastok::boundary::DispatchError::Driver(
            DriverRouteError::UnavailableOutbound(_)
        )))
    ));
    assert_eq!(
        journal.open_calls().len(),
        1,
        "a refused cancellation leaves the call and deadline replayable"
    );

    bindings.refuse_cancel = false;
    vm.advance(&mut journal, &mut bindings, 5_000_000_000)
        .unwrap();
    assert_eq!(root_exit_cases(&vm), ["TimeoutExpired"]);
    assert!(
        vm.is_stable(),
        "the claimed mailbox was eventually released"
    );
}

#[test]
fn a_budget_runs_from_delivery_and_covers_queue_time() {
    // The claim serializes the connector: the second request
    // waits in the mailbox behind the open call. Its budget runs
    // anyway — queue time counts (execution-model §8) — and the
    // call-closed rule applies even to a call that expires
    // before it started: the queued request's call opens and
    // closes at the expiry, back to back.
    let compilation = compile(
        &[TIMED_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    vm.inject(&mut journal, submission("un")).unwrap();
    vm.inject(&mut journal, submission("deux")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    assert_eq!(journal.open_calls().len(), 1, "one open, one queued");
    // Nobody answers; both budgets expire at the same sweep.
    vm.advance(&mut journal, &mut world, 5_000_000_000).unwrap();
    assert!(vm.is_stable());
    assert_eq!(
        root_exit_cases(&vm),
        ["TimeoutExpired", "TimeoutExpired"],
        "both requests expired exactly once"
    );
    let statuses: Vec<&OutboundStatus> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Outbound(OutboundRecord::Call(o)) => Some(&o.status),
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
                OutboundStatus::Closed(rastok::journal::Closure::Timeout)
            ))
            .count(),
        2,
        "both closed by the machine's Timeout fact"
    );
    // Both completion transactions fired under the timeout
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
input in: struct { text: text }
on in { out = data.text }
  }

  . -> shout
  shout -> alpha -> nothing
  shout -> beta -> nothing
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    vm.inject(&mut journal, submission("salut")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
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
    let compilation = compile(
        &[TIMED_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    vm.inject(&mut journal, submission("salut")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    // The driver answers first; the expiry then loses the race —
    // one closure fact, one completion transaction, no duplicate.
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.advance(&mut journal, &mut world, 5_000_000_000).unwrap();
    assert!(vm.is_stable());
    let closures = journal
        .records()
        .iter()
        .filter(|record| {
            matches!(
                record,
                Record::Outbound(OutboundRecord::Call(o))
                    if matches!(o.status, OutboundStatus::Closed(_))
            )
        })
        .count();
    assert_eq!(closures, 1, "close-once");
    assert_eq!(vm.outputs().len(), 1);
    assert!(vm.errors().is_empty(), "no phantom timeout");
}

#[test]
fn a_late_answer_is_a_reported_observation_not_a_silence() {
    let compilation = compile(
        &[TIMED_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    vm.inject(&mut journal, submission("salut")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    let call_ref = journal.open_calls()[0].call_ref;
    // The budget expires first…
    vm.advance(&mut journal, &mut world, 5_000_000_000).unwrap();
    // …then the world answers a closed call.
    let mut late = ScriptedWorld {
        answers: vec![Response {
            call_ref,
            completion: Completion::process(
                vec![Data::encoded(Encoding::Raw, *b"tard")],
                vec![],
                ProcessExit::Exited {
                    exit_code: 0,
                    signal: None,
                },
            ),
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
    let compilation = compile(
        &[TIMED_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = FlakyJournal {
        refuse_closes: true,
        ..FlakyJournal::healthy(GraphVersion(1))
    };
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    vm.inject(&mut journal, submission("salut")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    assert!(
        vm.advance(&mut journal, &mut world, 5_000_000_000).is_err(),
        "the refused closure surfaces"
    );
    // Heal and replay: the timer was never acked.
    journal.refuse_closes = false;
    vm.advance(&mut journal, &mut world, 5_000_000_000).unwrap();
    assert!(vm.is_stable());
    assert_eq!(root_exit_cases(&vm), ["TimeoutExpired"]);
}

#[test]
fn a_refused_open_on_a_queued_expiry_replays_without_duplicates() {
    // Case: the queued delivery's open fact is refused at expiry.
    let compilation = compile(
        &[TIMED_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = FlakyJournal::healthy(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    vm.inject(&mut journal, submission("un")).unwrap();
    vm.inject(&mut journal, submission("deux")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    assert_eq!(journal.inner.open_calls().len(), 1, "one open, one queued");
    journal.refuse_opens = true;
    assert!(vm.advance(&mut journal, &mut world, 5_000_000_000).is_err());
    journal.refuse_opens = false;
    vm.advance(&mut journal, &mut world, 5_000_000_000).unwrap();
    assert!(vm.is_stable());
    assert_eq!(
        root_exit_cases(&vm),
        ["TimeoutExpired", "TimeoutExpired"],
        "both expired exactly once"
    );
}

#[test]
fn a_refused_commit_on_a_queued_expiry_loses_nothing() {
    // Case: the queued delivery expires and the timeout
    // transaction's commit is refused — the delivery must stay.
    let compilation = compile(
        &["graph g : App {\n  connector echo : Process {\n    mode = job ; command = \"cat\"\n    codec = lines\n    effect = idempotent ; behavior = deterministic\n    timeout = duration(\"5s\")\n  }\n  operator wrap {\n    on in { out = . }\n  }\n  . -> wrap -> echo -> nothing\n}\n"],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = FlakyJournal::healthy(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    vm.inject(&mut journal, text_submission("salut")).unwrap();
    // Drive only the wrap transaction, then expire the conforming
    // delivery while it is still queued. B2's ingress gate makes the
    // former unframeable-payload setup unreachable by construction.
    while !journal.inner.records().iter().any(|record| {
        matches!(
            record,
            Record::Transaction(transaction)
                if transaction.owner_path.to_string() == "wrap"
                    && matches!(transaction.trigger, node::Trigger::Port(_))
        )
    }) {
        assert!(vm.step(&mut journal).unwrap());
    }
    let facts_before = journal.inner.records().len();
    journal.refuse_commits = true;
    assert!(vm.advance(&mut journal, &mut world, 5_000_000_000).is_err());
    assert!(
        journal.inner.records()[facts_before..]
            .iter()
            .all(|record| !matches!(record, Record::Transaction(_))),
        "the open and closure facts may commit, but the refused transaction may not"
    );
    // Heal and replay: the delivery was never removed, the timer
    // never acked — exactly one typed terminal fact results.
    journal.refuse_commits = false;
    vm.advance(&mut journal, &mut world, 5_000_000_000).unwrap();
    vm.drain(&mut journal).unwrap();
    assert!(vm.is_stable());
    assert!(vm.errors().is_empty());
    assert_eq!(root_exit_cases(&vm), ["TimeoutExpired"]);
    let terminal_transactions: Vec<_> = journal
        .inner
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(t)
                if t.owner_path.to_string() == "echo" && !t.consumed.is_empty() =>
            {
                Some(t)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        terminal_transactions.len(),
        1,
        "exactly one transaction consumed the delivery"
    );
    assert_eq!(terminal_transactions[0].consumed.len(), 1);
}

#[test]
fn a_suspended_call_keeps_its_mailbox_claimed() {
    let compilation = compile(
        &[M0_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    vm.inject(&mut journal, submission("un")).unwrap();
    vm.inject(&mut journal, submission("deux")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    // Two requests reached the connector, but only one call is
    // open: the second waits for the mailbox claim to lift.
    assert_eq!(journal.open_calls().len(), 1);
    assert!(vm.has_open_obligations());
    // Settle, then the claim lifts and the second call opens.
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    assert_eq!(journal.open_calls().len(), 1, "the second call is now open");
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain(&mut journal).unwrap();
    assert!(vm.is_stable());
    assert_eq!(vm.outputs().len(), 2);
}

#[test]
fn a_filtered_input_reaches_stability_without_output() {
    let compilation = compile(
        &[M0_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
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

#[test]
fn loop_delay_dates_reentry_and_recovers_the_same_wakeup() {
    let compilation = compile(
        &[r#"
graph countdown : App + Loop {
  input in: int
  output out: int
  Loop.fuel = 3
  Loop.delay = { kind = fixed ; after = duration("2s") }

  on in where data <= 0 { out = Loop.state.data }
  on in where data > 0  { next = data - 1 }
}
"#],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a fixed-delay Loop is executable");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut before_restart = Vm::boot_at(&compilation.plan, GraphVersion(1), 0);
    before_restart
        .inject(&mut journal, int_submission(2))
        .expect("the root is admitted");
    before_restart
        .drain(&mut journal)
        .expect("the first traversal arms, rather than runs, its successor");
    let (run, first) = journal
        .open_loop_runs()
        .expect("the Loop fold is valid")
        .into_iter()
        .next()
        .expect("one run remains open");
    assert_eq!(first.iteration, 1);
    assert_eq!(first.due_at, Some(2_000_000_000));
    assert!(before_restart.outputs().is_empty());
    drop(before_restart);

    let mut recovered = Vm::boot_at(&compilation.plan, GraphVersion(1), 1_000_000_000);
    recovered
        .recover_loop_runs(&journal)
        .expect("the persisted due-date annotation re-arms the same wakeup");
    recovered
        .recover_loop_runs(&journal)
        .expect("a duplicate recovery observation may re-arm but cannot re-enter twice");
    let mut world = EchoWorld::default();
    recovered
        .advance(&mut journal, &mut world, 1_999_999_999)
        .expect("advancing short of the date is inert");
    assert_eq!(journal.loop_run(&run).unwrap().unwrap().iteration, 1);
    recovered
        .advance(&mut journal, &mut world, 2_000_000_000)
        .expect("the exact date admits one traversal");
    let second = journal.loop_run(&run).unwrap().unwrap();
    assert_eq!(second.iteration, 2);
    assert_eq!(second.due_at, Some(4_000_000_000));
    recovered
        .advance(&mut journal, &mut world, 4_000_000_000)
        .expect("the second exact date reaches the terminal branch");
    assert_eq!(recovered.outputs().len(), 1);
    assert_eq!(
        rastok::runtime::data_value(recovered.outputs()[0].envelope.data()).unwrap(),
        Value::Int(0)
    );
    assert_eq!(
        journal.loop_run(&run).unwrap().unwrap().phase,
        rastok::journal::LoopRunPhase::Closed
    );
    let delayed_facts: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction)
                if matches!(
                    transaction.loop_fact.as_deref(),
                    Some(rastok::journal::LoopFact::Next { .. })
                ) =>
            {
                Some(transaction)
            }
            _ => None,
        })
        .collect();
    assert_eq!(delayed_facts.len(), 2);
    assert!(delayed_facts.iter().all(|transaction| {
        transaction.timers.len() == 1
            && transaction.timers[0].0 == rastok::journal::timer_kind::LOOP_DELAY
    }));
}

#[test]
fn exponential_loop_delay_saturates_without_extending_fuel() {
    let compilation = compile(
        &[r#"
graph countdown : App + Loop {
  input in: int
  output out: int
  Loop.fuel = 4
  Loop.delay = {
    kind = exponential
    first = duration("1s")
    ceiling = duration("3s")
  }

  on in where data <= 0 { out = Loop.state.data }
  on in where data > 0  { next = data - 1 }
}
"#],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("an exponential-delay Loop is executable");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    vm.inject(&mut journal, int_submission(3)).unwrap();
    vm.drain(&mut journal).unwrap();
    let run = journal.open_loop_runs().unwrap()[0].0;
    assert_eq!(
        journal.loop_run(&run).unwrap().unwrap().due_at,
        Some(1_000_000_000)
    );
    vm.advance(&mut journal, &mut world, 1_000_000_000).unwrap();
    assert_eq!(
        journal.loop_run(&run).unwrap().unwrap().due_at,
        Some(3_000_000_000)
    );
    vm.advance(&mut journal, &mut world, 3_000_000_000).unwrap();
    assert_eq!(
        journal.loop_run(&run).unwrap().unwrap().due_at,
        Some(6_000_000_000)
    );
    vm.advance(&mut journal, &mut world, 6_000_000_000).unwrap();
    assert_eq!(vm.outputs().len(), 1);
    assert_eq!(journal.loop_run(&run).unwrap().unwrap().remaining, 1);
}

#[test]
fn instance_closing_fences_a_delayed_loop_from_the_successor_generation() {
    let compilation = compile(
        &[r#"
graph countdown : App + Loop {
  input in: int
  output out: int
  lifetime = { kind = fixed ; after = duration("1s") }
  Loop.fuel = 3
  Loop.delay = { kind = fixed ; after = duration("2s") }

  on in where data <= 0 { out = Loop.state.data }
  on in where data > 0  { next = data - 1 }
}
"#],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("Loop delay and instance lifetime are orthogonal");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    vm.inject(&mut journal, int_submission(1)).unwrap();
    vm.drain(&mut journal).unwrap();
    let old_run = journal.open_loop_runs().unwrap()[0].0;

    vm.advance(&mut journal, &mut world, 1_000_000_000)
        .expect("the lifetime closes before the later Loop wakeup");
    vm.inject(&mut journal, int_submission(0))
        .expect("the next root reserves a clean successor generation");
    vm.drain(&mut journal)
        .expect("the successor run reaches its own terminal branch");
    assert_eq!(vm.outputs().len(), 1);
    vm.advance(&mut journal, &mut world, 2_000_000_000)
        .expect("the stale predecessor wakeup is a harmless observation");
    assert_eq!(vm.outputs().len(), 1, "the stale wakeup did not re-enter");
    let old = journal.loop_run(&old_run).unwrap().unwrap();
    assert_eq!(old.iteration, 1);
    assert_eq!(old.phase, rastok::journal::LoopRunPhase::Open);
}

#[test]
fn equal_date_loop_wakeups_keep_their_causal_arm_order() {
    let compilation = compile(
        &[r#"
graph countdown : App + Loop {
  input in: int
  output out: int
  Loop.fuel = 3
  Loop.delay = { kind = fixed ; after = duration("1s") }

  on in where data <= 0 { out = Loop.state.data }
  on in where data > 0  { next = data - 1 }
}
"#],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a delayed Loop is executable");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(&mut journal, int_submission(1)).unwrap();
    vm.inject(&mut journal, int_submission(2)).unwrap();
    vm.drain(&mut journal).unwrap();
    let runs: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction) => match transaction.loop_fact.as_deref() {
                Some(rastok::journal::LoopFact::Open { run, .. }) => Some(*run),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(runs.len(), 2);
    let before = journal.records().len();
    vm.advance(&mut journal, &mut EchoWorld::default(), 1_000_000_000)
        .unwrap();
    let transitions: Vec<_> = journal.records()[before..]
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction) => transaction.loop_fact.as_deref().filter(|fact| {
                matches!(
                    fact,
                    rastok::journal::LoopFact::Next { .. }
                        | rastok::journal::LoopFact::Close { .. }
                )
            }),
            _ => None,
        })
        .collect();
    assert!(matches!(
        transitions.as_slice(),
        [
            rastok::journal::LoopFact::Close { run: first, .. },
            rastok::journal::LoopFact::Next { run: second, .. }
        ] if *first == runs[0] && *second == runs[1]
    ));
}

#[test]
fn loop_delay_starts_only_after_the_call_timeout_exit_decides_next() {
    let compilation = compile(
        &[r#"
graph retry : App + Loop {
  input in: text
  output done: text
  Loop.fuel = 2
  Loop.delay = { kind = fixed ; after = duration("2s") }

  connector shell : Process {
    mode = job
    command = "cat"
    codec = lines
    effect = idempotent
    behavior = deterministic
    timeout = duration("1s")
    max_output = "1 MiB"
  }

  . -> shell
  shell.out -> nothing
  shell.err -> nothing
  on shell.exit where data is CaptureFailed  { done = "capture" }
  on shell.exit where data is DecodeFailed   { done = "decode" }
  on shell.exit where data is Exited         { done = "exited" }
  on shell.exit where data is OutcomeUnknown { done = "unknown" }
  on shell.exit where data is OutputOverflow { done = "overflow" }
  on shell.exit where data is SpawnFailed    { done = "spawn" }
  on shell.exit where data is TimeoutExpired { next = Loop.state.data }
}
"#],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a bounded Call may decide one Loop traversal");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = ClosingWorld::default();

    vm.inject(&mut journal, text_submission("again")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    assert_eq!(journal.open_calls().len(), 1);
    let run = journal.open_loop_runs().unwrap()[0].0;

    vm.advance(&mut journal, &mut world, 1_000_000_000)
        .expect("the Call budget publishes TimeoutExpired first");
    let after_timeout = journal.loop_run(&run).unwrap().unwrap();
    assert_eq!(after_timeout.iteration, 1);
    assert_eq!(after_timeout.due_at, Some(3_000_000_000));
    assert!(!after_timeout.iteration_admitted);
    assert!(journal.open_calls().is_empty());

    vm.advance(&mut journal, &mut world, 2_999_999_999)
        .expect("the Loop delay has not elapsed");
    assert!(journal.open_calls().is_empty());
    vm.advance(&mut journal, &mut world, 3_000_000_000)
        .expect("the delayed state re-enters the spatial Call body");
    assert_eq!(journal.open_calls().len(), 1);

    vm.advance(&mut journal, &mut world, 4_000_000_000)
        .expect("the second independent Call timeout exhausts fuel");
    assert!(journal.open_calls().is_empty());
    assert_eq!(vm.outputs().len(), 1);
    assert!(matches!(
        journal.loop_run(&run).unwrap().unwrap().phase,
        rastok::journal::LoopRunPhase::Expired
    ));

    let call_timeouts = journal
        .records()
        .iter()
        .filter(|record| {
            matches!(
                record,
                Record::Outbound(OutboundRecord::Call(call))
                    if matches!(
                        call.status,
                        OutboundStatus::Closed(rastok::journal::Closure::Timeout)
                    )
            )
        })
        .count();
    assert_eq!(
        call_timeouts, 2,
        "each body application owns one Call timeout"
    );
}

#[test]
fn spatial_loop_recovery_waits_for_general_instance_recovery() {
    let compilation = compile(
        &[r#"
graph retry : App + Loop {
  input in: text
  output done: text
  Loop.fuel = 2
  Loop.delay = { kind = fixed ; after = duration("2s") }

  connector shell : Process {
    mode = job
    command = "cat"
    codec = lines
    effect = idempotent
    behavior = deterministic
    timeout = duration("1s")
    max_output = "1 MiB"
  }

  . -> shell
  shell.out -> nothing
  shell.err -> nothing
  on shell.exit where data is CaptureFailed  { done = "capture" }
  on shell.exit where data is DecodeFailed   { done = "decode" }
  on shell.exit where data is Exited         { done = "exited" }
  on shell.exit where data is OutcomeUnknown { done = "unknown" }
  on shell.exit where data is OutputOverflow { done = "overflow" }
  on shell.exit where data is SpawnFailed    { done = "spawn" }
  on shell.exit where data is TimeoutExpired { next = Loop.state.data }
}
"#],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the spatial Loop is valid");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = ClosingWorld::default();

    vm.inject(&mut journal, text_submission("again")).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.advance(&mut journal, &mut world, 1_000_000_000)
        .expect("the first Call timeout selects delayed recurrence");
    drop(vm);

    let mut recovered = Vm::boot_at(&compilation.plan, GraphVersion(1), 2_000_000_000);
    let error = recovered
        .recover_loop_runs(&journal)
        .expect_err("spatial recovery must fail closed until owner bundles are replayed");
    assert!(
        error.to_string().contains(
            "a spatial Loop waits for general instance and Connector obligation recovery"
        )
    );
}
