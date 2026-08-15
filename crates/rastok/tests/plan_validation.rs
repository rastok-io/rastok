//! The `ValidatedExecutionPlan` boundary, held to its own claim:
//! every invariant the type asserts is provoked here at the raw
//! `ExecutionPlan` level — boundary conventions inverted from node
//! conventions, canonical orders, and uniqueness — each refusing
//! with its semantic variant, not a prose string.

#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::canonical_cbor::MAX_NESTING;
use rastok::compiler::compile;
use rastok::contract::Registry;
use rastok::datatype::{DataType, StructType, TaggedCase, TaggedUnionType};
use rastok::expr;
use rastok::node::{Direction, Port};
use rastok::plan::{
    self, Edge, EndpointRef, ExecutionPlan, PlanValidationError, Slot, Step, ValidatedExecutionPlan,
};

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
    compile(
        &[PROBE],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan")
    .plan
    .into_inner()
}

fn port(name: &str) -> Port {
    Port::new(name).unwrap()
}

#[test]
fn a_declared_port_extends_the_conventional_ports() {
    // Source conventions need no declaration, while the checked HIR
    // materializes every used row. A declared port extends that table;
    // the validator must accept the complete union or a checked program
    // would die at lowering as a false compiler bug.
    let source = r#"
graph probe : App {
  operator o {
    output extra: text

    on in where data.anchor == "" {
      out = data
    }
  }

  . -> o -> .
}
"#;
    compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a program the checker accepts must validate at the plan");
}

#[test]
fn redeclaring_a_default_boundary_port_in_its_direction_is_a_no_op() {
    // The grammar makes the defaults REDEFINABLE: `input in` restates
    // what is already true. B5 materializes every used conventional
    // port, so the restatement and the edge share one row.
    let source = r#"
graph g : App {
  input in

  operator o {
    on in where data.anchor == "" {
      out = data
    }
  }

  . -> o -> .
}
"#;
    let compilation = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a same-direction redeclaration is a no-op");
    assert_eq!(compilation.plan.plan().boundary.len(), 2);
    assert!(
        compilation
            .plan
            .plan()
            .boundary
            .iter()
            .any(|port| port.port.name() == "in")
    );
}

#[test]
fn typed_conventional_ports_are_real_constraints() {
    let source = r#"
graph g : App {
  input in: variant

  operator o {
    input in: variant
    on in { out = "done" }
  }

  . -> o -> .
}
"#;
    let compilation = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("an explicit conventional pin is supported");
    let raw = compilation.plan.plan();
    assert!(
        raw.boundary.iter().any(|decl| {
            decl.port.name() == "in" && decl.ty.as_ref() == Some(&DataType::Variant)
        })
    );
    assert!(
        raw.nodes[0].ports.iter().any(|decl| {
            decl.port.name() == "in" && decl.ty.as_ref() == Some(&DataType::Variant)
        })
    );
    let boundary = compilation
        .plan
        .resolved_ports()
        .boundary()
        .iter()
        .find(|decl| decl.port().name() == "in")
        .expect("the boundary port resolves");
    assert_eq!(boundary.ty(), &DataType::Variant);
    assert_eq!(boundary.origin(), plan::TypeOrigin::Declared);

    let package = plan::encode(&compilation.plan, &compilation.hash);
    let (decoded, _) = plan::decode(&package).expect("the source pin survives v6");
    assert_eq!(
        decoded
            .resolved_ports()
            .boundary()
            .iter()
            .find(|decl| decl.port().name() == "in")
            .expect("the decoded pin resolves")
            .ty(),
        &DataType::Variant
    );
}

