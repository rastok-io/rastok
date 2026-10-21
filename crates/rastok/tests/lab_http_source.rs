//! The Lab substitutes an inbound HTTP peer, never admission or Call bookkeeping.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::compiler::compile;
use rastok::contract::ContractCatalog;
use rastok::journal::Record;
use rastok::lab::{self, Simulator};
use rastok::plan::GraphVersion;

const SOURCE: &str = include_str!("fixtures/lab-http-source.rkg");

#[test]
fn decoded_requests_use_the_source_contract_without_a_physical_driver_or_app_input() {
    let catalogue = ContractCatalog::try_m0().unwrap();
    let production = compile(&[SOURCE.split("lab webhook").next().unwrap()], &catalogue).unwrap();
    let compilation = compile(&[SOURCE], &catalogue)
        .unwrap_or_else(|bag| panic!("a decoded source request must check: {bag:#?}"));
    assert_eq!(
        production.hash, compilation.hash,
        "the Lab is not the graph"
    );
    assert_eq!(
        rastok::plan::encode(&production.plan, &production.hash),
        rastok::plan::encode(&compilation.plan, &compilation.hash)
    );
    assert!(
        compilation
            .plan
            .resolved_ports()
            .boundary()
            .iter()
            .all(|port| { port.direction() == rastok::node::PortDirection::Output }),
        "no artificial App input"
    );

    let first = lab::run(&compilation.plan, GraphVersion(1), &compilation.labs[0]);
    assert!(first[0].report.passed(), "{:#?}", first[0].report);
    let second = lab::run(&compilation.plan, GraphVersion(1), &compilation.labs[0]);
    assert_eq!(first, second, "the entire scripted world is deterministic");
    let inbound: Vec<_> = first[0]
        .records
        .iter()
        .filter_map(|record| match record {
            Record::Ingress(record) => Some(record),
            _ => None,
        })
        .collect();
    assert_eq!(inbound.len(), 2, "one root per admitted request");
    assert!(
        inbound
            .iter()
            .all(|record| record.provenance.connector_path().to_string() == "listener")
    );
    assert_eq!(
        inbound.iter().map(|record| record.seq).collect::<Vec<_>>(),
        [0, 1]
    );
    assert!(inbound.iter().all(|record| {
        record.connector_path.as_ref().unwrap().to_string() == "listener"
            && record.port.name() == "out"
            && matches!(
                record.exchange.as_deref(),
                Some(rastok::journal::InboundExchangeRecord::Call(_))
            )
    }));
    assert_eq!(
        inbound[0].envelope.meta().ext().get("http_method"),
        Some(&rastok::envelope::MetaValue::Scalar(
            rastok::envelope::Scalar::Text(String::from("POST"))
        ))
    );
    assert!(
        inbound[1]
            .envelope
            .meta()
            .ext()
            .get("http_method")
            .is_none(),
        "the Lab invents no transport metadata"
    );
    assert!(
        rastok::connectors::RuntimeDrivers::prepare(
            &compilation.plan,
            &rastok::connectors::DriverRegistry::default(),
        )
        .is_err(),
        "real execution still needs its Driver binding"
    );

    let simulation = &compilation.labs[0].simulations[0];
    let mut stepped = Simulator::try_new(
        &compilation.plan,
        GraphVersion(1),
        &simulation.name,
        &simulation.mocks,
        &simulation.scenarios[0],
        None,
    )
    .unwrap();
    let draft = stepped.next_injection_draft().unwrap().unwrap();
    assert_eq!(draft.connector.unwrap().to_string(), "listener");
    assert_eq!(draft.port.name(), "out");
    for _ in 0..100 {
        if stepped.is_complete() {
            break;
        }
        if stepped.is_quiescent() {
            stepped.next_action().unwrap();
        } else {
            stepped.step_transaction().unwrap();
        }
    }
    assert!(stepped.is_complete());
    let stepped = stepped.finish();
    assert!(stepped.report.passed(), "{:#?}", stepped.report);
    assert_eq!(
        first[0].records, stepped.records,
        "batch and stepping share one world"
    );
}

#[test]
fn source_injections_reject_wrong_ports_types_and_unmodeled_profiles_at_check() {
    let catalogue = ContractCatalog::try_m0().unwrap();
    for (before, after, code, message) in [
        (
            "listener.out =",
            "label.in =",
            "E-CHECK-DEFERRED",
            "Operator",
        ),
        (
            "listener.out =",
            "label.out =",
            "E-CHECK-DEFERRED",
            "Operator",
        ),
        (
            "listener.out =",
            "listener.exit =",
            "E-CHECK-DEFERRED",
            "terminal",
        ),
        (
            "listener.out =",
            "listener.missing =",
            "E-PORT-UNKNOWN",
            "missing",
        ),
        (
            "listener.out =",
            "listener.in =",
            "E-PORT-DIRECTION",
            "input port",
        ),
        (
            "listener.out =",
            "absent.out =",
            "E-RESOLVE-UNKNOWN",
            "absent",
        ),
        (
            "listener.out =",
            "listener =",
            "E-CHECK-DEFERRED",
            "explicit source port",
        ),
        (
            "listener.out = \"second\"",
            "listener.out = 17",
            "E-PORT-TYPE",
            "",
        ),
        (
            "codec = text",
            "codec = text ; mode = respond ; timeout = duration(\"1s\")",
            "E-CHECK-DEFERRED",
            "respond",
        ),
    ] {
        let source = SOURCE.replace(before, after);
        let bag = compile(&[&source], &catalogue).expect_err(after);
        assert!(
            bag.diagnostics().iter().any(|diagnostic| {
                diagnostic.code().as_str() == code
                    && diagnostic.message().contains(message)
                    && !diagnostic.spans().is_empty()
            }),
            "{after}: {bag:#?}"
        );
    }
}

