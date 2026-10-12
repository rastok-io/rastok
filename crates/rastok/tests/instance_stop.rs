//! Explicit lifecycle dispositions never add a port or an edge to the DAG.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::diagnostics::{Bag, SourceId};
use rastok::node;
use rastok::{compiler, contract::ContractCatalog, plan};

fn compile(source: &str) -> Result<compiler::Compilation, rastok::diagnostics::Bag> {
    compiler::compile(&[source], &ContractCatalog::try_m0().unwrap())
}

fn checked(source: &str) -> compiler::Hir {
    let catalog = ContractCatalog::try_m0().unwrap();
    let mut bag = Bag::new();
    let id = SourceId::new(0);
    let tree = compiler::parser::parse(id, source, &catalog, &mut bag);
    let unit = compiler::Unit {
        files: vec![(id, tree)],
    };
    let hir = compiler::check::check(&unit, &catalog, &mut bag, None).unwrap();
    assert!(!bag.has_errors(), "{bag:?}");
    hir
}

fn source(destination: &str) -> String {
    format!(
        r#"graph app : App {{
  input in: int
  operator worker {{
    input in: int
    output end: int
    on in {{ end = data }}
  }}
  in -> worker
  worker.end -> {destination}
}}"#
    )
}

#[test]
fn instance_closure_is_not_a_spatial_edge_or_port() {
    for destination in ["instance.stop", "worker.instance.stop"] {
        let hir = checked(&source(destination));
        assert_eq!(hir.app.edges.len(), 1);
        assert_eq!(hir.app.nodes[0].ports.len(), 2);
        assert!(hir.app.routes.iter().any(|rule| matches!(&rule.selection,
            plan::RouteSelection::All(set) if set.iter().any(|destination| matches!(destination, plan::RouteDestination::InstanceStop(_))))));
        let mut routes = hir.app.routes;
        routes.sort_by_key(plan::RouteRule::canonical_order_key);
        let bytes = plan::fields::encode_routing(&routes, |_| None).unwrap();
        assert_eq!(
            plan::fields::decode_routing(&bytes, |_| None).unwrap(),
            routes
        );
    }
}

#[test]
fn closure_target_is_part_of_semantic_identity() {
    let graph = checked(&source("instance.stop"));
    let child = checked(&source("worker.instance.stop"));
    assert_ne!(
        compiler::lower::semantic_hash(&graph),
        compiler::lower::semantic_hash(&child)
    );
}

