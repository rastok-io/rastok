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
    let registry = Registry::m0().with(PROBE_SHEET);
    let source = "graph g : App {\n  connector probe : Probe {\n    effect = idempotent ; behavior = deterministic\n  }\n\n  operator relay {\n    on in { out = data }\n  }\n\n  . -> relay\n  relay.out -> probe.request\n  probe.response -> .\n}\n";
    // 1. The checker accepts the wiring to the contract's named ports.
    let compilation = compile(&[source], &registry).expect("the contract wiring compiles");
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
