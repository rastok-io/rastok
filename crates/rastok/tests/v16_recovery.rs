//! Recovery consumes retained facts, never a second execution of producers.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::boundary::{EnvelopeDraft, IngressProposal, IngressProvenance, IngressSequence};
use rastok::compiler;
use rastok::contract::ContractCatalog;
use rastok::envelope::{Data, Namespace, Value};
use rastok::journal::{DeadlineKind, Journal, MemJournal, Record, ids};
use rastok::plan::{self, GraphVersion};
use rastok::runtime::execution::Execution;

#[test]
fn an_overdue_queued_call_cannot_start_an_effect_after_recovery() {
    let plan = compile(CALL_LOOP);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Execution::new(&plan, GraphVersion(1));
    inject(&mut vm, &mut journal, "in", 3);
    let mut world = World::default();
    let mut recovered = Execution::new_at(&plan, GraphVersion(1), 4_000_000_000);
    recovered
        .recover_with_bindings(&journal, &mut world)
        .unwrap();
    recovered
        .run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    recovered.poll_drivers(&mut journal, &mut world).unwrap();
    assert_eq!(
        world.starts, 0,
        "queue time still counts while the Worker was absent"
    );
    assert!(journal.open_calls().is_empty());
}

#[test]
fn bounded_closing_recovers_cancellation_settlement_and_residual_consumption() {
    let plan = compile(
        r#"
graph app : App {
  input in: int
  output exit: variant
  lifetime = { kind = fixed; after = duration("2s") }
  closing = { drain_for = duration("1s"); settle_for = duration("1s") }
  connector call : Process {
    command = "cat"; codec = jsonl
    idempotent = true; behavior = deterministic
  }
  in -> call
  call.out -> nothing
  call.err -> nothing
  call.exit -> exit
}
"#,
    );
    for queued in [false, true] {
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Execution::new_at(&plan, GraphVersion(1), 7_000_000_000);
        let mut world = World::default();
        inject(&mut vm, &mut journal, "in", 3);
        if queued {
            inject(&mut vm, &mut journal, "in", 4);
        }
        vm.run_ready_with_bindings(&mut journal, &mut world)
            .unwrap();
        vm.poll_drivers(&mut journal, &mut world).unwrap();
        assert_eq!(world.starts, 1);
        for at in [8_000_000_000, 9_000_000_000, 10_000_000_000, 11_000_000_000] {
            vm.advance_to(&mut journal, &mut world, at).unwrap();
            let retained = journal.records().to_vec();
            let mut recovered = Execution::new_at(&plan, GraphVersion(1), at);
            recovered
                .recover_with_bindings(&journal, &mut world)
                .unwrap_or_else(|error| panic!("queued={queued}, at={at}: {error}"));
            assert_eq!(world.starts, 1, "recovery starts no effect");
            assert_eq!(journal.records(), retained);
            recovered.advance_to(&mut journal, &mut world, at).unwrap();
            assert_eq!(
                recovered.inspection(),
                vm.inspection(),
                "queued={queued}, at={at}"
            );
            assert_eq!(
                journal.records(),
                retained,
                "no duplicate closure or residual decision"
            );
        }
        assert!(vm.is_stable());
        assert_eq!(vm.outputs().len(), 1);
    }
}

fn compile(source: &str) -> plan::ValidatedExecutionPlan {
    let compiled = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    plan::decode(&plan::encode(&compiled.plan, &compiled.hash))
        .unwrap()
        .0
}

fn inject(vm: &mut Execution<'_>, journal: &mut MemJournal, port: &str, value: i64) {
    vm.accept_input(
        journal,
        IngressProposal {
            provenance: IngressProvenance::single(
                rastok::node::Path::new(["recovery-test"]).unwrap(),
            ),
            draft: EnvelopeDraft {
                port: rastok::node::Port::new(port).unwrap(),
                data: Data::materialized(Value::Int(value)),
                ext: Namespace::new(),
            },
            sequence: IngressSequence::Next,
            session: None,
        },
    )
    .unwrap();
}

