use super::*;
use crate::datatype::{EnumType, StructType, TaggedCase, TaggedUnionType};
use crate::node;
use crate::plan::{EndpointOwner, LoopCommandKind, LoopCommandTarget, OwnerPath, RouteArm};
use alloc::vec;

fn endpoint(name: &str, port: &str) -> Endpoint {
    Endpoint {
        owner: EndpointOwner::Node(node::Path::new([name]).unwrap()),
        port: node::Port::new(port).unwrap(),
    }
}

fn port(name: &str) -> RouteDestination {
    RouteDestination::Port(endpoint(name, "in"))
}

fn command(kind: LoopCommandKind) -> RouteDestination {
    RouteDestination::LoopCommand(LoopCommandTarget {
        owner: EndpointOwner::Graph(OwnerPath::root()),
        kind,
    })
}

fn all(destinations: Vec<RouteDestination>) -> RouteRule {
    RouteRule {
        source: endpoint("source", "out"),
        selection: RouteSelection::All(destinations),
    }
}

fn cases(rows: &[(&[&str], Vec<RouteDestination>)]) -> RouteRule {
    RouteRule {
        source: endpoint("source", "out"),
        selection: RouteSelection::Cases(
            rows.iter()
                .map(|(names, destinations)| RouteArm {
                    cases: names.iter().map(|name| String::from(*name)).collect(),
                    destinations: destinations.clone(),
                })
                .collect(),
        ),
    }
}

fn verify(rules: &[RouteRule], ty: &DataType) -> Result<(), RouteValidationError> {
    validate_routing(rules, |_| Some(ty.clone()))
}

#[test]
fn unconditional_routes_need_no_closed_type_or_case_lookup() {
    for destinations in [
        vec![port("left"), port("right")],
        vec![RouteDestination::Discard],
        vec![command(LoopCommandKind::Next)],
        vec![port("audit"), command(LoopCommandKind::Again)],
    ] {
        assert_eq!(
            validate_routing(&[all(destinations)], |_| panic!("no case-source lookup")),
            Ok(())
        );
    }
    assert_eq!(validate_routing(&[], |_| None), Ok(()));
}

#[test]
fn case_source_types_are_looked_up_by_exact_endpoint() {
    let rules = [
        cases(&[
            (&["false"], vec![port("no")]),
            (&["true"], vec![port("yes")]),
        ]),
        RouteRule {
            source: endpoint("other", "exit"),
            selection: RouteSelection::Cases(vec![RouteArm {
                cases: vec!["Ok".into()],
                destinations: vec![RouteDestination::Discard],
            }]),
        },
    ];
    let types = BTreeMap::from([
        (endpoint("source", "out"), DataType::Bool),
        (
            endpoint("other", "exit"),
            DataType::Enum(EnumType::new(vec!["Ok".into()]).unwrap()),
        ),
    ]);
    assert_eq!(
        validate_routing(&rules, |source| types.get(source).cloned()),
        Ok(())
    );
    let error = validate_routing(&rules, |_| None).unwrap_err();
    assert_eq!(error.path, "routes[0].source");
    assert_eq!(error.kind, RouteErrorKind::UnresolvedCaseSource);
    let error = verify(&rules, &DataType::INT).unwrap_err();
    assert_eq!(
        error.kind,
        RouteErrorKind::CasePartition(CasePartitionError::NotClosed)
    );
}

#[test]
fn selected_cases_are_a_partition_of_the_actual_union() {
    let ty = DataType::TaggedUnion(
        TaggedUnionType::new(vec![
            TaggedCase::new("Failed", StructType::new(vec![]).unwrap()).unwrap(),
            TaggedCase::new("Ready", StructType::new(vec![]).unwrap()).unwrap(),
        ])
        .unwrap(),
    );
    let good = cases(&[
        (&["Failed"], vec![port("failed")]),
        (&["Ready"], vec![port("ready")]),
    ]);
    assert_eq!(verify(&[good], &ty), Ok(()));
    for (rule, expected) in [
        (
            cases(&[(&["Ready"], vec![port("ready")])]),
            CasePartitionError::MissingCase {
                case: "Failed".into(),
            },
        ),
        (
            cases(&[(&["Failed", "Ghost", "Ready"], vec![port("sink")])]),
            CasePartitionError::UnknownCase {
                arm: 0,
                case: "Ghost".into(),
            },
        ),
        (
            cases(&[
                (&["Failed"], vec![port("one")]),
                (&["Failed", "Ready"], vec![port("two")]),
            ]),
            CasePartitionError::Overlap {
                arm: 1,
                case: "Failed".into(),
            },
        ),
        (
            cases(&[
                (&["Ready"], vec![port("ready")]),
                (&["Failed"], vec![port("failed")]),
            ]),
            CasePartitionError::ArmOrder { arm: 1 },
        ),
    ] {
        let before = rule.clone();
        let error = verify(core::slice::from_ref(&rule), &ty).unwrap_err();
        assert_eq!(error.path, "routes[0].selection.arms");
        assert_eq!(error.kind, RouteErrorKind::CasePartition(expected));
        assert_eq!(rule, before, "validation must not repair a case table");
    }
}