#[test]
fn the_legacy_runtime_fault_has_a_resolved_producer_pin() {
    let compilation = compile(
        &[PROBE],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the probe resolves");
    let relay = compilation
        .plan
        .plan()
        .nodes
        .iter()
        .position(|node| node.path.to_string() == "relay")
        .expect("the relay operator exists");
    let slot = Slot::new(u32::try_from(relay).expect("the test plan is small"));
    let err = compilation
        .plan
        .resolved_ports()
        .node(slot)
        .expect("the node row exists")
        .iter()
        .find(|port| port.port().name() == "err")
        .expect("a VM-produced runtime fault has an effective pin");

    assert_eq!(err.ty(), &DataType::text_map(DataType::Text));
    assert_eq!(err.origin(), plan::TypeOrigin::Declared);
}

#[test]
fn an_unconstrained_raw_port_never_becomes_variant() {
    let mut raw = raw_probe();
    let relay = raw
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "relay")
        .expect("the relay operator");
    relay
        .ports
        .push(plan::PortDecl::untyped(port("unused"), Direction::Output));

    assert_eq!(
        ValidatedExecutionPlan::try_new(raw).unwrap_err(),
        PlanValidationError::UnresolvedPortType {
            node: Some(rastok::node::Path::new(["relay"]).unwrap()),
            port: port("unused"),
        }
    );
}

#[test]
fn a_raw_port_cannot_contradict_its_instantiated_contract() {
    let mut raw = raw_probe();
    let shell = raw
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "shell")
        .expect("the Process node");
    shell
        .ports
        .iter_mut()
        .find(|decl| decl.port.name() == "in")
        .expect("the instantiated input")
        .ty = Some(DataType::INT);

    assert_eq!(
        ValidatedExecutionPlan::try_new(raw).unwrap_err(),
        PlanValidationError::ContractPortMismatch {
            node: rastok::node::Path::new(["shell"]).unwrap(),
            port: port("in"),
        }
    );
}

#[test]
fn a_second_identical_redeclaration_is_a_duplicate() {
    // The decision, fixed by this test: "redefinable" means AT MOST
    // ONE explicit restatement — the same rule every declaration
    // follows — never a silently idempotent repetition.
    let source = r#"
graph g : App {
  input in
  input in

  operator o {
    on in {
      out = data
    }
  }

  . -> o -> .
}
"#;
    let bag = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("the repetition refuses");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-PORT-CONFLICT"),
        "{bag:?}"
    );
}

#[test]
fn redeclaring_a_conventional_boundary_port_backwards_refuses_at_check() {
    // `output in` CONTRADICTS the conventional boundary direction; it must die in
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
    let bag = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("the redeclaration refuses");
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
fn a_process_table_cannot_omit_a_contract_port() {
    let mut plan = raw_probe();
    let shell = plan
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "shell")
        .expect("the connector");
    shell.ports.retain(|decl| decl.port.name() != "exit");
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::MissingContractPort {
            node: rastok::node::Path::new(["shell"]).unwrap(),
            port: "exit",
        }
    );
}

#[test]
fn a_process_plan_cannot_omit_the_effective_err_codec() {
    let mut plan = raw_probe();
    let shell = plan
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "shell")
        .expect("the Process node");
    let rastok::plan::NodeKind::Connector(shell) = &mut shell.kind else {
        panic!("the Process spec");
    };
    shell.attributes.retain(|(name, _)| name != "err_codec");
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::MissingErrCodec {
            node: rastok::node::Path::new(["shell"]).unwrap(),
        }
    );
}

#[test]
fn a_boundary_port_cannot_carry_a_node_root_sink_policy() {
    let mut plan = raw_probe();
    let input = plan
        .boundary
        .iter_mut()
        .find(|decl| decl.port.name() == "in")
        .expect("the used conventional input is materialized");
    input.unwired = plan::UnwiredPolicy::RootSink;
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::BoundaryHasRootSink { port: port("in") }
    );
}