#[test]
fn closure_commits_its_request_before_the_exact_target_closes() {
    use rastok::journal::{MemJournal, Record};
    use rastok::node::{Lifecycle, Trigger};
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;
    for destination in ["instance.stop", "worker.instance.stop"] {
        let compiled = compile(&source(destination)).unwrap();
        let (plan, _) = plan::decode(&plan::encode(&compiled.plan, &compiled.hash)).unwrap();
        let mut vm = Vm::boot(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        inject(&mut vm, &mut journal, rastok::envelope::Value::Int(1));
        vm.run_ready(&mut journal).unwrap();
        let produced = journal.records().iter().position(|record| matches!(record, Record::Transaction(record) if record.instance_commands.as_ref().is_some_and(|packet| !packet.produced.is_empty()))).unwrap();
        let closing = journal.records().iter().position(|record| matches!(record, Record::Transaction(record) if record.instance_commands.as_ref().is_some_and(|packet| packet.consumed.is_some()))).unwrap();
        assert!(produced < closing);
        let target = if destination == "instance.stop" {
            ""
        } else {
            "worker"
        };
        assert!(journal.records().iter().any(|record| matches!(record, Record::Transaction(record) if record.owner_path.to_string() == target && record.trigger == Trigger::Lifecycle(Lifecycle::Stop))));
        for prefix in [produced + 1, closing + 1, journal.records().len()] {
            let mut recovered_journal =
                MemJournal::from_records(GraphVersion(1), None, &journal.records()[..prefix])
                    .unwrap();
            let mut recovered = Vm::boot(&plan, GraphVersion(1));
            recovered.recover_from_journal(&recovered_journal).unwrap();
            assert_eq!(recovered_journal.records(), &journal.records()[..prefix]);
            recovered.run_ready(&mut recovered_journal).unwrap();
            assert_eq!(recovered_journal.records(), journal.records());
        }
    }
}

#[test]
fn forged_packages_cannot_omit_the_lexical_scope_proof() {
    let compilation = compile(&source("nothing")).unwrap();
    let commands = checked(&source("instance.stop"));
    for boundary in [false, true] {
        let mut forged = compilation.plan.clone().into_inner();
        if boundary {
            forged.boundary_routes = commands.app.routes.clone();
        } else {
            forged.routes = commands.app.routes.clone();
        }
        forged
            .routes
            .sort_by_key(plan::RouteRule::canonical_order_key);
        forged
            .boundary_routes
            .sort_by_key(plan::RouteRule::canonical_order_key);
        let error = plan::ValidatedExecutionPlan::try_new(forged).unwrap_err();
        assert!(
            error.to_string().contains("instance closure")
                || error.to_string().contains("physical edges"),
            "{error}"
        );
    }
    assert_eq!(
        plan::FORMAT_VERSION,
        20,
        "the current package requires the exact-instance closure proof"
    );
}

fn inject(
    vm: &mut rastok::runtime::vm::Vm<'_>,
    journal: &mut rastok::journal::MemJournal,
    value: rastok::envelope::Value,
) {
    inject_at(vm, journal, "in", value).unwrap();
}

fn inject_at(
    vm: &mut rastok::runtime::vm::Vm<'_>,
    journal: &mut rastok::journal::MemJournal,
    port: &str,
    value: rastok::envelope::Value,
) -> Result<rastok::envelope::Id, rastok::runtime::vm::VmFault> {
    use rastok::boundary::{EnvelopeDraft, IngressProposal, IngressProvenance, IngressSequence};
    vm.inject(
        journal,
        IngressProposal {
            provenance: IngressProvenance::single(
                rastok::node::Path::new(["instance-stop-test"]).unwrap(),
            ),
            draft: EnvelopeDraft {
                port: rastok::node::Port::new(port).unwrap(),
                data: rastok::envelope::Data::materialized(value),
                ext: rastok::envelope::Namespace::new(),
            },
            sequence: IngressSequence::Next,
            session: None,
        },
    )
}

const KEYED_CLOSURE: &str = r#"
type Message = struct { tenant: text, action: text }
graph app : App {
  input in: Message
  output out: int
  output ended: text
  graph conversation {
    input in: Message
    output out: int
    output ended: text
    instance = { key = data.tenant }
    operator decide {
      input in: Message
      output forward: Message
      output child: int
      output all: int
      on in match {
        data.action == "child" => child = 0
        data.action == "all" => all = 0
        _ => forward = data
      }
    }
    operator worker {
      input in: Message
      output out: int
      output ended: text
      var count: int = 0
      var tenant: text = ""
      on in { tenant = data.tenant; count = count + 1; out = count }
      on stop { ended = tenant }
    }
    in -> decide
    decide.forward -> worker
    decide.child -> worker.instance.stop
    decide.all -> instance.stop
    worker.out -> out
    worker.ended -> ended
  }
  in -> conversation
  conversation.out -> out
  conversation.ended -> ended
}
"#;

#[test]
fn child_and_graph_closure_preserve_the_other_tenant_and_survive_every_prefix() {
    use rastok::envelope::{Data, Value};
    use rastok::journal::{MemJournal, Record};
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;
    let compilation = compile(KEYED_CLOSURE).unwrap();
    let plan = &compilation.plan;
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(plan, GraphVersion(1));
    for (tenant, action) in [
        ("alice", "message"),
        ("bob", "message"),
        ("alice", "child"),
        ("alice", "child"),
        ("bob", "message"),
        ("alice", "message"),
        ("alice", "all"),
        ("bob", "message"),
    ] {
        inject(
            &mut vm,
            &mut journal,
            Value::Struct(vec![
                ("tenant".into(), Value::Text(tenant.into())),
                ("action".into(), Value::Text(action.into())),
            ]),
        );
        vm.run_ready(&mut journal).unwrap();
    }
    let counts: Vec<_> = vm
        .outputs()
        .iter()
        .filter(|output| output.port.name() == "out")
        .map(|output| output.envelope.data().clone())
        .collect();
    assert_eq!(
        counts,
        [1, 1, 2, 1, 3].map(|n| Data::materialized(Value::Int(n)))
    );
    let ended: Vec<_> = vm
        .outputs()
        .iter()
        .filter(|output| output.port.name() == "ended")
        .map(|output| output.envelope.data().clone())
        .collect();
    assert_eq!(
        ended,
        vec![Data::materialized(Value::Text("alice".into())); 2]
    );
    let records = journal.records();
    for prefix in 0..=records.len() {
        let retained = MemJournal::from_records(GraphVersion(1), None, &records[..prefix]).unwrap();
        let mut recovered = Vm::boot(plan, GraphVersion(1));
        recovered
            .recover_from_journal(&retained)
            .unwrap_or_else(|error| panic!("prefix {prefix}: {error}"));
        assert_eq!(retained.records(), &records[..prefix]);
    }
    let stop_count = records.iter().filter(|record| matches!(record, Record::Transaction(record) if record.trigger == rastok::node::Trigger::Lifecycle(rastok::node::Lifecycle::Closing))).count();
    assert_eq!(
        stop_count, 2,
        "a repeated child stop adds no second Closing"
    );
}

#[test]
fn an_unopened_child_refuses_without_creating_it_or_committing_the_producer() {
    use rastok::envelope::Value;
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;
    let compilation = compile(KEYED_CLOSURE).unwrap();
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    inject(
        &mut vm,
        &mut journal,
        Value::Struct(vec![
            ("tenant".into(), Value::Text("alice".into())),
            ("action".into(), Value::Text("child".into())),
        ]),
    );
    let error = vm.run_ready(&mut journal).unwrap_err();
    assert!(error.to_string().contains("never opened"), "{error}");
    let records = journal.records().to_vec();
    assert_eq!(vm.step(&mut journal).unwrap_err(), error);
    assert_eq!(journal.records(), records);
    assert!(
        !vm.inspection()
            .instances
            .iter()
            .any(|instance| instance.owner.to_string().ends_with("worker"))
    );
}

#[test]
fn command_saturation_refuses_the_entire_inbound_fanout_before_commit() {
    use rastok::envelope::Value;
    use rastok::journal::{Journal, MemJournal};
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::{Vm, VmFault};
    let compiled = compile(
        r#"graph app : App {
      input in: int
      input close: int
      output out: int
      operator worker { input in: int; on in { nothing } }
      in -> worker
      close -> { worker.instance.stop; out }
    }"#,
    )
    .unwrap();
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compiled.plan, GraphVersion(1));
    inject(&mut vm, &mut journal, Value::Int(0));
    vm.run_ready(&mut journal).unwrap();
    let capacity = vm.inspection().mailbox_capacity;
    for _ in 0..capacity {
        inject_at(&mut vm, &mut journal, "close", Value::Int(1)).unwrap();
    }
    let before = journal.records().len();
    assert_eq!(
        inject_at(&mut vm, &mut journal, "close", Value::Int(1)),
        Err(VmFault::InstanceCommandQueueFull)
    );
    assert_eq!(journal.records().len(), before);
    assert_eq!(vm.outputs().len(), capacity);
    vm.run_ready(&mut journal).unwrap();
    assert!(journal.instance_commands().unwrap().commands().all(
        |(_, disposition)| disposition != rastok::journal::instance_stop::Disposition::Pending
    ));
}

