//! The jsonl codec on `Process`, job mode (M1 item 2,
//! D-2026-08-11-1): one canonical JSON line out, decoded lines back
//! as `array<variant>`, exact numbers surviving the round trip, and
//! every refusal typed with the offending line.

#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
use rastok::compiler::compile;
use rastok::envelope::{Data, Namespace, Scalar, Value};
use rastok::journal::MemJournal;
use rastok::plan::GraphVersion;
use rastok::runtime::vm::Vm;
use rastok::stdlib::Registry;

/// Injects, drains, and pumps a REAL child process to quiescence.
fn drive(source: &str, data: Value) -> (Vec<rastok::runtime::vm::BoundaryOutput>, Vec<Value>) {
    let compilation = compile(&[source], &Registry::m0()).expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = rastok::connectors::Process::new(&compilation.plan);
    vm.inject(
        &mut journal,
        InboundSubmission {
            authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(data),
                ext: Namespace::new(),
            },
            sequence: SubmissionSequence::Next,
        },
    )
    .expect("the injection lands");
    vm.drain(&mut journal).expect("the request opens");
    for _ in 0..200 {
        if vm.is_stable() && !vm.has_open_obligations() {
            break;
        }
        vm.pump(&mut journal, &mut world)
            .expect("the world answers");
        vm.drain(&mut journal).expect("the reply routes");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(vm.is_stable(), "the run must settle");
    let errors = vm
        .errors()
        .iter()
        .map(|error| match error.data() {
            Data::Materialized(value) => value.clone(),
            Data::Encoded { bytes, .. } => {
                Value::Scalar(Scalar::Text(String::from_utf8_lossy(bytes).into_owned()))
            }
        })
        .collect();
    (vm.outputs().to_vec(), errors)
}

const CAT_JSONL: &str = "graph g : App {\n  connector shell : Process {\n    mode = job ; command = \"cat\"\n    codec = jsonl\n    effect = idempotent ; behavior = deterministic\n    timeout = duration(\"5s\")\n  }\n  . -> shell -> .\n}\n";

#[test]
fn a_structured_value_round_trips_through_a_real_process() {
    // `cat` echoes the canonical line back: the reply is the decoded
    // array of ONE variant, numbers exact — the decimal survives.
    let request = Value::Struct(vec![
        (
            String::from("price"),
            Value::Scalar(Scalar::Decimal {
                unscaled: 1999,
                scale: 2,
            }),
        ),
        (String::from("n"), Value::Scalar(Scalar::Int(42))),
    ]);
    let (outputs, errors) = drive(CAT_JSONL, request);
    assert_eq!(errors, Vec::<Value>::new(), "no fault");
    assert_eq!(outputs.len(), 1);
    let Data::Materialized(Value::Array(items)) = outputs[0].envelope.data() else {
        panic!(
            "a jsonl job replies array<variant>: {:?}",
            outputs[0].envelope.data()
        );
    };
    assert_eq!(items.len(), 1, "one line, one variant");
    let Value::Map(fields) = &items[0] else {
        panic!("the JSON model reads objects as maps");
    };
    assert_eq!(fields.get("n"), Some(&Value::Scalar(Scalar::Int(42))));
    assert_eq!(
        fields.get("price"),
        Some(&Value::Scalar(Scalar::Decimal {
            unscaled: 1999,
            scale: 2
        })),
        "the exact decimal survives the wire"
    );
}

#[test]
fn multiple_lines_decode_to_multiple_variants() {
    // A process emitting several JSON lines: each is one variant of
    // the reply array, in order.
    let source = "graph g : App {\n  connector shell : Process {\n    mode = job ; command = \"printf\"\n    args = [\"1\\n{\\\"a\\\":2}\\n[3]\\n\"]\n    codec = jsonl\n    effect = idempotent ; behavior = deterministic\n    timeout = duration(\"5s\")\n  }\n  . -> shell -> .\n}\n";
    let (outputs, errors) = drive(source, Value::Scalar(Scalar::Int(1)));
    assert_eq!(errors, Vec::<Value>::new(), "no fault");
    let Data::Materialized(Value::Array(items)) = outputs[0].envelope.data() else {
        panic!("array<variant>");
    };
    assert_eq!(items.len(), 3);
    assert_eq!(items[0], Value::Scalar(Scalar::Int(1)));
    assert!(
        matches!(&items[1], Value::Map(fields) if fields.get("a") == Some(&Value::Scalar(Scalar::Int(2))))
    );
    assert_eq!(items[2], Value::Array(vec![Value::Scalar(Scalar::Int(3))]));
}

