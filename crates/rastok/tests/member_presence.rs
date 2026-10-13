//! Optional JSON members are inspected by the pure expression consumer.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::{compiler::compile, contract::ContractCatalog, expr, plan};

const SOURCE: &str = r#"
graph presence : App {
  input in: map<text, variant>
  output out: bool
  operator inspect {
    input in: map<text, variant>
    output out: bool
    on in { out = has(data.usage) }
  }
  in -> inspect -> out
}
lab presence {
  simulation objects on app {
    scenario absent {
      app.in = { "other": 1 }
      expect app.out count == 1 where data == false
    }
    scenario null_is_present {
      app.in = { "usage": null }
      expect app.out count == 1 where data == true
    }
    scenario zero_is_present {
      app.in = { "usage": 0 }
      expect app.out count == 1 where data == true
    }
  }
}
"#;

#[test]
fn presence_survives_package_validation_and_execution() {
    let compiled = compile(&[SOURCE], &ContractCatalog::try_m0().unwrap()).unwrap();
    let bytes = plan::encode(&compiled.plan, &compiled.hash);
    let (decoded, hash) = plan::decode(&bytes).unwrap();
    assert_eq!(decoded, compiled.plan);
    assert_eq!(hash, compiled.hash);
    assert_eq!(plan::encode(&decoded, &hash), bytes);
    let runs = rastok::lab::run(&decoded, plan::GraphVersion(1), &compiled.labs[0]);
    assert_eq!(runs.len(), 3);
    assert!(runs.iter().all(|run| run.report.passed()), "{runs:?}");
}

#[test]
fn pure_constants_do_not_read_the_missing_final_member() {
    let source = SOURCE
        .replace("graph presence", "val options = { \"model\": \"fixture\" }\nval configured = has(options.usage)\ngraph presence")
        .replace("out = has(data.usage)", "out = configured")
        .replace("data == true", "data == false");
    let compiled = compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let runs = rastok::lab::run(&compiled.plan, plan::GraphVersion(1), &compiled.labs[0]);
    assert!(runs.iter().all(|run| run.report.passed()), "{runs:?}");
}

#[test]
fn a_forged_presence_call_cannot_hide_its_receiver_or_key_type() {
    let compiled = compile(&[SOURCE], &ContractCatalog::try_m0().unwrap()).unwrap();
    for args in [
        vec![],
        vec![expr::Expr::Path(vec!["data".into()])],
        vec![
            expr::Expr::Path(vec!["data".into()]),
            expr::Expr::Path(vec!["data".into()]),
        ],
        vec![
            expr::Expr::Literal(rastok::envelope::Value::Int(7)),
            expr::Expr::Literal(rastok::envelope::Value::Text("usage".into())),
        ],
    ] {
        let mut raw = compiled.plan.clone().into_inner();
        let plan::NodeExecution::Operator { handlers, .. } = &mut raw.nodes[0].execution else {
            panic!("one Operator")
        };
        let plan::Dispatch::Linear(steps) = &mut handlers[0].dispatch else {
            panic!("one linear handler")
        };
        let plan::DispatchStep::Emit {
            value: plan::EmitValue::Data(value),
            ..
        } = &mut steps[0]
        else {
            panic!("one value emission")
        };
        // Keep the serialized result claim Bool. Independent checking must
        // derive validity from the operands, not from that claim.
        value.root = expr::Expr::Call {
            function: expr::Function::Has,
            args,
        };
        assert!(plan::ValidatedExecutionPlan::try_from(raw).is_err());
    }
}

#[test]
fn presence_and_presence_control_flow_preserve_receiver_classification() {
    for body in [
        "out = has(data.usage)",
        "match { has(data.usage) => out = true; _ => out = false }",
    ] {
        let source = SOURCE
            .replace(
                "input in: map<text, variant>",
                "input in: map<text, variant> @sensitive(level = restricted)",
            )
            .replace("out = has(data.usage)", body);
        let compiled = compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
        let (decoded, _) = plan::decode(&plan::encode(&compiled.plan, &compiled.hash)).unwrap();
        let output = decoded
            .resolved_ports()
            .boundary()
            .iter()
            .find(|port| port.port().name() == "out")
            .unwrap();
        assert!(
            !output.annotations().is_empty(),
            "presence cannot declassify the receiver"
        );
    }
}