#[test]
fn independent_pending_targets_are_each_closed_once() {
    use rastok::envelope::Value;
    use rastok::journal::{Journal, MemJournal, instance_stop::Disposition};
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;
    let compiled = compile(
        r#"graph app : App {
      input in: int
      input close: int
      operator alice { input in: int; on in { nothing } }
      operator bob { input in: int; on in { nothing } }
      in -> { alice; bob }
      close -> { alice.instance.stop; bob.instance.stop }
    }"#,
    )
    .unwrap();
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compiled.plan, GraphVersion(1));
    inject(&mut vm, &mut journal, Value::Int(0));
    vm.run_ready(&mut journal).unwrap();
    inject_at(&mut vm, &mut journal, "close", Value::Int(1)).unwrap();
    let produced = journal.records().len();
    vm.run_ready(&mut journal).unwrap();
    assert_eq!(
        journal
            .instance_commands()
            .unwrap()
            .commands()
            .filter(|(_, state)| matches!(state, Disposition::Consumed { .. }))
            .count(),
        2
    );
    let mut retained =
        MemJournal::from_records(GraphVersion(1), None, &journal.records()[..produced]).unwrap();
    let mut recovered = Vm::boot(&compiled.plan, GraphVersion(1));
    recovered.recover_from_journal(&retained).unwrap();
    recovered.run_ready(&mut retained).unwrap();
    assert_eq!(retained.records(), journal.records());
}

