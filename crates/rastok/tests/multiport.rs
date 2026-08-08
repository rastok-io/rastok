//! Multiport execution — the first M1 lift: envelopes enter through
//! DECLARED boundary inputs and fan out through named node ports and
//! named boundary outputs, end to end in the Lab. The refusals stay
//! typed: a wrong-direction injection, an unknown input, a source-
//! connector injection (still deferred), and the machine's own
//! verified ingress. The inbound record carries its port as key 24,
//! omitted for the default `in` (identity.md §3, D-2026-08-08-2).

#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::compiler::compile;
use rastok::stdlib::Registry;

/// One input fanning to two named outputs through named node ports.
const SORTER: &str = r#"
graph sorter : App {
  operator route {
    on in {
      hot = data
      cold = data
    }
  }

  . -> route
  route.hot -> .high
  route.cold -> .low
  output high
  output low
}

lab sorter {
  simulation probing on app {
    scenario split "one input fans to both named outputs" {
      app.in = {v = 1}
      expect "hot side" app.high count == 1
      expect "cold side" app.low count == 1
    }
  }
}
"#;

/// Two declared inputs, each reaching its own handler.
const MERGER: &str = r#"
graph merge : App {
  input alpha
  input beta

  operator join {
    var seen : int = 0
    on a {
      seen = seen + 1
      out = data
    }
    on b {
      seen = seen + 1
      out = data
    }
  }

  .alpha -> join.a
  .beta -> join.b
  join -> .
}

lab merge {
  simulation probing on app {
    scenario both "each named input reaches its handler" {
      app.alpha = {v = 1}
      expect "first" app.out count == 1
      app.beta = {v = 2}
      expect "second" app.out count == 1
    }
  }
}
"#;

fn lab_passes(source: &str) {
    let compilation = compile(&[source], &Registry::m0()).expect("a valid unit");
    let runs = rastok::lab::run(
        &compilation.plan,
        rastok::plan::GraphVersion(1),
        &compilation.labs[0],
    );
    for run in &runs {
        assert!(run.report.passed(), "{:#?}", run.report.failures);
    }
    assert!(!runs.is_empty(), "the lab ran its scenarios");
}

#[test]
fn named_outputs_fan_out() {
    lab_passes(SORTER);
}

#[test]
fn named_inputs_reach_their_handlers() {
    lab_passes(MERGER);
}

fn refusal_code(graph: &str, injection: &str) -> Vec<&'static str> {
    let source = format!(
        "{graph}\nlab merge {{\n  simulation probing on app {{\n    scenario s \"x\" {{\n      \
         {injection} = {{v = 1}}\n      expect \"y\" app.out count == 1\n    }}\n  }}\n}}\n"
    );
    let bag = compile(&[source.as_str()], &Registry::m0()).expect_err("the scenario refuses");
    bag.diagnostics()
        .iter()
        .map(|diagnostic| diagnostic.code().as_str())
        .collect()
}

const MERGER_GRAPH: &str = r#"
graph merge : App {
  input alpha

  operator join {
    on a {
      out = data
    }
  }

  .alpha -> join.a
  join -> .
}
"#;

#[test]
fn an_injection_at_an_output_refuses_by_direction() {
    // The implicit `out`, and a graph output generally, is not an
    // ingress — the world cannot write the graph's own answers.
    assert!(
        refusal_code(MERGER_GRAPH, "app.out").contains(&"E-PORT-DIRECTION"),
        "{:?}",
        refusal_code(MERGER_GRAPH, "app.out")
    );
}

#[test]
fn an_injection_at_an_unknown_input_refuses_by_name() {
    assert!(
        refusal_code(MERGER_GRAPH, "app.ghost").contains(&"E-PORT-UNKNOWN"),
        "{:?}",
        refusal_code(MERGER_GRAPH, "app.ghost")
    );
}

#[test]
fn a_source_connector_injection_stays_deferred() {
    // Injection at a node keeps refusing BY NAME — it joins with the
    // source adapters, not with this lift.
    assert!(
        refusal_code(MERGER_GRAPH, "join.a").contains(&"E-CHECK-DEFERRED"),
        "{:?}",
        refusal_code(MERGER_GRAPH, "join.a")
    );
}

#[test]
fn the_machine_verifies_the_ingress_port() {
    // The raw API is verified, not trusted: a submission at a port
    // that is not a graph input refuses typed, before any journal
    // append.
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, Namespace, Scalar, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::{Vm, VmError};

    let compilation = compile(&[MERGER_GRAPH], &Registry::m0()).expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let submission = |port: &str| InboundSubmission {
        authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
        port: rastok::node::Port::new(port).unwrap(),
        draft: Draft {
            data: Data::materialized(Value::Scalar(Scalar::Text(String::from("go")))),
            mime_type: None,
            app: Namespace::new(),
        },
        sequence: SubmissionSequence::Next,
    };
    assert!(matches!(
        vm.inject(&mut journal, submission("out")).unwrap_err(),
        VmError::UnknownIngress(port) if port.name() == "out"
    ));
    assert!(matches!(
        vm.inject(&mut journal, submission("ghost")).unwrap_err(),
        VmError::UnknownIngress(port) if port.name() == "ghost"
    ));
    // The declared input accepts.
    vm.inject(&mut journal, submission("alpha"))
        .expect("a declared input ingresses");
}

