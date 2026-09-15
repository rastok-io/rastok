//! Retry executes through the ordinary Worker and real journal interface;
//! the test world substitutes effects only, not recurrence or disposition.
#![allow(clippy::unwrap_used)]

use rastok::{
    boundary::*,
    compiler,
    contract::ContractCatalog,
    diagnostics::{Bag, SourceId},
    envelope::{Data, Namespace, Value},
    journal::retry_protocol::Fact,
    journal::{MemJournal, OutboundRecord, Record},
    node,
    plan::{self, GraphVersion},
    runtime::vm::Vm,
};
use std::collections::BTreeMap;

fn program(settings: &str) -> (plan::ValidatedExecutionPlan, plan::SemanticHash) {
    let source = format!(
        r#"
graph app : App {{
  input in: text
  output out: text
  output errors: text
  output exit: variant
  connector call : Process + Retry {{
    command = "cat"; codec = lines; behavior = deterministic; idempotent = true
    timeout = duration("1s"); Retry.attempts = 3
    Retry.on = [TimeoutExpired]
    Retry.delay = {{ kind = fixed; after = duration("100ms") }}
    {settings}
  }}
  in -> call -> out
  call.err -> errors; call.exit -> exit
}}
"#
    );
    let (plan, hash, _) = checked(&source);
    (plan, hash)
}

fn checked(
    source: &str,
) -> (
    plan::ValidatedExecutionPlan,
    plan::SemanticHash,
    Vec<compiler::hir::Lab>,
) {
    let catalog = ContractCatalog::try_m0().unwrap();
    let mut diagnostics = Bag::new();
    let id = SourceId::new(0);
    let parsed = compiler::parser::parse(id, source, &catalog, &mut diagnostics);
    let hir = compiler::check::check(
        &compiler::Unit {
            files: vec![(id, parsed)],
        },
        &catalog,
        &mut diagnostics,
        None,
    )
    .unwrap();
    assert!(!diagnostics.has_errors(), "{diagnostics:?}");
    let hash = compiler::lower::semantic_hash(&hir);
    let (raw, _) = compiler::lower::lower(&hir).unwrap();
    (
        plan::ValidatedExecutionPlan::try_new(raw).unwrap(),
        hash,
        hir.labs,
    )
}

#[derive(Default)]
struct World {
    preparations: usize,
    starts: Vec<Intent>,
    responses: Vec<Response>,
}

impl CallConnectorDriver for World {
    fn prepare_request(
        &mut self,
        _: &node::Path,
        data: &Data,
        _: &BTreeMap<String, Value>,
    ) -> RequestPreparation {
        self.preparations += 1;
        prepare_process_request(RequestCodec::Lines, data)
    }
    fn start(&mut self, intent: &Intent, _: &rastok::envelope::Envelope) -> CallStart {
        self.starts.push(intent.clone());
        CallStart::Started
    }
    fn events(&mut self) -> Vec<Response> {
        std::mem::take(&mut self.responses)
    }
    fn scheduler_completion(
        &mut self,
        _: &node::Path,
        terminal: SchedulerCompletion,
    ) -> Completion {
        process_scheduler_completion(terminal)
    }
}

fn inject(vm: &mut Vm<'_>, journal: &mut MemJournal) {
    vm.inject(
        journal,
        InboundSubmission {
            provenance: InboundEnvelopeProvenance::single(node::Path::new(["test"]).unwrap()),
            port: node::Port::new("in").unwrap(),
            sequence: SubmissionSequence::Next,
            draft: Draft {
                data: Data::materialized(Value::Text("request".into())),
                ext: Namespace::new(),
            },
        },
    )
    .unwrap();
}

fn reply(world: &mut World, failure: bool) {
    world.responses.push(Response {
        call_ref: world.starts.last().unwrap().call_ref,
        completion: Completion::process(
            vec![Data::materialized(Value::Text("stdout".into()))],
            vec![Data::materialized(Value::Text("stderr".into()))],
            if failure {
                ProcessExit::TimeoutExpired
            } else {
                ProcessExit::Exited {
                    exit_code: 0,
                    signal: None,
                }
            },
        ),
    });
}

