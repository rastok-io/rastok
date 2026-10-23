//! The Execution end to end, driven through compiled probes — integration
//! tests of the core machine, living in the std crate because they
//! need the compiler.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
// Harness helpers live outside `#[test]` bodies, where the
// allow-unwrap-in-tests toggle does not reach; a broken fixture's
// correct report is a panic.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "support/call_driver.rs"]
mod call_driver;
use call_driver::CallDriver;

use rastok::connector::driver::DriverRouteError;
use rastok::connector::driver::OutboundCall;
use rastok::connector::driver::OutboundDrivers;
use rastok::connector::exchange::CallOpen;
use rastok::connector::exchange::CallStart;
use rastok::connector::exchange::InboundExchangeId;
use rastok::connector::exchange::InboundSession;
use rastok::connector::exchange::IngressAdmission;
use rastok::connector::exchange::IngressProposal;
use rastok::connector::exchange::IngressProposalHandle;
use rastok::connector::exchange::PendingIngress;
use rastok::connector::exchange::PreparedRequest;
use rastok::connector::exchange::SessionId;
use rastok::connector::exchange::SessionObservation;
use rastok::connector::exchange::SessionOpen;
use rastok::connector::exchange::SessionStart;
use rastok::connector::exchange::SessionWrite;
use rastok::connector::exchange::SessionWriteId;
use rastok::connector::stdlib::WebSocketExit;
use rastok::envelope::{Data, Envelope};
use rastok::instance;

use rastok::journal::Journal;
use rastok::node;
use rastok::plan::{AttrSetting, EvaluationSite, GraphVersion, OwnerValue, ValidatedExecutionPlan};
use rastok::runtime::execution::{Execution, ExecutionFault};

use rastok::compiler::compile;
use rastok::connector::contract::ContractCatalog;
use rastok::connector::exchange::CallResponse;
use rastok::connector::exchange::Completion;
use rastok::connector::exchange::EnvelopeDraft;
use rastok::connector::exchange::IngressProvenance;
use rastok::connector::exchange::IngressSequence;
use rastok::connector::exchange::RequestPreparation;
use rastok::connector::exchange::SchedulerCompletion;
use rastok::connector::stdlib::ProcessExit;
use rastok::envelope::{Encoding, Namespace, Value};
use rastok::journal::loop_protocol::{Fact as LoopFact, LoopView as LoopFold, Phase as LoopPhase};
use rastok::journal::{CallStatus, CloseVerdict, EgressRecord, MemJournal, Record};

fn loop_state(
    journal: &MemJournal,
    run: rastok::journal::LoopRunId,
) -> rastok::journal::loop_protocol::Run {
    LoopFold::from_records(journal.records())
        .unwrap()
        .run(run)
        .unwrap()
        .clone()
}

fn first_loop(journal: &MemJournal) -> rastok::journal::LoopRunId {
    *LoopFold::from_records(journal.records())
        .unwrap()
        .runs()
        .next()
        .unwrap()
        .0
}

