//! Source, field codecs and the categorical package share one decision model.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::boundary::{Draft, IngressEnvelopeProvenance, IngressSubmission, SubmissionSequence};
use rastok::envelope::{Data, Namespace, Value};
use rastok::journal::{MemJournal, Record};
use rastok::runtime::vm::Vm;
use rastok::{
    compiler,
    contract::ContractCatalog,
    datatype::DataType,
    diagnostics::{Bag, SourceId},
    expr, plan,
};
use std::collections::BTreeMap;

fn submission(data: Data) -> IngressSubmission {
    IngressSubmission {
        provenance: IngressEnvelopeProvenance::single(rastok::node::Path::new(["probe"]).unwrap()),
        port: rastok::node::Port::new("in").unwrap(),
        draft: Draft {
            data,
            ext: Namespace::new(),
        },
        sequence: SubmissionSequence::Next,
    }
}

fn checked(source: &str) -> compiler::Hir {
    let registry = ContractCatalog::try_m0().expect("the standard contracts are consistent");
    let source_id = SourceId::new(0);
    let mut bag = Bag::new();
    let tree = compiler::parser::parse(source_id, source, &registry, &mut bag);
    let unit = compiler::Unit {
        files: vec![(source_id, tree)],
    };
    let hir = compiler::check::check(&unit, &registry, &mut bag, None)
        .expect("the source contains one App");
    assert!(!bag.has_errors(), "{bag:?}");
    hir
}

#[test]
fn source_decisions_and_broadcasts_cross_the_categorical_package() {
    let source = r#"
graph app : App {
  input in: bool
  output out: text
  operator choose {
    input in: bool
    output sms: text
    output email: text
    on in match data {
      false => (sms, email) = "no"
      true => match {
        data => (sms, email) = "yes"
        _ => nothing
      }
    }
  }
  in -> choose
  choose.sms -> out
  choose.email -> nothing
}
"#;
    let hir = checked(source);
    let original = &hir.app.nodes[0].handlers[0].dispatch;
    let signature = expr::Signature::new().with("data", DataType::Bool);
    let bytes = plan::fields::encode_dispatch(original, &signature).unwrap();
    assert_eq!(plan::decode(&bytes), Err(plan::DecodeError::BadMagic));
    let decoded = plan::fields::decode_dispatch(&bytes, &signature).unwrap();
    assert_eq!(&decoded, original);
    for (value, expected) in [(false, "no"), (true, "yes")] {
        let bindings = BTreeMap::from([("data".into(), expr::EvalValue::Bool(value))]);
        let selected = rastok::runtime::select_dispatch(&decoded, &bindings).unwrap();
        let [
            plan::DispatchStep::MultiEmit {
                ports,
                values: plan::MultiEmitValues::Broadcast(plan::EmitValue::Data(tree)),
            },
        ] = selected.statements
        else {
            panic!("broadcast leaf")
        };
        assert_eq!(
            ports.iter().map(|port| port.name()).collect::<Vec<_>>(),
            ["sms", "email"]
        );
        assert_eq!(
            expr::eval(tree, &bindings).unwrap(),
            expr::EvalValue::Text(expected.into())
        );
    }
    let (raw, _) = compiler::lower::lower(&hir).unwrap();
    let validated = plan::ValidatedExecutionPlan::try_new(raw).unwrap();
    let hash = compiler::lower::semantic_hash(&hir);
    let package = plan::encode(&validated, &hash);
    let (decoded, decoded_hash) = plan::decode(&package).unwrap();
    assert_eq!(decoded, validated);
    assert_eq!(decoded_hash, hash);
    assert_eq!(plan::FORMAT_VERSION, 20);
}