#[test]
fn the_inbound_record_carries_its_port_and_omits_the_default() {
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::canonical_cbor::{Value as Cbor, decode};
    use rastok::envelope::{Data, Encoding, Namespace};
    use rastok::journal::{Journal, MemJournal};
    use rastok::plan::GraphVersion;

    let mut journal = MemJournal::new(GraphVersion(1));
    let submission = |port: &str| InboundSubmission {
        authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
        port: rastok::node::Port::new(port).unwrap(),
        draft: Draft {
            data: Data::encoded(Encoding::Raw, *b"go"),
            mime_type: None,
            app: Namespace::new(),
        },
        sequence: SubmissionSequence::Next,
    };
    journal.append_inbound(submission("in")).unwrap();
    journal.append_inbound(submission("alpha")).unwrap();
    let trace = rastok::golden::project(journal.records());
    let key_24 = |index: usize| {
        let bytes = trace.record_bytes(index).expect("a projected record");
        let (tree, _) = decode(bytes).expect("canonical bytes");
        let Cbor::Map(pairs) = tree else {
            panic!("a record map");
        };
        pairs.get(&24).cloned()
    };
    assert_eq!(
        key_24(0),
        None,
        "the default `in` is omitted — pre-multiport traces hold"
    );
    assert_eq!(
        key_24(1),
        Some(Cbor::Text(String::from("alpha"))),
        "a named ingress is sealed as key 24"
    );
}

// ===== typed ports — the annotation is a public contract =====

/// A pinned input types the handler's `data`; the emission is `int`
/// through `size`, satisfying the pinned output — and the whole
/// contract rides the Lab.
const TYPED: &str = r#"
graph typed : App {
  input feed : text

  operator gauge {
    input evt : text
    output score : int

    on evt {
      score = size(data)
    }
  }

  .feed -> gauge.evt
  gauge.score -> .
}

lab typed {
  simulation probing on app {
    scenario measures "text in, its size out" {
      app.feed = "salut"
      expect "sized" app.out count == 1 where data == 5
    }
  }
}
"#;

#[test]
fn a_typed_graph_rides_the_lab() {
    lab_passes(TYPED);
}

#[test]
fn a_pinned_input_types_the_handler_data() {
    // `data` is TEXT in this handler — multiplying it must refuse at
    // check; with the old `dyn`, this compiled and faulted at run.
    let source = "graph g : App {\n  operator o {\n    input evt : text\n    on evt { out = data * 2 }\n  }\n  . -> o.evt\n  o -> .\n}\n";
    let bag = compile(&[source], &Registry::m0()).expect_err("text times int refuses");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-CEL-EXPR"),
        "{bag:?}"
    );
}

#[test]
fn an_emission_against_a_pinned_output_refuses() {
    let source = "graph g : App {\n  operator o {\n    output score : int\n    on in { score = \"nope\" }\n  }\n  . -> o\n  o.score -> .\n}\n";
    let bag = compile(&[source], &Registry::m0()).expect_err("a text emission into an int port");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-PORT-TYPE"),
        "{bag:?}"
    );
}

#[test]
fn mismatched_pinned_edge_ends_refuse() {
    let source = "graph g : App {\n  operator a {\n    output score : int\n    on in { score = 1 }\n  }\n  operator b {\n    input evt : text\n    on evt { out = data }\n  }\n  . -> a\n  a.score -> b.evt\n  b -> .\n}\n";
    let bag = compile(&[source], &Registry::m0()).expect_err("int wired into text");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-PORT-TYPE"),
        "{bag:?}"
    );
}

#[test]
fn a_typed_injection_refuses_by_contract() {
    let source = "graph g : App {\n  input alpha : int\n\n  operator o {\n    on a { out = data }\n  }\n  .alpha -> o.a\n  o -> .\n}\n\nlab g {\n  simulation probing on app {\n    scenario s \"x\" {\n      app.alpha = \"text\"\n      expect \"y\" app.out count == 1\n    }\n  }\n}\n";
    let bag = compile(&[source], &Registry::m0()).expect_err("a text injection into an int input");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-PORT-TYPE"),
        "{bag:?}"
    );
}

#[test]
fn typed_ports_travel_in_the_package() {
    use rastok::plan;
    let compilation = compile(&[TYPED], &Registry::m0()).expect("a valid plan");
    let gauge = compilation
        .plan
        .plan()
        .nodes
        .iter()
        .find(|node| node.path.to_string() == "gauge")
        .expect("the operator");
    assert!(
        gauge
            .ports
            .iter()
            .any(|decl| decl.port.name() == "evt" && decl.ty == Some(rastok::cel::Type::Text)),
        "the pinned input travels: {:?}",
        gauge.ports
    );
    assert!(
        compilation
            .plan
            .plan()
            .boundary
            .iter()
            .any(|decl| decl.port.name() == "feed" && decl.ty == Some(rastok::cel::Type::Text)),
        "the pinned boundary input travels: {:?}",
        compilation.plan.plan().boundary
    );
    let bytes = plan::encode(&compilation.plan, &compilation.hash);
    let (decoded, declared) = plan::decode(&bytes).expect("the typed package opens");
    assert_eq!(decoded, compilation.plan, "types included");
    assert_eq!(declared, compilation.hash);
}

#[test]
fn the_annotation_is_identity() {
    // Pinning a type is a public contract — two graphs differing only
    // by the annotation are two identities.
    let annotated = "graph g : App {\n  operator o {\n    input evt : text\n    on evt { out = data }\n  }\n  . -> o.evt\n  o -> .\n}\n";
    let bare = annotated.replace(" : text", "");
    let one = compile(&[annotated], &Registry::m0()).expect("a valid plan");
    let two = compile(&[bare.as_str()], &Registry::m0()).expect("a valid plan");
    assert_ne!(one.hash, two.hash, "the annotation is identity material");
    assert_ne!(one.plan, two.plan);
}

// ===== observation direction — no false green at the boundary =====

