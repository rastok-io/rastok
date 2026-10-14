//! Command Loop regressions cross the real source, package reader and Worker.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::boundary::{EnvelopeDraft, IngressProposal, IngressProvenance, IngressSequence};
use rastok::compiler;
use rastok::connectors::{DriverRegistry, RuntimeBindings};
use rastok::contract::ContractCatalog;
use rastok::envelope::{Data, Namespace, Value};
use rastok::journal::{
    Journal, MemJournal, Record,
    loop_protocol::{Error as LoopError, Fact, LoopView, Phase},
};
use rastok::node;
use rastok::plan::{self, GraphVersion};
use rastok::runtime::execution::Execution;

const GRAPH_LOOP: &str = r#"
graph app : App {
  input in: int
  output out: int
  output expired: union { LoopExpired { last_state: int } }
  graph cycle : Loop {
    input in: int
    output err: union { LoopExpired { last_state: int } }
    output out: int
    Loop.fuel = 3
    operator choose {
      input in: int
      output revised: int
      output accepted: int
      on in match {
        data > 0 => revised = data - 1
        _ => accepted = data
      }
    }
    operator audit {
      input in: int
      on in { nothing }
    }
    in -> { choose; audit }
    choose.revised -> Loop.next
    choose.accepted -> out
  }
  in -> cycle
  cycle.out -> out
  cycle.err -> expired
}
"#;

#[test]
fn the_app_graph_can_loop_and_publish_its_own_expiry() {
    let source = r#"
graph app : App + Loop {
  input in: int
  output err: union { LoopExpired { last_state: int } }
  output out: int
  Loop.fuel = 2
  operator choose {
    input in: int
    output revised: int
    output accepted: int
    on in match { data > 0 => revised = data - 1; _ => accepted = data }
  }
  in -> choose
  choose.revised -> Loop.next
  choose.accepted -> out
}
"#;
    let compilation = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let (plan, _) = plan::decode(&plan::encode(&compilation.plan, &compilation.hash)).unwrap();
    for (input, port) in [(1, "out"), (2, "err")] {
        let mut vm = Execution::new(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        inject(&mut vm, &mut journal, input);
        while !vm.is_stable() {
            assert!(vm.has_ready_work(), "the retained Graph admission is ready");
            assert!(vm.step(&mut journal).unwrap());
        }
        assert_eq!(vm.outputs().len(), 1, "input {input}");
        assert_eq!(vm.outputs()[0].port.name(), port);
        let mut restored = Execution::new(&plan, GraphVersion(1));
        restored.recover_from_journal(&journal).unwrap();
        assert_eq!(restored.outputs(), vm.outputs());
        assert!(restored.is_stable());
    }
}

#[test]
fn two_graph_loop_admissions_with_one_origin_open_distinct_runs() {
    let source = GRAPH_LOOP.replace(
        "  in -> cycle",
        r#"
  operator duplicate {
    input in: int
    output first: int
    output second: int
    on in { (first, second) = data }
  }
  in -> duplicate
  { duplicate.first; duplicate.second } -> cycle
"#,
    );
    let compilation = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let mut vm = Execution::new(&compilation.plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 1);
    vm.run_ready(&mut journal).unwrap();
    assert_eq!(vm.outputs().len(), 2);
    assert_eq!(
        vm.outputs()[0].envelope.meta().sys().origin_id(),
        vm.outputs()[1].envelope.meta().sys().origin_id()
    );
    let fold = LoopView::from_records(journal.records()).unwrap();
    assert_eq!(fold.runs().count(), 2);
    assert!(fold.runs().all(|(_, run)| run.phase == Phase::Closed));
    let mut restored = Execution::new(&compilation.plan, GraphVersion(1));
    restored.recover_from_journal(&journal).unwrap();
    assert_eq!(restored.outputs(), vm.outputs());
}

#[test]
fn a_forged_graph_next_cannot_change_the_entry_type() {
    let source = GRAPH_LOOP
        .replace("output revised: int", "output revised: text")
        .replace("revised = data - 1", "revised = \"retry\"")
        .replace(
            "choose.revised -> Loop.next",
            "choose.revised -> Loop.again",
        );
    let compilation = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let mut forged = compilation.plan.plan().clone();
    for route in forged.routes.iter_mut().chain(&mut forged.boundary_routes) {
        if let plan::RouteSelection::All(destinations) = &mut route.selection {
            for destination in destinations {
                if let plan::RouteDestination::LoopCommand(command) = destination {
                    command.kind = plan::LoopCommandKind::Next;
                }
            }
        }
    }
    let error = plan::ValidatedExecutionPlan::try_new(forged).unwrap_err();
    assert!(error.to_string().contains("entry type"), "{error}");
}

#[test]
fn graph_admissions_consume_each_edge_in_delivery_order() {
    let compilation =
        compiler::compile(&[GRAPH_LOOP], &ContractCatalog::try_m0().unwrap()).unwrap();
    let mut vm = Execution::new(&compilation.plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    for _ in 0..12 {
        inject(&mut vm, &mut journal, 0);
    }
    vm.run_ready(&mut journal).unwrap();
    let consumed: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(record)
                if record.loop_protocol().is_some_and(|packet| {
                    packet
                        .facts
                        .iter()
                        .any(|fact| matches!(fact, Fact::Admit { iteration: 0, .. }))
                }) =>
            {
                Some(record.consumed[0].seq)
            }
            _ => None,
        })
        .collect();
    assert_eq!(consumed, (1..=12).collect::<Vec<_>>());
    assert_eq!(vm.outputs().len(), 12);
}

#[test]
fn a_graph_loop_retains_one_boundary_admission_before_its_internal_fanout() {
    let compilation =
        compiler::compile(&[GRAPH_LOOP], &ContractCatalog::try_m0().unwrap()).unwrap();
    let (plan, _) = plan::decode(&plan::encode(&compilation.plan, &compilation.hash)).unwrap();
    let mut vm = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 2);
    vm.run_ready(&mut journal).unwrap();
    let fold = LoopView::from_records(journal.records()).unwrap();
    assert_eq!(
        fold.runs().count(),
        1,
        "one Graph input, not one run per leaf"
    );
    let (_, run) = fold.runs().next().unwrap();
    assert_eq!((run.iteration, run.phase), (2, Phase::Closed));
    assert_eq!(vm.outputs().len(), 1);
    assert_eq!(
        vm.outputs()[0].envelope.data(),
        &Data::materialized(Value::Int(0))
    );
    assert!(vm.is_stable());
    let expected = journal.records().to_vec();
    for cut in 1..=expected.len() {
        let mut running = Execution::new(&plan, GraphVersion(1));
        let mut prefix = MemJournal::new(GraphVersion(1));
        inject(&mut running, &mut prefix, 2);
        while prefix.records().len() < cut {
            assert!(running.step(&mut prefix).unwrap());
        }
        let mut restored = Execution::new(&plan, GraphVersion(1));
        restored
            .recover_from_journal(&prefix)
            .unwrap_or_else(|error| panic!("cut {cut}: {error}"));
        assert_eq!(prefix.records().len(), cut);
        restored.run_ready(&mut prefix).unwrap();
        assert_eq!(prefix.records(), expected, "cut {cut}");
        assert_eq!(restored.outputs(), vm.outputs(), "cut {cut}");
    }
}

