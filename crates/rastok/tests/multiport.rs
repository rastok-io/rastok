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
use rastok::contract::Registry;

/// One input fanning to two named outputs through named node ports.
const SORTER: &str = r#"
graph sorter : App {
  input in: variant
  output high: variant
  output low: variant
  operator route {
    on in {
      hot = data
      cold = data
    }
  }

  . -> route
  route.hot -> .high
  route.cold -> .low
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
  input alpha: variant
  input beta: variant
  output out: variant

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

/// The world's half of an Avro ingress: the transport's content
/// type rides `meta.ext` (D-2026-08-10-4).
fn avro_ext() -> rastok::envelope::Namespace {
    let mut ext = rastok::envelope::Namespace::new();
    ext.insert(
        String::from("content_type"),
        rastok::envelope::MetaValue::Scalar(rastok::envelope::Scalar::Text(String::from(
            "application/avro",
        ))),
    );
    ext
}

fn lab_passes(source: &str) {
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
    let bag = compile(
        &[source.as_str()],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("the scenario refuses");
    bag.diagnostics()
        .iter()
        .map(|diagnostic| diagnostic.code().as_str())
        .collect()
}

const MERGER_GRAPH: &str = r#"
graph merge : App {
  input alpha: variant
  output out: variant

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
    // Conventional `out`, and a graph output generally, is not an
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
    use rastok::envelope::{Data, Namespace, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::{Vm, VmError};

    let compilation = compile(
        &[MERGER_GRAPH],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let submission = |port: &str| InboundSubmission {
        authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
        port: rastok::node::Port::new(port).unwrap(),
        draft: Draft {
            data: Data::materialized(Value::Text(String::from("go"))),
            ext: Namespace::new(),
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
            ext: Namespace::new(),
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
    let bag = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("text times int refuses");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-EXPR"),
        "{bag:?}"
    );
}

#[test]
fn an_emission_against_a_pinned_output_refuses() {
    let source = "graph g : App {\n  operator o {\n    output score : int\n    on in { score = \"nope\" }\n  }\n  . -> o\n  o.score -> .\n}\n";
    let bag = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("a text emission into an int port");
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
    let bag = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("int wired into text");
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
    let bag = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("a text injection into an int input");
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
    let compilation = compile(
        &[TYPED],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
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
            .any(|decl| decl.port.name() == "evt"
                && decl.ty == Some(rastok::datatype::DataType::Text)),
        "the pinned input travels: {:?}",
        gauge.ports
    );
    assert!(
        compilation
            .plan
            .plan()
            .boundary
            .iter()
            .any(|decl| decl.port.name() == "feed"
                && decl.ty == Some(rastok::datatype::DataType::Text)),
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
    let annotated = "graph g : App {\n  input in: text\n  output out: text\n  operator o {\n    input evt : text\n    on evt { out = data }\n  }\n  . -> o.evt\n  o -> .\n}\n";
    let bare = annotated.replace(" : text", "");
    let one = compile(
        &[annotated],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let two = compile(
        &[bare.as_str()],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    assert_ne!(one.hash, two.hash, "the annotation is identity material");
    assert_ne!(one.plan, two.plan);
}

// ===== observation direction — no false green at the boundary =====

fn expect_refusal(expectation: &str) -> Vec<&'static str> {
    let source = format!(
        "{MERGER_GRAPH}\nlab merge {{\n  simulation probing on app {{\n    scenario s \"x\" {{\n      \
         app.alpha = {{v = 1}}\n      {expectation}\n    }}\n  }}\n}}\n"
    );
    let bag = compile(
        &[source.as_str()],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("the expectation refuses");
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
    let bag = compile(
        &[source.as_str()],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("the pin holds");
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
    conformance_refuses("score = .");
    // Record: a struct construction into a scalar pin.
    conformance_refuses("score = {v = 1}");
    // Patch, field write: builds structure, no scalar pin holds it.
    conformance_refuses("score = {data.x = 1}");
    // Patch, whole-data write: answers with the written type.
    conformance_refuses("score = {data = \"nope\"}");
    // Data: the 32-bit bound of `int`, on a constant — the exact
    // var-initializer rule.
    conformance_refuses("score = 2147483648");
}

#[test]
fn conforming_forms_pass() {
    // Data with the right type and range.
    compile(
        &[gauge_graph("score = size(data)").as_str()],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("int through the int pin");
    // Patch, whole-data write of the pinned type.
    compile(
        &[gauge_graph("score = {data = 7}").as_str()],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a whole-data write of the pinned type");
    // Nothing produces nothing.
    compile(
        &[gauge_graph("score = nothing").as_str()],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("nothing conforms");
    // Identity through matching pins.
    let echoed = "graph g : App {\n  operator o {\n    input evt : text\n    output echo : text\n\n    on evt {\n      echo = .\n    }\n  }\n\n  . -> o.evt\n  o.echo -> .\n}\n";
    compile(
        &[echoed],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("text through the text pin, unchanged");
}

#[test]
fn a_typed_injection_matrix_holds_at_the_boundary() {
    let inject = |value: &str| {
        format!(
            "graph g : App {{\n  input alpha : int\n\n  operator o {{\n    on a {{ out = data }}\n  }}\n  .alpha -> o.a\n  o -> .\n}}\n\nlab g {{\n  simulation probing on app {{\n    scenario s \"x\" {{\n      app.alpha = {value}\n      expect \"y\" app.out count == 1\n    }}\n  }}\n}}\n"
        )
    };
    // The reviewer's map through the int input.
    let bag = compile(
        &[inject("{v = 1}").as_str()],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("a map breaches");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-PORT-TYPE"),
        "{bag:?}"
    );
    // The bound, at ingress.
    let bag = compile(
        &[inject("2147483648").as_str()],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("the bound holds");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-PORT-TYPE"),
        "{bag:?}"
    );
    // A conforming constant passes.
    compile(
        &[inject("7").as_str()],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("an int through the int input");
}

// ===== the resolved contract table travels in the plan =====

#[test]
fn a_resolved_contract_port_table_enters_the_validated_plan_complete() {
    use rastok::contract::{
        AttributeSpec, ConnectorContract, PortContract, PortDataType, PortTypeRule, Presence,
        ValueDomain,
    };
    use rastok::node::Direction;
    use rastok::plan::UnwiredPolicy;

    // B5 closes the old hole: a contract name and direction are not enough
    // to make a usable port. The resolved sheet ships every row with its
    // effective type; no named port becomes an implicit variant.
    const PROBE_SHEET: ConnectorContract = ConnectorContract {
        name: "Probe",
        ports: &[
            PortContract {
                name: "in",
                direction: Direction::Input,
                unwired: UnwiredPolicy::RequireWire,
                ty: PortTypeRule::Fixed(PortDataType::Variant),
            },
            PortContract {
                name: "request",
                direction: Direction::Input,
                unwired: UnwiredPolicy::RequireWire,
                ty: PortTypeRule::Fixed(PortDataType::Variant),
            },
            PortContract {
                name: "out",
                direction: Direction::Output,
                unwired: UnwiredPolicy::RequireWire,
                ty: PortTypeRule::Fixed(PortDataType::Variant),
            },
            PortContract {
                name: "err",
                direction: Direction::Output,
                unwired: UnwiredPolicy::RootSink,
                ty: PortTypeRule::Fixed(PortDataType::Variant),
            },
            PortContract {
                name: "response",
                direction: Direction::Output,
                unwired: UnwiredPolicy::RequireWire,
                ty: PortTypeRule::Fixed(PortDataType::Variant),
            },
        ],
        common: &[
            AttributeSpec {
                name: "behavior",
                presence: Presence::Required,
                domain: ValueDomain::Choice(&["deterministic"]),
            },
            AttributeSpec {
                name: "effect",
                presence: Presence::Required,
                domain: ValueDomain::Choice(&["idempotent"]),
            },
        ],
        selector: None,
    };
    let registry = Registry::try_m0()
        .expect("the standard catalogue holds")
        .try_with(PROBE_SHEET)
        .expect("a valid sheet enters");
    let source = "graph g : App {\n  connector probe : Probe {\n    effect = idempotent ; behavior = deterministic\n  }\n\n  operator relay {\n    on in { out = data }\n  }\n\n  . -> relay -> probe -> .\n}\n";
    let compilation = compile(&[source], &registry).expect("the sheet resolves every port type");
    let probe = compilation
        .plan
        .nodes
        .iter()
        .find(|node| node.path.to_string() == "probe")
        .expect("the connector");
    assert_eq!(probe.ports.len(), 5, "the complete contract table travels");
    assert!(probe.ports.iter().all(|port| port.ty.is_some()));
}

#[test]
fn a_named_connector_edge_defers_by_name() {
    use rastok::contract::{
        AttributeSpec, ConnectorContract, PortContract, PortDataType, PortTypeRule, Presence,
        ValueDomain,
    };
    use rastok::node::Direction;
    use rastok::plan::UnwiredPolicy;
    const PROBE_SHEET: ConnectorContract = ConnectorContract {
        name: "Probe",
        ports: &[
            PortContract {
                name: "in",
                direction: Direction::Input,
                unwired: UnwiredPolicy::RequireWire,
                ty: PortTypeRule::Fixed(PortDataType::Variant),
            },
            PortContract {
                name: "request",
                direction: Direction::Input,
                unwired: UnwiredPolicy::RequireWire,
                ty: PortTypeRule::Fixed(PortDataType::Variant),
            },
            PortContract {
                name: "out",
                direction: Direction::Output,
                unwired: UnwiredPolicy::RequireWire,
                ty: PortTypeRule::Fixed(PortDataType::Variant),
            },
            PortContract {
                name: "err",
                direction: Direction::Output,
                unwired: UnwiredPolicy::RootSink,
                ty: PortTypeRule::Fixed(PortDataType::Variant),
            },
            PortContract {
                name: "response",
                direction: Direction::Output,
                unwired: UnwiredPolicy::RequireWire,
                ty: PortTypeRule::Fixed(PortDataType::Variant),
            },
        ],
        common: &[
            AttributeSpec {
                name: "behavior",
                presence: Presence::Required,
                domain: ValueDomain::Choice(&["deterministic"]),
            },
            AttributeSpec {
                name: "effect",
                presence: Presence::Required,
                domain: ValueDomain::Choice(&["idempotent"]),
            },
        ],
        selector: None,
    };
    let registry = Registry::try_m0()
        .expect("the standard catalogue holds")
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

// ===== the ordered fold and the braced form's spec semantics =====

#[test]
fn the_fold_follows_the_runtime_order() {
    // `data = 1` then `data.x = 2`: the runtime ends on a STRUCTURE —
    // the checker must too, whatever wrote data earlier.
    conformance_refuses("score = {\n        data = 1\n        data.x = 2\n      }");
    // The reverse order ends on the written int and conforms.
    compile(
        &[gauge_graph("score = {\n        data.x = 2\n        data = 1\n      }").as_str()],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the final whole-data write decides");
}

#[test]
fn a_metadata_only_injection_still_carries_the_draft_struct() {
    // The draft is BORN a struct (`lab::draft`); patching only meta
    // does not change what crosses the pinned scalar input.
    let source = "graph g : App {\n  input alpha : int\n\n  operator o {\n    on a { out = data }\n  }\n  .alpha -> o.a\n  o -> .\n}\n\nlab g {\n  simulation probing on app {\n    scenario s \"x\" {\n      app.alpha = {meta.tag = \"x\"}\n      expect \"y\" app.out count == 1\n    }\n  }\n}\n";
    let bag = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("the draft map breaches the int pin");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-PORT-TYPE"),
        "{bag:?}"
    );
}

#[test]
fn the_braced_record_is_physically_a_struct() {
    // Not cosmetic: Struct and Map are two domain values (canonical
    // tags 17 and 18). A braced record produces a STRUCT — asserted
    // on the envelope itself for all three interpreters' shared
    // path, and on the canonical seal below.
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, Namespace, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in {\n      out = {\n        v = 7\n      }\n    }\n  }\n  . -> o -> .\n}\n";
    let compilation = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(
        &mut journal,
        InboundSubmission {
            authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Int(1)),
                ext: Namespace::new(),
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
    assert_eq!(fields[0].0, "v");
    assert_eq!(fields[0].1, Value::Int(7));
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
fn the_escape_gives_reserved_names_as_plain_fields() {
    // Business fields that happen to be named `data` or `meta` need
    // the explicit escape: `out = { data = { … } }` is the envelope
    // form whose whole-data write is a record — inside that record,
    // reserved names are ordinary fields (D-2026-08-09-1). Mocks
    // share this resolver with handler emissions and injections.
    let source = r#"
type ReservedPayload = struct { data: int, meta: struct { flag: bool } }

graph g : App {
  input in: variant
  operator o {
    on in {
      out = {
        data = {
          data = 7
          meta.flag = true
        }
      }
    }
  }
  . -> o -> .
}

lab g {
  simulation probing on app {
    scenario s "reserved names are plain fields inside the escape" {
      app.in = {v = 1}
      expect "field named data" app.out count == 1 where data is ReservedPayload && data.data == 7
      expect "field named meta" app.out count == 1 where data is ReservedPayload && data.meta.flag == true
    }
  }
}
"#;
    lab_passes(source);
}

#[test]
fn an_empty_brace_is_an_empty_record() {
    // `{}` builds an EMPTY struct — never an identity (identity is
    // `.`). The injected payload must not leak through.
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, Namespace, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in { out = {} }\n  }\n  . -> o -> .\n}\n";
    let compilation = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(
        &mut journal,
        InboundSubmission {
            authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Int(1)),
                ext: Namespace::new(),
            },
            sequence: SubmissionSequence::Next,
        },
    )
    .expect("the injection lands");
    vm.drain(&mut journal).expect("the drain completes");
    assert_eq!(
        vm.outputs()[0].envelope.data(),
        &Data::materialized(Value::Struct(Vec::new())),
        "an empty record, not the input"
    );
}

#[test]
fn a_whole_data_write_is_the_payload_itself() {
    // `{data = 1}` is the ENVELOPE form: the payload becomes the
    // integer 1 — never a struct with a field named `data`.
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, Namespace, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in { out = {data = 1} }\n  }\n  . -> o -> .\n}\n";
    let compilation = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(
        &mut journal,
        InboundSubmission {
            authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Text(String::from("gone"))),
                ext: Namespace::new(),
            },
            sequence: SubmissionSequence::Next,
        },
    )
    .expect("the injection lands");
    vm.drain(&mut journal).expect("the drain completes");
    assert_eq!(
        vm.outputs()[0].envelope.data(),
        &Data::materialized(Value::Int(1)),
        "the payload itself"
    );
}

#[test]
fn a_mixed_brace_refuses_with_the_dedicated_diagnostic() {
    // An ordinary field beside a reserved root is never silently
    // reinterpreted — E-BRACE-MIXED names the offending field.
    let source = "graph g : App {\n  operator o {\n    on in {\n      out = {\n        v = 1\n        meta.flag = true\n      }\n    }\n  }\n  . -> o -> .\n}\n";
    let bag = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("the mix refuses");
    assert!(
        bag.diagnostics().iter().any(|diagnostic| {
            diagnostic.code().as_str() == "E-BRACE-MIXED"
                && diagnostic
                    .fields()
                    .iter()
                    .any(|(name, value)| *name == "field" && *value == "v")
        }),
        "the dedicated code, naming the field: {bag:?}"
    );
}

#[test]
fn a_block_entry_carries_its_root_into_the_classification() {
    // `ordinary { … }` beside `meta.*` is a MIX — and the diagnostic
    // names the block, because a sub-block's root is its name, not
    // an empty string.
    let source = "graph g : App {\n  operator o {\n    on in {\n      out = {\n        ordinary { x = 1 }\n        meta.flag = true\n      }\n    }\n  }\n  . -> o -> .\n}\n";
    let bag = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("the mix refuses");
    assert!(
        bag.diagnostics().iter().any(|diagnostic| {
            diagnostic.code().as_str() == "E-BRACE-MIXED"
                && diagnostic
                    .fields()
                    .iter()
                    .any(|(name, value)| *name == "field" && *value == "ordinary")
        }),
        "the block's name is the offending field: {bag:?}"
    );
    // And a `meta { … }` block ALONE still classifies as the
    // envelope form — then refuses as the DEAD block spelling with
    // its migration (dotted `meta.field = …` entries), never
    // silently a record (D-2026-08-12-1).
    let source = "graph g : App {\n  operator o {\n    on in {\n      out = {\n        meta { flag = true }\n      }\n    }\n  }\n  . -> o -> .\n}\n";
    let bag = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("the dead block refuses");
    assert!(
        bag.diagnostics().iter().any(|diagnostic| {
            diagnostic.code().as_str() == "E-CONTRACT-ATTR"
                && diagnostic
                    .message()
                    .contains("`meta { … }` is no longer a form")
        }),
        "the migration speaks on the envelope side too: {bag:?}"
    );
}

#[test]
fn identity_re_emits_the_materialized_envelope_whole() {
    // `out = .` — the exact re-emission: payload AND writable
    // metadata cross untouched. (The encoded twin lives in
    // `data_materializes_only_on_demand`.)
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, MetaValue, Namespace, Scalar, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in { out = . }\n  }\n  . -> o -> .\n}\n";
    let compilation = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut app = Namespace::new();
    // The one ext field this plan's schema declares — the standard
    // `content_type` (inject validates the zone, D-2026-08-10-1).
    app.insert(
        String::from("content_type"),
        MetaValue::Scalar(Scalar::Text(String::from("text/x-probe"))),
    );
    vm.inject(
        &mut journal,
        InboundSubmission {
            authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Struct(vec![(String::from("v"), Value::Int(1))])),
                ext: app,
            },
            sequence: SubmissionSequence::Next,
        },
    )
    .expect("the injection lands");
    vm.drain(&mut journal).expect("the drain completes");
    let output = &vm.outputs()[0];
    assert_eq!(
        output.envelope.data(),
        &Data::materialized(Value::Struct(vec![(String::from("v"), Value::Int(1),)])),
        "the payload crosses untouched"
    );
    assert_eq!(
        output.envelope.meta().ext().get("content_type"),
        Some(&MetaValue::Scalar(Scalar::Text(String::from(
            "text/x-probe"
        )))),
        "the writable metadata rides along"
    );
}

#[test]
fn an_empty_nested_record_is_a_value_not_an_absence() {
    // `parent = {}` writes an EMPTY STRUCT under `parent` — a
    // leaf-splice alone would write nothing and the field would
    // vanish. Every nesting and both braced forms hold it.
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, Namespace, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let run = |body: &str| {
        let source = format!(
            "graph g : App {{\n  input in: variant\n  operator o {{\n    on in {{ out = {body} }}\n  }}\n  . -> o -> .\n}}\n"
        );
        let compilation = compile(
            &[source.as_str()],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .expect("a valid plan");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        vm.inject(
            &mut journal,
            InboundSubmission {
                authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
                port: rastok::node::Port::new("in").unwrap(),
                draft: Draft {
                    data: Data::materialized(Value::Int(1)),
                    ext: Namespace::new(),
                },
                sequence: SubmissionSequence::Next,
            },
        )
        .expect("the injection lands");
        vm.drain(&mut journal).expect("the drain completes");
        vm.outputs()[0].envelope.data().clone()
    };
    let empty = || Value::Struct(Vec::new());
    // The record form.
    assert_eq!(
        run("{ parent = {} }"),
        Data::materialized(Value::Struct(vec![(String::from("parent"), empty())])),
    );
    // Two levels down.
    assert_eq!(
        run("{ parent = { child = {} } }"),
        Data::materialized(Value::Struct(vec![(
            String::from("parent"),
            Value::Struct(vec![(String::from("child"), empty())]),
        )])),
    );
    // The envelope form's whole-data record.
    assert_eq!(
        run("{ data = { parent = {} } }"),
        Data::materialized(Value::Struct(vec![(String::from("parent"), empty())])),
    );
}

#[test]
fn the_empty_record_travels_and_hashes_stably() {
    // The empty record remains a useful boundary case of the complete v7
    // struct family: it survives the package round trip, revalidates, and its
    // hash is deterministic across compiles.
    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in { out = { parent = {} } }\n  }\n  . -> o -> .\n}\n";
    let first = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let second = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    assert_eq!(first.hash, second.hash, "the hash is stable");
    let bytes = rastok::plan::encode(&first.plan, &first.hash);
    // `decode` re-validates: the empty-record literal is inside the
    // expressible subset, so the trip is total.
    let (_, hash) = rastok::plan::decode(&bytes).expect("the package round-trips");
    assert_eq!(hash, first.hash, "one identity across the trip");
}

#[test]
fn an_injection_holds_the_empty_record_too() {
    // The scenario builder and the machine share the same structural
    // value. A variant boundary is intentional here: the test is about
    // preserving an empty nested record, not inferring its schema.
    let source = r#"
type HollowPayload = struct { hollow: struct { } }

graph g : App {
  input in: variant
  output out: variant
  operator relay {
    on in { out = data }
  }
  . -> relay -> .
}

lab g {
  simulation probing on app {
    scenario s "empty records survive an injection" {
      app.in = { hollow = {} }
      expect "the empty record is a field" app.out count == 1 where data is HollowPayload
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
    use rastok::contract::{
        ConnectorContract, PortContract, PortDataType, PortTypeRule, SheetDefect,
    };
    use rastok::node::Direction;
    use rastok::plan::UnwiredPolicy;
    const MISDIRECTED: ConnectorContract = ConnectorContract {
        name: "Bad",
        ports: &[PortContract {
            name: "out",
            direction: Direction::Input,
            unwired: UnwiredPolicy::RequireWire,
            ty: PortTypeRule::Fixed(PortDataType::Variant),
        }],
        common: &[],
        selector: None,
    };
    const CONFLICTING: ConnectorContract = ConnectorContract {
        name: "Bad",
        ports: &[
            PortContract {
                name: "request",
                direction: Direction::Input,
                unwired: UnwiredPolicy::RequireWire,
                ty: PortTypeRule::Fixed(PortDataType::Variant),
            },
            PortContract {
                name: "request",
                direction: Direction::Output,
                unwired: UnwiredPolicy::RequireWire,
                ty: PortTypeRule::Fixed(PortDataType::Variant),
            },
        ],
        common: &[],
        selector: None,
    };
    // A default port against its implicit direction — the exact
    // defect that would have split the checker from the validator
    // through the old Registry fallback.
    assert_eq!(
        Registry::try_m0()
            .expect("the standard catalogue holds")
            .try_with(MISDIRECTED)
            .unwrap_err(),
        SheetDefect::MisdirectedDefault("out")
    );
    // One name in both directions.
    assert_eq!(
        Registry::try_m0()
            .expect("the standard catalogue holds")
            .try_with(CONFLICTING)
            .unwrap_err(),
        SheetDefect::ConflictingPort("request")
    );
    // The shipped M0 sheet passes its own door.
    Registry::try_m0()
        .expect("the standard catalogue holds")
        .try_with(rastok::stdlib::PROCESS)
        .expect("the Process sheet is valid");
}

#[test]
fn boundary_outputs_carry_their_port() {
    // Two named outputs are two different FACTS: the business API
    // answers (port, envelope) pairs, not an anonymous stream the
    // caller would have to re-derive from the plan.
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, Namespace, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let graph = &SORTER[..SORTER.find("lab ").expect("the lab block")];
    let compilation = compile(
        &[graph],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(
        &mut journal,
        InboundSubmission {
            authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Int(1)),
                ext: Namespace::new(),
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
    use rastok::envelope::{Data, Namespace, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let source = "graph g : App {\n  input alpha : int\n\n  operator o {\n    on a { out = data }\n  }\n  .alpha -> o.a\n  o -> .\n}\n";
    let compilation = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let submission = InboundSubmission {
        authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
        port: rastok::node::Port::new("alpha").unwrap(),
        draft: Draft {
            data: Data::materialized(Value::Text(String::from("wrong"))),
            ext: Namespace::new(),
        },
        sequence: SubmissionSequence::Next,
    };
    vm.inject(&mut journal, submission)
        .expect_err("a text draft cannot enter an int input");
    assert_eq!(journal.records().len(), 0, "refused BEFORE the append");
}

#[test]
fn an_unchanged_pass_through_holds_the_pin_at_run() {
    // The identity makes the boundary input int too. A text supplied
    // by the world is therefore refused at ingress, before a handler
    // or journal record can exist.
    let source = r#"
graph g : App {
  output result : int

  operator o {
    output score : int

    on in {
      score = .
    }
  }

  . -> o
  o.score -> .result
}

lab g {
  simulation probing on app {
    scenario s "a dynamic wrong shape faults" {
      app.in = "wrong"
    }
  }
}
"#;
    let compilation = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the graph has one resolved int flow");
    let runs = rastok::lab::run(
        &compilation.plan,
        rastok::plan::GraphVersion(1),
        &compilation.labs[0],
    );
    assert!(
        runs[0]
            .report
            .failures
            .iter()
            .any(|failure| failure.contains("injection was refused"))
    );
    assert!(runs[0].records.is_empty(), "the journal stays untouched");
}

#[test]
fn a_typed_destination_holds_against_an_untyped_source() {
    // The formerly "untyped" source is inferred from the destination.
    // The graph has one int flow, so a text supplied by the world is
    // refused at ingress rather than discovered after execution.
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
    }
  }
}
"#;
    let compilation = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the destination constrains the complete flow");
    let runs = rastok::lab::run(
        &compilation.plan,
        rastok::plan::GraphVersion(1),
        &compilation.labs[0],
    );
    assert!(
        runs[0]
            .report
            .failures
            .iter()
            .any(|failure| failure.contains("injection was refused"))
    );
    assert!(runs[0].records.is_empty(), "the journal stays untouched");
}

#[test]
fn a_connector_reply_holds_the_destination_pin() {
    // Process(lines).out is text. An int destination is impossible
    // independently of any partner outcome, so B2 rejects the edge
    // before a VM or journal exists.
    let source = r#"
graph g : App {
  input in: variant
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
      on in {
        out = {v = 1}
        exit = Exited { exit_code = 0, signal = null }
      }
    }

    scenario s "a structured reply cannot land on an int input" {
      app.in = "go"
      expect "nothing crosses" app.out count == 0
      expect "the fault is typed" app.err count == 1
    }
  }
}
"#;
    let bag = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("text cannot feed an int port");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-PORT-TYPE")
    );
}

// ===== the validator holds pinned edges =====

#[test]
fn the_validator_refuses_mismatched_edge_pins() {
    use rastok::plan::{PlanValidationError, ValidatedExecutionPlan};
    let compiled = || {
        compile(
            &[TYPED],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
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
    evt.ty = Some(rastok::datatype::DataType::INT);
    assert!(matches!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::MismatchedEdgePins { port, expected, found }
            if port.name() == "evt"
                && expected == rastok::datatype::DataType::INT
                && found == rastok::datatype::DataType::Text
    ));
    // Boundary target: the gauge fixture wires `meter.score : int`
    // into `.measure : int` — retune the boundary side to text.
    let gauge = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/gauge.rkg"),
    )
    .expect("the example reads");
    let mut plan = compile(
        &[gauge.as_str()],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan")
    .plan
    .into_inner();
    let measure = plan
        .boundary
        .iter_mut()
        .find(|decl| decl.port.name() == "measure")
        .expect("the pinned output");
    measure.ty = Some(rastok::datatype::DataType::Text);
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
    let bag = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("the alias breaches like the literal");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-PORT-TYPE"),
        "{bag:?}"
    );
}

// ===== round five — variant coherence, synthesized errors, the relay =====

#[test]
fn an_explicit_variant_is_dynamic_not_an_inference_hole() {
    // `variant` is a resolved logical type. Precise values can enter
    // it, but a dynamic value cannot enter a precise port without an
    // explicit narrowing operation.
    let flexible = "graph g : App {\n  input in: variant\n  operator o {\n    output flexible : variant\n    on in { flexible = 1 }\n  }\n  . -> o\n  o.flexible -> .\n}\n";
    compile(
        &[flexible],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("an int through a variant pin");

    let variant_to_int = "graph g : App {\n  input in: variant\n  operator a {\n    output flexible : variant\n    on in { flexible = data }\n  }\n  operator b {\n    input evt : int\n    on evt { out = data }\n  }\n  . -> a\n  a.flexible -> b.evt\n  b -> .\n}\n";
    let bag = compile(
        &[variant_to_int],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("variant cannot silently narrow to int");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-PORT-TYPE")
    );

    let int_to_variant = "graph g : App {\n  input in: variant\n  operator a {\n    output score : int\n    on in { score = 1 }\n  }\n  operator b {\n    input evt : variant\n    on evt { out = data }\n  }\n  . -> a\n  a.score -> b.evt\n  b -> .\n}\n";
    let compilation = compile(
        &[int_to_variant],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("int is assignable to variant");

    // The same rule holds for a raw plan re-earning the type after
    // the trip — the directional int-to-variant rule is preserved.
    let bytes = rastok::plan::encode(&compilation.plan, &compilation.hash);
    let (decoded, _) = rastok::plan::decode(&bytes).expect("the package opens");
    rastok::plan::ValidatedExecutionPlan::try_new(decoded.into_inner())
        .expect("the validator applies the same variant rule");
}

#[test]
fn a_synthesized_runtime_fault_makes_an_incompatible_edge_untypable() {
    // The VM is a real producer of source.err even though no source
    // Step::Emit names it. The resolved table includes that two-text map,
    // so `source.err -> sink.evt : int` refuses statically rather than
    // waiting for a faulted transaction to discover the contradiction.
    let source = r#"
graph g : App {
  operator source {
    on in { out = data as text }
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
"#;
    let bag = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("a map producer cannot feed an int consumer");
    assert!(
        bag.diagnostics().iter().any(|diagnostic| {
            diagnostic.code().as_str() == "E-PORT-TYPE"
                && diagnostic
                    .fields()
                    .iter()
                    .any(|(key, value)| *key == "port" && value == "evt")
                && diagnostic
                    .fields()
                    .iter()
                    .any(|(key, value)| *key == "found" && value == "map<text,text>")
        }),
        "the contradiction names the real hidden producer: {bag:?}"
    );
}

#[test]
fn a_decode_error_cannot_cross_a_pin() {
    // Process.err is variant in the B2 bridge. It cannot feed an int
    // destination without narrowing, regardless of which failure kind
    // might eventually be emitted.
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
      on in {
        out = .
        exit = Exited { exit_code = 0, signal = null }
      }
    }

    scenario s "a decode-error payload cannot enter an int pin" {
      app.in = {v = 1}
      expect "nothing crosses" app.out count == 0
    }
  }
}
"#;
    let bag = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("variant cannot silently narrow to int");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-PORT-TYPE")
    );
}

#[test]
fn the_bare_data_relay_preserves_the_logical_type() {
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, Namespace, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;
    use std::collections::BTreeMap;

    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in { out = data }\n  }\n  . -> o -> .\n}\n";
    let compilation = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
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
                    ext: Namespace::new(),
                },
                sequence: SubmissionSequence::Next,
            },
        )
        .expect("the injection lands");
        vm.drain(&mut journal).expect("the drain completes");
        vm.outputs()[0].envelope.data().clone()
    };
    // Struct in, Struct out — tag 17 stays tag 17.
    let sent = Value::Struct(vec![(String::from("field"), Value::Int(7))]);
    assert!(
        matches!(relay(sent), Data::Materialized(Value::Struct(_))),
        "a bare relay must not retype a struct"
    );
    // Map in, Map out — tag 18 stays tag 18.
    let mut entries = BTreeMap::new();
    entries.insert(String::from("k"), Value::Int(1));
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
    use rastok::envelope::{Data, Namespace, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::{Vm, VmError};

    let source = "graph g : App {\n  input alpha : int\n\n  operator o {\n    on a { out = data }\n  }\n  .alpha -> o.a\n  o -> .\n}\n";
    let compilation = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let error = vm
        .inject(
            &mut journal,
            InboundSubmission {
                authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
                port: rastok::node::Port::new("alpha").unwrap(),
                draft: Draft {
                    data: Data::materialized(Value::Text(String::from("wrong"))),
                    ext: Namespace::new(),
                },
                sequence: SubmissionSequence::Next,
            },
        )
        .unwrap_err();
    // The CLIENT's value is wrong, not the plan — the error says so
    // by name, and its structure survives: the submitted ingress AND
    // the endpoint whose pin actually refused (here the ingress
    // itself; a fan-out destination names itself instead).
    assert!(matches!(
        error,
        VmError::IngressPinViolation { ingress, rejected_at, .. }
            if ingress.name() == "alpha"
                && rejected_at.node.is_none()
                && rejected_at.port.name() == "alpha"
    ));
}