const DELIVERY_DEADLINES: &str = r#"
graph app : App {
  input in: int
  input right: int
  operator forward {
    input in: int
    output out: int
    on in { out = data }
  }
  operator pair {
    input left: int
    input right: int
    output expired: union { JoinExpired {} }
    on (left, right) within duration("2s") { nothing }
  }
  connector call : Process {
    command = "cat"
    codec = jsonl
    idempotent = true
    behavior = deterministic
    timeout = duration("3s")
  }
  in -> forward
  forward.out -> { pair.left; call.in }
  right -> pair.right
  pair.expired -> nothing
  call.out -> nothing
  call.err -> nothing
  call.exit -> nothing
}
"#;

#[derive(Default)]
struct World {
    starts: usize,
    answers: Vec<rastok::boundary::Response>,
}

impl rastok::boundary::CallConnectorDriver for World {
    fn prepare_request(
        &mut self,
        _: &rastok::node::Path,
        data: &Data,
        _: &std::collections::BTreeMap<String, Value>,
    ) -> rastok::boundary::RequestPreparation {
        rastok::boundary::prepare_process_request(rastok::boundary::RequestCodec::Jsonl, data)
    }

    fn scheduler_completion(
        &mut self,
        _: &rastok::node::Path,
        completion: rastok::boundary::SchedulerCompletion,
    ) -> rastok::boundary::Completion {
        rastok::boundary::process_scheduler_completion(completion)
    }

    fn start(
        &mut self,
        _: &rastok::boundary::CallOpen,
        _: &rastok::envelope::Envelope,
    ) -> rastok::boundary::CallStart {
        self.starts += 1;
        rastok::boundary::CallStart::Started
    }

    fn events(&mut self) -> Vec<rastok::boundary::Response> {
        std::mem::take(&mut self.answers)
    }
}