#[test]
fn destination_sets_are_nonempty_canonical_and_never_repaired() {
    for (destinations, kind) in [
        (vec![], RouteErrorKind::EmptyDestinations),
        (vec![port("z"), port("a")], RouteErrorKind::DestinationOrder),
        (
            vec![port("a"), port("a")],
            RouteErrorKind::DuplicateDestination(port("a")),
        ),
        (
            vec![RouteDestination::Discard, RouteDestination::Discard],
            RouteErrorKind::DuplicateDestination(RouteDestination::Discard),
        ),
        (
            vec![
                command(LoopCommandKind::Done),
                command(LoopCommandKind::Done),
            ],
            RouteErrorKind::DuplicateDestination(command(LoopCommandKind::Done)),
        ),
        (
            vec![port("a"), RouteDestination::Discard],
            RouteErrorKind::MixedDiscard,
        ),
        (
            vec![RouteDestination::Discard, command(LoopCommandKind::Again)],
            RouteErrorKind::MixedDiscard,
        ),
    ] {
        let rule = all(destinations);
        let before = rule.clone();
        let error = verify(core::slice::from_ref(&rule), &DataType::Variant).unwrap_err();
        assert_eq!(error.path, "routes[0].selection.destinations");
        assert_eq!(error.kind, kind);
        assert_eq!(rule, before);
    }
    let invalid = cases(&[(&["false"], vec![port("a")]), (&["true"], vec![])]);
    let error = verify(&[invalid], &DataType::Bool).unwrap_err();
    assert_eq!(error.path, "routes[0].selection.arms[1].destinations");
    assert_eq!(error.kind, RouteErrorKind::EmptyDestinations);
}

#[test]
fn universal_and_case_routes_are_additive_in_either_order() {
    let universal = all(vec![port("audit")]);
    let conditional = cases(&[
        (
            &["false"],
            vec![port("failed"), command(LoopCommandKind::Done)],
        ),
        (&["true"], vec![command(LoopCommandKind::Again)]),
    ]);
    assert_eq!(
        verify(&[universal.clone(), conditional.clone()], &DataType::Bool),
        Ok(())
    );
    assert_eq!(verify(&[conditional, universal], &DataType::Bool), Ok(()));
}

#[test]
fn discard_is_exclusive_across_declarations_in_either_order() {
    for universal in [
        all(vec![port("audit")]),
        all(vec![command(LoopCommandKind::Done)]),
    ] {
        let conditional = cases(&[
            (&["false"], vec![RouteDestination::Discard]),
            (&["true"], vec![port("ok")]),
        ]);
        for rules in [
            [universal.clone(), conditional.clone()],
            [conditional.clone(), universal.clone()],
        ] {
            let error = verify(&rules, &DataType::Bool).unwrap_err();
            assert_eq!(error.path, "routes[1].selection");
            assert_eq!(
                error.kind,
                RouteErrorKind::Conflict(RouteConflict {
                    cases: Some(vec!["false".into()]),
                    kind: RouteConflictKind::MixedDiscard,
                })
            );
        }
    }
    let error = verify(
        &[
            all(vec![RouteDestination::Discard]),
            all(vec![port("sink")]),
        ],
        &DataType::INT,
    )
    .unwrap_err();
    assert_eq!(
        error.kind,
        RouteErrorKind::Conflict(RouteConflict {
            cases: None,
            kind: RouteConflictKind::MixedDiscard,
        })
    );
}

#[test]
fn duplicate_destinations_are_rejected_only_for_overlapping_values() {
    for destination in [port("sink"), command(LoopCommandKind::Again)] {
        let conditional = cases(&[
            (&["false"], vec![destination.clone()]),
            (&["true"], vec![destination.clone()]),
        ]);
        assert_eq!(
            verify(core::slice::from_ref(&conditional), &DataType::Bool),
            Ok(()),
            "two exclusive arms can select the same physical destination or command"
        );
        for rules in [
            [all(vec![destination.clone()]), conditional.clone()],
            [conditional, all(vec![destination.clone()])],
        ] {
            assert_eq!(
                verify(&rules, &DataType::Bool).unwrap_err().kind,
                RouteErrorKind::Conflict(RouteConflict {
                    cases: Some(vec!["false".into()]),
                    kind: RouteConflictKind::DuplicateDestination(destination.clone()),
                })
            );
        }
    }
    for destination in [port("sink"), RouteDestination::Discard] {
        assert_eq!(
            verify(
                &[
                    all(vec![destination.clone()]),
                    all(vec![destination.clone()])
                ],
                &DataType::INT
            )
            .unwrap_err()
            .kind,
            RouteErrorKind::Conflict(RouteConflict {
                cases: None,
                kind: RouteConflictKind::DuplicateDestination(destination),
            })
        );
    }
}