#[test]
fn the_lab_builder_seals_structs_before_a_typed_mock_reply() {
    // The Lab draft is held to tag 17. The lines mock answers with
    // text because its reply must conform to Process.out.
    let source = r#"
graph g : App {
  input in: variant
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
      on in {
        out = "reply"
        exit = Exited { exit_code = 0, signal = null }
      }
    }

    scenario s "sugar everywhere" {
      app.in = {field = 7}
      expect "the reply lands" app.out count == 1
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
    }
    assert!(inbound_struct, "the Lab draft seals tag 17");
}

#[test]
fn the_validator_reads_the_table_and_gates_the_capability() {
    // Two DISTINCT refusals prove the table is read without any Registry:
    // Process cannot grow a port beyond its resolved contract, while a
    // referenced row absent from the table refuses by ignorance.
    use rastok::datatype::DataType;
    use rastok::node::Direction;
    use rastok::plan::{Edge, EndpointRef, PortDecl, Slot, ValidatedExecutionPlan};
    let source = "graph g : App {\n  input in: text\n  output out: text\n  connector probe : Process {\n    mode = job ; command = \"cat\"\n    codec = lines\n    effect = idempotent ; behavior = deterministic\n  }\n  . -> probe -> .\n}\n";
    let mut plan = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the Process contract compiles")
    .plan
    .into_inner();
    let probe_slot = plan
        .nodes
        .iter()
        .position(|node| node.path.to_string() == "probe")
        .expect("the connector");
    let probe = &mut plan.nodes[probe_slot];
    probe.ports.push(PortDecl {
        port: rastok::node::Port::new("request").unwrap(),
        direction: Direction::Input,
        unwired: rastok::plan::UnwiredPolicy::RequireWire,
        ty: Some(DataType::Text),
    });
    probe
        .ports
        .sort_by(|left, right| left.key().cmp(&right.key()));
    plan.edges.push(Edge {
        from: EndpointRef {
            node: None,
            port: rastok::node::Port::new("in").unwrap(),
        },
        to: Some(EndpointRef {
            node: Some(Slot::new(u32::try_from(probe_slot).unwrap())),
            port: rastok::node::Port::new("request").unwrap(),
        }),
    });
    let mut edges = plan.edges.clone();
    edges.sort_by_key(|edge| plan.edge_text(edge));
    plan.edges = edges;
    let error = ValidatedExecutionPlan::try_new(plan.clone()).unwrap_err();
    assert!(matches!(
        &error,
        rastok::plan::PlanValidationError::UnexpectedContractPort { node, port }
            if node.to_string() == "probe" && port.name() == "request"
    ));
    // A port the table does NOT know refuses by ignorance instead —
    // the distinction is what proves the table answered.
    let ghost = plan
        .edges
        .iter_mut()
        .find(|edge| {
            edge.to
                .as_ref()
                .is_some_and(|to| to.port.name() == "request")
        })
        .expect("the spliced edge");
    ghost.to.as_mut().expect("a target").port = rastok::node::Port::new("ghost").unwrap();
    plan.nodes[probe_slot]
        .ports
        .retain(|port| port.port.name() != "request");
    let mut edges = plan.edges.clone();
    edges.sort_by_key(|edge| plan.edge_text(edge));
    plan.edges = edges;
    assert!(matches!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        rastok::plan::PlanValidationError::WrongPortDirection { port, .. } if port.name() == "ghost"
    ));
}