#[test]
fn a_nested_call_run_recovers_timeouts_and_never_restarts_an_old_effect() {
    use rastok::journal::loop_protocol::{LoopView, Phase};
    let plan = compile(
        r#"
graph app : App + Loop {
  input in: int
  output err: union { LoopExpired { last_state: int } }
  output out: int
  output exhausted: variant
  Loop.fuel = 2
  Loop.terminal = [out, exhausted, err]
  connector call : Process + Loop {
    command = "cat"; codec = jsonl
    idempotent = true; behavior = deterministic
    timeout = duration("3s")
    Loop.fuel = 2
    Loop.delay = { kind = fixed; after = duration("1s") }
  }
  operator finish { input in: variant; output out: int; on in { out = 1 } }
  in -> call
  call.out -> nothing
  call.err -> nothing
  call.exit -> match {
    LoopExpired -> exhausted
    TimeoutExpired -> call.Loop.again
    _ -> finish
  }
  finish.out -> out
}
"#,
    );
    for exhausted in [false, true] {
        let mut vm = Execution::new(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut world = World::default();
        inject(&mut vm, &mut journal, "in", 1);
        vm.run_ready_with_bindings(&mut journal, &mut world)
            .unwrap();
        vm.poll_drivers(&mut journal, &mut world).unwrap();
        assert_eq!(world.starts, 1);
        for at in [0, 3_000_000_000, 4_000_000_000] {
            vm.advance_to(&mut journal, &mut world, at).unwrap();
            vm.poll_drivers(&mut journal, &mut world).unwrap();
            let mut restored = Execution::new_at(&plan, GraphVersion(1), at);
            restored
                .recover_with_bindings(&journal, &mut world)
                .unwrap();
            assert_eq!(restored.inspection(), vm.inspection());
            vm = restored;
        }
        vm.poll_drivers(&mut journal, &mut world).unwrap();
        assert_eq!(world.starts, 2);
        if exhausted {
            vm.advance_to(&mut journal, &mut world, 7_000_000_000)
                .unwrap();
        } else {
            world.answers.push(rastok::boundary::Response {
                call_id: journal.open_calls()[0].call_id,
                completion: rastok::boundary::Completion::process_exit(
                    rastok::boundary::ProcessExit::Exited {
                        exit_code: 0,
                        signal: None,
                    },
                ),
            });
            vm.poll_drivers(&mut journal, &mut world).unwrap();
        }
        for _ in 0..100 {
            let mut restored = Execution::new_at(&plan, GraphVersion(1), vm.logical_time());
            restored
                .recover_with_bindings(&journal, &mut world)
                .unwrap();
            assert_eq!(restored.inspection(), vm.inspection());
            vm = restored;
            if vm.is_stable() {
                break;
            }
            assert!(vm.step_with_bindings(&mut journal, &mut world).unwrap());
        }
        assert!(vm.is_stable());
        assert_eq!(world.starts, 2);
        assert_eq!(vm.outputs().len(), 1);
        assert_eq!(
            vm.outputs()[0].port.name(),
            if exhausted { "exhausted" } else { "out" }
        );
        let fold = LoopView::from_records(journal.records()).unwrap();
        assert_eq!(fold.runs().count(), 2);
        assert!(
            fold.runs()
                .all(|(_, state)| matches!(state.phase, Phase::Closed | Phase::Expired))
        );
    }
}

const CALL_LOOP: &str = r#"
graph app : App {
  input in: int
  output out: variant
  connector call : Process + Loop {
    command = "cat"
    codec = jsonl
    idempotent = true
    behavior = deterministic
    timeout = duration("3s")
    Loop.fuel = 2
    Loop.delay = { kind = fixed; after = duration("1s") }
  }
  in -> call
  call.out -> out
  call.err -> nothing
  call.exit -> match { TimeoutExpired -> call.Loop.again; _ -> nothing }
}
"#;

const GRAPH_CALLS: &str = r#"
graph app : App {
  input in: int
  output out: int
  output expired: union { LoopExpired { last_state: int } }
  graph cycle : Loop {
    input in: int
    output err: union { LoopExpired { last_state: int } }
    output out: int
    Loop.fuel = 2
    operator a_decide {
      input in: int
      output revised: int
      output accepted: int
      on in match { data > 0 => revised = data - 1; _ => accepted = data }
    }
    connector call : Process {
      command = "cat"
      codec = jsonl
      idempotent = true
      behavior = deterministic
      timeout = duration("3s")
    }
    operator z_audit {
      input in: variant
      on in { nothing }
    }
    in -> { a_decide; call }
    a_decide.revised -> Loop.next
    a_decide.accepted -> out
    call.out -> z_audit
    call.err -> nothing
    call.exit -> nothing
  }
  in -> cycle
  cycle.out -> out
  cycle.err -> expired
}
"#;

#[test]
fn graph_commands_wait_for_calls_and_recovery_never_restarts_them() {
    use rastok::journal::loop_protocol::{LoopView, Phase};

    let plan = compile(GRAPH_CALLS);
    let mut vm = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal, "in", 1);
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    assert_eq!(world.starts, 1);
    let fold = LoopView::from_records(journal.records()).unwrap();
    let (_, state) = fold.runs().next().unwrap();
    assert_eq!((state.iteration, state.phase), (0, Phase::Active));
    assert!(
        state.pending.is_some(),
        "the sibling has already proposed Next"
    );
    assert!(
        !vm.has_ready_work(),
        "a verdict blocked by a Call is not ready work"
    );
    assert!(
        !vm.is_stable(),
        "the unfinished application still owns its Call"
    );

    let retained = journal.records().to_vec();
    let mut restored = Execution::new(&plan, GraphVersion(1));
    restored
        .recover_with_bindings(&journal, &mut world)
        .unwrap();
    assert_eq!(restored.inspection(), vm.inspection());
    restored.poll_drivers(&mut journal, &mut world).unwrap();
    assert_eq!(world.starts, 1, "recovery does not recall the open Call");
    assert_eq!(journal.records(), retained);

    for iteration in 0..2 {
        let call = journal.open_calls()[0].call_id;
        world.answers.push(rastok::boundary::Response {
            call_id: call,
            completion: rastok::boundary::Completion::process(
                vec![Data::materialized(Value::Int(42))],
                vec![],
                rastok::boundary::ProcessExit::Exited {
                    exit_code: 0,
                    signal: None,
                },
            ),
        });
        restored.poll_drivers(&mut journal, &mut world).unwrap();
        // The retained closure is not yet the consuming transaction. From
        // here, restore before every commit, including the frame's audit.
        while !journal.open_calls().is_empty() || restored.has_ready_work() {
            let before = journal.records().to_vec();
            let mut next = Execution::new(&plan, GraphVersion(1));
            next.recover_with_bindings(&journal, &mut world)
                .unwrap_or_else(|error| {
                    panic!("iteration {iteration}, prefix {}: {error}", before.len())
                });
            assert_eq!(next.inspection(), restored.inspection());
            assert_eq!(journal.records(), before);
            let progressed = next.step_with_bindings(&mut journal, &mut world).unwrap();
            restored = next;
            if !progressed || !journal.open_calls().is_empty() {
                break;
            }
        }
        restored.poll_drivers(&mut journal, &mut world).unwrap();
        assert_eq!(
            world.starts, 2,
            "only a new application can start another Call"
        );
    }
    assert_eq!(restored.outputs().len(), 1);
    let fold = LoopView::from_records(journal.records()).unwrap();
    assert!(fold.runs().all(|(_, state)| state.phase == Phase::Closed));
    assert_eq!(fold.runs().count(), 1);
}