fn expect_refusal(expectation: &str) -> Vec<&'static str> {
    let source = format!(
        "{MERGER_GRAPH}\nlab merge {{\n  simulation probing on app {{\n    scenario s \"x\" {{\n      \
         app.alpha = {{v = 1}}\n      {expectation}\n    }}\n  }}\n}}\n"
    );
    let bag = compile(&[source.as_str()], &Registry::m0()).expect_err("the expectation refuses");
    bag.diagnostics()
        .iter()
        .map(|diagnostic| diagnostic.code().as_str())
        .collect()
}

#[test]
fn an_expectation_on_an_input_refuses_by_direction() {
    // The reviewer's false green: the machine logs nothing at an
    // ingress, so `count == 0` there was vacuously true. An input as
    // an observation target now refuses like any wrong direction.
    assert!(
        expect_refusal("expect \"vacuous\" app.alpha count == 0").contains(&"E-PORT-DIRECTION"),
        "{:?}",
        expect_refusal("expect \"vacuous\" app.alpha count == 0")
    );
    assert!(
        expect_refusal("expect \"implicit\" app.in count == 0").contains(&"E-PORT-DIRECTION"),
        "{:?}",
        expect_refusal("expect \"implicit\" app.in count == 0")
    );
}

// ===== the conformance matrix — one notion, every EmitValue form =====

fn gauge_graph(body: &str) -> String {
    format!(
        "graph g : App {{\n  operator o {{\n    input evt : text\n    output score : int\n\n    \
         on evt {{\n      {body}\n    }}\n  }}\n\n  . -> o.evt\n  o.score -> .\n}}\n"
    )
}

fn conformance_refuses(body: &str) {
    let source = gauge_graph(body);
    let bag = compile(&[source.as_str()], &Registry::m0()).expect_err("the pin holds");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-PORT-TYPE"),
        "`{body}` must breach: {bag:?}"
    );
}

#[test]
fn every_emit_form_answers_to_the_pin() {
    // The reviewer's counter-examples, one per form. Identity: the
    // text input crosses an int output unchanged.
    conformance_refuses("score = envelope");
    // Replace: a map construction into a scalar pin.
    conformance_refuses("score = {v = 1}");
    // Patch, field write: builds structure, no scalar pin holds it.
    conformance_refuses("score = envelope {data.x = 1}");
    // Patch, whole-data write: answers with the written type.
    conformance_refuses("score = envelope {data = \"nope\"}");
    // Data: the 32-bit bound of `int`, on a constant — the exact
    // var-initializer rule.
    conformance_refuses("score = 2147483648");
}

#[test]
fn conforming_forms_pass() {
    // Data with the right type and range.
    compile(
        &[gauge_graph("score = size(data)").as_str()],
        &Registry::m0(),
    )
    .expect("int through the int pin");
    // Patch, whole-data write of the pinned type.
    compile(
        &[gauge_graph("score = envelope {data = 7}").as_str()],
        &Registry::m0(),
    )
    .expect("a whole-data write of the pinned type");
    // Nothing produces nothing.
    compile(&[gauge_graph("score = nothing").as_str()], &Registry::m0()).expect("nothing conforms");
    // Identity through matching pins.
    let echoed = "graph g : App {\n  operator o {\n    input evt : text\n    output echo : text\n\n    on evt {\n      echo = envelope\n    }\n  }\n\n  . -> o.evt\n  o.echo -> .\n}\n";
    compile(&[echoed], &Registry::m0()).expect("text through the text pin, unchanged");
}

#[test]
fn a_typed_injection_matrix_holds_at_the_boundary() {
    let inject = |value: &str| {
        format!(
            "graph g : App {{\n  input alpha : int\n\n  operator o {{\n    on a {{ out = data }}\n  }}\n  .alpha -> o.a\n  o -> .\n}}\n\nlab g {{\n  simulation probing on app {{\n    scenario s \"x\" {{\n      app.alpha = {value}\n      expect \"y\" app.out count == 1\n    }}\n  }}\n}}\n"
        )
    };
    // The reviewer's map through the int input.
    let bag = compile(&[inject("{v = 1}").as_str()], &Registry::m0()).expect_err("a map breaches");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-PORT-TYPE"),
        "{bag:?}"
    );
    // The bound, at ingress.
    let bag =
        compile(&[inject("2147483648").as_str()], &Registry::m0()).expect_err("the bound holds");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-PORT-TYPE"),
        "{bag:?}"
    );
    // A conforming constant passes.
    compile(&[inject("7").as_str()], &Registry::m0()).expect("an int through the int input");
}

// ===== the resolved contract table travels in the plan =====