// ===== round six — the witness, the capability gate, the paths =====

#[test]
fn the_struct_witness_survives_every_equivalent_spelling() {
    // The logical type rides the VALUE through the Rastok expression bridge — not
    // an AST special case: a local binding and a ternary preserve it
    // exactly like the bare relay.
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, Namespace, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;
    use std::collections::BTreeMap;

    let relay = |body: &str, data: Value| {
        let source = format!(
            "graph g : App {{\n  input in: variant\n  operator o {{\n    on in {{\n      {body}\n    }}\n  }}\n  . -> o -> .\n}}\n"
        );
        let compilation = compile(
            &[source.as_str()],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .expect("a valid plan");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
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
        vm.drain(&mut journal).expect("the drain completes");
        vm.outputs()[0].envelope.data().clone()
    };
    let a_struct = || Value::Struct(vec![(String::from("field"), Value::Int(7))]);
    let a_map = || {
        let mut entries = BTreeMap::new();
        entries.insert(String::from("k"), Value::Int(1));
        Value::Map(entries)
    };
    for body in [
        "out = data",
        "val copy = data\n      out = copy",
        "out = true ? data : data",
    ] {
        assert!(
            matches!(
                relay(body, a_struct()),
                Data::Materialized(Value::Struct(_))
            ),
            "`{body}` must keep a struct a struct"
        );
        assert!(
            matches!(relay(body, a_map()), Data::Materialized(Value::Map(_))),
            "`{body}` must keep a map a map"
        );
    }
}

#[test]
fn a_mock_relay_keeps_the_struct_witness() {
    // A mock is checked against the same connector pin as a real adapter.
    // Each JSONL line is one variant emission, never a batch carrier.
    let source = r#"
graph g : App {
  input in: variant
  connector shell : Process {
    mode = job ; command = "cat"
    codec = jsonl
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
      on in {
        out = {"v": 1}
        exit = Exited { exit_code = 0, signal = null }
      }
    }

    scenario s "the mock reply follows the jsonl pin" {
      app.in = {"field": 7}
      expect "the reply lands" app.out count == 1
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
    assert!(runs[0].report.passed(), "{:#?}", runs[0].report.failures);
}

#[test]
fn an_encoded_bare_relay_passes_the_representation_through() {
    // DECIDED: `out = data` on an ENCODED input relays the
    // representation — bytes, encoding, and content-type hint —
    // before any Rastok expression binding materializes. An Avro relay therefore
    // works; reading an Avro FIELD still faults, typed.
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, Encoding};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in { out = data }\n  }\n  . -> o -> .\n}\n";
    let compilation = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(
        &mut journal,
        InboundSubmission {
            authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::encoded(Encoding::Avro, *b"\x01\x02"),
                ext: avro_ext(),
            },
            sequence: SubmissionSequence::Next,
        },
    )
    .expect("the injection lands");
    vm.drain(&mut journal).expect("an Avro RELAY works");
    let output = &vm.outputs()[0];
    assert!(
        matches!(
            output.envelope.data(),
            Data::Encoded { encoding: Encoding::Avro, bytes, .. } if bytes.as_slice() == b"\x01\x02"
        ),
        "the representation passes through: {:?}",
        output.envelope.data()
    );
    assert_eq!(
        output.envelope.meta().ext().get("content_type"),
        Some(&rastok::envelope::MetaValue::Scalar(
            rastok::envelope::Scalar::Text(String::from("application/avro"))
        )),
        "the content type rides the ext zone"
    );
}