#[test]
fn an_ordinary_full_destination_prevents_the_closure_request_too() {
    use rastok::envelope::Value;
    use rastok::journal::{Journal, MemJournal};
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;
    let compiled = compile(
        r#"graph app : App {
      input in: int
      input fill: int
      input close: int
      operator worker { input in: int; on in { nothing } }
      operator receiver { input in: int; on in { nothing } }
      in -> worker
      fill -> receiver
      close -> { receiver; worker.instance.stop }
    }"#,
    )
    .unwrap();
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compiled.plan, GraphVersion(1));
    inject(&mut vm, &mut journal, Value::Int(0));
    vm.run_ready(&mut journal).unwrap();
    for _ in 0..vm.inspection().mailbox_capacity {
        inject_at(&mut vm, &mut journal, "fill", Value::Int(1)).unwrap();
    }
    let before = journal.records().len();
    assert!(inject_at(&mut vm, &mut journal, "close", Value::Int(1)).is_err());
    assert_eq!(journal.records().len(), before);
    assert_eq!(journal.instance_commands().unwrap().commands().count(), 0);
}

#[test]
fn recovery_requires_the_command_in_the_validated_plan() {
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;
    let authorized = compile(&source("instance.stop")).unwrap();
    let ordinary = compile(&source("nothing")).unwrap();
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&authorized.plan, GraphVersion(1));
    inject(&mut vm, &mut journal, rastok::envelope::Value::Int(1));
    vm.run_ready(&mut journal).unwrap();
    let mut recovered = Vm::boot(&ordinary.plan, GraphVersion(1));
    let error = recovered.recover_from_journal(&journal).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("instance closure commands are absent"),
        "{error}"
    );
    assert!(
        recovered.inspection().instances.is_empty(),
        "failed recovery is atomic"
    );
}

#[test]
fn recovery_rederives_command_presence_and_target_from_the_plan() {
    use rastok::journal::{MemJournal, Record, instance_stop::Command};
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;
    let compiled = compile(&source("worker.instance.stop")).unwrap();
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compiled.plan, GraphVersion(1));
    inject(&mut vm, &mut journal, rastok::envelope::Value::Int(1));
    vm.run_ready(&mut journal).unwrap();
    let producer = journal.records().iter().position(|record| matches!(record, Record::Transaction(record) if record.instance_commands.as_ref().is_some_and(|packet| !packet.produced.is_empty()))).unwrap();
    for missing in [true, false] {
        let mut records = journal.records()[..=producer].to_vec();
        let Record::Transaction(record) = &mut records[producer] else {
            panic!("producer");
        };
        if missing {
            record.instance_commands = None;
        } else {
            let command = &mut record.instance_commands.as_deref_mut().unwrap().produced[0];
            *command = Command::new(
                command.source.clone(),
                command.source_instance.clone(),
                command.scope.clone(),
                command.scope.clone(),
                command.cause,
            );
        }
        // The command is valid as a lifecycle fact, but contradicts this plan.
        let retained = MemJournal::from_records(GraphVersion(1), None, &records).unwrap();
        let mut recovered = Vm::boot(&compiled.plan, GraphVersion(1));
        let error = recovered.recover_from_journal(&retained).unwrap_err();
        assert!(
            error.to_string().contains("instance commands differ"),
            "{error}"
        );
        assert!(recovered.inspection().instances.is_empty());
    }
}

