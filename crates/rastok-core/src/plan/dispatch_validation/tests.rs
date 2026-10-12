use super::*;
use crate::datatype::TaggedUnionType;
use crate::datatype::{StructType, TaggedCase, TypeField};
use crate::node::Port;
use crate::plan::{ClosedArm, FieldWrite, OpenArm};
use crate::schema_annotation::{AnnotationSet, AnnotationTable, SchemaPath};
use crate::value::Value;
use alloc::{boxed::Box, vec, vec::Vec};

fn empty() -> Dispatch {
    Dispatch::Linear(Vec::new())
}

fn tree(source: &str, signature: &Signature) -> Tree {
    expr::compile(source, signature).unwrap()
}

fn literal(value: Value, ty: DataType) -> Tree {
    Tree {
        root: Expr::Literal(value),
        ty,
    }
}

fn local(name: &str, value: Tree) -> DispatchStep {
    DispatchStep::Let {
        name: name.into(),
        value,
        annotations: AnnotationTable::empty(),
    }
}

fn emit(value: Tree) -> DispatchStep {
    DispatchStep::Emit {
        port: Port::conventional_out(),
        value: EmitValue::Data(value),
    }
}

fn open(predicate: Tree, branch: Dispatch, fallback: Dispatch) -> Dispatch {
    Dispatch::OpenMatch {
        prefix: Vec::new(),
        arms: vec![OpenArm {
            predicate,
            dispatch: Box::new(branch),
        }],
        fallback: Box::new(fallback),
    }
}

fn closed(scrutinee: Tree, cases: &[&str]) -> Dispatch {
    Dispatch::ClosedMatch {
        prefix: Vec::new(),
        scrutinee,
        arms: cases
            .iter()
            .map(|case| ClosedArm {
                cases: vec![(*case).into()],
                dispatch: Box::new(empty()),
            })
            .collect(),
    }
}

fn outcome() -> DataType {
    DataType::TaggedUnion(
        TaggedUnionType::new(vec![
            TaggedCase::new(
                "Failed",
                StructType::new(vec![TypeField::new("reason", DataType::Text)]).unwrap(),
            )
            .unwrap(),
            TaggedCase::new(
                "Ready",
                StructType::new(vec![TypeField::new("count", DataType::INT)]).unwrap(),
            )
            .unwrap(),
        ])
        .unwrap(),
    )
}

fn assert_refusal(dispatch: &Dispatch, signature: &Signature, path: &str, kind: DispatchErrorKind) {
    assert_eq!(
        validate_dispatch(dispatch, signature),
        Err(DispatchValidationError::at(path, kind))
    );
}

#[test]
fn local_prefixes_feed_nested_decisions_without_mutating_the_outer_environment() {
    let outer = Signature::new().with("data", DataType::INT);
    let scope = outer.clone().with("positive", DataType::Bool);
    let mut dispatch = closed(tree("positive", &scope), &["false", "true"]);
    let Dispatch::ClosedMatch { prefix, arms, .. } = &mut dispatch else {
        unreachable!()
    };
    prefix.push(local("positive", tree("data > 0", &outer)));
    for arm in arms {
        // Sibling locals with the same name do not shadow each other.
        let branch_scope = scope.clone().with("count", DataType::INT);
        *arm.dispatch = Dispatch::OpenMatch {
            prefix: vec![local("count", tree("data + 1", &scope))],
            arms: vec![OpenArm {
                predicate: tree("count > 0", &branch_scope),
                dispatch: Box::new(Dispatch::Linear(vec![emit(tree("count", &branch_scope))])),
            }],
            fallback: Box::new(empty()),
        };
    }
    let original = dispatch.clone();
    assert_eq!(validate_dispatch(&dispatch, &outer), Ok(()));
    assert_eq!(dispatch, original);
    assert_eq!(outer.get("positive"), None);
    assert_eq!(outer.get("count"), None);
}

