//! The jsonl codec on `Process`, job mode (M1 item 2,
//! D-2026-08-11-1): one canonical JSON line out, decoded lines back
//! as repeated `variant` emissions, exact numbers surviving the round trip, and
//! every refusal typed with the offending line.

#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
use rastok::compiler::compile;
use rastok::contract::Registry;
use rastok::envelope::{Data, Namespace, Value};
use rastok::journal::MemJournal;
use rastok::plan::GraphVersion;
use rastok::runtime::vm::Vm;

/// Injects, drains, and pumps a REAL child process to quiescence.
fn drive(
    source: &str,
    data: Value,
) -> (
    Vec<rastok::runtime::vm::BoundaryOutput>,
    Vec<Value>,
    Vec<Value>,
) {
    let compilation = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world =
        rastok::connectors::Process::try_new(&compilation.plan).expect("the plan binds");
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
            Data::Encoded { bytes, .. } => Value::Text(String::from_utf8_lossy(bytes).into_owned()),
        })
        .collect();
    let exits = vm
        .observations()
        .iter()
        .filter(|observed| observed.node.is_none() && observed.port.name() == "exit")
        .map(|observed| match observed.envelope.data() {
            Data::Materialized(value) => value.clone(),
            Data::Encoded { .. } => panic!("Process.exit is materialized"),
        })
        .collect();
    (vm.outputs().to_vec(), errors, exits)
}

fn assert_exit<'a>(exits: &'a [Value], expected: &str) -> &'a [(String, Value)] {
    assert_eq!(exits.len(), 1, "one terminal fact per Process invocation");
    let Value::TaggedUnion(tagged) = &exits[0] else {
        panic!("Process.exit carries ProcessExit");
    };
    assert_eq!(tagged.case, expected);
    &tagged.payload
}

const CAT_JSONL: &str = "graph g : App {\n  connector shell : Process {\n    mode = job ; command = \"cat\"\n    codec = jsonl\n    effect = idempotent ; behavior = deterministic\n    timeout = duration(\"5s\")\n  }\n  . -> shell -> .\n}\n";

#[test]
fn a_structured_value_round_trips_through_a_real_process() {
    // `cat` echoes the canonical line back: the reply is the decoded
    // one variant emission, numbers exact — the decimal survives.
    let request = Value::Struct(vec![
        (
            String::from("price"),
            Value::Decimal {
                unscaled: 1999,
                scale: 2,
            },
        ),
        (String::from("n"), Value::Int(42)),
    ]);
    let (outputs, errors, exits) = drive(CAT_JSONL, request);
    assert_eq!(errors, Vec::<Value>::new(), "no fault");
    assert_eq!(outputs.len(), 1);
    let Data::Materialized(Value::Map(fields)) = outputs[0].envelope.data() else {
        panic!("the JSON model reads objects as maps");
    };
    assert_eq!(fields.get("n"), Some(&Value::Int(42)));
    assert_eq!(
        fields.get("price"),
        Some(&Value::Decimal {
            unscaled: 1999,
            scale: 2
        }),
        "the exact decimal survives the wire"
    );
    assert_exit(&exits, "Exited");
}

#[test]
fn multiple_lines_decode_to_multiple_variants() {
    // A process emitting several JSON lines: each is one ordinary `out`
    // emission, in order.
    let source = "graph g : App {\n  connector shell : Process {\n    mode = job ; command = \"printf\"\n    args = [\"1\\n{\\\"a\\\":2}\\n[3]\\n\"]\n    codec = jsonl\n    effect = idempotent ; behavior = deterministic\n    timeout = duration(\"5s\")\n  }\n  . -> shell -> .\n}\n";
    let (outputs, errors, exits) = drive(source, Value::Int(1));
    assert_eq!(errors, Vec::<Value>::new(), "no fault");
    let items: Vec<&Value> = outputs
        .iter()
        .map(|output| match output.envelope.data() {
            Data::Materialized(value) => value,
            Data::Encoded { .. } => panic!("jsonl materializes its frames"),
        })
        .collect();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0], &Value::Int(1));
    assert!(matches!(items[1], Value::Map(fields) if fields.get("a") == Some(&Value::Int(2))));
    assert_eq!(items[2], &Value::Array(vec![Value::Int(3)]));
    assert_exit(&exits, "Exited");
}

