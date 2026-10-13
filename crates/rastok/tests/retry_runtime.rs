//! Retry executes through the ordinary Worker and real journal interface;
//! the test world substitutes effects only, not recurrence or disposition.
#![allow(clippy::unwrap_used)]

use rastok::{
    boundary::*,
    compiler,
    contract::ContractCatalog,
    envelope::{Data, Namespace, Value},
    journal::retry_protocol::Fact,
    journal::{EgressRecord, MemJournal, Record},
    node,
    plan::{self, GraphVersion},
    runtime::execution::Execution,
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
    let compilation = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    (compilation.plan, compilation.hash, compilation.labs)
}

#[derive(Default)]
struct World {
    preparations: usize,
    starts: Vec<CallOpen>,
    responses: Vec<Response>,
    cancellations: Vec<CallId>,
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
    fn start(&mut self, intent: &CallOpen, _: &rastok::envelope::Envelope) -> CallStart {
        self.starts.push(intent.clone());
        CallStart::Started
    }
    fn events(&mut self) -> Vec<Response> {
        std::mem::take(&mut self.responses)
    }
    fn cancel(&mut self, call_id: &CallId) {
        self.cancellations.push(*call_id);
    }
    fn scheduler_completion(
        &mut self,
        _: &node::Path,
        terminal: SchedulerCompletion,
    ) -> Completion {
        process_scheduler_completion(terminal)
    }
}

fn inject(vm: &mut Execution<'_>, journal: &mut MemJournal) {
    vm.accept_input(
        journal,
        IngressProposal {
            provenance: IngressProvenance::single(node::Path::new(["test"]).unwrap()),
            draft: EnvelopeDraft {
                port: node::Port::new("in").unwrap(),
                data: Data::materialized(Value::Text("request".into())),
                ext: Namespace::new(),
            },
            sequence: IngressSequence::Next,
            session: None,
        },
    )
    .unwrap();
}

fn reply(world: &mut World, failure: bool) {
    world.responses.push(Response {
        call_id: world.starts.last().unwrap().call_id,
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
    let mut vm = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal);
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    assert_eq!(world.preparations, 1);
    for ordinal in 1..=3 {
        assert_eq!(world.starts.len(), ordinal);
        reply(&mut world, true);
        vm.poll_drivers(&mut journal, &mut world).unwrap();
        vm.run_ready_with_bindings(&mut journal, &mut world)
            .unwrap();
        if ordinal < 3 {
            assert!(vm.outputs().is_empty());
            assert!(!vm.is_stable());
            let due = ordinal as i64 * 100_000_000;
            vm.advance_to(&mut journal, &mut world, due - 1).unwrap();
            vm.poll_drivers(&mut journal, &mut world).unwrap();
            assert_eq!(world.starts.len(), ordinal);
            vm.advance_to(&mut journal, &mut world, due).unwrap();
            vm.poll_drivers(&mut journal, &mut world).unwrap();
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
                && calls[0].call_id != calls[1].call_id
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
            Record::Egress(EgressRecord::Call(call)) if call.closed_at.is_some() => Some(call),
            _ => None,
        })
        .collect();
    assert_eq!(closed.len(), 3);
    assert_eq!(
        closed[0].deadlines.as_ref().unwrap().items()[0].due_at,
        100_000_000
    );
    assert!(closed[2].deadlines.is_none());
}

#[test]
fn success_publishes_once_without_waiting_for_the_retry_bound() {
    let (plan, hash) = program("");
    let mut vm = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal);
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    reply(&mut world, false);
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    vm.advance_to(&mut journal, &mut world, 10_000_000_000)
        .unwrap();
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    assert_eq!(world.starts.len(), 1);
    assert_eq!(vm.outputs().len(), 3);
    assert!(vm.is_stable());
}

#[test]
fn real_timeout_retains_its_original_backoff_and_queue_time() {
    let (plan, hash) = program("");
    let mut vm = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal);
    inject(&mut vm, &mut journal);
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    assert_eq!(
        world.starts.len(),
        1,
        "the second request waits in the same instance"
    );
    vm.advance_to(&mut journal, &mut world, 1_000_000_000)
        .unwrap();
    assert!(vm.outputs().is_empty());
    let closures: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|r| match r {
            Record::Egress(EgressRecord::Call(call)) if call.closed_at.is_some() => Some(call),
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
            .all(|call| call.deadlines.as_ref().unwrap().items()[0].due_at == 1_100_000_000)
    );
    vm.advance_to(&mut journal, &mut world, 1_099_999_999)
        .unwrap();
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    assert_eq!(world.starts.len(), 1);
    vm.advance_to(&mut journal, &mut world, 1_100_000_000)
        .unwrap();
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    assert_eq!(world.starts.len(), 2);
    assert_eq!(
        world.preparations, 2,
        "one preparation per opening operation, including the queued timeout"
    );
}