#[test]
fn graph_loop_commands_wait_for_siblings_and_keep_concurrent_runs_separate() {
    // The decision sorts before the audit, so a command exists while work remains.
    let source = GRAPH_LOOP
        .replace("choose", "a_decide")
        .replace("audit", "z_audit");
    let compilation = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let mut vm = Execution::new(&compilation.plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 2);
    inject(&mut vm, &mut journal, 1);
    vm.run_ready(&mut journal).unwrap();
    assert_eq!(vm.outputs().len(), 2);
    let fold = LoopView::from_records(journal.records()).unwrap();
    assert_eq!(fold.runs().count(), 2);
    assert!(fold.runs().all(|(_, state)| state.phase == Phase::Closed));
    // Every manager consumption follows exactly one audit for that iteration.
    let mut audits = std::collections::BTreeMap::new();
    let mut saw_pending_sibling = false;
    for record in journal.records() {
        let Record::Transaction(record) = record else {
            continue;
        };
        let Some(packet) = record.loop_protocol() else {
            continue;
        };
        if record.owner_path.to_string() == "cycle.z_audit" {
            for run in &packet.context {
                *audits.entry(*run).or_insert(0_usize) += 1;
            }
        }
        for command in &packet.produced {
            if audits.get(&command.run).copied().unwrap_or(0) == command.iteration as usize {
                saw_pending_sibling = true;
            }
        }
        for fact in &packet.facts {
            let iteration = match fact {
                Fact::Advance { from_iteration, .. } => Some(*from_iteration),
                Fact::Close { iteration, .. } => Some(*iteration),
                _ => None,
            };
            if let Some(iteration) = iteration {
                assert_eq!(
                    audits.get(&fact.run()).copied(),
                    Some(iteration as usize + 1)
                );
            }
        }
    }
    assert!(
        saw_pending_sibling,
        "a command is produced before its sibling audit commits"
    );
}

#[test]
fn graph_loop_state_belongs_to_the_consumed_application() {
    let source = GRAPH_LOOP
        .replace("data > 0", "Loop.state.data > 0")
        .replace("revised = data - 1", "revised = Loop.state.data - 1")
        .replace("accepted = data", "accepted = Loop.state.data")
        .replace(
            "    in -> { choose; audit }",
            r#"
    operator rewrite {
      input in: int
      output out: int
      on in { out = 99 }
    }
    in -> rewrite
    rewrite.out -> { choose; audit }
"#,
        );
    let compilation = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let (plan, _) = plan::decode(&plan::encode(&compilation.plan, &compilation.hash)).unwrap();
    let mut vm = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 2);
    vm.run_ready(&mut journal).unwrap();
    assert_eq!(vm.outputs().len(), 1);
    assert_eq!(
        vm.outputs()[0].envelope.data(),
        &Data::materialized(Value::Int(0))
    );
}