#[test]
fn file_http_and_process_use_the_same_instance_local_closing_protocol() {
    use rastok::journal::{Journal, MemJournal, Record};
    use rastok::plan::GraphVersion;
    for (contract, settings) in [
        ("File", "mode = read; path = \"input.txt\""),
        (
            "HttpClient",
            "url = \"https://partner.invalid\"; codec = json; idempotent = true; behavior = probabilistic",
        ),
        (
            "Process",
            "command = \"cat\"; codec = jsonl; idempotent = true; behavior = deterministic",
        ),
    ] {
        let source = format!(
            r#"
type Message = struct {{ tenant: text, action: text }}
graph app : App {{
  input in: Message
  output exit: variant
  instance = {{ key = data.tenant }}
  operator decide {{
    input in: Message
    output request: Message
    output end: int
    on in match {{ data.action == "stop" => end = 0; _ => request = data }}
  }}
  connector client : {contract} {{
    {settings}
    timeout = duration("100s")
    closing = {{ drain_for = duration("1s"); settle_for = duration("1s") }}
  }}
  in -> decide
  decide.request -> client
  decide.end -> client.instance.stop
  client.out -> nothing
  {stderr}
  client.exit -> exit
}}
lab app {{
  simulation isolation on app {{
    mock client {{ on in {{ out = nothing }} }}
    scenario two_instances {{
      app.in = {{ tenant = "alice"; action = "call" }}
      app.in = {{ tenant = "bob"; action = "call" }}
      app.in = {{ tenant = "alice"; action = "stop" }}
      advance duration("500ms")
      app.in = {{ tenant = "alice"; action = "stop" }}
      advance duration("500ms")
      expect nothing app.exit
      advance duration("1s")
      expect client.exit count == 1 where data is OutcomeUnknown
      advance duration("98s")
      expect client.exit count == 1 where data is TimeoutExpired
    }}
  }}
}}
"#,
            stderr = if contract == "Process" {
                "client.err -> nothing"
            } else {
                ""
            }
        );
        let compiled = compile(&source).unwrap_or_else(|error| panic!("{contract}: {error:?}"));
        let runs = rastok::lab::run(&compiled.plan, GraphVersion(1), &compiled.labs[0]);
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        assert!(run.report.passed(), "{contract}: {:?}", run.report.failures);
        let journal = MemJournal::from_records(GraphVersion(1), None, &run.records).unwrap();
        let fold = journal.instance_commands().unwrap();
        assert_eq!(fold.commands().count(), 2);
        assert_eq!(
            fold.commands()
                .filter(|(_, disposition)| matches!(
                    disposition,
                    rastok::journal::instance_stop::Disposition::Consumed { .. }
                ))
                .count(),
            1
        );
        let command = fold.commands().next().unwrap().0;
        let calls: Vec<_> = run
            .records
            .iter()
            .filter_map(|record| match record {
                Record::Egress(rastok::journal::EgressRecord::Call(call))
                    if call.status == rastok::journal::CallStatus::Open =>
                {
                    Some(&call.intent)
                }
                _ => None,
            })
            .collect();
        assert_eq!(calls.len(), 2, "repeated stop never starts an extra effect");
        let call_instance = |call: &rastok::boundary::CallOpen| {
            run.records
                .iter()
                .find_map(|record| {
                    let Record::Transaction(record) = record else {
                        return None;
                    };
                    record
                        .routes
                        .iter()
                        .find(|route| route.delivery == call.emitting_id && route.edge == call.edge)
                        .map(|route| &route.target)
                })
                .unwrap()
        };
        assert_ne!(call_instance(calls[0]), call_instance(calls[1]));
        assert_eq!(&command.target, call_instance(calls[0]));
        let captured =
            rastok::recording::Recording::capture(&compiled, &journal, 100_000_000_000).unwrap();
        let decoded = rastok::recording::Recording::decode(&captured.encode().unwrap()).unwrap();
        assert_eq!(decoded.records(), run.records);
        for through in 0..=run.records.len() {
            decoded
                .inspect(through)
                .unwrap_or_else(|error| panic!("{contract} prefix {through}: {error}"));
        }
    }
}

