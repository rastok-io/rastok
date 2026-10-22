//! Composition is checked and recovered from exact contexts, never origins.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::compiler;
use rastok::connector::contract::ContractCatalog;
use rastok::connector::exchange::EnvelopeDraft;
use rastok::connector::exchange::IngressProposal;
use rastok::connector::exchange::IngressProvenance;
use rastok::connector::exchange::IngressSequence;
use rastok::envelope::{Data, Namespace, Value};
use rastok::journal::{
    MemJournal,
    loop_protocol::{LoopView, Phase},
};
use rastok::plan::{self, GraphVersion};
use rastok::runtime::execution::Execution;

#[test]
fn boundary_only_graphs_have_manager_verdicts_without_synthetic_executors() {
    for body in [
        "in -> out",
        "in -> Loop.next",
        "in -> Loop.again",
        "in -> Loop.done",
    ] {
        let source = format!(
            r#"
graph app : App {{
  input in: int
  output out: int
  output exhausted: variant
  graph work : Loop {{
    input in: int
    output err: union {{ LoopExpired {{ last_state: int }} }}
    output out: int
    Loop.fuel = 2
    {body}
  }}
  in -> work
  work.out -> out
  work.err -> exhausted
}}
"#
        );
        let compiled =
            compiler::compile(&[source.as_str()], &ContractCatalog::try_m0().unwrap()).unwrap();
        let (plan, _) = plan::decode(&plan::encode(&compiled.plan, &compiled.hash)).unwrap();
        assert!(
            plan.nodes
                .iter()
                .all(|node| matches!(node.body, plan::NodeKind::Graph(_))),
            "a boundary-only Graph needs no synthetic Operator"
        );
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut execution = Execution::new(&plan, GraphVersion(1));
        execution
            .accept_input(
                &mut journal,
                IngressProposal {
                    provenance: IngressProvenance::single(
                        rastok::node::Path::new(["test"]).unwrap(),
                    ),
                    draft: EnvelopeDraft {
                        port: rastok::node::Port::new("in").unwrap(),
                        data: Data::materialized(Value::Int(7)),
                        ext: Namespace::new(),
                    },
                    sequence: IngressSequence::Next,
                    session: None,
                },
            )
            .unwrap();
        for _ in 0..30 {
            execution.recover_from_journal(&journal).unwrap();
            if !execution.step(&mut journal).unwrap() {
                break;
            }
        }
        assert!(execution.is_stable());
        let fold =
            rastok::journal::loop_protocol::LoopView::from_records(journal.records()).unwrap();
        let run = fold.runs().next().unwrap().1;
        assert!(matches!(
            run.phase,
            rastok::journal::loop_protocol::Phase::Closed
                | rastok::journal::loop_protocol::Phase::Expired
        ));
        assert_eq!(
            execution.outputs().len(),
            usize::from(body != "in -> Loop.done")
        );
    }
}

fn countdown(name: &str) -> String {
    format!(
        r#"
  graph {name} : Loop {{
    input in: int
    output err: union {{ LoopExpired {{ last_state: int }} }}
    output out: int
    Loop.fuel = 3
    operator choose {{
      input in: int
      output accepted: int
      output revised: int
      on in match {{ data <= 0 => accepted = data; _ => revised = data - 1 }}
    }}
    in -> choose
    choose.accepted -> out
    choose.revised -> Loop.next
  }}
"#
    )
}

