//! The `ValidatedExecutionPlan` boundary, held to its own claim:
//! every invariant the type asserts is provoked here at the raw
//! `ExecutionPlan` level — boundary conventions inverted from node
//! conventions, canonical orders, and uniqueness — each refusing
//! with its semantic variant, not a prose string.

#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::canonical_cbor::MAX_NESTING;
use rastok::compiler::compile;
use rastok::contract::ContractCatalog;
use rastok::datatype::{DataType, StructType, TaggedCase, TaggedUnionType};
use rastok::expr;
use rastok::instance;
use rastok::node::{self, Port, PortDirection};
use rastok::plan::{
    self, DispatchStep, Edge, EdgeEnd, ExecutionPlan, PlanValidationError, Slot,
    ValidatedExecutionPlan,
};

fn linear_statements(handler: &mut plan::Handler) -> &mut Vec<DispatchStep> {
    let plan::Dispatch::Linear(steps) = &mut handler.dispatch else {
        panic!("linear adversarial fixture")
    };
    steps
}

const PROBE: &str = r#"
graph probe : App {
  connector shell : Process {
    mode = job ; command = "cat"
    codec = lines
    idempotent = true; behavior = deterministic
  }

  operator relay {
    var seen : int = 0
    on in {
      seen = seen + 1
      out = data
    }
  }

  in -> relay -> shell -> out
  shell.err -> nothing
  shell.exit -> nothing
}
"#;