#[test]
fn only_the_final_batch_is_routed_and_preparation_runs_once() {
    let (plan, hash) = program("");
    let mut vm = Vm::boot_program(&plan, hash, GraphVersion(1), 0, None);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal);
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    assert_eq!(world.preparations, 1);
    for ordinal in 1..=3 {
        assert_eq!(world.starts.len(), ordinal);
        reply(&mut world, true);
        vm.pump(&mut journal, &mut world).unwrap();
        vm.drain_with_bindings(&mut journal, &mut world).unwrap();
        if ordinal < 3 {
            assert!(vm.outputs().is_empty());
            assert!(!vm.is_stable());
            let due = ordinal as i64 * 100_000_000;
            vm.advance(&mut journal, &mut world, due - 1).unwrap();
            vm.pump(&mut journal, &mut world).unwrap();
            assert_eq!(world.starts.len(), ordinal);
            vm.advance(&mut journal, &mut world, due).unwrap();
            vm.pump(&mut journal, &mut world).unwrap();
        }
    }
    assert_eq!(
        world.preparations, 1,
        "request framing and invocation settings are pinned"
    );
    assert_eq!(
        world
            .starts
            .iter()
            .map(|call| call.logical_time)
            .collect::<Vec<_>>(),
        vec![0, 100_000_000, 200_000_000],
        "new Calls carry their own logical instant, not the original preparation time"
    );
    assert!(
        world
            .starts
            .windows(2)
            .all(|calls| calls[0].prepared == calls[1].prepared
                && calls[0].resolved_settings == calls[1].resolved_settings
                && calls[0].call_ref != calls[1].call_ref
                && calls[0].emitting_id != calls[1].emitting_id)
    );
    assert_eq!(
        vm.outputs().len(),
        3,
        "only the last stdout, stderr and terminal are visible"
    );
    assert!(vm.is_stable());
    let transactions: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(record) if record.retry_record.is_some() => Some(record),
            _ => None,
        })
        .collect();
    let held: Vec<_> = transactions
        .iter()
        .filter(|r| matches!(r.retry_record.as_ref().unwrap().facts[0], Fact::Held(_)))
        .collect();
    assert_eq!(held.len(), 2);
    assert!(
        held.iter()
            .all(|r| r.routes.is_empty() && r.emissions.len() == 3)
    );
    assert_eq!(
        transactions
            .iter()
            .filter(|r| matches!(
                r.retry_record.as_ref().unwrap().facts[0],
                Fact::Published(_)
            ))
            .count(),
        1
    );
    assert!(transactions.iter().all(|r| r.logical_time.is_none()));
    let closed: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Outbound(OutboundRecord::Call(call)) if call.closed_at.is_some() => Some(call),
            _ => None,
        })
        .collect();
    assert_eq!(closed.len(), 3);
    assert_eq!(
        closed[0].timer_schedule.as_ref().unwrap().arms()[0].due_at,
        100_000_000
    );
    assert!(closed[2].timer_schedule.is_none());
}

#[test]
fn success_publishes_once_without_waiting_for_the_retry_bound() {
    let (plan, hash) = program("");
    let mut vm = Vm::boot_program(&plan, hash, GraphVersion(1), 0, None);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal);
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    reply(&mut world, false);
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.advance(&mut journal, &mut world, 10_000_000_000)
        .unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    assert_eq!(world.starts.len(), 1);
    assert_eq!(vm.outputs().len(), 3);
    assert!(vm.is_stable());
}

#[test]
fn real_timeout_retains_its_original_backoff_and_queue_time() {
    let (plan, hash) = program("");
    let mut vm = Vm::boot_program(&plan, hash, GraphVersion(1), 0, None);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal);
    inject(&mut vm, &mut journal);
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    assert_eq!(
        world.starts.len(),
        1,
        "the second request waits in the same instance"
    );
    vm.advance(&mut journal, &mut world, 1_000_000_000).unwrap();
    assert!(vm.outputs().is_empty());
    let closures: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|r| match r {
            Record::Outbound(OutboundRecord::Call(call)) if call.closed_at.is_some() => Some(call),
            _ => None,
        })
        .collect();
    assert_eq!(
        closures.len(),
        2,
        "queue time belongs to each original request"
    );
    assert!(
        closures
            .iter()
            .all(|call| call.timer_schedule.as_ref().unwrap().arms()[0].due_at == 1_100_000_000)
    );
    vm.advance(&mut journal, &mut world, 1_099_999_999).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    assert_eq!(world.starts.len(), 1);
    vm.advance(&mut journal, &mut world, 1_100_000_000).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    assert_eq!(world.starts.len(), 2);
    assert_eq!(
        world.preparations, 2,
        "one preparation per opening operation, including the queued timeout"
    );
}