#[test]
fn graph_again_retains_state_and_explicit_done_emits_no_domain_output() {
    for (input, expected) in [(1, Phase::Expired), (0, Phase::Closed)] {
        let source = GRAPH_LOOP
            .replace(
                "choose.revised -> Loop.next",
                "choose.revised -> Loop.again",
            )
            .replace("choose.accepted -> out", "choose.accepted -> Loop.done");
        let compilation =
            compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
        let mut vm = Execution::new(&compilation.plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        inject(&mut vm, &mut journal, input);
        vm.run_ready(&mut journal).unwrap();
        let fold = LoopView::from_records(journal.records()).unwrap();
        let (_, run) = fold.runs().next().unwrap();
        assert_eq!(run.phase, expected);
        assert_eq!(run.state.data(), &Data::materialized(Value::Int(input)));
        assert_eq!(vm.outputs().len(), usize::from(expected == Phase::Expired));
        let mut restored = Execution::new(&compilation.plan, GraphVersion(1));
        restored.recover_from_journal(&journal).unwrap();
        assert_eq!(restored.outputs(), vm.outputs());
    }
}

#[test]
fn graph_manager_admissions_reserve_capacity_before_ingress_commits() {
    let compilation =
        compiler::compile(&[GRAPH_LOOP], &ContractCatalog::try_m0().unwrap()).unwrap();
    let mut vm = Execution::new(&compilation.plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    for _ in 0..1024 {
        inject(&mut vm, &mut journal, 0);
    }
    let count = journal.records().len();
    let error = vm
        .accept_input(
            &mut journal,
            IngressProposal {
                provenance: IngressProvenance::single(rastok::node::Path::new(["test"]).unwrap()),
                draft: EnvelopeDraft {
                    port: rastok::node::Port::new("in").unwrap(),
                    data: Data::materialized(Value::Int(0)),
                    ext: Namespace::new(),
                },
                sequence: IngressSequence::Next,
                session: None,
            },
        )
        .unwrap_err();
    assert!(matches!(
        error,
        rastok::runtime::execution::ExecutionFault::RecurrenceQueueFull { .. }
    ));
    assert_eq!(journal.records().len(), count);
}

#[test]
fn graph_loop_delay_and_expiry_recover_without_restarting_the_wait() {
    let source = GRAPH_LOOP.replace(
        "Loop.fuel = 3",
        "Loop.fuel = 2\n Loop.delay = { kind = fixed; after = duration(\"1s\") }",
    );
    let compilation = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let mut vm = Execution::new(&compilation.plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 2);
    vm.run_ready(&mut journal).unwrap();
    assert!(vm.outputs().is_empty());
    let mut restored = Execution::new_at(&compilation.plan, GraphVersion(1), 500_000_000);
    restored.recover_from_journal(&journal).unwrap();
    let mut world =
        RuntimeBindings::try_new(&compilation.plan, &DriverRegistry::default()).unwrap();
    restored
        .advance_to(&mut journal, &mut world, 1_000_000_000)
        .unwrap();
    assert_eq!(restored.outputs().len(), 1);
    assert_eq!(restored.outputs()[0].port.name(), "expired");
    let fold = LoopView::from_records(journal.records()).unwrap();
    assert!(
        fold.runs()
            .all(|(_, state)| state.phase == Phase::Expired && state.iteration == 1)
    );
    let last = journal.records().to_vec();
    let mut closed = Execution::new_at(&compilation.plan, GraphVersion(1), 2_000_000_000);
    closed.recover_from_journal(&journal).unwrap();
    closed
        .advance_to(&mut journal, &mut world, 3_000_000_000)
        .unwrap();
    assert_eq!(journal.records(), last);
    assert_eq!(closed.outputs(), restored.outputs());
}

#[test]
fn closing_a_graph_loop_invalidates_its_pending_reentry() {
    let source = GRAPH_LOOP.replace("Loop.fuel = 3", "Loop.fuel = 3\n lifetime = { kind = fixed; after = duration(\"2s\") }\n Loop.delay = { kind = fixed; after = duration(\"5s\") }");
    let compilation = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let mut vm = Execution::new(&compilation.plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world =
        RuntimeBindings::try_new(&compilation.plan, &DriverRegistry::default()).unwrap();
    inject(&mut vm, &mut journal, 2);
    vm.run_ready(&mut journal).unwrap();
    vm.advance_to(&mut journal, &mut world, 2_000_000_000)
        .unwrap();
    assert!(vm.outputs().is_empty());
    let records = journal.records().to_vec();
    assert!(
        LoopView::from_records(&records)
            .unwrap()
            .runs()
            .all(|(_, state)| state.phase == Phase::Closed)
    );
    let mut restored = Execution::new_at(&compilation.plan, GraphVersion(1), 2_000_000_000);
    restored.recover_from_journal(&journal).unwrap();
    restored
        .advance_to(&mut journal, &mut world, 6_000_000_000)
        .unwrap();
    assert_eq!(journal.records(), records);
    assert!(restored.is_stable());
}

#[test]
fn logical_advance_drains_graph_admissions_before_a_future_closure() {
    let source = GRAPH_LOOP.replace(
        "Loop.fuel = 3",
        "Loop.fuel = 3\n lifetime = { kind = fixed; after = duration(\"1s\") }",
    );
    let compilation = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let mut vm = Execution::new(&compilation.plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    for _ in 0..12 {
        inject(&mut vm, &mut journal, 1);
    }
    let mut world =
        RuntimeBindings::try_new(&compilation.plan, &DriverRegistry::default()).unwrap();
    vm.advance_to(&mut journal, &mut world, 2_000_000_000)
        .unwrap();
    // `advance` drains the current instant before moving to a future deadline.
    assert_eq!(vm.outputs().len(), 12);
    assert_eq!(
        LoopView::from_records(journal.records())
            .unwrap()
            .runs()
            .count(),
        12
    );
    assert!(vm.is_stable());
    let mut restored = Execution::new_at(&compilation.plan, GraphVersion(1), 2_000_000_000);
    restored.recover_from_journal(&journal).unwrap();
    assert!(restored.is_stable());
    inject(&mut restored, &mut journal, 0);
    restored.run_ready(&mut journal).unwrap();
    assert_eq!(
        restored.outputs().len(),
        13,
        "the successor can admit new work"
    );
}

#[test]
fn the_countdown_example_executes_from_the_v16_package() {
    let compilation = compiler::compile(
        &[include_str!("../../../examples/countdown.rkg")],
        &ContractCatalog::try_m0().unwrap(),
    )
    .unwrap();
    let (plan, _) = plan::decode(&plan::encode(&compilation.plan, &compilation.hash)).unwrap();
    let runs = rastok::lab::run(&plan, GraphVersion(1), &compilation.labs[0]);
    assert_eq!(runs.len(), 2);
    for run in runs {
        assert!(run.report.passed(), "{:?}", run.report.failures);
    }
}

#[test]
fn an_outbound_call_retries_only_after_its_terminal_then_exhausts_fuel() {
    let source = r#"
graph app : App {
  input in: text
  output expired: union { LoopExpired { last_state: text } }
  connector worker : Process + Loop {
    command = "cat"
    codec = lines
    idempotent = true
    behavior = probabilistic
    timeout = duration("1s")
    Loop.fuel = 2
    Loop.delay = { kind = fixed; after = duration("1s") }
  }
  in -> worker
  worker.out -> nothing
  worker.err -> nothing
  worker.exit -> match {
    LoopExpired -> expired
    TimeoutExpired -> worker.Loop.again
    _ -> nothing
  }
}
lab app {
  simulation calls on app {
    mock worker { on in { out = nothing } }
    scenario bounded {
      app.in = "same request"
      expect nothing app.expired
      advance duration("1s")
      expect nothing app.expired
      advance duration("1s")
      expect nothing app.expired
      advance duration("1s")
      expect app.expired count == 1
    }
  }
}
"#;
    let compilation = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let bytes = plan::encode(&compilation.plan, &compilation.hash);
    let (decoded, _) = plan::decode(&bytes).unwrap();
    let runs = rastok::lab::run(&decoded, GraphVersion(1), &compilation.labs[0]);
    assert_eq!(runs.len(), 1);
    assert!(
        runs[0].report.passed(),
        "{:?}; records: {:#?}",
        runs[0].report.failures,
        runs[0].records
    );
    let fold = LoopView::from_records(&runs[0].records).unwrap();
    let (_, state) = fold.runs().next().unwrap();
    assert_eq!(
        (state.iteration, state.remaining, state.phase),
        (1, 1, Phase::Expired)
    );
    assert_eq!(
        state.state.data(),
        &Data::materialized(Value::Text("same request".into()))
    );
    let concurrent = source
        .replace(
            "app.in = \"same request\"",
            "app.in = \"same request\"\n      app.in = \"same request\"",
        )
        .replace("app.expired count == 1", "app.expired count == 2");
    let compilation =
        compiler::compile(&[&concurrent], &ContractCatalog::try_m0().unwrap()).unwrap();
    let runs = rastok::lab::run(&compilation.plan, GraphVersion(1), &compilation.labs[0]);
    assert!(runs[0].report.passed(), "{:?}", runs[0].report.failures);
    let fold = LoopView::from_records(&runs[0].records).unwrap();
    assert_eq!(fold.runs().count(), 2);
    assert!(fold.runs().all(|(_, run)| run.phase == Phase::Expired));
}

#[test]
fn every_committed_prefix_restores_cursors_state_and_pending_loop_commands() {
    let plan = compile(
        r#"
      Loop.fuel = 4
      var total: int = 0
      on in match {
        data <= 0 => out = total
        _ => { total = total + data; Loop.next = data - 1 }
      }
    "#,
    );
    let mut reference = Execution::new(&plan, GraphVersion(1));
    let mut expected = MemJournal::new(GraphVersion(1));
    inject(&mut reference, &mut expected, 3);
    while reference.step(&mut expected).unwrap() {
        assert_eq!(
            expected.loop_protocol().unwrap(),
            LoopView::from_records(expected.records()).unwrap(),
            "the fast projection must equal independent replay after every commit"
        );
    }
    let expected_records = expected.records();

    for prefix in 1..=expected_records.len() {
        let mut interrupted = Execution::new(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        inject(&mut interrupted, &mut journal, 3);
        while journal.records().len() < prefix {
            assert!(interrupted.step(&mut journal).unwrap());
        }
        let before = interrupted.inspection();
        let mut restored = Execution::new(&plan, GraphVersion(1));
        restored
            .recover_from_journal(&journal)
            .unwrap_or_else(|error| panic!("prefix {prefix}: {error}"));
        assert_eq!(journal.records().len(), prefix, "recovery must not append");
        assert_eq!(restored.inspection(), before, "prefix {prefix}");
        restored.run_ready(&mut journal).unwrap();
        assert_eq!(journal.records(), expected_records, "prefix {prefix}");
        assert_eq!(restored.outputs(), reference.outputs(), "prefix {prefix}");
        assert!(restored.is_stable());
    }
}

#[test]
fn a_loop_record_cannot_borrow_another_executors_transaction() {
    let plan = compile("Loop.fuel = 1; on in { Loop.again }");
    let mut vm = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 7);
    vm.run_ready(&mut journal).unwrap();
    let root = journal
        .records()
        .iter()
        .find_map(|record| match record {
            Record::Transaction(record) if record.owner_path.is_root() => {
                Some(record.instance.clone())
            }
            _ => None,
        })
        .unwrap();
    for (index, record) in journal.records().iter().enumerate() {
        let Record::Transaction(record) = record else {
            continue;
        };
        if record.loop_protocol().is_none() {
            continue;
        }
        let mut forged = record.as_ref().clone();
        forged.instance = root.clone();
        forged.owner_path = node::OwnerPath::root();
        assert_loop_record_refused(
            &plan,
            journal.records(),
            index,
            forged,
            LoopError::ContextOwner,
        );
    }
}

#[test]
fn a_routed_loop_context_must_name_a_retained_delivery() {
    let plan = compile("Loop.fuel = 1; on in { Loop.again }");
    let mut vm = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 7);
    vm.run_ready(&mut journal).unwrap();
    let (index, mut forged) = journal
        .records()
        .iter()
        .enumerate()
        .find_map(|(index, record)| {
            let Record::Transaction(record) = record else {
                return None;
            };
            record
                .loop_protocol()
                .is_some_and(|packet| !packet.produced.is_empty())
                .then(|| (index, record.as_ref().clone()))
        })
        .unwrap();
    let Some(packet) = forged.loop_record.as_deref_mut() else {
        panic!("the producer carries r2");
    };
    packet
        .routes
        .push(rastok::journal::loop_protocol::RoutedContext {
            delivery: rastok::envelope::Id::derive(
                rastok::envelope::IdKind::Root,
                &[b"unretained-delivery"],
            ),
            runs: packet.context.clone(),
        });
    assert_loop_record_refused(
        &plan,
        journal.records(),
        index,
        forged,
        LoopError::ContextDelivery,
    );
}

#[test]
fn loop_expiry_cannot_be_folded_without_its_public_emission() {
    let plan = compile("Loop.fuel = 1; on in { Loop.again }");
    let mut vm = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 7);
    vm.run_ready(&mut journal).unwrap();
    let (index, forged) = journal
        .records()
        .iter()
        .enumerate()
        .find_map(|(index, record)| {
            let Record::Transaction(record) = record else {
                return None;
            };
            record
                .loop_protocol()
                .is_some_and(|packet| {
                    packet
                        .facts
                        .iter()
                        .any(|fact| matches!(fact, Fact::Expired { .. }))
                })
                .then(|| (index, record.as_ref().clone()))
        })
        .unwrap();
    for alteration in 0..4 {
        let mut changed = forged.clone();
        match alteration {
            0 => changed.emissions.clear(),
            1 => changed.emissions[0].port = rastok::node::Port::new("out").unwrap(),
            2 => changed.emissions[0].index = 1,
            _ => changed.emissions.push(changed.emissions[0].clone()),
        }
        assert_loop_record_refused(
            &plan,
            journal.records(),
            index,
            changed,
            LoopError::ExpiryEmission,
        );
    }
}

fn assert_loop_record_refused(
    plan: &plan::ValidatedExecutionPlan,
    reference: &[Record],
    index: usize,
    forged: rastok::journal::TransactionRecord,
    error: LoopError,
) {
    let mut records = reference[..index].to_vec();
    records.push(Record::Transaction(Box::new(forged.clone())));
    assert_eq!(
        LoopView::from_records(&records),
        Err(error),
        "forged record {index} replay"
    );

    let mut vm = Execution::new(plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 7);
    while journal.records().len() < index {
        assert!(vm.step(&mut journal).unwrap());
    }
    assert_eq!(journal.records(), &reference[..index]);
    let before = journal.loop_protocol().unwrap();
    assert_eq!(
        journal.commit(forged),
        Err(rastok::journal::Error::InvalidLoopProtocol(error)),
        "forged record {index} commit"
    );
    assert_eq!(journal.records(), &reference[..index]);
    assert_eq!(journal.loop_protocol().unwrap(), before);
}

#[test]
fn a_rejected_transaction_does_not_update_the_journal_loop_projection() {
    let plan = compile("Loop.fuel = 2; on in { out = data }");
    let mut vm = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 7);
    vm.run_ready(&mut journal).unwrap();
    let mut transaction = journal
        .records()
        .iter()
        .find_map(|record| match record {
            Record::Transaction(transaction)
                if transaction.loop_protocol().is_some_and(|packet| {
                    packet
                        .facts
                        .iter()
                        .any(|fact| matches!(fact, Fact::Open { .. }))
                }) =>
            {
                Some(transaction.clone())
            }
            _ => None,
        })
        .unwrap();
    let Some(packet) = transaction.loop_record.as_deref_mut() else {
        panic!("the selected record must carry the opening fact");
    };
    for fact in &mut packet.facts {
        if let Fact::Open {
            run,
            instance,
            opening,
            ..
        } = fact
        {
            *opening = rastok::journal::loop_protocol::OpeningAdmissionId::Delivery(
                rastok::envelope::Id::derive(rastok::envelope::IdKind::Root, &[b"another-opening"]),
            );
            *run = rastok::journal::loop_protocol::run_ref(instance, opening);
        }
    }
    // The Loop packet is valid; a later journal invariant must still reject
    // the entire transaction without publishing the prepared fold.
    journal.loop_protocol().unwrap().apply(packet).unwrap();
    transaction.owner_path = node::OwnerPath::new(["wrong-owner"]).unwrap();
    let before = journal.records().to_vec();
    let fold = journal.loop_protocol().unwrap();
    assert!(matches!(
        journal.commit(*transaction),
        Err(rastok::journal::Error::InstanceOwnerMismatch { .. })
    ));
    assert_eq!(journal.records(), before);
    assert_eq!(journal.loop_protocol().unwrap(), fold);
    assert_eq!(
        journal.loop_protocol().unwrap(),
        LoopView::from_records(&before).unwrap()
    );
}

#[test]
fn delayed_runs_reserve_manager_capacity_before_the_next_ingress_commit() {
    let plan = compile(
        r#"
      Loop.fuel = 2
      Loop.delay = { kind = fixed; after = duration("1s") }
      on in match { data == 0 => out = data; _ => Loop.next = 0 }
    "#,
    );
    let mut vm = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    // This is the Worker's existing mailbox/work bound, not a new quota.
    for _ in 0..1024 {
        inject(&mut vm, &mut journal, 1);
    }
    vm.run_ready(&mut journal).unwrap();
    assert!(
        vm.inspection()
            .mailboxes
            .iter()
            .all(|mailbox| mailbox.deliveries.is_empty())
    );
    let before = journal.records().to_vec();
    let incoming = || IngressProposal {
        provenance: IngressProvenance::single(rastok::node::Path::new(["test"]).unwrap()),
        draft: EnvelopeDraft {
            port: rastok::node::Port::new("in").unwrap(),
            data: Data::materialized(Value::Int(1)),
            ext: Namespace::new(),
        },
        sequence: IngressSequence::Next,
        session: None,
    };
    assert!(matches!(
        vm.accept_input(&mut journal, incoming()),
        Err(rastok::runtime::execution::ExecutionFault::RecurrenceQueueFull { .. })
    ));
    assert_eq!(
        journal.records(),
        before,
        "refusal must not admit another root"
    );
    let mut bindings = RuntimeBindings::try_new(&plan, &DriverRegistry::default()).unwrap();
    vm.advance_to(&mut journal, &mut bindings, 1_000_000_000)
        .unwrap();
    vm.run_ready(&mut journal).unwrap();
    assert_eq!(vm.outputs().len(), 1024);
    vm.accept_input(&mut journal, incoming()).unwrap();
    vm.run_ready(&mut journal).unwrap();
    let fold = LoopView::from_records(journal.records()).unwrap();
    assert_eq!(
        fold.runs()
            .filter(|(_, run)| run.phase == Phase::AwaitingAdmission)
            .count(),
        1
    );
}

#[test]
fn call_terminals_close_naturally_without_turning_frames_into_verdicts() {
    for (contract, settings, outputs, terminal, frame_count, extra_routes) in [
        (
            "Process",
            "command = \"cat\"; codec = lines; idempotent = true; behavior = deterministic",
            "out = \"first\"; out = \"second\"",
            "Exited { exit_code = 0; signal = null }",
            2,
            "worker.err -> nothing",
        ),
        (
            "File",
            "mode = read; path = \"unused.bin\"",
            "out = base64! { YQ== }",
            "Completed {}",
            1,
            "",
        ),
        (
            "HttpClient",
            "url = \"https://example.invalid\"; codec = text; idempotent = true; behavior = probabilistic",
            "out = \"reply\"",
            "Completed { status = 200 }",
            1,
            "",
        ),
    ] {
        let source = format!(
            r#"
graph app : App {{
  input in: text
  output out: variant
  connector worker : {contract} + Loop {{
    {settings}
    timeout = duration("1s")
    Loop.fuel = 2
  }}
  in -> worker
  worker.out -> out
  {extra_routes}
  worker.exit -> nothing
}}
lab app {{
  simulation completed on app {{
    mock worker {{ on in {{ {outputs}; exit = {terminal} }} }}
    scenario one_call {{
      app.in = "request"
      expect app.out count == {frame_count}
    }}
  }}
}}
"#
        );
        let compilation = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap())
            .unwrap_or_else(|error| panic!("{contract}: {error:?}"));
        let (plan, _) = plan::decode(&plan::encode(&compilation.plan, &compilation.hash)).unwrap();
        let runs = rastok::lab::run(&plan, GraphVersion(1), &compilation.labs[0]);
        assert!(
            runs[0].report.passed(),
            "{contract}: {:?}",
            runs[0].report.failures
        );
        let fold = LoopView::from_records(&runs[0].records).unwrap();
        let (_, state) = fold.runs().next().unwrap();
        assert_eq!(
            (state.iteration, state.remaining, state.phase),
            (0, 2, Phase::Closed),
            "{contract}"
        );
        assert_eq!(
            runs[0]
                .records
                .iter()
                .filter(|record| matches!(record,
            Record::Egress(rastok::journal::EgressRecord::Call(call))
                if call.status == rastok::journal::CallStatus::Open))
                .count(),
            1
        );
    }
}