#[test]
fn a_contract_port_table_travels_without_the_registry() {
    use rastok::plan;
    use rastok::stdlib::{AttrDomain, AttributeSpec, ConnectorContract};

    // A future-shaped contract: named ports beside the trio — what
    // File and the HTTP pair will declare. The m1-slice transport
    // rule: everything the embedded validator checks must travel,
    // because it has no Registry.
    const PROBE_SHEET: ConnectorContract = ConnectorContract {
        name: "Probe",
        input_ports: &["in", "request"],
        output_ports: &["out", "err", "response"],
        attributes: &[
            AttributeSpec {
                name: "effect",
                required: true,
                domain: AttrDomain::Word(&["idempotent"]),
            },
            AttributeSpec {
                name: "behavior",
                required: true,
                domain: AttrDomain::Word(&["deterministic"]),
            },
        ],
        error_set: &["ProbeFailed"],
    };
    let registry = Registry::m0()
        .try_with(PROBE_SHEET)
        .expect("a valid sheet enters");
    // The table TRAVELS today; EXECUTING a named connector port is
    // deferred by name until the Partner seam speaks it (the edge
    // test below) — so the probe is wired through its trio.
    let source = "graph g : App {\n  connector probe : Probe {\n    effect = idempotent ; behavior = deterministic\n  }\n\n  operator relay {\n    on in { out = data }\n  }\n\n  . -> relay -> probe -> .\n}\n";
    let compilation = compile(&[source], &registry).expect("the contract compiles");
    // 2. The RESOLVED table is in the plan — the trio stays implicit.
    let probe = compilation
        .plan
        .plan()
        .nodes
        .iter()
        .find(|node| node.path.to_string() == "probe")
        .expect("the connector");
    assert!(
        probe.ports.iter().any(|decl| decl.port.name() == "request"
            && decl.direction == rastok::node::Direction::Input),
        "{:?}",
        probe.ports
    );
    assert!(
        probe.ports.iter().any(|decl| decl.port.name() == "response"
            && decl.direction == rastok::node::Direction::Output),
        "{:?}",
        probe.ports
    );
    assert!(
        !probe
            .ports
            .iter()
            .any(|decl| matches!(decl.port.name(), "in" | "out" | "err")),
        "the trio never enters a table: {:?}",
        probe.ports
    );
    // 3. The package round-trips the table.
    let bytes = plan::encode(&compilation.plan, &compilation.hash);
    let (decoded, _) = plan::decode(&bytes).expect("the package opens");
    assert_eq!(decoded, compilation.plan);
    // 4. The validator stands WITHOUT the Registry: the raw plan
    //    re-earns the type from its own tables alone.
    let raw = decoded.into_inner();
    rastok::plan::ValidatedExecutionPlan::try_new(raw).expect("no Registry consulted");
}

#[test]
fn a_named_connector_edge_defers_by_name() {
    use rastok::stdlib::{AttrDomain, AttributeSpec, ConnectorContract};
    const PROBE_SHEET: ConnectorContract = ConnectorContract {
        name: "Probe",
        input_ports: &["in", "request"],
        output_ports: &["out", "err", "response"],
        attributes: &[
            AttributeSpec {
                name: "effect",
                required: true,
                domain: AttrDomain::Word(&["idempotent"]),
            },
            AttributeSpec {
                name: "behavior",
                required: true,
                domain: AttrDomain::Word(&["deterministic"]),
            },
        ],
        error_set: &["ProbeFailed"],
    };
    let registry = Registry::m0()
        .try_with(PROBE_SHEET)
        .expect("a valid sheet enters");
    // The Partner seam does not speak named connector ports yet:
    // wiring one refuses BY NAME instead of compiling into an
    // execution that silently answers on `out`.
    let source = "graph g : App {\n  connector probe : Probe {\n    effect = idempotent ; behavior = deterministic\n  }\n\n  operator relay {\n    on in { out = data }\n  }\n\n  . -> relay\n  relay.out -> probe.request\n  probe.response -> .\n}\n";
    let bag = compile(&[source], &registry).expect_err("not yet executable");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-CHECK-DEFERRED"),
        "{bag:?}"
    );
}

// ===== the ordered fold and the shorthand's spec semantics =====

#[test]
fn the_fold_follows_the_runtime_order() {
    // `data = 1` then `data.x = 2`: the runtime ends on a STRUCTURE —
    // the checker must too, whatever wrote data earlier.
    conformance_refuses("score = envelope {\n        data = 1\n        data.x = 2\n      }");
    // The reverse order ends on the written int and conforms.
    compile(
        &[
            gauge_graph("score = envelope {\n        data.x = 2\n        data = 1\n      }")
                .as_str(),
        ],
        &Registry::m0(),
    )
    .expect("the final whole-data write decides");
}

#[test]
fn a_metadata_only_injection_still_carries_the_draft_struct() {
    // The draft is BORN a struct (`lab::draft`); patching only meta
    // does not change what crosses the pinned scalar input.
    let source = "graph g : App {\n  input alpha : int\n\n  operator o {\n    on a { out = data }\n  }\n  .alpha -> o.a\n  o -> .\n}\n\nlab g {\n  simulation probing on app {\n    scenario s \"x\" {\n      app.alpha = envelope {meta.tag = \"x\"}\n      expect \"y\" app.out count == 1\n    }\n  }\n}\n";
    let bag = compile(&[source], &Registry::m0()).expect_err("the draft map breaches the int pin");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-PORT-TYPE"),
        "{bag:?}"
    );
}