#[test]
fn closing_in_backoff_does_not_publish_a_held_failure() {
    let (plan, hash) = program(
        "lifetime = { kind = fixed; after = duration(\"50ms\") } closing = { drain_for = duration(\"100ms\"); settle_for = duration(\"100ms\") }",
    );
    let mut vm = Vm::boot_program(&plan, hash, GraphVersion(1), 0, None);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal);
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    reply(&mut world, true);
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.advance(&mut journal, &mut world, 1_000_000_000).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    assert_eq!(world.starts.len(), 1);
    assert!(vm.outputs().is_empty());
    assert!(journal.records().iter().any(|r| matches!(r,
        Record::Transaction(r) if r.retry_record.as_ref().is_some_and(|p| p.facts.iter().any(|f|
            matches!(f, Fact::Close { reason: rastok::journal::retry_protocol::CloseReason::Stopped, .. }))))));
    assert!(vm.is_stable());
}

#[test]
fn closing_drains_an_unopened_request_once_without_creating_a_retry_operation() {
    for lifetime in ["persistent", "single_pass"] {
        let source = format!(
            r#"
graph app : App {{
  input in: text
  input close: bool
  output out: text
  output errors: text
  output exit: variant
  connector call : Process + Retry {{
    command = "cat"; codec = lines; behavior = deterministic; idempotent = true
    timeout = duration("1s"); Retry.attempts = 3; Retry.on = [TimeoutExpired]
    Retry.delay = {{ kind = fixed; after = duration("100ms") }}
    lifetime = {{ kind = {lifetime} }}
    closing = {{ drain_for = duration("2s"); settle_for = duration("1s") }}
  }}
  in -> call -> out
  call.err -> errors; call.exit -> exit
  close -> call.instance.stop
}}
"#
        );
        let (plan, hash, _) = checked(&source);
        let mut vm = Vm::boot_program(&plan, hash, GraphVersion(1), 0, None);
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut world = World::default();
        inject(&mut vm, &mut journal);
        for _ in 0..16 {
            vm.step_with_bindings(&mut journal, &mut world).unwrap();
            if journal.records().iter().any(|record| {
                matches!(record,
                Record::Transaction(record) if record.owner_path.to_string() == "call"
                    && record.trigger == node::Trigger::Lifecycle(node::Lifecycle::Start))
            }) {
                break;
            }
        }
        assert!(world.starts.is_empty());
        if lifetime == "persistent" {
            vm.inject(
                &mut journal,
                InboundSubmission {
                    provenance: InboundEnvelopeProvenance::single(
                        node::Path::new(["test"]).unwrap(),
                    ),
                    port: node::Port::new("close").unwrap(),
                    sequence: SubmissionSequence::Next,
                    draft: Draft {
                        data: Data::materialized(Value::Bool(true)),
                        ext: Namespace::new(),
                    },
                },
            )
            .unwrap();
        }
        vm.drain_with_bindings(&mut journal, &mut world).unwrap();
        vm.pump(&mut journal, &mut world).unwrap();
        assert_eq!(
            world.starts.len(),
            1,
            "{lifetime}: admitted work must drain"
        );
        reply(&mut world, true);
        vm.pump(&mut journal, &mut world).unwrap();
        vm.drain_with_bindings(&mut journal, &mut world).unwrap();
        vm.advance(&mut journal, &mut world, 3_000_000_000).unwrap();
        vm.pump(&mut journal, &mut world).unwrap();
        assert_eq!(
            world.starts.len(),
            1,
            "{lifetime}: Closing forbids recurrence"
        );
        assert_eq!(world.preparations, 1);
        assert_eq!(vm.outputs().len(), 3, "the ordinary whole batch publishes");
        assert!(vm.is_stable());
        assert!(
            journal.records().iter().all(|record| match record {
                Record::Transaction(record) => record.retry_record.is_none(),
                Record::Outbound(OutboundRecord::Call(call)) => call.retry_record.is_none(),
                _ => true,
            }),
            "drain does not fabricate a Retry operation or attempt"
        );
        for cut in 0..=journal.records().len() {
            let prefix =
                MemJournal::from_records(GraphVersion(1), None, &journal.records()[..cut]).unwrap();
            let mut restored = Vm::boot_program(&plan, hash, GraphVersion(1), 3_000_000_000, None);
            let mut world = World::default();
            let result = restored.recover_with_bindings(&prefix, &mut world);
            assert!(result.is_ok(), "{lifetime}, prefix {cut}: {result:?}");
            assert_eq!(world.preparations, 0);
            assert!(world.starts.is_empty());
            if cut == journal.records().len() {
                assert_eq!(restored.outputs(), vm.outputs());
            }
        }
    }
}

