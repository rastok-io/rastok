//! Civil origins and relative logical instants meet only at explicit observations.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use rastok::compiler::{Compilation, compile};
use rastok::connector::contract::ContractCatalog;
use rastok::connector::drivers::{DriverRegistry, RuntimeDrivers};
use rastok::connector::exchange::{
    EnvelopeDraft, IngressProposal, IngressProvenance, IngressSequence,
};
use rastok::envelope::{Data, Namespace};
use rastok::instance::{InstanceAddress, Segment};
use rastok::journal::{Journal, MemJournal, Record};
use rastok::node::{OwnerPath, Path, Port};
use rastok::plan::GraphVersion;
use rastok::recording::Recording;
use rastok::runtime::execution::{Execution, ExecutionFault};
use rastok::time::{Duration, LogicalTime, Timestamp};
use rastok::value::Value;

const CLOCK: &str = r#"
graph app : App {
  input in: int
  output out: struct { instant: timestamp }
  operator clock {
    input in: int
    output out: struct { instant: timestamp }
    on in { out = { instant = now() } }
  }
  in -> clock -> out
}
"#;

fn checked(source: &str) -> Compilation {
    compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap()
}

fn send(execution: &mut Execution<'_>, journal: &mut MemJournal, value: i64) {
    execution
        .accept_input(
            journal,
            IngressProposal {
                provenance: IngressProvenance::single(Path::new(["test"]).unwrap()),
                draft: EnvelopeDraft {
                    port: Port::new("in").unwrap(),
                    data: Data::materialized(Value::Int(value)),
                    ext: Namespace::new(),
                },
                sequence: IngressSequence::Next,
                session: None,
            },
        )
        .unwrap();
}

#[test]
fn scenario_origin_is_applied_but_does_not_enter_the_program_identity() {
    let source = format!(
        r#"{CLOCK}
lab app {{
  simulation civil {{
    scenario old {{
      origin = timestamp("1600-01-01T00:00:00Z")
      app.in = 1
      expect app.out where data.instant == timestamp("1600-01-01T00:00:00Z")
      advance duration("1.25s")
      app.in = 2
      expect app.out where data.instant == timestamp("1600-01-01T00:00:01.25Z")
    }}
    scenario epoch {{
      app.in = 1
      expect app.out where data.instant == timestamp("1970-01-01T00:00:00Z")
    }}
  }}
}}
"#
    );
    let compilation = checked(&source);
    assert_eq!(compilation.hash, checked(CLOCK).hash);
    let runs = rastok::lab::run(&compilation.plan, GraphVersion(1), &compilation.labs[0]);
    assert_eq!(runs.len(), 2);
    assert!(runs.iter().all(|run| run.report.passed()), "{runs:?}");
    let observed: Vec<_> = runs[0]
        .records
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(record) => record.observed_time,
            _ => None,
        })
        .collect();
    assert_eq!(
        observed,
        [
            Timestamp::try_new(-11_676_096_000, 0).unwrap(),
            Timestamp::try_new(-11_676_095_999, 250_000_000).unwrap(),
        ]
    );
}

#[test]
fn scenario_origin_rejects_duplicates_nonconstant_and_wrong_typed_values() {
    for (declaration, code) in [
        ("origin = 1", "E-EXPR"),
        ("origin = duration(\"1s\")", "E-EXPR"),
        ("origin = now()", "E-EXPR"),
        (
            "origin = timestamp(\"1600-01-01T00:00:00Z\")\n origin = timestamp(\"2000-01-01T00:00:00Z\")",
            "E-DUP-LAB-IDENTITY",
        ),
    ] {
        let source = format!(
            "{CLOCK}\nlab app {{ simulation civil {{ scenario invalid {{ {declaration}\n app.in = 1 }} }} }}"
        );
        let errors = compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap_err();
        assert!(
            errors
                .diagnostics()
                .iter()
                .any(|error| error.code().as_str() == code),
            "{declaration}: {errors:?}"
        );
    }
}