#[test]
fn an_unparseable_line_is_a_typed_decode_error() {
    // `echo not-json` answers garbage: the terminal fact is DecodeFailed
    // carrying the offending line — never a silent skip.
    let source = "graph g : App {\n  connector shell : Process {\n    mode = job ; command = \"echo\"\n    args = [\"not json\"]\n    codec = jsonl\n    effect = idempotent ; behavior = deterministic\n    timeout = duration(\"5s\")\n  }\n  . -> shell -> .\n}\n";
    let (outputs, errors, exits) = drive(source, Value::Int(1));
    assert_eq!(outputs.len(), 0, "nothing crosses on out");
    assert!(errors.is_empty(), "stderr is independent of decoding");
    let detail = assert_exit(&exits, "DecodeFailed");
    let Some((_, Value::Text(message))) = detail.iter().find(|(name, _)| name == "message") else {
        panic!("a message field: {detail:?}");
    };
    assert!(
        message.contains("does not parse"),
        "the refusal is named: {message}"
    );
    assert_eq!(
        detail.iter().find(|(name, _)| name == "line"),
        Some(&(
            String::from("line"),
            Value::StructuralUnion {
                branch: 1,
                value: Box::new(Value::Text(String::from("not json"))),
            }
        )),
        "the offending line remains structured data"
    );
}

#[test]
fn an_unserializable_request_refuses_before_any_spawn() {
    // Bytes have no JSON form: the request refuses TYPED before any
    // open fact — no child ever spawns.
    let (outputs, errors, exits) = drive(CAT_JSONL, Value::Bytes(vec![1, 2]));
    assert_eq!(outputs.len(), 0);
    assert!(errors.is_empty());
    assert_exit(&exits, "DecodeFailed");
}

#[test]
fn empty_stdout_emits_no_out_frame() {
    let source = "graph g : App {\n  connector shell : Process {\n    mode = job ; command = \"true\"\n    codec = jsonl\n    effect = idempotent ; behavior = deterministic\n    timeout = duration(\"5s\")\n  }\n  . -> shell -> .\n}\n";
    let (outputs, errors, exits) = drive(source, Value::Int(1));
    assert_eq!(errors, Vec::<Value>::new());
    assert!(outputs.is_empty(), "no line means no out envelope");
    assert_exit(&exits, "Exited");
}

#[test]
fn the_request_digest_is_canonical_across_spellings() {
    // Two by-name-equal structs — different write order — must open
    // ONE call identity: the canonical writer sorts keys, so the
    // digest cannot split what the seal unifies.
    use rastok::boundary::prepare_jsonl;
    let ab = Data::materialized(Value::Struct(vec![
        (String::from("a"), Value::Int(1)),
        (String::from("b"), Value::Int(2)),
    ]));
    let ba = Data::materialized(Value::Struct(vec![
        (String::from("b"), Value::Int(2)),
        (String::from("a"), Value::Int(1)),
    ]));
    assert_eq!(
        prepare_jsonl(&ab).unwrap().digest(),
        prepare_jsonl(&ba).unwrap().digest(),
        "one value, one digest"
    );
}

/// The one sift graph BOTH worlds run: each jsonl line is one `variant`
/// delivery — in the Lab against a mock, and against a real `cat`.
const SIFT_GRAPH: &str = r#"
graph g : App {
  connector shell : Process {
    mode = job ; command = "cat"
    codec = jsonl
    effect = idempotent ; behavior = deterministic
    timeout = duration("5s")
  }

  operator sift {
    on in where data["status"] == "ok" { out = data["value"] }
  }

  . -> shell
  shell.out -> sift
  sift -> .
}
"#;

