//! Recovery consumes retained facts, never a second execution of producers.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::boundary::{Draft, InboundEnvelopeProvenance, InboundSubmission, SubmissionSequence};
use rastok::compiler;
use rastok::contract::ContractCatalog;
use rastok::envelope::{Data, Namespace, Value};
use rastok::journal::{Journal, MemJournal, Record, ids, timer_kind};
use rastok::plan::{self, GraphVersion};
use rastok::runtime::vm::Vm;

#[test]
fn an_overdue_queued_call_cannot_start_an_effect_after_recovery() {
    let plan = compile(CALL_LOOP);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&plan, GraphVersion(1));
    inject(&mut vm, &mut journal, "in", 3);
    let mut world = World::default();
    let mut recovered = Vm::boot_at(&plan, GraphVersion(1), 4_000_000_000);
    recovered
        .recover_with_bindings(&journal, &mut world)
        .unwrap();
    recovered
        .drain_with_bindings(&mut journal, &mut world)
        .unwrap();
    recovered.pump(&mut journal, &mut world).unwrap();
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
    effect = idempotent; behavior = deterministic
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
        let mut vm = Vm::boot_at(&plan, GraphVersion(1), 7_000_000_000);
        let mut world = World::default();
        inject(&mut vm, &mut journal, "in", 3);
        if queued {
            inject(&mut vm, &mut journal, "in", 4);
        }
        vm.drain_with_bindings(&mut journal, &mut world).unwrap();
        vm.pump(&mut journal, &mut world).unwrap();
        assert_eq!(world.starts, 1);
        for at in [8_000_000_000, 9_000_000_000, 10_000_000_000, 11_000_000_000] {
            vm.advance(&mut journal, &mut world, at).unwrap();
            let retained = journal.records().to_vec();
            let mut recovered = Vm::boot_at(&plan, GraphVersion(1), at);
            recovered
                .recover_with_bindings(&journal, &mut world)
                .unwrap_or_else(|error| panic!("queued={queued}, at={at}: {error}"));
            assert_eq!(world.starts, 1, "recovery starts no effect");
            assert_eq!(journal.records(), retained);
            recovered.advance(&mut journal, &mut world, at).unwrap();
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

fn inject(vm: &mut Vm<'_>, journal: &mut MemJournal, port: &str, value: i64) {
    vm.inject(
        journal,
        InboundSubmission {
            provenance: InboundEnvelopeProvenance::single(
                rastok::node::Path::new(["recovery-test"]).unwrap(),
            ),
            port: rastok::node::Port::new(port).unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Int(value)),
                ext: Namespace::new(),
            },
            sequence: SubmissionSequence::Next,
        },
    )
    .unwrap();
}

const DELIVERY_TIMERS: &str = r#"
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
    effect = idempotent
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
        _: &rastok::boundary::Intent,
        _: &rastok::envelope::Envelope,
    ) -> rastok::boundary::CallStart {
        self.starts += 1;
        rastok::boundary::CallStart::Started
    }

    fn events(&mut self) -> Vec<rastok::boundary::Response> {
        std::mem::take(&mut self.answers)
    }
}

const CALL_LOOP: &str = r#"
graph app : App {
  input in: int
  output out: variant
  connector call : Process + Loop {
    command = "cat"
    codec = jsonl
    effect = idempotent
    behavior = deterministic
    timeout = duration("3s")
    Loop.fuel = 2
    Loop.delay = { kind = fixed; after = duration("1s") }
  }
  in -> call
  call.out -> out
  call.err -> nothing
  call.exit -> match { TimeoutExpired -> call.Loop.again; _ -> nothing }
  call.Loop.expired -> nothing
}
"#;