fn raw_probe() -> ExecutionPlan {
    compile(
        &[PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan")
    .plan
    .into_inner()
}

#[test]
fn a_connector_cannot_acquire_a_dsl_handler_from_a_raw_plan() {
    let mut raw = raw_probe();
    let target = raw
        .nodes
        .iter()
        .position(|node| node.connector().is_some())
        .unwrap();
    let node = raw.nodes[target].path.clone();
    raw.handlers.push(plan::Handler {
        owner: Slot::new(u32::try_from(target).unwrap()),
        activation: node::HandlerActivation::Port(Port::new("in").unwrap()),
        guard: None,
        dispatch: plan::Dispatch::Linear(vec![]),
    });
    assert_eq!(
        ValidatedExecutionPlan::try_new(raw).unwrap_err(),
        PlanValidationError::NonOperatorHandler { node }
    );
}

#[test]
fn a_handler_owner_is_bounds_checked_before_any_signature_is_derived() {
    let mut raw = raw_probe();
    let node_count = raw.nodes.len();
    raw.handlers[0].owner = Slot::new(u32::MAX);
    assert_eq!(
        ValidatedExecutionPlan::try_new(raw).unwrap_err(),
        PlanValidationError::SlotOutOfBounds {
            slot: Slot::new(u32::MAX),
            node_count,
        }
    );
}

#[test]
fn raw_commands_require_an_owning_loop_application() {
    let variants = [
        (
            DispatchStep::LoopCommand(plan::LoopCommand::Next(plan::EmitValue::Identity)),
            "a Loop command without an owning Loop application",
        ),
        (
            DispatchStep::LoopCommand(plan::LoopCommand::Again),
            "a Loop command without an owning Loop application",
        ),
        (
            DispatchStep::LoopCommand(plan::LoopCommand::Done),
            "a Loop command without an owning Loop application",
        ),
    ];
    for (step, statement) in variants {
        let mut raw = raw_probe();
        linear_statements(&mut raw.handlers[0]).push(step);
        assert_eq!(
            ValidatedExecutionPlan::try_new(raw).unwrap_err(),
            PlanValidationError::UnsupportedStatement {
                handler: 0,
                statement
            }
        );
    }
}

fn raw_join(bounded: bool) -> ExecutionPlan {
    let expiry_type = "type JoinExpiry = union { JoinExpired {} }\n";
    let expiry_ports = if bounded {
        "output expired: JoinExpiry"
    } else {
        ""
    };
    let bound = if bounded { "last 2" } else { "" };
    let expiry_edge = if bounded {
        "pair.expired -> expired"
    } else {
        ""
    };
    let source = format!(
        r#"
{expiry_type}graph joined : App {{
  input left: int
  input right: int
  output out: int
  {expiry_ports}
  operator pair {{
    input left: int
    input right: int
    output out: int
    {expiry_ports}
    on (left, right) {bound} {{ out = left.data }}
  }}
  left -> pair.left
  right -> pair.right
  pair.out -> out
  {expiry_edge}
}}
"#
    );
    compile(
        &[source.as_str()],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the join fixture is valid")
    .plan
    .into_inner()
}

fn raw_loop() -> ExecutionPlan {
    let source = r#"
graph countdown : App {
  input in: text
  output out: text
  operator relay : Loop {
    input in: text
    output err: union { LoopExpired { last_state: text } }
    output out: text
    Loop.fuel = 3
    on in { out = Loop.state.data }
  }
  in -> relay -> out
  relay.err -> nothing
}
"#;
    compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the categorical Loop fixture is valid")
    .plan
    .into_inner()
}

#[test]
fn an_app_val_is_substituted_into_a_nested_loop_constant() {
    let source = r#"
graph app : App {
  input in: text
  output out: text
  val attempts = 3

  operator countdown : Loop {
    input in: text
    output err: union { LoopExpired { last_state: text } }
    output out: text
    Loop.fuel = attempts
    on in { out = Loop.state.data }
  }
  in -> countdown -> out
  countdown.err -> nothing
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("an App val remains visible in its nested Loop");
    let countdown = compilation
        .plan
        .owners
        .iter()
        .find(|owner| owner.path.to_string() == "countdown")
        .expect("the Operator owner travels");
    let [plan::ExtensionSpec::Loop(spec)] = countdown.extensions.as_slice() else {
        panic!("the Operator carries Loop");
    };
    assert_eq!(
        spec.fuel.as_ref().expect("fuel").expr.root,
        expr::Expr::Literal(rastok::value::Value::Int(3)),
        "lexical App constants are substituted before categorical transport"
    );
    let bytes = plan::encode(&compilation.plan, &compilation.hash);
    let (decoded, declared) = plan::decode(&bytes).expect("v16 re-derives inherited val types");
    assert_eq!(decoded, compilation.plan);
    assert_eq!(declared, compilation.hash);

    let mut forged = compilation.plan.clone().into_inner();
    let countdown = forged
        .owners
        .iter_mut()
        .find(|owner| owner.path.to_string() == "countdown")
        .expect("the Operator owner");
    let [plan::ExtensionSpec::Loop(spec)] = countdown.extensions.as_mut_slice() else {
        panic!("the Operator carries Loop");
    };
    spec.fuel.as_mut().expect("fuel").expr.root = expr::Expr::Path(vec![String::from("attempts")]);
    assert!(matches!(
        ValidatedExecutionPlan::try_new(forged),
        Err(PlanValidationError::InvalidGraphContract {
            owner,
            detail: "Loop.fuel is not its claimed plan constant",
        }) if owner == "countdown"
    ));
}

#[test]
fn loop_claims_are_rederived_from_the_host_interface_and_dispatch() {
    let mut zero_fuel = raw_loop();
    let [plan::ExtensionSpec::Loop(spec)] = zero_fuel.owners[1].extensions.as_mut_slice() else {
        panic!("the fixture has one Loop");
    };
    spec.fuel.as_mut().expect("fuel").resolved = rastok::value::Value::Int(0);
    assert!(matches!(
        ValidatedExecutionPlan::try_new(zero_fuel),
        Err(PlanValidationError::InvalidGraphContract { detail, .. })
            if detail.contains("Loop.fuel")
    ));

    let mut wrong_next = raw_loop();
    wrong_next.handlers[0].dispatch = plan::Dispatch::Linear(vec![DispatchStep::LoopCommand(
        plan::LoopCommand::Next(plan::EmitValue::Data(expr::Tree {
            root: expr::Expr::Literal(rastok::value::Value::Int(3)),
            ty: DataType::INT,
        })),
    )]);
    assert!(matches!(
        ValidatedExecutionPlan::try_new(wrong_next),
        Err(PlanValidationError::InvalidLoopControl { detail, .. })
            if detail.contains("Loop.next")
    ));

    let mut wrong_entry = raw_loop();
    let [plan::ExtensionSpec::Loop(spec)] = wrong_entry.owners[1].extensions.as_mut_slice() else {
        panic!("the Operator carries Loop");
    };
    spec.entry = Port::new("interrupt").unwrap();
    assert!(matches!(
        ValidatedExecutionPlan::try_new(wrong_entry),
        Err(PlanValidationError::InvalidGraphContract { detail, .. })
            if detail == "Loop.entry must name a typed input"
    ));

    let mut unbound_asset = raw_loop();
    let handler = &mut unbound_asset.handlers[0];
    let DispatchStep::Emit {
        value: plan::EmitValue::Data(tree),
        ..
    } = &mut linear_statements(handler)[0]
    else {
        panic!("the Operator emits its result");
    };
    tree.root = expr::Expr::Asset(0);
    assert_eq!(
        ValidatedExecutionPlan::try_new(unbound_asset).unwrap_err(),
        PlanValidationError::AssetOutOfBounds { index: 0, count: 0 }
    );
}

#[test]
fn loop_progress_is_reproved_registry_free_from_the_transported_body() {
    let mut silent = raw_loop();
    let handler = &mut silent.handlers[0];
    let DispatchStep::Emit { value, .. } = &mut linear_statements(handler)[0] else {
        panic!("the Operator emits its result")
    };
    *value = plan::EmitValue::Nothing;
    assert!(matches!(
        ValidatedExecutionPlan::try_new(silent),
        Err(PlanValidationError::InvalidLoopControl { detail, .. })
            if detail == "each Loop application must select exactly one of Next, Again, Done, or stop"
    ));

    let mut mixed = raw_loop();
    let handler = &mut mixed.handlers[0];
    linear_statements(handler).push(DispatchStep::LoopCommand(plan::LoopCommand::Again));
    assert!(matches!(
        ValidatedExecutionPlan::try_new(mixed),
        Err(PlanValidationError::InvalidLoopControl { detail, .. })
            if detail == "each Loop application must select exactly one of Next, Again, Done, or stop"
    ));
}

#[test]
fn a_loop_spec_cannot_be_detached_from_its_host_contributions() {
    let mut plan = raw_loop();
    let extensions = core::mem::take(&mut plan.owners[1].extensions);
    plan.owners[0].extensions.extend(extensions);
    let error = ValidatedExecutionPlan::try_new(plan).unwrap_err();
    assert!(
        matches!(&error, PlanValidationError::InvalidGraphContract { .. }),
        "{error:?}"
    );
}

fn bare_operator_plan() -> ExecutionPlan {
    ExecutionPlan {
        boundary_routes: Vec::new(),
        routes: vec![],
        owners: vec![
            plan::Owner {
                path: node::OwnerPath::root(),
                parent: None,
                kind: plan::OwnerKind::Graph,
                ports: vec![],
                extensions: vec![plan::ExtensionSpec::App],
                instance: instance::Spec {
                    key: None,
                    lifetime: instance::Lifetime::Persistent,
                    closing: None,
                },
                vals: vec![],
            },
            plan::Owner {
                path: node::OwnerPath::new(["worker"]).unwrap(),
                parent: Some(node::OwnerSlot::new(0)),
                kind: plan::OwnerKind::Node(Slot::new(0)),
                ports: vec![],
                extensions: vec![],
                instance: instance::Spec {
                    key: None,
                    lifetime: instance::Lifetime::Persistent,
                    closing: None,
                },
                vals: vec![],
            },
        ],
        nodes: vec![plan::Node {
            path: rastok::node::Path::new(["worker"]).unwrap(),
            execution: plan::NodeExecution::Operator,
            ports: vec![],
            vars: vec![],
            owner: node::OwnerSlot::new(1),
        }],
        boundary: vec![],
        edges: vec![],
        handlers: vec![],
        app_schema: vec![],
        ext_schema: vec![],
        assets: vec![],
    }
}

fn port(name: &str) -> Port {
    Port::new(name).unwrap()
}

fn owner_mut<'a>(plan: &'a mut ExecutionPlan, node: &str) -> &'a mut plan::Owner {
    let owner = plan
        .nodes
        .iter()
        .find(|candidate| candidate.path.to_string() == node)
        .expect("the node exists")
        .owner;
    plan.owners
        .get_mut(owner.index())
        .expect("its owner exists")
}

fn owner_slot(plan: &ExecutionPlan, node: &str) -> node::OwnerSlot {
    plan.nodes
        .iter()
        .find(|candidate| candidate.path.to_string() == node)
        .expect("the node exists")
        .owner
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
    input in: struct { anchor: text }
    output extra: text

    on in where data.anchor == "" {
      out = data
    }
  }

  in -> o -> out
}
"#;
    compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
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
    input in: struct { anchor: text }
    on in where data.anchor == "" {
      out = data
    }
  }

  in -> o -> out
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
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

  in -> o -> out
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
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
    let (decoded, _) = plan::decode(&package).expect("the source pin survives v15");
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
fn an_operator_without_err_has_no_hidden_producer_pin() {
    let compilation = compile(
        &[PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
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
    let ports = compilation
        .plan
        .resolved_ports()
        .node(slot)
        .expect("the node row exists");

    assert!(ports.iter().any(|port| port.port().name() == "in"));
    assert!(ports.iter().any(|port| port.port().name() == "out"));
    assert!(
        ports.iter().all(|port| port.port().name() != "err"),
        "validation must not manufacture an undeclared capability"
    );
}

#[test]
fn an_unconstrained_raw_port_never_becomes_variant() {
    let mut raw = raw_probe();
    let relay = raw
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "relay")
        .expect("the relay operator");
    relay.ports.push(plan::PortDecl::untyped(
        port("unused"),
        PortDirection::Output,
    ));

    assert_eq!(
        ValidatedExecutionPlan::try_new(raw).unwrap_err(),
        PlanValidationError::UnresolvedPortType {
            node: Some(rastok::node::Path::new(["relay"]).unwrap()),
            port: port("unused"),
        }
    );
}

#[test]
fn a_raw_instance_requires_a_canonical_key_type() {
    let mut raw = raw_probe();
    owner_mut(&mut raw, "relay").instance = instance::Spec {
        key: Some(expr::Tree {
            root: expr::Expr::Literal(rastok::value::Value::Int(1)),
            ty: DataType::Variant,
        }),
        lifetime: instance::Lifetime::Persistent,
        closing: None,
    };

    assert_eq!(
        ValidatedExecutionPlan::try_new(raw).unwrap_err(),
        PlanValidationError::InvalidInstanceKey {
            owner: String::from("relay"),
            ty: DataType::Variant,
        }
    );
}

#[test]
fn every_duration_lifetime_requires_a_positive_value() {
    for lifetime in [
        instance::Lifetime::Idle(0),
        instance::Lifetime::Fixed(-1),
        instance::Lifetime::Tumbling(0),
    ] {
        let mut raw = raw_probe();
        owner_mut(&mut raw, "relay").instance = instance::Spec {
            key: Some(expr::Tree {
                root: expr::Expr::Literal(rastok::value::Value::Int(1)),
                ty: DataType::INT,
            }),
            lifetime,
            closing: None,
        };

        assert_eq!(
            ValidatedExecutionPlan::try_new(raw).unwrap_err(),
            PlanValidationError::InvalidInstanceLifetime {
                owner: String::from("relay"),
            }
        );
    }
}

#[test]
fn every_closed_lifetime_profile_validates_once_the_scheduler_supports_it() {
    for lifetime in [
        instance::Lifetime::Fixed(1),
        instance::Lifetime::SinglePass,
        instance::Lifetime::Tumbling(1),
    ] {
        let mut raw = bare_operator_plan();
        raw.owners[1].instance.lifetime = lifetime;
        ValidatedExecutionPlan::try_new(raw).expect("R4 executes every closed lifetime profile");
    }
}

#[test]
fn a_persistent_graph_owned_instance_is_a_valid_route_scope() {
    let mut raw = bare_operator_plan();
    raw.owners[0].instance.key = Some(expr::Tree {
        root: expr::Expr::Path(vec![
            String::from("meta"),
            String::from("sys"),
            String::from("id"),
        ]),
        ty: DataType::Text,
    });
    ValidatedExecutionPlan::try_new(raw).expect("R3 routes through Graph-owned key frames");
}

#[test]
fn a_data_key_without_an_admission_signature_never_trusts_its_claimed_type() {
    let mut raw = bare_operator_plan();
    raw.owners[1].instance.key = Some(expr::Tree {
        root: expr::Expr::Path(vec![String::from("data")]),
        ty: DataType::Text,
    });

    assert_eq!(
        ValidatedExecutionPlan::try_new(raw).unwrap_err(),
        PlanValidationError::UnresolvedInstanceAdmission {
            owner: String::from("worker"),
        }
    );
}

#[test]
fn instance_key_type_is_a_derived_fact_not_a_package_claim() {
    let mut metadata = bare_operator_plan();
    metadata.owners[1].instance.key = Some(expr::Tree {
        root: expr::Expr::Path(vec![
            String::from("meta"),
            String::from("sys"),
            String::from("id"),
        ]),
        ty: DataType::BYTES,
    });
    assert_eq!(
        ValidatedExecutionPlan::try_new(metadata).unwrap_err(),
        PlanValidationError::InstanceKeyTypeMismatch {
            owner: String::from("worker"),
            expected: DataType::Text,
            found: DataType::BYTES,
        }
    );

    let mut data = raw_probe();
    owner_mut(&mut data, "relay").instance.key = Some(expr::Tree {
        root: expr::Expr::Path(vec![String::from("data")]),
        ty: DataType::BYTES,
    });
    assert_eq!(
        ValidatedExecutionPlan::try_new(data).unwrap_err(),
        PlanValidationError::InstanceKeyTypeMismatch {
            owner: String::from("relay"),
            expected: DataType::Text,
            found: DataType::BYTES,
        }
    );

    let mut lowering_claim = raw_probe();
    owner_mut(&mut lowering_claim, "relay").instance.key = Some(expr::Tree {
        root: expr::Expr::Path(vec![String::from("data")]),
        ty: DataType::BYTES,
    });
    assert_eq!(
        ValidatedExecutionPlan::try_from_lowered(lowering_claim).unwrap_err(),
        PlanValidationError::InstanceKeyTypeMismatch {
            owner: String::from("relay"),
            expected: DataType::Text,
            found: DataType::BYTES,
        },
        "the compiler-only door closes only an actual inference placeholder"
    );
}

#[test]
fn every_finite_scope_with_a_process_requires_explicit_closing() {
    let mut raw = raw_probe();
    owner_mut(&mut raw, "shell").instance.lifetime = instance::Lifetime::Idle(1);
    assert_eq!(
        ValidatedExecutionPlan::try_new(raw).unwrap_err(),
        PlanValidationError::MissingClosingPolicy {
            connector: String::from("shell"),
            owner: String::from("shell"),
        }
    );

    let mut graph = raw_probe();
    graph.owners[0].instance.lifetime = instance::Lifetime::Fixed(1);
    assert_eq!(
        ValidatedExecutionPlan::try_new(graph).unwrap_err(),
        PlanValidationError::MissingClosingPolicy {
            connector: String::from("shell"),
            owner: String::new(),
        },
        "a finite Graph owns Closing for every Process in its subtree"
    );

    let mut bounded = raw_probe();
    let shell = owner_mut(&mut bounded, "shell");
    shell.instance.lifetime = instance::Lifetime::Idle(1);
    shell.instance.closing = Some(instance::Closing {
        drain_for: 2,
        settle_for: 3,
    });
    ValidatedExecutionPlan::try_new(bounded)
        .expect("explicit positive Closing budgets bound the Process call");
}

#[test]
fn closing_budgets_and_stop_recursion_are_registry_free_invariants() {
    let mut invalid = bare_operator_plan();
    invalid.owners[1].instance.closing = Some(instance::Closing {
        drain_for: 0,
        settle_for: 1,
    });
    assert_eq!(
        ValidatedExecutionPlan::try_new(invalid).unwrap_err(),
        PlanValidationError::InvalidClosingPolicy {
            owner: String::from("worker"),
        }
    );

    let mut recursive = bare_operator_plan();
    recursive.nodes[0].ports.push(plan::PortDecl {
        port: port("in"),
        direction: PortDirection::Input,

        ty: Some(DataType::INT),
        annotations: Default::default(),
    });
    recursive.handlers.push(plan::Handler {
        owner: Slot::new(0),
        activation: rastok::node::HandlerActivation::Lifecycle(rastok::node::Lifecycle::Stop),
        guard: None,
        dispatch: rastok_core::plan::Dispatch::Linear(vec![DispatchStep::Stop]),
    });
    assert_eq!(
        ValidatedExecutionPlan::try_new(recursive).unwrap_err(),
        PlanValidationError::RecursiveStop {
            node: rastok::node::Path::new(["worker"]).unwrap(),
        }
    );
}

#[test]
fn a_raw_node_and_owner_must_point_to_each_other() {
    let mut raw = raw_probe();
    let relay_index = raw
        .nodes
        .iter()
        .position(|node| node.path.to_string() == "relay")
        .expect("the relay slot");
    let relay_slot = Slot::new(u32::try_from(relay_index).expect("the plan is small"));
    raw.nodes[relay_index].owner = node::OwnerSlot::new(0);

    assert_eq!(
        ValidatedExecutionPlan::try_new(raw).unwrap_err(),
        PlanValidationError::OwnerNodeMismatch {
            node: relay_slot,
            owner: node::OwnerSlot::new(1),
        }
    );
}

#[test]
fn a_raw_plan_requires_one_unique_root_owner() {
    let mut missing = raw_probe();
    missing.owners.clear();
    assert_eq!(
        ValidatedExecutionPlan::try_new(missing).unwrap_err(),
        PlanValidationError::MissingRootOwner
    );

    let mut duplicate = raw_probe();
    let relay = owner_slot(&duplicate, "relay");
    let shell = owner_slot(&duplicate, "shell");
    duplicate.owners[shell.index()].path = duplicate.owners[relay.index()].path.clone();
    assert_eq!(
        ValidatedExecutionPlan::try_new(duplicate).unwrap_err(),
        PlanValidationError::DuplicateOwnerPath {
            path: String::from("relay"),
        }
    );
}

#[test]
fn a_raw_owner_parent_must_be_an_enclosing_graph() {
    let mut raw = raw_probe();
    let relay = owner_slot(&raw, "relay");
    let shell = owner_slot(&raw, "shell");
    raw.owners[shell.index()].parent = Some(relay);

    assert_eq!(
        ValidatedExecutionPlan::try_new(raw).unwrap_err(),
        PlanValidationError::InvalidOwnerTree { owner: shell }
    );
}

#[test]
fn an_edge_scope_must_name_a_graph_owner() {
    let mut raw = raw_probe();
    let relay = owner_slot(&raw, "relay");
    raw.edges[0].scope = relay;

    assert_eq!(
        ValidatedExecutionPlan::try_new(raw).unwrap_err(),
        PlanValidationError::InvalidEdgeScope { owner: relay }
    );
}

#[test]
fn an_owner_value_cannot_lie_about_its_evaluation_site() {
    let mut raw = raw_probe();
    let relay = owner_slot(&raw, "relay");
    raw.owners[relay.index()].vals.push(plan::OwnerValue {
        name: String::from("workdir"),
        site: plan::EvaluationSite::Instance,
        expr: expr::Tree {
            root: expr::Expr::Literal(rastok::value::Value::Text(String::from("/tmp"))),
            ty: DataType::Text,
        },
        resolved: Some(rastok::value::Value::Text(String::from("/tmp"))),
        annotations: Default::default(),
    });

    assert_eq!(
        ValidatedExecutionPlan::try_new(raw).unwrap_err(),
        PlanValidationError::BindingSiteMismatch {
            owner: String::from("relay"),
            name: String::from("workdir"),
        }
    );
}

#[test]
fn a_plan_owner_value_cannot_lie_about_its_reduction() {
    let mut raw = raw_probe();
    raw.owners[0].vals.push(plan::OwnerValue {
        name: String::from("answer"),
        site: plan::EvaluationSite::Plan,
        expr: expr::Tree {
            root: expr::Expr::Literal(rastok::value::Value::Int(41)),
            ty: DataType::INT,
        },
        resolved: Some(rastok::value::Value::Int(42)),
        annotations: Default::default(),
    });

    assert_eq!(
        ValidatedExecutionPlan::try_new(raw).unwrap_err(),
        PlanValidationError::OwnerValueResolutionMismatch {
            owner: String::new(),
            name: String::from("answer"),
        }
    );
}

#[test]
fn an_owner_value_cannot_read_outside_its_lexical_environment() {
    let mut raw = raw_probe();
    raw.owners[0].vals.push(plan::OwnerValue {
        name: String::from("answer"),
        site: plan::EvaluationSite::Plan,
        expr: expr::Tree {
            root: expr::Expr::Path(vec![String::from("missing")]),
            ty: DataType::INT,
        },
        resolved: Some(rastok::value::Value::Int(42)),
        annotations: Default::default(),
    });

    assert_eq!(
        ValidatedExecutionPlan::try_new(raw).unwrap_err(),
        PlanValidationError::InvalidBinding {
            owner: String::new(),
            name: String::from("answer"),
            path: String::from("missing"),
        }
    );
}

#[test]
fn owner_value_dependencies_are_order_free_but_never_cyclic() {
    let mut raw = raw_probe();
    raw.owners[0].vals = vec![
        plan::OwnerValue {
            name: String::from("a"),
            site: plan::EvaluationSite::Plan,
            expr: expr::Tree {
                root: expr::Expr::Binary {
                    op: expr::BinaryOp::Add,
                    left: Box::new(expr::Expr::Path(vec![String::from("b")])),
                    right: Box::new(expr::Expr::Literal(rastok::value::Value::Int(1))),
                },
                ty: DataType::INT,
            },
            resolved: Some(rastok::value::Value::Int(2)),
            annotations: Default::default(),
        },
        plan::OwnerValue {
            name: String::from("b"),
            site: plan::EvaluationSite::Plan,
            expr: expr::Tree {
                root: expr::Expr::Literal(rastok::value::Value::Int(1)),
                ty: DataType::INT,
            },
            resolved: Some(rastok::value::Value::Int(1)),
            annotations: Default::default(),
        },
    ];
    ValidatedExecutionPlan::try_new(raw.clone())
        .expect("a canonical dependency is independently reducible");

    raw.owners[0].vals[1].expr.root = expr::Expr::Path(vec![String::from("a")]);
    assert_eq!(
        ValidatedExecutionPlan::try_new(raw).unwrap_err(),
        PlanValidationError::CyclicOwnerValue {
            owner: String::new(),
            name: String::from("a"),
        }
    );
}

#[test]
fn an_owner_value_cannot_claim_a_later_phase_for_a_literal() {
    let mut raw = raw_probe();
    let relay = owner_slot(&raw, "relay");
    raw.owners[relay.index()].vals.push(plan::OwnerValue {
        name: String::from("workdir"),
        site: plan::EvaluationSite::Instance,
        expr: expr::Tree {
            root: expr::Expr::Literal(rastok::value::Value::Text(String::from("/tmp"))),
            ty: DataType::Text,
        },
        resolved: None,
        annotations: Default::default(),
    });

    assert_eq!(
        ValidatedExecutionPlan::try_new(raw).unwrap_err(),
        PlanValidationError::BindingSiteMismatch {
            owner: String::from("relay"),
            name: String::from("workdir"),
        }
    );
}

#[test]
fn a_dynamic_attribute_cannot_claim_a_later_phase_for_a_literal() {
    let mut raw = raw_probe();
    let shell = raw
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "shell")
        .expect("the Process node");
    let spec = match &mut shell.execution {
        plan::NodeExecution::Connector(spec) => spec,
        plan::NodeExecution::Operator => panic!("shell is a connector"),
    };
    let (_, command) = spec
        .attributes
        .iter_mut()
        .find(|(name, _)| name == "command")
        .expect("Process declares command");
    *command = plan::AttrSetting::Expr {
        site: plan::EvaluationSite::Instance,
        expr: expr::Tree {
            root: expr::Expr::Literal(rastok::value::Value::Text(String::from("cat"))),
            ty: DataType::Text,
        },
    };

    assert_eq!(
        ValidatedExecutionPlan::try_new(raw).unwrap_err(),
        PlanValidationError::BindingSiteMismatch {
            owner: String::from("shell"),
            name: String::from("command"),
        }
    );
}