#[test]
fn the_shorthand_is_physically_a_struct() {
    // Not cosmetic: Struct and Map are two domain values (canonical
    // tags 17 and 18). The builders produce STRUCTS — asserted on
    // the envelope itself for all three interpreters' shared path,
    // and on the canonical seal below.
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, Namespace, Scalar, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let source = "graph g : App {\n  operator o {\n    on in {\n      out = {\n        data = 7\n      }\n    }\n  }\n  . -> o -> .\n}\n";
    let compilation = compile(&[source], &Registry::m0()).expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(
        &mut journal,
        InboundSubmission {
            authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Scalar(Scalar::Int(1))),
                mime_type: None,
                app: Namespace::new(),
            },
            sequence: SubmissionSequence::Next,
        },
    )
    .expect("the injection lands");
    vm.drain(&mut journal).expect("the drain completes");
    let output = &vm.outputs()[0];
    let Data::Materialized(Value::Struct(fields)) = output.envelope.data() else {
        panic!("a STRUCT, not a map: {:?}", output.envelope.data());
    };
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].0, "data");
    assert_eq!(fields[0].1, Value::Scalar(Scalar::Int(7)));
    // And the canonical seal carries tag 17 (struct), not 18 (map):
    // find the emission record and walk envelope(20).data(8).
    let trace = rastok::golden::project(journal.records());
    let mut sealed_struct = false;
    for index in 0..trace.len() {
        let (tree, _) = rastok::canonical_cbor::decode(trace.record_bytes(index).unwrap())
            .expect("canonical bytes");
        let rastok::canonical_cbor::Value::Map(pairs) = tree else {
            continue;
        };
        let Some(rastok::canonical_cbor::Value::Array(emissions)) = pairs.get(&36) else {
            continue;
        };
        for emission in emissions {
            let rastok::canonical_cbor::Value::Array(parts) = emission else {
                continue;
            };
            let Some(rastok::canonical_cbor::Value::Map(spine)) = parts.last() else {
                continue;
            };
            // Materialized data seals as [1, bytes(tagged_value)] —
            // the tag lives inside the embedded bytes.
            if let Some(rastok::canonical_cbor::Value::Array(form)) = spine.get(&8)
                && let Some(rastok::canonical_cbor::Value::Bytes(inner)) = form.get(1)
                && let Ok((rastok::canonical_cbor::Value::Array(tagged), _)) =
                    rastok::canonical_cbor::decode(inner)
                && tagged.first() == Some(&rastok::canonical_cbor::Value::Uint(17))
            {
                sealed_struct = true;
            }
        }
    }
    assert!(sealed_struct, "the emission seals its data as tag 17");
}

#[test]
fn the_shorthand_builds_a_struct_with_ordinary_names() {
    // `{ … } ≡ envelope { data = struct { … } }` — `data`, `meta`,
    // and `mime_type` are FIELDS of the new structure, never
    // envelope-level writes. Mocks share this resolver with handler
    // emissions and injections.
    let source = r#"
graph g : App {
  operator o {
    on in {
      out = {
        data = 7
        meta.flag = true
      }
    }
  }
  . -> o -> .
}

lab g {
  simulation probing on app {
    scenario s "reserved names are plain fields" {
      app.in = {v = 1}
      expect "field named data" app.out count == 1 where data.data == 7
      expect "field named meta" app.out count == 1 where data.meta.flag == true
    }
  }
}
"#;
    lab_passes(source);
}

#[test]
fn a_computed_overflow_faults_at_the_pin() {
    // The reviewer's runtime escape: `data + 2147483647` with data=1
    // types as int and passes every static gate, but the VALUE
    // leaves the 32-bit contract. The emission's construction now
    // judges it like a state-cell write: the transaction faults, a
    // typed RuntimeFault reaches err, and NOTHING crosses the pinned
    // output.
    let source = r#"
graph g : App {
  input feed : int
  output result : int

  operator o {
    input evt : int
    output score : int

    on evt {
      score = data + 2147483647
    }
  }

  .feed -> o.evt
  o.score -> .result
}

lab g {
  simulation probing on app {
    scenario s "computed overflow" {
      app.feed = 1
      expect "no long through an int pin" app.result count == 0
      expect "the fault is typed" app.err count == 1
    }
  }
}
"#;
    lab_passes(source);
}

#[test]
fn a_defective_sheet_is_held_at_the_door() {
    use rastok::stdlib::{ConnectorContract, SheetDefect};
    let sheet = |input_ports: &'static [&'static str], output_ports: &'static [&'static str]| {
        ConnectorContract {
            name: "Bad",
            input_ports,
            output_ports,
            attributes: &[],
            error_set: &[],
        }
    };
    // A default port against its implicit direction — the exact
    // defect that would have split the checker from the validator
    // through the old Registry fallback.
    assert_eq!(
        Registry::m0().try_with(sheet(&["out"], &[])).unwrap_err(),
        SheetDefect::MisdirectedDefault("out")
    );
    // One name in both directions.
    assert_eq!(
        Registry::m0()
            .try_with(sheet(&["request"], &["request"]))
            .unwrap_err(),
        SheetDefect::ConflictingPort("request")
    );
    // The shipped M0 sheet passes its own door.
    Registry::m0()
        .try_with(rastok::stdlib::PROCESS)
        .expect("the Process sheet is valid");
}

#[test]
fn boundary_outputs_carry_their_port() {
    // Two named outputs are two different FACTS: the business API
    // answers (port, envelope) pairs, not an anonymous stream the
    // caller would have to re-derive from the plan.
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, Namespace, Scalar, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let graph = &SORTER[..SORTER.find("lab ").expect("the lab block")];
    let compilation = compile(&[graph], &Registry::m0()).expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(
        &mut journal,
        InboundSubmission {
            authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Scalar(Scalar::Int(1))),
                mime_type: None,
                app: Namespace::new(),
            },
            sequence: SubmissionSequence::Next,
        },
    )
    .expect("the injection lands");
    vm.drain(&mut journal).expect("the drain completes");
    let ports: Vec<&str> = vm
        .outputs()
        .iter()
        .map(|output| output.port.name())
        .collect();
    assert_eq!(
        ports,
        ["high", "low"],
        "each envelope names the boundary port it left through"
    );
    assert!(
        vm.outputs()
            .iter()
            .all(|output| output.envelope.data() == vm.outputs()[0].envelope.data()),
        "one fan-out, two named exits"
    );
}

// ===== the dynamic barrier — every path to a pin is judged =====