#[test]
fn call_recovery_restores_the_held_delivery_without_restarting_the_driver() {
    let plan = compile(CALL_LOOP);
    let mut vm = Vm::boot(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal, "in", 4);
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    assert_eq!(world.starts, 1);
    let before = journal.records().to_vec();
    let mut restored = Vm::boot_at(&plan, GraphVersion(1), 750_000_000);
    restored.recover_from_journal(&journal).unwrap();
    assert_eq!(restored.inspection().calls, vm.inspection().calls);
    restored.pump(&mut journal, &mut world).unwrap();
    assert_eq!(
        world.starts, 1,
        "recovery must never blindly repeat an effect"
    );
    assert_eq!(journal.records(), before);

    let call = journal.open_calls()[0].call_ref;
    world.answers.push(rastok::boundary::Response {
        call_ref: call,
        completion: rastok::boundary::Completion::process(
            vec![],
            vec![],
            rastok::boundary::ProcessExit::TimeoutExpired,
        ),
    });
    restored.pump(&mut journal, &mut world).unwrap();
    let retained = journal.records().to_vec();
    let mut after_response = Vm::boot_at(&plan, GraphVersion(1), 750_000_000);
    after_response.recover_from_journal(&journal).unwrap();
    assert_eq!(
        after_response.inspection().calls,
        restored.inspection().calls
    );
    assert!(after_response.inspection().calls[0].completion_retained);
    assert_eq!(journal.records(), retained);
    after_response.step(&mut journal).unwrap(); // completion produces Again
    let retained = journal.records().to_vec();
    let mut after_command = Vm::boot_at(&plan, GraphVersion(1), 750_000_000);
    after_command.recover_from_journal(&journal).unwrap();
    assert!(after_command.inspection().calls.is_empty());
    assert_eq!(journal.records(), retained);
    after_command
        .drain_with_bindings(&mut journal, &mut world)
        .unwrap();
    assert_eq!(world.starts, 1);
    after_command
        .advance(&mut journal, &mut world, 1_750_000_000)
        .unwrap();
    after_command.pump(&mut journal, &mut world).unwrap();
    assert_eq!(
        world.starts, 2,
        "only the new recurrence may start a new effect"
    );
}