#[test]
fn closing_in_backoff_does_not_publish_a_held_failure() {
    for lifetime_ms in [50, 100] {
        let (plan, hash) = program(&format!(
            "lifetime = {{ kind = fixed; after = duration(\"{lifetime_ms}ms\") }} closing = {{ drain_for = duration(\"100ms\"); settle_for = duration(\"100ms\") }}",
        ));
        let mut vm = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut world = World::default();
        inject(&mut vm, &mut journal);
        vm.run_ready_with_bindings(&mut journal, &mut world)
            .unwrap();
        vm.poll_drivers(&mut journal, &mut world).unwrap();
        reply(&mut world, true);
        vm.poll_drivers(&mut journal, &mut world).unwrap();
        vm.run_ready_with_bindings(&mut journal, &mut world)
            .unwrap();
        vm.advance_to(&mut journal, &mut world, 1_000_000_000)
            .unwrap();
        vm.poll_drivers(&mut journal, &mut world).unwrap();
        assert_eq!(world.starts.len(), 1);
        assert!(vm.outputs().is_empty());
        assert!(journal.records().iter().any(|r| matches!(r,
        Record::Transaction(r) if r.retry_record.as_ref().is_some_and(|p| p.facts.iter().any(|f|
            matches!(f, Fact::Close { reason: rastok::journal::retry_protocol::CloseReason::Stopped, .. }))))));
        assert!(vm.is_stable());
    }
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
        let mut vm = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
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
            vm.accept_input(
                &mut journal,
                IngressProposal {
                    provenance: IngressProvenance::single(node::Path::new(["test"]).unwrap()),
                    draft: EnvelopeDraft {
                        port: node::Port::new("close").unwrap(),
                        data: Data::materialized(Value::Bool(true)),
                        ext: Namespace::new(),
                    },
                    sequence: IngressSequence::Next,
                    session: None,
                },
            )
            .unwrap();
        }
        vm.run_ready_with_bindings(&mut journal, &mut world)
            .unwrap();
        vm.poll_drivers(&mut journal, &mut world).unwrap();
        assert_eq!(
            world.starts.len(),
            1,
            "{lifetime}: admitted work must drain"
        );
        reply(&mut world, true);
        vm.poll_drivers(&mut journal, &mut world).unwrap();
        vm.run_ready_with_bindings(&mut journal, &mut world)
            .unwrap();
        vm.advance_to(&mut journal, &mut world, 3_000_000_000)
            .unwrap();
        vm.poll_drivers(&mut journal, &mut world).unwrap();
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
                Record::Egress(EgressRecord::Call(call)) => call.retry_record.is_none(),
                _ => true,
            }),
            "drain does not fabricate a Retry operation or attempt"
        );
        for cut in 0..=journal.records().len() {
            let prefix =
                MemJournal::from_records(GraphVersion(1), None, &journal.records()[..cut]).unwrap();
            let mut restored =
                Execution::new_program(&plan, hash, GraphVersion(1), 3_000_000_000, None);
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
    let mut vm = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
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
        vm.run_ready_with_bindings(&mut journal, &mut world)
            .unwrap();
        vm.poll_drivers(&mut journal, &mut world).unwrap();
        assert_eq!(world.starts.len(), count, "one Call per exact generation");
        reply(&mut world, true);
        vm.poll_drivers(&mut journal, &mut world).unwrap();
    }
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    vm.advance_to(&mut journal, &mut world, 3_000_000_000)
        .unwrap();
    vm.poll_drivers(&mut journal, &mut world).unwrap();
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
        let mut restored =
            Execution::new_program(&plan, hash, GraphVersion(1), 3_000_000_000, None);
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
    let mut vm = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal);
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    let forged: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(record) if record.retry_record.is_some() => None,
            Record::Egress(EgressRecord::Call(call)) => {
                let mut call = call.clone();
                call.retry_record = None;
                Some(Record::Egress(EgressRecord::Call(call)))
            }
            _ => Some(record.clone()),
        })
        .collect();
    let forged = MemJournal::from_records(GraphVersion(1), None, &forged).unwrap();
    let mut restored = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
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
    let mut vm = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal);
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    reply(&mut world, true);
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    vm.advance_to(&mut journal, &mut world, 100_000_000)
        .unwrap();
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    reply(&mut world, false);
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    for cut in 0..=journal.records().len() {
        let retained =
            MemJournal::from_records(GraphVersion(1), None, &journal.records()[..cut]).unwrap();
        let mut recovery_world = World::default();
        let mut restored = Execution::new_program(&plan, hash, GraphVersion(1), 250_000_000, None);
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
    let mut vm = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal);
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    reply(&mut world, true);
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    vm.step_with_bindings(&mut journal, &mut world).unwrap();
    assert!(
        matches!(journal.records().last(), Some(Record::Transaction(r))
        if r.retry_record.as_ref().is_some_and(|packet| matches!(packet.facts.as_slice(), [Fact::Held(_)])))
    );
    let mut restored = Execution::new_program(&plan, hash, GraphVersion(1), 250_000_000, None);
    let mut world_after = World::default();
    restored
        .recover_with_bindings(&journal, &mut world_after)
        .unwrap();
    restored
        .run_ready_with_bindings(&mut journal, &mut world_after)
        .unwrap();
    restored
        .poll_drivers(&mut journal, &mut world_after)
        .unwrap();
    assert_eq!(
        world_after.starts.len(),
        1,
        "due 0.1s is already past at recovery 0.25s"
    );
    assert_eq!(world_after.preparations, 0);
    assert_eq!(world_after.starts[0].prepared, world.starts[0].prepared);
    assert_ne!(world_after.starts[0].call_id, world.starts[0].call_id);
    reply(&mut world_after, false);
    restored
        .poll_drivers(&mut journal, &mut world_after)
        .unwrap();
    restored
        .run_ready_with_bindings(&mut journal, &mut world_after)
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
    let mut restored = Execution::new_program(&plan, hash, GraphVersion(1), 1_100_000_000, None);
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
    let mut vm = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal);
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    reply(&mut world, true);
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    assert!(!vm.is_stable());
    assert!(vm.outputs().is_empty());
    vm.advance_to(&mut journal, &mut world, 100_000_000)
        .unwrap();
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    reply(&mut world, false);
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    assert_eq!(vm.outputs().len(), 1);
    assert!(vm.is_stable());
    for cut in 0..=journal.records().len() {
        let prefix =
            MemJournal::from_records(GraphVersion(1), None, &journal.records()[..cut]).unwrap();
        let mut restored = Execution::new_program(&plan, hash, GraphVersion(1), 100_000_000, None);
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
    let mut vm = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
    let mut world = World::default();
    inject(&mut vm, &mut journal);
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    reply(&mut world, false);
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
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
    let mut restored = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
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
    let mut vm = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal);
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    reply(&mut world, true);
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    vm.advance_to(&mut journal, &mut world, 100_000_000)
        .unwrap();
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    reply(&mut world, false);
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
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
        rastok::recording::Recording::capture(&compilation, &journal, vm.logical_time()).unwrap();
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
    let mut wrong_program = Execution::new_program(
        &plan,
        plan::SemanticHash::from_bytes([0; 32]),
        GraphVersion(1),
        vm.logical_time(),
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
    let mut vm = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    inject(&mut vm, &mut journal);
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    let mut openings: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Egress(EgressRecord::Call(call)) => {
                Some((call.retry_record.as_ref()?.context?, call.intent.call_id))
            }
            _ => None,
        })
        .collect();
    assert_eq!(openings.len(), 2);
    openings.sort_by_key(|(attempt, _)| std::cmp::Reverse(attempt.operation));
    world.responses = openings
        .iter()
        .map(|(_, call_id)| Response {
            call_id: *call_id,
            completion: Completion::process(vec![], vec![], ProcessExit::TimeoutExpired),
        })
        .collect();
    vm.poll_drivers(&mut journal, &mut world).unwrap();
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    vm.advance_to(&mut journal, &mut world, 1_000_000).unwrap();
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
fn loop_and_retry_readmissions_share_the_recorded_deadline_order_after_recovery() {
    use rastok::journal::loop_protocol::Fact as LoopFact;

    let (plan, hash, _) = checked(
        r#"
graph app : App {
  input retry: text
  input iterate: text
  input timeout: text
  output done: text
  output timed_out: variant
  connector call : Process + Retry {
    command = "cat"; codec = lines; behavior = deterministic; idempotent = true
    timeout = duration("1s"); Retry.attempts = 2; Retry.on = [TimeoutExpired]
    Retry.delay = { kind = fixed; after = duration("1ms") }
  }
  operator count : Loop {
    input in: text
    output out: text
    output err: union { LoopExpired { last_state: text } }
    Loop.fuel = 2
    Loop.delay = { kind = fixed; after = duration("1ms") }
    on in match {
      data == "request" => Loop.next = "done"
      _ => out = data
    }
  }
  connector silent : Process {
    command = "cat"; codec = lines; behavior = deterministic; idempotent = true
    timeout = duration("1ms")
  }
  retry -> call
  iterate -> count -> done
  timeout -> silent
  call.out -> nothing; call.err -> nothing; call.exit -> nothing
  count.err -> nothing
  silent.out -> nothing; silent.err -> nothing; silent.exit -> timed_out
}
"#,
    );
    for order in [
        ["retry", "iterate", "timeout"],
        ["retry", "timeout", "iterate"],
        ["iterate", "retry", "timeout"],
        ["iterate", "timeout", "retry"],
        ["timeout", "retry", "iterate"],
        ["timeout", "iterate", "retry"],
    ] {
        let mut vm = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut world = World::default();
        for port in order {
            vm.accept_input(
                &mut journal,
                IngressProposal {
                    provenance: IngressProvenance::single(node::Path::new(["test"]).unwrap()),
                    draft: EnvelopeDraft {
                        port: node::Port::new(port).unwrap(),
                        data: Data::materialized(Value::Text("request".into())),
                        ext: Namespace::new(),
                    },
                    sequence: IngressSequence::Next,
                    session: None,
                },
            )
            .unwrap();
            vm.run_ready_with_bindings(&mut journal, &mut world)
                .unwrap();
            if port == "retry" {
                vm.poll_drivers(&mut journal, &mut world).unwrap();
                reply(&mut world, true);
                vm.poll_drivers(&mut journal, &mut world).unwrap();
                vm.run_ready_with_bindings(&mut journal, &mut world)
                    .unwrap();
            }
        }
        let before = journal.records().len();
        vm.advance_to(&mut journal, &mut world, 1_000_000).unwrap();
        let admissions: Vec<_> = journal.records()[before..]
            .iter()
            .filter_map(|record| {
                let Record::Transaction(record) = record else {
                    return None;
                };
                let admitted = record.loop_protocol().is_some_and(|packet| {
                    packet
                        .facts
                        .iter()
                        .any(|fact| matches!(fact, LoopFact::Admit { iteration: 1, .. }))
                }) || record.retry_record.as_deref().is_some_and(|packet| {
                    packet.facts.iter().any(|fact| {
                        matches!(fact, Fact::Admitted { attempt, .. } if attempt.ordinal == 2)
                    })
                });
                admitted.then(|| record.owner_path.to_string())
            })
            .collect();
        let expected: Vec<_> = order
            .into_iter()
            .filter_map(|port| match port {
                "retry" => Some("call"),
                "iterate" => Some("count"),
                _ => None,
            })
            .collect();
        assert_eq!(admissions, expected, "creation order {order:?}");
        assert_eq!(
            vm.outputs().len(),
            2,
            "countdown and Call timeout both completed"
        );

        for (cut, resume_at) in std::iter::once((before, 0))
            .chain((before..=journal.records().len()).map(|cut| (cut, 1_000_000)))
        {
            let mut retained =
                MemJournal::from_records(GraphVersion(1), None, &journal.records()[..cut]).unwrap();
            let mut restored =
                Execution::new_program(&plan, hash, GraphVersion(1), resume_at, None);
            let mut resumed_world = World::default();
            let already_closed = retained.records().iter().any(|record| {
                matches!(record, Record::Egress(EgressRecord::Call(call))
                    if call.intent.connector_path.to_string() == "silent"
                        && call.status != rastok::journal::CallStatus::Open)
            });
            restored
                .recover_with_bindings(&retained, &mut resumed_world)
                .unwrap();
            restored
                .advance_to(&mut retained, &mut resumed_world, 1_000_000)
                .unwrap();
            assert_eq!(retained.records().len(), journal.records().len());
            for (index, (actual, expected)) in
                retained.records().iter().zip(journal.records()).enumerate()
            {
                assert_eq!(
                    actual, expected,
                    "creation order {order:?}, cut {cut}, resume {resume_at}, record {index}"
                );
            }
            assert_eq!(restored.outputs(), vm.outputs());
            assert_eq!(resumed_world.preparations, 0);
            assert!(resumed_world.starts.is_empty());
            assert_eq!(
                resumed_world.cancellations.len(),
                usize::from(!already_closed),
                "a retained closure must not cancel the Driver again: cut {cut}"
            );
        }
    }
}