#[test]
fn recording_and_recovery_restore_the_origin_without_resetting_logical_time() {
    let compilation = checked(CLOCK);
    let origin = Timestamp::try_new(253_402_300_799, 750_000_000).unwrap();
    let mut journal = MemJournal::new_with_origin(GraphVersion(1), origin);
    let mut execution = Execution::new_at(
        &compilation.plan,
        GraphVersion(1),
        origin,
        LogicalTime::ZERO,
    );
    let mut drivers =
        RuntimeDrivers::prepare(&compilation.plan, &DriverRegistry::default()).unwrap();
    let at = LogicalTime::from_nanoseconds(500_000_000);
    execution
        .advance_to(&mut journal, &mut drivers, at)
        .unwrap();
    send(&mut execution, &mut journal, 1);
    execution.run_ready(&mut journal).unwrap();
    let observed = origin
        .checked_add(Duration::from_nanoseconds(500_000_000))
        .unwrap();
    assert_eq!(
        rastok::runtime::data_value(execution.outputs()[0].envelope.data()).unwrap(),
        Value::Struct(vec![("instant".into(), Value::Timestamp(observed))])
    );

    let recording = Recording::capture(&compilation, &journal, at).unwrap();
    let decoded = Recording::decode(&recording.encode().unwrap()).unwrap();
    assert_eq!(decoded.origin(), origin);
    let (replayed_journal, replayed) = decoded.inspect(decoded.records().len()).unwrap();
    assert_eq!(replayed_journal.origin(), origin);
    assert_eq!(replayed.logical_time(), at);
    assert_eq!(replayed.origin(), origin);

    let mut resumed = Execution::new_at(
        &compilation.plan,
        GraphVersion(1),
        Timestamp::UNIX_EPOCH,
        at,
    );
    resumed.recover_from_journal(&journal).unwrap();
    assert_eq!(
        resumed.origin(),
        origin,
        "the journal header, not a new wall clock, supplies the origin"
    );
    let before = journal.records().to_vec();
    let mut too_early = Execution::new(&compilation.plan, GraphVersion(1));
    assert!(matches!(
        too_early.recover_from_journal(&journal),
        Err(ExecutionFault::InvalidRecovery { .. })
    ));
    assert_eq!(
        too_early.origin(),
        Timestamp::UNIX_EPOCH,
        "failed recovery is atomic"
    );
    assert_eq!(journal.records(), before);
}

#[test]
fn tumbling_boxes_are_epoch_aligned_and_recovery_keeps_their_civil_identity() {
    let compilation = checked(
        r#"
graph app : App {
  input in: int
  output out: int
  lifetime = { kind = tumbling; every = duration("2s") }
  operator counter {
    input in: int
    output out: int
    var count: int = 0
    on in { count = count + 1; out = count }
  }
  in -> counter -> out
}
"#,
    );
    for (seconds, fraction, start_seconds, remaining) in [
        (1, 250_000_000, 0, 750_000_000),
        (1, 750_000_000, 0, 250_000_000),
        (-1, 750_000_000, -2, 250_000_000),
        (-11_676_096_000, 250_000_000, -11_676_096_000, 1_750_000_000),
        (253_402_300_799, 750_000_000, 253_402_300_798, 250_000_000),
    ] {
        let origin = Timestamp::try_new(seconds, fraction).unwrap();
        let mut journal = MemJournal::new_with_origin(GraphVersion(1), origin);
        let mut execution = Execution::new_at(
            &compilation.plan,
            GraphVersion(1),
            origin,
            LogicalTime::ZERO,
        );
        let mut drivers =
            RuntimeDrivers::prepare(&compilation.plan, &DriverRegistry::default()).unwrap();
        send(&mut execution, &mut journal, 1);
        execution.run_ready(&mut journal).unwrap();
        let first = InstanceAddress::new(
            &OwnerPath::root(),
            &[Segment::ProcessingTimeBox {
                scope: OwnerPath::root(),
                start: Timestamp::try_new(start_seconds, 0).unwrap(),
            }],
        )
        .unwrap();
        assert!(journal.records().iter().any(
            |record| matches!(record, Record::Transaction(tx) if tx.instance.address() == &first)
        ));
        let at = LogicalTime::from_nanoseconds(remaining / 2);
        execution
            .advance_to(&mut journal, &mut drivers, at)
            .unwrap();
        send(&mut execution, &mut journal, 2);
        execution.run_ready(&mut journal).unwrap();
        assert_eq!(
            rastok::runtime::data_value(execution.outputs()[1].envelope.data()).unwrap(),
            Value::Int(2)
        );
        let mut resumed = Execution::new_at(
            &compilation.plan,
            GraphVersion(1),
            Timestamp::UNIX_EPOCH,
            at,
        );
        resumed.recover_from_journal(&journal).unwrap();
        resumed
            .advance_to(
                &mut journal,
                &mut drivers,
                LogicalTime::from_nanoseconds(remaining),
            )
            .unwrap();
        send(&mut resumed, &mut journal, 3);
        resumed.run_ready(&mut journal).unwrap();
        let second = InstanceAddress::new(
            &OwnerPath::root(),
            &[Segment::ProcessingTimeBox {
                scope: OwnerPath::root(),
                start: Timestamp::try_new(start_seconds + 2, 0).unwrap(),
            }],
        )
        .unwrap();
        assert!(journal.records().iter().any(
            |record| matches!(record, Record::Transaction(tx) if tx.instance.address() == &second)
        ));
        assert_eq!(
            rastok::runtime::data_value(resumed.outputs().last().unwrap().envelope.data()).unwrap(),
            Value::Int(1)
        );
    }
}