#[test]
fn a_single_frame_can_propose_next_but_cannot_compete_with_the_exit() {
    let source = r#"
graph app : App {
  input in: variant
  connector worker : HttpClient + Loop {
    url = "https://example.invalid"
    codec = json
    idempotent = true
    behavior = probabilistic
    timeout = duration("1s")
    Loop.fuel = 2
  }
  in -> worker
  worker.out -> worker.Loop.next
  worker.exit -> nothing
}
lab app {
  simulation repeated on app {
    mock worker { on in { out = 9; exit = Completed { status = 200 } } }
    scenario twice { app.in = 3 }
  }
}
"#;
    let compilation = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let runs = rastok::lab::run(&compilation.plan, GraphVersion(1), &compilation.labs[0]);
    assert!(runs[0].report.passed(), "{:?}", runs[0].report.failures);
    let fold = LoopView::from_records(&runs[0].records).unwrap();
    let (_, run) = fold.runs().next().unwrap();
    assert_eq!((run.iteration, run.phase), (1, Phase::Expired));
    assert_eq!(run.state.data(), &Data::materialized(Value::Int(9)));

    let mut forged = compilation.plan.plan().clone();
    let exit = forged
        .edges
        .iter()
        .find(|edge| edge.from.port.name() == "exit")
        .unwrap();
    let node = &forged.nodes[exit.from.node.index()];
    let owner = plan::EndpointOwner::Node(node.executor_path().unwrap().clone());
    forged.routes.push(plan::RouteRule {
        source: plan::Endpoint {
            owner: owner.clone(),
            port: exit.from.port.clone(),
        },
        selection: plan::RouteSelection::All(vec![plan::RouteDestination::LoopCommand(
            plan::LoopCommandTarget {
                owner,
                kind: plan::LoopCommandKind::Again,
            },
        )]),
    });
    forged.edges.retain(|edge| edge.from.port.name() != "exit");
    forged
        .routes
        .sort_by_key(plan::RouteRule::canonical_order_key);
    let error = plan::ValidatedExecutionPlan::try_new(forged).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("LoopExpired closes the run and cannot command it again"),
        "{error}"
    );

    let missing_frame = source.replace("out = 9", "out = nothing");
    let compilation =
        compiler::compile(&[&missing_frame], &ContractCatalog::try_m0().unwrap()).unwrap();
    let runs = rastok::lab::run(&compilation.plan, GraphVersion(1), &compilation.labs[0]);
    assert!(
        runs[0]
            .report
            .failures
            .iter()
            .any(|error| error.contains("ContractViolation")),
        "{:?}",
        runs[0].report.failures
    );
    assert!(
        runs[0].records.iter().all(|record| match record {
            Record::Transaction(transaction) => transaction
                .loop_protocol()
                .is_none_or(|packet| packet.produced.is_empty()),
            _ => true,
        }),
        "a missing promised frame cannot silently select Natural Done"
    );
}