#[test]
fn graph_call_deadlines_survive_recovery_without_extending_the_application() {
    use rastok::journal::loop_protocol::{LoopView, Phase};
    let plan = compile(GRAPH_CALLS);
    let mut vm = Execution::new_at(&plan, GraphVersion(1), 7_000_000_000);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal, "in", 1);
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    for (at, iteration, phase) in [
        (9_999_999_999, 0, Phase::Active),
        (10_000_000_000, 1, Phase::Active),
        (13_000_000_000, 1, Phase::Closed),
    ] {
        let mut restored = Execution::new_at(&plan, GraphVersion(1), at);
        let starts = world.starts;
        restored
            .recover_with_bindings(&journal, &mut world)
            .unwrap();
        assert_eq!(world.starts, starts);
        restored.advance_to(&mut journal, &mut world, at).unwrap();
        let fold = LoopView::from_records(journal.records()).unwrap();
        let (_, run) = fold.runs().next().unwrap();
        assert_eq!((run.iteration, run.phase), (iteration, phase), "at {at}");
        let retained = journal.records().to_vec();
        let mut checked = Execution::new_at(&plan, GraphVersion(1), at);
        checked.recover_with_bindings(&journal, &mut world).unwrap();
        assert_eq!(checked.inspection(), restored.inspection());
        assert_eq!(journal.records(), retained);
    }
    assert_eq!(
        journal
            .records()
            .iter()
            .filter(|record| matches!(record,
        Record::Egress(rastok::journal::EgressRecord::Call(call))
        if call.status == rastok::journal::CallStatus::Closed(rastok::journal::Closure::Timeout)))
            .count(),
        2
    );
}

#[test]
fn a_graph_call_from_another_instance_does_not_delay_run_quiescence() {
    use rastok::journal::loop_protocol::{LoopView, Phase};
    let source = GRAPH_CALLS.replace(
        "graph cycle : Loop {",
        "graph cycle : Loop {\n    instance = { key = data }",
    );
    let plan = compile(&source);
    let mut vm = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal, "in", 1);
    inject(&mut vm, &mut journal, "in", 0);
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    assert_eq!(world.starts, 2);
    let fold = LoopView::from_records(journal.records()).unwrap();
    let (finished_run, state) = fold
        .runs()
        .find(|(_, run)| run.state.data() == &Data::materialized(Value::Int(0)))
        .unwrap();
    let call = journal
        .open_calls()
        .iter()
        .find(|call| call.emitting_id == state.state.meta().sys().id())
        .unwrap()
        .call_id;
    world.answers.push(rastok::boundary::Response {
        call_id: call,
        completion: rastok::boundary::Completion::process(
            vec![],
            vec![],
            rastok::boundary::ProcessExit::Exited {
                exit_code: 0,
                signal: None,
            },
        ),
    });
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    let fold = LoopView::from_records(journal.records()).unwrap();
    assert_eq!(fold.run(*finished_run).unwrap().phase, Phase::Closed);
    assert_eq!(
        fold.runs()
            .filter(|(_, run)| run.phase == Phase::Active && run.pending.is_some())
            .count(),
        1
    );
    assert_eq!(journal.open_calls().len(), 1);
    let retained = journal.records().to_vec();
    let mut restored = Execution::new(&plan, GraphVersion(1));
    restored
        .recover_with_bindings(&journal, &mut world)
        .unwrap();
    restored.poll_drivers(&mut journal, &mut world).unwrap();
    assert_eq!(world.starts, 2);
    assert_eq!(restored.inspection(), vm.inspection());
    assert_eq!(journal.records(), retained);
}