#[test]
fn a_loop_scope_can_be_stopped_without_a_port_or_a_loop_verdict_command() {
    use rastok::journal::{
        MemJournal, Record,
        loop_protocol::{CloseReason, Fact},
    };
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;
    let compiled = compile(
        r#"graph app : App + Loop {
      input in: int
      output err: union { LoopExpired { last_state: int } }
      Loop.fuel = 2
      operator halt { input in: int; output end: int; on in { end = data } }
      in -> halt
      halt.end -> instance.stop
    }"#,
    )
    .unwrap();
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compiled.plan, GraphVersion(1));
    inject(&mut vm, &mut journal, rastok::envelope::Value::Int(1));
    vm.run_ready(&mut journal).unwrap();
    assert!(journal.records().iter().any(|record| matches!(record, Record::Transaction(record) if record.loop_protocol().is_some_and(|packet| packet.facts.iter().any(|fact| matches!(fact, Fact::Close { reason: CloseReason::Stopped, .. }))))));
    for through in 0..=journal.records().len() {
        let mut retained =
            MemJournal::from_records(GraphVersion(1), None, &journal.records()[..through]).unwrap();
        let mut vm = Vm::boot(&compiled.plan, GraphVersion(1));
        vm.recover_from_journal(&retained).unwrap();
        vm.run_ready(&mut retained).unwrap();
        if through > 0 {
            assert_eq!(retained.records(), journal.records());
        }
    }
}

#[test]
fn an_effectful_target_requires_bounded_closing_in_source_and_foreign_plans() {
    let source = r#"graph app : App {
      input in: text
      input close: int
      connector client : Process {
        command = "cat"; codec = lines
        idempotent = true; behavior = deterministic
        closing = { drain_for = duration("1s"); settle_for = duration("1s") }
      }
      in -> client
      client.out -> nothing
      client.err -> nothing
      client.exit -> nothing
      close -> client.instance.stop
    }"#;
    let compiled = compile(source).unwrap();
    let missing = source.replace(
        "closing = { drain_for = duration(\"1s\"); settle_for = duration(\"1s\") }",
        "",
    );
    let errors = compile(&missing).unwrap_err();
    assert!(
        errors
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.message().to_lowercase().contains("closing")),
        "{errors:?}"
    );
    let mut forged = compiled.plan.into_inner();
    for owner in &mut forged.owners {
        owner.instance.closing = None;
    }
    let error = plan::ValidatedExecutionPlan::try_new(forged).unwrap_err();
    assert!(error.to_string().contains("Closing"), "{error}");
}

#[test]
fn an_instance_named_data_port_does_not_become_a_command() {
    compile(
        r#"graph app : App {
        input instance: int
        output out: int
        operator worker {
            input in: int
            output instance: int
            on in { instance = data }
        }
        instance -> worker
        worker.instance -> out
    }"#,
    )
    .unwrap();
}

#[test]
fn an_instance_command_cannot_be_a_source_or_an_intermediate_endpoint() {
    for wiring in [
        "instance.stop -> worker",
        "worker.end -> instance.stop -> worker",
    ] {
        let source = source("nothing").replace("worker.end -> nothing", wiring);
        let errors = compile(&source).unwrap_err();
        assert!(
            errors
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code().as_str() == "E-INSTANCE-COMMAND"),
            "{errors:?}"
        );
    }
}