#[test]
fn a_mocked_jsonl_graph_runs_in_the_lab() {
    // The Lab replaces the process — but the mock must speak the
    // WIRE's unit shape (`variant`): the reply is
    // written as the real decode would deliver it, and the graph
    // read the same way in both worlds.
    let source = format!(
        r#"{SIFT_GRAPH}
lab g {{
  simulation probing on app {{
    mock shell {{
      on in {{
        out = {{"status": "ok", "value": 7}}
        exit = Exited {{ exit_code = 0, signal = null }}
      }}
    }}

    scenario s "a structured reply routes by its fields" {{
      app.in = {{ q = 1 }}
      expect "the sifted value" app.out count == 1 where data == 7
    }}
  }}
}}
"#
    );
    let compilation = compile(
        &[source.as_str()],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid unit");
    let runs = rastok::lab::run(
        &compilation.plan,
        rastok::plan::GraphVersion(1),
        &compilation.labs[0],
    );
    for run in &runs {
        assert!(run.report.passed(), "{:#?}", run.report.failures);
    }
}

#[test]
fn the_same_sift_graph_runs_against_a_real_child() {
    // `cat` echoes the canonical line; the decode delivers
    // `{"status": "ok", "value": 7}` and the SAME graph the mock
    // satisfied sifts the real reply — mock and world, one shape.
    let mut fields = std::collections::BTreeMap::new();
    fields.insert(String::from("status"), Value::Text(String::from("ok")));
    fields.insert(String::from("value"), Value::Int(7));
    let (outputs, errors, exits) = drive(SIFT_GRAPH, Value::Map(fields));
    assert_eq!(errors, Vec::<Value>::new(), "no fault");
    assert_eq!(outputs.len(), 1);
    assert_eq!(
        outputs[0].envelope.data(),
        &Data::materialized(Value::Int(7)),
        "the sifted value crossed both worlds identically"
    );
    assert_exit(&exits, "Exited");
}

#[test]
fn a_record_mock_reply_is_one_jsonl_variant_not_a_batch() {
    // B7 removes the batch carrier: one record is one variant emission.
    let source = format!(
        r#"{SIFT_GRAPH}
lab g {{
  simulation probing on app {{
    mock shell {{
      on in {{
        out = {{ status = "ok", value = 7 }}
        exit = Exited {{ exit_code = 0, signal = null }}
      }}
    }}

    scenario s "never runs" {{
      app.in = {{ q = 1 }}
      expect "unreachable" app.out count == 1
    }}
  }}
}}
"#
    );
    let compilation = compile(
        &[source.as_str()],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("one record inhabits the jsonl variant pin");
    let runs = rastok::lab::run(
        &compilation.plan,
        rastok::plan::GraphVersion(1),
        &compilation.labs[0],
    );
    assert!(runs[0].report.passed(), "{:#?}", runs[0].report.failures);
}

#[test]
fn a_null_line_is_the_null_value() {
    // `null` is a JSON value like any other (D-2026-08-11-1): a
    // line carrying it decodes to the null element, never a
    // DecodeFailed.
    let source = "graph g : App {\n  connector shell : Process {\n    mode = job ; command = \"printf\"\n    args = [\"null\\n\"]\n    codec = jsonl\n    effect = idempotent ; behavior = deterministic\n    timeout = duration(\"5s\")\n  }\n  . -> shell -> .\n}\n";
    let (outputs, errors, exits) = drive(source, Value::Int(1));
    assert_eq!(errors, Vec::<Value>::new(), "no fault");
    assert_eq!(
        outputs[0].envelope.data(),
        &Data::materialized(Value::Null),
        "null rides its own frame"
    );
    assert_exit(&exits, "Exited");
}

#[test]
fn json_null_is_an_executable_value() {
    // `json` takes documents only (D-2026-08-13-1), and a BARE
    // `null` emission stays deferred with the null static type —
    // but null inside a document must EXECUTE, not merely compile.
    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in { out = json! [null] }\n  }\n  . -> o -> .\n}\n";
    let (outputs, errors, _) = drive(source, Value::Int(1));
    assert_eq!(errors, Vec::<Value>::new(), "no fault");
    assert_eq!(
        outputs[0].envelope.data(),
        &Data::materialized(Value::Array(vec![Value::Null])),
        "the null value crosses"
    );
}

#[test]
fn an_app_token_survives_the_world() {
    // Uniform propagation (D-2026-08-10-2) across a REAL call: a
    // token explicitly kept in meta.app before the request must
    // still be there on the reply — the exact idiom that replaced
    // `last`.
    let source = r#"
graph g : App {
  connector shell : Process {
    mode = job ; command = "cat"
    codec = jsonl
    effect = idempotent ; behavior = deterministic
    timeout = duration("5s")
  }

  operator stamp {
    on in { out = { data = data, meta.app.token = "tok" } }
  }

  operator probe {
    on in where meta.app.token == "tok" { out = meta.app.token }
  }

  . -> stamp -> shell
  shell.out -> probe
  probe -> .
}
"#;
    let (outputs, errors, exits) = drive(source, Value::Int(1));
    assert_eq!(errors, Vec::<Value>::new(), "no fault");
    assert_eq!(outputs.len(), 1, "the token survived the call");
    assert_eq!(
        outputs[0].envelope.data(),
        &Data::materialized(Value::Text(String::from("tok")))
    );
    assert_exit(&exits, "Exited");
}

#[test]
fn an_app_token_survives_the_mocked_world_too() {
    // The SAME propagation rule holds in the Lab: a mock's reply
    // envelope inherits the request's app zone, so the token idiom
    // simulates exactly as it runs.
    let source = r#"
graph g : App {
  connector shell : Process {
    mode = job ; command = "cat"
    codec = jsonl
    effect = idempotent ; behavior = deterministic
    timeout = duration("5s")
  }

  operator stamp {
    on in { out = { data = data, meta.app.token = "tok" } }
  }

  operator probe {
    on in where meta.app.token == "tok" { out = meta.app.token }
  }

  . -> stamp -> shell
  shell.out -> probe
  probe -> .
}

lab g {
  simulation probing on app {
    mock shell {
      on in {
        out = 1
        exit = Exited { exit_code = 0, signal = null }
      }
    }

    scenario s "the token survives the mock" {
      app.in = 1
      expect "the token on out" app.out count == 1 where data == "tok"
    }
  }
}
"#;
    let compilation = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid unit");
    let runs = rastok::lab::run(
        &compilation.plan,
        rastok::plan::GraphVersion(1),
        &compilation.labs[0],
    );
    for run in &runs {
        assert!(run.report.passed(), "{:#?}", run.report.failures);
    }
}

