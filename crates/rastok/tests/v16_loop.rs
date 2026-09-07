//! Command Loop regressions cross the real source, package reader and Worker.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::boundary::{Draft, InboundEnvelopeProvenance, InboundSubmission, SubmissionSequence};
use rastok::compiler;
use rastok::connectors::{DriverRegistry, RuntimeBindings};
use rastok::contract::ContractCatalog;
use rastok::envelope::{Data, Namespace, Value};
use rastok::journal::{
    Journal, MemJournal, Record,
    loop_protocol::{Fact, Fold, Phase},
};
use rastok::plan::{self, GraphVersion};
use rastok::runtime::vm::Vm;

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
    effect = idempotent
    behavior = probabilistic
    timeout = duration("1s")
    Loop.fuel = 2
    Loop.delay = { kind = fixed; after = duration("1s") }
  }
  in -> worker
  worker.out -> nothing
  worker.err -> nothing
  worker.exit -> match {
    TimeoutExpired -> worker.Loop.again
    _ -> nothing
  }
  worker.Loop.expired -> expired
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
    let fold = Fold::from_records(&runs[0].records).unwrap();
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
    let fold = Fold::from_records(&runs[0].records).unwrap();
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
    let mut reference = Vm::boot(&plan, GraphVersion(1));
    let mut expected = MemJournal::new(GraphVersion(1));
    inject(&mut reference, &mut expected, 3);
    while reference.step(&mut expected).unwrap() {
        assert_eq!(
            expected.loop_protocol().unwrap(),
            Fold::from_records(expected.records()).unwrap(),
            "the fast projection must equal independent replay after every commit"
        );
    }
    let expected_records = expected.records();

    for prefix in 1..=expected_records.len() {
        let mut interrupted = Vm::boot(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        inject(&mut interrupted, &mut journal, 3);
        while journal.records().len() < prefix {
            assert!(interrupted.step(&mut journal).unwrap());
        }
        let before = interrupted.inspection();
        let mut restored = Vm::boot(&plan, GraphVersion(1));
        restored
            .recover_from_journal(&journal)
            .unwrap_or_else(|error| panic!("prefix {prefix}: {error}"));
        assert_eq!(journal.records().len(), prefix, "recovery must not append");
        assert_eq!(restored.inspection(), before, "prefix {prefix}");
        restored.drain(&mut journal).unwrap();
        assert_eq!(journal.records(), expected_records, "prefix {prefix}");
        assert_eq!(restored.outputs(), reference.outputs(), "prefix {prefix}");
        assert!(restored.is_stable());
    }
}