#[test]
fn an_unrepresentable_tumbling_start_refuses_before_admission() {
    let compilation = checked(
        r#"graph app : App {
            input in: int
            output out: int
            lifetime = { kind = tumbling; every = duration("7ns") }
            operator forward {
                input in: int
                output out: int
                on in { out = data }
            }
            in -> forward -> out
        }"#,
    );
    for (origin, now) in [
        (Timestamp::MIN, LogicalTime::ZERO),
        (Timestamp::MAX, LogicalTime::from_nanoseconds(1)),
    ] {
        let mut journal = MemJournal::new_with_origin(GraphVersion(1), origin);
        let mut execution = Execution::new_at(&compilation.plan, GraphVersion(1), origin, now);
        let result = execution.accept_input(
            &mut journal,
            IngressProposal {
                provenance: IngressProvenance::single(Path::new(["test"]).unwrap()),
                draft: EnvelopeDraft {
                    port: Port::new("in").unwrap(),
                    data: Data::materialized(Value::Int(1)),
                    ext: Namespace::new(),
                },
                sequence: IngressSequence::Next,
                session: None,
            },
        );
        assert!(
            matches!(result, Err(ExecutionFault::CivilTimeOutOfRange)),
            "origin {origin:?}, logical {now:?}: {result:?}"
        );
        assert!(journal.records().is_empty());
        assert!(execution.outputs().is_empty());
    }
}

#[test]
fn an_unrepresentable_civil_now_refuses_only_the_path_that_reads_it() {
    let compilation = checked(
        r#"
graph app : App {
  input in: int
  output out: struct { instant: timestamp }
  operator choose {
    input in: int
    output out: struct { instant: timestamp }
    on in match {
      data == 0 => out = { instant = timestamp("1970-01-01T00:00:00Z") }
      _ => out = { instant = now() }
    }
  }
  in -> choose -> out
}
"#,
    );
    let mut journal = MemJournal::new_with_origin(GraphVersion(1), Timestamp::MAX);
    let mut execution = Execution::new_at(
        &compilation.plan,
        GraphVersion(1),
        Timestamp::MAX,
        LogicalTime::from_nanoseconds(1),
    );
    send(&mut execution, &mut journal, 0);
    execution.run_ready(&mut journal).unwrap();
    assert_eq!(execution.outputs().len(), 1);
    send(&mut execution, &mut journal, 1);
    let count = journal.records().len();
    let error = execution.run_ready(&mut journal).unwrap_err();
    assert!(matches!(
        error,
        ExecutionFault::Fault(rastok::runtime::Fault::Eval(rastok::expr::Error {
            kind: rastok::expr::ErrorKind::TimeOutOfRange,
            ..
        }))
    ));
    assert_eq!(
        journal.records().len(),
        count,
        "no transaction commits an invented or truncated date"
    );
    assert_eq!(execution.outputs().len(), 1);
}