#[test]
fn recovery_reschedules_the_original_deadline_not_a_fresh_delay() {
    let plan = compile(
        r#"
      Loop.fuel = 2
      Loop.delay = { kind = fixed; after = duration("1s") }
      on in match { data <= 0 => out = data; _ => Loop.next = data - 1 }
    "#,
    );
    let mut interrupted = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut interrupted, &mut journal, 1);
    interrupted.run_ready(&mut journal).unwrap();
    let before = journal.records().len();
    let mut restored = Execution::new_at(&plan, GraphVersion(1), 750_000_000);
    restored.recover_from_journal(&journal).unwrap();
    restored.run_ready(&mut journal).unwrap();
    assert!(restored.outputs().is_empty());
    assert_eq!(journal.records().len(), before);
    let mut bindings = RuntimeBindings::try_new(&plan, &DriverRegistry::default()).unwrap();
    restored
        .advance_to(&mut journal, &mut bindings, 1_000_000_000)
        .unwrap();
    restored.run_ready(&mut journal).unwrap();
    assert_eq!(restored.outputs().len(), 1);
    assert!(restored.is_stable());
}

#[test]
fn unsealed_deadline_bytes_are_required_for_delays_but_not_part_of_the_causal_seal() {
    use rastok::journal::deadline::Deadlines;
    let plan = compile(
        r#"
      Loop.fuel = 2
      Loop.delay = { kind = fixed; after = duration("1s") }
      on in { Loop.next = data }
    "#,
    );
    let mut vm = Execution::new_at(&plan, GraphVersion(1), 7_000_000_000);
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 1);
    vm.run_ready(&mut journal).unwrap();
    let index = journal
        .records()
        .iter()
        .position(|record| record.deadlines().is_some())
        .unwrap();
    let schedule = journal.records()[index].deadlines().unwrap();
    assert_eq!(schedule.created_at(), 7_000_000_000);
    assert_eq!(schedule.items()[0].due_at, 8_000_000_000);
    let mut decoder = rastok::journal::codec::Decoder::default();
    for record in &journal.records()[..=index] {
        let restored = decoder
            .decode(&rastok::journal::codec::encode(record))
            .unwrap();
        assert_eq!(&restored, record);
    }

    let before = journal.records().to_vec();
    let seal = rastok::golden::project(&before).seal();
    for corrupt in 0..4 {
        let mut records = before.clone();
        let Record::Transaction(transaction) = &mut records[index] else {
            unreachable!()
        };
        match corrupt {
            0 => transaction.deadlines = None,
            1 => {
                let schedule = transaction.deadlines.as_deref().unwrap();
                let mut deadlines = schedule.items().to_vec();
                deadlines[0].due_at += 1;
                transaction.deadlines = Some(Box::new(
                    Deadlines::try_new(schedule.created_at(), deadlines).unwrap(),
                ));
            }
            2 => {
                let packet = transaction.loop_record.as_deref_mut().unwrap();
                let Fact::Advance { due_at, .. } = &mut packet.facts[0] else {
                    unreachable!()
                };
                *due_at = Some(8_000_000_001);
            }
            _ => {
                let schedule = transaction.deadlines.as_deref().unwrap();
                let deadline = schedule.items()[0];
                transaction.deadlines = Some(Box::new(
                    Deadlines::try_new(schedule.created_at(), vec![deadline, deadline]).unwrap(),
                ));
            }
        }
        assert!(matches!(
            journal.commit(transaction.as_ref().clone()),
            Err(rastok::journal::Error::InvalidDeadlines(_))
        ));
        assert_eq!(
            rastok::golden::project(&records).seal(),
            seal,
            "retention needs storage integrity, not the causal seal"
        );
        assert_eq!(
            journal.records(),
            before,
            "a rejected schedule commits nothing"
        );
    }
}