#[test]
fn a_spawn_failure_cannot_cross_a_pin() {
    // The exact failure kind is irrelevant to the graph type: the
    // variant error port cannot enter an int input without narrowing.

    let source = "graph g : App {\n  connector shell : Process {\n    mode = job ; command = \"/definitely/not/a/binary\"\n    codec = lines\n    effect = idempotent ; behavior = deterministic\n  }\n\n  operator sink {\n    input evt : int\n    on evt { out = data }\n  }\n\n  . -> shell\n  shell.err -> sink.evt\n  shell.out -> .\n  sink -> .\n}\n";
    let bag = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("variant cannot silently narrow to int");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-PORT-TYPE")
    );
}

#[test]
fn a_timeout_error_cannot_cross_a_pin() {
    // Scheduler failures have the same resolved variant port as
    // partner failures, so this impossible edge is rejected statically.
    let source = r#"
graph g : App {
  connector shell : Process {
    mode = job ; command = "cat"
    codec = lines
    timeout = duration("50ms")
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
      on in { out = nothing }
    }

    scenario s "a timeout payload cannot enter an int pin" {
      app.in = "go"
      advance duration("1s")
      expect "nothing crosses" app.out count == 0
    }
  }
}
"#;
    let bag = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("variant cannot silently narrow to int");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-PORT-TYPE")
    );
}