#[test]
fn single_pass_drain_never_transfers_an_operation_into_the_successor_generation() {
    let (plan, hash) = program(
        "lifetime = { kind = single_pass } closing = { drain_for = duration(\"2s\"); settle_for = duration(\"1s\") }",
    );
    let mut vm = Vm::boot_program(&plan, hash, GraphVersion(1), 0, None);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal);
    // The successor is selected by the retained Closing fact, not a pending
    // lifecycle request. Before that commit, ordinary admission backpressures.
    for _ in 0..16 {
        vm.step_with_bindings(&mut journal, &mut world).unwrap();
        if journal.records().iter().any(|record| {
            matches!(record,
            Record::Transaction(record) if record.owner_path.to_string() == "call"
                && record.trigger == node::Trigger::Lifecycle(node::Lifecycle::Closing))
        }) {
            break;
        }
    }
    assert!(journal.records().iter().any(|record| matches!(record,
        Record::Transaction(record) if record.owner_path.to_string() == "call"
            && record.trigger == node::Trigger::Lifecycle(node::Lifecycle::Closing))));
    inject(&mut vm, &mut journal);
    for count in 1..=2 {
        vm.drain_with_bindings(&mut journal, &mut world).unwrap();
        vm.pump(&mut journal, &mut world).unwrap();
        assert_eq!(world.starts.len(), count, "one Call per exact generation");
        reply(&mut world, true);
        vm.pump(&mut journal, &mut world).unwrap();
    }
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.advance(&mut journal, &mut world, 3_000_000_000).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    assert_eq!(world.starts.len(), 2);
    assert_eq!(vm.outputs().len(), 6);
    assert!(vm.is_stable());
    let completions: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(record)
                if record.owner_path.to_string() == "call" && !record.emissions.is_empty() =>
            {
                Some(record)
            }
            _ => None,
        })
        .collect();
    assert_eq!(completions.len(), 2);
    assert_ne!(completions[0].instance, completions[1].instance);
    assert!(
        completions
            .iter()
            .all(|record| record.retry_record.is_none())
    );
    for cut in 0..=journal.records().len() {
        let prefix =
            MemJournal::from_records(GraphVersion(1), None, &journal.records()[..cut]).unwrap();
        let mut restored = Vm::boot_program(&plan, hash, GraphVersion(1), 3_000_000_000, None);
        let mut world = World::default();
        let result = restored.recover_with_bindings(&prefix, &mut world);
        assert!(result.is_ok(), "prefix {cut}: {result:?}");
        assert_eq!(world.preparations, 0);
        assert!(world.starts.is_empty());
    }
}

#[test]
fn recovery_cannot_bypass_retry_without_the_exact_retained_closing() {
    let (plan, hash) = program("");
    let mut vm = Vm::boot_program(&plan, hash, GraphVersion(1), 0, None);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal);
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    let forged: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(record) if record.retry_record.is_some() => None,
            Record::Outbound(OutboundRecord::Call(call)) => {
                let mut call = call.clone();
                call.retry_record = None;
                Some(Record::Outbound(OutboundRecord::Call(call)))
            }
            _ => Some(record.clone()),
        })
        .collect();
    let forged = MemJournal::from_records(GraphVersion(1), None, &forged).unwrap();
    let mut restored = Vm::boot_program(&plan, hash, GraphVersion(1), 0, None);
    let mut world = World::default();
    let error = restored
        .recover_with_bindings(&forged, &mut world)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("no operation admission or retained Closing"),
        "{error}"
    );
    assert!(restored.outputs().is_empty());
    assert_eq!(world.preparations, 0);
    assert!(world.starts.is_empty());
}