#[test]
fn retry_join_and_lifetime_deadlines_resume_in_the_retained_order() {
    let source = r#"
graph app : App {
  input retry: text
  input waiting: text
  input partner: text
  input window: text
  output expired: union { JoinExpired {} }
  output closed: text
  connector call : Process + Retry {
    command = "cat"; codec = lines; behavior = deterministic; idempotent = true
    timeout = duration("1s"); Retry.attempts = 2; Retry.on = [TimeoutExpired]
    Retry.delay = { kind = fixed; after = duration("1ms") }
  }
  operator pair {
    input left: text
    input right: text
    output expired: union { JoinExpired {} }
    on (left, right) within duration("1ms") { nothing }
  }
  operator accumulator {
    input in: text
    output out: text
    lifetime = { kind = fixed; after = duration("1ms") }
    on in { nothing }
    on stop { out = "closed" }
  }
  operator forward {
    input in: union { JoinExpired {} }
    output out: union { JoinExpired {} }
    on in { out = . }
  }
  retry -> call
  waiting -> pair.left; partner -> pair.right
  window -> accumulator
  pair.expired -> expired; accumulator.out -> closed
  forward -> expired
  call.out -> nothing; call.err -> nothing; call.exit -> nothing
}
"#;
    for expiry_target in ["expired", "forward"] {
        let (plan, hash, _) = checked(&source.replace(
            "pair.expired -> expired",
            &format!("pair.expired -> {expiry_target}"),
        ));
        for order in [
            ["retry", "waiting", "window"],
            ["retry", "window", "waiting"],
            ["waiting", "retry", "window"],
            ["waiting", "window", "retry"],
            ["window", "retry", "waiting"],
            ["window", "waiting", "retry"],
        ] {
            let mut vm = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
            let mut journal = MemJournal::new(GraphVersion(1));
            let mut world = World::default();
            for port in order {
                vm.accept_input(
                    &mut journal,
                    IngressProposal {
                        provenance: IngressProvenance::single(node::Path::new(["test"]).unwrap()),
                        draft: EnvelopeDraft {
                            port: node::Port::new(port).unwrap(),
                            data: Data::materialized(Value::Text("request".into())),
                            ext: Namespace::new(),
                        },
                        sequence: IngressSequence::Next,
                        session: None,
                    },
                )
                .unwrap();
                vm.run_ready_with_bindings(&mut journal, &mut world)
                    .unwrap();
                if port == "retry" {
                    vm.poll_drivers(&mut journal, &mut world).unwrap();
                    reply(&mut world, true);
                    vm.poll_drivers(&mut journal, &mut world).unwrap();
                    vm.run_ready_with_bindings(&mut journal, &mut world)
                        .unwrap();
                }
            }
            assert!(vm.outputs().is_empty());
            let before = journal.records().len();
            vm.advance_to(&mut journal, &mut world, 1_000_000).unwrap();
            assert_eq!(vm.outputs().len(), 2);
            assert!(vm.outputs().iter().any(|output| matches!(
                output.envelope.data(),
                Data::Materialized(Value::TaggedUnion(value)) if value.case == "JoinExpired"
            )));
            assert!(vm.outputs().iter().any(|output| output.envelope.data()
                == &Data::materialized(Value::Text("closed".into()))));
            assert_eq!(
                journal
                    .records()
                    .iter()
                    .filter(|record| matches!(record,
            Record::Transaction(record) if record.retry_record.as_deref().is_some_and(|packet|
                packet.facts.iter().any(|fact| matches!(fact,
                    Fact::Admitted { attempt, .. } if attempt.ordinal == 2)))))
                    .count(),
                1
            );

            for (cut, resume_at) in std::iter::once((before, 0))
                .chain((before..=journal.records().len()).map(|cut| (cut, 1_000_000)))
            {
                let mut retained =
                    MemJournal::from_records(GraphVersion(1), None, &journal.records()[..cut])
                        .unwrap();
                let mut restored =
                    Execution::new_program(&plan, hash, GraphVersion(1), resume_at, None);
                let mut resumed_world = World::default();
                restored
                    .recover_with_bindings(&retained, &mut resumed_world)
                    .unwrap();
                restored
                    .advance_to(&mut retained, &mut resumed_world, 1_000_000)
                    .unwrap();
                assert_eq!(retained.records().len(), journal.records().len());
                for (index, (actual, expected)) in
                    retained.records().iter().zip(journal.records()).enumerate()
                {
                    assert_eq!(
                        actual, expected,
                        "expiry target {expiry_target}, creation order {order:?}, cut {cut}, resume {resume_at}, record {index}"
                    );
                }
                assert_eq!(restored.outputs(), vm.outputs());
                assert_eq!(resumed_world.preparations, 0);
                assert!(resumed_world.starts.is_empty());
                assert!(resumed_world.cancellations.is_empty());
            }
        }
    }
}