#[test]
fn a_raw_ingress_holds_the_pin_before_any_append() {
    // The reviewer's escape: the raw API accepted a wrong-typed
    // draft and JOURNALED it. The pins are judged before any append
    // — replay never meets a record a pin refuses.
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, Namespace, Scalar, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let source = "graph g : App {\n  input alpha : int\n\n  operator o {\n    on a { out = data }\n  }\n  .alpha -> o.a\n  o -> .\n}\n";
    let compilation = compile(&[source], &Registry::m0()).expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let submission = InboundSubmission {
        authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
        port: rastok::node::Port::new("alpha").unwrap(),
        draft: Draft {
            data: Data::materialized(Value::Scalar(Scalar::Text(String::from("wrong")))),
            mime_type: None,
            app: Namespace::new(),
        },
        sequence: SubmissionSequence::Next,
    };
    vm.inject(&mut journal, submission)
        .expect_err("a text draft cannot enter an int input");
    assert_eq!(journal.records().len(), 0, "refused BEFORE the append");
}

#[test]
fn an_unchanged_pass_through_holds_the_pin_at_run() {
    // Statically dyn (the activation is unpinned), dynamically text:
    // the pass-through is judged at emission construction — a typed
    // RuntimeFault leaves on err, nothing crosses `score : int`.
    let source = r#"
graph g : App {
  output result : int

  operator o {
    output score : int

    on in {
      score = envelope
    }
  }

  . -> o
  o.score -> .result
}

lab g {
  simulation probing on app {
    scenario s "a dynamic wrong shape faults" {
      app.in = "wrong"
      expect "nothing crosses" app.result count == 0
      expect "the fault is typed" app.err count == 1
    }
  }
}
"#;
    lab_passes(source);
}

#[test]
fn a_typed_destination_holds_against_an_untyped_source() {
    // The static rule admits dyn → pinned; the DESTINATION pin is
    // judged at emission construction, before the commit.
    let source = r#"
graph g : App {
  output result : int

  operator o {
    on in {
      out = data
    }
  }

  . -> o
  o.out -> .result
}

lab g {
  simulation probing on app {
    scenario s "a text cannot land on an int output" {
      app.in = "wrong"
      expect "nothing crosses" app.result count == 0
      expect "the fault is typed" app.err count == 1
    }
  }
}
"#;
    lab_passes(source);
}

#[test]
fn a_connector_reply_holds_the_destination_pin() {
    // A materialized partner answer fans out like any content: the
    // typed input downstream judges it before the outcome commits.
    let source = r#"
graph g : App {
  connector shell : Process {
    mode = job ; command = "cat"
    codec = lines
    effect = idempotent ; behavior = deterministic
  }

  operator gauge {
    input evt : int

    on evt {
      out = data
    }
  }

  . -> shell
  shell.out -> gauge.evt
  gauge -> .
}

lab g {
  simulation probing on app {
    mock shell {
      on in { out = {v = 1} }
    }

    scenario s "a structured reply cannot land on an int input" {
      app.in = "go"
      expect "nothing crosses" app.out count == 0
      expect "the fault is typed" app.err count == 1
    }
  }
}
"#;
    lab_passes(source);
}

// ===== the validator holds pinned edges =====

#[test]
fn the_validator_refuses_mismatched_edge_pins() {
    use rastok::plan::{PlanValidationError, ValidatedExecutionPlan};
    let compiled = || {
        compile(&[TYPED], &Registry::m0())
            .expect("a valid plan")
            .plan
            .into_inner()
    };
    // Node target: retune `gauge.evt` to int under a text source.
    let mut plan = compiled();
    let gauge = plan
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "gauge")
        .expect("the operator");
    let evt = gauge
        .ports
        .iter_mut()
        .find(|decl| decl.port.name() == "evt")
        .expect("the pinned input");
    evt.ty = Some(rastok::cel::Type::Int);
    assert!(matches!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::MismatchedEdgePins { port, expected, found }
            if port.name() == "evt"
                && expected == rastok::cel::Type::Int
                && found == rastok::cel::Type::Text
    ));
    // Boundary target: the gauge fixture wires `meter.score : int`
    // into `.measure : int` — retune the boundary side to text.
    let gauge = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/gauge.rkg"),
    )
    .expect("the example reads");
    let mut plan = compile(&[gauge.as_str()], &Registry::m0())
        .expect("a valid plan")
        .plan
        .into_inner();
    let measure = plan
        .boundary
        .iter_mut()
        .find(|decl| decl.port.name() == "measure")
        .expect("the pinned output");
    measure.ty = Some(rastok::cel::Type::Text);
    assert!(matches!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::MismatchedEdgePins { port, .. } if port.name() == "measure"
    ));
}

#[test]
fn a_named_constant_answers_like_its_literal() {
    // `val huge = 2147483648` then `score = huge` must refuse exactly
    // like the literal — the visible constants feed the bound check.
    let source = "graph g : App {\n  operator o {\n    val huge = 2147483648\n    output score : int\n    on in { score = huge }\n  }\n  . -> o\n  o.score -> .\n}\n";
    let bag = compile(&[source], &Registry::m0()).expect_err("the alias breaches like the literal");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-PORT-TYPE"),
        "{bag:?}"
    );
}

// ===== round five — Dyn coherence, synthesized errors, the relay =====