#[test]
fn an_unparseable_line_is_a_typed_decode_error() {
    // `echo not-json` answers garbage: the reply is a DecodeError
    // carrying the offending line — never a silent skip.
    let source = "graph g : App {\n  connector shell : Process {\n    mode = job ; command = \"echo\"\n    args = [\"not json\"]\n    codec = jsonl\n    effect = idempotent ; behavior = deterministic\n    timeout = duration(\"5s\")\n  }\n  . -> shell -> .\n}\n";
    let (outputs, errors) = drive(source, Value::Scalar(Scalar::Int(1)));
    assert_eq!(outputs.len(), 0, "nothing crosses on out");
    assert_eq!(errors.len(), 1, "the typed error leaves on err");
    let Value::Map(fields) = &errors[0] else {
        panic!("a typed error payload");
    };
    assert_eq!(
        fields.get("kind"),
        Some(&Value::Scalar(Scalar::Text(String::from("DecodeError"))))
    );
    let Some(Value::Map(detail)) = fields.get("detail") else {
        panic!("the offending line rides the payload: {fields:?}");
    };
    let Some(Value::Scalar(Scalar::Text(message))) = detail.get("message") else {
        panic!("a message field: {detail:?}");
    };
    assert!(message.contains("not json"), "the line is named: {message}");
}

#[test]
fn an_unserializable_request_refuses_before_any_spawn() {
    // Bytes have no JSON form: the request refuses TYPED before any
    // open fact — no child ever spawns.
    let (outputs, errors) = drive(CAT_JSONL, Value::Scalar(Scalar::Bytes(vec![1, 2])));
    assert_eq!(outputs.len(), 0);
    assert_eq!(errors.len(), 1, "DecodeError on err");
    let Value::Map(fields) = &errors[0] else {
        panic!("typed");
    };
    assert_eq!(
        fields.get("kind"),
        Some(&Value::Scalar(Scalar::Text(String::from("DecodeError"))))
    );
}

#[test]
fn empty_stdout_is_the_empty_array() {
    let source = "graph g : App {\n  connector shell : Process {\n    mode = job ; command = \"true\"\n    codec = jsonl\n    effect = idempotent ; behavior = deterministic\n    timeout = duration(\"5s\")\n  }\n  . -> shell -> .\n}\n";
    let (outputs, errors) = drive(source, Value::Scalar(Scalar::Int(1)));
    assert_eq!(errors, Vec::<Value>::new());
    assert_eq!(
        outputs[0].envelope.data(),
        &Data::materialized(Value::Array(Vec::new())),
        "no lines, the empty array"
    );
}

#[test]
fn the_request_digest_is_canonical_across_spellings() {
    // Two by-name-equal structs — different write order — must open
    // ONE call identity: the canonical writer sorts keys, so the
    // digest cannot split what the seal unifies.
    use rastok::boundary::prepare_jsonl;
    let ab = Data::materialized(Value::Struct(vec![
        (String::from("a"), Value::Scalar(Scalar::Int(1))),
        (String::from("b"), Value::Scalar(Scalar::Int(2))),
    ]));
    let ba = Data::materialized(Value::Struct(vec![
        (String::from("b"), Value::Scalar(Scalar::Int(2))),
        (String::from("a"), Value::Scalar(Scalar::Int(1))),
    ]));
    assert_eq!(
        prepare_jsonl(&ab).unwrap().digest(),
        prepare_jsonl(&ba).unwrap().digest(),
        "one value, one digest"
    );
}

#[test]
fn a_mocked_jsonl_graph_runs_in_the_lab() {
    // The Lab replaces the process: the mock answers structured
    // values directly, and the graph's guards read them — the codec
    // is the WORLD's business, not the simulation's.
    let source = r#"
graph g : App {
  connector shell : Process {
    mode = job ; command = "cat"
    codec = jsonl
    effect = idempotent ; behavior = deterministic
  }

  operator sift {
    on in where data.status == "ok" { out = data.value }
  }

  . -> shell
  shell.out -> sift
  sift -> .
}

lab g {
  simulation probing on app {
    mock shell {
      on in { out = { status = "ok", value = 7 } }
    }

    scenario s "a structured reply routes by its fields" {
      app.in = { q = 1 }
      expect "the sifted value" app.out count == 1 where data == 7
    }
  }
}
"#;
    let compilation = compile(&[source], &Registry::m0()).expect("a valid unit");
    let runs = rastok::lab::run(
        &compilation.plan,
        rastok::plan::GraphVersion(1),
        &compilation.labs[0],
    );
    for run in &runs {
        assert!(run.report.passed(), "{:#?}", run.report.failures);
    }
}