#[test]
fn retry_wakeups_share_closing_drain_and_settlement_without_recalling_calls() {
    for delay_ms in [1, 2] {
        let (plan, hash, _) = checked(&format!(
            r#"
graph app : App {{
  input retry: text
  input stopping: text
  input close: bool
  output exit: variant
  connector call : Process + Retry {{
    command = "cat"; codec = lines; behavior = deterministic; idempotent = true
    timeout = duration("1s"); Retry.attempts = 2; Retry.on = [TimeoutExpired]
    Retry.delay = {{ kind = fixed; after = duration("{delay_ms}ms") }}
  }}
  connector stopped : Process + Retry {{
    command = "cat"; codec = lines; behavior = deterministic; idempotent = true
    timeout = duration("1s"); Retry.attempts = 3; Retry.on = [OutcomeUnknown]
    closing = {{ drain_for = duration("1ms"); settle_for = duration("1ms") }}
  }}
  retry -> call; stopping -> stopped; close -> stopped.instance.stop
  call.out -> nothing; call.err -> nothing; call.exit -> nothing
  stopped.out -> nothing; stopped.err -> nothing; stopped.exit -> exit
}}
"#,
        ));
        for order in [["retry", "stopping"], ["stopping", "retry"]] {
            let mut vm = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
            let mut journal = MemJournal::new(GraphVersion(1));
            let mut world = World::default();
            for port in order {
                vm.accept_input(
                    &mut journal,
                    IngressProposal {
                        provenance: IngressProvenance::single(node::Path::new(["test"]).unwrap()),
                        draft: EnvelopeDraft {
                            port: node::Port::new(port).unwrap(),
                            data: Data::materialized(Value::Text("request".into())),
                            ext: Namespace::new(),
                        },
                        sequence: IngressSequence::Next,
                        session: None,
                    },
                )
                .unwrap();
                vm.run_ready_with_bindings(&mut journal, &mut world)
                    .unwrap();
                vm.poll_drivers(&mut journal, &mut world).unwrap();
                if port == "retry" {
                    reply(&mut world, true);
                    vm.poll_drivers(&mut journal, &mut world).unwrap();
                } else {
                    vm.accept_input(
                        &mut journal,
                        IngressProposal {
                            provenance: IngressProvenance::single(
                                node::Path::new(["test"]).unwrap(),
                            ),
                            draft: EnvelopeDraft {
                                port: node::Port::new("close").unwrap(),
                                data: Data::materialized(Value::Bool(true)),
                                ext: Namespace::new(),
                            },
                            sequence: IngressSequence::Next,
                            session: None,
                        },
                    )
                    .unwrap();
                }
                vm.run_ready_with_bindings(&mut journal, &mut world)
                    .unwrap();
            }
            let before = journal.records().len();
            let due = delay_ms * 1_000_000;
            vm.advance_to(&mut journal, &mut world, due).unwrap();
            assert_eq!(vm.outputs().len(), usize::from(delay_ms == 2));
            if delay_ms == 2 {
                assert!(matches!(vm.outputs()[0].envelope.data(),
                    Data::Materialized(Value::TaggedUnion(value)) if value.case == "OutcomeUnknown"));
            }
            assert_eq!(
                world.starts.len(),
                2,
                "settlement does not start another effect"
            );

            for (cut, resume_at) in std::iter::once((before, 0))
                .chain((before..=journal.records().len()).map(|cut| (cut, due)))
            {
                let mut retained =
                    MemJournal::from_records(GraphVersion(1), None, &journal.records()[..cut])
                        .unwrap();
                let mut restored =
                    Execution::new_program(&plan, hash, GraphVersion(1), resume_at, None);
                let mut resumed_world = World::default();
                restored
                    .recover_with_bindings(&retained, &mut resumed_world)
                    .unwrap();
                assert!(
                    resumed_world.cancellations.is_empty(),
                    "reconstruction has no effects"
                );
                restored
                    .advance_to(&mut retained, &mut resumed_world, due)
                    .unwrap();
                assert_eq!(retained.records().len(), journal.records().len());
                for (index, (actual, expected)) in
                    retained.records().iter().zip(journal.records()).enumerate()
                {
                    assert_eq!(
                        actual, expected,
                        "delay {delay_ms}ms, order {order:?}, cut {cut}, resume {resume_at}, record {index}"
                    );
                }
                assert_eq!(restored.outputs(), vm.outputs());
                assert_eq!(resumed_world.preparations, 0);
                assert!(resumed_world.starts.is_empty());
            }
        }
    }
}