#[test]
fn a_variant_pin_is_inference_territory_everywhere() {
    // `variant` resolves to Dyn — an explicit "anything". A pinned
    // Dyn admits every emission, and only two PRECISE pins can make
    // an edge disagree: checker, validator, and runtime share the
    // rule.
    let flexible = "graph g : App {\n  operator o {\n    output flexible : variant\n    on in { flexible = 1 }\n  }\n  . -> o\n  o.flexible -> .\n}\n";
    compile(&[flexible], &Registry::m0()).expect("an int through a variant pin");

    let variant_to_int = "graph g : App {\n  operator a {\n    output flexible : variant\n    on in { flexible = 1 }\n  }\n  operator b {\n    input evt : int\n    on evt { out = data }\n  }\n  . -> a\n  a.flexible -> b.evt\n  b -> .\n}\n";
    compile(&[variant_to_int], &Registry::m0()).expect("variant -> int is inference territory");

    let int_to_variant = "graph g : App {\n  operator a {\n    output score : int\n    on in { score = 1 }\n  }\n  operator b {\n    input evt : variant\n    on evt { out = data }\n  }\n  . -> a\n  a.score -> b.evt\n  b -> .\n}\n";
    let compilation =
        compile(&[int_to_variant], &Registry::m0()).expect("int -> variant is inference territory");

    // The same rule holds for a raw plan re-earning the type after
    // the trip — a Dyn/Int pinned edge is never a mismatch.
    let bytes = rastok::plan::encode(&compilation.plan, &compilation.hash);
    let (decoded, _) = rastok::plan::decode(&bytes).expect("the package opens");
    rastok::plan::ValidatedExecutionPlan::try_new(decoded.into_inner())
        .expect("the validator applies the same Dyn rule");
}