#[test]
fn where_desugaring_and_closed_arm_permutations_keep_their_field_bytes() {
    let graph = |handler: &str| {
        format!(
            r#"
graph app : App {{
  input in: bool
  output out: bool
  operator filter {{ input in: bool; output out: bool; {handler} }}
  in -> filter -> out
}}
"#
        )
    };
    let signature = expr::Signature::new().with("data", DataType::Bool);
    for (left, right) in [
        (
            "on in where data { out = data }",
            "on in match { data => out = data; _ => nothing }",
        ),
        (
            "on in match data { false => nothing; true => out = data }",
            "on in match data { true => out = data; false => nothing }",
        ),
    ] {
        let first = checked(&graph(left));
        let second = checked(&graph(right));
        let bytes = |hir: &compiler::Hir| {
            plan::fields::encode_dispatch(&hir.app.nodes[0].handlers[0].dispatch, &signature)
                .unwrap()
        };
        assert_eq!(bytes(&first), bytes(&second));
        assert_eq!(
            compiler::lower::semantic_hash(&first),
            compiler::lower::semantic_hash(&second)
        );
    }
}

#[test]
fn source_route_cases_keep_one_physical_edge_per_destination() {
    let hir = checked(
        r#"
graph app : App {
  input in: bool
  output out: bool
  operator relay { input in: bool; output out: bool; on in { out = data } }
  operator sink { input in: bool; on in { nothing } }
  in -> relay
  relay.out -> match {
    true -> { out; sink }
    false -> sink
  }
}

"#,
    );
    let mut rules = hir.app.routes.clone();
    rules.sort_by_key(|rule| {
        plan::fields::encode_routing(std::slice::from_ref(rule), |_| Some(DataType::Bool)).unwrap()
    });
    let bytes = plan::fields::encode_routing(&rules, |_| Some(DataType::Bool)).unwrap();
    let decoded = plan::fields::decode_routing(&bytes, |_| Some(DataType::Bool)).unwrap();
    assert_eq!(decoded, rules);
    let routes: Vec<_> = decoded
        .iter()
        .filter(|rule| rule.source.port.name() == "out")
        .collect();
    assert_eq!(routes.len(), 1);
    let plan::RouteSelection::Cases(arms) = &routes[0].selection else {
        panic!("case routing")
    };
    assert_eq!(arms.len(), 2);
    let is_sink = |endpoint: &plan::Endpoint| matches!(&endpoint.owner, plan::EndpointOwner::Node(path) if path.to_string() == "sink");
    assert!(arms.iter().all(|arm| {
        arm.destinations.iter().any(|destination| {
            matches!(destination, plan::RouteDestination::Port(endpoint) if is_sink(endpoint))
        })
    }));
    assert_eq!(
        hir.app
            .edges
            .iter()
            .filter(|edge| {
                matches!(&edge.to, compiler::hir::EdgeTarget::Port(endpoint) if is_sink(endpoint))
            })
            .count(),
        1,
    );
    let (raw, _) = compiler::lower::lower(&hir).unwrap();
    assert_eq!(raw.routes.len(), 1);
    let validated = plan::ValidatedExecutionPlan::try_new(raw).unwrap();
    let bytes = plan::encode(&validated, &compiler::lower::semantic_hash(&hir));
    assert_eq!(plan::decode(&bytes).unwrap().0, validated);
}

#[test]
fn transparent_graph_paths_cannot_hide_a_duplicate_or_mixed_destination() {
    let graph = |left: &str, wiring: &str| {
        format!(
            r#"
graph app : App {{
  input in: bool
  output out: bool
  graph left {{ input in: bool; output out: bool; {left} }}
  graph right {{ input in: bool; output out: bool; in -> out }}
  {wiring}
}}
"#
        )
    };
    for (source, code, case) in [
        (
            graph(
                "in -> out",
                "in -> { left; right }; { left.out; right.out } -> out",
            ),
            "E-ROUTE-DUPLICATE",
            "all",
        ),
        (
            graph("in -> nothing", "in -> { left; right }; right.out -> out"),
            "E-ROUTE-MIXED",
            "all",
        ),
        (
            graph(
                "in -> out",
                "in -> left; in -> match { true -> right; false -> nothing }; { left.out; right.out } -> out",
            ),
            "E-ROUTE-MIXED",
            "false",
        ),
    ] {
        let errors = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap())
            .expect_err("composition must reject a conflicting disposition before lowering");
        assert!(
            errors.diagnostics().iter().any(|diagnostic| {
                diagnostic.code().as_str() == code
                    && diagnostic
                        .fields()
                        .iter()
                        .any(|(name, value)| *name == "case" && value == case)
            }),
            "{errors:?}"
        );
    }
}