#[test]
fn a_struct_initializer_cannot_enter_an_int_state_cell() {
    // V7 carries struct constants, but the resolved state-cell type still
    // judges the initializer before a hand-built plan can become executable.
    use rastok::plan::{PlanValidationError, ValidatedExecutionPlan};
    let source = "graph g : App {\n  input in: variant\n  operator relay {\n    var seen : int = 0\n    on in { seen = seen + 1\n      out = data }\n  }\n  . -> relay -> .\n}\n";
    let mut plan = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan")
    .plan
    .into_inner();
    let relay = plan
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "relay")
        .expect("the operator");
    relay.vars[0].init = Some(rastok::value::Value::Struct(vec![(
        String::from("field"),
        rastok::value::Value::Int(7),
    )]));
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::UnrepresentableConstant
    );
}

// ===== round seven, phase one — laziness, name-keyed structs =====

#[test]
fn data_materializes_only_on_demand() {
    // An expression that never reads `data` must never force an
    // opaque payload to decode: a `true` guard, a state write, and
    // an identity emission all leave Avro bytes untouched.
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, Encoding};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let run = |source: &str| {
        let compilation = compile(
            &[source],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .expect("a valid plan");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        vm.inject(
            &mut journal,
            InboundSubmission {
                authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
                port: rastok::node::Port::new("in").unwrap(),
                draft: Draft {
                    data: Data::encoded(Encoding::Avro, *b"\x01\x02"),
                    ext: avro_ext(),
                },
                sequence: SubmissionSequence::Next,
            },
        )
        .expect("the injection lands");
        vm.drain(&mut journal).expect("no forced materialization");
        (vm.outputs().to_vec(), vm.errors().len())
    };
    // A guard reading nothing + identity: bytes, encoding, and MIME
    // survive whole.
    let (outputs, errors) = run(
        "graph g : App {\n  input in: variant\n  operator o {\n    on in where true { out = . }\n  }\n  . -> o -> .\n}\n",
    );
    assert_eq!(errors, 0, "no opaque read was asked for");
    assert!(matches!(
        outputs[0].envelope.data(),
        Data::Encoded {
            encoding: Encoding::Avro,
            ..
        }
    ));
    assert_eq!(
        outputs[0].envelope.meta().ext().get("content_type"),
        Some(&rastok::envelope::MetaValue::Scalar(
            rastok::envelope::Scalar::Text(String::from("application/avro"))
        ))
    );
    // A false guard consumes without decoding and without error.
    let (outputs, errors) = run(
        "graph g : App {\n  input in: variant\n  operator o {\n    on in where false { out = . }\n  }\n  . -> o -> .\n}\n",
    );
    assert_eq!((outputs.len(), errors), (0, 0));
    // A meta-only guard touches meta, never data.
    let (outputs, errors) = run(
        "graph g : App {\n  input in: variant\n  operator o {\n    on in where meta.sys.graph_version == 1 { out = . }\n  }\n  . -> o -> .\n}\n",
    );
    assert_eq!(errors, 0);
    assert_eq!(outputs.len(), 1);
    // A state write independent of data materializes nothing.
    let (outputs, errors) = run(
        "graph g : App {\n  input in: variant\n  operator o {\n    var seen : int = 0\n    on in {\n      seen = seen + 1\n      out = .\n    }\n  }\n  . -> o -> .\n}\n",
    );
    assert_eq!(errors, 0);
    assert_eq!(outputs.len(), 1);
}