#[test]
fn a_graph_join_keeps_runs_separate_and_recovers_the_exact_tuple() {
    let source = r#"
graph app : App {
  input in: int
  output out: int
  graph cycle : Loop {
    input in: int
    output err: union { LoopExpired { last_state: int } }
    output out: int
    Loop.fuel = 2
    operator pair {
      input left: int
      input right: int
      output out: int
      on (left, right) { out = left.data + right.data + Loop.state.data }
    }
    in -> { pair.left; pair.right }
    pair.out -> out
  }
  in -> cycle -> out
  cycle.err -> nothing
}

"#;
    let compiled = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let (plan, _) = plan::decode(&plan::encode(&compiled.plan, &compiled.hash)).unwrap();
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&plan, GraphVersion(1));
    for n in [1, 10] {
        execution
            .accept_input(
                &mut journal,
                IngressProposal {
                    provenance: IngressProvenance::single(
                        rastok::node::Path::new(["test"]).unwrap(),
                    ),
                    draft: EnvelopeDraft {
                        port: rastok::node::Port::new("in").unwrap(),
                        data: Data::materialized(Value::Int(n)),
                        ext: Namespace::new(),
                    },
                    sequence: IngressSequence::Next,
                    session: None,
                },
            )
            .unwrap();
    }
    for _ in 0..100 {
        let mut restored = Execution::new(&plan, GraphVersion(1));
        restored.recover_from_journal(&journal).unwrap();
        assert_eq!(restored.inspection(), execution.inspection());
        execution = restored;
        if execution.is_stable() {
            break;
        }
        assert!(execution.step(&mut journal).unwrap());
    }
    assert!(execution.is_stable());
    let mut values: Vec<_> = execution
        .outputs()
        .iter()
        .map(|output| {
            let Data::Materialized(Value::Int(n)) = output.envelope.data() else {
                panic!("expected int")
            };
            *n
        })
        .collect();
    values.sort();
    assert_eq!(values, [3, 30]);
    assert_eq!(
        LoopView::from_records(journal.records())
            .unwrap()
            .runs()
            .count(),
        2
    );
}

#[test]
fn nested_graph_runs_keep_the_parent_until_child_completion_or_expiry() {
    for inner in [
        countdown("inner"),
        String::from(
            r#"
operator inner : Loop {
  input in: int
  output err: union { LoopExpired { last_state: int } }
  output out: int
  Loop.fuel = 3
  on in match { data <= 0 => out = data; _ => Loop.next = data - 1 }
}
"#,
        ),
    ]
    .into_iter()
    .flat_map(|source| {
        let delayed = source.replace(
            "Loop.fuel = 3",
            "Loop.fuel = 3\nLoop.delay = { kind = fixed; after = duration(\"1s\") }",
        );
        [source, delayed]
    }) {
        for n in [1, 4] {
            let source = format!(
                r#"
graph app : App {{
  input in: int
  output out: int
  output exhausted: union {{ LoopExpired {{ last_state: int }} }}
  graph outer : Loop {{
    input in: int
    output err: union {{ LoopExpired {{ last_state: int }} }}
    output out: int
    output exhausted: union {{ LoopExpired {{ last_state: int }} }}
    Loop.fuel = 2
    Loop.terminal = [out, exhausted, err]
    {}
    in -> inner -> out
    inner.err -> exhausted
  }}
  in -> outer -> out
  outer.exhausted -> exhausted
  outer.err -> nothing
}}
"#,
                inner
            );
            let compiled =
                compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
            let (plan, _) = plan::decode(&plan::encode(&compiled.plan, &compiled.hash)).unwrap();
            let mut journal = MemJournal::new(GraphVersion(1));
            let mut execution = Execution::new(&plan, GraphVersion(1));
            execution
                .accept_input(
                    &mut journal,
                    IngressProposal {
                        provenance: IngressProvenance::single(
                            rastok::node::Path::new(["nested"]).unwrap(),
                        ),
                        draft: EnvelopeDraft {
                            port: rastok::node::Port::new("in").unwrap(),
                            data: Data::materialized(Value::Int(n)),
                            ext: Namespace::new(),
                        },
                        sequence: IngressSequence::Next,
                        session: None,
                    },
                )
                .unwrap();
            for _ in 0..100 {
                let mut restored =
                    Execution::new_at(&plan, GraphVersion(1), execution.logical_time());
                restored.recover_from_journal(&journal).unwrap();
                assert_eq!(restored.inspection(), execution.inspection());
                execution = restored;
                if execution.is_stable() {
                    if let Some(due) = LoopView::from_records(journal.records())
                        .unwrap()
                        .runs()
                        .filter_map(|(_, run)| {
                            (run.phase == Phase::AwaitingAdmission)
                                .then_some(run.due_at)
                                .flatten()
                        })
                        .min()
                    {
                        let mut drivers = rastok::connector::drivers::RuntimeDrivers::prepare(
                            &plan,
                            &rastok::connector::drivers::DriverRegistry::default(),
                        )
                        .unwrap();
                        execution
                            .advance_to(&mut journal, &mut drivers, due)
                            .unwrap();
                        continue;
                    }
                    break;
                }
                assert!(execution.step(&mut journal).unwrap());
            }
            assert!(execution.is_stable());
            assert_eq!(execution.outputs().len(), 1);
            assert_eq!(
                execution.outputs()[0].port.name(),
                if n == 1 { "out" } else { "exhausted" }
            );
            let fold = LoopView::from_records(journal.records()).unwrap();
            assert_eq!(fold.runs().count(), 2);
            assert!(
                fold.runs()
                    .all(|(_, run)| matches!(run.phase, Phase::Closed | Phase::Expired))
            );
        }
    }
}

