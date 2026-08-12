// A test harness drives the OS on purpose; the frontier lints
// guard the core modules, not the evidence.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
//! The golden seal of the M0 probe: the canonical projection of a
//! full run, pinned byte for byte. `RASTOK_BLESS=1 cargo test` blesses
//! a new golden after an intentional behavior change — the diff in
//! the failure message names what moved, so blessing is a reviewed
//! decision, never a reflex.

use rastok::boundary::{
    CallOutcome, Draft, InboundSubmission, IngressAuthority, Intent, Partner, Response,
    SubmissionSequence,
};
use rastok::compiler::compile;
use rastok::contract::Registry;
use rastok::envelope::{Data, Encoding, Envelope, Namespace, Scalar, Value};
use rastok::golden::{Trace, diff, project};
use rastok::journal::{MemJournal, Record};
use rastok::node;
use rastok::plan::GraphVersion;
use rastok::runtime::vm::Vm;
use std::path::Path;

const M0_PROBE: &str = r#"
graph m0_probe : App {
  connector echo : Process {
    mode = job ; command = "cat"
    codec = lines
    effect = idempotent ; behavior = deterministic
  }

  operator shout {
    var count : int = 0
    on in where data.text != "" {
      count = count + 1
      out = data.text
    }
  }

  . -> shout -> echo -> .
}
"#;

/// The scripted world half: echoes every request, like `cat`.
#[derive(Default)]
struct EchoWorld {
    answers: Vec<Response>,
}

impl Partner for EchoWorld {
    fn start(&mut self, intent: &Intent, _request: &Envelope) {
        self.answers.push(Response {
            call_ref: intent.call_ref,
            outcome: CallOutcome::Success(Data::encoded(Encoding::Raw, *b"salut")),
        });
    }

    fn events(&mut self) -> Vec<Response> {
        std::mem::take(&mut self.answers)
    }
}

// A test harness helper: failure here is a broken fixture, and a
// panic is its correct report. (`allow-unwrap-in-tests` covers only
// `#[test]` bodies.)
#[allow(clippy::unwrap_used, clippy::expect_used)]
fn run(source: &str) -> Vec<Record> {
    let compilation = compile(&[source], &Registry::m0()).expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    let mut data = std::collections::BTreeMap::new();
    data.insert(
        String::from("text"),
        Value::Scalar(Scalar::Text(String::from("salut"))),
    );
    vm.inject(
        &mut journal,
        InboundSubmission {
            authority: IngressAuthority::single(node::Path::new(["cli"]).unwrap()),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Map(data)),
                ext: Namespace::new(),
            },
            sequence: SubmissionSequence::Next,
        },
    )
    .unwrap();
    vm.drain(&mut journal).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain(&mut journal).unwrap();
    assert!(vm.is_stable());
    journal.records().to_vec()
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

/// Parses a blessed golden file back into a trace: one hex line per
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
    if std::env::var("RASTOK_BLESS").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &rendered).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(&path)
        .expect("no golden trace; run once with RASTOK_BLESS=1 to bless");
    if golden != rendered {
        // Explain before failing: the first semantic divergence
        // against the STORED golden, by record and key path — never
        // a wall of hex.
        let divergence = diff(&project(&records), &stored_trace(&golden));
        panic!(
            "the trace diverged from its golden.\n\
             {divergence:?}\n\
             if the change is intentional, bless: RASTOK_BLESS=1 cargo test"
        );
    }
}

/// The Compare seam of the M0 slice: a completed Lab run — the DSL
/// scenario itself, not a hand-driven re-enactment — projects to a
/// canonical trace and matches its stored golden or explains why not.
#[test]
fn the_lab_scenario_matches_its_stored_golden() {
    let source = std::fs::read_to_string(example()).expect("the example reads");
    let compilation =
        rastok::compiler::compile(&[source.as_str()], &rastok::contract::Registry::m0())
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
    if std::env::var("RASTOK_BLESS").is_ok() {
        std::fs::write(&path, &rendered).expect("a writable golden dir");
        return;
    }
    let golden = std::fs::read_to_string(&path)
        .expect("no golden trace; run once with RASTOK_BLESS=1 to bless");
    if golden != rendered {
        let divergence = diff(&project(&roundtrip.records), &stored_trace(&golden));
        panic!(
            "the lab trace diverged from its golden.\n\
             {divergence:?}\n\
             if the change is intentional, bless: RASTOK_BLESS=1 cargo test"
        );
    }
}

/// A deliberately stale golden is named by the diff — the workflow
/// the blessing decision rests on, exercised end to end against a
/// STORED trace, not a second live run.
#[test]
fn a_stale_stored_golden_is_named_by_the_diff() {
    let records = run(M0_PROBE);
    let rendered = render(&records);
    // Age the stored golden: the inbound payload said something else.
    let stale = rendered.replace("73616c7574", "6175747265"); // "salut" -> "autre"
    assert_ne!(stale, rendered, "the alteration took");
    let divergence = diff(&project(&records), &stored_trace(&stale)).expect("the traces differ");
    assert_eq!(divergence.record, 0, "the inbound record");
    assert!(
        divergence.path.contains("data(8)"),
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
    assert_eq!(divergence.record, 1, "the shout transaction");
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