#[test]
fn an_out_of_domain_decimal_refuses_at_the_door() {
    // A public injection carrying an unbounded scale must refuse
    // TYPED at the door — never reach a writer as a runaway
    // allocation (D-2026-08-10-7).
    let compilation = compile(
        &[CAT_JSONL],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let refused = vm.inject(
        &mut journal,
        InboundSubmission {
            authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Decimal {
                    unscaled: 5,
                    scale: i32::MIN,
                }),
                ext: Namespace::new(),
            },
            sequence: SubmissionSequence::Next,
        },
    );
    assert!(refused.is_err(), "the door refuses, instantly and typed");
}

#[test]
fn a_validated_plan_never_carries_an_unknown_codec() {
    // The validator holds the codec's domain: a crafted package
    // with `codec = bogus` must never earn a ValidatedExecutionPlan
    // and quietly run as `lines`.
    let compilation = compile(
        &[CAT_JSONL],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut plan: rastok::plan::ExecutionPlan = (*compilation.plan).clone();
    for node in &mut plan.nodes {
        if let rastok::plan::NodeKind::Connector(spec) = &mut node.kind {
            for (name, setting) in &mut spec.attributes {
                if name == "codec" {
                    *setting = rastok::plan::AttrSetting::Word(String::from("bogus"));
                }
            }
        }
    }
    let refused = rastok::plan::ValidatedExecutionPlan::try_from(plan);
    assert!(
        matches!(
            refused,
            Err(rastok::plan::PlanValidationError::UnknownCodec { .. })
        ),
        "the domain holds: {refused:?}"
    );
}

#[test]
fn a_process_plan_without_a_codec_never_validates() {
    // The sheet REQUIRES codec, so a Process plan without one never
    // left the compiler: a crafted package omitting it must refuse
    // instead of silently running as `lines`.
    let compilation = compile(
        &[CAT_JSONL],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut plan: rastok::plan::ExecutionPlan = (*compilation.plan).clone();
    for node in &mut plan.nodes {
        if let rastok::plan::NodeKind::Connector(spec) = &mut node.kind {
            spec.attributes.retain(|(name, _)| name != "codec");
        }
    }
    let refused = rastok::plan::ValidatedExecutionPlan::try_from(plan);
    assert!(
        matches!(
            refused,
            Err(rastok::plan::PlanValidationError::MissingCodec { .. })
        ),
        "presence is part of validity: {refused:?}"
    );
}

#[test]
fn another_contracts_codec_vocabulary_is_untouched() {
    // The codec rule is the PROCESS contract's, not the attribute
    // name's: a sheet spelling its own `codec = json` must not be
    // refused by Process's domain.
    let compilation = compile(
        &[CAT_JSONL],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut plan: rastok::plan::ExecutionPlan = (*compilation.plan).clone();
    for node in &mut plan.nodes {
        if let rastok::plan::NodeKind::Connector(spec) = &mut node.kind {
            spec.contract = String::from("Custom");
            for (name, setting) in &mut spec.attributes {
                if name == "codec" {
                    *setting = rastok::plan::AttrSetting::Word(String::from("json"));
                }
            }
            node.ports = vec![
                rastok::plan::PortDecl {
                    port: rastok::node::Port::new("in").expect("a conventional port"),
                    direction: rastok::node::Direction::Input,
                    unwired: rastok::plan::UnwiredPolicy::RequireWire,
                    ty: Some(rastok::datatype::DataType::Variant),
                },
                rastok::plan::PortDecl {
                    port: rastok::node::Port::new("out").expect("a conventional port"),
                    direction: rastok::node::Direction::Output,
                    unwired: rastok::plan::UnwiredPolicy::RequireWire,
                    ty: Some(rastok::datatype::DataType::array(
                        rastok::datatype::DataType::Variant,
                    )),
                },
            ];
            node.ports
                .sort_by(|left, right| left.key().cmp(&right.key()));
        }
    }
    let validated = rastok::plan::ValidatedExecutionPlan::try_from(plan);
    assert!(
        validated.is_ok(),
        "another sheet's vocabulary validates: {:?}",
        validated.err()
    );
}
