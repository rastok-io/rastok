// A test harness drives the OS on purpose; the frontier lints
// guard the core modules, not the evidence.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
//! The golden seal of the M0 probe: the canonical projection of a
//! full run, pinned byte for byte. `RASTOK_SAVE_GOLDENS=1 cargo test` writes
//! a new golden after an intentional behavior change — the diff in
//! the failure message names what moved, so approval is a reviewed
//! decision, never a reflex.
//! The compact format-20 movement is enumerated before approval in
//! docs/architecture/human-review-v20-identity.md.

use rastok::compiler::compile;
use rastok::connector::contract::ContractCatalog;
use rastok::connector::driver::OutboundCall;
use rastok::connector::exchange::CallOpen;
use rastok::connector::exchange::CallResponse;
use rastok::connector::exchange::CallStart;
use rastok::connector::exchange::Completion;
use rastok::connector::exchange::EnvelopeDraft;
use rastok::connector::exchange::IngressProposal;
use rastok::connector::exchange::IngressProvenance;
use rastok::connector::exchange::IngressSequence;
use rastok::connector::exchange::RequestPreparation;
use rastok::connector::exchange::SchedulerCompletion;
use rastok::connector::stdlib::ProcessExit;
use rastok::envelope::{Data, Encoding, Envelope, Namespace, Value};
use rastok::golden::{Trace, diff, project};
use rastok::journal::{MemJournal, Record};
use rastok::node;
use rastok::plan::GraphVersion;
use rastok::runtime::execution::Execution;
use std::path::Path;

const M0_PROBE: &str = r#"
graph m0_probe : App {
  connector echo : Process {
    mode = job ; command = "cat"
    codec = lines
    idempotent = true; behavior = deterministic
  }

  operator shout {
    input in: struct { text: text }
    var count : int = 0
    on in where data.text != "" {
      count = count + 1
      out = data.text
    }
  }

  in -> shout -> echo -> out
  echo.err -> nothing
  echo.exit -> nothing
}
"#;

/// The scripted world half: echoes every request, like `cat`.
#[derive(Default)]
struct EchoWorld {
    answers: Vec<CallResponse>,
}

impl OutboundCall for EchoWorld {
    fn prepare_request(
        &mut self,
        _: &node::Path,
        data: &Data,
        _: &std::collections::BTreeMap<String, rastok::value::Value>,
    ) -> RequestPreparation {
        rastok::connector::stdlib::prepare_process_request(
            rastok::connector::stdlib::RequestCodec::Lines,
            data,
        )
    }

    fn scheduler_completion(
        &mut self,
        _: &node::Path,
        completion: SchedulerCompletion,
    ) -> Completion {
        rastok::connector::stdlib::process_scheduler_completion(completion)
    }

    fn start(&mut self, intent: &CallOpen, _request: &Envelope) -> CallStart {
        self.answers.push(CallResponse {
            call_id: intent.call_id,
            completion: Completion::process(
                vec![Data::encoded(Encoding::Raw, *b"salut")],
                vec![],
                ProcessExit::Exited {
                    exit_code: 0,
                    signal: None,
                },
            ),
        });
        CallStart::Started
    }

    fn poll_responses(&mut self) -> Vec<CallResponse> {
        std::mem::take(&mut self.answers)
    }
}