#[test]
fn final_retry_publication_reserves_the_whole_fanout_before_closing() {
    let (plan, hash, _) = checked(
        r#"
graph app : App {
  input in: text
  connector call : Process + Retry {
    command = "cat"; codec = lines; behavior = deterministic; idempotent = true
    timeout = duration("1s"); Retry.attempts = 2; Retry.on = [TimeoutExpired]
    Retry.delay = { kind = fixed; after = duration("100ms") }
  }
  operator a { input in: text; on in { nothing } }
  operator b { input in: text; on in { nothing } }
  in -> call
  call.out -> { a; b }
  call.err -> nothing; call.exit -> nothing
}
"#,
    );
    let capacity = Execution::new_program(&plan, hash, GraphVersion(1), 0, None)
        .inspection()
        .mailbox_capacity;
    for frames in [capacity, capacity + 1] {
        let mut vm = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut world = World::default();
        inject(&mut vm, &mut journal);
        vm.run_ready_with_bindings(&mut journal, &mut world)
            .unwrap();
        vm.poll_drivers(&mut journal, &mut world).unwrap();
        reply(&mut world, true);
        vm.poll_drivers(&mut journal, &mut world).unwrap();
        vm.advance_to(&mut journal, &mut world, 100_000_000)
            .unwrap();
        vm.poll_drivers(&mut journal, &mut world).unwrap();
        assert_eq!(world.starts.len(), 2);
        world.responses.push(Response {
            call_id: world.starts[1].call_id,
            completion: Completion::process(
                (0..frames)
                    .map(|index| Data::materialized(Value::Text(index.to_string())))
                    .collect(),
                vec![],
                ProcessExit::Exited {
                    exit_code: 0,
                    signal: None,
                },
            ),
        });
        vm.poll_drivers(&mut journal, &mut world).unwrap();
        let before = journal.records().to_vec();
        let inspection = vm.inspection();
        let observations = vm.envelope_views().to_vec();
        let result = vm.step_with_bindings(&mut journal, &mut world);
        if frames == capacity {
            assert!(result.unwrap());
            let Some(Record::Transaction(record)) = journal.records().last() else {
                panic!("final Connector transaction")
            };
            assert!(matches!(
                record.retry_record.as_deref().unwrap().facts.as_slice(),
                [Fact::Published(_), Fact::Close { .. }]
            ));
            assert_eq!(record.emissions.len(), frames + 1, "stdout and terminal");
            assert_eq!(
                record.routes.len(),
                2 * frames,
                "both destinations commit together"
            );
            for node in ["a", "b"] {
                let inspection = vm.inspection();
                let mailbox = inspection
                    .mailboxes
                    .iter()
                    .find(|mailbox| mailbox.node.to_string() == node)
                    .unwrap();
                assert_eq!(mailbox.deliveries.len(), capacity);
            }
        } else {
            assert!(
                matches!(
                    result,
                    Err(rastok::runtime::execution::ExecutionFault::MailboxFull { .. })
                ),
                "{result:?}"
            );
            assert_eq!(journal.records(), before);
            assert_eq!(vm.inspection(), inspection);
            assert_eq!(vm.envelope_views(), observations);
            assert!(matches!(
                vm.step_with_bindings(&mut journal, &mut world),
                Err(rastok::runtime::execution::ExecutionFault::MailboxFull { .. })
            ));
            assert_eq!(
                journal.records(),
                before,
                "no partial publication or Close on retry"
            );
        }

        let mut retained = MemJournal::from_records(GraphVersion(1), None, &before).unwrap();
        let mut restored =
            Execution::new_program(&plan, hash, GraphVersion(1), vm.logical_time(), None);
        let mut resumed_world = World::default();
        restored
            .recover_with_bindings(&retained, &mut resumed_world)
            .unwrap();
        let result = restored.step_with_bindings(&mut retained, &mut resumed_world);
        if frames == capacity {
            assert!(result.unwrap());
        } else {
            assert!(matches!(
                result,
                Err(rastok::runtime::execution::ExecutionFault::MailboxFull { .. })
            ));
        }
        assert_eq!(retained.records(), journal.records());
        assert_eq!(restored.inspection(), vm.inspection());
        assert_eq!(resumed_world.preparations, 0);
        assert!(resumed_world.starts.is_empty());
        assert!(resumed_world.cancellations.is_empty());
    }
}