#[test]
fn a_corrupt_consumption_cannot_partially_replace_a_running_vm() {
    let source = r#"
graph app : App {
  input in: int
  output out: int
  operator sum {
    var value: int = 0
    on in { value = value + data; out = value }
  }
  in -> sum -> out
}
"#;
    let plan = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap())
        .unwrap()
        .plan;
    let mut vm = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 7);
    vm.run_ready(&mut journal).unwrap();
    let transaction = journal
        .records()
        .iter()
        .find_map(|record| match record {
            Record::Transaction(transaction) if !transaction.consumed.is_empty() => {
                Some(transaction.clone())
            }
            _ => None,
        })
        .unwrap();
    journal.commit(*transaction).unwrap();
    let before = vm.inspection();
    let error = vm.recover_from_journal(&journal).unwrap_err();
    assert!(matches!(
        error,
        rastok::runtime::execution::ExecutionFault::InvalidRecovery {
            reason: "a consumed cursor has no pending delivery",
            ..
        }
    ));
    assert_eq!(vm.inspection(), before);
}

#[test]
fn a_rederived_measure_bounds_commands_without_authored_fuel() {
    for dispatch in [
        "on in match { data <= 0 => out = data; _ => Loop.next = data - 1 }",
        "on in match data > 0 { true => Loop.next = Loop.state.data - 1; false => out = data }",
    ] {
        let plan = compile(&format!("Loop.measure = Loop.state.data; {dispatch}"));
        let proof = plan
            .nodes
            .iter()
            .find_map(|owner| owner.loop_spec())
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(proof.decreases, [1]);
        for value in [-2, 0, 3] {
            let mut vm = Execution::new(&plan, GraphVersion(1));
            let mut journal = MemJournal::new(GraphVersion(1));
            inject(&mut vm, &mut journal, value);
            vm.run_ready(&mut journal).unwrap();
            assert_eq!(vm.outputs().len(), 1);
            assert_eq!(vm.outputs()[0].port.name(), "out");
            let fold = LoopView::from_records(journal.records()).unwrap();
            let run = fold.runs().next().unwrap().1;
            assert_eq!(run.phase, Phase::Closed);
            assert_eq!(run.iteration, u64::try_from(value).unwrap_or(0));
        }
    }
}