#[test]
fn short_circuits_never_demand_the_data() {
    // The demand comes from the EVALUATOR, not a pre-scan of the
    // tree: a branch the evaluation never takes must never force an
    // opaque payload to decode, even though `data` appears in it.
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, Encoding, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let run = |source: &str| {
        let compilation = compile(
            &[source],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .expect("a valid plan");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        vm.inject(
            &mut journal,
            InboundSubmission {
                authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
                port: rastok::node::Port::new("in").unwrap(),
                draft: Draft {
                    data: Data::encoded(Encoding::Avro, *b"\x01\x02"),
                    ext: avro_ext(),
                },
                sequence: SubmissionSequence::Next,
            },
        )
        .expect("the injection lands");
        vm.drain(&mut journal).expect("no forced materialization");
        (vm.outputs().to_vec(), vm.errors().len())
    };
    // `false && (data as bool)`: the right side never evaluates —
    // filtered, no decode, no fault.
    let (outputs, errors) = run(
        "graph g : App {\n  input in: variant\n  operator o {\n    on in where false && (data as bool) { out = . }\n  }\n  . -> o -> .\n}\n",
    );
    assert_eq!((outputs.len(), errors), (0, 0), "filtered without a fault");
    // `true || (data as bool)`: the identity ships the bytes whole.
    let (outputs, errors) = run(
        "graph g : App {\n  input in: variant\n  operator o {\n    on in where true || (data as bool) { out = . }\n  }\n  . -> o -> .\n}\n",
    );
    assert_eq!(errors, 0, "the right side never ran");
    assert!(matches!(
        outputs[0].envelope.data(),
        Data::Encoded { encoding: Encoding::Avro, bytes, .. } if bytes.as_slice() == b"\x01\x02"
    ));
    // `false ? size(data) : 1`: the untaken arm never demands.
    let (outputs, errors) = run(
        "graph g : App {\n  input in: variant\n  operator o {\n    on in { out = false ? size(data) : 1 }\n  }\n  . -> o -> .\n}\n",
    );
    assert_eq!(errors, 0, "the untaken arm never ran");
    assert_eq!(
        outputs[0].envelope.data(),
        &Data::materialized(Value::Int(1))
    );
    // The control: a taken read still faults, typed.
    let (outputs, errors) = run(
        "graph g : App {\n  input in: variant\n  operator o {\n    on in where true && (data as bool) { out = . }\n  }\n  . -> o -> .\n}\n",
    );
    assert_eq!((outputs.len(), errors), (0, 1), "a real read still faults");
}

#[test]
fn the_carry_is_as_lazy_as_the_name() {
    // Semantics, not spelling: `val copy = data; out = copy` and a
    // taken ternary arm behave exactly like `out = data` — the whole
    // payload ships its REPRESENTATION (bytes, encoding, MIME),
    // opaque or not, however the carry is written.
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, Encoding, Namespace};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let run = |body: &str, encoding: Encoding, bytes: &'static [u8]| {
        let source = format!(
            "graph g : App {{\n  input in: variant\n  operator o {{\n    on in {{\n{body}\n    }}\n  }}\n  . -> o -> .\n}}\n"
        );
        let compilation = compile(
            &[source.as_str()],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .expect("a valid plan");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        vm.inject(
            &mut journal,
            InboundSubmission {
                authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
                port: rastok::node::Port::new("in").unwrap(),
                draft: Draft {
                    data: Data::encoded(encoding, bytes.to_vec()),
                    ext: {
                        let mut ext = Namespace::new();
                        ext.insert(
                            String::from("content_type"),
                            rastok::envelope::MetaValue::Scalar(rastok::envelope::Scalar::Text(
                                String::from("application/x-opaque"),
                            )),
                        );
                        ext
                    },
                },
                sequence: SubmissionSequence::Next,
            },
        )
        .expect("the injection lands");
        vm.drain(&mut journal).expect("no forced materialization");
        assert_eq!(vm.errors().len(), 0, "no decode was demanded");
        vm.outputs()[0].envelope.clone()
    };
    let holds = |body: &str| {
        let output = run(body, Encoding::Avro, b"\x01\x02");
        assert!(
            matches!(
                output.data(),
                Data::Encoded { encoding: Encoding::Avro, bytes, .. } if bytes.as_slice() == b"\x01\x02"
            ),
            "`{body}` must ship the representation: {:?}",
            output.data()
        );
        assert_eq!(
            output.meta().ext().get("content_type"),
            Some(&rastok::envelope::MetaValue::Scalar(
                rastok::envelope::Scalar::Text(String::from("application/x-opaque"))
            )),
            "`{body}` must keep the content type"
        );
    };
    holds("      out = data");
    holds("      val copy = data\n      out = copy");
    holds("      out = true ? data : 1");
    holds("      val copy = true ? data : data\n      out = copy");
}