#[test]
fn closing_a_graph_call_run_cancels_recurrence_and_recovers_settlement() {
    use rastok::journal::loop_protocol::{LoopView, Phase};
    let source = GRAPH_CALLS
        .replacen("graph app : App {", "graph app : App {\n  lifetime = { kind = fixed; after = duration(\"2s\") }\n  closing = { drain_for = duration(\"1s\"); settle_for = duration(\"1s\") }", 1)
        .replace("duration(\"3s\")", "duration(\"10s\")");
    let plan = compile(&source);
    let mut vm = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal, "in", 1);
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    for at in [2_000_000_000, 3_000_000_000, 4_000_000_000, 20_000_000_000] {
        vm.advance_to(&mut journal, &mut world, at).unwrap();
        let mut restored = Execution::new_at(&plan, GraphVersion(1), at);
        restored
            .recover_with_bindings(&journal, &mut world)
            .unwrap_or_else(|error| panic!("at {at}: {error}"));
        // Recovery reconstructs due cancellation work; it performs no Driver
        // action. Processing that same instant reaches the settling phase.
        let retained = journal.records().to_vec();
        restored.advance_to(&mut journal, &mut world, at).unwrap();
        assert_eq!(journal.records(), retained);
        assert_eq!(restored.inspection(), vm.inspection(), "at {at}");
        vm = restored;
    }
    assert_eq!(
        world.starts, 1,
        "Closing never admits the proposed recurrence"
    );
    assert!(vm.outputs().is_empty());
    assert!(vm.is_stable());
    let fold = LoopView::from_records(journal.records()).unwrap();
    assert!(
        fold.runs()
            .all(|(_, run)| run.phase == Phase::Closed && run.iteration == 0)
    );
}

#[test]
fn call_recovery_restores_the_held_delivery_without_restarting_the_driver() {
    let plan = compile(CALL_LOOP);
    let mut vm = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal, "in", 4);
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    assert_eq!(world.starts, 1);
    let before = journal.records().to_vec();
    let mut restored = Execution::new_at(&plan, GraphVersion(1), 750_000_000);
    restored.recover_from_journal(&journal).unwrap();
    assert_eq!(restored.inspection().calls, vm.inspection().calls);
    restored.poll_drivers(&mut journal, &mut world).unwrap();
    assert_eq!(
        world.starts, 1,
        "recovery must never blindly repeat an effect"
    );
    assert_eq!(journal.records(), before);

    let call = journal.open_calls()[0].call_id;
    world.answers.push(rastok::boundary::Response {
        call_id: call,
        completion: rastok::boundary::Completion::process(
            vec![],
            vec![],
            rastok::boundary::ProcessExit::TimeoutExpired,
        ),
    });
    restored.poll_drivers(&mut journal, &mut world).unwrap();
    let retained = journal.records().to_vec();
    let mut after_response = Execution::new_at(&plan, GraphVersion(1), 750_000_000);
    after_response.recover_from_journal(&journal).unwrap();
    assert_eq!(
        after_response.inspection().calls,
        restored.inspection().calls
    );
    assert!(after_response.inspection().calls[0].completion_recorded);
    assert_eq!(journal.records(), retained);
    after_response.step(&mut journal).unwrap(); // completion produces Again
    let retained = journal.records().to_vec();
    let mut after_command = Execution::new_at(&plan, GraphVersion(1), 750_000_000);
    after_command.recover_from_journal(&journal).unwrap();
    assert!(after_command.inspection().calls.is_empty());
    assert_eq!(journal.records(), retained);
    after_command
        .run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    assert_eq!(world.starts, 1);
    after_command
        .advance_to(&mut journal, &mut world, 1_750_000_000)
        .unwrap();
    after_command
        .poll_drivers(&mut journal, &mut world)
        .unwrap();
    assert_eq!(
        world.starts, 2,
        "only the new recurrence may start a new effect"
    );
}