#[test]
fn explicit_fuel_remains_a_cap_even_when_a_measure_is_proved() {
    let plan = compile(
        "Loop.fuel = 2; Loop.measure = Loop.state.data; on in match { data <= 0 => out = data; _ => Loop.next = data - 1 }",
    );
    let mut vm = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 8);
    vm.run_ready(&mut journal).unwrap();
    assert_eq!(vm.outputs().len(), 1);
    assert_eq!(vm.outputs()[0].port.name(), "expired");
}

#[test]
fn a_package_cannot_replace_the_measured_decrease_with_a_claim() {
    let plan = compile(
        "Loop.measure = Loop.state.data; on in match { data <= 0 => out = data; _ => Loop.next = data - 1 }",
    );
    let mut raw = plan.plan().clone();
    let proof = raw
        .nodes
        .iter_mut()
        .find_map(|owner| {
            owner
                .extensions
                .iter_mut()
                .find_map(|extension| match extension {
                    plan::ExtensionSpec::Loop(spec) => spec.proof.as_mut(),
                    plan::ExtensionSpec::App | plan::ExtensionSpec::Retry(_) => None,
                })
        })
        .unwrap();
    proof.decreases = vec![2];
    assert!(matches!(
        plan::ValidatedExecutionPlan::try_new(raw),
        Err(plan::PlanValidationError::InvalidLoopControl {
            detail: "the Loop proof certificate disagrees with the shipped handlers",
            ..
        })
    ));
}

#[test]
fn a_package_cannot_erase_its_terminal_selection() {
    let plan = compile("Loop.fuel = 1; on in { out = data }");
    let mut raw = plan.plan().clone();
    for owner in &mut raw.nodes {
        for extension in &mut owner.extensions {
            if let plan::ExtensionSpec::Loop(spec) = extension {
                spec.terminals.clear();
            }
        }
    }
    assert!(matches!(
        plan::ValidatedExecutionPlan::try_new(raw),
        Err(plan::PlanValidationError::InvalidGraphContract {
            detail: "Loop exhaustion must name the resolved compatible host terminal",
            ..
        })
    ));
}

#[test]
fn a_measure_refuses_an_unproved_fallback_or_an_unchanged_again() {
    for dispatch in [
        "on in match { data == 0 => out = data; _ => Loop.next = data - 1 }",
        "on in match { data <= 0 => out = data; _ => Loop.again }",
        "on in match { data <= 0 => out = data; _ => Loop.next = data + 1 }",
    ] {
        let source = format!(
            "graph app : App {{ input in: int; output out: int; operator work : Loop {{ input in: int; output out: int; Loop.measure = Loop.state.data; {dispatch} }} in -> work -> out }}"
        );
        let errors =
            compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap_err();
        assert!(
            format!("{errors:?}").contains("cannot be proved finite"),
            "{errors:?}"
        );
    }
}
fn inject(vm: &mut Execution<'_>, journal: &mut MemJournal, value: i64) {
    vm.accept_input(
        journal,
        IngressProposal {
            provenance: IngressProvenance::single(rastok::node::Path::new(["test"]).unwrap()),
            draft: EnvelopeDraft {
                port: rastok::node::Port::new("in").unwrap(),
                data: Data::materialized(Value::Int(value)),
                ext: Namespace::new(),
            },
            sequence: IngressSequence::Next,
            session: None,
        },
    )
    .unwrap();
}

fn compile(body: &str) -> plan::ValidatedExecutionPlan {
    let source = format!(
        r#"
graph app : App {{
  input in: int
  output out: int
  output expired: union {{ LoopExpired {{ last_state: int }} }}
  operator work : Loop {{
    input in: int
    output out: int
    output err: union {{ LoopExpired {{ last_state: int }} }}
    {body}
  }}
  in -> work -> out
  work.err -> expired
}}
"#
    );
    let result = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
    plan::decode(&plan::encode(&result.plan, &result.hash))
        .unwrap()
        .0
}

#[test]
fn again_keeps_data_and_private_state_but_not_the_previous_causal_identity() {
    let plan = compile(
        r#"
      Loop.fuel = 3
      var attempts: int = 0
      on in match {
        attempts == 2 => out = Loop.state.data
        _ => { attempts = attempts + 1; Loop.again }
      }
    "#,
    );
    let mut vm = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 42);
    vm.run_ready(&mut journal).unwrap();
    assert_eq!(vm.outputs().len(), 1);
    assert_eq!(vm.outputs()[0].port.name(), "out");
    assert_eq!(
        vm.outputs()[0].envelope.data(),
        &Data::materialized(Value::Int(42))
    );
    let admissions: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction) => transaction.loop_protocol(),
            _ => None,
        })
        .flat_map(|packet| &packet.facts)
        .filter_map(|fact| match fact {
            Fact::Admit { envelope, .. } => Some(envelope),
            _ => None,
        })
        .collect();
    assert_eq!(admissions.len(), 3);
    assert_eq!(
        admissions
            .iter()
            .map(|env| env.meta().sys().id())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
    assert!(
        admissions
            .iter()
            .all(|env| env.data() == admissions[0].data()
                && env.meta().sys().origin_id() == admissions[0].meta().sys().origin_id())
    );
    assert_eq!(
        LoopView::from_records(journal.records())
            .unwrap()
            .runs()
            .next()
            .unwrap()
            .1
            .phase,
        Phase::Closed
    );
}