#[test]
fn keyed_fanout_opens_independent_operations_and_never_retries_the_other_key() {
    let (plan, hash, _) = checked(
        r#"
graph app : App {
  input in: text
  output out: text
  instance = { key = data }
  operator split {
    input in: text
    output a: text
    output b: text
    on in { (a, b) = data }
  }
  connector call : Process + Retry {
    command = "cat"; codec = lines; behavior = deterministic; idempotent = true
    timeout = duration("1s"); Retry.attempts = 2; Retry.on = [TimeoutExpired]
    Retry.delay = { kind = fixed; after = duration("100ms") }
  }
  in -> split
  split.a -> call; split.b -> call
  call.out -> out; call.err -> nothing; call.exit -> nothing
}
"#,
    );
    let mut vm = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut world = World::default();
    for key in ["alice", "bob"] {
        vm.accept_input(
            &mut journal,
            IngressProposal {
                provenance: IngressProvenance::single(node::Path::new(["test"]).unwrap()),
                draft: EnvelopeDraft {
                    port: node::Port::new("in").unwrap(),
                    data: Data::materialized(Value::Text(key.into())),
                    ext: Namespace::new(),
                },
                sequence: IngressSequence::Next,
                session: None,
            },
        )
        .unwrap();
    }
    let mut answered = 0;
    for now in [0, 100_000_000] {
        vm.advance_to(&mut journal, &mut world, now).unwrap();
        for round in 0..8 {
            vm.poll_drivers(&mut journal, &mut world).unwrap();
            if answered == world.starts.len() {
                break;
            }
            assert!(round < 7, "the bounded test must settle");
            for call in &world.starts[answered..] {
                let alice = call.prepared.bytes() == b"alice\n";
                let failed = alice && now == 0;
                world.responses.push(Response {
                    call_id: call.call_id,
                    completion: Completion::process(
                        if failed {
                            vec![]
                        } else {
                            vec![Data::materialized(Value::Text(
                                if alice { "alice" } else { "bob" }.into(),
                            ))]
                        },
                        vec![],
                        if failed {
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
            answered = world.starts.len();
            vm.poll_drivers(&mut journal, &mut world).unwrap();
            vm.run_ready_with_bindings(&mut journal, &mut world)
                .unwrap();
        }
        assert_eq!(vm.outputs().len(), if now == 0 { 2 } else { 4 });
        assert!(
            vm.outputs()
                .iter()
                .take(2)
                .all(|output| output.envelope.data()
                    == &Data::materialized(Value::Text("bob".into())))
        );
    }
    assert!(vm.is_stable());
    assert_eq!(
        world.preparations, 4,
        "one preparation for each exact opening delivery"
    );
    assert_eq!(world.starts.len(), 6);
    assert_eq!(
        world
            .starts
            .iter()
            .filter(|call| call.prepared.bytes() == b"bob\n")
            .count(),
        2
    );
    let mut by_instance = BTreeMap::<_, Vec<_>>::new();
    for record in journal.records() {
        if let Record::Transaction(record) = record
            && let Some(packet) = record.retry_record.as_deref()
            && let [
                Fact::Opened {
                    operation, opening, ..
                },
                Fact::Admitted { envelope, .. },
            ] = packet.facts.as_slice()
        {
            by_instance
                .entry(record.instance.clone())
                .or_default()
                .push((*operation, *opening, envelope.meta().sys().origin_id()));
        }
    }
    assert_eq!(by_instance.len(), 2);
    for openings in by_instance.values() {
        assert_eq!(openings.len(), 2);
        assert_ne!(
            openings[0].0, openings[1].0,
            "same origin is not the operation identity"
        );
        assert_ne!(openings[0].1, openings[1].1);
        assert_eq!(openings[0].2, openings[1].2);
    }
    for cut in 0..=journal.records().len() {
        let retained =
            MemJournal::from_records(GraphVersion(1), None, &journal.records()[..cut]).unwrap();
        let mut restored =
            Execution::new_program(&plan, hash, GraphVersion(1), vm.logical_time(), None);
        let mut resumed_world = World::default();
        restored
            .recover_with_bindings(&retained, &mut resumed_world)
            .unwrap();
        assert_eq!(resumed_world.preparations, 0, "prefix {cut}");
        assert!(resumed_world.starts.is_empty());
        assert!(resumed_world.cancellations.is_empty());
        if cut == journal.records().len() {
            assert_eq!(restored.outputs(), vm.outputs());
            assert!(restored.is_stable());
        }
    }
    let before = journal.records().to_vec();
    world.responses.push(Response {
        call_id: world.starts[0].call_id,
        completion: Completion::process(
            vec![],
            vec![],
            ProcessExit::Exited {
                exit_code: 0,
                signal: None,
            },
        ),
    });
    let late = vm.poll_drivers(&mut journal, &mut world).unwrap();
    assert_eq!(late.discarded.len(), 1);
    assert_eq!(
        journal.records(),
        before,
        "a late outcome cannot become another attempt's result"
    );
}

#[test]
fn stopping_at_each_held_batch_cut_preserves_its_committed_disposition() {
    let (plan, hash, _) = checked(
        r#"
graph app : App {
  input in: text
  input close: bool
  output out: text
  output errors: text
  output exit: variant
  connector call : Process + Retry {
    command = "cat"; codec = lines; behavior = deterministic; idempotent = true
    timeout = duration("1s"); Retry.attempts = 3; Retry.on = [TimeoutExpired]
    Retry.delay = { kind = fixed; after = duration("100ms") }
    closing = { drain_for = duration("1s"); settle_for = duration("1s") }
  }
  in -> call -> out
  call.err -> errors; call.exit -> exit
  close -> call.instance.stop
}
"#,
    );
    for steps_after_closure in 0..=2 {
        let mut vm = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut world = World::default();
        inject(&mut vm, &mut journal);
        vm.run_ready_with_bindings(&mut journal, &mut world)
            .unwrap();
        vm.poll_drivers(&mut journal, &mut world).unwrap();
        reply(&mut world, true);
        vm.poll_drivers(&mut journal, &mut world).unwrap();
        for _ in 0..steps_after_closure {
            assert!(vm.step_with_bindings(&mut journal, &mut world).unwrap());
        }
        match (steps_after_closure, journal.records().last().unwrap()) {
            (0, Record::Egress(EgressRecord::Call(call))) => {
                assert_ne!(call.status, rastok::journal::CallStatus::Open);
            }
            (1 | 2, Record::Transaction(record)) => {
                let facts = &record.retry_record.as_deref().unwrap().facts;
                assert!(matches!(
                    (steps_after_closure, facts.as_slice()),
                    (1, [Fact::Held(_)]) | (2, [Fact::Consumed(_), Fact::Advance { .. }])
                ));
            }
            _ => panic!("the probe must stop at closure, Held or Consumed"),
        }
        vm.accept_input(
            &mut journal,
            IngressProposal {
                provenance: IngressProvenance::single(node::Path::new(["test"]).unwrap()),
                draft: EnvelopeDraft {
                    port: node::Port::new("close").unwrap(),
                    data: Data::materialized(Value::Bool(true)),
                    ext: Namespace::new(),
                },
                sequence: IngressSequence::Next,
                session: None,
            },
        )
        .unwrap();
        let before_close = journal.records().len();
        vm.advance_to(&mut journal, &mut world, 3_000_000_000)
            .unwrap();
        assert!(vm.outputs().is_empty());
        assert!(vm.is_stable());
        assert_eq!(world.starts.len(), 1);
        assert!(world.cancellations.is_empty());
        let facts: Vec<_> = journal
            .records()
            .iter()
            .filter_map(|record| match record {
                Record::Transaction(record) => record.retry_record.as_deref(),
                _ => None,
            })
            .flat_map(|packet| &packet.facts)
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
        assert!(matches!(
            facts.last(),
            Some(Fact::Close {
                reason: rastok::journal::retry_protocol::CloseReason::Stopped,
                ..
            })
        ));
        assert!(!facts.iter().any(|fact| match fact {
            Fact::Published(_) => true,
            Fact::Admitted { attempt, .. } => attempt.ordinal > 1,
            _ => false,
        }));

        for cut in before_close..=journal.records().len() {
            let mut retained =
                MemJournal::from_records(GraphVersion(1), None, &journal.records()[..cut]).unwrap();
            let mut restored = Execution::new_program(&plan, hash, GraphVersion(1), 0, None);
            let mut resumed_world = World::default();
            restored
                .recover_with_bindings(&retained, &mut resumed_world)
                .unwrap();
            restored
                .advance_to(&mut retained, &mut resumed_world, 3_000_000_000)
                .unwrap();
            assert_eq!(
                retained.records(),
                journal.records(),
                "steps after closure {steps_after_closure}, cut {cut}"
            );
            assert!(restored.outputs().is_empty());
            assert!(restored.is_stable());
            assert_eq!(resumed_world.preparations, 0);
            assert!(resumed_world.starts.is_empty());
            assert!(resumed_world.cancellations.is_empty());
        }
    }
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
        let mut restored = Execution::new_program(&plan, hash, GraphVersion(1), 100_000_000, None);
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
            let mut restored =
                Execution::new_program(&plan, hash, GraphVersion(1), 100_000_000, None);
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

#[test]
fn typed_http_usage_stays_in_each_original_attempt_without_a_report() {
    let (plan, hash, labs) = checked(
        r#"
type Amount = decimal(38,18)
type Reply = struct {
  usage: struct { tokens: uint @measure(id = input_tokens, unit = "token") }
  cost: Amount @measure(id = amount, unit = "USD")
}
graph app : App {
  connector call : HttpClient + Retry {
    url = "https://partner.invalid/model"; codec = json
    behavior = probabilistic; idempotent = true; timeout = duration("1s")
    output out: Reply
    Retry.attempts = 2; Retry.on = [ServerError]
    Retry.delay = { kind = fixed; after = duration("100ms") }
  }
  in -> call -> out
  call.exit -> nothing
}
lab app {
  simulation measured on app {
    mock call {
      on in where now() == timestamp("1970-01-01T00:00:00Z") {
        out = { usage = { tokens = 100u }; cost = decimal("0.01") as Amount }
        exit = ServerError { status = 503 }
      }
      on in {
        out = { usage = { tokens = 200u }; cost = decimal("0.02") as Amount }
        exit = Completed { status = 200 }
      }
    }
    scenario repeat {
      app.in = json! {}
      expect nothing app.out
      advance duration("100ms")
      expect app.out count == 1
      expect call.out count == 1 where data.usage.tokens == 200u
    }
  }
}
"#,
    );
    let runs = rastok::lab::run_program(&plan, hash, GraphVersion(1), &labs[0], None);
    assert!(runs[0].report.passed(), "{:?}", runs[0].report);
    let records = &runs[0].records;
    assert_original_attempt_measurements(
        &plan,
        hash,
        records,
        &[
            Value::Struct(vec![
                (
                    "cost".into(),
                    Value::Decimal {
                        unscaled: 10_000_000_000_000_000,
                        scale: 18,
                    },
                ),
                (
                    "usage".into(),
                    Value::Struct(vec![("tokens".into(), Value::Uint(100))]),
                ),
            ]),
            Value::Struct(vec![
                (
                    "cost".into(),
                    Value::Decimal {
                        unscaled: 20_000_000_000_000_000,
                        scale: 18,
                    },
                ),
                (
                    "usage".into(),
                    Value::Struct(vec![("tokens".into(), Value::Uint(200))]),
                ),
            ]),
        ],
    );
}

#[test]
fn signed_business_quantities_survive_held_attempts_and_prefix_recovery() {
    let (plan, hash, labs) = checked(
        r#"
type Quantity = decimal(38,3)
type Reply = struct { adjustment: Quantity @measure(id = carrots, unit = "kg") }
graph app : App {
  connector call : HttpClient + Retry {
    url = "https://partner.invalid/stock"; codec = json
    behavior = probabilistic; idempotent = true; timeout = duration("1s")
    output out: Reply
    Retry.attempts = 2; Retry.on = [ServerError]
    Retry.delay = { kind = fixed; after = duration("100ms") }
  }
  in -> call -> out
  call.exit -> nothing
}

lab app {
  simulation measured on app {
    mock call {
      on in where now() == timestamp("1970-01-01T00:00:00Z") {
        out = { adjustment = decimal("1.250") as Quantity }
        exit = ServerError { status = 503 }
      }
      on in {
        out = { adjustment = decimal("-3.500") as Quantity }
        exit = Completed { status = 200 }
      }
    }
    scenario repeat {
      app.in = json! {}
      expect nothing app.out
      advance duration("100ms")
      expect app.out count == 1
    }
  }
}
"#,
    );
    let runs = rastok::lab::run_program(&plan, hash, GraphVersion(1), &labs[0], None);
    assert!(runs[0].report.passed(), "{:?}", runs[0].report);
    let records = &runs[0].records;
    assert_original_attempt_measurements(
        &plan,
        hash,
        records,
        &[
            Value::Struct(vec![(
                "adjustment".into(),
                Value::Decimal {
                    unscaled: 1250,
                    scale: 3,
                },
            )]),
            Value::Struct(vec![(
                "adjustment".into(),
                Value::Decimal {
                    unscaled: -3500,
                    scale: 3,
                },
            )]),
        ],
    );
}

#[test]
fn named_monetary_refunds_stay_in_each_attempt_and_survive_recovery() {
    let (plan, hash, labs) = checked(
        r#"
type Amount = decimal(38,18)
type Reply = struct { credited: Amount @measure(id = refund, unit = "EUR") }
graph app : App {
  connector call : HttpClient + Retry {
    url = "https://partner.invalid/credit"; codec = json
    behavior = probabilistic; idempotent = true; timeout = duration("1s")
    output out: Reply
    Retry.attempts = 2; Retry.on = [ServerError]
    Retry.delay = { kind = fixed; after = duration("100ms") }
  }
  in -> call -> out
  call.exit -> nothing
}
lab app {
  simulation measured on app {
    mock call {
      on in where now() == timestamp("1970-01-01T00:00:00Z") {
        out = { credited = decimal("-1.25") as Amount }
        exit = ServerError { status = 503 }
      }
      on in {
        out = { credited = decimal("-3.50") as Amount }
        exit = Completed { status = 200 }
      }
    }
    scenario repeat {
      app.in = json! {}
      expect nothing app.out
      advance duration("100ms")
      expect app.out count == 1
    }
  }
}
"#,
    );
    let runs = rastok::lab::run_program(&plan, hash, GraphVersion(1), &labs[0], None);
    assert!(runs[0].report.passed(), "{:?}", runs[0].report);
    let records = &runs[0].records;
    assert_original_attempt_measurements(
        &plan,
        hash,
        records,
        &[
            Value::Struct(vec![(
                "credited".into(),
                Value::Decimal {
                    unscaled: -1_250_000_000_000_000_000,
                    scale: 18,
                },
            )]),
            Value::Struct(vec![(
                "credited".into(),
                Value::Decimal {
                    unscaled: -3_500_000_000_000_000_000,
                    scale: 18,
                },
            )]),
        ],
    );
}

fn assert_original_attempt_measurements(
    plan: &plan::ValidatedExecutionPlan,
    hash: plan::SemanticHash,
    records: &[Record],
    expected: &[Value],
) {
    let batches: Vec<_> = records
        .iter()
        .filter_map(|record| {
            let Record::Transaction(transaction) = record else {
                return None;
            };
            transaction
                .retry_record
                .as_ref()?
                .facts
                .iter()
                .find_map(|fact| match fact {
                    Fact::Held(batch) | Fact::Published(batch) => Some((transaction, batch)),
                    _ => None,
                })
        })
        .collect();
    assert_eq!(batches.len(), expected.len());
    let mut reply_ids = Vec::new();
    for (index, ((transaction, batch), value)) in batches.iter().zip(expected).enumerate() {
        assert_eq!(batch.attempt.ordinal, index as u64 + 1);
        assert_eq!(batch.attempt.operation, batches[0].1.attempt.operation);
        assert_eq!(
            batch.emissions,
            transaction
                .emissions
                .iter()
                .map(|emission| emission.envelope.meta().sys().id())
                .collect::<Vec<_>>()
        );
        let reply = transaction
            .emissions
            .iter()
            .find(|emission| emission.port.name() == "out")
            .unwrap();
        assert_eq!(reply.envelope.data().materialized_value(), Some(value));
        reply_ids.push(reply.envelope.meta().sys().id());
        if index + 1 < expected.len() {
            assert!(
                transaction
                    .retry_record
                    .as_ref()
                    .unwrap()
                    .facts
                    .iter()
                    .any(|fact| matches!(fact, Fact::Held(_)))
            );
            assert!(
                transaction.routes.is_empty(),
                "held values stay in the journal, not downstream"
            );
        }
    }
    assert_ne!(reply_ids[0], reply_ids[1]);
    let consumed: Vec<_> = records
        .iter()
        .filter_map(|record| {
            let Record::Transaction(transaction) = record else {
                return None;
            };
            transaction
                .retry_record
                .as_ref()?
                .facts
                .iter()
                .find_map(|fact| match fact {
                    Fact::Consumed(batch) => {
                        assert!(
                            transaction.emissions.is_empty(),
                            "consumption does not re-emit measurements"
                        );
                        Some(batch)
                    }
                    _ => None,
                })
        })
        .collect();
    assert_eq!(consumed, vec![batches[0].1]);
    for id in &reply_ids {
        let copies = records
            .iter()
            .filter_map(|record| match record {
                Record::Transaction(transaction) => Some(&transaction.emissions),
                _ => None,
            })
            .flatten()
            .filter(|emission| emission.envelope.meta().sys().id() == *id)
            .count();
        assert_eq!(
            copies, 1,
            "each original measurement-bearing emission is retained once"
        );
    }
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
    let journal = MemJournal::from_records(GraphVersion(1), None, records).unwrap();
    let recording =
        rastok::recording::Recording::capture(&compilation, &journal, 100_000_000).unwrap();
    let decoded = rastok::recording::Recording::decode(&recording.encode().unwrap()).unwrap();
    assert_eq!(
        decoded.records(),
        records,
        "diagnostic retention preserves every exact value and disposition"
    );
    for cut in 0..=records.len() {
        let prefix =
            MemJournal::from_records(GraphVersion(1), None, &decoded.records()[..cut]).unwrap();
        let mut restored = Execution::new_program(plan, hash, GraphVersion(1), 100_000_000, None);
        let mut world = World::default();
        restored.recover_with_bindings(&prefix, &mut world).unwrap();
        assert_eq!(world.preparations, 0);
        assert!(world.starts.is_empty());
        assert_eq!(prefix.records(), &records[..cut]);
        let (inspected, _) = decoded.inspect(cut).unwrap();
        assert_eq!(inspected.records(), prefix.records());
    }
}