#[test]
fn delivery_deadlines_are_retained_at_both_ingress_and_producer_commits() {
    for direct in [false, true] {
        let source = if direct {
            DELIVERY_DEADLINES
                .replace("in -> forward", "in -> { pair.left; call.in }")
                .replace(
                    "forward.out -> { pair.left; call.in }",
                    "forward.out -> nothing",
                )
        } else {
            DELIVERY_DEADLINES.into()
        };
        let plan = compile(&source);
        let mut vm = Execution::new_at(&plan, GraphVersion(1), 7_000_000_000);
        let mut journal = MemJournal::new(GraphVersion(1));
        inject(&mut vm, &mut journal, "in", 4);
        if !direct {
            while !journal.records().iter().any(|record| {
                matches!(record,
                Record::Transaction(record) if !record.emissions.is_empty())
            }) {
                assert!(vm.step(&mut journal).unwrap());
            }
        }
        let (schedule, routes): (_, Vec<_>) = if direct {
            let Record::Ingress(record) = &journal.records()[0] else {
                panic!("ingress")
            };
            (
                record.deadlines.as_deref(),
                record
                    .routes
                    .iter()
                    .map(|route| (route.edge.as_str(), record.envelope.meta().sys().id()))
                    .collect(),
            )
        } else {
            let Record::Transaction(record) = journal.records().last().unwrap() else {
                panic!("producer")
            };
            (
                record.deadlines.as_deref(),
                record
                    .routes
                    .iter()
                    .map(|route| (route.edge.as_str(), route.delivery))
                    .collect(),
            )
        };
        let schedule = schedule.expect("every delivery deadline has a retained due instant");
        assert_eq!(schedule.created_at(), 7_000_000_000);
        assert_eq!(schedule.items().len(), 2);
        for deadline in schedule.items() {
            assert!(
                routes
                    .iter()
                    .any(|(edge, envelope)| ids::delivery_id(edge, *envelope) == deadline.reference)
            );
            assert_eq!(
                deadline.due_at,
                match deadline.kind {
                    DeadlineKind::JoinWithin => 9_000_000_000,
                    DeadlineKind::CallTimeout => 10_000_000_000,
                    _ => panic!("unexpected deadline kind"),
                }
            );
        }
    }
}

#[test]
fn a_timed_join_recovers_its_candidates_and_original_deadline() {
    let plan = compile(
        r#"
graph app : App {
  input left: int
  input right: int
  output out: int
  output expired: union { JoinExpired {} }
  operator pair {
    input left: int
    input right: int
    output out: int
    output expired: union { JoinExpired {} }
    on (left, right) within duration("2s") { out = left.data + right.data }
  }
  left -> pair.left
  right -> pair.right
  pair.out -> out
  pair.expired -> expired
}
"#,
    );
    let mut vm = Execution::new_at(&plan, GraphVersion(1), 7_000_000_000);
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, "left", 4);
    vm.run_ready(&mut journal).unwrap();
    let mut restored = Execution::new_at(&plan, GraphVersion(1), 8_500_000_000);
    restored.recover_from_journal(&journal).unwrap();
    assert_eq!(restored.inspection().mailboxes, vm.inspection().mailboxes);
    let mut world = World::default();
    restored
        .advance_to(&mut journal, &mut world, 8_999_999_999)
        .unwrap();
    assert!(restored.outputs().is_empty());
    restored
        .advance_to(&mut journal, &mut world, 9_000_000_000)
        .unwrap();
    assert_eq!(restored.outputs().len(), 1);
    assert_eq!(restored.outputs()[0].port.name(), "expired");
    let before = journal.records().to_vec();
    let mut after_eviction = Execution::new_at(&plan, GraphVersion(1), 10_000_000_000);
    after_eviction.recover_from_journal(&journal).unwrap();
    after_eviction
        .advance_to(&mut journal, &mut world, 10_000_000_000)
        .unwrap();
    assert_eq!(
        journal.records(),
        before,
        "a consumed candidate never expires twice"
    );
    assert_eq!(after_eviction.outputs(), restored.outputs());
    inject(&mut after_eviction, &mut journal, "left", 5);
    inject(&mut after_eviction, &mut journal, "right", 6);
    after_eviction.run_ready(&mut journal).unwrap();
    assert_eq!(
        after_eviction.outputs().last().unwrap().envelope.data(),
        &Data::materialized(Value::Int(11))
    );
    let mut after_join = Execution::new_at(&plan, GraphVersion(1), 10_000_000_000);
    after_join.recover_from_journal(&journal).unwrap();
    assert_eq!(after_join.outputs(), after_eviction.outputs());
    assert!(
        after_join
            .inspection()
            .mailboxes
            .iter()
            .all(|mailbox| mailbox.deliveries.is_empty())
    );
}