#[test]
fn every_retained_prefix_recovers_without_preparing_or_starting_an_effect() {
    let (plan, hash) = program("");
    let mut vm = Vm::boot_program(&plan, hash, GraphVersion(1), 0, None);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal);
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    reply(&mut world, true);
    vm.pump(&mut journal, &mut world).unwrap();
    vm.advance(&mut journal, &mut world, 100_000_000).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    reply(&mut world, false);
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    for cut in 0..=journal.records().len() {
        let retained =
            MemJournal::from_records(GraphVersion(1), None, &journal.records()[..cut]).unwrap();
        let mut recovery_world = World::default();
        let mut restored = Vm::boot_program(&plan, hash, GraphVersion(1), 250_000_000, None);
        let result = restored.recover_with_bindings(&retained, &mut recovery_world);
        assert!(
            result.is_ok(),
            "cut {cut}: {result:?}; last {:?}",
            retained.records().last()
        );
        assert_eq!(recovery_world.preparations, 0);
        assert!(recovery_world.starts.is_empty());
        assert_eq!(retained.records(), &journal.records()[..cut]);
        if cut == journal.records().len() {
            assert!(restored.is_stable());
            assert_eq!(restored.outputs(), vm.outputs());
        }
    }
}

#[test]
fn recovery_after_held_consumes_once_and_does_not_renew_the_backoff() {
    let (plan, hash) = program("");
    let mut vm = Vm::boot_program(&plan, hash, GraphVersion(1), 0, None);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal);
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    reply(&mut world, true);
    vm.pump(&mut journal, &mut world).unwrap();
    vm.step_with_bindings(&mut journal, &mut world).unwrap();
    assert!(
        matches!(journal.records().last(), Some(Record::Transaction(r))
        if r.retry_record.as_ref().is_some_and(|packet| matches!(packet.facts.as_slice(), [Fact::Held(_)])))
    );
    let mut restored = Vm::boot_program(&plan, hash, GraphVersion(1), 250_000_000, None);
    let mut world_after = World::default();
    restored
        .recover_with_bindings(&journal, &mut world_after)
        .unwrap();
    restored
        .drain_with_bindings(&mut journal, &mut world_after)
        .unwrap();
    restored.pump(&mut journal, &mut world_after).unwrap();
    assert_eq!(
        world_after.starts.len(),
        1,
        "due 0.1s is already past at recovery 0.25s"
    );
    assert_eq!(world_after.preparations, 0);
    assert_eq!(world_after.starts[0].prepared, world.starts[0].prepared);
    assert_ne!(world_after.starts[0].call_ref, world.starts[0].call_ref);
    reply(&mut world_after, false);
    restored.pump(&mut journal, &mut world_after).unwrap();
    restored
        .drain_with_bindings(&mut journal, &mut world_after)
        .unwrap();
    assert_eq!(restored.outputs().len(), 3);
    assert_eq!(journal.records().iter().filter(|r| matches!(r, Record::Transaction(r)
        if r.retry_record.as_ref().is_some_and(|packet| matches!(packet.facts.first(), Some(Fact::Consumed(_)))))).count(), 1);
}

#[test]
fn the_lab_observes_new_attempt_time_without_changing_the_request() {
    let (plan, hash, labs) = checked(
        r#"
graph app : App {
  input in: text
  output out: text
  connector call : Process + Retry {
    command = "cat"; codec = lines; behavior = probabilistic; idempotent = true
    timeout = duration("1s"); Retry.attempts = 2; Retry.on = [TimeoutExpired]
    Retry.delay = { kind = fixed; after = duration("100ms") }
  }
  in -> call -> out
  call.err -> nothing; call.exit -> nothing
}
lab app {
  simulation time on app {
    mock call {
      on in where now() == timestamp("1970-01-01T00:00:00Z") { out = nothing }
      on in { out = data; exit = Exited { exit_code = 0; signal = null } }
    }
    scenario retry {
      app.in = "same request"
      expect nothing app.out
      advance duration("1s")
      expect nothing app.out
      advance duration("100ms")
      expect app.out count == 1 where data == "same request"
    }
  }
}
"#,
    );
    let runs = rastok::lab::run_program(&plan, hash, GraphVersion(1), &labs[0], None);
    assert_eq!(runs.len(), 1);
    assert!(runs[0].report.passed(), "{:?}", runs[0].report);
    let first = MemJournal::from_records(GraphVersion(1), None, &runs[0].records).unwrap();
    let mut restored = Vm::boot_program(&plan, hash, GraphVersion(1), 1_100_000_000, None);
    restored
        .recover_with_bindings(&first, &mut World::default())
        .unwrap();
    assert_eq!(restored.outputs().len(), 1);
}

