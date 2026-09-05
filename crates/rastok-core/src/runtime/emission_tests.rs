use super::*;
use crate::plan::MultiEmitValues;

fn ports() -> Vec<node::Port> {
    vec![node::Port::new("z").unwrap(), node::Port::new("a").unwrap()]
}

fn emitted(number: i64) -> PlanEmission {
    PlanEmission::Data(expr::Tree {
        root: expr::Expr::Literal(Value::Int(number)),
        ty: crate::datatype::DataType::INT,
    })
}

fn build(value: &PlanEmission) -> Result<Option<Built>, Fault> {
    match value {
        PlanEmission::Data(expr::Tree {
            root: expr::Expr::Literal(value),
            ..
        }) => Ok(Some(Built {
            data: BuiltData::Value(value.clone()),
            app: BTreeMap::new(),
            ext: BTreeMap::new(),
        })),
        PlanEmission::Nothing => Ok(None),
        _ => Err(Fault::Unrepresentable(String::from(
            "test builder requires a literal",
        ))),
    }
}

fn capture(
    outputs: &mut Vec<(String, Value)>,
    port: &node::Port,
    built: Built,
) -> Result<(), Fault> {
    let BuiltData::Value(value) = built.data else {
        return Err(Fault::Unrepresentable(String::from(
            "test builder materializes literals",
        )));
    };
    outputs.push((port.name().into(), value));
    Ok(())
}

#[test]
fn broadcast_evaluates_once_and_creates_outputs_in_authored_order() {
    let mut evaluations = 0;
    let mut outputs = Vec::new();
    evaluate_emissions(
        &DispatchStep::MultiEmit {
            ports: ports(),
            values: MultiEmitValues::Broadcast(emitted(7)),
        },
        |value| {
            evaluations += 1;
            build(value)
        },
        |port, built| capture(&mut outputs, port, built),
    )
    .unwrap();
    assert_eq!(evaluations, 1);
    assert_eq!(
        outputs,
        vec![("z".into(), Value::Int(7)), ("a".into(), Value::Int(7))]
    );
}

#[test]
fn tuple_evaluates_each_value_once_in_order_and_keeps_nothing_absent() {
    for values in [
        vec![emitted(2), emitted(1)],
        vec![PlanEmission::Nothing, emitted(1)],
    ] {
        let mut evaluated = Vec::new();
        let mut outputs = Vec::new();
        evaluate_emissions(
            &DispatchStep::MultiEmit {
                ports: ports(),
                values: MultiEmitValues::Tuple(values.clone()),
            },
            |value| {
                evaluated.push(value.clone());
                build(value)
            },
            |port, built| capture(&mut outputs, port, built),
        )
        .unwrap();
        assert_eq!(evaluated, values);
        assert_eq!(outputs.last(), Some(&("a".into(), Value::Int(1))));
        assert_eq!(
            outputs.len(),
            if matches!(values[0], PlanEmission::Nothing) {
                1
            } else {
                2
            }
        );
    }
}

#[test]
fn bad_tuple_arity_refuses_before_any_evaluation_or_staging() {
    let mut evaluations = 0;
    let mut outputs = Vec::new();
    let result = evaluate_emissions(
        &DispatchStep::MultiEmit {
            ports: ports(),
            values: MultiEmitValues::Tuple(vec![emitted(1)]),
        },
        |value| {
            evaluations += 1;
            build(value)
        },
        |port, built| capture(&mut outputs, port, built),
    );
    assert!(matches!(result, Err(Fault::Unrepresentable(_))));
    assert_eq!(evaluations, 0);
    assert!(outputs.is_empty());
}

#[test]
fn a_failed_target_aborts_the_remaining_tuple_evaluations() {
    let mut evaluations = 0;
    let refusal = String::from("destination pin rejected the built value");
    let result = evaluate_emissions(
        &DispatchStep::MultiEmit {
            ports: ports(),
            values: MultiEmitValues::Tuple(vec![emitted(2), emitted(1)]),
        },
        |value| {
            evaluations += 1;
            build(value)
        },
        |_, _| Err(Fault::Unrepresentable(refusal.clone())),
    );
    assert!(matches!(result, Err(Fault::Unrepresentable(message)) if message == refusal));
    assert_eq!(evaluations, 1);
}