#[test]
fn scheduler_closed_calls_recover_through_pure_contract_projection_only() {
    for abandoned in [false, true] {
        let plan = compile(CALL_LOOP);
        let mut vm = Execution::new(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut world = World::default();
        inject(&mut vm, &mut journal, "in", 4);
        vm.run_ready_with_bindings(&mut journal, &mut world)
            .unwrap();
        let call = journal.open_calls()[0].call_id;
        if abandoned {
            journal.abandon_egress(&call, 0).unwrap();
        } else {
            journal
                .close_egress(&call, rastok::journal::Closure::Timeout, 0, None)
                .unwrap();
        }
        let before = journal.records().to_vec();
        let mut restored = Execution::new_at(&plan, GraphVersion(1), 3_000_000_000);
        let empty = restored.inspection();
        assert!(restored.recover_from_journal(&journal).is_err());
        assert_eq!(
            restored.inspection(),
            empty,
            "missing binding refuses atomically"
        );
        restored
            .recover_with_bindings(&journal, &mut world)
            .unwrap();
        assert_eq!(world.starts, 0);
        assert_eq!(journal.records(), before);
        assert!(restored.inspection().calls[0].completion_recorded);
        restored.step(&mut journal).unwrap();
        assert!(restored.inspection().calls.is_empty());
        let mut after = Execution::new_at(&plan, GraphVersion(1), 3_000_000_000);
        after.recover_with_bindings(&journal, &mut world).unwrap();
        assert!(after.inspection().calls.is_empty());
        assert_eq!(world.starts, 0);
    }
}

#[test]
fn recovery_uses_each_admission_prefix_across_closed_generations() {
    let plan = compile(
        r#"
graph app : App {
  input in: int
  output out: int
  output closed: int
  operator counter {
    input in: int
    output out: int
    output closed: int
    var count: int = 0
    on in { count = count + data; out = count; stop }
    on stop { closed = count }
  }
  in -> counter
  counter.out -> out
  counter.closed -> closed
}

"#,
    );
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Execution::new(&plan, GraphVersion(1));
    for value in [3, 7, 11] {
        inject(&mut vm, &mut journal, "in", value);
        loop {
            let prefix = journal.records().to_vec();
            let mut recovered = Execution::new(&plan, GraphVersion(1));
            recovered.recover_from_journal(&journal).unwrap();
            assert_eq!(journal.records(), prefix, "recovery appends nothing");
            assert_eq!(
                recovered.inspection(),
                vm.inspection(),
                "prefix {}",
                prefix.len()
            );
            if !vm.step(&mut journal).unwrap() {
                break;
            }
        }
    }
    let values: Vec<_> = vm
        .outputs()
        .iter()
        .map(|output| output.envelope.data().clone())
        .collect();
    assert_eq!(
        values,
        [3, 3, 7, 7, 11, 11]
            .into_iter()
            .map(|n| Data::materialized(Value::Int(n)))
            .collect::<Vec<_>>()
    );
}

#[test]
fn graph_window_stop_products_reach_live_siblings_before_parent_stop() {
    for (aggregate, formatter, bridge, lifetime) in [
        ("a_total", "z_print", false, "fixed"),
        ("z_total", "a_print", false, "fixed"),
        ("z_total", "a_print", true, "fixed"),
        ("z_total", "a_print", true, "tumbling"),
    ] {
        let bound = if lifetime == "tumbling" {
            "every"
        } else {
            "after"
        };
        let bridge_decl = if bridge {
            r#"graph bridge {
              input in: int
              output out: int
              operator relay {
                input in: int
                output out: int
                on in { out = data }
              }
              in -> relay -> out
            }"#
        } else {
            ""
        };
        let bridge_route = if bridge { "bridge -> " } else { "" };
        let plan = compile(&format!(
            r#"
graph app : App {{
  input in: int
  output out: int
  graph window {{
    input in: int
    output out: int
    lifetime = {{ kind = {lifetime}; {bound} = duration("3s") }}
    operator {aggregate} {{
      input in: int
      output out: int
      var sum: int = 0
      on in {{ sum = sum + data }}
      on stop {{ out = sum }}
    }}
    operator {formatter} {{
      input seed: int
      input in: int
      output out: int
      var prefix: int = 0
      on seed {{ prefix = prefix + 10 }}
      on in {{ out = data + prefix }}
    }}
    in -> {{ {aggregate}; {formatter}.seed }}
    {bridge_decl}
    {aggregate} -> {bridge_route}{formatter} -> out
  }}
  in -> window -> out
}}
"#
        ));
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Execution::new(&plan, GraphVersion(1));
        let mut world = World::default();
        inject(&mut vm, &mut journal, "in", 3);
        inject(&mut vm, &mut journal, "in", 4);
        vm.run_ready(&mut journal).unwrap();
        assert!(vm.outputs().is_empty());
        let mut recovered = Execution::new(&plan, GraphVersion(1));
        recovered.recover_from_journal(&journal).unwrap();
        recovered
            .advance_to(&mut journal, &mut world, 3_000_000_000)
            .unwrap();
        assert_eq!(recovered.outputs().len(), 1, "{aggregate} -> {formatter}");
        assert_eq!(
            recovered.outputs()[0].envelope.data(),
            &Data::materialized(Value::Int(27)),
            "the existing formatter state must survive until the upstream stop product"
        );
        let stops: Vec<_> = journal
            .records()
            .iter()
            .filter_map(|record| match record {
                Record::Transaction(record)
                    if record.trigger
                        == rastok::node::Trigger::Lifecycle(rastok::node::Lifecycle::Stop) =>
                {
                    Some(record.owner_path.to_string())
                }
                _ => None,
            })
            .collect();
        let at = |name: &str| stops.iter().position(|path| path == name).unwrap();
        assert!(at(&format!("window.{aggregate}")) < at(&format!("window.{formatter}")));
        assert!(at(&format!("window.{formatter}")) < at("window"));
        assert_eq!(journal.records().iter().filter(|record| matches!(record,
            Record::Transaction(record)
                if record.owner_path.to_string() == format!("window.{formatter}")
                && record.trigger == rastok::node::Trigger::Lifecycle(rastok::node::Lifecycle::Start)
        )).count(), 1, "a closing receiver is not recreated in a successor generation");
        let retained = journal.records().to_vec();
        let mut closed = Execution::new_at(&plan, GraphVersion(1), 3_000_000_000);
        closed.recover_from_journal(&journal).unwrap();
        closed
            .advance_to(&mut journal, &mut world, 4_000_000_000)
            .unwrap();
        assert_eq!(journal.records(), retained);
        assert_eq!(closed.outputs(), recovered.outputs());
    }
}

