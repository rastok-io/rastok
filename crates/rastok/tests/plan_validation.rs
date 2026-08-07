//! The `ValidatedExecutionPlan` boundary, held to its own claim:
//! every invariant the type asserts is provoked here at the raw
//! `ExecutionPlan` level — boundary conventions inverted from node
//! conventions, canonical orders, and uniqueness — each refusing
//! with its semantic variant, not a prose string.

#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::canonical_cbor::MAX_NESTING;
use rastok::cel;
use rastok::compiler::compile;
use rastok::node::{Direction, Port};
use rastok::plan::{
    self, Edge, EndpointRef, ExecutionPlan, PlanValidationError, Slot, Step, ValidatedExecutionPlan,
};
use rastok::stdlib::Registry;

const PROBE: &str = r#"
graph probe : App {
  connector shell : Process {
    mode = job ; command = "cat"
    codec = lines
    effect = idempotent ; behavior = deterministic
  }

  operator relay {
    var seen : int = 0
    on in {
      seen = seen + 1
      out = data
    }
  }

  . -> relay -> shell -> .
}
"#;

fn raw_probe() -> ExecutionPlan {
    compile(&[PROBE], &Registry::m0())
        .expect("a valid plan")
        .plan
        .into_inner()
}

fn port(name: &str) -> Port {
    Port::new(name).unwrap()
}

#[test]
fn a_declared_port_extends_the_implicit_trio() {
    // The checker keeps `in`/`out`/`err` implicit and lets a declared
    // port EXTEND them; the validator must hold the same union, or a
    // checked program dies at lowering as a false compiler bug.
    let source = r#"
graph probe : App {
  operator o {
    output extra

    on in {
      out = data
    }
  }

  . -> o -> .
}
"#;
    compile(&[source], &Registry::m0())
        .expect("a program the checker accepts must validate at the plan");
}

#[test]
fn redeclaring_an_implicit_boundary_port_refuses_at_check() {
    // `output in` contradicts the implicit boundary; it must die in
    // the checker with a named diagnostic, never reach the validator
    // as an internal error.
    let source = r#"
graph probe : App {
  output in

  operator o {
    on in {
      out = data
    }
  }

  . -> o -> .
}
"#;
    let bag = compile(&[source], &Registry::m0()).expect_err("the redeclaration refuses");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-PORT-CONFLICT"),
        "a check-phase refusal, not an internal-plan error: {bag:?}"
    );
    assert!(
        bag.diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.code().as_str() != "E-INTERNAL-PLAN"),
        "{bag:?}"
    );
}

#[test]
fn a_node_table_cannot_declare_the_implicit_trio() {
    let mut plan = raw_probe();
    let relay = plan
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "relay")
        .expect("the operator");
    relay.ports = vec![(port("in"), Direction::Input)];
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::ImplicitPortDeclared {
            node: Some(rastok::node::Path::new(["relay"]).unwrap()),
            port: port("in"),
        }
    );
}

#[test]
fn the_boundary_cannot_declare_the_implicit_trio() {
    let mut plan = raw_probe();
    plan.boundary = vec![(port("err"), Direction::Output)];
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::ImplicitPortDeclared {
            node: None,
            port: port("err"),
        }
    );
}

#[test]
fn a_boundary_input_cannot_be_an_edge_target() {
    // `operator.out -> .request` with `request` declared as a graph
    // INPUT: the boundary convention inverts the node one, and the
    // checker refuses this shape — the type must too.
    let mut plan = raw_probe();
    plan.boundary = vec![(port("request"), Direction::Input)];
    plan.edges.push(Edge {
        from: EndpointRef {
            node: Some(Slot::new(0)),
            port: port("out"),
        },
        to: Some(EndpointRef {
            node: None,
            port: port("request"),
        }),
    });
    let mut edges = plan.edges.clone();
    edges.sort_by_key(|edge| plan.edge_text(edge));
    plan.edges = edges;
    let error = ValidatedExecutionPlan::try_new(plan).unwrap_err();
    assert_eq!(
        error,
        PlanValidationError::WrongPortDirection {
            node: None,
            port: port("request"),
            expected: Direction::Output,
        }
    );
}

#[test]
fn a_boundary_output_cannot_be_an_edge_source() {
    // `.response -> operator.in` with `response` declared as a graph
    // OUTPUT.
    let mut plan = raw_probe();
    plan.boundary = vec![(port("response"), Direction::Output)];
    plan.edges.push(Edge {
        from: EndpointRef {
            node: None,
            port: port("response"),
        },
        to: Some(EndpointRef {
            node: Some(Slot::new(0)),
            port: port("in"),
        }),
    });
    let mut edges = plan.edges.clone();
    edges.sort_by_key(|edge| plan.edge_text(edge));
    plan.edges = edges;
    let error = ValidatedExecutionPlan::try_new(plan).unwrap_err();
    assert_eq!(
        error,
        PlanValidationError::WrongPortDirection {
            node: None,
            port: port("response"),
            expected: Direction::Input,
        }
    );
}

