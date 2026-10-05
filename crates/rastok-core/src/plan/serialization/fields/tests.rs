use super::*;
use crate::node;
use crate::plan::{DispatchErrorKind, Emission, FieldWrite, RouteErrorKind};
use crate::schema_annotation::{AnnotationSet, AnnotationTable, SchemaPath};
use crate::value::Value;
use alloc::collections::BTreeMap;

fn boolean(value: bool) -> expr::Tree {
    expr::Tree {
        root: expr::Expr::Literal(Value::Bool(value)),
        ty: DataType::Bool,
    }
}

fn empty() -> Dispatch {
    Dispatch::Linear(Vec::new())
}

fn open(predicate: expr::Tree, branch: Dispatch, fallback: Dispatch) -> Dispatch {
    Dispatch::OpenMatch {
        prefix: Vec::new(),
        arms: vec![OpenArm {
            predicate,
            dispatch: Box::new(branch),
        }],
        fallback: Box::new(fallback),
    }
}

fn output(name: &str) -> Dispatch {
    Dispatch::Linear(vec![DispatchStep::Emit {
        port: node::Port::new(name).unwrap(),
        value: Emission::Identity,
    }])
}

fn bool_match(signature: &expr::Signature) -> Dispatch {
    Dispatch::ClosedMatch {
        prefix: Vec::new(),
        scrutinee: expr::compile("data", signature).unwrap(),
        arms: vec![
            ClosedArm {
                cases: vec!["false".into()],
                dispatch: Box::new(output("no")),
            },
            ClosedArm {
                cases: vec!["true".into()],
                dispatch: Box::new(output("yes")),
            },
        ],
    }
}

fn encoded_raw(dispatch: &Dispatch) -> Vec<u8> {
    canonical_cbor::encode(&dispatch_tree(dispatch))
}

#[test]
fn decision_tags_and_nested_shape_are_independently_pinned() {
    let signature = expr::Signature::new();
    // [Open, prefix=[], [[bool(true), [Linear, [Stop]]]], [Linear, []]]
    let expected = Cbor::Array(vec![
        Cbor::Uint(3),
        Cbor::Array(vec![]),
        Cbor::Array(vec![Cbor::Array(vec![
            Cbor::Array(vec![
                Cbor::Uint(2),
                Cbor::Array(vec![
                    Cbor::Uint(1),
                    Cbor::Array(vec![Cbor::Uint(1), Cbor::Bool(true)]),
                ]),
            ]),
            Cbor::Array(vec![
                Cbor::Uint(1),
                Cbor::Array(vec![Cbor::Array(vec![Cbor::Uint(4)])]),
            ]),
        ])]),
        Cbor::Array(vec![Cbor::Uint(1), Cbor::Array(vec![])]),
    ]);
    let dispatch = open(
        boolean(true),
        Dispatch::Linear(vec![DispatchStep::Stop]),
        empty(),
    );
    let bytes = encode_dispatch(&dispatch, &signature).unwrap();
    assert_eq!(canonical_cbor::decode(&bytes).unwrap().0, expected);
    assert_eq!(decode_dispatch(&bytes, &signature).unwrap(), dispatch);
}