// A test harness helper: failure here is a broken fixture, and a
// panic is its correct report. (`allow-unwrap-in-tests` covers only
// `#[test]` bodies.)
#[allow(clippy::unwrap_used, clippy::expect_used)]
fn run(source: &str) -> Vec<Record> {
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Execution::new(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    let data = vec![(String::from("text"), Value::Text(String::from("salut")))];
    vm.accept_input(
        &mut journal,
        IngressProposal {
            provenance: IngressProvenance::single(node::Path::new(["cli"]).unwrap()),
            draft: EnvelopeDraft {
                port: rastok::node::Port::new("in").unwrap(),
                data: Data::materialized(Value::Struct(data)),
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
    vm.run_ready_with_bindings(&mut journal, &mut world)
        .unwrap();
    assert!(vm.is_stable());
    journal.records().to_vec()
}

#[test]
fn call_time_is_unsealed_but_retry_context_and_deadline_identity_are_causal() {
    use rastok::canonical_cbor::{self, Value as Cbor};
    use rastok::journal::{
        CallStatus, DeadlineKind, EgressRecord,
        deadline::{Deadline, Deadlines},
        retry_protocol as retry,
    };
    let original = run(M0_PROBE);
    let index = original
        .iter()
        .position(|record| {
            matches!(record,
        Record::Egress(EgressRecord::Call(call)) if matches!(call.status, CallStatus::Closed(_)))
        })
        .unwrap();
    let mut changed = original.clone();
    let Record::Egress(EgressRecord::Call(call)) = &mut changed[index] else {
        unreachable!()
    };
    call.closed_at = Some(99);
    assert_eq!(
        project(&original),
        project(&changed),
        "wall-independent causal facts do not seal a closure instant"
    );
    assert_ne!(
        rastok::journal::codec::encode(&original[index]),
        rastok::journal::codec::encode(&changed[index])
    );

    let Record::Egress(EgressRecord::Call(call)) = &mut changed[index] else {
        unreachable!()
    };
    let operation = retry::OperationId::from_id(rastok::envelope::Id::from_bytes([7; 16]));
    let context = retry::AttemptId {
        operation,
        ordinal: 1,
    };
    call.retry_record = Some(Box::new(retry::RetryRecord {
        context: Some(context),
        facts: vec![],
    }));
    let reference = retry::admission_ref(operation, 2, call.intent.call_id);
    call.deadlines = Some(Box::new(
        Deadlines::try_new(
            99,
            vec![Deadline {
                kind: DeadlineKind::RecurrenceDelay,
                reference,
                due_at: 199,
            }],
        )
        .unwrap(),
    ));
    let sealed = project(&changed);
    let (Cbor::Map(keys), _) = canonical_cbor::decode(sealed.record_bytes(index).unwrap()).unwrap()
    else {
        unreachable!()
    };
    assert_eq!(
        keys.get(&43),
        Some(
            &retry::RetryRecord {
                context: Some(context),
                facts: vec![]
            }
            .tree()
        )
    );
    assert_eq!(
        keys.get(&24),
        Some(&Cbor::Array(vec![Cbor::Array(vec![
            Cbor::Uint(4),
            Cbor::Bytes(reference.as_bytes().to_vec())
        ])]))
    );
    assert!(!keys.contains_key(&41) && !keys.contains_key(&44));
    let Record::Egress(EgressRecord::Call(call)) = &mut changed[index] else {
        unreachable!()
    };
    call.deadlines = Some(Box::new(
        Deadlines::try_new(
            99,
            vec![Deadline {
                kind: DeadlineKind::RecurrenceDelay,
                reference,
                due_at: 299,
            }],
        )
        .unwrap(),
    ));
    assert_eq!(
        sealed,
        project(&changed),
        "retained due time is not a causal timestamp"
    );
    let Record::Egress(EgressRecord::Call(call)) = &mut changed[index] else {
        unreachable!()
    };
    call.retry_record
        .as_mut()
        .unwrap()
        .context
        .as_mut()
        .unwrap()
        .ordinal = 2;
    assert_ne!(
        sealed,
        project(&changed),
        "the attempted operation is causal identity"
    );
}

/// Renders a trace for the golden file: one hex line per canonical
/// record, then the seal.
#[allow(clippy::unwrap_used)]
fn render(records: &[Record]) -> String {
    let trace = project(records);
    let mut out = String::new();
    for index in 0..trace.len() {
        let bytes = trace.record_bytes(index).unwrap();
        for byte in bytes {
            out.push_str(&format!("{byte:02x}"));
        }
        out.push('\n');
    }
    out.push_str("seal ");
    out.push_str(&trace.seal_hex());
    out.push('\n');
    out
}

/// Parses an approved golden file back into a trace: one hex line per
/// record, the trailing `seal` line ignored (it is derived).
#[allow(clippy::unwrap_used, clippy::expect_used)]
fn stored_trace(rendered: &str) -> Trace {
    let records: Vec<Vec<u8>> = rendered
        .lines()
        .filter(|line| !line.starts_with("seal ") && !line.is_empty())
        .map(|line| {
            (0..line.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(&line[at..at + 2], 16).expect("hex"))
                .collect()
        })
        .collect();
    Trace::from_stored(&records).expect("a golden this build can read")
}

#[test]
fn the_m0_probe_trace_matches_its_golden() {
    let records = run(M0_PROBE);
    let rendered = render(&records);
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/m0-probe.trace");
    if std::env::var("RASTOK_SAVE_GOLDENS").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &rendered).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(&path)
        .expect("no golden trace; run once with RASTOK_SAVE_GOLDENS=1 to save");
    if golden != rendered {
        // Explain before failing: the first semantic divergence
        // against the STORED golden, by record and key path — never
        // a wall of hex.
        let proposed = project(&records);
        let stored = stored_trace(&golden);
        let divergence = diff(&proposed, &stored);
        panic!(
            "the trace diverged from its golden.\n\
             {divergence:?}\n\
             records {} -> {}; seal {} -> {}\n\
             if the change is intentional, save: RASTOK_SAVE_GOLDENS=1 cargo test",
            stored.len(),
            proposed.len(),
            stored.seal_hex(),
            proposed.seal_hex()
        );
    }
}

/// The Compare seam of the M0 slice: a completed Lab run — the DSL
/// scenario itself, not a hand-driven re-enactment — projects to a
/// canonical trace and matches its stored golden or explains why not.
#[test]
fn the_lab_scenario_matches_its_stored_golden() {
    let source = std::fs::read_to_string(example()).expect("the example reads");
    let compilation = rastok::compiler::compile(
        &[source.as_str()],
        &rastok::connector::contract::ContractCatalog::try_m0()
            .expect("the standard catalogue holds"),
    )
    .expect("a valid unit");
    let runs = rastok::lab::run(
        &compilation.plan,
        rastok::plan::GraphVersion(1),
        &compilation.labs[0],
    );
    let roundtrip = runs
        .iter()
        .find(|run| run.report.scenario == "roundtrip")
        .expect("the roundtrip scenario");
    assert!(
        roundtrip.report.passed(),
        "{:#?}",
        roundtrip.report.failures
    );
    let rendered = render(&roundtrip.records);
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/echo-roundtrip.trace");
    if std::env::var("RASTOK_SAVE_GOLDENS").is_ok() {
        std::fs::write(&path, &rendered).expect("a writable golden dir");
        return;
    }
    let golden = std::fs::read_to_string(&path)
        .expect("no golden trace; run once with RASTOK_SAVE_GOLDENS=1 to save");
    if golden != rendered {
        let divergence = diff(&project(&roundtrip.records), &stored_trace(&golden));
        panic!(
            "the lab trace diverged from its golden.\n\
             {divergence:?}\n\
             if the change is intentional, save: RASTOK_SAVE_GOLDENS=1 cargo test"
        );
    }
}

/// A deliberately stale golden is named by the diff — the workflow
/// the approval decision rests on, exercised end to end against a
/// STORED trace, not a second live run.
#[test]
fn a_stale_stored_golden_is_named_by_the_diff() {
    let records = run(M0_PROBE);
    let rendered = render(&records);
    // Age the stored golden: the inbound payload said something else.
    let stale = rendered.replace("73616c7574", "6175747265"); // "salut" -> "autre"
    assert_ne!(stale, rendered, "the alteration took");
    let divergence = diff(&project(&records), &stored_trace(&stale)).expect("the traces differ");
    assert_eq!(divergence.record, 0, "the INGRESS record");
    assert!(
        divergence.path.contains("data(5)"),
        "the diff names the payload key: {divergence}"
    );
}

fn example() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/echo.rkg")
}

#[test]
fn an_intentional_change_is_named_by_the_diff() {
    let records = run(M0_PROBE);
    let changed = run(&M0_PROBE.replace("count = count + 1", "count = count + 2"));
    let divergence = diff(&project(&records), &project(&changed)).expect("the behaviors differ");
    // The first divergence is semantic and located: the operator's
    // transaction, its state delta — not an opaque byte offset.
    assert_eq!(
        divergence.record, 3,
        "root and Node l:start facts precede the shout transaction"
    );
    assert!(
        divergence.path.contains("state_delta"),
        "the diff names the changed table: {divergence}"
    );
    assert!(
        divergence.to_string().contains('≠'),
        "renders both sides: {divergence}"
    );
}

#[test]
fn identical_runs_share_one_seal() {
    let once = project(&run(M0_PROBE));
    let twice = project(&run(M0_PROBE));
    assert_eq!(once.seal(), twice.seal());
    assert!(diff(&once, &twice).is_none());
}

#[test]
fn the_readable_diff_labels_expected_actual_and_the_causal_trigger() {
    let plan = compile(&[M0_PROBE], &ContractCatalog::try_m0().unwrap())
        .unwrap()
        .plan;
    let expected = project(&run(M0_PROBE));
    let actual = project(&run(
        &M0_PROBE.replace("count = count + 1", "count = count + 2")
    ));
    let message = rastok::golden::explain_diff(&actual, &expected, &plan).unwrap();
    assert!(message.contains("expected 1; got 2"), "{message}");
    assert!(
        message.contains("cause: TRANSACTION \"shout\" · trigger \"p:in\""),
        "{message}"
    );
    assert!(message.contains("cause_id"));
    let empty = Trace::from_stored(&[]).unwrap();
    let missing = rastok::golden::explain_diff(&empty, &expected, &plan).unwrap();
    assert!(
        missing.contains("record count")
            && missing.contains("got 0")
            && missing.contains("cause: INGRESS"),
        "{missing}"
    );
}

#[test]
fn readable_golden_values_do_not_bypass_classified_data_policy() {
    let source = M0_PROBE.replace("text: text", "text: text @sensitive(level = confidential)");
    let plan = compile(&[&source], &ContractCatalog::try_m0().unwrap())
        .unwrap()
        .plan;
    let expected = project(&run(M0_PROBE));
    let actual = project(&run(
        &M0_PROBE.replace("count = count + 1", "count = count + 2")
    ));
    let message = rastok::golden::explain_diff(&actual, &expected, &plan).unwrap();
    assert!(
        message.contains("expected [REDACTED]; got [REDACTED]; cause: [REDACTED]"),
        "{message}"
    );
    assert!(
        message.contains("state_delta"),
        "the structural location stays useful"
    );
}