#[test]
fn a_node_input_cannot_publish_to_a_root_sink() {
    let mut plan = raw_probe();
    let shell = plan
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "shell")
        .expect("the Process node");
    shell
        .ports
        .iter_mut()
        .find(|decl| decl.port.name() == "in")
        .expect("the resolved input")
        .unwired = plan::UnwiredPolicy::RootSink;
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::InputHasRootSink {
            node: rastok::node::Path::new(["shell"]).unwrap(),
            port: port("in"),
        }
    );
}

#[test]
fn a_boundary_input_cannot_be_an_edge_target() {
    // `operator.out -> .request` with `request` declared as a graph
    // INPUT: the boundary convention inverts the node one, and the
    // checker refuses this shape — the type must too.
    let mut plan = raw_probe();
    plan.boundary
        .push(plan::PortDecl::untyped(port("request"), Direction::Input));
    plan.boundary
        .sort_by(|left, right| left.key().cmp(&right.key()));
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
    plan.boundary
        .push(plan::PortDecl::untyped(port("response"), Direction::Output));
    plan.boundary
        .sort_by(|left, right| left.key().cmp(&right.key()));
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
fn conventional_ports_hold_their_directions() {
    // Conventional `in` used as a TARGET — the graph's input fed from
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

    // Conventional `out` used as a SOURCE refuses likewise.
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
fn exit_is_a_connector_convention_not_a_global_reserved_direction() {
    let source = r#"
graph probe : App {
  input in: text
  output out: text

  operator relay {
    input exit: text
    output out: text

    on exit {
      out = data
    }
  }

  . -> relay.exit
  relay -> .
}
"#;
    compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("an Operator may use the ordinary name `exit` for an input");
}

#[test]
fn an_unsorted_boundary_is_not_canonical() {
    let mut plan = raw_probe();
    plan.boundary = vec![
        plan::PortDecl::untyped(port("zeta"), Direction::Input),
        plan::PortDecl::untyped(port("alpha"), Direction::Input),
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
        plan::PortDecl::untyped(port("request"), Direction::Input),
        plan::PortDecl::untyped(port("request"), Direction::Input),
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
fn a_duplicated_node_refuses_as_a_duplicate() {
    // Two identical nodes break the sort too — but the defect is the
    // duplication, and the dedicated variant must say so.
    let mut plan = raw_probe();
    let relay = plan.nodes[0].clone();
    plan.nodes.insert(0, relay);
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::DuplicateNodePath {
            path: rastok::node::Path::new(["relay"]).unwrap(),
        }
    );

    // The same path under two KINDS sorts apart (operators before
    // connectors) — with another operator between them, the
    // NON-adjacent duplicate must still refuse.
    let mut plan = raw_probe();
    let mut ghost = plan.nodes[0].clone();
    ghost.path = shell_path();
    let mut zeta = plan.nodes[0].clone();
    zeta.path = rastok::node::Path::new(["zeta"]).unwrap();
    plan.nodes.insert(1, ghost);
    plan.nodes.insert(2, zeta);
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::DuplicateNodePath { path: shell_path() }
    );
}

#[test]
fn a_duplicated_node_port_refuses() {
    let mut plan = raw_probe();
    let relay = plan
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "relay")
        .expect("the operator");
    relay.ports = vec![
        plan::PortDecl::untyped(port("extra"), Direction::Output),
        plan::PortDecl::untyped(port("extra"), Direction::Output),
    ];
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::DuplicatePort {
            node: rastok::node::Path::new(["relay"]).unwrap(),
            port: port("extra"),
        }
    );
}

#[test]
fn a_duplicated_var_refuses() {
    let mut plan = raw_probe();
    let relay = plan
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "relay")
        .expect("the operator");
    let seen = relay.vars[0].clone();
    relay.vars.insert(0, seen.clone());
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::DuplicateVar {
            node: rastok::node::Path::new(["relay"]).unwrap(),
            name: seen.name,
        }
    );
}