#[test]
fn every_statement_builder_and_annotation_survives_transport() {
    let signature = expr::Signature::new()
        .with("data", DataType::INT)
        .with("count", DataType::INT);
    let expression = expr::compile("data + 1", &signature).unwrap();
    let annotations = AnnotationTable::new(vec![(
        SchemaPath::root(),
        AnnotationSet::new(vec![crate::schema_annotation::tests::measure(
            "tokens", "token",
        )])
        .unwrap(),
    )])
    .unwrap();
    let mut statements = vec![
        DispatchStep::Let {
            name: "tokens".into(),
            value: expression.clone(),
            annotations,
        },
        DispatchStep::SetVar {
            name: "count".into(),
            value: expression.clone(),
        },
        DispatchStep::Stop,
    ];
    let builders = vec![
        Emission::Identity,
        Emission::Nothing,
        Emission::Data(expression.clone()),
        Emission::Patch(vec![FieldWrite {
            target: crate::envelope::WriteTarget::Data(vec![]),
            value: expression.clone(),
        }]),
        Emission::FreshData(vec![FieldWrite {
            target: crate::envelope::WriteTarget::Data(vec![]),
            value: expression,
        }]),
    ];
    let ports = vec![
        node::Port::new("sms").unwrap(),
        node::Port::new("email").unwrap(),
    ];
    for value in builders {
        statements.push(DispatchStep::Emit {
            port: node::Port::conventional_out(),
            value: value.clone(),
        });
        statements.push(DispatchStep::MultiEmit {
            ports: ports.clone(),
            values: MultiEmitValues::Broadcast(value.clone()),
        });
        statements.push(DispatchStep::MultiEmit {
            ports: ports.clone(),
            values: MultiEmitValues::Tuple(vec![value.clone(), value.clone()]),
        });
        statements.push(DispatchStep::LoopCommand(LoopCommand::Next(value)));
    }
    statements.extend([
        DispatchStep::LoopCommand(LoopCommand::Again),
        DispatchStep::LoopCommand(LoopCommand::Done),
    ]);
    let dispatch = Dispatch::Linear(statements);
    let bytes = encode_dispatch(&dispatch, &signature).unwrap();
    assert_eq!(decode_dispatch(&bytes, &signature).unwrap(), dispatch);
    // A field proof is not a Loop control proof: this deliberately contradictory
    // list of commands cannot be mistaken for an executable plan.
    assert!(
        super::super::step_from(&step_tree(&DispatchStep::LoopCommand(LoopCommand::Again)))
            .is_err()
    );
}

#[test]
fn decoded_nested_dispatch_selects_the_same_path_and_locals() {
    let signature = expr::Signature::new().with("data", DataType::Bool);
    let local_signature = signature.clone().with("selected", DataType::Bool);
    let mut dispatch = open(
        expr::compile("selected", &local_signature).unwrap(),
        bool_match(&signature),
        output("fallback"),
    );
    let Dispatch::OpenMatch { prefix, .. } = &mut dispatch else {
        unreachable!()
    };
    prefix.push(DispatchStep::Let {
        name: "selected".into(),
        value: expr::compile("data", &signature).unwrap(),
        annotations: AnnotationTable::empty(),
    });
    let bytes = encode_dispatch(&dispatch, &signature).unwrap();
    let decoded = decode_dispatch(&bytes, &signature).unwrap();
    for value in [false, true] {
        let bindings = BTreeMap::from([("data".into(), expr::EvalValue::Bool(value))]);
        let before = crate::runtime::select_dispatch(&dispatch, &bindings).unwrap();
        let after = crate::runtime::select_dispatch(&decoded, &bindings).unwrap();
        assert_eq!(before.statements, after.statements);
        assert_eq!(before.locals, after.locals);
    }
}

#[test]
fn hostile_closed_partitions_and_types_are_rederived_not_normalized() {
    let signature = expr::Signature::new().with("data", DataType::Bool);
    for mutation in 0..5 {
        let mut dispatch = bool_match(&signature);
        let Dispatch::ClosedMatch {
            scrutinee, arms, ..
        } = &mut dispatch
        else {
            unreachable!()
        };
        match mutation {
            0 => {
                arms.pop();
            }
            1 => arms.swap(0, 1),
            2 => arms[1].cases = arms[0].cases.clone(),
            3 => arms[0].cases = vec!["unknown".into()],
            4 => scrutinee.ty = DataType::INT,
            _ => unreachable!(),
        }
        let bytes = encoded_raw(&dispatch);
        assert!(
            matches!(
                decode_dispatch(&bytes, &signature),
                Err(FieldError::Dispatch(_))
            ),
            "mutation {mutation}"
        );
        assert!(encode_dispatch(&dispatch, &signature).is_err());
    }
}