#[test]
fn disjoint_cases_can_share_one_destination_through_different_graph_paths() {
    let source = r#"
graph app : App {
  input in: bool
  output out: bool
  graph left { input in: bool; output out: bool; in -> out }
  graph right { input in: bool; output out: bool; in -> out }
  in -> match { false -> left; true -> right }
  { left.out; right.out } -> out
}
"#;
    let compilation = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    assert_eq!(compilation.plan.edges.len(), 1);
    let decoded = plan::decode(&plan::encode(&compilation.plan, &compilation.hash))
        .unwrap()
        .0;
    for value in [false, true] {
        let mut vm = Vm::boot(&decoded, plan::GraphVersion(1));
        let mut journal = MemJournal::new(plan::GraphVersion(1));
        vm.inject(
            &mut journal,
            submission(Data::materialized(Value::Bool(value))),
        )
        .unwrap();
        vm.run_ready(&mut journal).unwrap();
        assert_eq!(vm.outputs().len(), 1);
        assert_eq!(
            vm.outputs()[0].envelope.data(),
            &Data::materialized(Value::Bool(value))
        );
    }
}

#[test]
fn real_transactions_execute_only_the_selected_case_and_preserve_fanout() {
    let source = r#"
graph routing : App {
  input in: bool
  output yes: bool
  output no: bool
  output audit: bool
  operator relay { input in: bool; output out: bool; on in { out = data } }
  in -> relay
  relay.out -> audit
  relay.out -> match { true -> yes; false -> no }
}

lab routing {
  simulation cases on app {
    scenario select "select before routing" {
      app.in = true
      expect app.yes count == 1
      expect nothing app.no
      expect app.audit count == 1
      app.in = false
      expect app.no count == 1
      expect nothing app.yes
      expect app.audit count == 1
    }
  }
}
"#;
    let compilation = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let bytes = plan::encode(&compilation.plan, &compilation.hash);
    let decoded = plan::decode(&bytes).unwrap().0;
    let runs = rastok::lab::run(&decoded, plan::GraphVersion(1), &compilation.labs[0]);
    assert_eq!(runs.len(), 1);
    assert!(runs[0].report.passed(), "{:?}", runs[0].report);
}

#[test]
fn inferred_expression_types_cross_the_package_with_resolved_port_bounds() {
    let hir = checked(
        r#"
graph app : App {
 input in: int
 output out: int
 operator increment { on in { out = data + 1 } }
 in -> increment -> out
}
"#,
    );
    let (raw, _) = compiler::lower::lower(&hir).unwrap();
    let resolved = plan::ValidatedExecutionPlan::try_from_lowered(raw).unwrap();
    let hash = compiler::lower::semantic_hash(&hir);
    assert_eq!(
        plan::decode(&plan::encode(&resolved, &hash)).unwrap().0,
        resolved
    );
}

#[test]
fn a_closed_arm_preserves_the_tagged_envelope_with_a_narrower_output_type() {
    let source = r#"
type Reply = union { Ready { count: int } Missing {} }
type ReadyReply = union { Ready { count: int } }
graph app : App {
  input in: Reply
  output out: ReadyReply
  operator choose {
    input in: Reply
    output out: ReadyReply
    on in match data {
      Ready => out = .
      Missing => nothing
    }
  }
  in -> choose -> out
}
"#;
    let compilation = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let bytes = plan::encode(&compilation.plan, &compilation.hash);
    assert_eq!(plan::decode(&bytes).unwrap().0, compilation.plan);
}