#[test]
fn closed_partition_and_scrutinee_type_are_both_rederived() {
    let signature = Signature::new().with("data", DataType::Bool);
    let mut dispatch = closed(tree("data", &signature), &["false"]);
    assert_refusal(
        &dispatch,
        &signature,
        "dispatch",
        DispatchErrorKind::CasePartition(CasePartitionError::MissingCase {
            case: "true".into(),
        }),
    );
    let Dispatch::ClosedMatch { scrutinee, .. } = &mut dispatch else {
        unreachable!()
    };
    scrutinee.ty = outcome();
    assert_refusal(
        &dispatch,
        &signature,
        "dispatch.scrutinee",
        DispatchErrorKind::Expression,
    );
}

#[test]
fn closed_arms_preserve_the_selected_union_instead_of_unwrapping_data() {
    let ty = outcome();
    let signature = Signature::new().with("data", ty.clone());
    let mut dispatch = closed(tree("data", &signature), &["Failed", "Ready"]);
    let DataType::TaggedUnion(union) = &ty else {
        unreachable!()
    };
    let Dispatch::ClosedMatch { arms, .. } = &mut dispatch else {
        unreachable!()
    };
    for (arm, case) in arms.iter_mut().zip(union.cases()) {
        let selected = DataType::TaggedUnion(TaggedUnionType::new(vec![case.clone()]).unwrap());
        let branch = Signature::new().with("data", selected);
        *arm.dispatch = Dispatch::Linear(vec![emit(tree("data", &branch))]);
    }
    assert_eq!(validate_dispatch(&dispatch, &signature), Ok(()));
    let Dispatch::ClosedMatch { arms, .. } = &mut dispatch else {
        unreachable!()
    };
    *arms[0].dispatch = Dispatch::Linear(vec![emit(Tree {
        root: Expr::Path(vec!["data".into()]),
        ty: DataType::Struct(union.cases()[0].payload().clone()),
    })]);
    assert_refusal(
        &dispatch,
        &signature,
        "dispatch.arms[0].linear[0].value",
        DispatchErrorKind::Expression,
    );
}

#[test]
fn true_facts_narrow_only_the_selected_open_arm() {
    let signature = Signature::new().with("data", outcome());
    let predicate = tree("data is Ready", &signature);
    let narrowed = signature.narrowed_by(&predicate.root);
    let branch = Dispatch::Linear(vec![emit(tree("data.count", &narrowed))]);
    let mut dispatch = open(predicate, branch.clone(), empty());
    assert_eq!(validate_dispatch(&dispatch, &signature), Ok(()));
    let Dispatch::OpenMatch { fallback, .. } = &mut dispatch else {
        unreachable!()
    };
    **fallback = branch;
    assert_refusal(
        &dispatch,
        &signature,
        "dispatch.fallback.linear[0].value",
        DispatchErrorKind::Expression,
    );
}

#[test]
fn neither_a_local_nor_a_positive_fact_can_escape_into_the_next_predicate() {
    let signature = Signature::new().with("data", outcome());
    let first = tree("data is Ready", &signature);
    let narrowed = signature.narrowed_by(&first.root);
    let mut dispatch = open(
        first,
        Dispatch::Linear(vec![local("private", tree("true", &signature))]),
        empty(),
    );
    let Dispatch::OpenMatch { arms, .. } = &mut dispatch else {
        unreachable!()
    };
    arms.push(OpenArm {
        predicate: tree("data.count > 0", &narrowed),
        dispatch: Box::new(empty()),
    });
    assert_refusal(
        &dispatch,
        &signature,
        "dispatch.arms[1].predicate",
        DispatchErrorKind::Expression,
    );
    let Dispatch::OpenMatch { arms, .. } = &mut dispatch else {
        unreachable!()
    };
    arms[1].predicate = tree(
        "private",
        &signature.clone().with("private", DataType::Bool),
    );
    assert_refusal(
        &dispatch,
        &signature,
        "dispatch.arms[1].predicate",
        DispatchErrorKind::Expression,
    );
}