fn int_address(owner: &str, value: i64) -> instance::InstanceAddress {
    let owner = node::OwnerPath::new([owner]).expect("valid owner path");
    instance::InstanceAddress::new(
        &owner,
        &[instance::Segment::SelectedKey {
            scope: owner.clone(),
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
idempotent = true; behavior = deterministic
  }

  operator shout {
input in: struct { text: text }
var count : int = 0
on in where data.text != "" {
  count = count + 1
  out = data.text
}
  }

  in -> shout -> echo -> out
  echo.err -> nothing
  echo.exit -> nothing
}
"#;

/// The world scripted: echoes every request back, like `cat`.
#[derive(Default)]
struct EchoWorld {
    seen: Vec<CallOpen>,
    fail_next: bool,
}

struct ProjectionRefusal {
    inner: EchoWorld,
    refuse_scheduler_completion: bool,
    refuse_cancel: bool,
}

impl OutboundDrivers for ProjectionRefusal {
    fn prepare_request(
        &mut self,
        connector: &node::Path,
        data: &Data,
        resolved_settings: &std::collections::BTreeMap<String, rastok::value::Value>,
    ) -> Result<RequestPreparation, DriverRouteError> {
        Ok(OutboundCall::prepare_request(
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
        Ok(OutboundCall::scheduler_completion(
            &mut self.inner,
            connector,
            completion,
        ))
    }

    fn cancel(
        &mut self,
        connector: &node::Path,
        call_id: &rastok::connector::exchange::CallId,
    ) -> Result<(), DriverRouteError> {
        if self.refuse_cancel {
            return Err(DriverRouteError::UnavailableOutbound(connector.clone()));
        }
        OutboundCall::cancel(&mut self.inner, call_id);
        Ok(())
    }

    fn start(
        &mut self,
        intent: &CallOpen,
        request: &Envelope,
    ) -> Result<CallStart, DriverRouteError> {
        Ok(OutboundCall::start(&mut self.inner, intent, request))
    }

    fn poll_responses(&mut self) -> Result<Vec<CallResponse>, DriverRouteError> {
        Ok(OutboundCall::poll_responses(&mut self.inner))
    }
}

/// A boundary seam that records deterministic cancellation order. It can
/// either confirm each cancellation with an observed normal exit or remain
/// silent so bounded settlement has to produce `OutcomeUnknown`.
#[derive(Default)]
struct ClosingWorld {
    started: Vec<rastok::connector::exchange::CallId>,
    cancelled: Vec<rastok::connector::exchange::CallId>,
    answers: Vec<CallResponse>,
    confirm_cancel: bool,
}

impl OutboundCall for ClosingWorld {
    fn prepare_request(
        &mut self,
        _: &node::Path,
        data: &Data,
        _: &std::collections::BTreeMap<String, rastok::value::Value>,
    ) -> RequestPreparation {
        rastok::connector::stdlib::prepare_process_request(
            rastok::connector::stdlib::RequestCodec::Lines,
            data,
        )
    }

    fn scheduler_completion(
        &mut self,
        _: &node::Path,
        completion: SchedulerCompletion,
    ) -> Completion {
        rastok::connector::stdlib::process_scheduler_completion(completion)
    }

    fn start(&mut self, intent: &CallOpen, _: &Envelope) -> CallStart {
        self.started.push(intent.call_id);
        CallStart::Started
    }

    fn cancel(&mut self, call_id: &rastok::connector::exchange::CallId) {
        self.cancelled.push(*call_id);
        if self.confirm_cancel {
            self.answers.push(CallResponse {
                call_id: *call_id,
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

    fn poll_responses(&mut self) -> Vec<CallResponse> {
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
    idempotent = true
    behavior = deterministic
  }

  in -> shell
  shell.out -> nothing
  shell.err -> nothing
  shell.exit -> exit
}
"#;

impl OutboundCall for EchoWorld {
    fn prepare_request(
        &mut self,
        _: &node::Path,
        data: &Data,
        _: &std::collections::BTreeMap<String, rastok::value::Value>,
    ) -> RequestPreparation {
        rastok::connector::stdlib::prepare_process_request(
            rastok::connector::stdlib::RequestCodec::Lines,
            data,
        )
    }

    fn scheduler_completion(
        &mut self,
        _: &node::Path,
        completion: SchedulerCompletion,
    ) -> Completion {
        rastok::connector::stdlib::process_scheduler_completion(completion)
    }

    fn start(&mut self, intent: &CallOpen, request: &Envelope) -> CallStart {
        let _ = request;
        self.seen.push(intent.clone());
        CallStart::Started
    }

    fn poll_responses(&mut self) -> Vec<CallResponse> {
        self.seen
            .drain(..)
            .map(|intent| CallResponse {
                call_id: intent.call_id,
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

impl OutboundCall for FileSettingsWorld {
    fn prepare_request(
        &mut self,
        _: &node::Path,
        _: &Data,
        settings: &std::collections::BTreeMap<String, Value>,
    ) -> RequestPreparation {
        self.prepared.push(settings.clone());
        RequestPreparation::Prepared(rastok::connector::exchange::PreparedRequest::seal(
            Vec::new(),
        ))
    }

    fn scheduler_completion(&mut self, _: &node::Path, _: SchedulerCompletion) -> Completion {
        Completion::try_new(vec![EnvelopeDraft {
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

    fn start(&mut self, _: &CallOpen, _: &Envelope) -> CallStart {
        CallStart::Started
    }

    fn poll_responses(&mut self) -> Vec<CallResponse> {
        Vec::new()
    }
}

#[test]
fn inspection_is_a_detached_read_only_snapshot() {
    let compilation = compile(
        &[r#"graph inspect : App {
          input in: int
          output out: int
          operator count {
            input in: int
            output out: int
            var total: int = 0
            on in { total = total + data ; out = total }
          }
          in -> count -> out
        }"#],
        &ContractCatalog::try_m0().unwrap(),
    )
    .unwrap();
    let version = GraphVersion(1);
    let mut execution = Execution::new(&compilation.plan, version);
    let mut journal = MemJournal::new(version);
    execution.run_ready(&mut journal).unwrap();
    execution
        .accept_input(&mut journal, int_submission(7))
        .unwrap();
    let records = journal.records().to_vec();
    let snapshot = execution.inspection();
    let retained = snapshot.clone();
    assert_eq!(snapshot, execution.inspection());
    assert_eq!(records, journal.records());
    assert_eq!(
        snapshot
            .mailboxes
            .iter()
            .map(|mailbox| mailbox.deliveries.len())
            .sum::<usize>(),
        1
    );

    execution.run_ready(&mut journal).unwrap();
    assert_eq!(
        snapshot, retained,
        "later execution cannot update an owned view"
    );
    assert_ne!(snapshot, execution.inspection());
    assert!(
        execution
            .inspection()
            .mailboxes
            .iter()
            .all(|mailbox| mailbox.deliveries.is_empty())
    );
    assert!(
        execution
            .inspection()
            .instances
            .iter()
            .any(|instance| instance.state.get("total") == Some(&Value::Int(7)))
    );
}

fn submission(text: &str) -> IngressProposal {
    let data = vec![(String::from("text"), Value::Text(String::from(text)))];
    IngressProposal {
        provenance: IngressProvenance::single(node::Path::new(["cli"]).unwrap()),
        draft: EnvelopeDraft {
            port: rastok::node::Port::new("in").unwrap(),
            data: Data::materialized(Value::Struct(data)),
            ext: Namespace::new(),
        },
        sequence: IngressSequence::Next,
        session: None,
    }
}

fn text_submission(text: &str) -> IngressProposal {
    IngressProposal {
        provenance: IngressProvenance::single(node::Path::new(["cli"]).unwrap()),
        draft: EnvelopeDraft {
            port: rastok::node::Port::new("in").unwrap(),
            data: Data::materialized(Value::Text(String::from(text))),
            ext: Namespace::new(),
        },
        sequence: IngressSequence::Next,
        session: None,
    }
}

fn websocket_proposal(reference: u64, text: &str, session: InboundSession) -> PendingIngress {
    PendingIngress {
        handle: IngressProposalHandle::new(reference),
        proposal: IngressProposal {
            provenance: IngressProvenance::single(
                node::Path::new(["socket"]).expect("the fixture path is valid"),
            ),
            draft: EnvelopeDraft {
                port: node::Port::conventional_out(),
                data: Data::materialized(Value::Text(String::from(text))),
                ext: Namespace::new(),
            },
            sequence: IngressSequence::Next,
            session: Some(session),
        },
    }
}

fn int_submission(value: i64) -> IngressProposal {
    IngressProposal {
        provenance: IngressProvenance::single(node::Path::new(["cli"]).unwrap()),
        draft: EnvelopeDraft {
            port: rastok::node::Port::new("in").unwrap(),
            data: Data::materialized(Value::Int(value)),
            ext: Namespace::new(),
        },
        sequence: IngressSequence::Next,
        session: None,
    }
}

fn port_int_submission(port: &str, value: i64) -> IngressProposal {
    IngressProposal {
        provenance: IngressProvenance::single(node::Path::new(["cli"]).unwrap()),
        draft: EnvelopeDraft {
            port: rastok::node::Port::new(port).unwrap(),
            data: Data::materialized(Value::Int(value)),
            ext: Namespace::new(),
        },
        sequence: IngressSequence::Next,
        session: None,
    }
}

fn encoded_submission(
    port: &str,
    encoding: Encoding,
    bytes: &'static [u8],
    content_type: &str,
) -> IngressProposal {
    let mut ext = Namespace::new();
    ext.insert(
        String::from("content_type"),
        rastok::envelope::MetaValue::Scalar(rastok::envelope::Scalar::Text(String::from(
            content_type,
        ))),
    );
    IngressProposal {
        provenance: IngressProvenance::single(node::Path::new(["cli"]).unwrap()),
        draft: EnvelopeDraft {
            port: rastok::node::Port::new(port).unwrap(),
            data: Data::encoded(encoding, bytes),
            ext,
        },
        sequence: IngressSequence::Next,
        session: None,
    }
}

fn tenant_submission(tenant: &str, user: i64) -> IngressProposal {
    IngressProposal {
        provenance: IngressProvenance::single(node::Path::new(["cli"]).unwrap()),
        draft: EnvelopeDraft {
            port: rastok::node::Port::new("in").unwrap(),
            data: Data::materialized(Value::Struct(vec![
                (String::from("tenant"), Value::Text(String::from(tenant))),
                (String::from("user"), Value::Int(user)),
            ])),
            ext: Namespace::new(),
        },
        sequence: IngressSequence::Next,
        session: None,
    }
}

fn source_exit_cases<'a>(execution: &'a Execution<'_>) -> Vec<&'a str> {
    execution
        .envelope_views()
        .iter()
        .filter_map(|observed| {
            if observed.node.is_none() || observed.port.name() != "exit" {
                return None;
            }
            let Data::Materialized(Value::TaggedUnion(tagged)) = observed.envelope.data() else {
                return None;
            };
            Some(tagged.case.as_str())
        })
        .collect()
}

fn run_probe() -> (MemJournal, Vec<rastok::runtime::execution::BoundaryOutput>) {
    let compilation = compile(
        &[M0_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    execution
        .accept_input(&mut journal, submission("salut"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    execution
        .poll_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    assert!(execution.is_stable());
    (journal, execution.outputs().to_vec())
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

  in -> file
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
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = FileSettingsWorld::default();

    execution
        .accept_input(&mut journal, text_submission("objects/report.bin"))
        .expect("the text path is admitted");
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
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
            Record::Ingress(_) => String::from("inbound"),
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
            Record::Egress(EgressRecord::Call(o)) => match o.status {
                CallStatus::Open => String::from("outbound:open"),
                CallStatus::Closed(_) => String::from("outbound:closed"),
                CallStatus::Abandoned => String::from("outbound:abandoned"),
            },
            Record::Egress(EgressRecord::Session(_)) => String::from("session"),
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
    let Record::Ingress(root) = &journal.records()[0] else {
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

  in -> first -> second -> out
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("R3 admits a persistent Graph-owned key");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    execution
        .accept_input(&mut journal, int_submission(7))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();

    let Record::Ingress(inbound) = &journal.records()[0] else {
        panic!("the first fact is ingress");
    };
    let root_id = inbound.envelope.meta().sys().id();
    let frame = instance::Segment::SelectedKey {
        scope: node::OwnerPath::root(),
        ty: rastok::datatype::DataType::Text,
        value: Value::Text(root_id.to_string()),
    };
    let expected = |node: &str| {
        instance::Generation::new(
            instance::InstanceAddress::new(
                &node::OwnerPath::new([node]).expect("valid owner path"),
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
        "the first hop assigned different meta.sys fields"
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

  in -> counter -> out
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the two key dimensions are type-closed");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    for (tenant, user) in [("acme", 7), ("acme", 7), ("other", 7)] {
        execution
            .accept_input(&mut journal, tenant_submission(tenant, user))
            .unwrap();
        execution.run_ready(&mut journal).unwrap();
    }
    assert_eq!(
        execution
            .outputs()
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
    in -> greet -> out
  }

  in -> sessions -> out
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the public nested Graph source compiles");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    for _ in 0..2 {
        execution
            .accept_input(&mut journal, text_submission("alice"))
            .unwrap();
        execution.run_ready(&mut journal).unwrap();
    }

    assert_eq!(execution.outputs().len(), 2);
    assert!(execution.outputs().iter().all(|output| {
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
    let session = node::OwnerPath::new(["sessions"]).expect("valid Graph owner");
    let expected = instance::InstanceAddress::new(
        &node::OwnerPath::new(["sessions", "greet"]).expect("valid child owner"),
        &[instance::Segment::SelectedKey {
            scope: session,
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

  left -> pair.left
  right -> pair.right
  pair.out -> out
  pair.expired -> expired
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
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));

    execution
        .accept_input(&mut journal, port_int_submission("left", 10))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();
    assert!(
        execution.outputs().is_empty(),
        "an incomplete presence emits nothing"
    );
    execution
        .accept_input(&mut journal, port_int_submission("right", 20))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();

    let output = execution
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
                    Record::Ingress(inbound) if inbound.port.name() == "left" => {
                        Some(inbound.envelope.meta().sys().id())
                    }
                    _ => None,
                })
                .unwrap(),
            journal
                .records()
                .iter()
                .find_map(|record| match record {
                    Record::Ingress(inbound) if inbound.port.name() == "right" => {
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
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));

    execution
        .accept_input(&mut journal, port_int_submission("right", 20))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();
    assert!(
        execution.is_stable(),
        "retained partial presence is quiescent"
    );
    execution
        .accept_input(&mut journal, port_int_submission("left", 10))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();

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
                Record::Ingress(inbound) if inbound.port.name() == port => {
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

  left -> pair.left
  right -> pair.right
  pair.out -> out
  pair.expired -> expired
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
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    for (port, value) in [("left", 1), ("left", 2), ("right", 2)] {
        execution
            .accept_input(&mut journal, port_int_submission(port, value))
            .unwrap();
        execution.run_ready(&mut journal).unwrap();
    }

    let output = execution
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
        execution.is_stable(),
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
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    for value in [1, 2, 3] {
        execution
            .accept_input(&mut journal, port_int_submission("left", value))
            .unwrap();
        execution.run_ready(&mut journal).unwrap();
    }

    let expiry = execution
        .outputs()
        .iter()
        .find(|output| output.port.name() == "expired")
        .expect("overflow is an explicit typed outcome");
    assert!(matches!(
        expiry.envelope.data(),
        Data::Materialized(Value::TaggedUnion(value)) if value.case == "JoinExpired"
    ));
    execution
        .accept_input(&mut journal, port_int_submission("right", 9))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();
    let joined = execution
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

  left -> pair.left
  right -> pair.right
  pair.out -> out
  pair.expired -> expired
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
    let mut expired = Execution::new(&compilation.plan, GraphVersion(1));
    expired
        .accept_input(&mut expired_journal, port_int_submission("left", 4))
        .unwrap();
    expired.run_ready(&mut expired_journal).unwrap();
    expired
        .advance_to(
            &mut expired_journal,
            &mut CallDriver(&mut EchoWorld::default()),
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
    let mut joined = Execution::new(&compilation.plan, GraphVersion(1));
    joined
        .accept_input(&mut joined_journal, port_int_submission("left", 4))
        .unwrap();
    joined.run_ready(&mut joined_journal).unwrap();
    joined
        .accept_input(&mut joined_journal, port_int_submission("right", 5))
        .unwrap();
    joined.run_ready(&mut joined_journal).unwrap();
    joined
        .advance_to(
            &mut joined_journal,
            &mut CallDriver(&mut EchoWorld::default()),
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
        "the stale deadline cannot evict a contribution already committed"
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

  left -> pair.left
  right -> pair.right
  pair.out -> out
  pair.expired -> expired
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
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));

    for (port, value) in [("left", 1), ("right", 2)] {
        execution
            .accept_input(&mut journal, port_int_submission(port, value))
            .unwrap();
        execution.run_ready(&mut journal).unwrap();
    }
    assert!(
        execution.outputs().is_empty(),
        "different instance addresses never form one presence set"
    );
    execution
        .accept_input(&mut journal, port_int_submission("right", 1))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();

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
        execution
            .outputs()
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

  left -> mark_left -> pair.left
  right -> mark_right -> pair.right
  pair.out -> read_pick -> out
  pair.expired -> nothing
}

"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("named join metadata follows the same sealed grid as bare meta");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    execution
        .accept_input(&mut journal, port_int_submission("left", 1))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();
    execution
        .accept_input(&mut journal, port_int_submission("right", 2))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();

    assert_eq!(
        execution
            .outputs()
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

  left -> pair.left
  right -> mark_right -> pair.right
  pair.out -> out
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("an unused optional join field is legal");

    for arrivals in [[("left", 1), ("right", 2)], [("right", 2), ("left", 1)]] {
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
        for (port, value) in arrivals {
            execution
                .accept_input(&mut journal, port_int_submission(port, value))
                .unwrap();
            execution.run_ready(&mut journal).unwrap();
        }
        assert_eq!(
            execution.outputs()[0].envelope.meta().app().get("token"),
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
  left -> pair.left
  right -> pair.right
  pair.out -> out
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
    let mut execution = Execution::new(&carried.plan, GraphVersion(1));
    execution
        .accept_input(
            &mut journal,
            encoded_submission(
                "left",
                Encoding::ArrowIpc,
                b"left-opaque",
                "application/vnd.apache.arrow.stream-left",
            ),
        )
        .unwrap();
    execution.run_ready(&mut journal).unwrap();
    execution
        .accept_input(
            &mut journal,
            encoded_submission(
                "right",
                Encoding::ArrowIpc,
                b"right-opaque",
                "application/vnd.apache.arrow.stream-right",
            ),
        )
        .unwrap();
    execution
        .run_ready(&mut journal)
        .expect("carrying left never asks either opaque decoder");
    let output = &execution.outputs()[0].envelope;
    assert!(matches!(
        output.data(),
        Data::Encoded { encoding: Encoding::ArrowIpc, bytes, .. }
            if bytes.as_ref() == b"left-opaque"
    ));
    assert_eq!(
        output.meta().ext().get("content_type"),
        Some(&rastok::envelope::MetaValue::Scalar(
            rastok::envelope::Scalar::Text(String::from(
                "application/vnd.apache.arrow.stream-left"
            ))
        )),
        "the selected payload, not the causal trigger, decides content_type"
    );

    let inspected = compile_join("size(left.data)", "int");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&inspected.plan, GraphVersion(1));
    execution
        .accept_input(
            &mut journal,
            encoded_submission(
                "right",
                Encoding::ArrowIpc,
                b"never-read",
                "application/vnd.apache.arrow.stream",
            ),
        )
        .unwrap();
    execution.run_ready(&mut journal).unwrap();
    execution
        .accept_input(
            &mut journal,
            encoded_submission("left", Encoding::Raw, b"abc", "text/plain"),
        )
        .unwrap();
    execution
        .run_ready(&mut journal)
        .expect("only the structurally inspected left payload decodes");
    assert_eq!(
        execution.outputs()[0].envelope.data(),
        &Data::materialized(Value::Int(3))
    );
    assert!(
        execution.outputs()[0]
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
    idempotent = true
    behavior = deterministic
  }

  in -> shell
  shell.out -> out
  shell.err -> nothing
  shell.exit -> nothing
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the static spelling supplies a valid plan skeleton");
    let mut raw = compilation.plan.into_inner();
    raw.nodes[0].instance.key = Some(rastok::expr::Tree {
        root: rastok::expr::Expr::Literal(Value::Text(String::from("cat"))),
        ty: rastok::datatype::DataType::Text,
    });
    raw.nodes[0].vals.push(OwnerValue {
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
    let rastok::plan::NodeKind::Connector(spec) = &mut shell.body else {
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
    let mut execution = Execution::new(&plan, GraphVersion(1));
    let mut process = rastok::connector::drivers::Process::try_new(&plan)
        .expect("the Driver accepts a validated instance-bound sheet");
    execution
        .accept_input(&mut journal, text_submission("salut"))
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        execution
            .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut process))
            .unwrap();
        execution
            .poll_drivers(&mut journal, &mut CallDriver(&mut process))
            .unwrap();
        execution
            .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut process))
            .unwrap();
        if execution.is_stable() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the instance-bound Process did not settle"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        execution
            .outputs()
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

  in -> relay -> out
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the keyed plan checks");
    let mut raw = compilation.plan.into_inner();
    raw.nodes[0].vals.push(OwnerValue {
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
    let mut execution = Execution::new(&plan, GraphVersion(1));

    assert!(matches!(
        execution.accept_input(&mut journal, int_submission(7)),
        Err(ExecutionFault::Fault(_))
    ));
    assert!(
        journal.records().is_empty(),
        "candidate evaluation precedes ingress, lifecycle, and Driver effects"
    );
    assert!(
        execution.is_stable(),
        "the failed candidate installed no work"
    );
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

  in -> counter -> sink -> out
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the keyed graph checks");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));

    for value in [7, 9, 7] {
        execution
            .accept_input(&mut journal, int_submission(value))
            .unwrap();
        execution.run_ready(&mut journal).unwrap();
    }

    let output_values: Vec<i64> = execution
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
            Record::Ingress(inbound) => Some(inbound.routes[0].target.address().clone()),
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

  in -> counter
  counter.out -> out
  counter.stopped -> stopped
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the lifecycle graph checks");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));

    let first = execution
        .accept_input(&mut journal, int_submission(7))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();
    let second = execution
        .accept_input(&mut journal, int_submission(7))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();
    let other = execution
        .accept_input(&mut journal, int_submission(9))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();
    assert_eq!(
        execution
            .outputs()
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
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 1_000_000_000)
        .unwrap();
    assert_eq!(
        execution
            .outputs()
            .iter()
            .filter(|output| output.port.name() == "stopped")
            .map(|output| match output.envelope.data() {
                Data::Materialized(Value::Int(value)) => *value,
                other => panic!("the stop hook emits ints, got {other:?}"),
            })
            .collect::<Vec<_>>(),
        [102, 101],
        "same-time expiries retain deadline order after stale generations disappear"
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
    let root = instance::Generation::new(
        instance::InstanceAddress::singleton(&node::OwnerPath::root()),
        0,
    );
    let seven = instance::Generation::new(int_address("counter", 7), 0);
    let nine = instance::Generation::new(int_address("counter", 9), 0);
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
                node::Trigger::Lifecycle(node::Lifecycle::Close),
                Some(second),
                seven.clone(),
                None,
            ),
            (
                node::Trigger::Lifecycle(node::Lifecycle::Close),
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

    let next = execution
        .accept_input(&mut journal, int_submission(7))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();
    let last = execution
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
            Record::Ingress(inbound) if inbound.envelope.meta().sys().id() == next => {
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
    idempotent = true
    behavior = deterministic
  }

  in -> shell
  shell.out -> out
  shell.err -> nothing
  shell.exit -> exit
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the keyed Process graph checks");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let cause = execution
        .accept_input(&mut journal, text_submission("salut"))
        .unwrap();
    let mut world = EchoWorld::default();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    assert!(execution.has_open_obligations());

    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 1_000_000_000)
        .unwrap();
    assert!(
        journal.records().iter().all(|record| !matches!(
            record,
            Record::Transaction(transaction)
                if transaction.trigger
                    == node::Trigger::Lifecycle(node::Lifecycle::Stop)
        )),
        "instance expiry cannot replace an open call's terminal fact"
    );

    execution
        .poll_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();
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
        node::Trigger::Lifecycle(node::Lifecycle::Close)
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
    idempotent = true
    behavior = deterministic
  }
  operator collect {
    instance = { key = data }
    on in { out = . }
  }

  in -> seed -> collect -> out
  in -> shell
  shell.out -> collect
  shell.err -> nothing
  shell.exit -> nothing
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a bounded call may live inside a finite Graph");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();

    execution
        .accept_input(&mut journal, text_submission("salut"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    assert!(execution.has_open_obligations());
    assert_eq!(
        execution.outputs().len(),
        1,
        "the seed creates collect first"
    );

    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 1_000_000_000)
        .unwrap();
    execution
        .poll_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();

    assert_eq!(execution.outputs().len(), 2);
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
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = ClosingWorld {
        confirm_cancel: true,
        ..ClosingWorld::default()
    };

    execution
        .accept_input(&mut journal, text_submission("salut"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    execution
        .poll_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    assert_eq!(world.started.len(), 1);

    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 1_000_000_000)
        .unwrap();
    assert!(
        world.cancelled.is_empty(),
        "Closing first offers the drain budget"
    );
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 2_000_000_000)
        .unwrap();

    assert_eq!(world.cancelled, world.started);
    assert_eq!(source_exit_cases(&execution), ["Exited"]);
    assert!(journal.records().iter().any(|record| matches!(
        record,
        Record::Egress(EgressRecord::Call(outbound))
            if matches!(outbound.status, CallStatus::Closed(_))
    )));
    assert!(journal.records().iter().all(|record| !matches!(
        record,
        Record::Egress(EgressRecord::Call(outbound))
            if outbound.status == CallStatus::Abandoned
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
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = ClosingWorld::default();

    execution
        .accept_input(&mut journal, text_submission("salut"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    execution
        .poll_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 2_000_000_000)
        .unwrap();
    assert_eq!(world.cancelled.len(), 1);
    assert!(source_exit_cases(&execution).is_empty());
    assert!(
        execution.has_open_obligations(),
        "settlement is a real grace interval"
    );

    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 3_000_000_000)
        .unwrap();
    assert_eq!(source_exit_cases(&execution), ["OutcomeUnknown"]);
    assert_eq!(
        journal
            .records()
            .iter()
            .filter(|record| matches!(
                record,
                Record::Egress(EgressRecord::Call(outbound))
                    if outbound.status == CallStatus::Abandoned
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
    world.answers.push(CallResponse {
        call_id: world.started[0],
        completion: Completion::process(
            vec![],
            vec![],
            ProcessExit::Exited {
                exit_code: 0,
                signal: None,
            },
        ),
    });
    let report = execution
        .poll_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    assert_eq!(
        report.discarded.len(),
        1,
        "a late reply is observable but non-causal"
    );
    assert_eq!(source_exit_cases(&execution), ["OutcomeUnknown"]);
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
        let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
        let mut world = ClosingWorld {
            confirm_cancel: matches!(reply_at, ReplyAt::OnCancel),
            ..ClosingWorld::default()
        };
        execution
            .accept_input(&mut journal, text_submission("one"))
            .unwrap();
        execution
            .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
            .unwrap();
        execution
            .poll_drivers(&mut journal, &mut CallDriver(&mut world))
            .unwrap();
        let call = world.started[0];
        let reply = || CallResponse {
            call_id: call,
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
        execution
            .advance_to(&mut journal, &mut CallDriver(&mut world), 1_000_000_000)
            .unwrap();
        if matches!(reply_at, ReplyAt::BeforeDrain) {
            world.answers.push(reply());
            execution
                .poll_drivers(&mut journal, &mut CallDriver(&mut world))
                .unwrap();
            execution
                .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
                .unwrap();
        }
        execution
            .advance_to(&mut journal, &mut CallDriver(&mut world), 2_000_000_000)
            .unwrap();
        if matches!(reply_at, ReplyAt::DuringSettlement) {
            world.answers.push(reply());
            execution
                .poll_drivers(&mut journal, &mut CallDriver(&mut world))
                .unwrap();
            execution.run_ready(&mut journal).unwrap();
        }
        execution
            .advance_to(&mut journal, &mut CallDriver(&mut world), 3_000_000_000)
            .unwrap();
        let late = if matches!(reply_at, ReplyAt::AfterSettlement) {
            world.answers.push(reply());
            execution
                .poll_drivers(&mut journal, &mut CallDriver(&mut world))
                .unwrap()
        } else {
            Default::default()
        };
        execution.run_ready(&mut journal).unwrap();

        let expected = if matches!(
            reply_at,
            ReplyAt::BeforeDrain | ReplyAt::DuringSettlement | ReplyAt::OnCancel
        ) {
            "Exited"
        } else {
            "OutcomeUnknown"
        };
        assert_eq!(
            source_exit_cases(&execution),
            [expected],
            "schedule {reply_at:?}"
        );
        assert_eq!(
            journal
                .records()
                .iter()
                .filter(|record| matches!(
                    record,
                    Record::Egress(EgressRecord::Call(outbound))
                        if outbound.intent.call_id == call
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
        assert!(execution.is_stable(), "schedule {reply_at:?} reaches rest");
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
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = ClosingWorld {
        confirm_cancel: true,
        ..ClosingWorld::default()
    };

    execution
        .accept_input(&mut journal, text_submission("first"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    execution
        .poll_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 1_000_000_000)
        .unwrap();
    let second = execution
        .accept_input(&mut journal, text_submission("second"))
        .expect("Closing reserves a successor instead of refusing the arrival");
    assert!(
        journal.records().iter().all(|record| !matches!(
            record,
            Record::Transaction(transaction) if transaction.cause == Some(second)
        )),
        "the committed arrival has not executed in the closing generation"
    );

    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 2_000_000_000)
        .unwrap();
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
            (node::Trigger::Lifecycle(node::Lifecycle::Close), 0),
            (node::Trigger::Lifecycle(node::Lifecycle::Stop), 0),
            (node::Trigger::Lifecycle(node::Lifecycle::Start), 1),
            (node::Trigger::Lifecycle(node::Lifecycle::Close), 1),
        ],
        "the old stop precedes the successor start; its fixed deadline still dates from admission while waiting"
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
    idempotent = true; behavior = deterministic
  }
  connector b : Process {
    mode = job ; command = "cat" ; codec = lines
    idempotent = true; behavior = deterministic
  }

  in -> a
  in -> b
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
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = ClosingWorld::default();

    execution
        .accept_input(&mut journal, text_submission("first"))
        .unwrap();
    execution
        .accept_input(&mut journal, text_submission("queued"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    execution
        .poll_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    assert_eq!(
        world.started.len(),
        2,
        "one call per claimed connector starts"
    );
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 2_000_000_000)
        .unwrap();
    let mut sorted = world.cancelled.clone();
    sorted.sort();
    assert_eq!(
        world.cancelled, sorted,
        "cancels follow canonical CallId order"
    );
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 3_000_000_000)
        .unwrap();

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
    assert_eq!(
        source_exit_cases(&execution),
        ["OutcomeUnknown", "OutcomeUnknown"]
    );
    assert!(
        execution.outputs().is_empty(),
        "explicit discards are not App outputs"
    );
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

  in -> counter -> out
}}
"#
        );
        let compilation = compile(
            &[&source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("R4 admits the finite profile");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
        let mut world = EchoWorld::default();

        execution
            .accept_input(&mut journal, int_submission(1))
            .unwrap();
        execution.run_ready(&mut journal).unwrap();
        if needs_advance {
            execution
                .advance_to(&mut journal, &mut CallDriver(&mut world), 1_000_000_000)
                .unwrap();
        }
        execution
            .accept_input(&mut journal, int_submission(2))
            .unwrap();
        execution.run_ready(&mut journal).unwrap();

        let values: Vec<i64> = execution
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
fn persistent_owners_start_once_and_create_no_lifetime_deadline() {
    let source = r#"
graph persistent : App {
  input in: int
  output out: int
  operator relay { on in { out = . } }
  in -> relay -> out
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("persistent is the executable omission default");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();

    execution
        .accept_input(&mut journal, int_submission(1))
        .unwrap();
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), i64::MAX)
        .unwrap();

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
                    transaction.trigger.clone(),
                    transaction.deadline_ids.clone(),
                ))
            }
            _ => None,
        })
        .collect();
    assert_eq!(lifecycle.len(), 2, "root and relay start lazily once");
    assert!(lifecycle.iter().all(|(trigger, deadline_ids)| {
        *trigger == node::Trigger::Lifecycle(node::Lifecycle::Start) && deadline_ids.is_empty()
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
  in -> relay -> out
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("key and fixed lifetime are orthogonal");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();

    execution
        .accept_input(&mut journal, int_submission(1))
        .unwrap();
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 500_000_000)
        .unwrap();
    execution
        .accept_input(&mut journal, int_submission(2))
        .unwrap();
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 1_000_000_000)
        .unwrap();

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

    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 1_500_000_000)
        .unwrap();
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
  in -> relay -> out
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("single-pass is executable");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));

    execution
        .accept_input(&mut journal, int_submission(1))
        .unwrap();
    while !journal.records().iter().any(|record| {
        matches!(
            record,
            Record::Transaction(transaction) if transaction.close_scope.is_some()
        )
    }) {
        assert!(
            execution.step(&mut journal).unwrap(),
            "the first root reaches Closing"
        );
    }
    let second = execution
        .accept_input(&mut journal, int_submission(2))
        .unwrap();
    assert!(
        matches!(
            execution.accept_input(&mut journal, int_submission(3)),
            Err(ExecutionFault::InstanceClosing { owner }) if owner.to_string() == "relay"
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
    execution.run_ready(&mut journal).unwrap();
    assert_eq!(execution.outputs().len(), 2);
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
  in -> relay -> out
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("bare stop names the current owner without a target language");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    execution
        .accept_input(&mut journal, int_submission(1))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();
    execution
        .accept_input(&mut journal, int_submission(2))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();

    assert_eq!(
        execution
            .outputs()
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

  in -> counter -> out
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("an unkeyed Graph can own a temporal window");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();

    execution
        .accept_input(&mut journal, int_submission(1))
        .unwrap();
    execution
        .accept_input(&mut journal, int_submission(2))
        .unwrap();
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 5_000_000_000)
        .unwrap();
    execution
        .accept_input(&mut journal, int_submission(3))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();

    let values: Vec<i64> = execution
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

  in -> window -> out
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("nested finite profiles are executable");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();

    execution
        .accept_input(&mut journal, int_submission(1))
        .unwrap();
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 1_000_000_000)
        .unwrap();
    assert!(matches!(
        execution.accept_input(&mut journal, int_submission(2)),
        Err(ExecutionFault::ClosedTimeBox { owner, start_ns: 0 }) if owner.to_string() == "window"
    ));
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 5_000_000_000)
        .unwrap();
    execution
        .accept_input(&mut journal, int_submission(2))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();
    assert_eq!(execution.outputs().len(), 2);
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

  in -> child -> out
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("nested fixed generations are executable");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();

    execution
        .accept_input(&mut journal, int_submission(1))
        .unwrap();
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 1_000_000_000)
        .unwrap();
    execution
        .accept_input(&mut journal, int_submission(2))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 2_000_000_000)
        .unwrap();

    let records_after_successor = journal.records().len();
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 5_000_000_000)
        .unwrap();

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
  in -> alpha -> nothing
  in -> beta -> nothing
}
"#;
    let run = || {
        let compilation = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("equal fixed deadlines are executable");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
        execution
            .accept_input(&mut journal, int_submission(1))
            .unwrap();
        execution
            .advance_to(
                &mut journal,
                &mut CallDriver(&mut EchoWorld::default()),
                1_000_000_000,
            )
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
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld {
        fail_next: true,
        ..EchoWorld::default()
    };
    execution
        .accept_input(&mut journal, submission("salut"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    execution
        .poll_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    // stderr is ordinary data on `err`; the non-zero child status is an
    // independent, typed `Exited` terminal fact.
    assert!(execution.outputs().is_empty());
    let last = journal.records().last().unwrap();
    let Record::Transaction(transaction) = last else {
        panic!("expected the completion transaction");
    };
    assert_eq!(transaction.emissions[0].port.name(), "err");
    assert_eq!(transaction.emissions[1].port.name(), "exit");
    let stderr: Vec<_> = execution
        .envelope_views()
        .iter()
        .filter(|observed| observed.node.is_some() && observed.port.name() == "err")
        .collect();
    assert_eq!(stderr.len(), 1);
    assert_eq!(
        stderr[0].envelope.data(),
        &Data::materialized(Value::Text(String::from("exit 1")))
    );
    assert_eq!(source_exit_cases(&execution), ["Exited"]);
}

#[test]
fn malformed_terminal_facts_are_refused_loudly() {
    let compilation = compile(
        &[M0_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut opener = EchoWorld::default();
    execution
        .accept_input(&mut journal, submission("salut"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut opener))
        .unwrap();
    let open = journal.open_calls();
    let mut world = ScriptedWorld {
        answers: vec![
            CallResponse {
                call_id: open[0].call_id,
                completion: Completion::try_new(vec![EnvelopeDraft {
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
            CallResponse {
                call_id: open[0].call_id,
                completion: Completion::try_new(vec![EnvelopeDraft {
                    port: node::Port::conventional_exit(),
                    data: Data::encoded(rastok::envelope::Encoding::Raw, b"not a union"),
                    ext: rastok::envelope::Namespace::new(),
                }])
                .unwrap(),
            },
        ],
    };
    let report = execution
        .poll_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    // Refused BEFORE the causal fact: the reply is rejected with
    // its reason, and the call stays open — the timeout machinery
    // can still settle it honestly.
    assert_eq!(report.settled, 0);
    assert_eq!(report.rejected.len(), 2);
    assert!(report.rejected[0].1.contains("undeclared tagged case"));
    assert!(
        !report.rejected[0].1.contains("MadeUp"),
        "a malformed classified discriminant is never formatted"
    );
    assert!(report.rejected[1].1.contains("materialized"));
    assert_eq!(journal.open_calls().len(), 1, "no garbage closure");
    execution.run_ready(&mut journal).unwrap();
    assert!(!execution.is_stable(), "the obligation is still open");
}

/// A world answering from a script, for integrity tests.
struct ScriptedWorld {
    answers: Vec<CallResponse>,
}

impl OutboundCall for ScriptedWorld {
    fn prepare_request(
        &mut self,
        _: &node::Path,
        data: &Data,
        _: &std::collections::BTreeMap<String, rastok::value::Value>,
    ) -> RequestPreparation {
        rastok::connector::stdlib::prepare_process_request(
            rastok::connector::stdlib::RequestCodec::Lines,
            data,
        )
    }

    fn scheduler_completion(
        &mut self,
        _: &node::Path,
        completion: SchedulerCompletion,
    ) -> Completion {
        rastok::connector::stdlib::process_scheduler_completion(completion)
    }

    fn start(&mut self, _: &CallOpen, _: &Envelope) -> CallStart {
        CallStart::Started
    }
    fn poll_responses(&mut self) -> Vec<CallResponse> {
        std::mem::take(&mut self.answers)
    }
}

#[derive(Default)]
struct SessionWorld {
    events: Vec<SessionObservation>,
    opened: Vec<SessionId>,
    close_requests: Vec<SessionId>,
    abort_requests: Vec<SessionId>,
    released: Vec<SessionId>,
    pending_opens: bool,
    hold_writes: bool,
    hold_closes: bool,
    writes: Vec<(SessionId, SessionWriteId)>,
}

impl SessionWorld {
    fn close(&mut self, session: SessionId) {
        self.events.push(SessionObservation::Exited {
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
            .push(SessionObservation::WriteSettled { session, write });
    }
}

impl OutboundDrivers for SessionWorld {
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
        _: &rastok::connector::exchange::CallId,
    ) -> Result<(), DriverRouteError> {
        Err(DriverRouteError::UnboundOutbound(connector.clone()))
    }

    fn start(&mut self, intent: &CallOpen, _: &Envelope) -> Result<CallStart, DriverRouteError> {
        Err(DriverRouteError::UnboundOutbound(
            intent.connector_path.clone(),
        ))
    }

    fn poll_responses(&mut self) -> Result<Vec<CallResponse>, DriverRouteError> {
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
            Data::Encoded { bytes, .. } => bytes.to_vec(),
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
        intent: &SessionOpen,
        _: &Envelope,
    ) -> Result<SessionStart, DriverRouteError> {
        self.opened.push(intent.session_id);
        if self.pending_opens {
            Ok(SessionStart::Pending)
        } else {
            self.events
                .push(SessionObservation::Opened(intent.session_id));
            Ok(SessionStart::Started)
        }
    }

    fn prepare_session_write(
        &mut self,
        _: &node::Path,
        _: SessionId,
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
        intent: &SessionWrite,
        _: &Envelope,
    ) -> Result<SessionStart, DriverRouteError> {
        self.writes.push((intent.session_id, intent.write_ref));
        if !self.hold_writes {
            self.events.push(SessionObservation::WriteSettled {
                session: intent.session_id,
                write: intent.write_ref,
            });
        }
        Ok(SessionStart::Started)
    }

    fn close_session(
        &mut self,
        _: &node::Path,
        session: SessionId,
    ) -> Result<SessionStart, DriverRouteError> {
        self.close_requests.push(session);
        if !self.hold_closes {
            self.close(session);
        }
        Ok(SessionStart::Started)
    }

    fn poll_observations(&mut self) -> Result<Vec<SessionObservation>, DriverRouteError> {
        Ok(std::mem::take(&mut self.events))
    }

    fn forget_session(&mut self, session: SessionId) -> Result<(), DriverRouteError> {
        self.released.push(session);
        Ok(())
    }

    fn abort_session(
        &mut self,
        _: &node::Path,
        session: SessionId,
    ) -> Result<(), DriverRouteError> {
        if !self.abort_requests.contains(&session) {
            self.abort_requests.push(session);
        }
        Ok(())
    }
}

// Qualify the generic resource lifecycle through an executable Session
// Contract, independently of Process service mode.
const KEYED_SESSION_LIFECYCLE: &str = r#"
graph app : App {
  input in: text
  input close: text
  output exit: variant
  graph conversation {
    input in: text
    input close: text
    output exit: variant
    instance = { key = data }
    connector socket : WebSocketClient {
      url = "ws://example.test/socket"
      handshake_timeout = duration("5s")
      codec = text;  behavior = deterministic
      closing = { drain_for = duration("1s"); settle_for = duration("1s") }
    }
    operator close_request {
      input in: text
      output out: text
      on in { out = data }
    }
    in -> socket
    close -> close_request
    close_request.out -> socket.instance.stop
    socket.out -> nothing
    socket.exit -> exit
  }
  in -> conversation
  close -> conversation.close
  conversation.exit -> exit
}
"#;

#[test]
fn a_new_input_reopens_only_its_ended_session_without_resending_old_writes() {
    use rastok::journal::SessionStatus;
    use rastok::runtime::execution::inspection::InstancePhaseInspection;

    let compiled = compile(
        &[KEYED_SESSION_LIFECYCLE],
        &ContractCatalog::try_m0().unwrap(),
    )
    .unwrap();
    let (plan, _) =
        rastok::plan::decode(&rastok::plan::encode(&compiled.plan, &compiled.hash)).unwrap();
    let mut expected_records = None;
    for recover in [false, true] {
        let mut execution = Execution::new(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut world = SessionWorld {
            hold_writes: true,
            ..SessionWorld::default()
        };
        for tenant in ["alice", "bob", "alice", "bob"] {
            execution
                .accept_input(&mut journal, text_submission(tenant))
                .unwrap();
            execution
                .run_ready_with_drivers(&mut journal, &mut world)
                .unwrap();
            execution.poll_drivers(&mut journal, &mut world).unwrap();
        }
        let [alice, bob] = world.opened.as_slice() else {
            panic!("one resource per conversation, not per message");
        };
        let (alice, bob) = (*alice, *bob);
        let alice_instance = execution
            .inspection()
            .sessions
            .iter()
            .find(|session| session.reference == alice)
            .unwrap()
            .instance
            .clone();
        assert_eq!(world.writes.len(), 2);
        assert_eq!(world.writes[0].0, alice);
        assert_eq!(world.writes[1].0, bob);
        let original_writes = world.writes.clone();

        world.close(alice);
        execution.poll_drivers(&mut journal, &mut world).unwrap();
        execution
            .run_ready_with_drivers(&mut journal, &mut world)
            .unwrap();
        assert_eq!(journal.live_sessions(), vec![bob]);
        assert_eq!(journal.open_session_writes().len(), 1);
        assert_eq!(journal.open_session_writes()[0].session_id, bob);
        assert!(journal.records().iter().any(|record| matches!(record,
            Record::Egress(EgressRecord::Session(record))
                if record.session_id == alice
                    && record.status == SessionStatus::WriteAbandoned(original_writes[0].1))));
        assert!(execution.inspection().instances.iter().any(|instance| {
            instance.reference == alice_instance && instance.phase == InstancePhaseInspection::Open
        }));
        if recover {
            let retained = journal.records().to_vec();
            execution.recover_from_journal(&journal).unwrap();
            assert_eq!(journal.records(), retained);
        }
        execution
            .advance_to(&mut journal, &mut world, 10_000_000_000)
            .unwrap();
        assert_eq!(world.opened, vec![alice, bob], "no timed respawn");
        assert_eq!(world.writes, original_writes, "no recovery resend");

        let fresh_input = execution
            .accept_input(&mut journal, text_submission("alice"))
            .unwrap();
        execution
            .run_ready_with_drivers(&mut journal, &mut world)
            .unwrap();
        execution.poll_drivers(&mut journal, &mut world).unwrap();
        assert_eq!(world.opened.len(), 3);
        let fresh_alice = world.opened[2];
        assert_ne!(fresh_alice, alice);
        assert_ne!(fresh_alice, bob);
        assert_eq!(
            world.writes, original_writes,
            "old writes stay on the old resource"
        );
        assert!(execution.inspection().sessions.iter().any(|session| {
            session.reference == fresh_alice && session.instance == alice_instance
        }));
        assert!(journal.records().iter().any(|record| matches!(record,
            Record::Egress(EgressRecord::Session(record))
                if record.session_id == fresh_alice
                    && matches!(&record.status, SessionStatus::Opening(intent)
                        if intent.emitting_id == fresh_input))));

        world.close(alice);
        world.settle_write(0);
        let retained = journal.records().to_vec();
        let report = execution.poll_drivers(&mut journal, &mut world).unwrap();
        assert_eq!(report.uncorrelated_sessions.len(), 2);
        execution
            .run_ready_with_drivers(&mut journal, &mut world)
            .unwrap();
        assert_eq!(
            journal.records(),
            retained,
            "late facts cannot target a replacement"
        );
        assert_eq!(
            execution.outputs().len(),
            1,
            "one terminal for the ended resource"
        );

        for (tenant, resource) in [("alice", fresh_alice), ("bob", bob)] {
            execution
                .accept_input(&mut journal, text_submission(tenant))
                .unwrap();
            execution
                .run_ready_with_drivers(&mut journal, &mut world)
                .unwrap();
            assert_eq!(world.writes.last().unwrap().0, resource);
        }
        assert_eq!(world.opened.len(), 3, "Bob was never reopened");
        if let Some(expected) = &expected_records {
            assert_eq!(
                journal.records(),
                expected,
                "recovery changes no causal fact"
            );
        } else {
            expected_records = Some(journal.records().to_vec());
        }
    }
}

#[test]
fn repeated_child_stop_closes_only_the_exact_session_instance() {
    let compiled = compile(
        &[KEYED_SESSION_LIFECYCLE],
        &ContractCatalog::try_m0().unwrap(),
    )
    .unwrap();
    let (plan, _) =
        rastok::plan::decode(&rastok::plan::encode(&compiled.plan, &compiled.hash)).unwrap();
    let mut execution = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = SessionWorld::default();
    for tenant in ["alice", "bob"] {
        execution
            .accept_input(&mut journal, text_submission(tenant))
            .unwrap();
        execution
            .run_ready_with_drivers(&mut journal, &mut world)
            .unwrap();
        execution.poll_drivers(&mut journal, &mut world).unwrap();
    }
    let [alice, bob] = world.opened.as_slice() else {
        panic!("one Session per keyed conversation");
    };
    let (alice, bob) = (*alice, *bob);
    let alice_instance = execution
        .inspection()
        .sessions
        .iter()
        .find(|session| session.reference == alice)
        .unwrap()
        .instance
        .clone();

    for through in [500_000_000, 1_000_000_000] {
        let mut close = text_submission("alice");
        close.draft.port = node::Port::new("close").unwrap();
        execution.accept_input(&mut journal, close).unwrap();
        execution
            .run_ready_with_drivers(&mut journal, &mut world)
            .unwrap();
        execution
            .advance_to(&mut journal, &mut world, through)
            .unwrap();
    }
    assert_eq!(
        world.close_requests,
        vec![alice],
        "no renewed drain or duplicate close"
    );
    execution.poll_drivers(&mut journal, &mut world).unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut world)
        .unwrap();
    execution
        .advance_to(&mut journal, &mut world, 3_000_000_000)
        .unwrap();
    assert_eq!(journal.live_sessions(), vec![bob]);
    assert_eq!(world.opened, vec![alice, bob], "a stop does not respawn");
    assert!(journal.records().iter().any(|record| matches!(record,
        Record::Transaction(record)
            if record.instance == alice_instance
                && record.trigger == node::Trigger::Lifecycle(node::Lifecycle::Stop))));

    let mut repeated = text_submission("alice");
    repeated.draft.port = node::Port::new("close").unwrap();
    execution.accept_input(&mut journal, repeated).unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut world)
        .unwrap();
    execution
        .accept_input(&mut journal, text_submission("bob"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut world)
        .unwrap();
    execution.poll_drivers(&mut journal, &mut world).unwrap();
    assert_eq!(world.writes.last().unwrap().0, bob);
    assert_eq!(world.close_requests, vec![alice]);
    assert_eq!(world.opened, vec![alice, bob]);
    assert_eq!(execution.outputs().len(), 1);
    assert!(
        !execution
            .inspection()
            .instances
            .iter()
            .any(|instance| instance.reference == alice_instance)
    );
}

#[test]
fn a_protocol_quit_is_data_until_an_observed_exit_or_bounded_instance_close() {
    use rastok::journal::SessionStatus;
    use rastok::runtime::execution::inspection::InstancePhaseInspection;

    let source = r#"
type Message = struct { tenant: text, text: text }
graph app : App {
  input in: Message
  input close: Message
  output exit: variant
  instance = { key = data.tenant }
  operator encode {
    input in: Message
    output out: text
    on in { out = data.text }
  }
  operator close_request {
    input in: Message
    output out: text
    on in { out = data.text }
  }
  connector socket : WebSocketClient {
    url = "ws://example.test/socket"
    handshake_timeout = duration("5s")
    codec = text;  behavior = deterministic
    closing = { drain_for = duration("1s"); settle_for = duration("1s") }
  }
  in -> encode -> socket
  close -> close_request
  close_request.out -> socket.instance.stop
  socket.out -> nothing
  socket.exit -> exit
}
"#;
    let compilation = compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let (plan, _) =
        rastok::plan::decode(&rastok::plan::encode(&compilation.plan, &compilation.hash)).unwrap();
    let message = |tenant: &str, text: &str, port: &str| {
        let mut submission = text_submission(text);
        submission.draft.port = node::Port::new(port).unwrap();
        submission.draft.data = Data::materialized(Value::Struct(vec![
            (String::from("tenant"), Value::Text(tenant.into())),
            (String::from("text"), Value::Text(text.into())),
        ]));
        submission
    };

    for responds_to_quit in [true, false] {
        let mut expected = None;
        for recover in [false, true] {
            let mut execution = Execution::new(&plan, GraphVersion(1));
            let mut journal = MemJournal::new(GraphVersion(1));
            let mut world = SessionWorld {
                hold_writes: true,
                hold_closes: true,
                ..SessionWorld::default()
            };
            for tenant in ["alice", "bob"] {
                execution
                    .accept_input(&mut journal, message(tenant, "hello", "in"))
                    .unwrap();
                execution
                    .run_ready_with_drivers(&mut journal, &mut world)
                    .unwrap();
                execution.poll_drivers(&mut journal, &mut world).unwrap();
            }
            let [alice, bob] = world.opened.as_slice() else {
                panic!("one resource per keyed instance");
            };
            let (alice, bob) = (*alice, *bob);
            let alice_instance = execution
                .inspection()
                .sessions
                .iter()
                .find(|session| session.reference == alice)
                .unwrap()
                .instance
                .clone();

            execution
                .accept_input(&mut journal, message("alice", "quit", "in"))
                .unwrap();
            execution
                .run_ready_with_drivers(&mut journal, &mut world)
                .unwrap();
            assert_eq!(world.writes.len(), 1);
            assert_eq!(world.writes[0].0, alice);
            let writes = journal.open_session_writes();
            assert_eq!(writes.len(), 1);
            assert_eq!(writes[0].prepared.bytes(), b"quit");
            assert_eq!(writes[0].session_id, alice);
            let quit_write = writes[0].write_ref;
            world.settle_write(0);
            execution.poll_drivers(&mut journal, &mut world).unwrap();
            assert!(journal.open_session_writes().is_empty());
            assert!(
                execution.outputs().is_empty(),
                "writing quit is not an exit"
            );
            assert_eq!(journal.live_sessions().len(), 2);
            assert!(world.close_requests.is_empty(), "quit is not instance.stop");

            if recover {
                execution.recover_from_journal(&journal).unwrap();
            }
            if responds_to_quit {
                world.close(alice);
                execution.poll_drivers(&mut journal, &mut world).unwrap();
                execution
                    .run_ready_with_drivers(&mut journal, &mut world)
                    .unwrap();
                assert_eq!(source_exit_cases(&execution), ["Closed"]);
                assert!(world.close_requests.is_empty());
                assert!(
                    execution.inspection().instances.iter().any(|instance| {
                        instance.reference == alice_instance
                            && instance.phase == InstancePhaseInspection::Open
                    }),
                    "resource exit alone does not stop its Node instance"
                );
            } else {
                execution
                    .accept_input(&mut journal, message("alice", "stop", "close"))
                    .unwrap();
                execution
                    .run_ready_with_drivers(&mut journal, &mut world)
                    .unwrap();
                assert!(world.close_requests.is_empty(), "admitted work can drain");
                if recover {
                    execution.recover_from_journal(&journal).unwrap();
                }
                execution
                    .advance_to(&mut journal, &mut world, 1_000_000_000)
                    .unwrap();
                assert_eq!(world.close_requests, [alice]);
                assert!(
                    execution.outputs().is_empty(),
                    "a close request proves no exit"
                );
                if recover {
                    execution.recover_from_journal(&journal).unwrap();
                }
                execution
                    .advance_to(&mut journal, &mut world, 2_000_000_000)
                    .unwrap();
                execution
                    .run_ready_with_drivers(&mut journal, &mut world)
                    .unwrap();
                assert_eq!(source_exit_cases(&execution), ["OutcomeUnknown"]);
                assert!(
                    !execution
                        .inspection()
                        .instances
                        .iter()
                        .any(|instance| { instance.reference == alice_instance })
                );
            }
            assert_eq!(journal.live_sessions(), vec![bob]);
            assert_eq!(world.released, [alice]);
            let before_late_exit = journal.records().to_vec();
            world.close(alice);
            let report = execution.poll_drivers(&mut journal, &mut world).unwrap();
            execution
                .run_ready_with_drivers(&mut journal, &mut world)
                .unwrap();
            assert!(matches!(report.uncorrelated_sessions.as_slice(),
                [SessionObservation::Exited { session, .. }] if *session == alice));
            assert_eq!(journal.records(), before_late_exit);
            assert_eq!(execution.outputs().len(), 1, "late exit cannot close twice");
            assert_eq!(world.released, [alice], "late exit cannot clean up Bob");

            execution
                .accept_input(&mut journal, message("bob", "still here", "in"))
                .unwrap();
            execution
                .run_ready_with_drivers(&mut journal, &mut world)
                .unwrap();
            assert_eq!(world.writes.last().unwrap().0, bob);
            assert_eq!(world.opened, [alice, bob]);
            assert_eq!(world.writes.len(), 2, "recovery never resends quit");
            assert_eq!(
                journal
                    .records()
                    .iter()
                    .filter(|record| matches!(record,
                        Record::Egress(EgressRecord::Session(record))
                            if record.status == SessionStatus::WriteSettled(quit_write)
                    ))
                    .count(),
                1
            );
            if let Some(expected) = &expected {
                assert_eq!(journal.records(), expected);
            } else {
                expected = Some(journal.records().to_vec());
            }
        }
    }
}

#[test]
fn session_release_waits_for_the_committed_terminal_even_after_a_storage_failure() {
    let compiled = compile(
        &[KEYED_SESSION_LIFECYCLE],
        &ContractCatalog::try_m0().unwrap(),
    )
    .unwrap();
    for observed_exit in [false, true] {
        for recover in [false, true] {
            let mut execution = Execution::new(&compiled.plan, GraphVersion(1));
            let mut journal = FlakyJournal::healthy(GraphVersion(1));
            let mut world = SessionWorld {
                hold_closes: true,
                ..SessionWorld::default()
            };
            for tenant in ["alice", "bob"] {
                execution
                    .accept_input(&mut journal, text_submission(tenant))
                    .unwrap();
                execution
                    .run_ready_with_drivers(&mut journal, &mut world)
                    .unwrap();
                execution.poll_drivers(&mut journal, &mut world).unwrap();
            }
            let (alice, bob) = (world.opened[0], world.opened[1]);
            let mut close = text_submission("alice");
            close.draft.port = node::Port::new("close").unwrap();
            execution.accept_input(&mut journal, close).unwrap();
            execution
                .run_ready_with_drivers(&mut journal, &mut world)
                .unwrap();
            execution
                .advance_to(&mut journal, &mut world, 999_999_999)
                .unwrap();
            assert!(world.close_requests.is_empty());
            assert!(world.abort_requests.is_empty());
            assert!(world.released.is_empty());
            execution
                .advance_to(&mut journal, &mut world, 1_000_000_000)
                .unwrap();
            assert_eq!(world.close_requests, [alice]);
            assert!(
                world.abort_requests.is_empty(),
                "no forced stop during settlement"
            );
            assert!(
                world.released.is_empty(),
                "a close request is not a terminal"
            );

            if observed_exit {
                world.close(alice);
            }
            journal.refuse_commits = true;
            let retained = journal.inner.records().to_vec();
            assert!(matches!(
                execution.advance_to(&mut journal, &mut world, 2_000_000_000),
                Err(ExecutionFault::Journal(_))
            ));
            assert_eq!(journal.inner.records(), retained);
            assert!(
                world.released.is_empty(),
                "no cleanup before the terminal commit"
            );
            assert_eq!(journal.live_sessions().len(), 2);
            assert!(execution.outputs().is_empty());
            assert_eq!(
                world.abort_requests,
                if observed_exit { vec![] } else { vec![alice] },
                "the unsealed deadline authorizes cleanup even if terminal storage fails"
            );

            journal.refuse_commits = false;
            if recover {
                execution.recover_from_journal(&journal).unwrap();
                assert!(
                    world.released.is_empty(),
                    "reconstruction performs no cleanup"
                );
                // An observed but uncommitted terminal must be supplied again
                // by its original world, not reconstructed from an invented exit.
                if observed_exit {
                    world.close(alice);
                }
            }
            execution
                .advance_to(&mut journal, &mut world, 2_000_000_000)
                .unwrap();
            assert_eq!(world.released, [alice]);
            assert_eq!(
                world.abort_requests,
                if observed_exit { vec![] } else { vec![alice] }
            );
            assert_eq!(journal.live_sessions(), [bob]);
            assert_eq!(
                source_exit_cases(&execution),
                [if observed_exit {
                    "Closed"
                } else {
                    "OutcomeUnknown"
                }]
            );
            let retained = journal.inner.records().to_vec();
            execution.recover_from_journal(&journal).unwrap();
            execution
                .advance_to(&mut journal, &mut world, 3_000_000_000)
                .unwrap();
            assert_eq!(
                world.released,
                [alice],
                "a retained terminal is not repeated"
            );
            assert_eq!(journal.inner.records(), retained);
            execution
                .accept_input(&mut journal, text_submission("bob"))
                .unwrap();
            execution
                .run_ready_with_drivers(&mut journal, &mut world)
                .unwrap();
            assert_eq!(world.writes.last().unwrap().0, bob);
        }
    }
}

#[test]
fn session_abort_does_not_wait_for_a_full_terminal_route() {
    let source = r#"
graph app : App {
  input in: text
  input close: text
  input seed: variant
  connector socket : WebSocketClient {
    url = "ws://example.test/socket"
    handshake_timeout = duration("5s")
    codec = text;  behavior = deterministic
    closing = { drain_for = duration("1s"); settle_for = duration("1s") }
  }
  operator close_request {
    input in: text
    output out: text
    on in { out = data }
  }
  operator hold {
    input left: variant
    input right: variant
    on (left, right) { nothing }
  }
  in -> socket
  close -> close_request
  close_request.out -> socket.instance.stop
  socket.out -> nothing
  seed -> hold.left
  socket.exit -> hold.left
}
"#;
    let compilation = compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = SessionWorld {
        hold_closes: true,
        ..SessionWorld::default()
    };
    // The current Worker has 1,024 slots. No right-hand partner arrives, so
    // these real deliveries saturate the terminal's destination mailbox.
    for _ in 0..1024 {
        let mut seed = text_submission("waiting");
        seed.draft.port = node::Port::new("seed").unwrap();
        execution.accept_input(&mut journal, seed).unwrap();
        execution
            .run_ready_with_drivers(&mut journal, &mut world)
            .unwrap();
    }
    execution
        .accept_input(&mut journal, text_submission("open"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut world)
        .unwrap();
    execution.poll_drivers(&mut journal, &mut world).unwrap();
    let session = world.opened[0];
    let mut close = text_submission("stop");
    close.draft.port = node::Port::new("close").unwrap();
    execution.accept_input(&mut journal, close).unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut world)
        .unwrap();
    execution
        .advance_to(&mut journal, &mut world, 1_000_000_000)
        .unwrap();
    assert_eq!(world.close_requests, [session]);
    assert!(world.abort_requests.is_empty());
    let retained = journal.records().to_vec();
    assert!(matches!(
        execution.advance_to(&mut journal, &mut world, 2_000_000_000),
        Err(ExecutionFault::MailboxFull { node }) if node == node::Path::new(["hold"]).unwrap()
    ));
    assert_eq!(
        world.abort_requests,
        [session],
        "the stop handoff is not backpressured"
    );
    assert!(
        world.released.is_empty(),
        "terminal publication still needs capacity"
    );
    assert_eq!(journal.records(), retained);
    assert_eq!(journal.live_sessions(), [session]);
}

#[test]
fn session_recovery_restores_open_write_settlement_and_terminal_without_repeating_io() {
    let source = r#"
graph app : App {
  input in: text
  output out: text
  output exit: variant
  connector socket : WebSocketClient {
    url = "ws://example.test/socket"
    handshake_timeout = duration("5s")
    codec = text;  behavior = deterministic
  }
  in -> socket
  socket.out -> out
  socket.exit -> exit
}
"#;
    let compilation = compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let mut expected = None;
    for restart in [false, true] {
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
        let mut world = SessionWorld {
            hold_writes: true,
            ..SessionWorld::default()
        };
        let recover = |execution: &mut Execution<'_>, journal: &MemJournal| {
            if restart {
                let records = journal.records().to_vec();
                execution.recover_from_journal(journal).unwrap();
                assert_eq!(records, journal.records());
            }
        };
        execution
            .accept_input(&mut journal, text_submission("open"))
            .unwrap();
        while execution
            .step_with_drivers(&mut journal, &mut world)
            .unwrap()
        {
            recover(&mut execution, &journal);
        }
        assert_eq!(world.opened.len(), 1);
        execution.poll_drivers(&mut journal, &mut world).unwrap();
        recover(&mut execution, &journal);
        for text in ["first", "second"] {
            execution
                .accept_input(&mut journal, text_submission(text))
                .unwrap();
            recover(&mut execution, &journal);
            while execution
                .step_with_drivers(&mut journal, &mut world)
                .unwrap()
            {
                recover(&mut execution, &journal);
            }
        }
        assert_eq!(world.writes.len(), 2);
        assert_eq!(journal.open_session_writes().len(), 2);
        for write in [1, 0] {
            world.settle_write(write);
            execution.poll_drivers(&mut journal, &mut world).unwrap();
            recover(&mut execution, &journal);
        }
        world.close(world.opened[0]);
        execution.poll_drivers(&mut journal, &mut world).unwrap();
        while execution
            .step_with_drivers(&mut journal, &mut world)
            .unwrap()
        {
            recover(&mut execution, &journal);
        }
        assert_eq!(world.opened.len(), 1, "no repeated handshake");
        assert_eq!(world.writes.len(), 2, "no repeated message");
        assert_eq!(execution.outputs().len(), 1);
        assert!(journal.live_sessions().is_empty());
        if let Some(expected) = &expected {
            assert_eq!(
                journal.records(),
                expected,
                "recovery is causally invisible"
            );
        } else {
            expected = Some(journal.records().to_vec());
        }
    }
}

#[test]
fn session_recovery_keeps_the_original_handshake_deadline_and_rejects_crossed_facts() {
    let source = r#"
graph app : App {
  input in: text
  output exit: variant
  connector socket : WebSocketClient {
    url = "ws://example.test/socket"; handshake_timeout = duration("5s")
    codec = text;  behavior = deterministic
  }
  in -> socket
  socket.out -> nothing
  socket.exit -> exit
}
"#;
    let compilation = compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let mut journal = FlakyJournal::healthy(GraphVersion(1));
    let mut execution = Execution::new_at(&compilation.plan, GraphVersion(1), 7_000_000_000);
    let mut world = SessionWorld {
        pending_opens: true,
        ..SessionWorld::default()
    };
    execution
        .accept_input(&mut journal, text_submission("open"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut world)
        .unwrap();
    let retained = journal.inner.records().to_vec();
    for sabotage in 0..3 {
        let mut records = retained.clone();
        match sabotage {
            0 => {
                let Record::Ingress(record) = &mut records[0] else {
                    panic!("ingress");
                };
                record.deadlines = None;
            }
            1 => {
                let Record::Egress(rastok::journal::EgressRecord::Session(record)) =
                    records.last_mut().unwrap()
                else {
                    panic!("opening");
                };
                record.connector_path = node::Path::new(["other"]).unwrap();
            }
            _ => {
                records.push(records.last().unwrap().clone());
            }
        }
        journal.recovery_records = Some(records);
        let before = execution.inspection();
        assert!(matches!(
            execution.recover_from_journal(&journal),
            Err(ExecutionFault::InvalidRecovery { .. })
        ));
        assert_eq!(before, execution.inspection());
        assert_eq!(retained, journal.inner.records());
    }
    journal.recovery_records = None;
    let mut restored = Execution::new_at(&compilation.plan, GraphVersion(1), 11_000_000_000);
    restored.recover_from_journal(&journal).unwrap();
    restored
        .advance_to(&mut journal, &mut world, 11_000_000_000)
        .unwrap();
    assert!(restored.outputs().is_empty());
    restored
        .advance_to(&mut journal, &mut world, 12_000_000_000)
        .unwrap();
    assert_eq!(restored.outputs().len(), 1);
    assert_eq!(source_exit_cases(&restored), ["TimeoutExpired"]);
    assert_eq!(
        world.opened.len(),
        1,
        "the retained opening is never re-offered"
    );
    restored.recover_from_journal(&journal).unwrap();
    assert_eq!(restored.outputs().len(), 1);
}

#[test]
fn local_session_contracts_refuse_peer_roots_before_journaling() {
    for (contract, settings, stderr) in [
        (
            "WebSocketClient",
            "url = \"ws://example.test/socket\"; handshake_timeout = duration(\"5s\"); codec = text",
            "",
        ),
        (
            "Process",
            "mode = service; command = \"cat\"; codec = lines",
            "socket.err -> nothing",
        ),
    ] {
        let source = format!(
            r#"
graph app : App {{
  input in: text
  connector socket : {contract} {{
    {settings}
    behavior = deterministic
  }}
  in -> socket
  socket.out -> nothing
  socket.exit -> nothing
  {stderr}
}}
"#
        );
        let compiled = compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
        let mut execution = Execution::new(&compiled.plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        let before = execution.inspection();
        assert!(matches!(
            execution.accept_connector_output(
                &mut journal,
                &websocket_proposal(1, "", InboundSession::Open),
            ),
            Err(ExecutionFault::PlanInvariant {
                reason: "this Contract does not admit peer Session openings",
                ..
            })
        ));
        assert!(
            journal.records().is_empty(),
            "{contract}: no forged peer root"
        );
        assert_eq!(
            execution.inspection(),
            before,
            "{contract}: no partial admission"
        );
    }
}

#[test]
fn a_refused_ingress_commit_installs_no_exchange_and_keeps_the_same_identity() {
    for (contract, settings, framing) in [
        ("HttpServer", "codec = text", None),
        (
            "WebSocketServer",
            "codec = text; handshake_timeout = duration(\"5s\")",
            Some(InboundSession::Open),
        ),
    ] {
        let source = format!(
            r#"
graph app : App {{
  input in: text
  output out: text
  connector socket : {contract} {{
    listen = "127.0.0.1:0"; {settings}
  }}
  in -> nothing
  socket.out -> out
  socket.exit -> nothing
}}
"#
        );
        let compiled = compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
        let mut proposal = websocket_proposal(1, "request", InboundSession::Open);
        proposal.proposal.session = framing;

        let mut reference = Execution::new(&compiled.plan, GraphVersion(1));
        let mut reference_journal = MemJournal::new(GraphVersion(1));
        let expected = reference
            .accept_connector_output(&mut reference_journal, &proposal)
            .unwrap();

        let mut execution = Execution::new(&compiled.plan, GraphVersion(1));
        let mut journal = FlakyJournal::healthy(GraphVersion(1));
        let before = execution.inspection();
        journal.refuse_ingress = true;
        assert!(matches!(
            execution.accept_connector_output(&mut journal, &proposal),
            Err(ExecutionFault::Journal(rastok::journal::Error::Storage(_)))
        ));
        assert_eq!(
            execution.inspection(),
            before,
            "{contract}: no partial installation"
        );
        assert!(journal.inner.records().is_empty());
        assert!(journal.open_inbound_calls().is_empty());
        assert!(journal.live_sessions().is_empty());

        journal.refuse_ingress = false;
        assert_eq!(
            execution
                .accept_connector_output(&mut journal, &proposal)
                .unwrap(),
            expected
        );
        assert_eq!(journal.inner.records(), reference_journal.records());
        assert_eq!(execution.inspection(), reference.inspection());
        assert_eq!(
            execution.outputs().len(),
            usize::from(framing.is_none()),
            "a Call request is routed; a Session opening is not a peer message"
        );
    }
}

#[test]
fn a_session_opening_requires_its_journal_identity_before_commit() {
    let source = r#"
graph app : App {
  input in: text
  connector socket : WebSocketServer {
    listen = "127.0.0.1:0"; codec = text; handshake_timeout = duration("5s")
  }
  in -> nothing
  socket.out -> nothing
  socket.exit -> nothing
}
"#;
    let compiled = compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let mut execution = Execution::new(&compiled.plan, GraphVersion(1));
    let mut journal = FlakyJournal::healthy(GraphVersion(1));
    journal.omit_prepared_session = true;
    let before = execution.inspection();
    assert!(matches!(
        execution.accept_connector_output(
            &mut journal,
            &websocket_proposal(1, "", InboundSession::Open),
        ),
        Err(ExecutionFault::PlanInvariant {
            reason: "a prepared Session opening lost its logical reference",
            ..
        })
    ));
    assert!(
        journal.inner.records().is_empty(),
        "refuse before the accepting commit"
    );
    assert_eq!(execution.inspection(), before);
}

#[test]
fn server_orphan_writes_cannot_open_or_guess_a_session_after_recovery() {
    let source = r#"
graph app : App {
  input in: text
  connector socket : WebSocketServer {
    listen = "127.0.0.1:0"; handshake_timeout = duration("5s"); codec = text
  }
  in -> socket
  socket.out -> nothing
  socket.exit -> nothing
}
"#;
    let compiled = compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let mut expected_records = None;
    for recover in [false, true] {
        let mut execution = Execution::new(&compiled.plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut world = SessionWorld::default();
        for live_peers in [0, 1] {
            if live_peers == 1 {
                assert!(matches!(
                    execution
                        .accept_connector_output(
                            &mut journal,
                            &websocket_proposal(1, "", InboundSession::Open),
                        )
                        .unwrap(),
                    IngressAdmission::Accepted(_)
                ));
            }
            execution
                .accept_input(&mut journal, text_submission("uncorrelated"))
                .unwrap();
            if recover {
                execution.recover_from_journal(&journal).unwrap();
            }
            execution
                .run_ready_with_drivers(&mut journal, &mut world)
                .unwrap();
            assert_eq!(journal.live_sessions().len(), live_peers);
            assert!(world.opened.is_empty(), "an orphan cannot open a client");
            assert!(world.writes.is_empty(), "one live peer is not a correlator");
            if recover {
                let retained = journal.records().to_vec();
                execution.recover_from_journal(&journal).unwrap();
                assert_eq!(journal.records(), retained);
            }
        }
        if let Some(expected) = &expected_records {
            assert_eq!(
                journal.records(),
                expected,
                "recovery changes no dispositions"
            );
        } else {
            expected_records = Some(journal.records().to_vec());
        }
    }
}

#[test]
fn inbound_sessions_recover_peer_messages_without_crossing_two_openings() {
    let source = r#"
graph app : App {
  input in: text
  output out: text
  output exit: variant
  connector socket : WebSocketServer {
    listen = "127.0.0.1:0"; path = "/socket"
    handshake_timeout = duration("5s"); codec = text
  }
  in -> socket
  socket.out -> out
  socket.exit -> exit
}
"#;
    let compilation = compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = SessionWorld::default();
    let mut sessions = Vec::new();
    for sequence in [1, 2] {
        let IngressAdmission::Accepted(accepted) = execution
            .accept_connector_output(
                &mut journal,
                &websocket_proposal(sequence, "", InboundSession::Open),
            )
            .unwrap()
        else {
            panic!("opening admitted");
        };
        let Some(InboundExchangeId::Session(session)) = accepted.exchange else {
            panic!("Session reference");
        };
        sessions.push(session);
        execution.recover_from_journal(&journal).unwrap();
    }
    execution.run_ready(&mut journal).unwrap();
    for (sequence, text, session) in [(3, "alice", sessions[0]), (4, "bob", sessions[1])] {
        execution
            .accept_connector_output(
                &mut journal,
                &websocket_proposal(sequence, text, InboundSession::Message(session)),
            )
            .unwrap();
        execution.recover_from_journal(&journal).unwrap();
    }
    assert_eq!(execution.outputs().len(), 2);
    for session in sessions.into_iter().rev() {
        world.close(session);
        execution.poll_drivers(&mut journal, &mut world).unwrap();
        execution.run_ready(&mut journal).unwrap();
        execution.recover_from_journal(&journal).unwrap();
    }
    assert_eq!(execution.outputs().len(), 4);
    assert!(journal.live_sessions().is_empty());
    assert!(world.opened.is_empty());
    assert!(world.writes.is_empty());
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
     behavior = deterministic
  }
  in -> socket -> out
  socket.exit -> nothing
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the WebSocket session probe checks");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = SessionWorld::default();

    execution
        .accept_input(&mut journal, text_submission("hello"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut world)
        .unwrap();
    assert_eq!(world.opened.len(), 1);
    assert!(
        journal.live_sessions().is_empty(),
        "the opening is not confirmed yet"
    );
    execution.poll_drivers(&mut journal, &mut world).unwrap();
    assert_eq!(journal.live_sessions().len(), 1);

    execution
        .accept_input(&mut journal, text_submission("again"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut world)
        .unwrap();
    assert_eq!(world.writes.len(), 1);
    assert_eq!(world.writes[0].0, world.opened[0]);
    execution.poll_drivers(&mut journal, &mut world).unwrap();
    assert!(journal.open_session_writes().is_empty());

    world.close(world.opened[0]);
    execution.poll_drivers(&mut journal, &mut world).unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut world)
        .unwrap();
    assert!(journal.live_sessions().is_empty());
    assert!(execution.is_stable());
    assert_eq!(source_exit_cases(&execution), ["Closed"]);
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
     behavior = deterministic
  }
  in -> socket -> out
  socket.exit -> nothing
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the bounded Session probe checks");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = SessionWorld {
        hold_writes: true,
        ..SessionWorld::default()
    };

    execution
        .accept_input(&mut journal, text_submission("open"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut world)
        .unwrap();
    execution.poll_drivers(&mut journal, &mut world).unwrap();

    for _ in 0..1_024 {
        execution
            .accept_input(&mut journal, text_submission("held"))
            .unwrap();
        execution
            .run_ready_with_drivers(&mut journal, &mut world)
            .unwrap();
    }
    assert_eq!(world.writes.len(), 1_024);

    execution
        .accept_input(&mut journal, text_submission("still-bounded"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut world)
        .unwrap();
    assert_eq!(
        world.writes.len(),
        1_024,
        "the durable write table cannot become an unbounded side queue"
    );

    world.settle_write(0);
    execution.poll_drivers(&mut journal, &mut world).unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut world)
        .unwrap();
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
     behavior = deterministic
  }
  in -> socket -> out
  socket.exit -> nothing
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the WebSocket timeout probe checks");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = SessionWorld {
        pending_opens: true,
        ..SessionWorld::default()
    };

    execution
        .accept_input(&mut journal, text_submission("hello"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut world)
        .unwrap();
    assert_eq!(world.opened.len(), 1);
    assert_eq!(journal.open_session_writes().len(), 0);
    execution.poll_drivers(&mut journal, &mut world).unwrap();
    assert_eq!(world.opened.len(), 2, "one pump makes one bounded retry");

    execution
        .advance_to(&mut journal, &mut world, 5_000_000_000)
        .unwrap();
    assert!(journal.live_sessions().is_empty());
    assert!(execution.is_stable());
    assert_eq!(source_exit_cases(&execution), ["TimeoutExpired"]);
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
  in -> socket -> out
  socket.exit -> nothing
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the inbound WebSocket session probe checks");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = SessionWorld::default();

    let open = |execution: &mut Execution<'_>, journal: &mut MemJournal, reference| {
        let IngressAdmission::Accepted(accepted) = execution
            .accept_connector_output(
                journal,
                &websocket_proposal(reference, "", InboundSession::Open),
            )
            .expect("the bounded opening admits")
        else {
            panic!("the uncongested opening must commit")
        };
        let Some(InboundExchangeId::Session(session)) = accepted.exchange else {
            panic!("an opening must have a SessionId")
        };
        session
    };
    let first = open(&mut execution, &mut journal, 1);
    let second = open(&mut execution, &mut journal, 2);
    assert!(
        execution.outputs().is_empty(),
        "a handshake is causal session state, not an empty peer message"
    );

    for (reference, text, session) in [(3, "alice", first), (4, "bob", second)] {
        assert!(matches!(
            execution
                .accept_connector_output(
                    &mut journal,
                    &websocket_proposal(reference, text, InboundSession::Message(session)),
                )
                .expect("the message admission is healthy"),
            IngressAdmission::Accepted(_)
        ));
    }
    assert_eq!(execution.outputs().len(), 2);
    for (output, (text, expected)) in execution
        .outputs()
        .iter()
        .zip([("alice", first), ("bob", second)])
    {
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
    execution.poll_drivers(&mut journal, &mut world).unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut world)
        .unwrap();
    assert!(execution.is_stable());
    assert_eq!(source_exit_cases(&execution), ["Closed", "Closed"]);
    let exit_sessions = execution
        .envelope_views()
        .iter()
        .filter(|observed| observed.node.is_some() && observed.port.name() == "exit")
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
  in -> pick -> out
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    execution
        .accept_input(&mut journal, submission("salut"))
        .unwrap();
    let error = execution
        .run_ready(&mut journal)
        .expect_err("a failed narrowing is an Execution fault, not a domain result");
    assert!(matches!(error, ExecutionFault::Fault(_)), "{error:?}");
    assert!(
        journal.records().iter().all(|record| !matches!(
            record,
            Record::Transaction(transaction)
                if matches!(transaction.trigger, node::Trigger::Port(_))
        )),
        "the faulted transaction must not consume or emit"
    );
    assert!(
        execution
            .envelope_views()
            .iter()
            .all(|event| event.port.name() != "err"),
        "no implicit err output exists"
    );
    assert!(
        !execution.is_stable(),
        "the triggering delivery stays reserved for supervision/recovery"
    );
}

/// A journal that refuses commits on demand — the durable-store
/// failure, scripted.
struct FlakyJournal {
    inner: MemJournal,
    refuse_ingress: bool,
    omit_prepared_session: bool,
    refuse_commits: bool,
    refuse_opens: bool,
    refuse_closes: bool,
    refuse_call: Option<rastok::connector::exchange::CallId>,
    refuse_close_requests: bool,
    recovery_records: Option<Vec<Record>>,
}

impl FlakyJournal {
    fn healthy(version: GraphVersion) -> Self {
        Self {
            inner: MemJournal::new(version),
            refuse_ingress: false,
            omit_prepared_session: false,
            refuse_commits: false,
            refuse_opens: false,
            refuse_closes: false,
            refuse_call: None,
            refuse_close_requests: false,
            recovery_records: None,
        }
    }
}

impl Journal for FlakyJournal {
    fn visit_records(&self, visit: &mut dyn FnMut(&Record)) -> Result<(), rastok::journal::Error> {
        if let Some(records) = &self.recovery_records {
            for record in records {
                visit(record);
            }
            return Ok(());
        }
        self.inner.visit_records(visit)
    }

    fn loop_protocol(
        &self,
    ) -> Result<rastok::journal::loop_protocol::LoopView, rastok::journal::Error> {
        self.inner.loop_protocol()
    }
    fn retained_observations_from(
        &self,
        start: usize,
    ) -> Vec<rastok::observation::RecordObservation> {
        self.inner.retained_observations_from(start)
    }

    fn prepare_ingress(
        &self,
        submission: IngressProposal,
    ) -> Result<rastok::journal::PreparedIngress, rastok::journal::Error> {
        self.inner.prepare_ingress(submission)
    }
    fn prepare_connector_ingress(
        &self,
        mut submission: IngressProposal,
    ) -> Result<rastok::journal::PreparedIngress, rastok::journal::Error> {
        if self.omit_prepared_session {
            submission.session = None;
        }
        self.inner.prepare_connector_ingress(submission)
    }
    fn commit_ingress(
        &mut self,
        prepared: rastok::journal::PreparedIngress,
        routes: Vec<rastok::journal::RouteTarget>,
    ) -> Result<rastok::journal::CommittedIngress, rastok::journal::Error> {
        if self.refuse_ingress {
            return Err(rastok::journal::Error::Storage(String::from("scripted")));
        }
        self.inner.commit_ingress(prepared, routes)
    }
    fn instance_generation_for_admission(
        &self,
        address: &instance::InstanceAddress,
    ) -> Result<instance::Generation, rastok::journal::Error> {
        self.inner.instance_generation_for_admission(address)
    }
    fn commit(
        &mut self,
        transaction: rastok::journal::TransactionRecord,
    ) -> Result<(), rastok::journal::Error> {
        if self.refuse_commits || (self.refuse_close_requests && transaction.close_scope.is_some())
        {
            return Err(rastok::journal::Error::Storage(String::from("scripted")));
        }
        self.inner.commit(transaction)
    }
    fn open_egress(
        &mut self,
        intent: CallOpen,
        attempt: Option<rastok::journal::retry_protocol::AttemptId>,
    ) -> Result<(), rastok::journal::Error> {
        if self.refuse_opens {
            return Err(rastok::journal::Error::Storage(String::from("scripted")));
        }
        self.inner.open_egress(intent, attempt)
    }
    fn open_outbound_session(&mut self, intent: SessionOpen) -> Result<(), rastok::journal::Error> {
        self.inner.open_outbound_session(intent)
    }
    fn confirm_session_open(
        &mut self,
        session: &SessionId,
    ) -> Result<CloseVerdict, rastok::journal::Error> {
        self.inner.confirm_session_open(session)
    }
    fn open_session_write(&mut self, intent: SessionWrite) -> Result<(), rastok::journal::Error> {
        self.inner.open_session_write(intent)
    }
    fn settle_session_write(
        &mut self,
        write_ref: &SessionWriteId,
        abandoned: bool,
    ) -> Result<CloseVerdict, rastok::journal::Error> {
        self.inner.settle_session_write(write_ref, abandoned)
    }
    fn close_egress(
        &mut self,
        call_id: &rastok::connector::exchange::CallId,
        closure: rastok::journal::Closure,
        closed_at: i64,
        schedule: Option<Box<rastok::journal::deadline::Deadlines>>,
    ) -> Result<rastok::journal::CloseVerdict, rastok::journal::Error> {
        if self.refuse_closes || self.refuse_call.as_ref() == Some(call_id) {
            return Err(rastok::journal::Error::Storage(String::from("scripted")));
        }
        self.inner
            .close_egress(call_id, closure, closed_at, schedule)
    }
    fn abandon_egress(
        &mut self,
        call_id: &rastok::connector::exchange::CallId,
        closed_at: i64,
    ) -> Result<rastok::journal::CloseVerdict, rastok::journal::Error> {
        if self.refuse_closes {
            return Err(rastok::journal::Error::Storage(String::from("scripted")));
        }
        self.inner.abandon_egress(call_id, closed_at)
    }
    fn find_envelope(&self, id: rastok::envelope::Id) -> Option<Envelope> {
        self.inner.find_envelope(id)
    }
    fn find_call(&self, call_id: &rastok::connector::exchange::CallId) -> Option<CallOpen> {
        self.inner.find_call(call_id)
    }
    fn open_calls(&self) -> Vec<CallOpen> {
        self.inner.open_calls()
    }
    fn open_inbound_calls(&self) -> Vec<rastok::journal::InboundCallIngressRecord> {
        self.inner.open_inbound_calls()
    }
    fn live_sessions(&self) -> Vec<SessionId> {
        self.inner.live_sessions()
    }
    fn open_session_writes(&self) -> Vec<SessionWrite> {
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
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    execution
        .accept_input(&mut journal, submission("salut"))
        .unwrap();
    let error = execution.step(&mut journal).unwrap_err();
    assert!(matches!(error, ExecutionFault::Journal(_)));
    // The store recovers; the same delivery replays and commits —
    // nothing was consumed by the refusal.
    journal.refuse_commits = false;
    assert!(
        execution.step(&mut journal).unwrap(),
        "the delivery was still there"
    );
    assert!(matches!(
        journal.inner.records().last().unwrap(),
        Record::Transaction(_)
    ));
}

#[test]
fn single_pass_recovery_retains_a_close_request_after_a_committed_start() {
    let compilation = compile(
        &[r#"
graph app : App {
  input in: text
  output out: int
  operator count {
    input in: text
    output out: int
    lifetime = { kind = single_pass }
    var count: int = 0
    on in { count = count + 1 }
    on stop { out = count }
  }
  in -> count -> out
}
"#],
        &ContractCatalog::try_m0().unwrap(),
    )
    .unwrap();
    let mut journal = FlakyJournal::healthy(GraphVersion(1));
    journal.refuse_close_requests = true;
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    execution
        .accept_input(&mut journal, text_submission("first"))
        .unwrap();
    execution.step(&mut journal).unwrap();
    assert!(matches!(
        execution.step(&mut journal),
        Err(ExecutionFault::Journal(_))
    ));
    let count_starts = |records: &[Record]| {
        records.iter().filter(|record| matches!(record,
        Record::Transaction(record) if record.owner_path.to_string() == "count" && record.trigger == node::Trigger::Lifecycle(node::Lifecycle::Start))).count()
    };
    assert_eq!(count_starts(journal.inner.records()), 1);
    journal.refuse_close_requests = false;
    let prefix = journal.inner.records().to_vec();
    let mut recovered = Execution::new(&compilation.plan, GraphVersion(1));
    recovered.recover_from_journal(&journal).unwrap();
    assert_eq!(journal.inner.records(), prefix);
    assert!(recovered.has_ready_work());
    recovered.run_ready(&mut journal).unwrap();
    assert_eq!(
        count_starts(journal.inner.records()),
        1,
        "the accepted start is never repeated"
    );
    assert_eq!(recovered.outputs().len(), 1);
    assert_eq!(
        recovered.outputs()[0].envelope.data(),
        &Data::materialized(Value::Int(1))
    );
    recovered
        .accept_input(&mut journal, text_submission("second"))
        .unwrap();
    recovered.run_ready(&mut journal).unwrap();
    assert_eq!(count_starts(journal.inner.records()), 2);
    assert_eq!(
        recovered.outputs()[1].envelope.data(),
        &Data::materialized(Value::Int(1))
    );
}

#[test]
fn recovery_binds_loop_context_to_the_consumed_delivery_not_just_its_owner() {
    use rastok::journal::loop_protocol::{CommandProduced, Fact, LoopView};

    let compilation = compile(
        &[r#"
graph app : App {
  input in: text
  output out: text
  operator work : Loop {
    input in: text
    output err: union { LoopExpired { last_state: text } }
    output out: text
    Loop.fuel = 1
    on in { out = data }
  }
  in -> work -> out
  work.err -> nothing
}
"#],
        &ContractCatalog::try_m0().unwrap(),
    )
    .unwrap();
    let mut journal = FlakyJournal::healthy(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    execution
        .accept_input(&mut journal, text_submission("alice"))
        .unwrap();
    execution
        .accept_input(&mut journal, text_submission("bob"))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();

    // Both exact admissions may exist before either application consumes its
    // input. The following prefix differs only in manager scheduling, not in
    // the two ordinary deliveries or their per-edge cursor order.
    let original = journal.inner.records().to_vec();
    let mut prefix: Vec<_> = original
        .iter()
        .filter(|record| match record {
            Record::Ingress(_) => true,
            Record::Transaction(record) => {
                record.trigger == node::Trigger::Lifecycle(node::Lifecycle::Start)
                    || record.loop_protocol().is_some_and(|packet| {
                        packet.facts.iter().any(|fact| {
                            matches!(
                                fact,
                                Fact::Opened { .. } | Fact::Admitted { iteration: 0, .. }
                            )
                        })
                    })
            }
            Record::Egress(_) => false,
        })
        .cloned()
        .collect();
    let producer = original
        .iter()
        .find(|record| {
            matches!(record, Record::Transaction(record)
            if record.loop_protocol().is_some_and(|packet| !packet.produced.is_empty()))
        })
        .unwrap()
        .clone();
    let Record::Transaction(transaction) = &producer else {
        panic!("application");
    };
    let first_run = transaction.loop_protocol().unwrap().context[0];
    let fold = LoopView::from_records(&prefix).unwrap();
    let other = *fold.runs().find(|(run, _)| **run != first_run).unwrap().0;
    prefix.push(producer);
    journal.recovery_records = Some(prefix.clone());
    let mut valid = Execution::new(&compilation.plan, GraphVersion(1));
    valid.recover_from_journal(&journal).unwrap();

    for sabotage in 0..4 {
        let mut forged = prefix.clone();
        if sabotage == 3 {
            forged.retain(|record| {
                !matches!(record, Record::Transaction(record)
                if record.loop_protocol().is_some_and(|packet| !packet.facts.is_empty()))
            });
        }
        let Record::Transaction(record) = forged.last_mut().unwrap() else {
            panic!("application");
        };
        if sabotage == 0 {
            record.consumed.clear();
        } else if sabotage == 1 {
            let packet = record.loop_record.as_deref_mut().unwrap();
            packet.context = vec![other];
            let command = &packet.produced[0];
            packet.produced[0] =
                CommandProduced::new(other, command.iteration, command.value.clone());
        } else {
            record.loop_record = None;
        }
        // Protocol algebra and owner membership are necessary but do not
        // establish which queued input the producer actually consumed.
        LoopView::from_records(&forged).unwrap();
        journal.recovery_records = Some(forged);
        let mut rejected = Execution::new(&compilation.plan, GraphVersion(1));
        let before = rejected.inspection();
        let Err(ExecutionFault::InvalidRecovery { reason, .. }) =
            rejected.recover_from_journal(&journal)
        else {
            panic!("sabotage {sabotage} did not refuse as invalid recovery");
        };
        assert_eq!(
            reason,
            if sabotage == 3 {
                "a Loop entry is consumed before its retained run admission"
            } else {
                "the Loop context differs from the exact consumed admission"
            },
            "sabotage {sabotage}"
        );
        assert_eq!(rejected.inspection(), before);
        assert_eq!(journal.inner.records(), original);
    }
}

const GRAPH_MANAGER_PROBE: &str = r#"
graph app : App {
  input in: text
  output expired: union { LoopExpired { last_state: text } }
  graph cycle : Loop {
    input in: text
    output err: union { LoopExpired { last_state: text } }
    Loop.fuel = 2
    operator worker {
      input in: text
      output out: text
      on in { out = data }
    }
    operator z_audit { input in: text; on in { nothing } }
    in -> { worker; z_audit }
    worker.out -> Loop.again
  }
  in -> cycle
  cycle.err -> expired
}
"#;

const GRAPH_CALL_PROBE: &str = r#"
graph app : App {
  input in: text
  output expired: union { LoopExpired { last_state: text } }
  graph cycle : Loop {
    input in: text
    output err: union { LoopExpired { last_state: text } }
    Loop.fuel = 2
    operator decide { input in: text; output out: text; on in { out = data } }
    connector worker : Process {
      command = "cat"; codec = lines
      idempotent = true; behavior = deterministic
      timeout = duration("5s")
    }
    operator audit { input in: text; on in { nothing } }
    in -> { decide; worker }
    decide.out -> Loop.again
    worker.out -> audit
    worker.err -> nothing
    worker.exit -> nothing
  }
  in -> cycle
  cycle.err -> expired
}
"#;

#[test]
fn graph_call_commits_and_openings_are_atomic_under_storage_refusal() {
    let compilation = compile(&[GRAPH_CALL_PROBE], &ContractCatalog::try_m0().unwrap()).unwrap();
    let mut histories = Vec::new();
    for refuse in [false, true] {
        let mut journal = FlakyJournal::healthy(GraphVersion(1));
        let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
        let mut world = ClosingWorld::default();
        execution
            .accept_input(&mut journal, text_submission("same"))
            .unwrap();
        let mut answered = 0;
        for _ in 0..64 {
            if execution.has_ready_work() {
                if refuse {
                    let before = execution.inspection();
                    let records = journal.inner.records().to_vec();
                    journal.refuse_commits = true;
                    journal.refuse_opens = true;
                    assert!(matches!(
                        execution.step_with_drivers(&mut journal, &mut CallDriver(&mut world)),
                        Err(ExecutionFault::Journal(rastok::journal::Error::Storage(_)))
                    ));
                    assert_eq!(execution.inspection(), before);
                    assert_eq!(journal.inner.records(), records);
                    journal.refuse_commits = false;
                    journal.refuse_opens = false;
                }
                assert!(
                    execution
                        .step_with_drivers(&mut journal, &mut CallDriver(&mut world))
                        .unwrap()
                );
            }
            execution
                .poll_drivers(&mut journal, &mut CallDriver(&mut world))
                .unwrap();
            if world.started.len() > answered {
                world.answers.push(CallResponse {
                    call_id: world.started[answered],
                    completion: Completion::process(
                        vec![Data::materialized(Value::Text("frame".into()))],
                        vec![],
                        ProcessExit::Exited {
                            exit_code: 0,
                            signal: None,
                        },
                    ),
                });
                answered += 1;
                execution
                    .poll_drivers(&mut journal, &mut CallDriver(&mut world))
                    .unwrap();
            }
            if execution.is_stable() {
                break;
            }
        }
        assert!(execution.is_stable());
        assert_eq!(execution.outputs().len(), 1);
        assert_eq!(world.started.len(), 2);
        histories.push(journal.inner.records().to_vec());
    }
    assert_eq!(histories[0], histories[1]);
}

#[test]
fn graph_call_recovery_refuses_missing_contexts_and_unsettled_obligations() {
    use rastok::journal::loop_protocol::{Fact, LoopView};
    let compilation = compile(&[GRAPH_CALL_PROBE], &ContractCatalog::try_m0().unwrap()).unwrap();
    let mut journal = FlakyJournal::healthy(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = ClosingWorld::default();
    execution
        .accept_input(&mut journal, text_submission("same"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    execution
        .poll_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    world.answers.push(CallResponse {
        call_id: world.started[0],
        completion: Completion::process(
            vec![Data::materialized(Value::Text("frame".into()))],
            vec![],
            ProcessExit::Exited {
                exit_code: 0,
                signal: None,
            },
        ),
    });
    execution
        .poll_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    while !journal.inner.records().iter().any(|record| matches!(record, Record::Transaction(record)
        if record.loop_protocol().is_some_and(|packet| packet.facts.iter().any(|fact| matches!(fact, Fact::Advanced { .. }))))) {
        assert!(execution.step_with_drivers(&mut journal, &mut CallDriver(&mut world)).unwrap());
    }
    let original = journal.inner.records().to_vec();
    let completed = original.iter().position(|record| matches!(record, Record::Transaction(record)
        if record.owner_path.to_string() == "cycle.worker" && matches!(record.trigger, node::Trigger::Port(_)))).unwrap();
    for cut in 1..=original.len() {
        journal.recovery_records = Some(original[..cut].to_vec());
        let mut restored = Execution::new(&compilation.plan, GraphVersion(1));
        restored
            .recover_with_drivers(&journal, &mut CallDriver(&mut world))
            .unwrap_or_else(|error| panic!("prefix {cut}: {error}"));
        assert_eq!(world.started.len(), 1);
    }
    for sabotage in 0..3 {
        let mut prefix = if sabotage == 2 {
            original.clone()
        } else {
            original[..=completed].to_vec()
        };
        if sabotage == 2 {
            prefix.retain(|record| !matches!(record, Record::Transaction(record)
                if (record.owner_path.to_string() == "cycle.worker" || record.owner_path.to_string() == "cycle.audit") && matches!(record.trigger, node::Trigger::Port(_))));
        } else {
            let Record::Transaction(record) = prefix.last_mut().unwrap() else {
                panic!("completion");
            };
            let packet = record.loop_record.as_deref_mut().unwrap();
            packet.routes.clear();
            if sabotage == 1 {
                record.loop_record = None;
            }
        }
        LoopView::from_records(&prefix)
            .unwrap_or_else(|error| panic!("sabotage {sabotage}: {error:?}"));
        journal.recovery_records = Some(prefix);
        let before = execution.inspection();
        assert!(
            matches!(
                execution.recover_with_drivers(&journal, &mut CallDriver(&mut world)),
                Err(ExecutionFault::InvalidRecovery { .. })
            ),
            "sabotage {sabotage}"
        );
        assert_eq!(execution.inspection(), before);
        assert_eq!(journal.inner.records(), original);
        assert_eq!(world.started.len(), 1);
    }
}

#[test]
fn graph_manager_commit_failures_preserve_the_next_boundary() {
    let compilation = compile(&[GRAPH_MANAGER_PROBE], &ContractCatalog::try_m0().unwrap()).unwrap();
    let mut expected = FlakyJournal::healthy(GraphVersion(1));
    let mut continuous = Execution::new(&compilation.plan, GraphVersion(1));
    continuous
        .accept_input(&mut expected, text_submission("same"))
        .unwrap();
    continuous.run_ready(&mut expected).unwrap();
    let mut journal = FlakyJournal::healthy(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    execution
        .accept_input(&mut journal, text_submission("same"))
        .unwrap();
    while execution.has_ready_work() {
        let before = execution.inspection();
        let records = journal.inner.records().to_vec();
        journal.refuse_commits = true;
        assert!(matches!(
            execution.step(&mut journal),
            Err(ExecutionFault::Journal(rastok::journal::Error::Storage(_)))
        ));
        assert_eq!(execution.inspection(), before);
        assert_eq!(journal.inner.records(), records);
        journal.refuse_commits = false;
        assert!(execution.step(&mut journal).unwrap());
    }
    assert_eq!(journal.inner.records(), expected.inner.records());
    assert_eq!(execution.outputs(), continuous.outputs());
}

#[test]
fn graph_recovery_rederives_admission_context_and_quiescence() {
    use rastok::journal::loop_protocol::{CommandProduced, CommandValue, Fact, LoopView};
    let compilation = compile(&[GRAPH_MANAGER_PROBE], &ContractCatalog::try_m0().unwrap()).unwrap();
    let mut journal = FlakyJournal::healthy(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    execution
        .accept_input(&mut journal, text_submission("same"))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();
    let original = journal.inner.records().to_vec();
    for sabotage in 0..4 {
        let index = original
            .iter()
            .position(|record| {
                let Record::Transaction(record) = record else {
                    return false;
                };
                record.loop_protocol().is_some_and(|packet| match sabotage {
                    0 | 1 => packet
                        .facts
                        .iter()
                        .any(|fact| matches!(fact, Fact::Admitted { .. })),
                    2 => !packet.produced.is_empty(),
                    _ => packet
                        .facts
                        .iter()
                        .any(|fact| matches!(fact, Fact::Advanced { .. })),
                })
            })
            .unwrap();
        let mut prefix = original[..=index].to_vec();
        journal.recovery_records = Some(prefix.clone());
        let mut valid = Execution::new(&compilation.plan, GraphVersion(1));
        valid.recover_from_journal(&journal).unwrap();
        let Record::Transaction(record) = prefix.last_mut().unwrap() else {
            panic!("transaction")
        };
        let packet = record.loop_record.as_deref_mut().unwrap();
        match sabotage {
            0 => packet.routes.clear(),
            1 => record.consumed.clear(),
            2 => {
                let command = &packet.produced[0];
                packet.produced[0] = CommandProduced::new(command.run, command.iteration, CommandValue::Done { cause: command.value.cause(), kind: rastok::journal::loop_protocol::DoneKind::Explicit });
            }
            _ => prefix.retain(|record| !matches!(record, Record::Transaction(record) if record.owner_path.to_string() == "cycle.z_audit" && matches!(record.trigger, node::Trigger::Port(_)))),
        }
        LoopView::from_records(&prefix).unwrap();
        journal.recovery_records = Some(prefix);
        let before = execution.inspection();
        assert!(
            matches!(
                execution.recover_from_journal(&journal),
                Err(ExecutionFault::InvalidRecovery { .. })
            ),
            "sabotage {sabotage}"
        );
        assert_eq!(execution.inspection(), before);
        assert_eq!(journal.inner.records(), original);
    }
}

#[test]
fn retry_storage_refusals_preserve_batches_and_never_repeat_a_started_effect() {
    use rastok::compiler::{Unit, check, lower, parser};
    use rastok::diagnostics::{Bag, SourceId};
    use rastok::journal::retry_protocol::Fact;
    use rastok::runtime::execution::TimeAdvance;

    let source = r#"
graph app : App {
  input in: text
  output out: text
  output errors: text
  output exit: variant
  connector call : Process + Retry {
    command = "cat"; codec = lines; behavior = deterministic; idempotent = true
    timeout = duration("1s"); Retry.attempts = 2; Retry.on = [TimeoutExpired]
    Retry.delay = { kind = fixed; after = duration("100ms") }
  }
  in -> call -> out
  call.err -> errors; call.exit -> exit
}
"#;
    let catalog = ContractCatalog::try_m0().unwrap();
    let mut diagnostics = Bag::new();
    let id = SourceId::new(0);
    let parsed = parser::parse(id, source, &catalog, &mut diagnostics);
    let hir = check::check(
        &Unit {
            files: vec![(id, parsed)],
        },
        &catalog,
        &mut diagnostics,
        None,
    )
    .unwrap();
    assert!(!diagnostics.has_errors(), "{diagnostics:?}");
    let hash = lower::semantic_hash(&hir);
    let (raw, _) = lower::lower(&hir).unwrap();
    let plan = ValidatedExecutionPlan::try_new(raw).unwrap();

    let advance = |execution: &mut Execution<'_>,
                   journal: &mut FlakyJournal,
                   world: &mut ClosingWorld,
                   target: i64,
                   faulted: bool| {
        let mut sweep = TimeAdvance::new(target);
        let mut steps = 0;
        while !sweep.is_complete() {
            steps += 1;
            assert!(steps < 100, "the finite probe must make progress");
            let records = journal.inner.records().to_vec();
            let inspection = execution.inspection();
            let observations = execution.envelope_views().to_vec();
            let effects = world.started.clone();
            journal.refuse_commits = faulted;
            journal.refuse_opens = faulted;
            let result = sweep.advance_once(execution, journal, &mut CallDriver(world));
            journal.refuse_commits = false;
            journal.refuse_opens = false;
            match result {
                Ok(_) => {}
                Err(error) if faulted => {
                    assert!(
                        matches!(
                            error,
                            ExecutionFault::Journal(rastok::journal::Error::Storage(_))
                                | ExecutionFault::Dispatch(
                                    rastok::runtime::execution::driver_polling::PollFault::Journal(
                                        rastok::journal::Error::Storage(_)
                                    )
                                )
                        ),
                        "{error:?}"
                    );
                    assert_eq!(journal.inner.records(), records);
                    assert_eq!(execution.inspection(), inspection);
                    assert_eq!(execution.envelope_views(), observations);
                    assert_eq!(world.started, effects);
                    assert!(
                        sweep
                            .advance_once(execution, journal, &mut CallDriver(world))
                            .unwrap()
                    );
                }
                Err(error) => panic!("unfaulted transition: {error:?}"),
            }
        }
    };

    let mut reference = None;
    for faulted in [false, true] {
        let mut journal = FlakyJournal::healthy(GraphVersion(1));
        let mut execution = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
        let mut world = ClosingWorld::default();
        execution
            .accept_input(&mut journal, text_submission("same operation"))
            .unwrap();
        advance(&mut execution, &mut journal, &mut world, 0, faulted);
        for (attempt, now) in [(1, 0), (2, 100_000_000)] {
            advance(&mut execution, &mut journal, &mut world, now, faulted);
            execution
                .poll_drivers(&mut journal, &mut CallDriver(&mut world))
                .unwrap();
            assert_eq!(world.started.len(), attempt, "faulted={faulted}, now={now}");
            world.answers.push(CallResponse {
                call_id: *world.started.last().unwrap(),
                completion: Completion::process(
                    vec![Data::materialized(Value::Text(format!("stdout {attempt}")))],
                    vec![Data::materialized(Value::Text(format!("stderr {attempt}")))],
                    if attempt == 1 {
                        ProcessExit::TimeoutExpired
                    } else {
                        ProcessExit::Exited {
                            exit_code: 0,
                            signal: None,
                        }
                    },
                ),
            });
            if faulted {
                let records = journal.inner.records().to_vec();
                let inspection = execution.inspection();
                journal.refuse_closes = true;
                assert!(matches!(
                    execution.poll_drivers(&mut journal, &mut CallDriver(&mut world)),
                    Err(ExecutionFault::Dispatch(
                        rastok::runtime::execution::driver_polling::PollFault::Journal(
                            rastok::journal::Error::Storage(_)
                        )
                    ))
                ));
                assert_eq!(journal.inner.records(), records);
                assert_eq!(execution.inspection(), inspection);
                assert_eq!(world.started.len(), attempt);
                journal.refuse_closes = false;
            }
            execution
                .poll_drivers(&mut journal, &mut CallDriver(&mut world))
                .unwrap();
            advance(&mut execution, &mut journal, &mut world, now, faulted);
            assert_eq!(execution.outputs().len(), if attempt == 1 { 0 } else { 3 });
        }
        assert!(execution.is_stable());
        assert_eq!(world.started.len(), 2);
        assert_ne!(world.started[0], world.started[1]);
        let facts: Vec<_> = journal
            .inner
            .records()
            .iter()
            .flat_map(|record| match record {
                Record::Transaction(record) => record
                    .retry_record
                    .as_deref()
                    .map_or(&[][..], |packet| packet.facts.as_slice()),
                _ => &[],
            })
            .collect();
        assert_eq!(
            facts
                .iter()
                .filter(|fact| matches!(fact, Fact::Held(_)))
                .count(),
            1
        );
        assert_eq!(
            facts
                .iter()
                .filter(|fact| matches!(fact, Fact::Consumed(_)))
                .count(),
            1
        );
        assert_eq!(
            facts
                .iter()
                .filter(|fact| matches!(fact, Fact::Published(_)))
                .count(),
            1
        );
        let result = (
            journal.inner.records().to_vec(),
            execution.outputs().to_vec(),
            world.started,
        );
        if let Some(expected) = &reference {
            assert_eq!(&result, expected);
        } else {
            reference = Some(result);
        }
    }
}

#[test]
fn a_partly_committed_response_batch_survives_the_next_closure_refusal() {
    let compilation = compile(
        &[r#"
graph app : App {
  input in: text
  output out: text
  connector first : Process { command = "cat"; codec = lines; behavior = deterministic }
  connector second : Process { command = "cat"; codec = lines; behavior = deterministic }
  in -> { first; second }
  { first.out; second.out } -> out
  first.err -> nothing; first.exit -> nothing
  second.err -> nothing; second.exit -> nothing
}
"#],
        &ContractCatalog::try_m0().unwrap(),
    )
    .unwrap();
    for refused in 0..2 {
        let mut journal = FlakyJournal::healthy(GraphVersion(1));
        let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
        let mut world = ClosingWorld::default();
        execution
            .accept_input(&mut journal, text_submission("one fan-out"))
            .unwrap();
        execution
            .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
            .unwrap();
        execution
            .poll_drivers(&mut journal, &mut CallDriver(&mut world))
            .unwrap();
        assert_eq!(world.started.len(), 2);
        for call_id in &world.started {
            world.answers.push(CallResponse {
                call_id: *call_id,
                completion: Completion::process(
                    vec![Data::materialized(Value::Text("answer".into()))],
                    vec![],
                    ProcessExit::Exited {
                        exit_code: 0,
                        signal: None,
                    },
                ),
            });
        }
        let late = world.answers[0].clone();
        world.answers.insert(
            0,
            CallResponse {
                call_id: world.started[0],
                completion: Completion::process(
                    vec![Data::materialized(Value::Uint(0))],
                    vec![],
                    ProcessExit::Exited {
                        exit_code: 0,
                        signal: None,
                    },
                ),
            },
        );
        journal.refuse_call = Some(world.started[refused]);
        assert!(matches!(
            execution.poll_drivers(&mut journal, &mut CallDriver(&mut world)),
            Err(ExecutionFault::Dispatch(
                rastok::runtime::execution::driver_polling::PollFault::Journal(
                    rastok::journal::Error::Storage(_)
                )
            ))
        ));
        assert_eq!(journal.inner.open_calls().len(), 2 - refused);
        assert!(execution.outputs().is_empty());
        assert!(
            world.answers.is_empty(),
            "the Driver already handed off both responses"
        );
        world.answers.push(late);
        assert!(
            execution
                .poll_drivers(&mut journal, &mut CallDriver(&mut world))
                .is_err()
        );
        assert_eq!(
            world.answers.len(),
            1,
            "store pressure cannot pull another Driver batch"
        );
        journal.refuse_call = None;
        let report = execution
            .poll_drivers(&mut journal, &mut CallDriver(&mut world))
            .unwrap();
        assert_eq!(
            report.settled, 2,
            "both winners must reach the Execution exactly once"
        );
        assert!(report.discarded.is_empty());
        assert_eq!(
            report.rejected.len(),
            1,
            "the invalid first answer stays observable"
        );
        assert_eq!(world.answers.len(), 1);
        execution
            .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
            .unwrap();
        assert_eq!(execution.outputs().len(), 2);
        assert!(execution.is_stable());
        assert_eq!(world.started.len(), 2);
        let records = journal.inner.records().to_vec();
        let late = execution
            .poll_drivers(&mut journal, &mut CallDriver(&mut world))
            .unwrap();
        assert_eq!(late.settled, 0);
        assert_eq!(late.discarded.len(), 1);
        assert_eq!(journal.inner.records(), records);
    }
}

#[test]
fn call_recovery_requires_run_admission_and_the_retained_terminal_verdict() {
    use rastok::journal::loop_protocol::{CommandProduced, CommandValue, LoopView};

    let compilation = compile(
        &[r#"
graph app : App {
  input in: text
  connector call : Process + Loop {
    command = "cat"; codec = lines
    idempotent = true; behavior = deterministic
    timeout = duration("5s")
    Loop.fuel = 2
  }
  in -> call
  call.out -> nothing
  call.err -> nothing
  call.exit -> nothing
}
"#],
        &ContractCatalog::try_m0().unwrap(),
    )
    .unwrap();
    let mut journal = FlakyJournal::healthy(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = ClosingWorld::default();
    execution
        .accept_input(&mut journal, text_submission("request"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    execution
        .poll_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    assert_eq!(world.started.len(), 1);
    let opening = journal.inner.records().to_vec();
    world.answers.push(CallResponse {
        call_id: world.started[0],
        completion: Completion::process(
            vec![],
            vec![],
            ProcessExit::Exited {
                exit_code: 0,
                signal: None,
            },
        ),
    });
    execution
        .poll_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    execution
        .step_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    let completed = journal.inner.records().to_vec();
    for prefix in [&opening, &completed] {
        journal.recovery_records = Some(prefix.clone());
        Execution::new(&compilation.plan, GraphVersion(1))
            .recover_with_drivers(&journal, &mut CallDriver(&mut world))
            .unwrap();
    }

    for sabotage in 0..2 {
        let mut forged = if sabotage == 0 {
            opening.clone()
        } else {
            completed.clone()
        };
        if sabotage == 0 {
            forged.retain(|record| {
                !matches!(record, Record::Transaction(record)
                if record.loop_protocol().is_some())
            });
        } else {
            let Record::Transaction(record) = forged.last_mut().unwrap() else {
                panic!("Call completion");
            };
            let packet = record.loop_record.as_deref_mut().unwrap();
            let command = &packet.produced[0];
            packet.produced[0] = CommandProduced::new(
                command.run,
                command.iteration,
                CommandValue::Again(command.value.cause()),
            );
            assert!(matches!(
                record.emissions.last().unwrap().envelope.data(),
                Data::Materialized(Value::TaggedUnion(value)) if value.case == "Exited"
            ));
        }
        LoopView::from_records(&forged).unwrap();
        journal.recovery_records = Some(forged);
        let mut recovered = Execution::new(&compilation.plan, GraphVersion(1));
        let before = recovered.inspection();
        let Err(ExecutionFault::InvalidRecovery { reason, .. }) =
            recovered.recover_with_drivers(&journal, &mut CallDriver(&mut world))
        else {
            panic!("sabotage {sabotage} did not refuse as invalid recovery");
        };
        assert_eq!(
            reason,
            if sabotage == 0 {
                "a Call opens before its retained Loop admission"
            } else {
                "a Call transaction contradicts its retained Loop verdict"
            }
        );
        assert_eq!(recovered.inspection(), before);
        assert_eq!(journal.inner.records(), completed);
        assert_eq!(world.started.len(), 1);
        assert!(world.cancelled.is_empty());
    }
}

#[test]
fn recovery_rejects_missing_deadlines_and_changed_call_facts_atomically() {
    let compilation = compile(
        &[r#"
graph app : App {
  input in: text
  connector call : Process {
    command = "cat"; codec = lines
    idempotent = true; behavior = deterministic
    timeout = duration("5s")
  }
  in -> call
  call.out -> nothing
  call.err -> nothing
  call.exit -> nothing
}
"#],
        &ContractCatalog::try_m0().unwrap(),
    )
    .unwrap();
    let mut journal = FlakyJournal::healthy(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = ClosingWorld::default();
    execution
        .accept_input(&mut journal, text_submission("x"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    let call = journal.inner.open_calls()[0].call_id;
    journal
        .inner
        .close_egress(
            &call,
            rastok::journal::Closure::Response(Completion::process(
                vec![],
                vec![],
                ProcessExit::Exited {
                    exit_code: 0,
                    signal: None,
                },
            )),
            0,
            None,
        )
        .unwrap();
    let original = journal.inner.records().to_vec();
    for sabotage in 0..5 {
        let mut records = original.clone();
        match sabotage {
            0 | 1 => {
                let Record::Ingress(inbound) = &mut records[0] else {
                    panic!("ingress");
                };
                if sabotage == 0 {
                    inbound.deadlines = None;
                } else {
                    let schedule = inbound.deadlines.as_ref().unwrap();
                    let mut deadlines = schedule.items().to_vec();
                    deadlines[0].due_at += 1;
                    inbound.deadlines = Some(Box::new(
                        rastok::journal::deadline::Deadlines::try_new(
                            schedule.created_at(),
                            deadlines,
                        )
                        .unwrap(),
                    ));
                }
            }
            2 => {
                let Record::Egress(EgressRecord::Call(closed)) = records.last_mut().unwrap() else {
                    panic!("closure");
                };
                closed.intent.logical_time += 1;
            }
            3 => {
                let opening = records
                    .iter()
                    .position(|record| matches!(record, Record::Egress(_)))
                    .unwrap();
                let opening = records.remove(opening);
                records.insert(1, opening);
            }
            _ => {
                records.pop();
                let Record::Egress(EgressRecord::Call(opening)) = records.last_mut().unwrap()
                else {
                    panic!("opening");
                };
                opening.intent.logical_time = -1;
            }
        }
        journal.recovery_records = Some(records);
        let mut recovered = Execution::new(&compilation.plan, GraphVersion(1));
        let before = recovered.inspection();
        assert!(
            recovered
                .recover_with_drivers(&journal, &mut CallDriver(&mut world))
                .is_err(),
            "sabotage {sabotage}"
        );
        assert_eq!(recovered.inspection(), before);
        assert_eq!(journal.inner.records(), original);
    }
}

const TIMED_PROBE: &str = r#"
graph timed : App {
  connector echo : Process {
mode = job ; command = "cat"
codec = lines
idempotent = true; behavior = deterministic
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

  in -> shout -> echo -> out
  echo.err -> nothing
  echo.exit -> nothing
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

  in -> stamp -> out
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a handler may observe its logical instant");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new_at(&compilation.plan, GraphVersion(1), 1_000);
    let mut world = EchoWorld::default();

    execution
        .accept_input(&mut journal, submission("first"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 2_000)
        .unwrap();
    execution
        .accept_input(&mut journal, submission("second"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();

    let instants: Vec<_> = execution
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
            Record::Ingress(_) | Record::Egress(_) => None,
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

  in -> filter -> out
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a guard may observe its logical instant");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new_at(&compilation.plan, GraphVersion(1), 1_000);
    let mut world = EchoWorld::default();

    execution
        .accept_input(&mut journal, submission("filtered"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();

    assert!(
        execution.outputs().is_empty(),
        "the false guard emits nothing"
    );
    let observed: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction) => transaction.logical_time,
            Record::Ingress(_) | Record::Egress(_) => None,
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
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    execution
        .accept_input(&mut journal, submission("salut"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    assert!(execution.has_open_obligations());
    // Nobody answers; the budget expires at the discrete advance.
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 5_000_000_000)
        .unwrap();
    assert!(execution.is_stable(), "the expiry released the machine");
    assert_eq!(execution.logical_time(), 5_000_000_000);
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
    assert!(
        execution
            .envelope_views()
            .iter()
            .all(|event| event.port.name() != "err")
    );
    assert_eq!(source_exit_cases(&execution), ["TimeoutExpired"]);
    // The claim lifted: a second submission opens a second call.
    execution
        .accept_input(&mut journal, submission("encore"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
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
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut drivers = ProjectionRefusal {
        inner: EchoWorld::default(),
        refuse_scheduler_completion: false,
        refuse_cancel: false,
    };
    execution
        .accept_input(&mut journal, submission("salut"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut drivers)
        .unwrap();
    let facts_before = journal.records().len();
    assert_eq!(journal.open_calls().len(), 1);

    drivers.refuse_scheduler_completion = true;
    assert!(matches!(
        execution.advance_to(&mut journal, &mut drivers, 5_000_000_000),
        Err(ExecutionFault::Dispatch(
            rastok::runtime::execution::driver_polling::PollFault::Driver(
                DriverRouteError::UnavailableOutbound(_)
            )
        ))
    ));
    assert_eq!(journal.records().len(), facts_before);
    assert_eq!(journal.open_calls().len(), 1, "the call remains replayable");

    drivers.refuse_scheduler_completion = false;
    execution
        .advance_to(&mut journal, &mut drivers, 5_000_000_000)
        .unwrap();
    assert_eq!(source_exit_cases(&execution), ["TimeoutExpired"]);
}

#[test]
fn a_cancel_refusal_cannot_strand_an_expiring_call() {
    let compilation = compile(
        &[TIMED_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut drivers = ProjectionRefusal {
        inner: EchoWorld::default(),
        refuse_scheduler_completion: false,
        refuse_cancel: true,
    };
    execution
        .accept_input(&mut journal, submission("salut"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut drivers)
        .unwrap();

    assert!(matches!(
        execution.advance_to(&mut journal, &mut drivers, 5_000_000_000),
        Err(ExecutionFault::Dispatch(
            rastok::runtime::execution::driver_polling::PollFault::Driver(
                DriverRouteError::UnavailableOutbound(_)
            )
        ))
    ));
    assert_eq!(
        journal.open_calls().len(),
        1,
        "a refused cancellation leaves the call and deadline replayable"
    );

    drivers.refuse_cancel = false;
    execution
        .advance_to(&mut journal, &mut drivers, 5_000_000_000)
        .unwrap();
    assert_eq!(source_exit_cases(&execution), ["TimeoutExpired"]);
    assert!(
        execution.is_stable(),
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
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    execution
        .accept_input(&mut journal, submission("un"))
        .unwrap();
    execution
        .accept_input(&mut journal, submission("deux"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    assert_eq!(journal.open_calls().len(), 1, "one open, one queued");
    // Nobody answers; both budgets expire at the same sweep.
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 5_000_000_000)
        .unwrap();
    assert!(execution.is_stable());
    assert_eq!(
        source_exit_cases(&execution),
        ["TimeoutExpired", "TimeoutExpired"],
        "both requests expired exactly once"
    );
    let statuses: Vec<&CallStatus> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Egress(EgressRecord::Call(o)) => Some(&o.status),
            _ => None,
        })
        .collect();
    assert_eq!(
        statuses
            .iter()
            .filter(|status| matches!(status, CallStatus::Open))
            .count(),
        2,
        "both calls opened — the queued one at its expiry"
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| matches!(
                status,
                CallStatus::Closed(rastok::journal::Closure::Timeout)
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
    // The sealed deadline_ids name the obligations the transactions
    // created — one per budgeted delivery.
    let sealed: usize = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(t) => Some(t.deadline_ids.len()),
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
idempotent = true; behavior = deterministic
timeout = duration("500ms")
  }

  connector beta : Process {
mode = job ; command = "cat"
codec = lines
idempotent = true; behavior = deterministic
timeout = duration("100ms")
  }

  operator shout {
input in: struct { text: text }
on in { out = data.text }
  }

  in -> shout
  shout -> alpha -> nothing
  shout -> beta -> nothing
  alpha.err -> nothing
  alpha.exit -> nothing
  beta.err -> nothing
  beta.exit -> nothing
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    execution
        .accept_input(&mut journal, submission("salut"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    assert_eq!(journal.open_calls().len(), 2, "both calls opened");
    // Only beta's budget is due at 100ms.
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 100_000_000)
        .unwrap();
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
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    execution
        .accept_input(&mut journal, submission("salut"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    // The driver answers first; the expiry then loses the race —
    // one closure fact, one completion transaction, no duplicate.
    execution
        .poll_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 5_000_000_000)
        .unwrap();
    assert!(execution.is_stable());
    let closures = journal
        .records()
        .iter()
        .filter(|record| {
            matches!(
                record,
                Record::Egress(EgressRecord::Call(o))
                    if matches!(o.status, CallStatus::Closed(_))
            )
        })
        .count();
    assert_eq!(closures, 1, "close-once");
    assert_eq!(execution.outputs().len(), 1);
    assert!(
        execution
            .envelope_views()
            .iter()
            .all(|event| event.port.name() != "err"),
        "no phantom timeout"
    );
}

#[test]
fn a_late_answer_is_a_reported_observation_not_a_silence() {
    let compilation = compile(
        &[TIMED_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    execution
        .accept_input(&mut journal, submission("salut"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    let call_id = journal.open_calls()[0].call_id;
    // The budget expires first…
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 5_000_000_000)
        .unwrap();
    // …then the world answers a closed call.
    let mut late = ScriptedWorld {
        answers: vec![CallResponse {
            call_id,
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
    let report = execution
        .poll_drivers(&mut journal, &mut CallDriver(&mut late))
        .unwrap();
    assert_eq!(report.settled, 0);
    assert_eq!(report.discarded.len(), 1, "the late answer is visible");
    assert!(report.uncorrelated.is_empty());
}

#[test]
fn a_refused_expiry_stays_scheduled_and_replays() {
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
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    execution
        .accept_input(&mut journal, submission("salut"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    assert!(
        execution
            .advance_to(&mut journal, &mut CallDriver(&mut world), 5_000_000_000)
            .is_err(),
        "the refused closure surfaces"
    );
    // Heal and replay: the deadline was never acknowledged.
    journal.refuse_closes = false;
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 5_000_000_000)
        .unwrap();
    assert!(execution.is_stable());
    assert_eq!(source_exit_cases(&execution), ["TimeoutExpired"]);
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
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    execution
        .accept_input(&mut journal, submission("un"))
        .unwrap();
    execution
        .accept_input(&mut journal, submission("deux"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    assert_eq!(journal.inner.open_calls().len(), 1, "one open, one queued");
    journal.refuse_opens = true;
    assert!(
        execution
            .advance_to(&mut journal, &mut CallDriver(&mut world), 5_000_000_000)
            .is_err()
    );
    journal.refuse_opens = false;
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 5_000_000_000)
        .unwrap();
    assert!(execution.is_stable());
    assert_eq!(
        source_exit_cases(&execution),
        ["TimeoutExpired", "TimeoutExpired"],
        "both expired exactly once"
    );
}

#[test]
fn a_refused_commit_on_a_queued_expiry_loses_nothing() {
    // Case: the queued delivery expires and the timeout
    // transaction's commit is refused — the delivery must stay.
    let compilation = compile(
        &["graph g : App {\n  connector echo : Process {\n    mode = job ; command = \"cat\"\n    codec = lines\n    idempotent = true; behavior = deterministic\n    timeout = duration(\"5s\")\n  }\n  operator wrap {\n    on in { out = . }\n  }\n  in -> wrap -> echo -> nothing\n  echo.err -> nothing\n  echo.exit -> nothing\n}\n"],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = FlakyJournal::healthy(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    execution
        .accept_input(&mut journal, text_submission("salut"))
        .unwrap();
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
        assert!(execution.step(&mut journal).unwrap());
    }
    let facts_before = journal.inner.records().len();
    journal.refuse_commits = true;
    assert!(
        execution
            .advance_to(&mut journal, &mut CallDriver(&mut world), 5_000_000_000)
            .is_err()
    );
    assert!(
        journal.inner.records()[facts_before..]
            .iter()
            .all(|record| !matches!(record, Record::Transaction(_))),
        "the open and closure facts may commit, but the refused transaction may not"
    );
    // Heal and replay: the delivery was never removed, the deadline
    // never acked — exactly one typed terminal fact results.
    journal.refuse_commits = false;
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 5_000_000_000)
        .unwrap();
    execution.run_ready(&mut journal).unwrap();
    assert!(execution.is_stable());
    assert!(
        execution
            .envelope_views()
            .iter()
            .all(|event| event.port.name() != "err")
    );
    assert_eq!(source_exit_cases(&execution), ["TimeoutExpired"]);
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
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    execution
        .accept_input(&mut journal, submission("un"))
        .unwrap();
    execution
        .accept_input(&mut journal, submission("deux"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    // Two requests reached the connector, but only one call is
    // open: the second waits for the mailbox claim to lift.
    assert_eq!(journal.open_calls().len(), 1);
    assert!(execution.has_open_obligations());
    // Settle, then the claim lifts and the second call opens.
    execution
        .poll_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    assert_eq!(journal.open_calls().len(), 1, "the second call is now open");
    execution
        .poll_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();
    assert!(execution.is_stable());
    assert_eq!(execution.outputs().len(), 2);
}

#[test]
fn a_filtered_input_reaches_stability_without_output() {
    let compilation = compile(
        &[M0_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    execution
        .accept_input(&mut journal, submission(""))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();
    assert!(
        execution.is_stable(),
        "the guard filtered; nothing is pending"
    );
    assert!(execution.outputs().is_empty());
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
  output err: union { LoopExpired { last_state: int } }
  output out: int
  Loop.fuel = 3
  Loop.delay = { kind = fixed ; after = duration("2s") }

  operator decide {
    input in: int
    output done: int
    output revised: int
    on in match {
      data > 0 => revised = data - 1
      _ => done = Loop.state.data
    }
  }
  in -> decide
  decide.done -> out
  decide.revised -> Loop.next
}
"#],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a fixed-delay Loop is executable");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut before_restart = Execution::new_at(&compilation.plan, GraphVersion(1), 0);
    before_restart
        .accept_input(&mut journal, int_submission(2))
        .expect("the root is admitted");
    before_restart
        .run_ready(&mut journal)
        .expect("the first traversal schedules, rather than runs, its successor");
    let run = first_loop(&journal);
    let first = loop_state(&journal, run);
    assert_eq!(first.iteration, 1);
    assert_eq!(first.due_at, Some(2_000_000_000));
    assert!(before_restart.outputs().is_empty());
    drop(before_restart);

    let mut recovered = Execution::new_at(&compilation.plan, GraphVersion(1), 1_000_000_000);
    recovered
        .recover_from_journal(&journal)
        .expect("the persisted due-date annotation reschedules the same wakeup");
    recovered
        .recover_from_journal(&journal)
        .expect("a duplicate recovery observation may reschedule but cannot re-enter twice");
    let mut world = EchoWorld::default();
    recovered
        .advance_to(&mut journal, &mut CallDriver(&mut world), 1_999_999_999)
        .expect("advancing short of the date is inert");
    assert_eq!(loop_state(&journal, run).iteration, 1);
    recovered
        .advance_to(&mut journal, &mut CallDriver(&mut world), 2_000_000_000)
        .expect("the exact date admits one traversal");
    let second = loop_state(&journal, run);
    assert_eq!(second.iteration, 2);
    assert_eq!(second.due_at, Some(4_000_000_000));
    recovered
        .advance_to(&mut journal, &mut CallDriver(&mut world), 4_000_000_000)
        .expect("the second exact date reaches the terminal branch");
    assert_eq!(recovered.outputs().len(), 1);
    assert_eq!(
        rastok::runtime::data_value(recovered.outputs()[0].envelope.data()).unwrap(),
        Value::Int(0)
    );
    assert_eq!(loop_state(&journal, run).phase, LoopPhase::Closed);
    let delayed_facts: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction)
                if transaction.loop_protocol().is_some_and(|packet| {
                    packet
                        .facts
                        .iter()
                        .any(|fact| matches!(fact, LoopFact::Advanced { .. }))
                }) =>
            {
                Some(transaction)
            }
            _ => None,
        })
        .collect();
    assert_eq!(delayed_facts.len(), 2);
    assert!(delayed_facts.iter().all(|transaction| {
        transaction.deadline_ids.len() == 1
            && transaction.deadline_ids[0].0 == rastok::journal::DeadlineKind::RecurrenceDelay
    }));
}

#[test]
fn exponential_loop_delay_saturates_without_extending_fuel() {
    let compilation = compile(
        &[r#"
graph countdown : App + Loop {
  input in: int
  output err: union { LoopExpired { last_state: int } }
  output out: int
  Loop.fuel = 4
  Loop.delay = {
    kind = exponential
    first = duration("1s")
    ceiling = duration("3s")
  }

  operator decide {
    input in: int
    output done: int
    output revised: int
    on in match {
      data > 0 => revised = data - 1
      _ => done = Loop.state.data
    }
  }
  in -> decide
  decide.done -> out
  decide.revised -> Loop.next
}
"#],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("an exponential-delay Loop is executable");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    execution
        .accept_input(&mut journal, int_submission(3))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();
    let run = first_loop(&journal);
    assert_eq!(loop_state(&journal, run).due_at, Some(1_000_000_000));
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 1_000_000_000)
        .unwrap();
    assert_eq!(loop_state(&journal, run).due_at, Some(3_000_000_000));
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 3_000_000_000)
        .unwrap();
    assert_eq!(loop_state(&journal, run).due_at, Some(6_000_000_000));
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 6_000_000_000)
        .unwrap();
    assert_eq!(execution.outputs().len(), 1);
    assert_eq!(loop_state(&journal, run).remaining, 1);
}

#[test]
fn instance_closing_fences_a_delayed_loop_from_the_successor_generation() {
    let compilation = compile(
        &[r#"
graph countdown : App + Loop {
  input in: int
  output err: union { LoopExpired { last_state: int } }
  output out: int
  lifetime = { kind = fixed ; after = duration("1s") }
  Loop.fuel = 3
  Loop.delay = { kind = fixed ; after = duration("2s") }

  operator decide {
    input in: int
    output done: int
    output revised: int
    on in match {
      data > 0 => revised = data - 1
      _ => done = Loop.state.data
    }
  }
  in -> decide
  decide.done -> out
  decide.revised -> Loop.next
}
"#],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("Loop delay and instance lifetime are orthogonal");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    execution
        .accept_input(&mut journal, int_submission(1))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();
    let old_run = first_loop(&journal);

    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 1_000_000_000)
        .expect("the lifetime closes before the later Loop wakeup");
    execution
        .accept_input(&mut journal, int_submission(0))
        .expect("the next root reserves a clean successor generation");
    execution
        .run_ready(&mut journal)
        .expect("the successor run reaches its own terminal branch");
    assert_eq!(execution.outputs().len(), 1);
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 2_000_000_000)
        .expect("the stale predecessor wakeup is a harmless observation");
    assert_eq!(
        execution.outputs().len(),
        1,
        "the stale wakeup did not re-enter"
    );
    let old = loop_state(&journal, old_run);
    assert_eq!(old.iteration, 1);
    assert_eq!(old.phase, LoopPhase::Closed);
}

#[test]
fn equal_date_loop_wakeups_keep_their_deadline_creation_order() {
    let compilation = compile(
        &[r#"
graph countdown : App + Loop {
  input in: int
  output err: union { LoopExpired { last_state: int } }
  output out: int
  Loop.fuel = 3
  Loop.delay = { kind = fixed ; after = duration("1s") }

  operator decide {
    input in: int
    output done: int
    output revised: int
    on in match {
      data > 0 => revised = data - 1
      _ => done = Loop.state.data
    }
  }
  in -> decide
  decide.done -> out
  decide.revised -> Loop.next
}
"#],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a delayed Loop is executable");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    execution
        .accept_input(&mut journal, int_submission(1))
        .unwrap();
    execution
        .accept_input(&mut journal, int_submission(2))
        .unwrap();
    execution.run_ready(&mut journal).unwrap();
    let runs: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction) => transaction.loop_protocol(),
            _ => None,
        })
        .flat_map(|packet| &packet.facts)
        .filter_map(|fact| match fact {
            LoopFact::Opened { run, .. } => Some(*run),
            _ => None,
        })
        .collect();
    assert_eq!(runs.len(), 2);
    let before = journal.records().len();
    assert_eq!(
        loop_state(&journal, runs[0]).state.data(),
        &Data::materialized(Value::Int(0)),
        "the first input opened the first run"
    );
    assert_eq!(
        loop_state(&journal, runs[1]).state.data(),
        &Data::materialized(Value::Int(1)),
        "the second input opened the second run"
    );
    execution
        .advance_to(
            &mut journal,
            &mut CallDriver(&mut EchoWorld::default()),
            1_000_000_000,
        )
        .unwrap();
    let transitions: Vec<_> = journal.records()[before..]
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction) => transaction.loop_protocol(),
            _ => None,
        })
        .flat_map(|packet| &packet.facts)
        .filter(|fact| matches!(fact, LoopFact::Advanced { .. } | LoopFact::Closed { .. }))
        .collect();
    assert!(
        matches!(
            transitions.as_slice(),
            [
                LoopFact::Closed { run: first, .. },
                LoopFact::Advanced { run: second, .. }
            ] if *first == runs[0] && *second == runs[1]
        ),
        "opened {runs:?}, transitions {transitions:?}"
    );
    for cut in before..=journal.records().len() {
        let mut retained =
            MemJournal::from_records(GraphVersion(1), None, &journal.records()[..cut]).unwrap();
        let mut restored =
            Execution::new_at(&compilation.plan, GraphVersion(1), execution.logical_time());
        restored.recover_from_journal(&retained).unwrap();
        restored
            .advance_to(
                &mut retained,
                &mut CallDriver(&mut EchoWorld::default()),
                1_000_000_000,
            )
            .unwrap();
        assert_eq!(retained.records(), journal.records(), "recovery at {cut}");
        assert_eq!(restored.outputs(), execution.outputs(), "recovery at {cut}");
    }
}

#[test]
fn loop_delay_starts_only_after_the_call_timeout_exit_decides_next() {
    let compilation = compile(
        &[r#"
graph retry : App + Loop {
  output err: union { LoopExpired { last_state: text } }

  Loop.terminal = [done, err]

  input in: text
  output done: text
  Loop.fuel = 2
  Loop.delay = { kind = fixed ; after = duration("2s") }

  connector shell : Process {
    mode = job
    command = "cat"
    codec = lines
    idempotent = true
    behavior = deterministic
    timeout = duration("1s")
    max_output = "1 MiB"
  }

  in -> shell
  shell.out -> nothing
  shell.err -> nothing
  shell.exit -> match {
    TimeoutExpired -> Loop.again
    _ -> finish
  }
  operator finish {
    output out: text
    on in { out = "completed" }
  }
  finish.out -> done
}
"#],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a bounded Call may decide one Loop traversal");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = ClosingWorld::default();

    execution
        .accept_input(&mut journal, text_submission("again"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    assert_eq!(journal.open_calls().len(), 1);
    let run = first_loop(&journal);

    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 1_000_000_000)
        .expect("the Call budget publishes TimeoutExpired first");
    let after_timeout = loop_state(&journal, run);
    assert_eq!(after_timeout.iteration, 1);
    assert_eq!(after_timeout.due_at, Some(3_000_000_000));
    assert_eq!(after_timeout.phase, LoopPhase::AwaitingAdmission);
    assert!(journal.open_calls().is_empty());

    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 2_999_999_999)
        .expect("the Loop delay has not elapsed");
    assert!(journal.open_calls().is_empty());
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 3_000_000_000)
        .expect("the delayed state re-enters the spatial Call body");
    assert_eq!(journal.open_calls().len(), 1);

    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 4_000_000_000)
        .expect("the second independent Call timeout exhausts fuel");
    assert!(journal.open_calls().is_empty());
    assert_eq!(execution.outputs().len(), 1);
    assert!(matches!(
        loop_state(&journal, run).phase,
        LoopPhase::Expired
    ));

    let call_timeouts = journal
        .records()
        .iter()
        .filter(|record| {
            matches!(
                record,
                Record::Egress(EgressRecord::Call(call))
                    if matches!(
                        call.status,
                        CallStatus::Closed(rastok::journal::Closure::Timeout)
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
fn spatial_loop_recovery_preserves_the_retained_call_and_delay() {
    let compilation = compile(
        &[r#"
graph retry : App + Loop {
  output err: union { LoopExpired { last_state: text } }

  Loop.terminal = [done, err]

  input in: text
  output done: text
  Loop.fuel = 2
  Loop.delay = { kind = fixed ; after = duration("2s") }

  connector shell : Process {
    mode = job
    command = "cat"
    codec = lines
    idempotent = true
    behavior = deterministic
    timeout = duration("1s")
    max_output = "1 MiB"
  }

  in -> shell
  shell.out -> nothing
  shell.err -> nothing
  shell.exit -> match {
    TimeoutExpired -> Loop.again
    _ -> finish
  }
  operator finish {
    output out: text
    on in { out = "completed" }
  }
  finish.out -> done
}
"#],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the spatial Loop is valid");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = ClosingWorld::default();

    execution
        .accept_input(&mut journal, text_submission("again"))
        .unwrap();
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .unwrap();
    execution
        .advance_to(&mut journal, &mut CallDriver(&mut world), 1_000_000_000)
        .expect("the first Call timeout selects delayed recurrence");
    drop(execution);

    let mut recovered = Execution::new_at(&compilation.plan, GraphVersion(1), 2_000_000_000);
    let before = journal.records().to_vec();
    recovered
        .recover_with_drivers(&journal, &mut CallDriver(&mut world))
        .expect("the spatial fold retains the first timeout and delayed command");
    assert_eq!(journal.records(), before);
    recovered
        .advance_to(&mut journal, &mut CallDriver(&mut world), 2_999_999_999)
        .unwrap();
    assert!(journal.open_calls().is_empty());
    recovered
        .advance_to(&mut journal, &mut CallDriver(&mut world), 3_000_000_000)
        .unwrap();
    assert_eq!(journal.open_calls().len(), 1);
    recovered
        .advance_to(&mut journal, &mut CallDriver(&mut world), 4_000_000_000)
        .unwrap();
    assert_eq!(recovered.outputs().len(), 1);
    assert_eq!(
        loop_state(&journal, first_loop(&journal)).phase,
        LoopPhase::Expired
    );
}