#[test]
fn a_dynamic_attribute_requires_a_registry_free_contract_authority() {
    let mut raw = raw_probe();
    let shell = raw
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "shell")
        .expect("the connector node");
    let spec = match &mut shell.execution {
        plan::NodeExecution::Connector(spec) => spec,
        plan::NodeExecution::Operator => panic!("shell is a connector"),
    };
    spec.contract = rastok::connector::ContractId::parse("org.rastok.test/custom@1").unwrap();
    spec.attributes = vec![(
        String::from("late"),
        plan::AttrSetting::Expr {
            site: plan::EvaluationSite::Instance,
            expr: expr::Tree {
                root: expr::Expr::Path(vec![String::from("instance"), String::from("key")]),
                ty: DataType::Text,
            },
        },
    )];
    let owner = owner_slot(&raw, "shell");
    raw.owners[owner.index()].instance.key = Some(expr::Tree {
        root: expr::Expr::Literal(rastok::value::Value::Text(String::from("tenant"))),
        ty: DataType::Text,
    });

    assert_eq!(
        ValidatedExecutionPlan::try_new(raw).unwrap_err(),
        PlanValidationError::BindingSiteMismatch {
            owner: String::from("shell"),
            name: String::from("late"),
        }
    );
}

#[test]
fn instance_values_and_authorized_connector_settings_share_one_phase_judge() {
    let mut raw = raw_probe();
    let shell_owner = owner_slot(&raw, "shell");
    raw.owners[shell_owner.index()].instance.key = Some(expr::Tree {
        root: expr::Expr::Literal(rastok::value::Value::Text(String::from("tenant"))),
        ty: DataType::Text,
    });
    raw.owners[shell_owner.index()].vals.push(plan::OwnerValue {
        name: String::from("workdir"),
        site: plan::EvaluationSite::Instance,
        expr: expr::Tree {
            root: expr::Expr::Path(vec![String::from("instance"), String::from("key")]),
            ty: DataType::Text,
        },
        resolved: None,
        annotations: Default::default(),
    });
    let shell = raw
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "shell")
        .expect("the Process node");
    let spec = match &mut shell.execution {
        plan::NodeExecution::Connector(spec) => spec,
        plan::NodeExecution::Operator => panic!("shell is a connector"),
    };
    let (_, command) = spec
        .attributes
        .iter_mut()
        .find(|(name, _)| name == "command")
        .expect("Process declares command");
    *command = plan::AttrSetting::Expr {
        site: plan::EvaluationSite::Instance,
        expr: expr::Tree {
            root: expr::Expr::Path(vec![String::from("workdir")]),
            ty: DataType::Text,
        },
    };

    ValidatedExecutionPlan::try_new(raw)
        .expect("the owner value and Process setting both close at instance entry");
}