#[test]
fn a_field_access_materializes_exactly_once_per_envelope() {
    // The envelope's own cell is the cache authority: the first
    // structural access fills it; every later demand — this
    // transaction's or a later node's — reads the cell.
    use rastok::envelope::{Data, Encoding};
    use rastok::runtime::DataSlot;

    let data = Data::encoded(Encoding::Raw, *b"seven");
    assert!(data.materialized_value().is_none(), "nothing read yet");
    let slot = DataSlot::new(&data);
    let first = slot.materialize().expect("raw decodes");
    let second = slot.materialize().expect("the cache answers");
    assert_eq!(first, second);
    assert!(
        data.materialized_value().is_some(),
        "the decode landed in the ENVELOPE's cell, not a side cache"
    );
}

#[test]
fn an_inspection_then_identity_ships_the_filled_cell() {
    // A node inspects `data`, then re-emits the envelope whole: the
    // emitted data carries the FILLED cell, so the next node's
    // demand is a cache read — once per envelope, not once per
    // transaction.
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, Encoding, Namespace};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in where size(data) > 0 { out = . }\n  }\n  . -> o -> .\n}\n";
    let compilation = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(
        &mut journal,
        InboundSubmission {
            authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::encoded(Encoding::Raw, *b"ok"),
                ext: Namespace::new(),
            },
            sequence: SubmissionSequence::Next,
        },
    )
    .expect("the injection lands");
    vm.drain(&mut journal).expect("the drain completes");
    let output = &vm.outputs()[0];
    match output.envelope.data() {
        Data::Encoded { bytes, .. } => {
            assert_eq!(bytes.as_slice(), b"ok", "the bytes ship untouched");
        }
        other => panic!("the representation must survive: {other:?}"),
    }
    assert!(
        output.envelope.data().materialized_value().is_some(),
        "the guard's decode rides the emitted envelope"
    );
}