#[test]
fn a_write_to_an_undeclared_var_refuses() {
    let mut plan = raw_probe();
    for step in &mut plan.handlers[0].body {
        if let Step::SetVar { name, .. } = step {
            "ghost".clone_into(name);
        }
    }
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::UnknownVariable {
            node: rastok::node::Path::new(["relay"]).unwrap(),
            name: String::from("ghost"),
        }
    );
}

#[test]
fn an_edge_to_an_unknown_port_refuses() {
    // A node port that exists nowhere: neither implicit nor declared
    // — the reference is not usable in the required role.
    let mut plan = raw_probe();
    plan.edges.push(Edge {
        from: EndpointRef {
            node: Some(Slot::new(0)),
            port: port("ghost"),
        },
        to: None,
    });
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::WrongPortDirection {
            node: Some(rastok::node::Path::new(["relay"]).unwrap()),
            port: port("ghost"),
            expected: Direction::Output,
        }
    );

    // A boundary port that exists nowhere refuses by its own name.
    let mut plan = raw_probe();
    plan.edges.push(Edge {
        from: EndpointRef {
            node: None,
            port: port("ghost"),
        },
        to: None,
    });
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::UnknownBoundaryPort {
            port: port("ghost"),
        }
    );
}

const TWO_TRIGGERS: &str = "graph g : App {\n  operator o {\n    input a: text\n    input b: text\n    on a { out = data }\n    on b { out = data }\n  }\n  . -> o.a\n  . -> o.b\n  o -> .\n}\n";

#[test]
fn a_non_adjacent_duplicate_wins_over_the_broken_order() {
    // The `[a, b, a]` shape: duplicated AND disordered. Uniqueness is
    // a GLOBAL pass that runs first, so the dedicated variant names
    // the defect — never NonCanonicalOrder standing in front of it.
    let mut plan = raw_probe();
    plan.boundary = vec![
        plan::PortDecl::untyped(port("zeta"), Direction::Input),
        plan::PortDecl::untyped(port("alpha"), Direction::Input),
        plan::PortDecl::untyped(port("zeta"), Direction::Input),
    ];
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::DuplicateBoundaryPort { port: port("zeta") }
    );

    let mut plan = raw_probe();
    let relay = plan
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "relay")
        .expect("the operator");
    let seen = relay.vars[0].clone();
    let mut alpha = seen.clone();
    "alpha".clone_into(&mut alpha.name);
    let mut zeta = seen.clone();
    "zeta".clone_into(&mut zeta.name);
    relay.vars = vec![zeta.clone(), alpha, zeta];
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::DuplicateVar {
            node: rastok::node::Path::new(["relay"]).unwrap(),
            name: String::from("zeta"),
        }
    );

    let plan = with_setvar_root(expr::Expr::MapLit(vec![
        (
            String::from("z"),
            expr::Expr::Literal(rastok::value::Value::Int(1)),
        ),
        (
            String::from("a"),
            expr::Expr::Literal(rastok::value::Value::Int(2)),
        ),
        (
            String::from("z"),
            expr::Expr::Literal(rastok::value::Value::Int(3)),
        ),
    ]));
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::DuplicateMapKey {
            key: String::from("z"),
        }
    );
}