#[test]
fn a_forged_case_payload_cannot_supply_branch_facts() {
    let signature = Signature::new().with("data", outcome());
    let mut predicate = tree("data is Ready", &signature);
    let Expr::Is { target, .. } = &mut predicate.root else {
        unreachable!()
    };
    let expr::NarrowTarget::Case { payload, .. } = target.as_mut() else {
        unreachable!()
    };
    **payload =
        DataType::Struct(StructType::new(vec![TypeField::new("count", DataType::Text)]).unwrap());
    let forged_scope = signature.narrowed_by(&predicate.root);
    let dispatch = open(
        predicate,
        Dispatch::Linear(vec![emit(tree("data.count", &forged_scope))]),
        empty(),
    );
    assert_refusal(
        &dispatch,
        &signature,
        "dispatch.arms[0].predicate",
        DispatchErrorKind::Expression,
    );
}

#[test]
fn a_decision_cannot_relabel_a_non_boolean_literal_as_a_dynamic_guard() {
    let signature = Signature::new();
    for ty in [DataType::Bool, DataType::Variant] {
        let dispatch = open(literal(Value::Int(1), ty), empty(), empty());
        assert_refusal(
            &dispatch,
            &signature,
            "dispatch.arms[0].predicate",
            DispatchErrorKind::Expression,
        );
    }
    let dispatch = open(tree("1", &signature), empty(), empty());
    assert_refusal(
        &dispatch,
        &signature,
        "dispatch.arms[0].predicate",
        DispatchErrorKind::Predicate,
    );
}

#[test]
fn only_the_real_inference_binding_may_remain_dynamic() {
    let signature = Signature::new().with("data", DataType::Variant);
    let dispatch = open(tree("data", &signature), empty(), empty());
    assert_eq!(validate_dispatch(&dispatch, &signature), Ok(()));
    // The outer type-resolution pass cannot silently retain this old claim.
    let resolved = Signature::new().with("data", DataType::INT);
    assert_refusal(
        &dispatch,
        &resolved,
        "dispatch.arms[0].predicate",
        DispatchErrorKind::Expression,
    );
}

#[test]
fn every_effectful_prefix_refuses_before_a_branch_can_be_selected() {
    let signature = Signature::new();
    for step in [
        emit(tree("true", &signature)),
        DispatchStep::SetVar {
            name: "seen".into(),
            value: tree("1", &signature),
        },
        DispatchStep::Stop,
        DispatchStep::LoopCommand(LoopCommand::Done),
        DispatchStep::MultiEmit {
            ports: vec![Port::conventional_out(), Port::new("other").unwrap()],
            values: MultiEmitValues::Broadcast(EmitValue::Identity),
        },
    ] {
        let mut dispatch = closed(tree("true", &signature), &["false", "true"]);
        let Dispatch::ClosedMatch { prefix, .. } = &mut dispatch else {
            unreachable!()
        };
        prefix.push(step);
        assert_refusal(
            &dispatch,
            &signature,
            "dispatch.prefix[0]",
            DispatchErrorKind::ImpurePrefix,
        );
    }
}

#[test]
fn local_scope_rejects_shadowing_forward_reads_and_reserved_names() {
    let signature = Signature::new().with("existing", DataType::Bool);
    for name in ["existing", "data", "meta", "meta.app.x", ""] {
        assert_refusal(
            &Dispatch::Linear(vec![local(name, tree("true", &signature))]),
            &signature,
            "dispatch.linear[0]",
            DispatchErrorKind::Binding,
        );
    }
    let value = tree("true", &signature);
    assert_refusal(
        &Dispatch::Linear(vec![local("x", value.clone()), local("x", value)]),
        &signature,
        "dispatch.linear[1]",
        DispatchErrorKind::Binding,
    );
    let later = tree("later", &signature.clone().with("later", DataType::Bool));
    let dispatch = Dispatch::Linear(vec![
        local("early", later),
        local("later", tree("true", &signature)),
    ]);
    assert_refusal(
        &dispatch,
        &signature,
        "dispatch.linear[0].value",
        DispatchErrorKind::Expression,
    );
}