#[test]
fn a_second_case_match_is_not_merged_even_if_destinations_are_disjoint() {
    let rules = [
        cases(&[(&["false", "true"], vec![port("left")])]),
        cases(&[(&["false", "true"], vec![port("right")])]),
    ];
    assert_eq!(
        verify(&rules, &DataType::Bool).unwrap_err().kind,
        RouteErrorKind::Conflict(RouteConflict {
            cases: None,
            kind: RouteConflictKind::MultipleMatches,
        })
    );
}

#[test]
fn identical_destinations_on_distinct_source_ports_or_owners_do_not_conflict() {
    let mut other_port = all(vec![RouteDestination::Discard]);
    other_port.source.port = node::Port::new("err").unwrap();
    let mut other_node = all(vec![RouteDestination::Discard]);
    other_node.source.owner = EndpointOwner::Node(node::Path::new(["other"]).unwrap());
    let mut graph_port = all(vec![RouteDestination::Discard]);
    graph_port.source.owner = EndpointOwner::Graph(OwnerPath::new(["source"]).unwrap());
    assert_eq!(
        verify(
            &[all(vec![port("sink")]), other_port, other_node, graph_port],
            &DataType::INT
        ),
        Ok(())
    );
}

#[test]
fn manager_command_identity_is_not_a_port_with_a_similar_spelling() {
    let done = command(LoopCommandKind::Done);
    let ordinary = RouteDestination::Port(Endpoint {
        owner: EndpointOwner::Graph(OwnerPath::root()),
        port: node::Port::new("Loop.done").unwrap(),
    });
    assert_ne!(done, ordinary);
    assert_eq!(
        verify(&[all(vec![ordinary, done])], &DataType::Variant),
        Ok(())
    );
    // Endpoint and Extension admission are separate obligations; this local
    // proof must not reinterpret a port's spelling as a manager command.
}

#[test]
fn exhaustive_small_tables_agree_with_a_selected_value_oracle() {
    let vocabulary = [
        port("a"),
        port("b"),
        RouteDestination::Discard,
        command(LoopCommandKind::Again),
    ];
    let set = |mask: usize| -> Vec<RouteDestination> {
        vocabulary
            .iter()
            .enumerate()
            .filter(|(index, _)| mask & (1 << index) != 0)
            .map(|(_, destination)| destination.clone())
            .collect()
    };
    let valid_set = |destinations: &[RouteDestination]| {
        if destinations.is_empty()
            || (destinations.contains(&RouteDestination::Discard) && destinations.len() != 1)
        {
            return false;
        }
        let mut unique = destinations.to_vec();
        unique.sort();
        unique.dedup();
        unique.len() == destinations.len()
    };
    let mut examined = 0;
    // 16 is the absence of a universal declaration, distinct from an empty set.
    for universal in 0..=16 {
        for on_false in 0..16 {
            for on_true in 0..16 {
                let false_set = set(on_false);
                let true_set = set(on_true);
                let universal_set = if universal == 16 {
                    vec![]
                } else {
                    set(universal)
                };
                let mut rules = vec![cases(&[
                    (&["false"], false_set.clone()),
                    (&["true"], true_set.clone()),
                ])];
                if universal != 16 {
                    rules.push(all(universal_set.clone()));
                }
                let expected = (universal == 16 || valid_set(&universal_set))
                    && [false_set, true_set].iter().all(|destinations| {
                        let mut selected = destinations.clone();
                        selected.extend_from_slice(&universal_set);
                        valid_set(destinations) && valid_set(&selected)
                    });
                assert_eq!(
                    verify(&rules, &DataType::Bool).is_ok(),
                    expected,
                    "universal={universal}, false={on_false}, true={on_true}"
                );
                rules.reverse();
                assert_eq!(
                    verify(&rules, &DataType::Bool).is_ok(),
                    expected,
                    "reversed universal={universal}, false={on_false}, true={on_true}"
                );
                examined += 1;
            }
        }
    }
    assert_eq!(examined, 4352);
}