#[test]
fn handler_declaration_order_is_not_identity() {
    // identity.md: declaration order is insensitive; only statements
    // INSIDE a handler (and, later, ports inside a join) are order-
    // sensitive. Swapping two handlers must change neither the hash
    // nor the plan.
    let swapped = TWO_TRIGGERS.replacen(
        "on a { out = data }\n    on b { out = data }",
        "on b { out = data }\n    on a { out = data }",
        1,
    );
    let one = compile(
        &[TWO_TRIGGERS],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let two = compile(
        &[swapped.as_str()],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    assert_eq!(one.hash, two.hash, "same identity");
    assert_eq!(one.plan, two.plan, "same canonical plan");
}

#[test]
fn a_duplicated_handler_refuses() {
    // Two handlers for one (node, activation): dispatch would fire
    // the first and silently shadow the second — refused by name.
    let mut plan = raw_probe();
    let first = plan.handlers[0].clone();
    plan.handlers.insert(0, first);
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::DuplicateHandler {
            node: rastok::node::Path::new(["relay"]).unwrap(),
            activation: rastok::node::HandlerActivation::Port(port("in")),
        }
    );
}

#[test]
fn reordered_handlers_are_not_canonical() {
    let mut plan = compile(
        &[TWO_TRIGGERS],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan")
    .plan
    .into_inner();
    plan.handlers.swap(0, 1);
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::NonCanonicalOrder {
            collection: "handlers"
        }
    );
}

#[test]
fn a_reversed_contract_port_table_is_not_canonical() {
    let mut plan = raw_probe();
    let shell = plan
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "shell")
        .expect("the connector");
    shell.ports.reverse();
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::NonCanonicalOrder {
            collection: "ports"
        }
    );
}

#[test]
fn an_extra_process_port_refuses_without_a_registry() {
    let mut plan = raw_probe();
    let shell = plan
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "shell")
        .expect("the connector");
    shell.ports.push(plan::PortDecl {
        port: port("ghost"),
        direction: Direction::Output,
        unwired: plan::UnwiredPolicy::RequireWire,
        ty: Some(DataType::Variant),
    });
    shell
        .ports
        .sort_by(|left, right| left.key().cmp(&right.key()));
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::UnexpectedContractPort {
            node: shell_path(),
            port: port("ghost"),
        }
    );
}

/// Replaces the probe's `seen = …` expression — the chassis for the
/// map-literal canonicity provocations.
fn with_setvar_root(root: expr::Expr) -> ExecutionPlan {
    let mut plan = raw_probe();
    for step in &mut plan.handlers[0].body {
        if let Step::SetVar { expr, .. } = step {
            expr.root = root.clone();
        }
    }
    plan
}

#[test]
fn an_unsorted_map_literal_is_not_canonical() {
    // The parser construction-sorts map literals; a raw plan carrying
    // an unsorted one is a second byte form of the same expression —
    // one the re-encoding check reproduces faithfully, so the
    // validator must refuse it by walking the trees.
    let plan = with_setvar_root(expr::Expr::MapLit(vec![
        (
            String::from("b"),
            expr::Expr::Literal(rastok::value::Value::Int(1)),
        ),
        (
            String::from("a"),
            expr::Expr::Literal(rastok::value::Value::Int(2)),
        ),
    ]));
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::NonCanonicalOrder {
            collection: "map literal"
        }
    );
}