#[test]
fn the_depth_budget_counts_decisions_including_empty_fallback_only_matches() {
    let mut dispatch = empty();
    let signature = Signature::new();
    for _ in 0..MAX_DISPATCH_DEPTH {
        dispatch = Dispatch::OpenMatch {
            prefix: Vec::new(),
            arms: Vec::new(),
            fallback: Box::new(dispatch),
        };
    }
    assert_eq!(validate_dispatch(&dispatch, &signature), Ok(()));
    dispatch = Dispatch::OpenMatch {
        prefix: Vec::new(),
        arms: Vec::new(),
        fallback: Box::new(dispatch),
    };
    let error = validate_dispatch(&dispatch, &signature).unwrap_err();
    assert_eq!(error.kind, DispatchErrorKind::Depth);
    assert_eq!(
        error.path,
        format!("dispatch{}", ".fallback".repeat(MAX_DISPATCH_DEPTH))
    );
}

#[test]
fn hostile_expressions_are_refused_before_recursive_type_checking() {
    let signature = Signature::new();
    let mut deep = Expr::Literal(Value::Bool(true));
    for _ in 0..crate::canonical_cbor::MAX_NESTING {
        deep = Expr::Unary {
            op: expr::UnaryOp::Not,
            expr: Box::new(deep),
        };
    }
    for root in [
        Expr::Path(Vec::new()),
        Expr::Path(vec!["absent".into()]),
        deep,
    ] {
        let dispatch = open(
            Tree {
                root,
                ty: DataType::Bool,
            },
            empty(),
            empty(),
        );
        assert_refusal(
            &dispatch,
            &signature,
            "dispatch.arms[0].predicate",
            DispatchErrorKind::Expression,
        );
    }
}

#[test]
fn contextual_constructors_are_checked_field_by_field_not_by_their_claimed_root() {
    let signature = Signature::new().with("count", DataType::INT);
    let ty = DataType::Struct(
        StructType::new(vec![
            TypeField::new("count", DataType::LONG),
            TypeField::new("mime", DataType::MediaType),
        ])
        .unwrap(),
    );
    let mut value = Tree {
        ty,
        root: Expr::Struct(vec![
            ("count".into(), Expr::Path(vec!["count".into()])),
            (
                "mime".into(),
                Expr::Literal(Value::Text("application/json".into())),
            ),
        ]),
    };
    assert_eq!(
        validate_dispatch(
            &Dispatch::Linear(vec![local("response", value.clone())]),
            &signature
        ),
        Ok(())
    );
    let Expr::Struct(fields) = &mut value.root else {
        unreachable!()
    };
    fields[0].1 = Expr::Literal(Value::Text("not an integer".into()));
    assert_refusal(
        &Dispatch::Linear(vec![local("response", value)]),
        &signature,
        "dispatch.linear[0].value",
        DispatchErrorKind::Expression,
    );
}

#[test]
fn local_annotation_compatibility_is_not_a_source_only_claim() {
    let signature = Signature::new();
    let annotations = AnnotationTable::new(vec![(
        SchemaPath::root(),
        AnnotationSet::new(vec![crate::schema_annotation::tests::measure(
            "tokens", "token",
        )])
        .unwrap(),
    )])
    .unwrap();
    let valid = local("tokens", tree("1", &signature));
    let DispatchStep::Let { name, .. } = valid else {
        unreachable!()
    };
    let dispatch = |value| {
        Dispatch::Linear(vec![DispatchStep::Let {
            name: name.clone(),
            value,
            annotations: annotations.clone(),
        }])
    };
    assert_eq!(
        validate_dispatch(&dispatch(tree("1", &signature)), &signature),
        Ok(())
    );
    assert_refusal(
        &dispatch(tree("\"secret\"", &signature)),
        &signature,
        "dispatch.linear[0]",
        DispatchErrorKind::Annotations,
    );
}