#[test]
fn malformed_decisions_commands_and_tuple_arities_refuse() {
    let signature = expr::Signature::new();
    let leaf = Cbor::Array(vec![Cbor::Uint(1), Cbor::Array(vec![])]);
    for invalid in [
        Cbor::Array(vec![Cbor::Uint(4)]),
        Cbor::Array(vec![Cbor::Uint(1), Cbor::Array(vec![]), Cbor::Null]),
        Cbor::Array(vec![
            Cbor::Uint(3),
            Cbor::Array(vec![]),
            Cbor::Array(vec![]),
        ]),
        Cbor::Array(vec![
            Cbor::Uint(3),
            Cbor::Array(vec![]),
            Cbor::Array(vec![]),
            Cbor::Null,
        ]),
        Cbor::Array(vec![
            Cbor::Uint(1),
            Cbor::Array(vec![Cbor::Array(vec![
                Cbor::Uint(6),
                Cbor::Array(vec![Cbor::Uint(2), Cbor::Null]),
            ])]),
        ]),
        Cbor::Array(vec![
            Cbor::Uint(1),
            Cbor::Array(vec![Cbor::Array(vec![
                Cbor::Uint(6),
                Cbor::Array(vec![Cbor::Uint(99)]),
            ])]),
        ]),
    ] {
        assert!(matches!(
            decode_dispatch(&canonical_cbor::encode(&invalid), &signature),
            Err(FieldError::Decode(_))
        ));
    }
    assert_eq!(
        decode_dispatch(&canonical_cbor::encode(&leaf), &signature).unwrap(),
        empty()
    );
    let mismatched = Dispatch::Linear(vec![DispatchStep::MultiEmit {
        ports: vec![node::Port::new("a").unwrap(), node::Port::new("b").unwrap()],
        values: MultiEmitValues::Tuple(vec![Emission::Nothing]),
    }]);
    assert!(matches!(
        decode_dispatch(&encoded_raw(&mismatched), &signature),
        Err(FieldError::Dispatch(DispatchValidationError {
            kind: DispatchErrorKind::MultiEmission,
            ..
        }))
    ));
}

#[test]
fn expression_context_and_pure_prefixes_cannot_be_forged() {
    let known = expr::Signature::new().with("allowed", DataType::Bool);
    let mut dispatch = open(expr::compile("allowed", &known).unwrap(), empty(), empty());
    let bytes = encode_dispatch(&dispatch, &known).unwrap();
    assert!(matches!(
        decode_dispatch(&bytes, &expr::Signature::new()),
        Err(FieldError::Dispatch(_))
    ));
    let Dispatch::OpenMatch { prefix, .. } = &mut dispatch else {
        unreachable!()
    };
    prefix.push(DispatchStep::Stop);
    assert!(matches!(
        decode_dispatch(&encoded_raw(&dispatch), &known),
        Err(FieldError::Dispatch(DispatchValidationError {
            kind: DispatchErrorKind::ImpurePrefix,
            ..
        }))
    ));
}