#[test]
fn lifetimes_recover_the_original_admission_deadline_and_idle_renewals() {
    for kind in ["idle", "fixed", "tumbling"] {
        let attribute = if kind == "tumbling" { "every" } else { "after" };
        let plan = compile(&format!(
            r#"
graph app : App {{
  input in: int
  output out: int
  operator total {{
    input in: int
    output out: int
    lifetime = {{ kind = {kind}; {attribute} = duration("3s") }}
    var sum: int = 0
    on in {{ sum = sum + data }}
    on stop {{ out = sum }}
  }}
  in -> total -> out
}}
"#
        ));
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Execution::new_at(&plan, GraphVersion(1), 7_000_000_000);
        let mut world = World::default();
        inject(&mut vm, &mut journal, "in", 3);
        let schedule = journal.records()[0]
            .deadlines()
            .expect("lifetime is dated at admission");
        let first_due = if kind == "tumbling" {
            9_000_000_000
        } else {
            10_000_000_000
        };
        assert_eq!(schedule.created_at(), 7_000_000_000);
        assert_eq!(schedule.items()[0].due_at, first_due);
        vm.advance_to(&mut journal, &mut world, 8_000_000_000)
            .unwrap();
        inject(&mut vm, &mut journal, "in", 4);
        vm.run_ready(&mut journal).unwrap();
        let due = if kind == "idle" {
            11_000_000_000
        } else {
            first_due
        };
        let mut recovered = Execution::new_at(&plan, GraphVersion(1), 8_500_000_000);
        recovered.recover_from_journal(&journal).unwrap();
        recovered
            .advance_to(&mut journal, &mut world, due - 1)
            .unwrap();
        assert!(
            recovered.outputs().is_empty(),
            "{kind}: not before original deadline"
        );
        recovered.advance_to(&mut journal, &mut world, due).unwrap();
        assert_eq!(recovered.outputs().len(), 1, "{kind}");
        assert_eq!(
            recovered.outputs()[0].envelope.data(),
            &Data::materialized(Value::Int(7))
        );
        let prefix = journal.records().to_vec();
        let mut closed = Execution::new_at(&plan, GraphVersion(1), due);
        closed.recover_from_journal(&journal).unwrap();
        closed
            .advance_to(&mut journal, &mut world, due + 1)
            .unwrap();
        assert_eq!(
            journal.records(),
            prefix,
            "{kind}: an expired generation never closes twice"
        );
    }
}