#[test]
fn an_enclosing_loop_waits_for_the_whole_retry_operation() {
    let (plan, hash, _) = checked(
        r#"
graph app : App + Loop {
  input in: text
  output out: text
  output err: union { LoopExpired { last_state: text } }
  Loop.fuel = 2
  connector call : Process + Retry {
    command = "cat"; codec = lines; behavior = deterministic; idempotent = true
    timeout = duration("1s"); Retry.attempts = 3; Retry.on = [TimeoutExpired]
    Retry.delay = { kind = fixed; after = duration("100ms") }
  }
  operator finish {
    input in: variant
    output out: text
    on in { out = "done" }
  }
  in -> call
  call.out -> nothing; call.err -> nothing
  call.exit -> finish -> out
}
"#,
    );
    let mut vm = Vm::boot_program(&plan, hash, GraphVersion(1), 0, None);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal);
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    reply(&mut world, true);
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    assert!(!vm.is_stable());
    assert!(vm.outputs().is_empty());
    vm.advance(&mut journal, &mut world, 100_000_000).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    reply(&mut world, false);
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    assert_eq!(vm.outputs().len(), 1);
    assert!(vm.is_stable());
    for cut in 0..=journal.records().len() {
        let prefix =
            MemJournal::from_records(GraphVersion(1), None, &journal.records()[..cut]).unwrap();
        let mut restored = Vm::boot_program(&plan, hash, GraphVersion(1), 100_000_000, None);
        let result = restored.recover_with_bindings(&prefix, &mut World::default());
        assert!(
            result.is_ok(),
            "cut {cut}: {result:?}; last {:?}",
            prefix.records().last()
        );
    }
}

#[test]
fn recovery_rejects_a_retry_publication_attached_to_an_operator() {
    let (plan, hash, _) = checked(
        r#"
graph app : App {
  input in: text
  output out: text
  connector call : Process + Retry {
    command = "cat"; codec = lines; behavior = deterministic; idempotent = true
    timeout = duration("1s"); Retry.attempts = 3; Retry.on = [TimeoutExpired]
  }
  operator observer {
    input in: variant
    output out: text
    on in { out = "observed" }
  }
  in -> call
  call.out -> nothing; call.err -> nothing
  call.exit -> observer -> out
}
"#,
    );
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot_program(&plan, hash, GraphVersion(1), 0, None);
    let mut world = World::default();
    inject(&mut vm, &mut journal);
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    reply(&mut world, false);
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    let mut records = journal.records().to_vec();
    let mut packet = records
        .iter()
        .find_map(|record| match record {
            Record::Transaction(record)
                if record.retry_record.as_ref().is_some_and(|packet| {
                    matches!(packet.facts.first(), Some(Fact::Published(_)))
                }) =>
            {
                record.retry_record.clone()
            }
            _ => None,
        })
        .unwrap();
    let Record::Transaction(operator) = records.last_mut().unwrap() else {
        panic!("operator transaction")
    };
    assert_eq!(operator.owner_path.to_string(), "observer");
    let Fact::Published(batch) = &mut packet.facts[0] else {
        panic!("published batch")
    };
    batch.emissions = operator
        .emissions
        .iter()
        .map(|emission| emission.envelope.meta().sys().id())
        .collect();
    operator.retry_record = Some(packet);
    let forged = MemJournal::from_records(GraphVersion(1), None, &records).unwrap();
    let mut restored = Vm::boot_program(&plan, hash, GraphVersion(1), 0, None);
    assert!(
        restored
            .recover_with_bindings(&forged, &mut World::default())
            .is_err(),
        "a well-shaped Retry packet is not authority to give an Operator the Call's publication"
    );
    assert!(
        restored.outputs().is_empty(),
        "refused recovery must leave the VM unchanged"
    );
}