#[test]
fn a_discarded_source_is_still_observed_once_from_its_inbound_fact() {
    let source = r#"
graph discard : App {
  connector listener : HttpServer { listen = "127.0.0.1:8080" ; codec = text }
  listener -> nothing
  listener.exit -> nothing
}
lab discard {
  simulation requests on app {
    scenario one {
      listener.out = "discarded deliberately"
      expect listener.out count == 1
      expect listener.exit count == 1 where data is Completed
      advance duration("1s")
      expect nothing listener.out
      expect nothing listener.exit
    }
  }
}
"#;
    let compilation = compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let run = lab::run(&compilation.plan, GraphVersion(1), &compilation.labs[0]).remove(0);
    assert!(run.report.passed(), "{:#?}", run.report);
}

#[test]
fn a_nested_source_keeps_its_exact_connector_provenance() {
    let source = r#"
graph nested : App {
  output out: text
  graph ingress {
    output out: text
    connector listener : HttpServer { listen = "127.0.0.1:8080" ; codec = text }
    listener -> out
    listener.exit -> nothing
  }
  ingress -> out
}
lab nested {
  simulation requests on app {
    scenario one {
      ingress.listener.out = "hello"
      expect app.out count == 1 where data == "hello"
      expect ingress.listener.exit count == 1 where data is Completed
    }
  }
}
"#;
    let compilation = compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let run = lab::run(&compilation.plan, GraphVersion(1), &compilation.labs[0]).remove(0);
    assert!(run.report.passed(), "{:#?}", run.report);
    let Record::Ingress(root) = &run.records[0] else {
        panic!("source root")
    };
    assert_eq!(
        root.connector_path.as_ref().unwrap().to_string(),
        "ingress.listener"
    );
}

#[test]
fn decoded_text_bytes_and_json_requests_use_their_resolved_pins() {
    let catalogue = ContractCatalog::try_m0().unwrap();
    for (codec, ty, value) in [
        ("text", "text", "\"hello\""),
        ("bytes", "bytes", "base64! { aGVsbG8= }"),
        ("json", "variant", "json! { \"ok\": true }"),
    ] {
        let source = format!(
            r#"
graph typed : App {{
  output out: {ty}
  connector listener : HttpServer {{ listen = "127.0.0.1:8080" ; codec = {codec} }}
  listener -> out
  listener.exit -> nothing
}}
lab typed {{
  simulation requests on app {{
    scenario one {{
      listener.out = {value}
      expect listener.out count == 1
      expect app.out count == 1
      expect listener.exit count == 1 where data is Completed
    }}
  }}
}}
"#
        );
        let compilation =
            compile(&[&source], &catalogue).unwrap_or_else(|bag| panic!("{codec}: {bag:#?}"));
        let run = lab::run(&compilation.plan, GraphVersion(1), &compilation.labs[0]).remove(0);
        assert!(run.report.passed(), "{codec}: {:#?}", run.report);
    }
}

#[test]
fn a_sequential_scenario_cannot_skip_an_unadmitted_source_request_and_pass() {
    let source = r#"
graph blocked : App {
  input partner: text
  output out: text
  connector listener : HttpServer { listen = "127.0.0.1:8080" ; codec = text }
  operator pair {
    input a: text
    input b: text
    output out: text
    on (a, b) { out = a.data }
  }
  listener -> pair.a
  listener.exit -> nothing
  partner -> pair.b
  pair -> out
}
lab blocked {
  simulation requests on app {
    scenario no_partner {
      listener.out = "waiting for b"
      expect nothing app.out
    }
  }
}
"#;
    let mut compilation = compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let capacity = rastok::runtime::execution::Execution::new(&compilation.plan, GraphVersion(1))
        .inspection()
        .mailbox_capacity;
    let scenario = &mut compilation.labs[0].simulations[0].scenarios[0];
    let injection = scenario.steps[0].clone();
    let expectation = scenario.steps[1].clone();
    scenario.steps = vec![injection; capacity + 1];
    scenario.steps.push(expectation);
    let run = lab::run(&compilation.plan, GraphVersion(1), &compilation.labs[0]).remove(0);
    assert!(!run.report.passed());
    assert!(
        run.report
            .failures
            .iter()
            .any(|failure| failure.contains("pending under mailbox pressure")),
        "{:#?}",
        run.report
    );
    assert_eq!(
        run.records
            .iter()
            .filter(|record| matches!(record, Record::Ingress(_)))
            .count(),
        capacity
    );
}