#[test]
fn a_synthesized_runtime_fault_cannot_cross_a_pin() {
    // `source.err -> sink.evt : int`: the operator faults, the
    // machine synthesizes a RuntimeFault struct on err — and the ONE
    // choke point refuses it before ANY commit: no transaction in
    // the journal, the scenario reports the machine fault.
    let source = r#"
graph g : App {
  operator source {
    on in { out = data.missing }
  }

  operator sink {
    input evt : int
    on evt { out = data }
  }

  . -> source
  source.err -> sink.evt
  source -> .
  sink -> .
}

lab g {
  simulation probing on app {
    scenario s "a fault payload cannot enter an int pin" {
      app.in = "text"
      expect "nothing crosses" app.out count == 0
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
    let run = &runs[0];
    assert!(
        run.report
            .failures
            .iter()
            .any(|failure| failure.contains("cannot enter")),
        "the machine fault surfaces: {:#?}",
        run.report.failures
    );
    assert!(
        run.records
            .iter()
            .all(|record| !matches!(record, rastok::journal::Record::Transaction(_))),
        "NOTHING committed — the violating emission never reached the journal"
    );
}

#[test]
fn a_decode_error_cannot_cross_a_pin() {
    // The machine's own DecodeError synthesis takes the same choke
    // point: a map into a `lines` connector refuses framing, the
    // typed error struct heads for err — and err is wired into an
    // int pin, so nothing commits past the operator.
    let source = r#"
graph g : App {
  connector shell : Process {
    mode = job ; command = "cat"
    codec = lines
    effect = idempotent ; behavior = deterministic
  }

  operator sink {
    input evt : int
    on evt { out = data }
  }

  . -> shell
  shell.err -> sink.evt
  shell.out -> .
  sink -> .
}

lab g {
  simulation probing on app {
    mock shell {
      on in { out = envelope }
    }

    scenario s "a decode-error payload cannot enter an int pin" {
      app.in = {v = 1}
      expect "nothing crosses" app.out count == 0
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
    let run = &runs[0];
    assert!(
        run.report
            .failures
            .iter()
            .any(|failure| failure.contains("cannot enter")),
        "the machine fault surfaces: {:#?}",
        run.report.failures
    );
    // TimeoutExpired and every Partner failure route through the
    // SAME reserve_emissions choke point — one line guards them all.
    assert!(
        run.records
            .iter()
            .all(|record| !matches!(record, rastok::journal::Record::Transaction(_))),
        "nothing committed"
    );
}

#[test]
fn the_bare_data_relay_preserves_the_logical_type() {
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, Namespace, Scalar, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;
    use std::collections::BTreeMap;

    let source =
        "graph g : App {\n  operator o {\n    on in { out = data }\n  }\n  . -> o -> .\n}\n";
    let compilation = compile(&[source], &Registry::m0()).expect("a valid plan");
    let relay = |data: Value| {
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        vm.inject(
            &mut journal,
            InboundSubmission {
                authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
                port: rastok::node::Port::new("in").unwrap(),
                draft: Draft {
                    data: Data::materialized(data),
                    mime_type: None,
                    app: Namespace::new(),
                },
                sequence: SubmissionSequence::Next,
            },
        )
        .expect("the injection lands");
        vm.drain(&mut journal).expect("the drain completes");
        vm.outputs()[0].envelope.data().clone()
    };
    // Struct in, Struct out — tag 17 stays tag 17.
    let sent = Value::Struct(vec![(String::from("field"), Value::Scalar(Scalar::Int(7)))]);
    assert!(
        matches!(relay(sent), Data::Materialized(Value::Struct(_))),
        "a bare relay must not retype a struct"
    );
    // Map in, Map out — tag 18 stays tag 18.
    let mut entries = BTreeMap::new();
    entries.insert(String::from("k"), Value::Scalar(Scalar::Int(1)));
    assert!(
        matches!(
            relay(Value::Map(entries)),
            Data::Materialized(Value::Map(_))
        ),
        "a bare relay must not retype a map"
    );
}

#[test]
fn an_ingress_pin_violation_is_a_named_error() {
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, Namespace, Scalar, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::{Vm, VmError};

    let source = "graph g : App {\n  input alpha : int\n\n  operator o {\n    on a { out = data }\n  }\n  .alpha -> o.a\n  o -> .\n}\n";
    let compilation = compile(&[source], &Registry::m0()).expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let error = vm
        .inject(
            &mut journal,
            InboundSubmission {
                authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
                port: rastok::node::Port::new("alpha").unwrap(),
                draft: Draft {
                    data: Data::materialized(Value::Scalar(Scalar::Text(String::from("wrong")))),
                    mime_type: None,
                    app: Namespace::new(),
                },
                sequence: SubmissionSequence::Next,
            },
        )
        .unwrap_err();
    // The CLIENT's value is wrong, not the plan — the error says so
    // by name and carries the port.
    assert!(matches!(
        error,
        VmError::IngressPinViolation { port, .. } if port.name() == "alpha"
    ));
}

#[test]
fn the_lab_and_mock_builders_seal_structs_too() {
    // The VM path is asserted above; the Lab draft and the mock
    // reply have their own implementations of the same builder —
    // each is held to tag 17 in the sealed records it produces.
    let source = r#"
graph g : App {
  connector shell : Process {
    mode = job ; command = "cat"
    codec = lines
    effect = idempotent ; behavior = deterministic
  }

  operator relay {
    on in { out = "ping" }
  }

  . -> relay -> shell -> .
}

lab g {
  simulation probing on app {
    mock shell {
      on in { out = {v = 1} }
    }

    scenario s "sugar everywhere" {
      app.in = {field = 7}
      expect "the reply lands" app.out count == 1
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
    let run = &runs[0];
    assert!(run.report.passed(), "{:#?}", run.report.failures);
    let trace = rastok::golden::project(&run.records);
    let tagged_struct = |bytes: &[u8]| {
        let Ok((rastok::canonical_cbor::Value::Array(tagged), _)) =
            rastok::canonical_cbor::decode(bytes)
        else {
            return false;
        };
        tagged.first() == Some(&rastok::canonical_cbor::Value::Uint(17))
    };
    let mut inbound_struct = false;
    let mut outcome_struct = false;
    for index in 0..trace.len() {
        let (tree, _) = rastok::canonical_cbor::decode(trace.record_bytes(index).unwrap())
            .expect("canonical bytes");
        let rastok::canonical_cbor::Value::Map(pairs) = tree else {
            continue;
        };
        // The INBOUND record: the Lab draft's data seals as a struct.
        if let Some(rastok::canonical_cbor::Value::Map(spine)) = pairs.get(&20)
            && let Some(rastok::canonical_cbor::Value::Array(form)) = spine.get(&8)
            && let Some(rastok::canonical_cbor::Value::Bytes(inner)) = form.get(1)
            && tagged_struct(inner)
        {
            inbound_struct = true;
        }
        // The connector outcome: the MOCK reply's data seals as a
        // struct on the way to `out`.
        if let Some(rastok::canonical_cbor::Value::Array(emissions)) = pairs.get(&36) {
            for emission in emissions {
                let rastok::canonical_cbor::Value::Array(parts) = emission else {
                    continue;
                };
                if let Some(rastok::canonical_cbor::Value::Map(spine)) = parts.last()
                    && let Some(rastok::canonical_cbor::Value::Array(form)) = spine.get(&8)
                    && let Some(rastok::canonical_cbor::Value::Bytes(inner)) = form.get(1)
                    && tagged_struct(inner)
                {
                    outcome_struct = true;
                }
            }
        }
    }
    assert!(inbound_struct, "the Lab draft seals tag 17");
    assert!(outcome_struct, "the mock reply seals tag 17");
}

#[test]
fn the_validator_exploits_the_traveled_contract_table() {
    // Transport alone is not the proof — a raw edge into the
    // contract's named port must be ACCEPTED by the validator from
    // the plan's own table, no Registry anywhere.
    use rastok::plan::{Edge, EndpointRef, Slot, ValidatedExecutionPlan};
    use rastok::stdlib::{AttrDomain, AttributeSpec, ConnectorContract};
    const PROBE_SHEET: ConnectorContract = ConnectorContract {
        name: "Probe",
        input_ports: &["in", "request"],
        output_ports: &["out", "err", "response"],
        attributes: &[
            AttributeSpec {
                name: "effect",
                required: true,
                domain: AttrDomain::Word(&["idempotent"]),
            },
            AttributeSpec {
                name: "behavior",
                required: true,
                domain: AttrDomain::Word(&["deterministic"]),
            },
        ],
        error_set: &["ProbeFailed"],
    };
    let registry = Registry::m0()
        .try_with(PROBE_SHEET)
        .expect("a valid sheet enters");
    let source = "graph g : App {\n  connector probe : Probe {\n    effect = idempotent ; behavior = deterministic\n  }\n\n  operator relay {\n    on in { out = data }\n  }\n\n  . -> relay -> probe -> .\n}\n";
    let mut plan = compile(&[source], &registry)
        .expect("the contract compiles")
        .plan
        .into_inner();
    let probe_slot = plan
        .nodes
        .iter()
        .position(|node| node.path.to_string() == "probe")
        .expect("the connector");
    let relay_slot = plan
        .nodes
        .iter()
        .position(|node| node.path.to_string() == "relay")
        .expect("the operator");
    plan.edges.push(Edge {
        from: EndpointRef {
            node: Some(Slot::new(u32::try_from(relay_slot).unwrap())),
            port: rastok::node::Port::new("out").unwrap(),
        },
        to: Some(EndpointRef {
            node: Some(Slot::new(u32::try_from(probe_slot).unwrap())),
            port: rastok::node::Port::new("request").unwrap(),
        }),
    });
    let mut edges = plan.edges.clone();
    edges.sort_by_key(|edge| plan.edge_text(edge));
    plan.edges = edges;
    ValidatedExecutionPlan::try_new(plan)
        .expect("the traveled table answers for `request` — no Registry consulted");
}