#[test]
fn a_retry_recording_restores_held_batches_at_every_selected_prefix() {
    let (plan, hash) = program("");
    let mut vm = Vm::boot_program(&plan, hash, GraphVersion(1), 0, None);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal);
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    reply(&mut world, true);
    vm.pump(&mut journal, &mut world).unwrap();
    vm.advance(&mut journal, &mut world, 100_000_000).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    reply(&mut world, false);
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    let compilation = compiler::Compilation {
        symbols: plan::SymbolTable {
            nodes: plan
                .nodes
                .iter()
                .map(|node| node.path.to_string())
                .collect(),
        },
        plan: plan.clone(),
        hash,
        warnings: vec![],
        labs: vec![],
        source_definitions: vec![],
        scenario_definitions: vec![],
    };
    let recording =
        rastok::recording::Recording::capture(&compilation, &journal, vm.now()).unwrap();
    let bytes = recording.encode().unwrap();
    let decoded = rastok::recording::Recording::decode(&bytes).unwrap();
    assert_eq!(decoded.records(), journal.records());
    assert_eq!(
        rastok::golden::project(decoded.records()),
        rastok::golden::project(journal.records())
    );
    for prefix in 0..=decoded.records().len() {
        let (retained, inspected) = decoded.inspect(prefix).unwrap();
        assert_eq!(retained.records(), &journal.records()[..prefix]);
        if prefix == decoded.records().len() {
            assert_eq!(inspected.outputs(), vm.outputs());
        }
    }
    let mut wrong_program = Vm::boot_program(
        &plan,
        plan::SemanticHash::from_bytes([0; 32]),
        GraphVersion(1),
        vm.now(),
        None,
    );
    assert!(
        wrong_program
            .recover_with_bindings(&journal, &mut World::default())
            .is_err()
    );
    assert!(wrong_program.outputs().is_empty());
}

#[test]
fn equal_retry_deadlines_follow_the_retained_closure_order_not_operation_hashes() {
    let (plan, hash, _) = checked(
        r#"
graph app : App {
  input in: text
  connector a : Process + Retry {
    command = "cat"; codec = lines; behavior = deterministic; idempotent = true
    timeout = duration("1s"); Retry.attempts = 2; Retry.on = [TimeoutExpired]
    Retry.delay = { kind = fixed; after = duration("1ms") }
  }
  connector b : Process + Retry {
    command = "cat"; codec = lines; behavior = deterministic; idempotent = true
    timeout = duration("1s"); Retry.attempts = 2; Retry.on = [TimeoutExpired]
    Retry.delay = { kind = fixed; after = duration("1ms") }
  }
  in -> { a; b }
  a.out -> nothing; a.err -> nothing; a.exit -> nothing
  b.out -> nothing; b.err -> nothing; b.exit -> nothing
}
"#,
    );
    let mut vm = Vm::boot_program(&plan, hash, GraphVersion(1), 0, None);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal);
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    let mut openings: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Outbound(OutboundRecord::Call(call)) => {
                Some((call.retry_record.as_ref()?.context?, call.intent.call_ref))
            }
            _ => None,
        })
        .collect();
    assert_eq!(openings.len(), 2);
    openings.sort_by_key(|(attempt, _)| std::cmp::Reverse(attempt.operation));
    world.responses = openings
        .iter()
        .map(|(_, call_ref)| Response {
            call_ref: *call_ref,
            completion: Completion::process(vec![], vec![], ProcessExit::TimeoutExpired),
        })
        .collect();
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain_with_bindings(&mut journal, &mut world).unwrap();
    vm.advance(&mut journal, &mut world, 1_000_000).unwrap();
    let admissions: Vec<_> = journal
        .records()
        .iter()
        .flat_map(|record| match record {
            Record::Transaction(record) => record
                .retry_record
                .as_deref()
                .map(|packet| packet.facts.as_slice())
                .unwrap_or_default(),
            _ => &[],
        })
        .filter_map(|fact| match fact {
            Fact::Admitted { attempt, .. } if attempt.ordinal == 2 => Some(attempt.operation),
            _ => None,
        })
        .collect();
    assert_eq!(
        admissions,
        openings
            .iter()
            .map(|(attempt, _)| attempt.operation)
            .collect::<Vec<_>>()
    );
}