#[test]
fn the_implicit_trio_holds_its_directions() {
    // Implicit `in` used as a TARGET — the graph's input fed from
    // inside the graph — refuses.
    let mut plan = raw_probe();
    plan.edges.push(Edge {
        from: EndpointRef {
            node: Some(Slot::new(0)),
            port: port("out"),
        },
        to: Some(EndpointRef {
            node: None,
            port: port("in"),
        }),
    });
    let mut edges = plan.edges.clone();
    edges.sort_by_key(|edge| plan.edge_text(edge));
    plan.edges = edges;
    let error = ValidatedExecutionPlan::try_new(plan).unwrap_err();
    assert_eq!(
        error,
        PlanValidationError::WrongPortDirection {
            node: None,
            port: port("in"),
            expected: Direction::Output,
        }
    );

    // Implicit `out` used as a SOURCE refuses likewise.
    let mut plan = raw_probe();
    plan.edges.push(Edge {
        from: EndpointRef {
            node: None,
            port: port("out"),
        },
        to: Some(EndpointRef {
            node: Some(Slot::new(0)),
            port: port("in"),
        }),
    });
    let mut edges = plan.edges.clone();
    edges.sort_by_key(|edge| plan.edge_text(edge));
    plan.edges = edges;
    let error = ValidatedExecutionPlan::try_new(plan).unwrap_err();
    assert_eq!(
        error,
        PlanValidationError::WrongPortDirection {
            node: None,
            port: port("out"),
            expected: Direction::Input,
        }
    );
}

#[test]
fn an_unsorted_boundary_is_not_canonical() {
    let mut plan = raw_probe();
    plan.boundary = vec![
        (port("zeta"), Direction::Input),
        (port("alpha"), Direction::Input),
    ];
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::NonCanonicalOrder {
            collection: "boundary"
        }
    );
}

#[test]
fn a_duplicated_boundary_port_refuses() {
    let mut plan = raw_probe();
    plan.boundary = vec![
        (port("request"), Direction::Input),
        (port("request"), Direction::Input),
    ];
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::DuplicateBoundaryPort {
            port: port("request")
        }
    );
}

#[test]
fn reordered_nodes_are_not_canonical() {
    // Same graph, same declared identity, different table order —
    // exactly the second byte form the type exists to refuse.
    let mut plan = raw_probe();
    plan.nodes.swap(0, 1);
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::NonCanonicalOrder {
            collection: "nodes"
        }
    );
}

#[test]
fn reordered_edges_are_not_canonical() {
    let mut plan = raw_probe();
    plan.edges.swap(0, 1);
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::NonCanonicalOrder {
            collection: "edges"
        }
    );
}

#[test]
fn unsorted_vars_are_not_canonical() {
    let mut plan = raw_probe();
    let relay = plan
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "relay")
        .expect("the operator");
    let mut clone = relay.vars[0].clone();
    "aaa".clone_into(&mut clone.name);
    relay.vars.push(clone);
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::NonCanonicalOrder { collection: "vars" }
    );
}

#[test]
fn a_duplicated_connector_attribute_refuses() {
    let mut plan = raw_probe();
    let shell = plan
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "shell")
        .expect("the connector");
    let rastok::plan::NodeKind::Connector(spec) = &mut shell.kind else {
        panic!("a connector");
    };
    let first = spec.attributes[0].clone();
    spec.attributes.insert(0, first);
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::DuplicateAttribute {
            node: shell_path(),
            name: attribute_name(),
        }
    );
}

#[test]
fn what_the_type_accepts_the_format_reads_back() {
    // The reviewer's counter-example, generalized into the property
    // it violated: grow an expression one level per turn; every plan
    // the type accepts must ride the full round trip, and the first
    // refusal must be the typed budget, exactly at the reader's
    // bound. `encode ⇒ decode` is total — no gap at the boundary.
    let hash = compile(&[PROBE], &Registry::m0())
        .expect("a valid plan")
        .hash;
    let mut plan = raw_probe();
    let mut last_accepted = None;
    let mut refused = false;
    for _ in 0..(MAX_NESTING + 16) {
        let relay_steps = &mut plan.handlers[0].body;
        for step in relay_steps {
            if let Step::SetVar { expr, .. } = step {
                let previous =
                    core::mem::replace(&mut expr.root, cel::Expr::Literal(cel::Value::Int(0)));
                expr.root = cel::Expr::Binary {
                    op: cel::BinaryOp::Add,
                    left: Box::new(previous),
                    right: Box::new(cel::Expr::Literal(cel::Value::Int(0))),
                };
            }
        }
        match ValidatedExecutionPlan::try_new(plan.clone()) {
            Ok(validated) => last_accepted = Some(validated),
            Err(error) => {
                assert_eq!(
                    error,
                    PlanValidationError::NestingBudgetExceeded {
                        depth: MAX_NESTING,
                        budget: MAX_NESTING,
                    },
                    "the first refusal sits exactly at the reader's bound"
                );
                refused = true;
                break;
            }
        }
    }
    assert!(refused, "the chain crossed the budget");
    let deepest = last_accepted.expect("a deepest accepted plan");
    let bytes = plan::encode(&deepest, &hash);
    let (decoded, declared) = plan::decode(&bytes).expect("the boundary case reads back");
    assert_eq!(decoded, deepest, "the deepest accepted plan round-trips");
    assert_eq!(declared, hash);
}

fn shell_path() -> rastok::node::Path {
    rastok::node::Path::new(["shell"]).unwrap()
}

fn attribute_name() -> String {
    let plan = raw_probe();
    let shell = plan
        .nodes
        .iter()
        .find(|node| node.path.to_string() == "shell")
        .expect("the connector");
    let rastok::plan::NodeKind::Connector(spec) = &shell.kind else {
        panic!("a connector");
    };
    spec.attributes[0].0.clone()
}