#[test]
fn a_different_origin_refuses_before_any_ingress_is_committed() {
    let compilation = checked(CLOCK);
    let mut journal = MemJournal::new_with_origin(GraphVersion(1), Timestamp::MIN);
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    let result = execution.accept_input(
        &mut journal,
        IngressProposal {
            provenance: IngressProvenance::single(Path::new(["test"]).unwrap()),
            draft: EnvelopeDraft {
                port: Port::new("in").unwrap(),
                data: Data::materialized(Value::Int(1)),
                ext: Namespace::new(),
            },
            sequence: IngressSequence::Next,
            session: None,
        },
    );
    assert!(matches!(result, Err(ExecutionFault::OriginMismatch)));
    assert!(journal.records().is_empty());
    assert!(execution.outputs().is_empty());
}

#[test]
fn an_unobserved_origin_does_not_change_a_causal_trace() {
    let compilation = checked("graph app : App { input in: int; output out: int; in -> out }");
    let mut traces = Vec::new();
    for origin in [Timestamp::UNIX_EPOCH, Timestamp::MIN, Timestamp::MAX] {
        let mut journal = MemJournal::new_with_origin(GraphVersion(1), origin);
        let mut execution = Execution::new_at(
            &compilation.plan,
            GraphVersion(1),
            origin,
            LogicalTime::ZERO,
        );
        send(&mut execution, &mut journal, 1);
        execution.run_ready(&mut journal).unwrap();
        traces.push(rastok::golden::project(journal.records()).seal_hex());
    }
    assert!(traces.windows(2).all(|pair| pair[0] == pair[1]));
}

#[test]
fn call_mocks_observe_the_scenarios_origin_plus_relative_time() {
    let source = r#"
graph app : App {
  connector echo : Process {
    command = "cat"
    codec = lines
    timeout = duration("1s")
    behavior = deterministic
    idempotent = true
  }
  in -> echo -> out
  echo.err -> nothing
  echo.exit -> nothing
}
lab app {
  simulation civil {
    mock echo {
      on in where now() == timestamp("1600-01-01T00:00:02Z") {
        out = "civil"
        exit = Exited { exit_code = 0, signal = null }
      }
    }
    scenario old {
      origin = timestamp("1600-01-01T00:00:00Z")
      advance duration("2s")
      app.in = "hello"
      expect app.out where data == "civil"
    }
  }
}
"#;
    let compilation = checked(source);
    let runs = rastok::lab::run(&compilation.plan, GraphVersion(1), &compilation.labs[0]);
    assert_eq!(runs.len(), 1);
    assert!(runs[0].report.passed(), "{:?}", runs[0].report);
}

#[test]
fn wide_civil_literals_survive_independent_plan_and_recording_readers() {
    let compilation = checked(
        r#"
graph app : App {
  input in: int
  output out: struct { early: timestamp, late: timestamp }
  operator dates {
    input in: int
    output out: struct { early: timestamp, late: timestamp }
    on in { out = { early = timestamp("0001-01-01T00:00:00Z"); late = timestamp("9999-12-31T23:59:59.999999999Z") } }
  }
  in -> dates -> out
}
"#,
    );
    let package = rastok::plan::encode(&compilation.plan, &compilation.hash);
    let (plan, hash) = rastok::plan::decode(&package).unwrap();
    assert_eq!(hash, compilation.hash);
    assert_eq!(plan, compilation.plan);
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&plan, GraphVersion(1));
    send(&mut execution, &mut journal, 1);
    execution.run_ready(&mut journal).unwrap();
    let expected = Value::Struct(vec![
        (
            "early".into(),
            Value::Timestamp(Timestamp::try_new(-62_135_596_800, 0).unwrap()),
        ),
        (
            "late".into(),
            Value::Timestamp(Timestamp::try_new(253_402_300_799, 999_999_999).unwrap()),
        ),
    ]);
    assert_eq!(
        rastok::runtime::data_value(execution.outputs()[0].envelope.data()).unwrap(),
        expected
    );
    let recording = Recording::capture(&compilation, &journal, LogicalTime::ZERO).unwrap();
    let decoded = Recording::decode(&recording.encode().unwrap()).unwrap();
    let (_, replayed) = decoded.inspect(decoded.records().len()).unwrap();
    assert_eq!(
        rastok::runtime::data_value(replayed.outputs()[0].envelope.data()).unwrap(),
        expected
    );
}