#[test]
fn a_duplicated_map_literal_key_refuses() {
    let plan = with_setvar_root(expr::Expr::MapLit(vec![
        (
            String::from("a"),
            expr::Expr::Literal(rastok::value::Value::Int(1)),
        ),
        (
            String::from("a"),
            expr::Expr::Literal(rastok::value::Value::Int(2)),
        ),
    ]));
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::DuplicateMapKey {
            key: String::from("a"),
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
    let hash = compile(
        &[PROBE],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan")
    .hash;
    let mut plan = raw_probe();
    let mut last_accepted = None;
    let mut refused = false;
    for _ in 0..(MAX_NESTING + 16) {
        let relay_steps = &mut plan.handlers[0].body;
        for step in relay_steps {
            if let Step::SetVar { expr, .. } = step {
                let previous = core::mem::replace(
                    &mut expr.root,
                    expr::Expr::Literal(rastok::value::Value::Int(0)),
                );
                expr.root = expr::Expr::Binary {
                    op: expr::BinaryOp::Add,
                    left: Box::new(previous),
                    right: Box::new(expr::Expr::Literal(rastok::value::Value::Int(0))),
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

#[test]
fn valid_m13_types_cannot_leak_into_a_v6_package() {
    let mut plan = raw_probe();
    let relay = plan
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "relay")
        .expect("the operator");
    relay.vars[0].ty = DataType::LONG;

    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::DataTypeNotInFormat {
            location: String::from("state `relay.seen`"),
            ty: DataType::LONG,
            format: plan::FORMAT_VERSION,
        }
    );
}

#[test]
fn expression_result_types_share_the_v6_package_gate() {
    let mut plan = raw_probe();
    let expression = plan
        .handlers
        .iter_mut()
        .flat_map(|handler| handler.body.iter_mut())
        .find_map(|step| match step {
            Step::SetVar { expr, .. } => Some(expr),
            _ => None,
        })
        .expect("the probe state write");
    expression.ty = DataType::LONG;

    assert!(matches!(
        ValidatedExecutionPlan::try_new(plan),
        Err(PlanValidationError::DataTypeNotInFormat {
            location,
            ty: DataType::LONG,
            format: plan::FORMAT_VERSION,
        }) if location.contains("state expression")
    ));
}

#[test]
fn narrowing_expressions_cannot_leak_into_a_v6_package() {
    for (form, predicate) in [("is", true), ("as", false)] {
        let mut plan = raw_probe();
        let expression = plan
            .handlers
            .iter_mut()
            .flat_map(|handler| handler.body.iter_mut())
            .find_map(|step| match step {
                Step::SetVar { expr, .. } => Some(expr),
                _ => None,
            })
            .expect("the probe state write");
        let operand = core::mem::replace(
            &mut expression.root,
            expr::Expr::Literal(rastok::value::Value::Int(0)),
        );
        let target = expr::NarrowTarget::Type(Box::new(expr::NarrowType::new(DataType::INT)));
        expression.root = if predicate {
            expression.ty = DataType::Bool;
            expr::Expr::Is {
                expr: Box::new(operand),
                target: Box::new(target),
            }
        } else {
            expression.ty = DataType::INT;
            expr::Expr::As {
                expr: Box::new(operand),
                target: Box::new(target),
            }
        };

        assert_eq!(
            ValidatedExecutionPlan::try_new(plan).unwrap_err(),
            PlanValidationError::ExpressionNotInFormat {
                form,
                format: plan::FORMAT_VERSION,
            }
        );
    }
}

#[test]
fn constructed_values_cannot_leak_into_a_v6_package() {
    let payload = StructType::new(Vec::new()).expect("the empty payload is closed");
    let union = TaggedUnionType::new(vec![
        TaggedCase::new("Done", payload).expect("the case is named"),
    ])
    .expect("one named case is a tagged union");

    for (form, root) in [
        ("a struct constructor", expr::Expr::Struct(Vec::new())),
        (
            "a tagged-union constructor",
            expr::Expr::Tagged {
                union: Box::new(union.clone()),
                case: String::from("Done"),
                fields: Vec::new(),
            },
        ),
    ] {
        let mut plan = raw_probe();
        let expression = plan
            .handlers
            .iter_mut()
            .flat_map(|handler| handler.body.iter_mut())
            .find_map(|step| match step {
                Step::SetVar { expr, .. } => Some(expr),
                _ => None,
            })
            .expect("the probe state write");
        expression.root = root;

        assert_eq!(
            ValidatedExecutionPlan::try_new(plan).unwrap_err(),
            PlanValidationError::ExpressionNotInFormat {
                form,
                format: plan::FORMAT_VERSION,
            }
        );
    }
}

#[test]
fn source_narrowing_names_the_v7_transport_gate() {
    let source = r#"
graph g : App {
  input in: variant

  operator o {
    input in: variant
    on in where data is text { out = data }
  }

  . -> o -> .
}
"#;
    let bag = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("v6 has no narrowing-expression spelling");
    assert!(bag.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == rastok::compiler::check::E_CHECK_DEFERRED
            && diagnostic.message().contains("plan-package v6")
            && diagnostic.message().contains("`is`")
    }));
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