#[test]
fn authored_open_arm_and_multi_emission_order_survives() {
    let signature = expr::Signature::new().with("data", DataType::Bool);
    let mut dispatch = open(boolean(true), output("first"), empty());
    let Dispatch::OpenMatch { arms, .. } = &mut dispatch else {
        unreachable!()
    };
    arms.push(OpenArm {
        predicate: boolean(true),
        dispatch: Box::new(output("second")),
    });
    let first = encode_dispatch(&dispatch, &signature).unwrap();
    let Dispatch::OpenMatch { arms, .. } = &mut dispatch else {
        unreachable!()
    };
    arms.swap(0, 1);
    let second = encode_dispatch(&dispatch, &signature).unwrap();
    assert_ne!(first, second);
    assert_eq!(decode_dispatch(&second, &signature).unwrap(), dispatch);

    let mut dispatch = Dispatch::Linear(vec![DispatchStep::MultiEmit {
        ports: vec![node::Port::new("b").unwrap(), node::Port::new("a").unwrap()],
        values: MultiEmitValues::Broadcast(Emission::Identity),
    }]);
    let first = encode_dispatch(&dispatch, &signature).unwrap();
    let Dispatch::Linear(steps) = &mut dispatch else {
        unreachable!()
    };
    let DispatchStep::MultiEmit { ports, .. } = &mut steps[0] else {
        unreachable!()
    };
    ports.swap(0, 1);
    assert_ne!(first, encode_dispatch(&dispatch, &signature).unwrap());
    assert_eq!(
        decode_dispatch(&first, &signature)
            .unwrap()
            .as_linear()
            .unwrap()[0]
            .emissions()
            .next()
            .unwrap()
            .0
            .name(),
        "b"
    );
}

#[test]
fn nesting_is_bounded_before_typed_recursion_and_at_combined_encoding() {
    let signature = expr::Signature::new();
    let mut dispatch = empty();
    for _ in 0..MAX_DISPATCH_DEPTH {
        dispatch = open(boolean(true), dispatch, empty());
    }
    let bytes = encode_dispatch(&dispatch, &signature).unwrap();
    assert_eq!(decode_dispatch(&bytes, &signature).unwrap(), dispatch);
    dispatch = open(boolean(true), dispatch, empty());
    assert!(matches!(
        encode_dispatch(&dispatch, &signature),
        Err(FieldError::Dispatch(DispatchValidationError {
            kind: DispatchErrorKind::Depth,
            ..
        }))
    ));
    assert!(dispatch_from(&dispatch_tree(&dispatch), 0).is_err());

    let mut tree = Cbor::Null;
    for _ in 1..canonical_cbor::MAX_NESTING {
        tree = Cbor::Array(vec![tree]);
    }
    assert!(encode_field(&tree).is_ok());
    tree = Cbor::Array(vec![tree]);
    assert_eq!(encode_field(&tree), Err(FieldError::Nesting));
    assert!(matches!(
        decode_dispatch(&canonical_cbor::encode(&tree), &signature),
        Err(FieldError::Decode(DecodeError::Read(_)))
    ));
}

#[test]
fn trailing_bytes_and_nonminimal_cbor_never_alias_a_field() {
    let signature = expr::Signature::new();
    let bytes = encode_dispatch(&empty(), &signature).unwrap();
    assert_eq!(bytes, [0x82, 0x01, 0x80]);
    let mut trailing = bytes.clone();
    trailing.push(0xf6);
    assert!(decode_dispatch(&trailing, &signature).is_err());
    assert!(decode_dispatch(&[0x82, 0x18, 0x01, 0x80], &signature).is_err());
    for prefix in 0..bytes.len() {
        assert!(decode_dispatch(&bytes[..prefix], &signature).is_err());
    }
}

#[test]
fn individually_shallow_values_can_exceed_the_combined_decision_budget() {
    let signature = expr::Signature::new();
    let mut ty = DataType::Bool;
    let mut value = Value::Bool(true);
    for _ in 0..20 {
        ty = DataType::array(ty);
        value = Value::Array(vec![value]);
    }
    let mut dispatch = Dispatch::Linear(vec![DispatchStep::Emit {
        port: node::Port::conventional_out(),
        value: Emission::Data(expr::Tree {
            ty,
            root: expr::Expr::Literal(value),
        }),
    }]);
    assert!(encode_dispatch(&dispatch, &signature).is_ok());
    for _ in 0..MAX_DISPATCH_DEPTH {
        dispatch = open(boolean(true), dispatch, empty());
    }
    assert_eq!(validate_dispatch(&dispatch, &signature), Ok(()));
    assert_eq!(
        encode_dispatch(&dispatch, &signature),
        Err(FieldError::Nesting)
    );
}