#[test]
fn expiry_contains_the_proposed_next_state_without_an_extra_application() {
    let plan = compile("Loop.fuel = 2; on in { Loop.next = data + 1 }");
    let mut vm = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 7);
    vm.run_ready(&mut journal).unwrap();
    assert_eq!(vm.outputs().len(), 1);
    assert_eq!(vm.outputs()[0].port.name(), "expired");
    let Data::Materialized(Value::TaggedUnion(value)) = vm.outputs()[0].envelope.data() else {
        panic!("typed expiry")
    };
    assert_eq!(value.case, "LoopExpired");
    assert_eq!(value.payload, [("last_state".into(), Value::Int(9))]);
    let fold = LoopView::from_records(journal.records()).unwrap();
    let run = fold.runs().next().unwrap().1;
    assert_eq!(
        (run.iteration, run.remaining, run.phase),
        (1, 1, Phase::Expired)
    );
    assert!(vm.is_stable());
}

#[test]
fn delayed_commands_wait_on_logical_time_without_reserving_reentry() {
    let plan = compile(
        r#"
      Loop.fuel = 3
      Loop.delay = { kind = fixed; after = duration("1s") }
      on in match { data <= 0 => out = data; _ => Loop.next = data - 1 }
    "#,
    );
    let mut vm = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 1);
    vm.run_ready(&mut journal).unwrap();
    assert!(vm.outputs().is_empty());
    let fold = LoopView::from_records(journal.records()).unwrap();
    let run = fold.runs().next().unwrap().1;
    assert_eq!(run.phase, Phase::AwaitingAdmission);
    assert_eq!(run.due_at, Some(1_000_000_000));
    let mut bindings = RuntimeBindings::try_new(&plan, &DriverRegistry::default()).unwrap();
    vm.advance_to(&mut journal, &mut bindings, 999_999_999)
        .unwrap();
    vm.run_ready(&mut journal).unwrap();
    assert!(vm.outputs().is_empty());
    vm.advance_to(&mut journal, &mut bindings, 1_000_000_000)
        .unwrap();
    vm.run_ready(&mut journal).unwrap();
    assert_eq!(vm.outputs().len(), 1);
    assert_eq!(
        vm.outputs()[0].envelope.data(),
        &Data::materialized(Value::Int(0))
    );
}

#[test]
fn competing_or_absent_verdicts_are_rejected_before_execution() {
    for dispatch in [
        "on in { Loop.again; Loop.done }",
        "on in { out = data; Loop.again }",
        "on in { Loop.again; stop }",
        "on in match { data > 0 => Loop.again; _ => nothing }",
    ] {
        let source = format!(
            r#"
graph app : App {{
  input in: int
  output out: int
  operator work : Loop {{
    input in: int
    output out: int
    output err: union {{ LoopExpired {{ last_state: int }} }}
    Loop.fuel = 3
    {dispatch}
  }}
  in -> work -> out
  work.err -> nothing
}}
"#
        );
        let error = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap_err();
        assert!(format!("{error:?}").contains("E-LOOP-CONTROL"), "{error:?}");
    }
}

#[test]
fn fanout_runs_in_one_instance_keep_separate_admissions_and_state() {
    let source = r#"
graph app : App {
  input in: int
  output out: int
  operator left { on in { out = data } }
  operator right { on in { out = data + 1 } }
  operator work : Loop {
    input in: int
    output err: union { LoopExpired { last_state: int } }
    output out: int
    Loop.fuel = 3
    on in match { data <= 0 => out = Loop.state.data; _ => Loop.next = data - 1 }
  }
  in -> { left; right }
  { left.out; right.out } -> work
  work.out -> out
  work.err -> nothing
}
"#;
    let compilation = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let decoded = plan::decode(&plan::encode(&compilation.plan, &compilation.hash))
        .unwrap()
        .0;
    let mut vm = Execution::new(&decoded, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 1);
    vm.run_ready(&mut journal).unwrap();
    assert_eq!(vm.outputs().len(), 2);
    assert_ne!(
        vm.outputs()[0].envelope.meta().sys().id(),
        vm.outputs()[1].envelope.meta().sys().id()
    );
    assert_eq!(
        vm.outputs()[0].envelope.meta().sys().origin_id(),
        vm.outputs()[1].envelope.meta().sys().origin_id()
    );
    let fold = LoopView::from_records(journal.records()).unwrap();
    let runs: Vec<_> = fold.runs().collect();
    assert_eq!(runs.len(), 2);
    assert_ne!(runs[0].0, runs[1].0);
    assert_eq!(runs[0].1.instance, runs[1].1.instance);
    assert!(runs.iter().all(|(_, run)| run.phase == Phase::Closed));
    assert_eq!(runs.iter().map(|(_, run)| run.iteration).sum::<u64>(), 3);
}

#[test]
fn an_unrelated_join_does_not_bypass_loop_application_admission() {
    let source = r#"
graph app : App {
  input in: int
  output out: int
  operator work : Loop {
    input in: int; output out: int; Loop.fuel = 2
    output err: union { LoopExpired { last_state: int } }
    on in match { data <= 0 => out = data; _ => Loop.next = data - 1 }
  }
  operator joiner {
    input a: int; input b: int; output out: int
    on (a, b) { out = a.data + b.data }
  }
  in -> { work; joiner.a; joiner.b }
  work.out -> out
  work.err -> nothing
  joiner.out -> nothing
}
"#;
    let compilation = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let mut vm = Execution::new(&compilation.plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 1);
    vm.run_ready(&mut journal).unwrap();
    assert_eq!(vm.outputs().len(), 1);
    assert_eq!(
        vm.outputs()[0].envelope.data(),
        &Data::materialized(Value::Int(0))
    );
    assert!(vm.is_stable());
}

#[test]
fn lifetime_closure_invalidates_pending_reentry_before_the_next_generation() {
    let plan = compile(
        r#"
      lifetime = { kind = fixed; after = duration("1s") }
      Loop.fuel = 3
      Loop.delay = { kind = fixed; after = duration("2s") }
      on in { Loop.again }
    "#,
    );
    let mut vm = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut bindings = RuntimeBindings::try_new(&plan, &DriverRegistry::default()).unwrap();
    inject(&mut vm, &mut journal, 7);
    vm.run_ready(&mut journal).unwrap();
    vm.advance_to(&mut journal, &mut bindings, 1_000_000_000)
        .unwrap();
    let fold = LoopView::from_records(journal.records()).unwrap();
    assert_eq!(fold.runs().next().unwrap().1.phase, Phase::Closed);
    inject(&mut vm, &mut journal, 8);
    vm.run_ready(&mut journal).unwrap();
    vm.advance_to(&mut journal, &mut bindings, 2_000_000_000)
        .unwrap();
    let fold = LoopView::from_records(journal.records()).unwrap();
    assert_eq!(fold.runs().count(), 2);
    assert!(fold.runs().all(|(_, run)| run.iteration == 1));
    assert!(vm.outputs().is_empty());
}
