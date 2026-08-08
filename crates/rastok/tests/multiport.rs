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