#[test]
fn redundant_annotation_encoding_is_not_silently_canonicalized() {
    let signature = expr::Signature::new();
    let redundant = Cbor::Array(vec![
        Cbor::Uint(1),
        Cbor::Array(vec![Cbor::Array(vec![
            Cbor::Uint(1),
            Cbor::Text("chosen".into()),
            tree_tree(&boolean(true)),
            Cbor::Array(vec![]),
        ])]),
    ]);
    assert!(matches!(
        decode_dispatch(&canonical_cbor::encode(&redundant), &signature),
        Err(FieldError::Decode(DecodeError::NonCanonical(_))),
    ));
}

fn endpoint(owner: EndpointOwner, port: &str) -> Endpoint {
    Endpoint {
        owner,
        port: node::Port::new(port).unwrap(),
    }
}
fn node_owner(name: &str) -> EndpointOwner {
    EndpointOwner::Node(node::Path::new([name]).unwrap())
}
fn source() -> Endpoint {
    endpoint(node_owner("pay"), "exit")
}
fn sink(name: &str) -> RouteDestination {
    RouteDestination::Port(endpoint(node_owner(name), "in"))
}
fn command(kind: LoopCommandKind) -> RouteDestination {
    RouteDestination::LoopCommand(LoopCommandTarget {
        owner: node_owner("pay"),
        kind,
    })
}

fn sort_rules(rules: &mut [RouteRule]) {
    rules.sort_by_key(|rule| canonical_cbor::encode(&route_tree(rule)));
}
fn raw_routes(rules: &[RouteRule]) -> Vec<u8> {
    canonical_cbor::encode(&routing_tree(rules))
}

#[test]
fn routing_roundtrip_keeps_graph_boundaries_and_commands_distinct() {
    let mut rules = vec![
        RouteRule {
            source: source(),
            selection: RouteSelection::Cases(vec![
                RouteArm {
                    cases: vec!["false".into()],
                    destinations: vec![command(LoopCommandKind::Again)],
                },
                RouteArm {
                    cases: vec!["true".into()],
                    destinations: vec![command(LoopCommandKind::Done)],
                },
            ]),
        },
        RouteRule {
            source: source(),
            selection: RouteSelection::All(vec![sink("audit")]),
        },
    ];
    rules.push(RouteRule {
        source: endpoint(EndpointOwner::Graph(node::OwnerPath::root()), "in"),
        selection: RouteSelection::All(vec![sink("pay")]),
    });
    rules.push(RouteRule {
        source: endpoint(
            EndpointOwner::Graph(node::OwnerPath::new(["inner"]).unwrap()),
            "out",
        ),
        selection: RouteSelection::All(vec![RouteDestination::Port(endpoint(
            EndpointOwner::Graph(node::OwnerPath::root()),
            "out",
        ))]),
    });
    sort_rules(&mut rules);
    let bytes = encode_routing(&rules, |_| Some(DataType::Bool)).unwrap();
    assert_eq!(
        decode_routing(&bytes, |_| Some(DataType::Bool)).unwrap(),
        rules
    );
    assert!(matches!(
        decode_routing(&bytes, |_| Some(DataType::INT)),
        Err(FieldError::Routing(_))
    ));
    assert!(matches!(
        decode_routing(&bytes, |_| None),
        Err(FieldError::Routing(_))
    ));
}