#[test]
fn heterogeneous_collection_constructors_rederive_each_member() {
    let signature = Signature::new();
    let member = DataType::structural_union(vec![DataType::INT, DataType::Text]).unwrap();
    let mut map = Tree {
        root: Expr::MapLit(vec![
            ("status".into(), Expr::Literal(Value::Text("ok".into()))),
            ("value".into(), Expr::Literal(Value::Int(7))),
        ]),
        ty: DataType::text_map(member.clone()),
    };
    assert_eq!(
        validate_dispatch(&Dispatch::Linear(vec![emit(map.clone())]), &signature),
        Ok(())
    );
    map.ty = DataType::text_map(DataType::INT);
    assert_refusal(
        &Dispatch::Linear(vec![emit(map)]),
        &signature,
        "dispatch.linear[0].value",
        DispatchErrorKind::Expression,
    );

    let mut list = Tree {
        root: Expr::List(vec![
            Expr::Literal(Value::Text("ok".into())),
            Expr::Literal(Value::Int(7)),
        ]),
        ty: DataType::array(member.clone()),
    };
    assert_eq!(
        validate_dispatch(&Dispatch::Linear(vec![emit(list.clone())]), &signature),
        Ok(())
    );
    list.ty = DataType::Array(crate::datatype::ArrayType::fixed(member, 3).unwrap());
    assert_refusal(
        &Dispatch::Linear(vec![emit(list)]),
        &signature,
        "dispatch.linear[0].value",
        DispatchErrorKind::Expression,
    );
}

#[test]
fn multi_emission_keeps_authored_order_and_checks_every_value_and_target() {
    let signature = Signature::new();
    let a = Port::new("a").unwrap();
    let b = Port::new("b").unwrap();
    let one = EmitValue::Data(tree("1", &signature));
    let two = EmitValue::Data(tree("2", &signature));
    let make = |ports, values| Dispatch::Linear(vec![DispatchStep::MultiEmit { ports, values }]);
    let dispatch = make(
        vec![b.clone(), a.clone()],
        MultiEmitValues::Tuple(vec![two.clone(), one.clone()]),
    );
    let original = dispatch.clone();
    assert_eq!(validate_dispatch(&dispatch, &signature), Ok(()));
    assert_eq!(dispatch, original);
    for (ports, values) in [
        (Vec::new(), MultiEmitValues::Tuple(Vec::new())),
        (vec![a.clone()], MultiEmitValues::Broadcast(one.clone())),
        (
            vec![a.clone(), a.clone()],
            MultiEmitValues::Broadcast(one.clone()),
        ),
        (
            vec![a.clone(), b.clone()],
            MultiEmitValues::Tuple(vec![one.clone()]),
        ),
    ] {
        assert_refusal(
            &make(ports, values),
            &signature,
            "dispatch.linear[0]",
            DispatchErrorKind::MultiEmission,
        );
    }
    let bad = EmitValue::Data(literal(Value::Text("wrong".into()), DataType::INT));
    assert_refusal(
        &make(vec![a, b], MultiEmitValues::Tuple(vec![one, bad])),
        &signature,
        "dispatch.linear[0].values[1]",
        DispatchErrorKind::Expression,
    );
}

#[test]
fn every_emission_shape_and_loop_next_have_the_same_expression_proof() {
    let signature = Signature::new();
    let bad = literal(Value::Text("wrong".into()), DataType::INT);
    let write = FieldWrite {
        target: crate::envelope::WriteTarget::Data(Vec::new()),
        value: bad.clone(),
    };
    for (value, suffix) in [
        (EmitValue::Data(bad.clone()), ""),
        (EmitValue::Patch(vec![write.clone()]), ".writes[0]"),
        (EmitValue::FreshData(vec![write]), ".writes[0]"),
    ] {
        for step in [
            DispatchStep::Emit {
                port: Port::conventional_out(),
                value: value.clone(),
            },
            DispatchStep::LoopCommand(LoopCommand::Next(value)),
        ] {
            assert_refusal(
                &Dispatch::Linear(vec![step]),
                &signature,
                &format!("dispatch.linear[0].value{suffix}"),
                DispatchErrorKind::Expression,
            );
        }
    }
    assert_refusal(
        &Dispatch::Linear(vec![DispatchStep::SetVar {
            name: "state".into(),
            value: bad,
        }]),
        &signature,
        "dispatch.linear[0].value",
        DispatchErrorKind::Expression,
    );
}