#[test]
fn http_server_errors_hold_the_response_until_the_final_attempt() {
    let (plan, hash, labs) = checked(
        r#"
graph app : App {
  input in: text
  output out: text
  output exit: variant
  connector call : HttpClient + Retry {
    url = "https://partner.invalid/echo"; method = "POST"; codec = text
    behavior = probabilistic; idempotent = true; timeout = duration("1s")
    Retry.attempts = 2; Retry.on = [ServerError]
    Retry.delay = { kind = fixed; after = duration("100ms") }
  }
  in -> call -> out
  call.exit -> exit
}
lab app {
  simulation http on app {
    mock call {
      on in where now() == timestamp("1970-01-01T00:00:00Z") {
        out = "try later"; exit = ServerError { status = 503 }
      }
      on in { out = "accepted"; exit = Completed { status = 200 } }
    }
    scenario repeat {
      app.in = "same operation"
      expect nothing app.out
      expect nothing app.exit
      advance duration("100ms")
      expect app.out count == 1 where data == "accepted"
      expect app.exit count == 1
      expect call.exit count == 1 where data is Completed && data.status == 200
    }
  }
}
"#,
    );
    let runs = rastok::lab::run_program(&plan, hash, GraphVersion(1), &labs[0], None);
    assert_eq!(runs.len(), 1);
    assert!(runs[0].report.passed(), "{:?}", runs[0].report);
    let held = runs[0]
        .records
        .iter()
        .find_map(|record| match record {
            Record::Transaction(record)
                if record
                    .retry_record
                    .as_deref()
                    .is_some_and(|packet| matches!(packet.facts.as_slice(), [Fact::Held(_)])) =>
            {
                Some(record)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(
        held.emissions.len(),
        2,
        "the 503 response and exit are both retained"
    );
    assert!(held.routes.is_empty());
    for cut in 0..=runs[0].records.len() {
        let prefix =
            MemJournal::from_records(GraphVersion(1), None, &runs[0].records[..cut]).unwrap();
        let mut restored = Vm::boot_program(&plan, hash, GraphVersion(1), 100_000_000, None);
        let mut world = World::default();
        restored.recover_with_bindings(&prefix, &mut world).unwrap();
        assert_eq!(world.preparations, 0);
        assert!(world.starts.is_empty());
        if cut == runs[0].records.len() {
            assert_eq!(restored.outputs().len(), 2);
        }
    }
}

#[test]
fn file_read_and_replace_retry_without_reexecuting_the_prepared_request_on_recovery() {
    for (mode, input, request, output, expected) in [
        (
            "read",
            "text",
            "\"input.bin\"",
            "out = base64! { b2s= };",
            "expect app.out count == 1",
        ),
        (
            "replace",
            "bytes",
            "base64! { b2s= }",
            "",
            "expect nothing app.out",
        ),
    ] {
        let source = format!(
            r#"
graph app : App {{
  input in: {input}
  output out: bytes
  output exit: variant
  connector call : File + Retry {{
    mode = {mode}; path = "object.bin"; idempotent = true
    timeout = duration("1s"); Retry.attempts = 2; Retry.on = [IoFailed]
    Retry.delay = {{ kind = fixed; after = duration("100ms") }}
  }}
  in -> call -> out
  call.exit -> exit
}}
lab app {{
  simulation file on app {{
    mock call {{
      on in where now() == timestamp("1970-01-01T00:00:00Z") {{ exit = IoFailed {{}} }}
      on in {{ {output} exit = Completed {{}} }}
    }}
    scenario repeat {{
      app.in = {request}
      expect nothing app.out
      expect nothing app.exit
      advance duration("100ms")
      {expected}
      expect app.exit count == 1
      expect call.exit count == 1 where data is Completed
    }}
  }}
}}
"#
        );
        let (plan, hash, labs) = checked(&source);
        let runs = rastok::lab::run_program(&plan, hash, GraphVersion(1), &labs[0], None);
        assert_eq!(runs.len(), 1);
        assert!(runs[0].report.passed(), "{mode}: {:?}", runs[0].report);
        for cut in 0..=runs[0].records.len() {
            let prefix =
                MemJournal::from_records(GraphVersion(1), None, &runs[0].records[..cut]).unwrap();
            let mut restored = Vm::boot_program(&plan, hash, GraphVersion(1), 100_000_000, None);
            let mut world = World::default();
            let result = restored.recover_with_bindings(&prefix, &mut world);
            assert!(result.is_ok(), "{mode}, prefix {cut}: {result:?}");
            assert_eq!(world.preparations, 0);
            assert!(world.starts.is_empty());
            if cut == runs[0].records.len() {
                assert_eq!(restored.outputs().len(), if mode == "read" { 2 } else { 1 });
            }
        }
    }
}