#[test]
fn operator_loop_commands_cross_v16_and_readmit_the_typed_state() {
    let source = r#"
graph app : App {
  input in: int
  output out: int
  operator countdown : Loop {
    input in: int
    output err: union { LoopExpired { last_state: int } }
    output out: int
    Loop.fuel = 4
    on in match {
      data <= 0 => out = Loop.state.data
      _ => Loop.next = data - 1
    }
  }
  in -> countdown -> out
  countdown.err -> nothing
}
"#;
    let compilation = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let encoded = plan::encode(&compilation.plan, &compilation.hash);
    let decoded = plan::decode(&encoded).unwrap().0;
    let mut vm = Vm::boot(&decoded, plan::GraphVersion(1));
    let mut journal = MemJournal::new(plan::GraphVersion(1));
    vm.inject(&mut journal, submission(Data::materialized(Value::Int(2))))
        .unwrap();
    vm.run_ready(&mut journal).unwrap();
    assert!(vm.is_stable());
    assert_eq!(vm.outputs().len(), 1);
    assert_eq!(
        vm.outputs()[0].envelope.data(),
        &Data::materialized(Value::Int(0))
    );
    let fold = rastok::journal::loop_protocol::Fold::from_records(journal.records()).unwrap();
    let runs: Vec<_> = fold.runs().collect();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].1.iteration, 2);
    assert_eq!(
        runs[0].1.phase,
        rastok::journal::loop_protocol::Phase::Closed
    );
}

#[test]
fn losing_branches_neither_materialize_opaque_data_nor_observe_time() {
    let source = r#"
graph app : App {
  input in: variant
  output out: text
  operator choose {
    input in: variant
    output out: text
    on in match {
      true => out = "selected"
      (data as bool) && now() > timestamp("1970-01-01T00:00:00Z") => out = "unreachable"
      _ => nothing
    }
  }
  in -> choose -> out
}
"#;
    let compilation = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let package = plan::encode(&compilation.plan, &compilation.hash);
    let decoded = plan::decode(&package).unwrap().0;
    let mut vm = Vm::boot_at(&decoded, plan::GraphVersion(1), 17);
    let mut journal = MemJournal::new(plan::GraphVersion(1));
    vm.inject(
        &mut journal,
        submission(Data::encoded(
            rastok::envelope::Encoding::ArrowIpc,
            b"opaque",
        )),
    )
    .unwrap();
    vm.run_ready(&mut journal).unwrap();
    assert_eq!(vm.outputs().len(), 1);
    assert_eq!(
        vm.outputs()[0].envelope.data(),
        &Data::materialized(Value::Text("selected".into()))
    );
    assert!(journal.records().iter().all(|record| !matches!(record, Record::Transaction(transaction) if transaction.logical_time.is_some())));
}

#[test]
fn a_fault_in_the_second_tuple_value_commits_no_partial_emission_or_state() {
    let source = r#"
graph app : App {
  input in: int
  output out: int
  operator divide {
    input in: int
    output a: int
    output b: int
    var count: int = 0
    on in {
      count = count + 1
      (a, b) = (42, 1 / data)
    }
  }
  in -> divide
  { divide.a; divide.b } -> out
}
"#;
    let compilation = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let mut vm = Vm::boot(&compilation.plan, plan::GraphVersion(1));
    let mut journal = MemJournal::new(plan::GraphVersion(1));
    vm.inject(&mut journal, submission(Data::materialized(Value::Int(0))))
        .unwrap();
    assert!(matches!(
        vm.run_ready(&mut journal),
        Err(rastok::runtime::vm::VmFault::Fault(_))
    ));
    assert!(vm.outputs().is_empty());
    assert!(!vm.is_stable(), "the failed input stays pending");
    assert!(journal.records().iter().all(|record| !matches!(record, Record::Transaction(transaction) if !transaction.state_delta.is_empty() || !transaction.emissions.is_empty())));
}