#[test]
fn delivery_deadlines_are_retained_at_both_ingress_and_producer_commits() {
    for direct in [false, true] {
        let source = if direct {
            DELIVERY_TIMERS
                .replace("in -> forward", "in -> { pair.left; call.in }")
                .replace(
                    "forward.out -> { pair.left; call.in }",
                    "forward.out -> nothing",
                )
        } else {
            DELIVERY_TIMERS.into()
        };
        let plan = compile(&source);
        let mut vm = Vm::boot_at(&plan, GraphVersion(1), 7_000_000_000);
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
            let Record::Inbound(record) = &journal.records()[0] else {
                panic!("ingress")
            };
            (
                record.timer_schedule.as_deref(),
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
                record.timer_schedule.as_deref(),
                record
                    .routes
                    .iter()
                    .map(|route| (route.edge.as_str(), route.delivery))
                    .collect(),
            )
        };
        let schedule = schedule.expect("every delivery timer has a retained due instant");
        assert_eq!(schedule.armed_at(), 7_000_000_000);
        assert_eq!(schedule.arms().len(), 2);
        for arm in schedule.arms() {
            assert!(
                routes
                    .iter()
                    .any(|(edge, envelope)| ids::delivery_ref(edge, *envelope) == arm.reference)
            );
            assert_eq!(
                arm.due_at,
                match arm.kind {
                    timer_kind::JOIN_WITHIN => 9_000_000_000,
                    timer_kind::CALL_TIMEOUT => 10_000_000_000,
                    _ => panic!("unexpected timer kind"),
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
    let mut vm = Vm::boot_at(&plan, GraphVersion(1), 7_000_000_000);
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, "left", 4);
    vm.drain(&mut journal).unwrap();
    let mut restored = Vm::boot_at(&plan, GraphVersion(1), 8_500_000_000);
    restored.recover_from_journal(&journal).unwrap();
    assert_eq!(restored.inspection().mailboxes, vm.inspection().mailboxes);
    let mut world = World::default();
    restored
        .advance(&mut journal, &mut world, 8_999_999_999)
        .unwrap();
    assert!(restored.outputs().is_empty());
    restored
        .advance(&mut journal, &mut world, 9_000_000_000)
        .unwrap();
    assert_eq!(restored.outputs().len(), 1);
    assert_eq!(restored.outputs()[0].port.name(), "expired");
    let before = journal.records().to_vec();
    let mut after_eviction = Vm::boot_at(&plan, GraphVersion(1), 10_000_000_000);
    after_eviction.recover_from_journal(&journal).unwrap();
    after_eviction
        .advance(&mut journal, &mut world, 10_000_000_000)
        .unwrap();
    assert_eq!(
        journal.records(),
        before,
        "a consumed candidate never expires twice"
    );
    assert_eq!(after_eviction.outputs(), restored.outputs());
    inject(&mut after_eviction, &mut journal, "left", 5);
    inject(&mut after_eviction, &mut journal, "right", 6);
    after_eviction.drain(&mut journal).unwrap();
    assert_eq!(
        after_eviction.outputs().last().unwrap().envelope.data(),
        &Data::materialized(Value::Int(11))
    );
    let mut after_join = Vm::boot_at(&plan, GraphVersion(1), 10_000_000_000);
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
        let mut vm = Vm::boot(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut world = World::default();
        inject(&mut vm, &mut journal, "in", 4);
        vm.drain_with_bindings(&mut journal, &mut world).unwrap();
        let call = journal.open_calls()[0].call_ref;
        if abandoned {
            journal.abandon_outbound(&call).unwrap();
        } else {
            journal
                .close_outbound(&call, rastok::journal::Closure::Timeout)
                .unwrap();
        }
        let before = journal.records().to_vec();
        let mut restored = Vm::boot_at(&plan, GraphVersion(1), 3_000_000_000);
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
        assert!(restored.inspection().calls[0].completion_retained);
        restored.step(&mut journal).unwrap();
        assert!(restored.inspection().calls.is_empty());
        let mut after = Vm::boot_at(&plan, GraphVersion(1), 3_000_000_000);
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
    let mut vm = Vm::boot(&plan, GraphVersion(1));
    for value in [3, 7, 11] {
        inject(&mut vm, &mut journal, "in", value);
        loop {
            let prefix = journal.records().to_vec();
            let mut recovered = Vm::boot(&plan, GraphVersion(1));
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
        let mut vm = Vm::boot(&plan, GraphVersion(1));
        let mut world = World::default();
        inject(&mut vm, &mut journal, "in", 3);
        inject(&mut vm, &mut journal, "in", 4);
        vm.drain(&mut journal).unwrap();
        assert!(vm.outputs().is_empty());
        let mut recovered = Vm::boot(&plan, GraphVersion(1));
        recovered.recover_from_journal(&journal).unwrap();
        recovered
            .advance(&mut journal, &mut world, 3_000_000_000)
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
        let mut closed = Vm::boot_at(&plan, GraphVersion(1), 3_000_000_000);
        closed.recover_from_journal(&journal).unwrap();
        closed
            .advance(&mut journal, &mut world, 4_000_000_000)
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
        let mut vm = Vm::boot_at(&plan, GraphVersion(1), 7_000_000_000);
        let mut world = World::default();
        inject(&mut vm, &mut journal, "in", 3);
        let schedule = journal.records()[0]
            .timer_schedule()
            .expect("lifetime is dated at admission");
        let first_due = if kind == "tumbling" {
            9_000_000_000
        } else {
            10_000_000_000
        };
        assert_eq!(schedule.armed_at(), 7_000_000_000);
        assert_eq!(schedule.arms()[0].due_at, first_due);
        vm.advance(&mut journal, &mut world, 8_000_000_000).unwrap();
        inject(&mut vm, &mut journal, "in", 4);
        vm.drain(&mut journal).unwrap();
        let due = if kind == "idle" {
            11_000_000_000
        } else {
            first_due
        };
        let mut recovered = Vm::boot_at(&plan, GraphVersion(1), 8_500_000_000);
        recovered.recover_from_journal(&journal).unwrap();
        recovered
            .advance(&mut journal, &mut world, due - 1)
            .unwrap();
        assert!(
            recovered.outputs().is_empty(),
            "{kind}: not before original deadline"
        );
        recovered.advance(&mut journal, &mut world, due).unwrap();
        assert_eq!(recovered.outputs().len(), 1, "{kind}");
        assert_eq!(
            recovered.outputs()[0].envelope.data(),
            &Data::materialized(Value::Int(7))
        );
        let prefix = journal.records().to_vec();
        let mut closed = Vm::boot_at(&plan, GraphVersion(1), due);
        closed.recover_from_journal(&journal).unwrap();
        closed.advance(&mut journal, &mut world, due + 1).unwrap();
        assert_eq!(
            journal.records(),
            prefix,
            "{kind}: an expired generation never closes twice"
        );
    }
}