#[test]
fn a_rejected_transaction_does_not_update_the_journal_loop_projection() {
    let plan = compile("Loop.fuel = 2; on in { out = data }");
    let mut vm = Vm::boot(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 7);
    vm.drain(&mut journal).unwrap();
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
    let rastok::journal::LoopRecord::R2(packet) = packet else {
        panic!("the selected record must use r2");
    };
    for fact in &mut packet.facts {
        if let Fact::Open {
            run,
            instance,
            opening,
            ..
        } = fact
        {
            *opening = rastok::journal::loop_protocol::OpeningAdmissionRef::Delivery(
                rastok::envelope::Id::derive(1, &[b"another-opening"]),
            );
            *run = rastok::journal::loop_protocol::run_ref(instance, opening);
        }
    }
    // The Loop packet is valid; a later journal invariant must still reject
    // the entire transaction without publishing the prepared fold.
    journal.loop_protocol().unwrap().apply(packet).unwrap();
    transaction.owner_path = plan::OwnerPath::new(["wrong-owner"]).unwrap();
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
        Fold::from_records(&before).unwrap()
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
    let mut vm = Vm::boot(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    // This is the Worker's existing mailbox/work bound, not a new quota.
    for _ in 0..1024 {
        inject(&mut vm, &mut journal, 1);
    }
    vm.drain(&mut journal).unwrap();
    assert!(
        vm.inspection()
            .mailboxes
            .iter()
            .all(|mailbox| mailbox.deliveries.is_empty())
    );
    let before = journal.records().to_vec();
    let incoming = || InboundSubmission {
        provenance: InboundEnvelopeProvenance::single(rastok::node::Path::new(["test"]).unwrap()),
        port: rastok::node::Port::new("in").unwrap(),
        draft: Draft {
            data: Data::materialized(Value::Int(1)),
            ext: Namespace::new(),
        },
        sequence: SubmissionSequence::Next,
    };
    assert!(matches!(
        vm.inject(&mut journal, incoming()),
        Err(rastok::runtime::vm::VmError::LoopQueueFull { .. })
    ));
    assert_eq!(
        journal.records(),
        before,
        "refusal must not admit another root"
    );
    let mut bindings = RuntimeBindings::try_new(&plan, &DriverRegistry::default()).unwrap();
    vm.advance(&mut journal, &mut bindings, 1_000_000_000)
        .unwrap();
    vm.drain(&mut journal).unwrap();
    assert_eq!(vm.outputs().len(), 1024);
    vm.inject(&mut journal, incoming()).unwrap();
    vm.drain(&mut journal).unwrap();
    let fold = Fold::from_records(journal.records()).unwrap();
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
            "command = \"cat\"; codec = lines; effect = idempotent; behavior = deterministic",
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
            "url = \"https://example.invalid\"; codec = text; effect = idempotent; behavior = probabilistic",
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
  worker.Loop.expired -> nothing
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
        let fold = Fold::from_records(&runs[0].records).unwrap();
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
            Record::Outbound(rastok::journal::OutboundRecord::Call(call))
                if call.status == rastok::journal::OutboundStatus::Open))
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
    effect = idempotent
    behavior = probabilistic
    timeout = duration("1s")
    Loop.fuel = 2
  }
  in -> worker
  worker.out -> worker.Loop.next
  worker.exit -> nothing
  worker.Loop.expired -> nothing
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
    let fold = Fold::from_records(&runs[0].records).unwrap();
    let (_, run) = fold.runs().next().unwrap();
    assert_eq!((run.iteration, run.phase), (1, Phase::Expired));
    assert_eq!(run.state.data(), &Data::materialized(Value::Int(9)));

    let mut forged = compilation.plan.plan().clone();
    let exit = forged
        .edges
        .iter()
        .find(|edge| edge.from.port.name() == "exit")
        .unwrap();
    let node = &forged.nodes[exit.from.node.unwrap().index()];
    let owner = plan::EndpointOwner::Node(node.path.clone());
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
        error.to_string().contains("more than one Loop command"),
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
fn recovery_rearms_the_retained_deadline_not_a_fresh_delay() {
    let plan = compile(
        r#"
      Loop.fuel = 2
      Loop.delay = { kind = fixed; after = duration("1s") }
      on in match { data <= 0 => out = data; _ => Loop.next = data - 1 }
    "#,
    );
    let mut interrupted = Vm::boot(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut interrupted, &mut journal, 1);
    interrupted.drain(&mut journal).unwrap();
    let before = journal.records().len();
    let mut restored = Vm::boot_at(&plan, GraphVersion(1), 750_000_000);
    restored.recover_from_journal(&journal).unwrap();
    restored.drain(&mut journal).unwrap();
    assert!(restored.outputs().is_empty());
    assert_eq!(journal.records().len(), before);
    let mut bindings = RuntimeBindings::try_new(&plan, &DriverRegistry::default()).unwrap();
    restored
        .advance(&mut journal, &mut bindings, 1_000_000_000)
        .unwrap();
    restored.drain(&mut journal).unwrap();
    assert_eq!(restored.outputs().len(), 1);
    assert!(restored.is_stable());
}