#[test]
fn struct_equality_is_by_name_like_the_seal() {
    use rastok::envelope::Value;
    let left = Value::Struct(vec![
        (String::from("a"), Value::Int(1)),
        (String::from("b"), Value::Int(2)),
    ]);
    let right = Value::Struct(vec![
        (String::from("b"), Value::Int(2)),
        (String::from("a"), Value::Int(1)),
    ]);
    // One canonical identity, one value — equality agrees with the
    // sorted seal in every judge.
    assert_eq!(left, right, "envelope equality is by name");
    assert_eq!(
        rastok_core::canonical_cbor::encode(&rastok_core::identity::encoding::value_tagged(&left)),
        rastok_core::canonical_cbor::encode(&rastok_core::identity::encoding::value_tagged(&right)),
        "the canonical bytes already agreed"
    );
    // The Rastok expression operator agrees too.
    let signature = rastok::expr::Signature::new()
        .with("a", rastok::datatype::DataType::Variant)
        .with("b", rastok::datatype::DataType::Variant);
    let tree = rastok::expr::compile("a == b", &signature).expect("a valid expression");
    let mut env = std::collections::BTreeMap::new();
    env.insert(
        String::from("a"),
        rastok::value::Value::Struct(vec![
            (String::from("x"), rastok::value::Value::Int(1)),
            (String::from("y"), rastok::value::Value::Int(2)),
        ])
        .into(),
    );
    env.insert(
        String::from("b"),
        rastok::value::Value::Struct(vec![
            (String::from("y"), rastok::value::Value::Int(2)),
            (String::from("x"), rastok::value::Value::Int(1)),
        ])
        .into(),
    );
    assert_eq!(
        rastok::expr::eval(&tree, &env).expect("the comparison evaluates"),
        rastok::expr::EvalValue::Bool(true),
        "Rastok expression == follows the canonical identity"
    );
}

#[test]
fn a_duplicated_struct_field_refuses_at_the_door() {
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, Namespace, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in { out = . }\n  }\n  . -> o -> .\n}\n";
    let compilation = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let error = vm
        .inject(
            &mut journal,
            InboundSubmission {
                authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
                port: rastok::node::Port::new("in").unwrap(),
                draft: Draft {
                    data: Data::materialized(Value::Struct(vec![
                        (String::from("twin"), Value::Int(1)),
                        (String::from("twin"), Value::Int(2)),
                    ])),
                    ext: Namespace::new(),
                },
                sequence: SubmissionSequence::Next,
            },
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("`twin` appears twice"),
        "typed, never first-found: {error}"
    );
    assert_eq!(journal.records().len(), 0, "refused before the append");
}

#[test]
fn the_conformance_judgement_is_matchable() {
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, Namespace, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::{Vm, VmError};
    use rastok_core::runtime::{ConformanceReason, ValueKind};

    let source = "graph g : App {\n  input alpha : int\n\n  operator o {\n    on a { out = data }\n  }\n  .alpha -> o.a\n  o -> .\n}\n";
    let compilation = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let submission = |data: Value| InboundSubmission {
        authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
        port: rastok::node::Port::new("alpha").unwrap(),
        draft: Draft {
            data: Data::materialized(data),
            ext: Namespace::new(),
        },
        sequence: SubmissionSequence::Next,
    };
    // A tool MATCHES the judgement — no message parsing.
    let error = vm
        .inject(&mut journal, submission(Value::Text(String::from("wrong"))))
        .unwrap_err();
    assert!(matches!(
        error,
        VmError::IngressPinViolation { violation, .. }
            if violation.expected == rastok::datatype::DataType::INT
                && violation.found == ValueKind::Text
                && violation.reason == ConformanceReason::TypeMismatch
    ));
    let error = vm
        .inject(&mut journal, submission(Value::Int(1 << 40)))
        .unwrap_err();
    assert!(matches!(
        error,
        VmError::IngressPinViolation { violation, .. }
            if matches!(
                violation.reason,
                ConformanceReason::SignedIntegerOutOfRange { value, bits: 32 }
                    if value == 1 << 40
            )
    ));
}