#[test]
fn closure_target_must_be_the_wiring_graph_or_a_direct_child() {
    for destination in [
        "absent.instance.stop",
        "worker.deep.instance.stop",
        "instance.start",
    ] {
        let errors = compile(&source(destination)).unwrap_err();
        assert!(
            errors.diagnostics().iter().any(|diagnostic| {
                matches!(
                    diagnostic.code().as_str(),
                    "E-INSTANCE-COMMAND" | "E-RESOLVE-UNKNOWN"
                )
            }),
            "{destination}: {errors:?}"
        );
    }
}

#[test]
fn a_nested_closure_retains_the_lexical_graph_not_the_app() {
    for destination in ["instance.stop", "worker.instance.stop"] {
        let inner = source(destination).replace("graph app : App", "graph conversation");
        let hir = checked(&format!(
            "graph app : App {{ input in: int\n{inner}\nin -> conversation }}"
        ));
        let graph = &hir.app.graphs[0];
        let target = graph
            .routes
            .iter()
            .find_map(|rule| {
                let plan::RouteSelection::All(set) = &rule.selection else {
                    return None;
                };
                set.iter().find_map(|destination| {
                    let plan::RouteDestination::InstanceStop(target) = destination else {
                        return None;
                    };
                    Some(target)
                })
            })
            .unwrap();
        assert_eq!(target.scope.to_string(), "conversation");
        assert_eq!(
            target.owner,
            if destination == "instance.stop" {
                plan::EndpointOwner::Graph(graph.path.clone())
            } else {
                plan::EndpointOwner::Node(graph.nodes[0].path.clone())
            }
        );
    }
}

#[test]
fn a_forwarded_input_cannot_guess_the_nested_closure_instance() {
    for key in ["", "instance = { key = data }"] {
        for route in [
            "close -> worker.instance.stop",
            "close -> match { true -> worker.instance.stop; false -> nothing }",
        ] {
            let source = format!(
                r#"graph app : App {{
  input in: bool
  input close: bool
  graph conversation {{
    input in: bool
    input close: bool
    {key}
    operator worker {{ input in: bool; on in {{ nothing }} }}
    in -> worker
    {route}
  }}
  in -> conversation
  close -> conversation.close
}}"#
            );
            let errors = compile(&source).unwrap_err();
            assert!(
                errors.diagnostics().iter().any(|diagnostic| {
                    diagnostic.code().as_str() == "E-INSTANCE-COMMAND"
                        && diagnostic.message().contains("Operator inside")
                }),
                "{errors:?}"
            );
        }
    }
}

#[test]
fn a_forged_boundary_input_cannot_supply_a_nested_closure_context() {
    let inner = source("worker.instance.stop").replace("graph app : App", "graph conversation");
    let compiled = compile(&format!(
        "graph app : App {{ input in: int\n{inner}\nin -> conversation }}"
    ))
    .unwrap();
    let mut forged = compiled.plan.into_inner();
    let command = forged
        .boundary_routes
        .iter_mut()
        .find(|rule| matches!(&rule.selection, plan::RouteSelection::All(set)
            if set.iter().any(|destination| matches!(destination, plan::RouteDestination::InstanceStop(_)))))
        .unwrap();
    command.source = plan::Endpoint {
        owner: plan::EndpointOwner::Graph(node::OwnerPath::new(["conversation"]).unwrap()),
        port: rastok::node::Port::new("in").unwrap(),
    };
    forged
        .boundary_routes
        .sort_by_key(plan::RouteRule::canonical_order_key);
    let error = plan::ValidatedExecutionPlan::try_new(forged).unwrap_err();
    assert!(error.to_string().contains("Operator inside"), "{error}");
}

#[test]
fn closure_dispositions_preserve_duplicate_and_nothing_rules() {
    for (destination, code) in [
        ("{ instance.stop ; instance.stop }", "E-ROUTE-DUPLICATE"),
        ("{ instance.stop ; nothing }", "E-ROUTE-MIXED"),
        ("instance.stop\nworker.end -> nothing", "E-ROUTE-MIXED"),
    ] {
        let errors = compile(&source(destination)).unwrap_err();
        assert!(
            errors
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code().as_str() == code),
            "{errors:?}"
        );
    }
}
