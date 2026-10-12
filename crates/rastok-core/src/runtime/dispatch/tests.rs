use super::*;
use crate::expr::{Expr, Signature, Tree};
use crate::node::Port;
use crate::plan::{ClosedArm, EmitValue, OpenArm};
use crate::schema_annotation::AnnotationTable;
use crate::value::Value;
use alloc::{boxed::Box, vec, vec::Vec};

fn boolean(value: bool) -> Tree {
    Tree {
        root: Expr::Literal(Value::Bool(value)),
        ty: DataType::Bool,
    }
}

fn output(name: &str) -> Dispatch {
    Dispatch::Linear(vec![DispatchStep::Emit {
        port: Port::new(name).unwrap(),
        value: EmitValue::Identity,
    }])
}

fn open(predicate: Tree, accepted: Dispatch, rejected: Dispatch) -> Dispatch {
    Dispatch::OpenMatch {
        prefix: Vec::new(),
        arms: vec![OpenArm {
            predicate,
            dispatch: Box::new(accepted),
        }],
        fallback: Box::new(rejected),
    }
}

#[test]
fn the_first_true_arm_wins_without_evaluating_later_predicates() {
    let mut dispatch = open(boolean(true), output("first"), output("fallback"));
    let Dispatch::OpenMatch { arms, .. } = &mut dispatch else {
        unreachable!()
    };
    arms.push(OpenArm {
        predicate: Tree {
            root: Expr::Path(vec!["unbound".into()]),
            ty: DataType::Bool,
        },
        dispatch: Box::new(output("later")),
    });
    let selected = select_dispatch(&dispatch, &BTreeMap::new()).unwrap();
    assert_eq!(selected.statements, output("first").as_linear().unwrap());
    assert!(selected.locals.is_empty());
}

#[test]
fn a_faulting_or_non_boolean_predicate_never_falls_through() {
    for predicate in [
        Tree {
            root: Expr::Path(vec!["unbound".into()]),
            ty: DataType::Bool,
        },
        Tree {
            root: Expr::Literal(Value::Int(7)),
            ty: DataType::INT,
        },
    ] {
        let dispatch = open(predicate, output("accepted"), output("fallback"));
        assert!(select_dispatch(&dispatch, &BTreeMap::new()).is_err());
    }
}

#[test]
fn prefixes_are_evaluated_in_order_and_retained_only_for_the_selected_path() {
    let outer = Signature::new().with("data", DataType::INT);
    let scope = outer.clone().with("chosen", DataType::Bool);
    let mut dispatch = open(
        expr::compile("chosen", &scope).unwrap(),
        output("accepted"),
        output("fallback"),
    );
    let Dispatch::OpenMatch { prefix, .. } = &mut dispatch else {
        unreachable!()
    };
    prefix.push(DispatchStep::Let {
        name: "chosen".into(),
        value: expr::compile("data > 0", &outer).unwrap(),
        annotations: AnnotationTable::empty(),
    });
    let bindings = BTreeMap::from([("data".into(), EvalValue::Int(1))]);
    let selected = select_dispatch(&dispatch, &bindings).unwrap();
    assert!(matches!(
        selected.locals.get("chosen"),
        Some(EvalValue::Bool(true))
    ));
    assert!(!bindings.contains_key("chosen"));
    assert_eq!(selected.statements, output("accepted").as_linear().unwrap());
}

#[test]
fn closed_selection_uses_tags_without_replacing_the_input_payload() {
    use crate::datatype::{StructType, TaggedCase, TaggedUnionType, TypeField};
    let ty = DataType::TaggedUnion(
        TaggedUnionType::new(vec![
            TaggedCase::new(
                "Ready",
                StructType::new(vec![TypeField::new("count", DataType::INT)]).unwrap(),
            )
            .unwrap(),
            TaggedCase::new("Waiting", StructType::new(vec![]).unwrap()).unwrap(),
        ])
        .unwrap(),
    );
    let dispatch = Dispatch::ClosedMatch {
        prefix: Vec::new(),
        scrutinee: expr::compile("data", &Signature::new().with("data", ty)).unwrap(),
        arms: vec![
            ClosedArm {
                cases: vec!["Ready".into()],
                dispatch: Box::new(output("ready")),
            },
            ClosedArm {
                cases: vec!["Waiting".into()],
                dispatch: Box::new(output("waiting")),
            },
        ],
    };
    let input = EvalValue::TaggedUnion {
        case: "Ready".into(),
        payload: vec![("count".into(), EvalValue::Int(7))],
    };
    let bindings = BTreeMap::from([("data".into(), input)]);
    let selected = select_dispatch(&dispatch, &bindings).unwrap();
    assert_eq!(selected.statements, output("ready").as_linear().unwrap());
    assert!(
        matches!(bindings.get("data"), Some(EvalValue::TaggedUnion { case, payload })
        if case == "Ready" && matches!(payload[0].1, EvalValue::Int(7)))
    );
}

#[test]
fn a_payload_witness_materializes_only_when_a_decision_demands_it() {
    use crate::envelope::Data;
    use crate::runtime::{DataSlot, LazyBindings};
    let data = Data::materialized(Value::Bool(true));
    let slot = DataSlot::new(&data);
    let base = BTreeMap::new();
    let bindings = LazyBindings {
        base: &base,
        data: &slot,
        assets: &[],
    };
    let dispatch = open(
        expr::compile("data", &Signature::new().with("data", DataType::Bool)).unwrap(),
        output("true_branch"),
        output("false_branch"),
    );
    let selected = select_dispatch(&dispatch, &bindings).unwrap();
    assert_eq!(
        selected.statements,
        output("true_branch").as_linear().unwrap()
    );
    assert!(slot.take_fault().is_none());
}

#[test]
fn malformed_prefixes_and_excessive_depth_refuse_without_body_execution() {
    let mut dispatch = output("never");
    for _ in 0..MAX_DISPATCH_DEPTH {
        dispatch = open(boolean(true), dispatch, output("fallback"));
    }
    assert!(select_dispatch(&dispatch, &BTreeMap::new()).is_ok());
    dispatch = open(boolean(true), dispatch, output("fallback"));
    assert!(select_dispatch(&dispatch, &BTreeMap::new()).is_err());

    let mut impure = open(boolean(true), output("never"), output("fallback"));
    let Dispatch::OpenMatch { prefix, .. } = &mut impure else {
        unreachable!()
    };
    prefix.push(DispatchStep::Stop);
    assert!(select_dispatch(&impure, &BTreeMap::new()).is_err());
}

#[test]
fn a_closed_selection_refuses_missing_and_ambiguous_arms() {
    let mut dispatch = Dispatch::ClosedMatch {
        prefix: Vec::new(),
        scrutinee: boolean(true),
        arms: vec![],
    };
    assert!(select_dispatch(&dispatch, &BTreeMap::new()).is_err());
    let Dispatch::ClosedMatch { arms, .. } = &mut dispatch else {
        unreachable!()
    };
    for _ in 0..2 {
        arms.push(ClosedArm {
            cases: vec!["true".into()],
            dispatch: Box::new(output("duplicate")),
        });
    }
    assert!(select_dispatch(&dispatch, &BTreeMap::new()).is_err());
}
