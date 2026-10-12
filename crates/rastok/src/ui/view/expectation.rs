use super::*;
use crate::compiler::hir::ExpectTarget;
use crate::expr::{BinaryOp, EvalValue, Expr};

/// Explains the last checked assertion from the Lab's exact observation
/// window. Protected values and predicate results do not enter the read model.
#[must_use]
pub fn expectation_json(
    plan: &ValidatedExecutionPlan,
    session: &crate::lab::Simulator<'_, '_>,
) -> JsonValue {
    let Some(inspection) = session.last_expectation() else {
        return JsonValue::Null;
    };
    if explanation::classified(plan) {
        return json!({"redacted": true, "failed": inspection.failure.is_some(), "reason": "Assertion evidence is redacted because this program carries classified data."});
    }
    let (path, port) = match &inspection.expectation.target {
        ExpectTarget::Boundary(port) => (None, port),
        ExpectTarget::Node { path, port } => (Some(path), port),
    };
    let annotations = path
        .map(|path| node_port_annotations(plan, path, port))
        .unwrap_or_else(|| {
            plan.resolved_ports()
                .boundary()
                .iter()
                .find(|candidate| candidate.port() == port)
                .map(crate::plan::ResolvedPort::annotations)
                .unwrap_or(&EMPTY_ANNOTATIONS)
        });
    let rows: Vec<_> = inspection.envelopes.iter().map(|envelope| {
        let mut env = BTreeMap::new();
        env.insert(String::from("meta"), EvalValue::from(crate::runtime::meta_value(envelope.meta())));
        if let Some(value) = envelope.data().materialized_value() { env.insert(String::from("data"), EvalValue::from(value.clone())); }
        let matched = inspection.expectation.predicate.as_ref().map_or(Some(true), |predicate| match crate::expr::eval(predicate, &env) { Ok(EvalValue::Bool(value)) => Some(value), _ => None });
        let mut differences = Vec::new();
        if let Some(predicate) = &inspection.expectation.predicate {
            explain_comparisons(&predicate.root, &env, &mut differences);
        }
        json!({ "envelope_id": envelope.meta().sys().id().to_string(), "matched": matched,
            "data": data_json(envelope.data(), annotations), "meta": meta_json(envelope.meta()), "comparisons": differences })
    }).collect();
    let matched = rows
        .iter()
        .all(|row| row["matched"].is_boolean())
        .then(|| rows.iter().filter(|row| row["matched"] == true).count());
    json!({ "failed": inspection.failure.is_some(), "message": inspection.failure,
        "target": format!("{}.{}", path.map_or_else(|| String::from("App"), ToString::to_string), port),
        "quantifier": format!("{:?}", inspection.expectation.quantifier),
        "observed": rows.len(), "matched": matched, "envelopes": rows })
}

fn explain_comparisons(
    expr: &Expr,
    env: &BTreeMap<String, EvalValue>,
    output: &mut Vec<JsonValue>,
) {
    let Expr::Binary { op, left, right } = expr else {
        return;
    };
    if *op == BinaryOp::And {
        explain_comparisons(left, env, output);
        // Explain only predicates the short-circuit evaluator actually visits.
        if evaluate(left, env).is_some_and(|value| value == Value::Bool(true)) {
            explain_comparisons(right, env, output);
        }
        return;
    }
    let operator = match op {
        BinaryOp::Eq => "==",
        BinaryOp::Ne => "!=",
        BinaryOp::Lt => "<",
        BinaryOp::Le => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::Ge => ">=",
        _ => return,
    };
    let Expr::Path(path) = left.as_ref() else {
        return;
    };
    if let (Some(actual), Some(expected), Some(Value::Bool(held))) = (
        evaluate(left, env),
        evaluate(right, env),
        evaluate(expr, env),
    ) {
        output.push(json!({ "field": path.join("."), "operator": operator,
            "actual": materialized_value_json(&actual, &EMPTY_ANNOTATIONS), "expected": materialized_value_json(&expected, &EMPTY_ANNOTATIONS), "held": held }));
    }
}

fn evaluate(expr: &Expr, env: &BTreeMap<String, EvalValue>) -> Option<Value> {
    crate::expr::eval(
        &crate::expr::Tree {
            root: expr.clone(),
            ty: crate::datatype::DataType::Variant,
        },
        env,
    )
    .ok()?
    .into_value()
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_explanation_uses_the_exact_assertion_window_and_field() {
        let compilation = crate::compiler::compile(
            &[include_str!("../../../tests/fixtures/ui-explanation.rkg")],
            &crate::contract::ContractCatalog::try_m0().unwrap(),
        )
        .unwrap();
        let simulation = &compilation.labs[0].simulations[0];
        for (index, expected_matches) in [(0, 0), (1, 1)] {
            let mut session = crate::lab::Simulator::try_new(
                &compilation.plan,
                crate::plan::GraphVersion(1),
                &simulation.name,
                &simulation.mocks,
                &simulation.scenarios[index],
                None,
            )
            .unwrap();
            session.run_to_completion();
            assert!(!session.report().passed());
            let before = session.journal().records().to_vec();
            let evidence = expectation_json(&compilation.plan, &session);
            assert_eq!(evidence["failed"], true);
            assert_eq!(evidence["observed"], 1);
            assert_eq!(evidence["matched"], expected_matches);
            if index == 0 {
                assert_eq!(
                    evidence["envelopes"][0]["comparisons"][0]["field"],
                    "data.value"
                );
                assert_eq!(
                    evidence["envelopes"][0]["comparisons"][0]["actual"]["content"],
                    7
                );
                assert_eq!(
                    evidence["envelopes"][0]["comparisons"][0]["expected"]["content"],
                    8
                );
            }
            assert_eq!(session.journal().records(), before);
        }
    }

    #[test]
    fn classified_predicates_and_assertion_values_are_not_explanation_channels() {
        let source = r#"
type Protected = struct { value: int @sensitive(level = restricted) }
graph hidden : App {
  input in: Protected
  output out: Protected
  operator filter {
    input in: Protected
    output out: Protected
    on in match {
      data.value > 0 => out = .
      _ => nothing
    }
  }
  in -> filter -> out
}
lab hidden { simulation inspect on app { scenario private {
  app.in = { value = 1234567 }
  expect app.out count == 1 where data.value == 7654321
} } }
"#;
        let compilation = crate::compiler::compile(
            &[source],
            &crate::contract::ContractCatalog::try_m0().unwrap(),
        )
        .unwrap();
        let simulation = &compilation.labs[0].simulations[0];
        let mut session = crate::lab::Simulator::try_new(
            &compilation.plan,
            crate::plan::GraphVersion(1),
            &simulation.name,
            &simulation.mocks,
            &simulation.scenarios[0],
            None,
        )
        .unwrap();
        session.run_to_completion();
        assert!(!session.report().passed());
        let evidence = expectation_json(&compilation.plan, &session);
        assert_eq!(evidence["redacted"], true);
        assert!(evidence.get("envelopes").is_none());
        let explained = explanation::history(&compilation.plan, session.journal().records());
        let decision = explained
            .records
            .iter()
            .filter_map(|record| record.get("decision"))
            .find(|decision| !decision.is_null())
            .unwrap();
        assert!(decision.get("unavailable").is_some());
        assert!(decision.get("steps").is_none());
        for value in [evidence.to_string(), decision.to_string()] {
            assert!(!value.contains("1234567"));
            assert!(!value.contains("7654321"));
        }
    }
}