#[test]
fn a_raw_instance_key_admits_sys_but_not_destination_context() {
    let mut raw = raw_probe();
    owner_mut(&mut raw, "relay").instance = instance::Spec {
        key: Some(expr::Tree {
            root: expr::Expr::Path(vec![
                String::from("meta"),
                String::from("sys"),
                String::from("id"),
            ]),
            ty: DataType::Text,
        }),
        lifetime: instance::Lifetime::Persistent,
        closing: None,
    };

    ValidatedExecutionPlan::try_new(raw.clone())
        .expect("the complete entering envelope includes meta.sys");

    owner_mut(&mut raw, "relay").instance.key = Some(expr::Tree {
        root: expr::Expr::Path(vec![String::from("instance"), String::from("key")]),
        ty: DataType::Text,
    });
    assert_eq!(
        ValidatedExecutionPlan::try_new(raw).unwrap_err(),
        PlanValidationError::InvalidInstanceBinding {
            owner: String::from("relay"),
            path: String::from("instance.key"),
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

  in -> o -> out
}
"#;
    let bag = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
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

  in -> o -> out
}
"#;
    let bag = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
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
    let rastok::plan::NodeExecution::Connector(shell) = &mut shell.execution else {
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
fn a_boundary_input_cannot_be_an_edge_target() {
    // `operator.out -> request` with `request` declared as a graph
    // INPUT: the boundary convention inverts the node one, and the
    // checker refuses this shape — the type must too.
    let mut plan = raw_probe();
    plan.boundary.push(plan::PortDecl::untyped(
        port("request"),
        PortDirection::Input,
    ));
    plan.boundary
        .sort_by(|left, right| left.key().cmp(&right.key()));
    plan.edges.push(Edge {
        scope: node::OwnerSlot::new(0),
        from: EdgeEnd {
            node: Some(Slot::new(0)),
            port: port("out"),
        },
        to: Some(EdgeEnd {
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
            expected: PortDirection::Output,
        }
    );
}

#[test]
fn a_boundary_output_cannot_be_an_edge_source() {
    // `.response -> operator.in` with `response` declared as a graph
    // OUTPUT.
    let mut plan = raw_probe();
    plan.boundary.push(plan::PortDecl::untyped(
        port("response"),
        PortDirection::Output,
    ));
    plan.boundary
        .sort_by(|left, right| left.key().cmp(&right.key()));
    plan.edges.push(Edge {
        scope: node::OwnerSlot::new(0),
        from: EdgeEnd {
            node: None,
            port: port("response"),
        },
        to: Some(EdgeEnd {
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
            expected: PortDirection::Input,
        }
    );
}

#[test]
fn conventional_ports_hold_their_directions() {
    // Conventional `in` used as a TARGET — the graph's input fed from
    // inside the graph — refuses.
    let mut plan = raw_probe();
    plan.edges.push(Edge {
        scope: node::OwnerSlot::new(0),
        from: EdgeEnd {
            node: Some(Slot::new(0)),
            port: port("out"),
        },
        to: Some(EdgeEnd {
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
            expected: PortDirection::Output,
        }
    );

    // Conventional `out` used as a SOURCE refuses likewise.
    let mut plan = raw_probe();
    plan.edges.push(Edge {
        scope: node::OwnerSlot::new(0),
        from: EdgeEnd {
            node: None,
            port: port("out"),
        },
        to: Some(EdgeEnd {
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
            expected: PortDirection::Input,
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

  in -> relay.exit
  relay -> out
}
"#;
    compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("an Operator may use the ordinary name `exit` for an input");
}

#[test]
fn an_unsorted_boundary_is_not_canonical() {
    let mut plan = raw_probe();
    plan.boundary = vec![
        plan::PortDecl::untyped(port("zeta"), PortDirection::Input),
        plan::PortDecl::untyped(port("alpha"), PortDirection::Input),
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
        plan::PortDecl::untyped(port("request"), PortDirection::Input),
        plan::PortDecl::untyped(port("request"), PortDirection::Input),
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
    let rastok::plan::NodeExecution::Connector(spec) = &mut shell.execution else {
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
        plan::PortDecl::untyped(port("extra"), PortDirection::Output),
        plan::PortDecl::untyped(port("extra"), PortDirection::Output),
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
    for step in linear_statements(&mut plan.handlers[0]) {
        if let DispatchStep::SetVar { name, .. } = step {
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
        scope: node::OwnerSlot::new(0),
        from: EdgeEnd {
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
            expected: PortDirection::Output,
        }
    );

    // A boundary port that exists nowhere refuses by its own name.
    let mut plan = raw_probe();
    plan.edges.push(Edge {
        scope: node::OwnerSlot::new(0),
        from: EdgeEnd {
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

const TWO_TRIGGERS: &str = "graph g : App {\n  operator o {\n    input a: text\n    input b: text\n    on a { out = data }\n    on b { out = data }\n  }\n  in -> o.a\n  in -> o.b\n  o -> out\n}\n";

#[test]
fn a_non_adjacent_duplicate_wins_over_the_broken_order() {
    // The `[a, b, a]` shape: duplicated AND disordered. Uniqueness is
    // a GLOBAL pass that runs first, so the dedicated variant names
    // the defect — never NonCanonicalOrder standing in front of it.
    let mut plan = raw_probe();
    plan.boundary = vec![
        plan::PortDecl::untyped(port("zeta"), PortDirection::Input),
        plan::PortDecl::untyped(port("alpha"), PortDirection::Input),
        plan::PortDecl::untyped(port("zeta"), PortDirection::Input),
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
    // INSIDE a handler and ports inside a join are order-sensitive.
    // Swapping two handlers must change neither the hash nor the plan.
    let swapped = TWO_TRIGGERS.replacen(
        "on a { out = data }\n    on b { out = data }",
        "on b { out = data }\n    on a { out = data }",
        1,
    );
    let one = compile(
        &[TWO_TRIGGERS],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let two = compile(
        &[swapped.as_str()],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    assert_eq!(one.hash, two.hash, "same identity");
    assert_eq!(one.plan, two.plan, "same canonical plan");
}

#[test]
fn join_port_order_and_retention_bounds_are_identity() {
    let source = r#"
type JoinExpiry = union { JoinExpired {} }
graph joined : App {
  input left: int
  input right: int
  output out: int
  output expired: JoinExpiry
  operator pair {
    input left: int
    input right: int
    output out: int
    output expired: JoinExpiry
    on (left, right) within duration("1s") last 2 { out = left.data }
  }
  left -> pair.left
  right -> pair.right
  pair.out -> out
  pair.expired -> expired
}
"#;
    let registry = ContractCatalog::try_m0().expect("the standard catalogue holds");
    let original = compile(&[source], &registry).expect("the join compiles");
    let reversed_source = source.replace("on (left, right)", "on (right, left)");
    let reversed = compile(&[reversed_source.as_str()], &registry).expect("the reverse compiles");
    let wider_count_source = source.replace("last 2", "last 3");
    let wider_count =
        compile(&[wider_count_source.as_str()], &registry).expect("the count bound compiles");
    let wider_time_source = source.replace("duration(\"1s\")", "duration(\"2s\")");
    let wider_time =
        compile(&[wider_time_source.as_str()], &registry).expect("the time bound compiles");

    assert_ne!(
        original.hash, reversed.hash,
        "declared join order is meaning"
    );
    assert_ne!(original.hash, wider_count.hash, "`last` is meaning");
    assert_ne!(original.hash, wider_time.hash, "`within` is meaning");
    let node::HandlerActivation::Join { ports, .. } = &original.plan.handlers[0].activation else {
        panic!("the fixture carries one join")
    };
    assert_eq!(
        ports.iter().map(Port::name).collect::<Vec<_>>(),
        ["left", "right"],
        "the plan preserves declared order rather than sorting names"
    );
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
fn a_forged_executable_cycle_never_earns_the_validated_type() {
    let mut plan = raw_probe();
    let relay = plan
        .nodes
        .iter()
        .position(|node| node.path.to_string() == "relay")
        .map(|index| Slot::new(u32::try_from(index).unwrap()))
        .expect("the fixture has relay");
    let shell = plan
        .nodes
        .iter()
        .position(|node| node.path.to_string() == "shell")
        .map(|index| Slot::new(u32::try_from(index).unwrap()))
        .expect("the fixture has shell");
    let back_edge = plan
        .edges
        .iter_mut()
        .find(|edge| {
            edge.from.node == Some(shell) && edge.to.as_ref().is_some_and(|to| to.node.is_none())
        })
        .expect("the fixture routes shell to the boundary");
    back_edge.to = Some(EdgeEnd {
        node: Some(relay),
        port: Port::new("in").unwrap(),
    });

    assert!(matches!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::WiringCycle { node } if node.to_string() == "relay"
    ));
}

#[test]
fn a_duplex_session_boundary_is_not_an_executable_cycle() {
    let source = r#"
graph session_echo : App {
  connector socket : WebSocketServer {
    listen = "127.0.0.1:8080"
    codec = json
    handshake_timeout = duration("5s")
  }
  operator echo { on in { out = . } }
  socket.out -> echo.in
  echo.out -> socket.in
  socket.exit -> nothing
}
"#;
    let raw = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the source checker recognizes the Session boundary")
    .plan
    .into_inner();
    ValidatedExecutionPlan::try_new(raw)
        .expect("Registry-free validation recognizes the same Session boundary");
}

#[test]
fn a_foreign_plan_cannot_smuggle_malformed_join_activation_or_bounds() {
    let mut duplicate_port = raw_join(true);
    let rastok::node::HandlerActivation::Join { ports, .. } =
        &mut duplicate_port.handlers[0].activation
    else {
        panic!("the fixture carries a join")
    };
    ports[1] = ports[0].clone();
    assert!(matches!(
        ValidatedExecutionPlan::try_new(duplicate_port).unwrap_err(),
        PlanValidationError::InvalidJoinActivation { node } if node.to_string() == "pair"
    ));

    let mut zero_bound = raw_join(true);
    let rastok::node::HandlerActivation::Join { last, .. } = &mut zero_bound.handlers[0].activation
    else {
        panic!("the fixture carries a join")
    };
    *last = Some(0);
    assert!(matches!(
        ValidatedExecutionPlan::try_new(zero_bound).unwrap_err(),
        PlanValidationError::InvalidJoinBound { node } if node.to_string() == "pair"
    ));
}

#[test]
fn a_bounded_join_never_invents_an_untyped_or_missing_expiry_channel() {
    let mut missing = raw_join(false);
    let rastok::node::HandlerActivation::Join { last, .. } = &mut missing.handlers[0].activation
    else {
        panic!("the fixture carries a join")
    };
    *last = Some(1);
    assert!(matches!(
        ValidatedExecutionPlan::try_new(missing).unwrap_err(),
        PlanValidationError::MissingJoinExpiryPort { node } if node.to_string() == "pair"
    ));

    let mut identity = raw_join(false);
    let DispatchStep::Emit { value, .. } = &mut linear_statements(&mut identity.handlers[0])[0]
    else {
        panic!("the fixture emits")
    };
    *value = plan::EmitValue::Identity;
    assert!(matches!(
        ValidatedExecutionPlan::try_new(identity).unwrap_err(),
        PlanValidationError::AmbiguousJoinIdentity { node } if node.to_string() == "pair"
    ));
}

#[test]
fn a_required_metadata_read_after_a_join_needs_an_explicit_pick() {
    let without_pick = r#"
graph attributed : App {
  input left: int
  input right: int
  output out: variant

  operator mark_left {
    input in: int
    output out: int
    on in { out = { meta.app.token = data } }
  }
  operator mark_right {
    input in: int
    output out: int
    on in { out = { meta.app.token = data } }
  }
  operator pair {
    input left: int
    input right: int
    output out: int
    on (left, right) { out = left.data }
  }
  operator read_pick {
    input in: int
    output out: variant
    on in { out = meta.app.token }
  }

  left -> mark_left -> pair.left
  right -> mark_right -> pair.right
  pair.out -> read_pick -> out
}
"#;
    let bag = compile(
        &[without_pick],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("CommonOrAbsent cannot satisfy a required downstream read");
    assert!(
        bag.diagnostics().iter().any(|diagnostic| {
            diagnostic.code().as_str() == "E-META-ZONE"
                && diagnostic
                    .fields()
                    .iter()
                    .any(|(name, value)| *name == "field" && value == "meta.app.token")
        }),
        "{bag:?}"
    );

    let with_pick = without_pick.replace(
        "on (left, right) { out = left.data }",
        "on (left, right) { out = { data = left.data ; meta.app.token = right.meta.app.token } }",
    );
    let mut forged = compile(
        &[with_pick.as_str()],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("an explicit named-input pick resolves presence")
    .plan
    .into_inner();
    let pair = forged
        .handlers
        .iter_mut()
        .find(|handler| forged.nodes[handler.owner.index()].path.to_string() == "pair")
        .expect("the join handler");
    let DispatchStep::Emit {
        value: plan::EmitValue::FreshData(writes),
        ..
    } = &mut linear_statements(pair)[0]
    else {
        panic!("the explicit record builder lowers as Replace")
    };
    writes.retain(|write| {
        !matches!(write.target, rastok::envelope::WriteTarget::App(ref field) if field == "token")
    });
    assert!(matches!(
        ValidatedExecutionPlan::try_new(forged).unwrap_err(),
        PlanValidationError::AmbiguousJoinMetadataRead { owner, field }
            if owner == "read_pick" && field == "meta.app.token"
    ));
}

#[test]
fn an_internal_lifecycle_cannot_be_an_authored_handler() {
    let mut plan = raw_probe();
    plan.handlers[0].activation =
        rastok::node::HandlerActivation::Lifecycle(rastok::node::Lifecycle::Reply);
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::UnsupportedLifecycleHandler {
            node: rastok::node::Path::new(["relay"]).unwrap(),
            lifecycle: rastok::node::Lifecycle::Reply,
        }
    );
}

#[test]
fn reordered_handlers_are_not_canonical() {
    let mut plan = compile(
        &[TWO_TRIGGERS],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
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
        direction: PortDirection::Output,

        ty: Some(DataType::Variant),
        annotations: Default::default(),
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
    for step in linear_statements(&mut plan.handlers[0]) {
        if let DispatchStep::SetVar { value: expr, .. } = step {
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
fn an_unbound_asset_call_has_its_own_plan_refusal() {
    let plan = with_setvar_root(expr::Expr::Call {
        function: expr::Function::Asset,
        args: vec![expr::Expr::Literal(rastok::value::Value::Text(
            String::from("assets/logo.png"),
        ))],
    });
    assert_eq!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::UnboundAssetCall
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
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan")
    .hash;
    let mut plan = raw_probe();
    let mut last_accepted = None;
    let mut refused = false;
    for _ in 0..(MAX_NESTING + 16) {
        let relay_steps = linear_statements(&mut plan.handlers[0]);
        for step in relay_steps {
            if let DispatchStep::SetVar { value: expr, .. } = step {
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
                    PlanValidationError::NestingExceeded {
                        depth: MAX_NESTING,
                        bound: MAX_NESTING,
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
fn complete_m13_types_round_trip_through_v7() {
    let mut plan = raw_probe();
    let relay = plan
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "relay")
        .expect("the operator");
    relay.vars[0].ty = DataType::LONG;
    let DispatchStep::SetVar { value, .. } = &mut linear_statements(&mut plan.handlers[0])[0]
    else {
        panic!("the probe increments its state cell")
    };
    value.ty = DataType::LONG;
    let validated = ValidatedExecutionPlan::try_new(plan).expect("int64 is a v7 type");
    let bytes = plan::encode(&validated, &plan::SemanticHash::from_bytes([1; 32]));
    let (decoded, _) = plan::decode(&bytes).expect("the complete type reads back");
    assert_eq!(decoded, validated);
}

#[test]
fn complete_expression_result_types_round_trip_through_v7() {
    let mut plan = raw_probe();
    linear_statements(&mut plan.handlers[0])[0] = DispatchStep::Let {
        name: String::from("wide"),
        value: expr::Tree {
            root: expr::Expr::Literal(rastok::value::Value::Int(i64::MAX)),
            ty: DataType::LONG,
        },
        annotations: Default::default(),
    };
    let validated = ValidatedExecutionPlan::try_new(plan).expect("int64 trees are representable");
    let bytes = plan::encode(&validated, &plan::SemanticHash::from_bytes([2; 32]));
    let (decoded, _) = plan::decode(&bytes).expect("the expression type reads back");
    assert_eq!(decoded, validated);
}

#[test]
fn narrowing_expressions_round_trip_through_v7() {
    for predicate in [true, false] {
        let mut plan = raw_probe();
        let target = expr::NarrowTarget::Type(Box::new(expr::NarrowType::new(DataType::INT)));
        let root = if predicate {
            expr::Expr::Is {
                expr: Box::new(expr::Expr::Literal(rastok::value::Value::Int(0))),
                target: Box::new(target),
            }
        } else {
            expr::Expr::As {
                expr: Box::new(expr::Expr::Literal(rastok::value::Value::Int(0))),
                target: Box::new(target),
            }
        };
        linear_statements(&mut plan.handlers[0])[0] = DispatchStep::Let {
            name: String::from("narrowed"),
            value: expr::Tree {
                root,
                ty: if predicate {
                    DataType::Bool
                } else {
                    DataType::INT
                },
            },
            annotations: Default::default(),
        };
        let validated = ValidatedExecutionPlan::try_new(plan).expect("narrowing is a v7 form");
        let bytes = plan::encode(&validated, &plan::SemanticHash::from_bytes([3; 32]));
        let (decoded, _) = plan::decode(&bytes).expect("the narrowing tree reads back");
        assert_eq!(decoded, validated);
    }
}

#[test]
fn constructed_values_round_trip_through_v7() {
    let payload = StructType::new(Vec::new()).expect("the empty payload is closed");
    let union = TaggedUnionType::new(vec![
        TaggedCase::new("Done", payload.clone()).expect("the case is named"),
    ])
    .expect("one named case is a tagged union");

    for (root, ty) in [
        (expr::Expr::Struct(Vec::new()), DataType::Struct(payload)),
        (
            expr::Expr::Tagged {
                union: Box::new(union.clone()),
                case: String::from("Done"),
                fields: Vec::new(),
            },
            DataType::TaggedUnion(union.clone()),
        ),
    ] {
        let mut plan = raw_probe();
        linear_statements(&mut plan.handlers[0])[0] = DispatchStep::Let {
            name: String::from("constructed"),
            value: expr::Tree { root, ty },
            annotations: Default::default(),
        };
        let validated = ValidatedExecutionPlan::try_new(plan).expect("constructors are v7 forms");
        let bytes = plan::encode(&validated, &plan::SemanticHash::from_bytes([4; 32]));
        let (decoded, _) = plan::decode(&bytes).expect("the constructor reads back");
        assert_eq!(decoded, validated);
    }
}

#[test]
fn source_narrowing_crosses_the_v7_transport() {
    let source = r#"
graph g : App {
  input in: variant

  operator o {
    input in: variant
    on in where data is text { out = data }
  }

  in -> o -> out
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the source narrowing compiles");
    let bytes = plan::encode(&compilation.plan, &compilation.hash);
    let (decoded, hash) = plan::decode(&bytes).expect("the source narrowing reads back");
    assert_eq!(decoded, compilation.plan);
    assert_eq!(hash, compilation.hash);
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
    let rastok::plan::NodeExecution::Connector(spec) = &shell.execution else {
        panic!("a connector");
    };
    spec.attributes[0].0.clone()
}