#[test]
fn retained_timer_bytes_are_required_for_delays_but_not_part_of_the_causal_seal() {
    use rastok::journal::timing::TimerSchedule;
    let plan = compile(
        r#"
      Loop.fuel = 2
      Loop.delay = { kind = fixed; after = duration("1s") }
      on in { Loop.next = data }
    "#,
    );
    let mut vm = Vm::boot_at(&plan, GraphVersion(1), 7_000_000_000);
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 1);
    vm.drain(&mut journal).unwrap();
    let index = journal
        .records()
        .iter()
        .position(|record| record.timer_schedule().is_some())
        .unwrap();
    let schedule = journal.records()[index].timer_schedule().unwrap();
    assert_eq!(schedule.armed_at(), 7_000_000_000);
    assert_eq!(schedule.arms()[0].due_at, 8_000_000_000);
    assert_eq!(
        &TimerSchedule::decode(&schedule.encode()).unwrap(),
        schedule
    );

    let before = journal.records().to_vec();
    let seal = rastok::golden::project(&before).seal();
    for corrupt in 0..4 {
        let mut records = before.clone();
        let Record::Transaction(transaction) = &mut records[index] else {
            unreachable!()
        };
        match corrupt {
            0 => transaction.timer_schedule = None,
            1 => {
                let schedule = transaction.timer_schedule.as_deref().unwrap();
                let mut arms = schedule.arms().to_vec();
                arms[0].due_at += 1;
                transaction.timer_schedule = Some(Box::new(
                    TimerSchedule::try_new(schedule.armed_at(), arms).unwrap(),
                ));
            }
            2 => {
                let packet = transaction.loop_record.as_deref_mut().unwrap();
                let rastok::journal::LoopRecord::R2(packet) = packet else {
                    unreachable!()
                };
                let Fact::Advance { due_at, .. } = &mut packet.facts[0] else {
                    unreachable!()
                };
                *due_at = Some(8_000_000_001);
            }
            _ => {
                let schedule = transaction.timer_schedule.as_deref().unwrap();
                let arm = schedule.arms()[0];
                transaction.timer_schedule = Some(Box::new(
                    TimerSchedule::try_new(schedule.armed_at(), vec![arm, arm]).unwrap(),
                ));
            }
        }
        assert!(matches!(
            journal.commit(transaction.as_ref().clone()),
            Err(rastok::journal::Error::InvalidTimerSchedule(_))
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
    let mut vm = Vm::boot(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 7);
    vm.drain(&mut journal).unwrap();
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
        rastok::runtime::vm::VmError::InvalidRecovery {
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
            .owners
            .iter()
            .find_map(|owner| owner.loop_spec())
            .unwrap()
            .proof
            .as_ref()
            .unwrap();
        assert_eq!(proof.decreases, [1]);
        for value in [-2, 0, 3] {
            let mut vm = Vm::boot(&plan, GraphVersion(1));
            let mut journal = MemJournal::new(GraphVersion(1));
            inject(&mut vm, &mut journal, value);
            vm.drain(&mut journal).unwrap();
            assert_eq!(vm.outputs().len(), 1);
            assert_eq!(vm.outputs()[0].port.name(), "out");
            let fold = Fold::from_records(journal.records()).unwrap();
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
    let mut vm = Vm::boot(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 8);
    vm.drain(&mut journal).unwrap();
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
        .owners
        .iter_mut()
        .find_map(|owner| {
            owner
                .extensions
                .iter_mut()
                .find_map(|extension| match extension {
                    plan::ExtensionSpec::Loop(spec) => spec.proof.as_mut(),
                    plan::ExtensionSpec::App => None,
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
    for owner in &mut raw.owners {
        for extension in &mut owner.extensions {
            if let plan::ExtensionSpec::Loop(spec) = extension {
                spec.terminals.clear();
            }
        }
    }
    assert!(matches!(
        plan::ValidatedExecutionPlan::try_new(raw),
        Err(plan::PlanValidationError::InvalidGraphContract {
            detail: "a Loop host with ordinary outputs needs a non-empty terminal set",
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
            "graph app : App {{ input in: int; output out: int; operator work : Loop {{ input in: int; output out: int; Loop.measure = Loop.state.data; {dispatch} }} in -> work -> out; work.Loop.expired -> nothing }}"
        );
        let errors =
            compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap_err();
        assert!(
            format!("{errors:?}").contains("cannot be proved finite"),
            "{errors:?}"
        );
    }
}
fn inject(vm: &mut Vm<'_>, journal: &mut MemJournal, value: i64) {
    vm.inject(
        journal,
        InboundSubmission {
            provenance: InboundEnvelopeProvenance::single(
                rastok::node::Path::new(["test"]).unwrap(),
            ),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Int(value)),
                ext: Namespace::new(),
            },
            sequence: SubmissionSequence::Next,
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
    {body}
  }}
  in -> work -> out
  work.Loop.expired -> expired
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
    let mut vm = Vm::boot(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 42);
    vm.drain(&mut journal).unwrap();
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
        Fold::from_records(journal.records())
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
    let mut vm = Vm::boot(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 7);
    vm.drain(&mut journal).unwrap();
    assert_eq!(vm.outputs().len(), 1);
    assert_eq!(vm.outputs()[0].port.name(), "expired");
    let Data::Materialized(Value::TaggedUnion(value)) = vm.outputs()[0].envelope.data() else {
        panic!("typed expiry")
    };
    assert_eq!(value.case, "LoopExpired");
    assert_eq!(value.payload, [("last_state".into(), Value::Int(9))]);
    let fold = Fold::from_records(journal.records()).unwrap();
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
    let mut vm = Vm::boot(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 1);
    vm.drain(&mut journal).unwrap();
    assert!(vm.outputs().is_empty());
    let fold = Fold::from_records(journal.records()).unwrap();
    let run = fold.runs().next().unwrap().1;
    assert_eq!(run.phase, Phase::AwaitingAdmission);
    assert_eq!(run.due_at, Some(1_000_000_000));
    let mut bindings = RuntimeBindings::try_new(&plan, &DriverRegistry::default()).unwrap();
    vm.advance(&mut journal, &mut bindings, 999_999_999)
        .unwrap();
    vm.drain(&mut journal).unwrap();
    assert!(vm.outputs().is_empty());
    vm.advance(&mut journal, &mut bindings, 1_000_000_000)
        .unwrap();
    vm.drain(&mut journal).unwrap();
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
  operator work : Loop {{ input in: int; output out: int; Loop.fuel = 3; {dispatch} }}
  in -> work -> out
  work.Loop.expired -> nothing
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
    output out: int
    Loop.fuel = 3
    on in match { data <= 0 => out = Loop.state.data; _ => Loop.next = data - 1 }
  }
  in -> { left; right }
  { left.out; right.out } -> work
  work.out -> out
  work.Loop.expired -> nothing
}
"#;
    let compilation = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let decoded = plan::decode(&plan::encode(&compilation.plan, &compilation.hash))
        .unwrap()
        .0;
    let mut vm = Vm::boot(&decoded, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 1);
    vm.drain(&mut journal).unwrap();
    assert_eq!(vm.outputs().len(), 2);
    assert_ne!(
        vm.outputs()[0].envelope.meta().sys().id(),
        vm.outputs()[1].envelope.meta().sys().id()
    );
    assert_eq!(
        vm.outputs()[0].envelope.meta().sys().origin_id(),
        vm.outputs()[1].envelope.meta().sys().origin_id()
    );
    let fold = Fold::from_records(journal.records()).unwrap();
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
    on in match { data <= 0 => out = data; _ => Loop.next = data - 1 }
  }
  operator joiner {
    input a: int; input b: int; output out: int
    on (a, b) { out = a.data + b.data }
  }
  in -> { work; joiner.a; joiner.b }
  work.out -> out
  work.Loop.expired -> nothing
  joiner.out -> nothing
}
"#;
    let compilation = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject(&mut vm, &mut journal, 1);
    vm.drain(&mut journal).unwrap();
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
    let mut vm = Vm::boot(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut bindings = RuntimeBindings::try_new(&plan, &DriverRegistry::default()).unwrap();
    inject(&mut vm, &mut journal, 7);
    vm.drain(&mut journal).unwrap();
    vm.advance(&mut journal, &mut bindings, 1_000_000_000)
        .unwrap();
    let fold = Fold::from_records(journal.records()).unwrap();
    assert_eq!(fold.runs().next().unwrap().1.phase, Phase::Closed);
    inject(&mut vm, &mut journal, 8);
    vm.drain(&mut journal).unwrap();
    vm.advance(&mut journal, &mut bindings, 2_000_000_000)
        .unwrap();
    let fold = Fold::from_records(journal.records()).unwrap();
    assert_eq!(fold.runs().count(), 2);
    assert!(fold.runs().all(|(_, run)| run.iteration == 1));
    assert!(vm.outputs().is_empty());
}