#[test]
fn a_graph_measure_is_rederived_from_its_spatial_command_producers() {
    let source = format!(
        r#"
graph app : App {{
  input in: int
  output out: int
  {}
  in -> cycle -> out
}}
"#,
        countdown("cycle").replace("Loop.fuel = 3", "Loop.measure = Loop.state.data")
    );
    let compiled = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let (plan, _) = plan::decode(&plan::encode(&compiled.plan, &compiled.hash)).unwrap();
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&plan, GraphVersion(1));
    execution
        .accept_input(
            &mut journal,
            IngressProposal {
                provenance: IngressProvenance::single(rastok::node::Path::new(["test"]).unwrap()),
                draft: EnvelopeDraft {
                    port: rastok::node::Port::new("in").unwrap(),
                    data: Data::materialized(Value::Int(5)),
                    ext: Namespace::new(),
                },
                sequence: IngressSequence::Next,
                session: None,
            },
        )
        .unwrap();
    for _ in 0..100 {
        let mut restored = Execution::new(&plan, GraphVersion(1));
        restored.recover_from_journal(&journal).unwrap();
        assert_eq!(restored.inspection(), execution.inspection());
        execution = restored;
        if execution.is_stable() {
            break;
        }
        assert!(execution.step(&mut journal).unwrap());
    }
    assert!(execution.is_stable());
    let fold = LoopView::from_records(journal.records()).unwrap();
    let (_, run) = fold.runs().next().unwrap();
    assert_eq!((run.phase, run.iteration), (Phase::Closed, 5));
    assert_eq!(execution.outputs().len(), 1);
    let bad = source.replace("data - 1", "data + 1");
    assert!(compiler::compile(&[&bad], &ContractCatalog::try_m0().unwrap()).is_err());
    let mut forged = plan.plan().clone();
    for owner in &mut forged.nodes {
        for extension in &mut owner.extensions {
            if let plan::ExtensionSpec::Loop(spec) = extension {
                spec.proof.as_mut().unwrap().decreases.push(99);
            }
        }
    }
    assert!(plan::ValidatedExecutionPlan::try_new(forged).is_err());
}

#[test]
fn adjacent_graph_loops_do_not_share_application_contexts() {
    for wiring in [
        "in -> first -> second -> out",
        "in -> { first; second }\nfirst.out -> out\nsecond.out -> out",
    ] {
        let source = format!(
            r#"
graph app : App {{
  input in: int
  output out: int
  output exhausted: union {{ LoopExpired {{ last_state: int }} }}
  {} {}
  {wiring}
  {{ first.err; second.err }} -> exhausted
}}
"#,
            countdown("first"),
            countdown("second")
        );
        let compiled = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
        let (plan, _) = plan::decode(&plan::encode(&compiled.plan, &compiled.hash)).unwrap();
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut execution = Execution::new(&plan, GraphVersion(1));
        execution
            .accept_input(
                &mut journal,
                IngressProposal {
                    provenance: IngressProvenance::single(
                        rastok::node::Path::new(["test"]).unwrap(),
                    ),
                    draft: EnvelopeDraft {
                        port: rastok::node::Port::new("in").unwrap(),
                        data: Data::materialized(Value::Int(1)),
                        ext: Namespace::new(),
                    },
                    sequence: IngressSequence::Next,
                    session: None,
                },
            )
            .unwrap();
        for _ in 0..100 {
            let mut restored = Execution::new(&plan, GraphVersion(1));
            restored.recover_from_journal(&journal).unwrap();
            assert_eq!(restored.inspection(), execution.inspection());
            execution = restored;
            if execution.is_stable() {
                break;
            }
            assert!(execution.step(&mut journal).unwrap());
        }
        assert!(execution.is_stable());
        let fold = LoopView::from_records(journal.records()).unwrap();
        assert_eq!(fold.runs().count(), 2);
        assert!(fold.runs().all(|(_, run)| run.phase == Phase::Closed));
        assert_eq!(
            execution.outputs().len(),
            if wiring.contains('{') { 2 } else { 1 }
        );
    }
}