#[test]
fn manager_commands_and_explicit_discard_have_separate_wire_tags() {
    for (kind, tag) in [
        (LoopCommandKind::Next, 1),
        (LoopCommandKind::Again, 2),
        (LoopCommandKind::Done, 3),
    ] {
        let destination = command(kind);
        let tree = destinations_tree(core::slice::from_ref(&destination));
        assert_eq!(
            tree,
            Cbor::Array(vec![Cbor::Array(vec![
                Cbor::Uint(3),
                Cbor::Array(vec![Cbor::Uint(1), Cbor::Text("pay".into())]),
                Cbor::Uint(tag)
            ])])
        );
        assert_eq!(destinations_from(&tree).unwrap(), vec![destination]);
    }
    let discard = destinations_tree(&[RouteDestination::Discard]);
    assert_eq!(discard, Cbor::Array(vec![Cbor::Array(vec![Cbor::Uint(2)])]));
    assert_eq!(
        destinations_from(&discard).unwrap(),
        vec![RouteDestination::Discard]
    );
}

#[test]
fn routing_refuses_inventory_conflicts_even_when_rows_are_separately_valid() {
    let mut rules = vec![
        RouteRule {
            source: source(),
            selection: RouteSelection::All(vec![sink("audit")]),
        },
        RouteRule {
            source: source(),
            selection: RouteSelection::Cases(vec![
                RouteArm {
                    cases: vec!["false".into()],
                    destinations: vec![RouteDestination::Discard],
                },
                RouteArm {
                    cases: vec!["true".into()],
                    destinations: vec![command(LoopCommandKind::Again)],
                },
            ]),
        },
    ];
    sort_rules(&mut rules);
    assert!(matches!(
        decode_routing(&raw_routes(&rules), |_| Some(DataType::Bool)),
        Err(FieldError::Routing(RouteValidationError {
            kind: RouteErrorKind::Conflict(_),
            ..
        }))
    ));
    assert!(encode_routing(&rules, |_| Some(DataType::Bool)).is_err());
}

#[test]
fn neither_rows_nor_destination_sets_are_repaired_by_the_decoder() {
    let mut rules = vec![
        RouteRule {
            source: source(),
            selection: RouteSelection::All(vec![sink("audit"), sink("sink")]),
        },
        RouteRule {
            source: endpoint(node_owner("other"), "out"),
            selection: RouteSelection::All(vec![RouteDestination::Discard]),
        },
    ];
    sort_rules(&mut rules);
    let bytes = encode_routing(&rules, |_| None).unwrap();
    assert_eq!(decode_routing(&bytes, |_| None).unwrap(), rules);
    rules.reverse();
    assert_eq!(
        decode_routing(&raw_routes(&rules), |_| None),
        Err(FieldError::RouteOrder)
    );
    let bad_set = vec![RouteRule {
        source: source(),
        selection: RouteSelection::All(vec![sink("sink"), sink("audit")]),
    }];
    assert!(matches!(
        decode_routing(&raw_routes(&bad_set), |_| None),
        Err(FieldError::Routing(RouteValidationError {
            kind: RouteErrorKind::DestinationOrder,
            ..
        }))
    ));
}

#[test]
fn malformed_endpoint_owners_and_command_tags_refuse() {
    for owner in [
        Cbor::Array(vec![Cbor::Uint(1), Cbor::Text(String::new())]),
        Cbor::Array(vec![Cbor::Uint(1), Cbor::Text("a..b".into())]),
        Cbor::Array(vec![Cbor::Uint(2), Cbor::Text(".a".into())]),
        Cbor::Array(vec![Cbor::Uint(9), Cbor::Text("pay".into())]),
    ] {
        assert!(owner_from(&owner).is_err());
    }
    for tag in [0, 4, 256, 257, u64::MAX] {
        let tree = Cbor::Array(vec![Cbor::Array(vec![
            Cbor::Uint(3),
            owner_tree(&node_owner("pay")),
            Cbor::Uint(tag),
        ])]);
        assert!(destinations_from(&tree).is_err());
    }
    assert!(
        destinations_from(&Cbor::Array(vec![Cbor::Array(vec![
            Cbor::Uint(2),
            Cbor::Null
        ])]))
        .is_err()
    );
}
