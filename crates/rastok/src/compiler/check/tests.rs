use super::*;
use crate::compiler::parser::parse;

fn check_source(source: &str) -> (Option<Hir>, Bag) {
    let mut bag = Bag::new();
    let registry = contract::ContractCatalog::try_m0().expect("the standard catalogue holds");
    let tree = parse(SourceId::new(0), source, &registry, &mut bag);
    let unit = Unit {
        files: vec![(SourceId::new(0), tree)],
    };
    let hir = check(&unit, &registry, &mut bag, None);
    (hir, bag)
}

fn diagnostic_codes(bag: &Bag) -> Vec<Code> {
    bag.diagnostics()
        .iter()
        .map(crate::diagnostics::Diagnostic::code)
        .collect()
}

fn check_clean(source: &str) -> Hir {
    let (hir, bag) = check_source(source);
    assert!(
        !bag.has_errors(),
        "unexpected diagnostics: {:#?}",
        bag.diagnostics()
    );
    hir.expect("an app graph")
}

#[test]
fn mock_matches_validate_complete_replies_per_leaf() {
    let reply = "out = \"ok\"; exit = Exited { exit_code = 0, signal = null }";
    for (fallback, valid) in [
        (reply, true),
        ("nothing", true),
        ("out = \"no exit\"", false),
        (
            "exit = Exited { exit_code = 0, signal = null }; out = \"too late\"",
            false,
        ),
        (
            "exit = Exited { exit_code = 0, signal = null }; exit = Exited { exit_code = 0, signal = null }",
            false,
        ),
    ] {
        let source = format!(
            r#"
{M0_PROBE}
lab m0_probe {{
  simulation s on app {{
    mock echo {{
      on in match data == "hello" {{
        true => {{ {reply} }}
        false => {{ {fallback} }}
      }}
    }}
    scenario idle {{ app.in = {{ text = "hello" }} }}
  }}
}}
"#
        );
        let (_, bag) = check_source(&source);
        if valid {
            assert!(!bag.has_errors(), "{:#?}", bag.diagnostics());
        } else {
            assert!(
                diagnostic_codes(&bag).contains(&E_CONTRACT_REQUIRED),
                "{:#?}",
                bag.diagnostics()
            );
        }
    }
}

#[test]
fn a_mock_multi_emission_refuses_at_check_instead_of_faulting_during_simulation() {
    let source = format!(
        r#"
{M0_PROBE}
lab m0_probe {{
  simulation s on app {{
    mock echo {{
      on in {{
        (out, err) = "two frames"
        exit = Exited {{ exit_code = 0, signal = null }}
      }}
    }}
    scenario s {{ app.in = {{ text = "hello" }} }}
  }}
}}
"#
    );
    let (_, bag) = check_source(&source);
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == E_CHECK_DEFERRED
                && diagnostic.message().contains("multi-emission")),
        "the unsupported mock statement must refuse before execution: {:#?}",
        bag.diagnostics()
    );
}

fn handler_body(handler: &Handler) -> &[Statement] {
    handler
        .v15_linear_parts()
        .map(|(_, body)| body)
        .expect("a v15-compatible linear handler")
}

fn handler_guard(handler: &Handler) -> Option<&expr::Tree> {
    handler
        .v15_linear_parts()
        .expect("a v15-compatible linear handler")
        .0
}

const M0_PROBE: &str = r#"
graph m0_probe : App {
  val greeting = "hello"

  connector echo : Process {
mode = job ; command = "cat"
codec = lines
idempotent = true; behavior = deterministic
  }

  operator shout {
input in: struct { text: text }
var count : int = 0
on in where data.text != "" {
  count = count + 1
  out = data.text
}
  }

  in -> shout -> echo -> out
  echo.err -> nothing
  echo.exit -> nothing
}
"#;

#[test]
fn the_m0_probe_checks_into_hir() {
    let hir = check_clean(M0_PROBE);
    assert_eq!(hir.app.name, "m0_probe");
    assert_eq!(hir.app.extensions, vec![node::Extension::App]);
    assert_eq!(hir.app.vals[0].name, "greeting");
    assert_eq!(hir.app.nodes.len(), 2);
    let echo = &hir.app.nodes[0];
    assert_eq!(echo.kind(), node::Kind::Connector);
    assert_eq!(
        echo.connector_contract_id()
            .map(|contract| contract.as_str()),
        Some(crate::connector::stdlib::PROCESS_CONNECTOR_ID)
    );
    // The EFFECTIVE sheet (D-2026-08-12-1): the five written
    // attributes plus the injected defaults — `args = []`,
    // `err_codec = lines` and,
    // on the job mode, `max_output = "8MiB"` and
    // `on_timeout = kill`. `mode` was written here.
    assert_eq!(echo.attributes().len(), 9);
    assert_eq!(
        echo.attributes()[0].value,
        AttrValue::Word(String::from("job"))
    );
    let AttrValue::Const(command) = &echo.attributes()[1].value else {
        panic!("expected a constant attribute");
    };
    assert_eq!(
        command.value,
        crate::value::Value::Text(String::from("cat"))
    );
    let shout = &hir.app.nodes[1];
    assert_eq!(shout.kind(), node::Kind::Operator);
    assert_eq!(shout.vars[0].ty, DataType::INT);
    let handler = &shout.handlers[0];
    assert_eq!(
        handler.activation,
        node::HandlerActivation::Port(node::Port::new("in").unwrap())
    );
    assert!(handler_guard(handler).is_some());
    let body = handler_body(handler);
    assert!(matches!(&body[0], Statement::SetVar { name, .. } if name == "count"));
    let Statement::Emit { port, value } = &body[1] else {
        panic!("expected the emission");
    };
    assert_eq!(port.name(), "out");
    let EmitValue::Data(tree) = value else {
        panic!("expected a whole-data expression");
    };
    assert!(matches!(&tree.root, expr::Expr::Path(path) if path == &["data", "text"]));
}

#[test]
fn structural_node_type_annotations_have_named_migrations() {
    let implicit = r#"
graph app : App {
  input in: text
  output out: text
  operator relay {
input in: text
output out: text
on in { out = . }
  }
  in -> relay -> out
}
"#;
    let explicit = implicit.replace("operator relay {", "operator relay : Operator {");
    let implicit = check_clean(implicit);
    assert_eq!(implicit.app.nodes[0].kind(), node::Kind::Operator);
    let (_, bag) = check_source(&explicit);
    assert!(bag.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == E_NODE_ANNOTATION
            && diagnostic.message().contains("remove `: Operator`")
    }));

    let implicit_graph = r#"
graph app : App {
  input in: text
  output out: text
  graph child {
input in: text
output out: text
operator relay { on in { out = . } }
in -> relay -> out
  }
  in -> child -> out
}
"#;
    let explicit_graph = implicit_graph.replace("graph child {", "graph child : Graph {");
    let implicit_graph = check_clean(implicit_graph);
    assert!(implicit_graph.app.graphs[0].extensions.is_empty());
    let (_, bag) = check_source(&explicit_graph);
    assert!(bag.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == E_NODE_ANNOTATION && diagnostic.message().contains("remove `: Graph`")
    }));
}

#[test]
fn loop_graph_resolves_its_existing_interface_before_lowering() {
    let source = r#"
graph app : App {
  graph retry : Loop {
input in: text
output err: union { LoopExpired { last_state: text } }
output out: text
Loop.fuel = 3
operator relay { on in { out = . } }
in -> relay -> out
  }
  in -> retry -> out
  retry.err -> nothing
}
"#;
    let (hir, bag) = check_source(source);
    let hir = hir.expect("the recoverable HIR keeps the classified annotation");
    assert_eq!(hir.app.graphs[0].extensions, vec![node::Extension::Loop]);
    let spec = hir.app.graphs[0]
        .loop_spec
        .as_ref()
        .expect("the source half of Loop is checked before its plan gate");
    assert_eq!(
        spec.fuel.as_ref().map(|fuel| &fuel.value),
        Some(&crate::value::Value::Int(3))
    );
    assert!(spec.delay.is_none());
    let retry = &hir.app.graphs[0];
    assert_eq!(spec.entry.name(), "in");
    assert_eq!(
        spec.terminals,
        vec![
            node::Port::new("err").unwrap(),
            node::Port::new("out").unwrap()
        ]
    );
    assert!(
        retry
            .boundary
            .iter()
            .all(|port| !matches!(port.port.name(), "next" | "expired"))
    );
    assert!(diagnostic_codes(&bag).is_empty());
}

#[test]
fn loop_source_contract_reduces_every_plan_constant() {
    let source = r#"
graph app : App {
  graph retry : Loop {
input in: text
output err: union { LoopExpired { last_state: text } }
output out: text
val attempts = 4
Loop.fuel = attempts
Loop.delay = {
  kind = exponential
  first = duration("100ms")
  ceiling = duration("5s")
}
operator relay { on in { out = . } }
in -> relay -> out
  }
  in -> retry -> out
  retry.err -> nothing
}
"#;
    let (hir, bag) = check_source(source);
    assert!(diagnostic_codes(&bag).is_empty());
    let spec = hir
        .expect("a fail-closed Loop keeps its checked source contract")
        .app
        .graphs[0]
        .loop_spec
        .clone()
        .expect("LoopSpec");
    assert_eq!(
        spec.fuel.as_ref().map(|fuel| &fuel.value),
        Some(&crate::value::Value::Int(4))
    );
    let DelayPolicy::Exponential { first, ceiling } = spec.delay.expect("delay") else {
        panic!("expected exponential delay");
    };
    assert_eq!(first.value, crate::value::Value::Duration(100_000_000));
    assert_eq!(ceiling.value, crate::value::Value::Duration(5_000_000_000));
}

#[test]
fn removed_or_misplaced_loop_attributes_have_one_owner() {
    let cases = [
        (
            "graph retry : Loop { Loop.variant = { kind = measure } }",
            "Loop.variant` was removed",
        ),
        (
            "graph retry : Loop { max_steps = 3 }",
            "max_steps` was removed",
        ),
        (
            "graph retry : Loop { until = data.done }",
            "terminal paths emit an ordinary output",
        ),
        ("graph retry : Loop { fuel = 3 }", "write `Loop.fuel`"),
        (
            "graph retry { Loop.fuel = 3 }",
            "requires the Graph's `+ Loop` Extension",
        ),
        (
            "graph retry : Loop { input in: text; Loop.fuel = 0 }",
            "must be a positive integer",
        ),
        (
            "graph retry : Loop { input in: text; Loop.fuel = 3; Loop.delay = { kind = exponential; first = duration(\"2s\"); ceiling = duration(\"1s\") } }",
            "ceiling` must be no smaller",
        ),
    ];
    for (declaration, expected) in cases {
        let source = format!("graph app : App {{ {declaration} }}");
        let (_, bag) = check_source(&source);
        assert_eq!(diagnostic_codes(&bag), vec![E_LOOP], "{source}: {bag:#?}");
        assert!(bag.diagnostics()[0].message().contains(expected));
    }
}

#[test]
fn structural_kinds_and_extensions_have_one_compatibility_gate() {
    let cases = [
        (
            "operator o : Loop + Operator { on in { out = . } }",
            "`Operator` is structural, not an Extension",
        ),
        (
            "operator o : Saga { on in { out = . } }",
            "future `Saga` Extension belongs to a Graph",
        ),
        (
            "operator o : Reducer { on in { out = . } }",
            "Operator pattern, not an Extension",
        ),
    ];
    for (declaration, expected) in cases {
        let source = format!("graph app : App {{ {declaration} in -> o -> out }}");
        let (_, bag) = check_source(&source);
        assert!(
            bag.diagnostics().iter().any(|diagnostic| {
                diagnostic.code() == E_NODE_ANNOTATION && diagnostic.message().contains(expected)
            }),
            "{source}: {bag:#?}"
        );
    }

    let source = r#"
graph app : App {
  connector c : Loop { }
}
"#;
    let (_, bag) = check_source(source);
    assert_eq!(diagnostic_codes(&bag), vec![E_NODE_ANNOTATION]);
    assert!(
        bag.diagnostics()[0]
            .message()
            .contains("requires its exact Contract first")
    );

    let source = r#"
graph app : App {
  connector c : Process + Loop {
Loop.fuel = 3
command = "echo"
args = ["ok"]
codec = lines
idempotent = true
behavior = deterministic
  }
  c.out -> nothing
  c.err -> nothing
  c.exit -> nothing
  }
"#;
    let (hir, bag) = check_source(source);
    assert!(!bag.has_errors(), "{bag:#?}");
    assert_eq!(
        hir.expect("the checked App").app.nodes[0].extensions,
        vec![node::Extension::Loop]
    );
}

#[test]
fn edges_desugar_to_explicit_pairs() {
    let hir = check_clean(M0_PROBE);
    let edges = &hir.app.edges;
    assert_eq!(edges.len(), 5);
    assert!(matches!(
        &edges[0].from.owner,
        EndpointOwner::Graph(path) if path.is_root()
    ));
    assert_eq!(edges[0].from.port.name(), "in");
    let EdgeTarget::Port(to) = &edges[0].to else {
        panic!("expected a port target");
    };
    assert!(matches!(&to.owner, EndpointOwner::Node(path) if path.to_string() == "shout"));
    assert_eq!(to.port.name(), "in");
    let EdgeTarget::Port(last) = &edges[2].to else {
        panic!("expected a port target");
    };
    assert!(matches!(&last.owner, EndpointOwner::Graph(path) if path.is_root()));
    assert_eq!(last.port.name(), "out");
}

#[test]
fn case_routes_are_canonical_and_cross_the_plan() {
    let source = r#"
type Outcome = union {
  Good { value: int }
  Bad { reason: text }
}
graph g : App {
  input in: Outcome
  operator source {
input in: Outcome
output out: Outcome
on in { out = . }
  }
  operator good { input in: Outcome ; on in { nothing } }
  operator bad  { input in: Outcome ; on in { nothing } }
  in -> source
  source.out -> match {
Bad  -> bad
Good -> good
  }
}
"#;
    let hir = check_clean(source);
    let route = hir
        .app
        .routes
        .iter()
        .find(|route| route.source.port.name() == "out")
        .expect("the checked case route");
    let RouteSelection::Cases(arms) = &route.selection else {
        panic!("expected a closed route table");
    };
    assert_eq!(arms[0].cases, ["Bad"]);
    assert_eq!(arms[1].cases, ["Good"]);
    assert_eq!(
        hir.app.edges.len(),
        3,
        "one input edge plus two physical routes"
    );
    let (plan, _) = crate::compiler::lower::lower(&hir).unwrap();
    let plan = crate::plan::ValidatedExecutionPlan::try_from_lowered(plan).unwrap();
    assert_eq!(plan.routes().len(), 1);
}

#[test]
fn route_identity_preserves_case_selection_and_erases_set_order() {
    let source = |routes: &str| {
        format!(
            r#"
graph g : App {{
  input in: bool
  operator left {{ input in: bool ; on in {{ nothing }} }}
  operator right {{ input in: bool ; on in {{ nothing }} }}
  {routes}
}}
"#
        )
    };
    let hash = |routes| crate::compiler::lower::semantic_hash(&check_clean(&source(routes)));
    assert_ne!(
        hash("in -> match { true -> left ; false -> right }"),
        hash("in -> match { true -> right ; false -> left }"),
        "the same physical edges can implement different case routing"
    );
    assert_eq!(
        hash("in -> match { true -> left ; false -> right }"),
        hash("in -> match { false -> right ; true -> left }")
    );
    assert_eq!(
        hash("in -> { left ; right }"),
        hash("in -> { right ; left }")
    );
    assert_eq!(
        hash("in -> { left ; right }"),
        hash("in -> left ; in -> right"),
        "a destination set desugars to the same unconditional fan-out"
    );
}

#[test]
fn nothing_is_exclusive_even_across_route_declarations() {
    let source = r#"
type Outcome = union { Good {} Bad {} }
graph g : App {
  input in: Outcome
  operator source {
input in: Outcome
output out: Outcome
on in { out = . }
  }
  operator audit { input in: Outcome ; on in { nothing } }
  operator failed { input in: Outcome ; on in { nothing } }
  in -> source
  source.out -> audit
  source.out -> match { Good -> nothing ; Bad -> failed }
}
"#;
    let (_, bag) = check_source(source);
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == E_ROUTE_MIXED),
        "the universal audit route cannot coexist with `nothing`: {:#?}",
        bag.diagnostics()
    );
}

#[test]
fn additive_case_routes_have_one_identity_and_one_edge_per_destination() {
    let source = |routes: &str| {
        format!(
            r#"
graph g : App {{
  input in: bool
  operator audit {{ input in: bool ; on in {{ nothing }} }}
  operator sink {{ input in: bool ; on in {{ nothing }} }}
  {routes}
}}
"#
        )
    };
    let split = check_clean(&source(
        "in -> audit; in -> match { false -> sink; true -> sink }",
    ));
    let reversed = check_clean(&source(
        "in -> match { true -> sink; false -> sink }; in -> audit",
    ));
    let grouped = check_clean(&source("in -> match { false | true -> { sink; audit } }"));
    let expected = crate::compiler::lower::semantic_hash(&split);
    for hir in [&split, &reversed, &grouped] {
        assert_eq!(crate::compiler::lower::semantic_hash(hir), expected);
        assert_eq!(
            hir.app.edges.len(),
            2,
            "case membership is not edge identity"
        );
    }
}

#[test]
fn additive_route_conflicts_keep_source_diagnostics_and_exact_cases() {
    let universal = "in -> sink";
    for (conditional, code, case) in [
        (
            "in -> match { false -> nothing; true -> audit }",
            E_ROUTE_MIXED,
            "false",
        ),
        (
            "in -> match { false -> audit; true -> sink }",
            E_ROUTE_DUPLICATE,
            "true",
        ),
    ] {
        for routes in [
            format!("{universal}; {conditional}"),
            format!("{conditional}; {universal}"),
        ] {
            let source = format!(
                r#"
graph g : App {{
  input in: bool
  operator audit {{ input in: bool ; on in {{ nothing }} }}
  operator sink {{ input in: bool ; on in {{ nothing }} }}
  {routes}
}}
"#
            );
            let (_, bag) = check_source(&source);
            let error = bag
                .diagnostics()
                .iter()
                .find(|error| error.code() == code)
                .expect("the source conflict must remain a named diagnostic");
            assert!(!error.spans().is_empty());
            assert!(
                error
                    .fields()
                    .iter()
                    .any(|(key, value)| *key == "case" && value == case)
            );
            assert!(
                !diagnostic_codes(&bag).contains(&crate::compiler::E_INTERNAL_PLAN),
                "source mistakes must not become internal routing failures"
            );
        }
    }
}

#[test]
fn destination_sets_are_atomic_hir_not_duplicate_emissions() {
    let source = r#"
graph g : App {
  input in: int
  operator source { input in: int ; output out: int ; on in { out = . } }
  operator left   { input in: int ; on in { nothing } }
  operator right  { input in: int ; on in { nothing } }
  in -> source
  source.out -> { right ; left }
}
"#;
    let hir = check_clean(source);
    let route = hir
        .app
        .routes
        .iter()
        .find(|route| route.source.port.name() == "out")
        .expect("the checked destination set");
    let RouteSelection::All(destinations) = &route.selection else {
        panic!("expected an unconditional route set");
    };
    assert_eq!(destinations.len(), 2);
    assert!(
        route_destination_display(&destinations[0]) < route_destination_display(&destinations[1]),
        "destination order is canonical, not source order"
    );
    let (plan, _) = crate::compiler::lower::lower(&hir).unwrap();
    let plan = crate::plan::ValidatedExecutionPlan::try_from_lowered(plan).unwrap();
    assert_eq!(plan.edges().len(), 3);
}

#[test]
fn loop_manager_commands_are_routes_but_never_spatial_edges() {
    let source = r#"
graph g : App {
  connector call : Process + Loop {
mode = job
command = "cat"
codec = lines
idempotent = true
behavior = deterministic
Loop.fuel = 3
timeout = duration("1s")
  }
  call.out -> nothing
  call.err -> nothing
  call.exit -> match { LoopExpired -> nothing; _ -> call.Loop.again }
  }
"#;
    let hir = check_clean(source);
    let route = hir
        .app
        .routes
        .iter()
        .find(|route| route.source.port.name() == "exit")
        .expect("the terminal route");
    assert!(matches!(
        &route.selection,
        RouteSelection::Cases(arms)
            if arms.iter().any(|arm| matches!(arm.destinations.as_slice(), [RouteDestination::LoopCommand(target)]
                if target.kind == LoopCommandKind::Again))
    ));
    assert!(
        hir.app
            .edges
            .iter()
            .filter(|edge| edge.from.port.name() == "exit")
            .all(|edge| matches!(edge.to, EdgeTarget::Discard)),
        "a manager command must not become a PhysicalEdge"
    );
    let (plan, _) =
        crate::compiler::lower::lower(&hir).expect("the manager command is transported");
    let plan = crate::plan::ValidatedExecutionPlan::try_from_lowered(plan)
        .expect("the Call protocol validates independently");
    assert!(
        plan.edges()
            .iter()
            .filter(|edge| edge.from.port.name() == "exit")
            .all(|edge| edge.to.is_none())
    );
    assert!(
        plan.routes()
            .iter()
            .any(|route| route.source.port.name() == "exit")
    );
    let done = check_clean(&source.replace("call.Loop.again", "call.Loop.done"));
    assert_eq!(hir.app.edges, done.app.edges);
    assert_ne!(
        crate::compiler::lower::semantic_hash(&hir),
        crate::compiler::lower::semantic_hash(&done),
        "manager command kind is semantic even without a spatial edge"
    );
}

#[test]
fn source_sets_desugar_to_independent_mpsc_edges() {
    let source = |route: &str| {
        format!(
            "graph g : App {{ input first: int ; input second: int ; operator consume {{ input in: int ; on in {{ nothing }} }} {route} }}"
        )
    };
    let grouped = check_clean(&source("{ second ; first } -> consume"));
    let separate = check_clean(&source("first -> consume ; second -> consume"));
    assert_eq!(grouped.app.edges.len(), 2);
    assert_eq!(
        crate::compiler::lower::semantic_hash(&grouped),
        crate::compiler::lower::semantic_hash(&separate)
    );
    assert_eq!(
        crate::compiler::lower::lower(&grouped).unwrap(),
        crate::compiler::lower::lower(&separate).unwrap()
    );
}

#[test]
fn closed_dispatch_arm_order_is_not_semantic() {
    let source = |arms: &str| {
        format!(
            r#"
type Mode = enum {{ Ready, Failed }}
graph g : App {{
  input in: Mode
  operator choose {{
input in: Mode
output ready: Mode
output failed: Mode
on in match data {{ {arms} }}
  }}
  in -> choose
  choose.ready -> nothing
  choose.failed -> nothing
}}
"#
        )
    };
    let left = check_clean(&source("Ready => ready = data ; Failed => failed = data"));
    let right = check_clean(&source("Failed => failed = data ; Ready => ready = data"));
    assert_eq!(left.app.nodes[0].handlers, right.app.nodes[0].handlers);
    assert_eq!(
        crate::compiler::lower::semantic_hash(&left),
        crate::compiler::lower::semantic_hash(&right)
    );
    let Dispatch::ClosedMatch { arms, .. } = &left.app.nodes[0].handlers[0].dispatch else {
        panic!("expected a closed Dispatch");
    };
    assert_eq!(arms[0].cases, ["Failed"]);
    assert_eq!(arms[1].cases, ["Ready"]);
}

#[test]
fn closed_and_open_dispatches_refuse_ambiguous_control() {
    let closed = |arms: &str| {
        format!(
            r#"
type Mode = enum {{ Ready, Failed }}
graph g : App {{
  input in: Mode
  operator choose {{
input in: Mode
on in match data {{ {arms} }}
  }}
  in -> choose
}}
"#
        )
    };
    let (_, missing) = check_source(&closed("Ready => nothing"));
    assert!(
        missing
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == E_MATCH_NONEXHAUSTIVE)
    );

    let (_, overlap) = check_source(&closed("Ready => nothing ; Ready | Failed => nothing"));
    assert!(
        overlap
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == E_MATCH_OVERLAP)
    );

    let open = r#"
graph g : App {
  input in: int
  operator choose {
input in: int
on in match { data > 0 => nothing }
  }
  in -> choose
}
"#;
    let (_, missing_fallback) = check_source(open);
    assert!(
        missing_fallback
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == E_MATCH_NONEXHAUSTIVE)
    );
}

#[test]
fn dispatch_position_and_open_predicate_order_are_semantic() {
    let source = |body: &str| {
        format!(
            "graph g : App {{ input in: int ; operator choose {{ input in: int ; output out: int ; var seen: int = 0 ; on in {{ {body} }} }} in -> choose ; choose.out -> nothing }}"
        )
    };
    for prefix in ["out = data", "seen = seen + 1", "stop"] {
        let (_, bag) = check_source(&source(&format!("{prefix}; match {{ _ => nothing }}")));
        assert!(
            diagnostic_codes(&bag).contains(&E_DISPATCH_POSITION),
            "{:#?}",
            bag.diagnostics()
        );
    }
    let valid = check_clean(&source(
        "val score = data + 1; match { score > 0 => out = score ; _ => nothing }",
    ));
    assert!(matches!(&valid.app.nodes[0].handlers[0].dispatch,
        Dispatch::OpenMatch { prefix, .. } if matches!(prefix.as_slice(), [Statement::Let { .. }])));
    let first = check_clean(&source(
        "match { data > 0 => out = 1 ; data > 1 => out = 2 ; _ => nothing }",
    ));
    let reversed = check_clean(&source(
        "match { data > 1 => out = 2 ; data > 0 => out = 1 ; _ => nothing }",
    ));
    assert_ne!(
        crate::compiler::lower::semantic_hash(&first),
        crate::compiler::lower::semantic_hash(&reversed)
    );
}

#[test]
fn where_desugaring_counts_towards_the_dispatch_depth_budget() {
    let source = format!(
        "graph g : App {{ input in: int ; operator choose {{ input in: int ; on in where data > 0 {{ {}nothing{} }} }} in -> choose }}",
        "match { _ => ".repeat(crate::compiler::syntax::MAX_DISPATCH_DEPTH),
        " }".repeat(crate::compiler::syntax::MAX_DISPATCH_DEPTH),
    );
    let (_, bag) = check_source(&source);
    let diagnostics = bag.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].code(),
        crate::compiler::parser::E_DISPATCH_DEPTH
    );
    assert_eq!(diagnostics[0].phase(), Phase::Check);
}

#[test]
fn case_routing_preserves_the_union_and_exposes_its_narrowed_type() {
    let source = r#"
type Outcome = union { Good { value: int } Bad { reason: text } }
type GoodOnly = union { Good { value: int } }
type BadOnly = union { Bad { reason: text } }
graph g : App {
  input in: Outcome
  operator source {
input in: Outcome
output out: Outcome
on in { out = . }
  }
  operator good { input in: GoodOnly ; on in { nothing } }
  operator bad  { input in: BadOnly ; on in { nothing } }
  in -> source
  source.out -> match { Bad -> bad ; Good -> good }
}
"#;
    let hir = check_clean(source);
    let route = hir
        .app
        .routes
        .iter()
        .find(|route| route.source.port.name() == "out")
        .expect("the narrowed route");
    assert!(matches!(&route.selection, RouteSelection::Cases(arms) if arms.len() == 2));

    let wrong = source.replace("Good -> good", "Good -> bad");
    let (_, bag) = check_source(&wrong);
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == E_PORT_TYPE)
    );
}

#[test]
fn graph_ports_and_children_share_one_namespace_in_both_orders() {
    for body in [
        "input work: int ; operator work { on in { nothing } }",
        "operator work { on in { nothing } } ; input work: int",
    ] {
        let source = format!("graph g : App {{ {body} }}");
        let (_, bag) = check_source(&source);
        assert!(
            bag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == E_NAME_CLASH),
            "{source}: {:#?}",
            bag.diagnostics()
        );
    }
}

#[test]
fn a_meta_sys_write_carries_its_normative_code() {
    let source = "graph g : App {\n  operator o {\n    on in {\n      out = { meta.sys.id = data.x }\n    }\n  }\n  in -> o -> out\n}\n";
    let (_, bag) = check_source(source);
    let diagnostic = bag
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code() == E_META_SYS_READONLY)
        .expect("the meta.sys refusal");
    assert!(diagnostic.message().contains("read-only"));
}

#[test]
fn a_tagged_constructor_and_its_optional_field_are_wrapped_at_check() {
    let source = r#"
type ProcessExit = union {
  Exited { exit_code: int, signal: int? },
  SpawnFailed { message: text }
}
graph g : App {
  operator o {
input in: int
output exit: ProcessExit
on in {
  exit = Exited { exit_code = 0, signal = null }
}
  }
  o.exit -> nothing
}
"#;
    let hir = check_clean(source);
    let Statement::Emit { port, value } = &handler_body(&hir.app.nodes[0].handlers[0])[0] else {
        panic!("expected an emission");
    };
    assert_eq!(port.name(), "exit");
    let EmitValue::Data(tree) = value else {
        panic!("expected constructed data");
    };
    let expr::Expr::Tagged {
        case,
        fields,
        union,
        ..
    } = &tree.root
    else {
        panic!("expected a tagged constructor");
    };
    assert_eq!(case, "Exited");
    assert_eq!(union.cases()[0].name(), "Exited");
    assert!(matches!(
        &fields[1],
        (name, expr::Expr::As { .. }) if name == "signal"
    ));
}

#[test]
fn a_structural_union_is_selected_once_in_the_checked_tree() {
    let source = r#"
graph g : App {
  operator o {
input in: int
output out: int | text
on in { out = 5 }
  }
  o.out -> nothing
}
"#;
    let hir = check_clean(source);
    let Statement::Emit {
        value: EmitValue::Data(tree),
        ..
    } = &handler_body(&hir.app.nodes[0].handlers[0])[0]
    else {
        panic!("expected a data emission");
    };
    assert!(matches!(tree.root, expr::Expr::As { .. }));
    assert!(matches!(tree.ty, DataType::StructuralUnion(_)));
}

#[test]
fn different_structural_unions_require_an_explicit_as() {
    let implicit = r#"
type Small = int | text
type Wide = int | text | bool
graph g : App {
  operator o {
input in: Small
output out: Wide
on in { out = data }
  }
  o.out -> nothing
}
"#;
    let (_, bag) = check_source(implicit);
    assert!(bag.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == E_PORT_TYPE
            && diagnostic.message().contains("requires an explicit `as`")
    }));

    let explicit = implicit.replace("out = data", "out = data as Wide");
    let hir = check_clean(&explicit);
    let Statement::Emit {
        value: EmitValue::Data(tree),
        ..
    } = &handler_body(&hir.app.nodes[0].handlers[0])[0]
    else {
        panic!("expected a data emission");
    };
    assert!(matches!(tree.root, expr::Expr::As { .. }));
}

#[test]
fn err_is_an_ordinary_inferred_output_port() {
    let source = r#"
type Failure = union { Broken { message: text } }
graph g : App {
  operator o {
on in { err = Broken { message = "no" } }
  }
  o.err -> nothing
}
"#;
    let hir = check_clean(source);
    let err = hir.app.nodes[0]
        .ports
        .iter()
        .find(|port| port.port.name() == "err")
        .expect("err is materialized like every other emitted port");
    assert_eq!(err.direction, PortDirection::Output);
    assert!(matches!(
        &handler_body(&hir.app.nodes[0].handlers[0])[0],
        Statement::Emit { port, value: EmitValue::Data(expr::Tree { root: expr::Expr::Tagged { .. }, .. }) }
            if port.name() == "err"
    ));
}

#[test]
fn a_bad_tagged_constructor_refuses_at_the_typed_destination() {
    let source = r#"
type Exit = union { Exited { code: int } }
graph g : App {
  operator o {
output exit: Exit
on in { exit = Exited { message = "wrong" } }
  }
}
"#;
    let (_, bag) = check_source(source);
    assert_eq!(
        diagnostic_codes(&bag)
            .into_iter()
            .filter(|code| *code == E_PORT_TYPE)
            .count(),
        1
    );
}

#[test]
fn a_unit_needs_exactly_one_app() {
    let (hir, bag) = check_source("graph a : App { }\ngraph b : App { }\n");
    assert!(hir.is_none());
    assert_eq!(bag.diagnostics()[0].code(), E_APP_COUNT);
    let (hir, bag) = check_source("graph a { }\n");
    assert!(hir.is_none());
    assert_eq!(bag.diagnostics()[0].code(), E_APP_COUNT);
}

#[test]
fn unknown_names_and_ports_are_refused() {
    let source = "graph g : App {\n  operator o { }\n  o -> ghost\n  o.mystery -> o\n}\n";
    let (_, bag) = check_source(source);
    let codes: Vec<_> = bag
        .diagnostics()
        .iter()
        .map(crate::diagnostics::Diagnostic::code)
        .collect();
    assert!(codes.contains(&E_RESOLVE_UNKNOWN));
    assert!(codes.contains(&E_PORT_UNKNOWN));
}

#[test]
fn duplicate_nodes_are_refused() {
    let (_, bag) = check_source("graph g : App {\n  operator o { }\n  operator o { }\n}\n");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == E_DUP_NODE)
    );
}

#[test]
fn expression_faults_map_back_to_source_spans() {
    let source = "graph g : App {\n  operator o {\n    on in where mystery > 0 { out = . }\n  }\n  in -> o -> out\n}\n";
    let (_, bag) = check_source(source);
    let diagnostic = bag
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code() == E_EXPR)
        .expect("a Rastok expression diagnostic");
    let span = diagnostic.spans()[0];
    let range = span.range();
    assert_eq!(&source[range], "mystery > 0");
}

#[test]
fn a_missing_behavior_violates_the_sheet() {
    let source = "graph g : App {\n  connector c : Process {\n    command = \"cat\" ; codec = lines\n\n  }\n  in -> c -> out\n}\n";
    let (_, bag) = check_source(source);
    let diagnostic = bag
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code() == E_CONTRACT_REQUIRED)
        .expect("the missing-behavior refusal");
    assert_eq!(diagnostic.fields()[0].1, "behavior");
}

#[test]
fn a_domain_violation_names_the_sheet() {
    let source = "graph g : App {\n  connector c : Process {\n    command = \"cat\" ; codec = utf16\n    idempotent = true; behavior = deterministic\n  }\n  in -> c -> out\n}\n";
    let (_, bag) = check_source(source);
    let diagnostic = bag
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code() == E_CONTRACT_ATTR)
        .expect("the codec refusal");
    assert!(diagnostic.message().contains("Process sheet"));
}

#[test]
fn process_correlation_is_retired_in_favor_of_an_operator_adapter() {
    let source = "graph g : App {\n  connector c : Process {\n    command = \"cat\" ; codec = lines\n    idempotent = true; behavior = deterministic\n    correlate = data.id\n  }\n  in -> c -> out\n}\n";
    let (_, bag) = check_source(source);
    let diagnostic = bag
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code() == E_CONTRACT_ATTR)
        .expect("the correlate refusal");
    assert!(
        diagnostic
            .message()
            .contains("`correlate` is retired for Process"),
        "{}",
        diagnostic.message()
    );
    assert!(
        diagnostic
            .message()
            .contains("application correlation belongs to Operators"),
        "{}",
        diagnostic.message()
    );
}

#[test]
fn unavailable_and_unknown_connector_types_split() {
    let declared = "graph g : App {\n  connector c : Timer { }\n  in -> c -> out\n}\n";
    let (_, bag) = check_source(declared);
    assert!(diagnostic_codes(&bag).contains(&E_CONNECTOR_UNAVAILABLE));
    assert!(bag.diagnostics()[0].message().contains("catalogue"));

    let removed = "graph g : App {\n  connector c : Clock { }\n}\n";
    let (_, bag) = check_source(removed);
    assert_eq!(bag.diagnostics().len(), 1);
    assert!(
        bag.diagnostics()[0]
            .message()
            .contains("renamed to `Timer`")
    );

    let unknown = "graph g : App {\n  connector d : Postgres { }\n  in -> d -> out\n}\n";
    let (_, bag) = check_source(unknown);
    assert!(bag.diagnostics().iter().any(|diagnostic| {
        diagnostic
            .message()
            .contains("not a known Connector Contract")
    }));
}

#[test]
fn a_map_emission_compiles_to_the_map_literal_form() {
    // `{ "n": 1 }` is an executable emission now — the entries
    // compile into the existing MapLit expression.
    let source = "graph g : App {\n  operator o {\n    on in { out = { \"n\": 1 } }\n  }\n  in -> o -> out\n}\n";
    let (hir, bag) = check_source(source);
    assert!(!bag.has_errors(), "{:#?}", bag.diagnostics());
    let hir = hir.expect("a checked unit");
    let Statement::Emit { value, .. } = &handler_body(&hir.app.nodes[0].handlers[0])[0] else {
        panic!("an emission");
    };
    let EmitValue::Data(tree) = value else {
        panic!("a data emission");
    };
    assert!(
        matches!(&tree.root, expr::Expr::MapLit(pairs) if pairs.len() == 1),
        "the MapLit form: {tree:?}"
    );
}

#[test]
fn a_named_type_guard_narrows_the_handler_body() {
    let source = r#"
type Customer = struct { id: int }

graph g : App {
  operator inspect {
on in where data is Customer {
  out = data.id
}
  }

  in -> inspect -> out
}
"#;
    let hir = check_clean(source);
    let handler = &hir.app.nodes[0].handlers[0];
    let guard = handler_guard(handler).expect("the compiled guard");
    let customer = DataType::Struct(
        StructType::new(vec![TypeField::new("id", DataType::INT)]).expect("one closed field"),
    );
    let expr::Expr::Is { target, .. } = &guard.root else {
        panic!("the narrowing guard");
    };
    assert!(matches!(
        target.as_ref(),
        expr::NarrowTarget::Type(target) if target.ty() == &customer
    ));
    let Statement::Emit { value, .. } = &handler_body(handler)[0] else {
        panic!("the guarded emission");
    };
    let EmitValue::Data(tree) = value else {
        panic!("the expression emission");
    };
    assert_eq!(tree.ty, DataType::INT);
    assert!(matches!(&tree.root, expr::Expr::Path(path) if path == &["data", "id"]));
}

#[test]
fn the_removed_struct_spelling_is_gone_from_the_grammar() {
    // D-2026-08-09-1 removes `struct { … }` from value positions
    // cleanly — a braced record IS the struct. The old spelling
    // now falls outside the grammar and refuses; it never parses
    // as anything else.
    let source = "graph g : App {\n  operator o {\n    on in { out = struct { n = 1 } }\n  }\n  in -> o -> out\n}\n";
    let (_, bag) = check_source(source);
    assert!(
        !bag.diagnostics().is_empty(),
        "the removed form must refuse, not resolve"
    );
}

#[test]
fn saga_effect_words_are_not_ordinary_call_attributes() {
    for word in ["none", "idempotent", "compensable", "irreversible"] {
        let source = format!(
            r#"
graph g : App {{
  connector c : Process {{
    command = "cat"; codec = lines
    effect = {word}; behavior = deterministic
  }}
  in -> c -> out
  c.err -> nothing
  c.exit -> nothing
}}
"#
        );
        let (_, bag) = check_source(&source);
        assert!(
            bag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == E_CONTRACT_ATTR
                    && diagnostic.fields()[0].1 == "effect"),
            "{word}: {bag:#?}"
        );
    }
}

#[test]
fn an_unwired_output_refuses_until_discarded_explicitly() {
    let source = "graph g : App {\n  operator o {\n    on in {\n      out = .\n      side = .\n    }\n  }\n  in -> o -> out\n}\n";
    let (_, bag) = check_source(source);
    assert!(bag.has_errors(), "a possible emission needs a disposition");
    assert!(bag.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == E_UNWIRED_OUTPUT && diagnostic.to_string().contains("o.side")
    }));
    // Discarding in writing silences it.
    let discarded = "graph g : App {\n  operator o {\n    on in {\n      out = .\n      side = .\n    }\n  }\n  in -> o -> out\n  o.side -> nothing\n}\n";
    let (_, bag) = check_source(discarded);
    assert!(
        !bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == E_UNWIRED_OUTPUT)
    );
}

#[test]
fn a_second_handler_for_one_trigger_is_refused() {
    let source = "graph g : App {\n  operator o {\n    on in { out = . }\n    on in { out = . }\n  }\n  in -> o -> out\n}\n";
    let (_, bag) = check_source(source);
    assert!(bag.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == E_HANDLER_DUPLICATE && diagnostic.fields()[1].1 == "p:in"
    }));
}

#[test]
fn join_bounds_do_not_create_a_second_trigger_identity() {
    let source = r#"
type JoinExpiry = union { JoinExpired {} }
graph g : App {
  input left: int
  input right: int
  operator pair {
input left: int
input right: int
output expired: JoinExpiry
on (left, right) within duration("1s") { nothing }
on (left, right) within duration("2s") { nothing }
  }
  left -> pair.left
  right -> pair.right
  pair.expired -> nothing
}
"#;
    let (_, bag) = check_source(source);
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == E_HANDLER_DUPLICATE)
    );
}

#[test]
fn a_second_where_has_a_dispatch_diagnostic_not_a_generic_deferral() {
    let source = r#"
graph g : App {
  input in: int
  operator filter {
input in: int
on in where data > 0 where data < 10 { nothing }
  }
  in -> filter
}
"#;
    let (_, bag) = check_source(source);
    let diagnostics: Vec<_> = bag
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code() == E_DISPATCH_POSITION)
        .collect();
    assert_eq!(diagnostics.len(), 1, "{:#?}", bag.diagnostics());
    assert!(diagnostics[0].message().contains("at most one `where`"));
}

#[test]
fn one_port_where_and_its_open_match_desugaring_share_identity() {
    let where_source = r#"
graph g : App {
  input in: int
  output out: int
  operator keep {
input in: int
output out: int
on in where data > 0 { out = data }
  }
  in -> keep -> out
}
"#;
    let match_source = r#"
graph g : App {
  input in: int
  output out: int
  operator keep {
input in: int
output out: int
on in match {
  data > 0 => out = data
  _        => nothing
}
  }
  in -> keep -> out
}
"#;
    let where_hir = check_clean(where_source);
    let match_hir = check_clean(match_source);
    assert_eq!(
        where_hir.app.nodes[0].handlers,
        match_hir.app.nodes[0].handlers
    );
    assert_eq!(
        crate::compiler::lower::semantic_hash(&where_hir),
        crate::compiler::lower::semantic_hash(&match_hir)
    );
}

#[test]
fn loop_operator_resolves_entry_terminals_and_manager_commands() {
    let source = r#"
graph g : App {
  input in: int
  output out: int
  operator retry : Loop {
input request: int
output err: union { LoopExpired { last_state: int } }
output done: int
output failed: int
Loop.entry = request
Loop.terminal = [failed, done, err]
Loop.fuel = 3
on request match {
  data <= 0 => done = data
  _         => Loop.next = Loop.state.data - 1
}
  }
  in -> retry.request
  retry.done -> out
  retry.failed -> nothing
  retry.err -> nothing
}
"#;
    let hir = check_clean(source);
    let retry = &hir.app.nodes[0];
    let spec = retry.loop_spec.as_ref().expect("the checked Loop contract");
    assert_eq!(spec.entry.name(), "request");
    assert_eq!(
        spec.terminals
            .iter()
            .map(node::Port::name)
            .collect::<Vec<_>>(),
        ["done", "err", "failed"]
    );
    let Dispatch::OpenMatch { arms, .. } = &retry.handlers[0].dispatch else {
        panic!("expected the open predicate dispatch");
    };
    assert!(matches!(
        arms[0].dispatch.as_ref(),
        Dispatch::Linear(statements)
            if matches!(statements.as_slice(), [Statement::Emit { port, .. }] if port.name() == "done")
    ));
    let (raw, _) = crate::compiler::lower::lower(&hir).expect("the Operator manager is executable");
    let validated = crate::plan::ValidatedExecutionPlan::try_from_lowered(raw).unwrap();
    let hash = crate::compiler::lower::semantic_hash(&hir);
    let bytes = crate::plan::encode(&validated, &hash);
    assert_eq!(crate::plan::decode(&bytes).unwrap().0, validated);
}

#[test]
fn executor_loop_identity_includes_its_resolved_policy() {
    let source = |attributes: &str| {
        format!(
            r#"
graph g : App {{
  input in: int
  operator retry : Loop {{
input request: int
input alternate: int
output err: union {{ LoopExpired {{ last_state: int }} }}
output accepted: int
output failed: int
{attributes}
on request {{ Loop.done }}
on alternate {{ Loop.done }}
  }}
  in -> retry.request
  retry.accepted -> nothing
  retry.failed -> nothing
  retry.err -> nothing
}}
"#
        )
    };
    let policy = "Loop.entry = request\nLoop.terminal = [accepted, failed, err]\nLoop.fuel = 3";
    let hash =
        |attributes: &str| crate::compiler::lower::semantic_hash(&check_clean(&source(attributes)));
    let baseline = hash(policy);
    for changed in [
        policy.replace("fuel = 3", "fuel = 4"),
        policy.replace("entry = request", "entry = alternate"),
        policy.replace("[accepted, failed, err]", "[accepted, err]"),
        format!("{policy}\nLoop.delay = {{ kind = fixed ; after = duration(\"1s\") }}"),
    ] {
        assert_ne!(baseline, hash(&changed), "Loop policy: {changed}");
    }
    assert_eq!(
        baseline,
        hash(&policy.replace("[accepted, failed, err]", "[failed, err, accepted]"))
    );
}

#[test]
fn graph_loop_retains_its_boundary_orchestration_in_the_validated_plan() {
    let hir = check_clean(
        r#"
graph g : App {
  input in: int
  graph repeat : Loop {
input in: int
output err: union { LoopExpired { last_state: int } }
output out: int
Loop.fuel = 3
operator pass { input in: int ; output out: int ; on in { out = data } }
in -> pass -> out
  }
  in -> repeat
  repeat.out -> nothing
  repeat.err -> nothing
}
"#,
    );
    let (plan, _) = crate::compiler::lower::lower(&hir).unwrap();
    let plan = crate::plan::ValidatedExecutionPlan::try_new(plan).unwrap();
    assert!(plan.reference_routes().next().is_some());
    assert_eq!(
        plan.handlers().count(),
        1,
        "only the explicit child owns a handler"
    );
}

#[test]
fn loop_state_is_inherited_through_scopes_and_masked_by_an_inner_loop() {
    let source = |extension: &str, attributes: &str, state: &str| {
        format!(
            r#"
graph g : App {{
  input in: struct {{ n: int }}
  graph outer : Loop {{
input in: struct {{ n: int }}
output err: union {{ LoopExpired {{ last_state: struct {{ n: int }} }} }}
output out: int
Loop.fuel = 3
graph nested {extension} {{
  input in: int
  output out: int
  {attributes}
  operator read {{
    input in: int
    output out: int
    on in {{ out = {state} }}
  }}
  in -> read -> out
}}
operator project {{
  input in: struct {{ n: int }}
  output out: int
  on in {{ out = data.n }}
}}
in -> project -> nested -> out
  }}
  in -> outer
  outer.out -> nothing
  outer.err -> nothing
}}
"#
        )
    };
    for (extension, attributes, state) in [
        ("", "", "Loop.state.data.n"),
        (
            ": Loop",
            "Loop.fuel = 2; output err: union { LoopExpired { last_state: int } }",
            "Loop.state.data",
        ),
    ] {
        let source = source(extension, attributes, state);
        let source = if extension.is_empty() {
            source
        } else {
            source.replace(
                "in -> project -> nested -> out",
                "in -> project -> nested -> out\nnested.err -> nothing",
            )
        };
        let hir = check_clean(&source);
        let read = &hir.app.graphs[0].graphs[0].nodes[0];
        let Dispatch::Linear(body) = &read.handlers[0].dispatch else {
            panic!("expected the reader's linear handler");
        };
        assert!(
            matches!(body.as_slice(), [Statement::Emit { value: EmitValue::Data(tree), .. }]
            if tree.ty == DataType::INT)
        );
        let (plan, _) = crate::compiler::lower::lower(&hir).unwrap();
        crate::plan::ValidatedExecutionPlan::try_new(plan).unwrap();
    }
}

#[test]
fn new_loop_measure_reads_the_qualified_state_binding() {
    let hir = check_clean(
        r#"
graph g : App {
  input in: struct { n: int }
  operator count : Loop {
input in: struct { n: int }
output out: int
Loop.measure = Loop.state.data.n
on in match {
  data.n <= 0 => out = data.n
  _ => Loop.next = { n = data.n - 1 }
}
  }
  in -> count
  count.out -> nothing
}
"#,
    );
    let proof = hir.app.nodes[0]
        .loop_spec
        .as_ref()
        .unwrap()
        .proof
        .as_ref()
        .unwrap();
    assert!(matches!(&proof.measure.root, expr::Expr::Path(path)
        if path.join(".") == "Loop.state.data.n"));
    assert_eq!(proof.decreases, [1]);
    let (raw, _) = crate::compiler::lower::lower(&hir).unwrap();
    let plan = crate::plan::ValidatedExecutionPlan::try_from_lowered(raw).unwrap();
    let hash = crate::compiler::lower::semantic_hash(&hir);
    assert_eq!(
        crate::plan::decode(&crate::plan::encode(&plan, &hash))
            .unwrap()
            .0,
        plan
    );
}

#[test]
fn loop_next_is_typed_by_the_resolved_entry_not_a_conventional_name() {
    let source = r#"
graph g : App {
  input in: int
  operator retry : Loop {
input in: text
output err: union { LoopExpired { last_state: int } }
input request: int
Loop.entry = request
Loop.fuel = 3
on request { Loop.next = "wrong entry type" }
  }
  in -> retry.request
}
"#;
    let (_, bag) = check_source(source);
    let diagnostic = bag
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code() == E_PORT_TYPE)
        .expect("Loop.next must be held to the selected recurrent input");
    assert_eq!(
        diagnostic
            .fields()
            .iter()
            .find(|(name, _)| *name == "expected")
            .map(|(_, value)| value.as_str()),
        Some("int32")
    );
}

#[test]
fn loop_graph_uses_manager_commands_without_synthetic_boundary_ports() {
    let source = r#"
graph g : App {
  input in: int
  output out: int
  graph refine : Loop {
input request: int
output err: union { LoopExpired { last_state: int } }
output accepted: int
output failed: int
Loop.entry = request
Loop.terminal = [failed, accepted, err]
Loop.fuel = 3

operator judge {
  input in: int
  output accepted: int
  output retry: int
  on in match {
    data > 0 => accepted = data
    _        => retry = data + 1
  }
}

request -> judge
judge.accepted -> accepted
judge.retry -> Loop.next
  }

  in -> refine.request
  refine.accepted -> out
  refine.failed -> nothing
  refine.err -> nothing
}
"#;
    let hir = check_clean(source);
    let refine = &hir.app.graphs[0];
    let spec = refine.loop_spec.as_ref().expect("the Graph Loop contract");
    assert_eq!(spec.entry.name(), "request");
    assert_eq!(
        spec.terminals
            .iter()
            .map(node::Port::name)
            .collect::<Vec<_>>(),
        ["accepted", "err", "failed"]
    );
    assert!(
        refine
            .boundary
            .iter()
            .all(|port| !matches!(port.port.name(), "next" | "expired")),
        "Loop contributes no synthetic ordinary boundary port"
    );
    assert!(refine.routes.iter().any(|route| {
        matches!(
            &route.selection,
            RouteSelection::All(destinations)
                if matches!(destinations.as_slice(), [RouteDestination::LoopCommand(target)]
                    if target.kind == LoopCommandKind::Next)
        )
    }));
    assert!(
        hir.app.edges.iter().any(|edge| {
            edge.from.port.name() == "err" && matches!(edge.to, EdgeTarget::Discard)
        })
    );
}

#[test]
fn new_loop_graph_interface_cannot_mix_with_historical_graph_handlers() {
    let source = r#"
graph g : App {
  graph refine : Loop {
input request: int
output err: union { LoopExpired { last_state: int } }
output out: int
Loop.entry = request
Loop.fuel = 3
on request { out = data }
  }
}
"#;
    let (_, bag) = check_source(source);
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == E_GRAPH_HANDLER)
    );
}

#[test]
fn loop_port_selectors_never_fail_silently() {
    let source = r#"
graph g : App {
  operator retry : Loop {
input in: int
output err: union { LoopExpired { last_state: int } }
output done: int
output failed: int
Loop.terminal = [done, 7, err]
Loop.fuel = 3
on in { Loop.done }
  }
  retry.done -> nothing
  retry.failed -> nothing
}
"#;
    let (_, bag) = check_source(source);
    assert!(bag.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == E_LOOP_TERMINAL
            && diagnostic.message().contains("must name one output port")
    }));
}

#[test]
fn connector_loop_attributes_do_not_enter_its_contract_sheet() {
    let source = r#"
graph g : App {
  connector call : Process + Loop {
mode = job
command = "cat"
codec = lines
idempotent = true
behavior = deterministic
Loop.fuel = 3
  }
  call.out -> nothing
  call.err -> nothing
  call.exit -> nothing
  }
"#;
    let hir = check_clean(source);
    let call = &hir.app.nodes[0];
    let spec = call.loop_spec.as_ref().expect("the checked Connector Loop");
    assert_eq!(spec.entry.name(), "in");
    assert!(spec.terminals.is_empty());
    assert!(
        call.attributes()
            .iter()
            .all(|attribute| !attribute.name.starts_with("Loop."))
    );
}

#[test]
fn inbound_and_open_ended_connectors_are_not_loop_hosts() {
    let source = r#"
graph g : App {
  connector server : HttpServer + Loop {
bind = "127.0.0.1:0"
codec = json

behavior = probabilistic
Loop.fuel = 3
  }
}
"#;
    let (_, bag) = check_source(source);
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == E_LOOP_HOST),
        "{:#?}",
        bag.diagnostics()
    );
}

#[test]
fn connector_loop_routes_refuse_unbounded_verdicts() {
    let (routes, expected) = (
        "call.out -> call.Loop.done; call.exit -> nothing",
        Code::new("E-LOOP-CARDINALITY"),
    );
    let source = format!(
        r#"
graph g : App {{
  input in: text
  connector call : Process + Loop {{
    mode = job
    command = "cat"
    codec = lines
    idempotent = false
    behavior = deterministic
    Loop.fuel = 3
  }}
  in -> call
  call.err -> nothing
  {routes}
}}
"#
    );
    let (_, bag) = check_source(&source);
    assert!(
        diagnostic_codes(&bag).contains(&expected),
        "{expected} must be diagnosed before packaging: {:#?}",
        bag.diagnostics()
    );
}

#[test]
fn loop_checks_sources_without_imposing_retry_guarantees() {
    let source = r#"
graph g : App {
  connector call : Process + Loop {
    command = "cat"
    codec = lines
    idempotent = false
    behavior = deterministic
    Loop.fuel = 3
    timeout = duration("1s")
  }
  call.out -> nothing
  call.err -> nothing
  call.exit -> match { SpawnFailed -> call.Loop.again; _ -> nothing }
}
"#;
    for case in ["SpawnFailed", "Exited"] {
        let (_, bag) = check_source(&source.replace("SpawnFailed ->", &format!("{case} ->")));
        assert!(!bag.has_errors(), "{:?}", bag.diagnostics());
    }
}

#[test]
fn a_single_http_response_can_supply_one_idempotent_recurrence_command() {
    let source = r#"
graph g : App {
  input in: bytes
  connector call : HttpClient + Loop {
    url = "https://example.test"
    codec = bytes
    timeout = duration("1s")
    idempotent = true
    behavior = probabilistic
    Loop.fuel = 3
  }
  in -> call
  call.out -> call.Loop.again
  call.exit -> nothing
  }
"#;
    let hir = check_clean(source);
    let (plan, _) =
        crate::compiler::lower::lower(&hir).expect("single-response commands are transported");
    crate::plan::ValidatedExecutionPlan::try_from_lowered(plan)
        .expect("the independent proof derives one possible frame verdict");
}

#[test]
fn multi_emission_is_one_ordered_atomic_hir_statement() {
    let source = r#"
graph g : App {
  input in: int
  operator split {
input in: int
output left: int
output right: int
on in { (left, right) = data }
  }
  in -> split
  split.left -> nothing
  split.right -> nothing
}
"#;
    let hir = check_clean(source);
    let Dispatch::Linear(statements) = &hir.app.nodes[0].handlers[0].dispatch else {
        panic!("expected a linear dispatch");
    };
    let [Statement::MultiEmit { ports, values }] = statements.as_slice() else {
        panic!("expected exactly one multi-emission");
    };
    assert_eq!(
        ports.iter().map(node::Port::name).collect::<Vec<_>>(),
        ["left", "right"]
    );
    assert!(matches!(values, MultiEmitValues::Broadcast(_)));
    let (plan, _) = crate::compiler::lower::lower(&hir).unwrap();
    crate::plan::ValidatedExecutionPlan::try_from_lowered(plan).unwrap();
}

#[test]
fn multi_emission_validates_arity_duplicates_and_authored_order() {
    let source = |statement: &str| {
        format!(
            "graph g : App {{ input in: int ; operator split {{ input in: int ; output left: int ; output right: int ; output third: int ; on in {{ {statement} }} }} in -> split ; split.left -> nothing ; split.right -> nothing ; split.third -> nothing }}"
        )
    };
    for (statement, code) in [
        ("(left, right, third) = (1, 2)", E_MULTI_EMIT_ARITY),
        ("(left, left) = data", E_MULTI_EMIT_DUPLICATE),
    ] {
        let (_, bag) = check_source(&source(statement));
        assert!(
            diagnostic_codes(&bag).contains(&code),
            "{:#?}",
            bag.diagnostics()
        );
    }
    let left = check_clean(&source("(left, right) = (data, data + 1)"));
    let right = check_clean(&source("(right, left) = (data + 1, data)"));
    assert_ne!(
        crate::compiler::lower::semantic_hash(&left),
        crate::compiler::lower::semantic_hash(&right),
        "even equivalent values on each port retain authored emission order"
    );
}

#[test]
fn graph_and_connector_reject_dsl_state_by_name() {
    for (source, code) in [
        (
            "graph g : App { var count: int = 0 }",
            Code::new("E-GRAPH-STATE"),
        ),
        (
            "graph g : App { connector process : Process { command = \"cat\" ; idempotent = true; behavior = deterministic ; var count: int = 0 } }",
            Code::new("E-CONNECTOR-STATE"),
        ),
    ] {
        let (_, bag) = check_source(source);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == code)
            .unwrap_or_else(|| panic!("missing {code:?}: {:#?}", bag.diagnostics()));
        assert!(diagnostic.message().contains("Operator"));
        assert!(
            diagnostic
                .fields()
                .iter()
                .any(|(key, value)| *key == "name" && value == "count")
        );
    }
}

#[test]
fn removed_execution_and_wiring_forms_name_their_replacement() {
    for (source, code, hint) in [
        (
            "graph g : App { on in { out = . } }",
            E_GRAPH_HANDLER,
            "Operator",
        ),
        (
            "graph g : App { connector p : Process { command = \"cat\" ; idempotent = true; behavior = deterministic ; on in { out = data } } }",
            E_CONNECTOR_HANDLER,
            "Contract",
        ),
        (
            "graph g : App { input in: int ; operator o { on in { nothing } } in -> o when data > 0 }",
            E_WIRING_MIGRATION,
            "Operator",
        ),
    ] {
        let (_, bag) = check_source(source);
        assert!(
            bag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == code && diagnostic.message().contains(hint)),
            "{:#?}",
            bag.diagnostics()
        );
    }
}

#[test]
fn lifecycle_handlers_enter_the_general_handler_table() {
    let source = "graph g : App {\n  operator o {\n    on start { out = . }\n    on in { out = . }\n  }\n  in -> o -> out\n}\n";
    let hir = check_clean(source);
    assert!(hir.app.nodes[0].handlers.iter().any(|handler| {
        handler.activation == node::HandlerActivation::Lifecycle(node::Lifecycle::Start)
    }));
}

#[test]
fn an_admission_lifecycle_requires_a_causal_input() {
    let source = "graph g : App {\n  operator source {\n    output out: int\n    on start { out = 1 }\n  }\n  source -> out\n}\n";
    let (_, bag) = check_source(source);
    assert!(bag.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == E_INSTANCE
            && diagnostic.message().contains("requires an input envelope")
    }));
}

#[test]
fn unrepresentable_emissions_refuse_at_check_not_at_runtime() {
    let source = "graph g : App {\n  operator o {\n    on in { out = duration(\"3s\") }\n  }\n  in -> o -> out\n}\n";
    let (_, bag) = check_source(source);
    assert!(
        bag.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == E_CHECK_DEFERRED
                && diagnostic.fields()[0].1 == "a duration, timestamp, or null emission"
        }),
        "an accepted plan must not fault on a normal emission: {:#?}",
        bag.diagnostics()
    );
}

#[test]
fn the_jsonl_codec_compiles() {
    // M1 item 2 (D-2026-08-11-1): the JSON chain exists — the
    // deferral lifted with it.
    let source = "graph g : App {\n  connector c : Process {\n    mode = job ; command = \"cat\"\n    codec = jsonl\n    idempotent = true; behavior = deterministic\n  }\n  in -> c -> out\n}\n";
    let (_, bag) = check_source(source);
    assert!(
        !bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == E_CHECK_DEFERRED),
        "{:#?}",
        bag.diagnostics()
    );
}

#[test]
fn unknown_and_unproduced_expectation_targets_refuse() {
    // A typo or an unproduced conventional name cannot create an
    // observation target; predicates still owe a boolean.
    let base = "graph g : App {\n  operator relay {\n    on in { out = . }\n  }\n  in -> relay -> out\n}\n\nlab g {\n  simulation s on app {\n    scenario sc {\n      app.in = { text = \"x\" }\n      EXPECT\n    }\n  }\n}\n";
    let cases = [
        ("expect nothing relay.typo", E_PORT_UNKNOWN),
        ("expect nothing app.typo", E_PORT_UNKNOWN),
        ("expect nothing relay.err", E_PORT_UNKNOWN),
        ("expect nothing app.err", E_PORT_UNKNOWN),
        ("expect nothing app.out where 42", E_EXPR),
    ];
    for (expect, code) in cases {
        let source = base.replace("EXPECT", expect);
        let (_, bag) = check_source(&source);
        assert!(
            bag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == code),
            "`{expect}` must refuse with {code:?}: {:#?}",
            bag.diagnostics()
        );
    }
}

#[test]
fn lab_fragments_merge_and_homonym_simulations_refuse() {
    let source = "graph g : App {\n  operator o {\n    on in { out = . }\n  }\n  in -> o -> out\n}\n\nlab g {\n  simulation a on app { }\n}\n\nlab g {\n  simulation b on app { }\n}\n\nlab g {\n  simulation a on app { }\n}\n";
    let (hir, bag) = check_source(source);
    // Fragments merged into one corpus of two simulations…
    let hir = hir.expect("the unit checks");
    assert_eq!(hir.labs.len(), 1);
    assert_eq!(hir.labs[0].simulations.len(), 2);
    // …and the homonym refused by name.
    assert!(bag.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == E_DUP_LAB_IDENTITY && diagnostic.fields()[0].1 == "a"
    }));
}

#[test]
fn an_inferred_output_also_requires_a_destination() {
    let source = "graph g : App {\n  operator o {\n    on in { out = . }\n  }\n  in -> o\n}\n";
    let (_, bag) = check_source(source);
    assert!(
        bag.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == E_UNWIRED_OUTPUT && diagnostic.to_string().contains("o.out")
        }),
        "{:#?}",
        bag.diagnostics()
    );
}

#[test]
fn a_duplicated_edge_refuses_whatever_its_spelling() {
    // Textually identical, and sugar-vs-explicit: both resolve to
    // one endpoint pair and the second refuses with its own span.
    for second in ["o -> p", "o.out -> p.in"] {
        let source = format!(
            "graph g : App {{\n  operator o {{\n    on in {{ out = . }}\n  }}\n  operator p {{\n    on in {{ out = . }}\n  }}\n  in -> o -> p -> out\n  {second}\n}}\n"
        );
        let (_, bag) = check_source(&source);
        assert!(
            bag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == E_ROUTE_DUPLICATE),
            "`{second}` must refuse: {:#?}",
            bag.diagnostics()
        );
    }
}

#[test]
fn duplicate_mocks_refuse_at_both_levels() {
    let base = "graph g : App {\n  connector shell : Process {\n    mode = job ; command = \"cat\"\n    codec = lines\n    idempotent = true; behavior = deterministic\n  }\n  operator o {\n    input in: struct { text: text }\n    on in { out = data.text }\n  }\n  in -> o -> shell -> out\n}\n\nlab g {\n  simulation s on app {\nBODY  }\n}\n";
    let simulation_level = base.replace(
        "BODY",
        "    mock shell { on in { out = . } }\n    mock shell { on in { out = . } }\n",
    );
    let scenario_level = base.replace(
        "BODY",
        "    scenario x {\n      mock shell { on in { out = . } }\n      mock shell { on in { out = . } }\n      app.in = \"y\"\n    }\n",
    );
    for source in [simulation_level, scenario_level] {
        let (_, bag) = check_source(&source);
        assert!(
            bag.diagnostics().iter().any(|diagnostic| {
                diagnostic.code() == E_DUP_LAB_IDENTITY && diagnostic.fields()[0].1 == "shell"
            }),
            "the duplicate mock must refuse: {:#?}",
            bag.diagnostics()
        );
    }
}

#[test]
fn no_top_level_declaration_is_silently_ignored() {
    // Until unit composition and real liveness exist, an extra
    // top-level graph is an explicit deferral — an ERROR, so its
    // unchecked content can never ride along silently.
    let source = "graph main : App { }\ngraph orphan { }\nval loose = 1\n";
    let (_, bag) = check_source(source);
    assert!(bag.has_errors());
    let constructs: Vec<&str> = bag
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code() == E_CHECK_DEFERRED)
        .map(|diagnostic| diagnostic.fields()[0].1.as_str())
        .collect();
    assert!(constructs.contains(&"a top-level graph outside the App"));
    assert_eq!(constructs, ["a top-level graph outside the App"]);
}

#[test]
fn recognized_data_annotations_reach_the_annotated_port_schema() {
    let source = "type Prompt = text @sensitive(level = confidential)\ngraph g : App {\n  operator o {\n    input in: Prompt\n    on in { out = . }\n  }\n  in -> o -> out\n}\n";
    let (hir, bag) = check_source(source);
    assert!(!bag.has_errors(), "v15 carries valid policy metadata");
    let hir = hir.expect("the annotation reaches checked HIR");
    assert_eq!(hir.named_schemas.len(), 1);
    assert!(!hir.named_schemas[0].1.annotations().is_empty());
    assert!(
        !hir.app.nodes[0]
            .ports
            .iter()
            .find(|decl| decl.port.name() == "in")
            .expect("input port")
            .annotations
            .is_empty()
    );
}

#[test]
fn aliases_expand_annotations_to_exact_collection_and_union_paths() {
    let source = r#"
type TokenCount = int @measure(id = input_tokens, unit = "token")
type Result = union {
  Done @sensitive(level = internal) {
counts: array<TokenCount>
  },
  Empty
}
graph g : App {
  operator o {
input in: Result
on in { out = . }
  }
  in -> o -> out
}
"#;
    let (hir, bag) = check_source(source);
    assert!(!bag.has_errors());
    let hir = hir.expect("the annotated schema is checked into HIR");
    let result = hir
        .named_schemas
        .iter()
        .find(|(name, _)| name == "Result")
        .map(|(_, schema)| schema)
        .expect("Result schema");
    let paths: Vec<String> = result
        .annotations()
        .entries()
        .keys()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        paths,
        [
            "$.tagged_union_case(\"Done\")",
            "$.tagged_union_case(\"Done\").field(\"counts\").array_element",
        ]
    );
    let port_paths: Vec<String> = hir.app.nodes[0]
        .ports
        .iter()
        .find(|decl| decl.port.name() == "in")
        .expect("input port")
        .annotations
        .entries()
        .keys()
        .map(ToString::to_string)
        .collect();
    assert_eq!(port_paths, paths, "the alias is expanded at its port use");
}

#[test]
fn annotated_aliases_keep_the_canonical_structural_union_branch() {
    let source = r#"
type Classified = text @sensitive(level = confidential)
type Choice = Classified | int
graph g : App {
  operator o {
input in: Choice
on in { out = . }
  }
  in -> o -> out
}
"#;
    let (hir, bag) = check_source(source);
    assert!(!bag.has_errors());
    let hir = hir.expect("the structural-union annotation reaches checked HIR");
    let choice = hir
        .named_schemas
        .iter()
        .find(|(name, _)| name == "Choice")
        .map(|(_, schema)| schema)
        .expect("Choice schema");
    let paths: Vec<String> = choice
        .annotations()
        .entries()
        .keys()
        .map(ToString::to_string)
        .collect();
    assert_eq!(paths, ["$.structural_union_branch(1)"]);
    assert_eq!(
        hir.app.nodes[0]
            .ports
            .iter()
            .find(|decl| decl.port.name() == "in")
            .expect("input port")
            .annotations
            .entries()
            .keys()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        paths,
        "the canonical branch survives alias use at a typed port"
    );
}

#[test]
fn measure_secret_and_personal_contracts_reach_hir_without_changing_types() {
    let source = r#"
type ScalarCost = decimal(38,18) @measure(id = amount, unit = "EUR")
type Money = struct { amount: ScalarCost, currency: text }
type Credential = text @sensitive(level = restricted) @secret
type PersonalFact = text @personal
type SubjectId = text @personal(identifier = true)
type HealthFact = text @personal(special = true, criminal = false)
graph g : App {}
"#;
    let (hir, bag) = check_source(source);
    assert!(!bag.has_errors());
    let hir = hir.expect("the checked schemas survive resolution");
    let scalar = hir
        .named_schemas
        .iter()
        .find(|(name, _)| name == "ScalarCost")
        .map(|(_, schema)| schema)
        .expect("ScalarCost schema");
    assert!(matches!(scalar.ty(), DataType::Decimal(_)));
    let scalar_annotations = scalar
        .annotations()
        .entries()
        .get(&SchemaPath::root())
        .expect("root scalar annotations")
        .annotations();
    assert!(matches!(
        scalar_annotations,
        [DataAnnotation::Declared(application)]
            if application.quantity_measure() == Some(("amount", "EUR"))
    ));

    let money = hir
        .named_schemas
        .iter()
        .find(|(name, _)| name == "Money")
        .map(|(_, schema)| schema)
        .expect("Money schema");
    assert!(matches!(money.ty(), DataType::Struct(_)));
    assert!(matches!(
        money.annotations()
            .entries()
            .get(&SchemaPath::root().child(SchemaPathSegment::Field("amount".into())))
            .expect("amount field annotations")
            .annotations(),
        [DataAnnotation::Declared(application)]
            if application.quantity_measure() == Some(("amount", "EUR"))
    ));

    let credential = hir
        .named_schemas
        .iter()
        .find(|(name, _)| name == "Credential")
        .map(|(_, schema)| schema)
        .expect("Credential schema");
    assert_eq!(credential.ty(), &DataType::Text);
    assert_eq!(
        credential
            .annotations()
            .entries()
            .get(&SchemaPath::root())
            .expect("root credential annotations")
            .annotations(),
        [
            DataAnnotation::Sensitive(SensitivityLevel::Restricted),
            DataAnnotation::Secret,
        ]
    );

    let subject = hir
        .named_schemas
        .iter()
        .find(|(name, _)| name == "SubjectId")
        .map(|(_, schema)| schema)
        .expect("SubjectId schema");
    assert_eq!(subject.ty(), &DataType::Text);
    assert!(matches!(
        subject
            .annotations()
            .entries()
            .get(&SchemaPath::root())
            .map(AnnotationSet::annotations),
        Some([DataAnnotation::Personal(class)]) if class.is_identifier()
            && !class.is_special_category()
            && !class.is_criminal_offence()
    ));

    let personal = hir
        .named_schemas
        .iter()
        .find(|(name, _)| name == "PersonalFact")
        .map(|(_, schema)| schema)
        .expect("PersonalFact schema");
    assert!(matches!(
        personal
            .annotations()
            .entries()
            .get(&SchemaPath::root())
            .map(AnnotationSet::annotations),
        Some([DataAnnotation::Personal(class)]) if *class == PersonalDataClass::default()
    ));

    let health = hir
        .named_schemas
        .iter()
        .find(|(name, _)| name == "HealthFact")
        .map(|(_, schema)| schema)
        .expect("HealthFact schema");
    assert!(matches!(
        health
            .annotations()
            .entries()
            .get(&SchemaPath::root())
            .map(AnnotationSet::annotations),
        Some([DataAnnotation::Personal(class)]) if class.is_special_category()
            && !class.is_criminal_offence()
    ));
}

#[test]
fn boot_references_close_only_at_declared_connector_settings() {
    let source = r#"
graph app : App {
  connector client : HttpClient {
url = env("SERVICE_URL")
auth_bearer = secret("providers/openai/token")
method = "POST"
timeout = duration("2s")
codec = json
idempotent = true
behavior = deterministic
  }
  client.out -> nothing
  client.exit -> nothing
}
"#;
    let (hir, bag) = check_source(source);
    assert!(!bag.has_errors(), "{:#?}", bag.diagnostics());
    let hir = hir.expect("boot references check into HIR");
    let client = &hir.app.nodes[0];
    let attributes = client.attributes();
    for (name, expected) in [
        (
            "url",
            crate::deployment::BootReference::Environment(
                crate::deployment::EnvironmentName::new("SERVICE_URL")
                    .expect("portable environment name"),
            ),
        ),
        (
            "auth_bearer",
            crate::deployment::BootReference::Secret(
                crate::deployment::SecretName::new("providers/openai/token")
                    .expect("logical secret ref"),
            ),
        ),
    ] {
        let attribute = attributes
            .iter()
            .find(|attribute| attribute.name == name)
            .expect("boot-bound attribute");
        assert!(matches!(
            &attribute.value,
            AttrValue::Expr { site: crate::plan::EvaluationSite::Boot, tree }
                if crate::expr::boot_reference(&tree.root) == Ok(Some(expected.clone()))
        ));
    }

    for (attribute, value, detail) in [
        (
            "idempotency_key",
            "secret(\"providers/openai/token\")",
            "not authorized",
        ),
        ("auth_bearer", "env(\"API_TOKEN\")", "not authorized"),
        (
            "auth_bearer",
            "secret(\"providers/openai/token\") + \"leak\"",
            "exactly one string literal",
        ),
    ] {
        let source = format!(
            r#"graph app : App {{
  connector client : HttpClient {{
url = "https://example.test"
method = "GET"
timeout = duration("2s")
codec = json
idempotent = true
behavior = deterministic
{attribute} = {value}
  }}
}}
"#
        );
        let (_, bag) = check_source(&source);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == E_CONTRACT_ATTR)
            .expect("boot reference refusal");
        assert!(diagnostic.message().contains(detail), "{diagnostic:#?}");
    }
}

#[test]
fn explicit_binding_types_retain_their_annotation_tables_in_hir() {
    let source = r#"
graph g : App {
  val label: text @sensitive(level = internal) = "demo"
  operator o {
var credential: text @sensitive(level = restricted) = "placeholder"
on in {
  val tokens: int @measure(id = output_tokens, unit = "token") = 1
  out = .
}
  }
  in -> o -> out
}
"#;
    let (hir, bag) = check_source(source);
    assert!(!bag.has_errors());
    let hir = hir.expect("binding annotations reach checked HIR");
    assert!(!hir.app.vals[0].annotations.is_empty());
    assert!(!hir.app.nodes[0].vars[0].annotations.is_empty());
    assert!(matches!(
        &handler_body(&hir.app.nodes[0].handlers[0])[0],
        Statement::Let { annotations, .. } if !annotations.is_empty()
    ));
}

#[test]
fn executable_secret_schemas_refuse_until_fields_can_be_externalized() {
    let cases = [
        "type Secret = text @secret\ngraph g : App { input in: Secret }\n",
        "type Secret = text @secret\ngraph g : App { operator o { input in: Secret } }\n",
        "type Secret = text @secret\ngraph g : App { val token: Secret = \"x\" }\n",
        "type Secret = text @secret\ngraph g : App { operator o { var token: Secret = \"x\" } }\n",
        "type Secret = text @secret\ngraph g : App { operator o { on in { val token: Secret = \"x\"; out = . } } in -> o -> out }\n",
    ];
    for source in cases {
        let (_, bag) = check_source(source);
        let refusals = bag
            .diagnostics()
            .iter()
            .filter(|diagnostic| {
                diagnostic.code() == E_CHECK_DEFERRED
                    && diagnostic.message().contains("field-level externalization")
            })
            .count();
        assert_eq!(refusals, 1, "{source}\n{:#?}", bag.diagnostics());
    }
}

#[test]
fn boot_reference_functions_name_their_restricted_positions() {
    for (function, setting) in [
        (
            "secret(\"providers/openai/token\")",
            "HttpClient.auth_bearer",
        ),
        ("env(\"SERVICE_URL\")", "HttpClient.url"),
    ] {
        let source = format!(
            "graph g : App {{ operator o {{ on in {{ out = {function} }} }} in -> o -> out }}\n"
        );
        let (_, bag) = check_source(&source);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == E_EXPR)
            .expect("the reserved boot function refuses as an expression");
        assert!(
            diagnostic.message().contains("admitted only")
                && diagnostic.message().contains(setting),
            "{diagnostic:#?}"
        );
        assert!(!diagnostic.message().contains("unknown function"));
    }
}

#[test]
fn invalid_data_annotations_refuse_by_annotation_and_exact_path() {
    let cases = [
        (
            "type X = struct { field: text @unknown }\ngraph g : App {}\n",
            "unknown",
            "$.field(\"field\")",
            "unknown data annotation",
        ),
        (
            "type X = array<text @secret @secret>\ngraph g : App {}\n",
            "secret",
            "$.array_element",
            "appears twice",
        ),
        (
            "type X = int @measure(id = data)\ngraph g : App {}\n",
            "measure",
            "$",
            "missing required argument `unit`",
        ),
        (
            "type X = int @measure(id = env(\"MEASURE\"), unit = \"item\")\ngraph g : App {}\n",
            "measure",
            "$",
            "symbolic plan constant",
        ),
        (
            "type X = struct { prompt: text @measure(id = tokens, unit = \"token\") }\ngraph g : App {}\n",
            "measure",
            "$.field(\"prompt\")",
            "requires an integer or exact decimal",
        ),
        (
            "type X = union { Done @measure(id = amount, unit = \"USD\") }\ngraph g : App {}\n",
            "measure",
            "$.tagged_union_case(\"Done\")",
            "integer or exact decimal",
        ),
        (
            "type X = text @sensitive(level = internal, level = restricted)\ngraph g : App {}\n",
            "sensitive",
            "$",
            "argument `level` appears twice",
        ),
        (
            "type X = text @sensitive\ngraph g : App {}\n",
            "sensitive",
            "$",
            "missing required argument `level`",
        ),
        (
            "type X = text @secret(level = restricted)\ngraph g : App {}\n",
            "secret",
            "$",
            "unexpected argument `level`",
        ),
        (
            "type X = int @measure(id = http.client.request.duration)\ngraph g : App {}\n",
            "measure",
            "$",
            "missing required argument `unit`",
        ),
        (
            "type X = decimal(38,18) @measure(id = amount, currency = ZZZ, basis = provider_reported)\ngraph g : App {}\n",
            "measure",
            "$",
            "id and unit",
        ),
        (
            "type X = struct { amount: decimal(38,18), currency: text } @measure(id = amount, currency = EUR, basis = provider_reported)\ngraph g : App {}\n",
            "measure",
            "$",
            "id and unit",
        ),
        (
            "type X = text @personal(identifier = maybe)\ngraph g : App {}\n",
            "personal",
            "$",
            "must be `true` or `false`",
        ),
        (
            "type X = text @personal(purpose = support)\ngraph g : App {}\n",
            "personal",
            "$",
            "unexpected argument `purpose`",
        ),
    ];
    for (source, annotation, path, detail) in cases {
        let (_, bag) = check_source(source);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == E_DATA_ANNOTATION)
            .unwrap_or_else(|| panic!("missing annotation diagnostic: {:#?}", bag.diagnostics()));
        assert_eq!(diagnostic.fields()[0].1, annotation);
        assert_eq!(diagnostic.fields()[1].1, path);
        assert!(
            diagnostic.fields()[2].1.contains(detail),
            "{source}: {diagnostic:#?}"
        );
        assert!(
            bag.diagnostics()
                .iter()
                .all(|diagnostic| diagnostic.code() != E_CHECK_DEFERRED),
            "an invalid annotation must not also claim to have reached A2"
        );
    }
}

#[test]
fn one_faulty_alias_annotation_speaks_once_across_all_uses() {
    let source = r#"
type Bad = text @unknown
graph g : App {
  operator o {
input left: Bad
input right: Bad
  }
}
"#;
    let (_, bag) = check_source(source);
    let annotation_diagnostics: Vec<_> = bag
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code() == E_DATA_ANNOTATION)
        .collect();
    assert_eq!(annotation_diagnostics.len(), 1, "{:#?}", bag.diagnostics());
    assert_eq!(annotation_diagnostics[0].fields()[0].1, "unknown");
    assert_eq!(annotation_diagnostics[0].fields()[1].1, "$");
}

#[test]
fn a_var_initializer_must_fit_its_declared_type() {
    let source = "graph g : App {\n  operator o {\n    var count : int = \"oops\"\n    on in { out = . }\n  }\n  in -> o -> out\n}\n";
    let (_, bag) = check_source(source);
    let diagnostic = bag
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code() == E_PORT_TYPE)
        .expect("the type mismatch");
    assert!(diagnostic.message().contains("requires int32"));
}

#[test]
fn shadowing_and_reserved_names_are_refused() {
    let source = "graph g : App {\n  val limit = 1\n  operator o {\n    var limit : int = 0\n    var data : int = 0\n    on in { out = . }\n  }\n  in -> o -> out\n}\n";
    let (_, bag) = check_source(source);
    let shadows: Vec<&str> = bag
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code() == E_DUP_BINDING)
        .map(|diagnostic| diagnostic.fields()[0].1.as_str())
        .collect();
    assert_eq!(shadows, ["limit", "data"]);
}

#[test]
fn handler_and_emission_ports_are_inferred() {
    // The reviewer's reproduction: `on event` implies an input,
    // `result = .` implies an output — no declaration
    // needed. This program is conforming and must check cleanly.
    let source = "graph g : App {\n  input feed\n  output sink\n\n  operator o {\n    on event {\n      result = .\n    }\n  }\n\n  feed -> o.event\n  o.result -> sink\n}\n";
    let hir = check_clean(source);
    let ports = &hir.app.nodes[0].ports;
    assert!(
        ports
            .iter()
            .any(|decl| decl.port.name() == "event" && decl.direction == PortDirection::Input)
    );
    assert!(
        ports
            .iter()
            .any(|decl| decl.port.name() == "result" && decl.direction == PortDirection::Output)
    );
}

#[test]
fn conflicting_port_roles_are_refused() {
    // The same name inferred as input and output.
    let source = "graph g : App {\n  operator o {\n    on chan {\n      chan = .\n    }\n  }\n  in -> o.chan -> out\n}\n";
    let (_, bag) = check_source(source);
    assert!(diagnostic_codes(&bag).contains(&E_PORT_CONFLICT));
    // Explicit double declaration at the boundary.
    let source = "graph g : App {\n  input feed\n  output feed\n  operator o { on in { out = . } }\n  in -> o -> out\n}\n";
    let (_, bag) = check_source(source);
    assert!(diagnostic_codes(&bag).contains(&E_PORT_CONFLICT));
}

#[test]
fn a_connector_cannot_add_input_ports() {
    let source = "graph g : App {\n  connector c : Process {\n    input extra\n    command = \"cat\" ; codec = lines\n    idempotent = true; behavior = deterministic\n  }\n  in -> c -> out\n}\n";
    let (_, bag) = check_source(source);
    let diagnostic = bag
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code() == E_PORT_CONFLICT)
        .expect("the connector port refusal");
    assert!(
        diagnostic
            .message()
            .contains("input `extra` belongs to the Contract")
    );
}

#[test]
fn physical_integer_types_are_not_silently_merged() {
    let long = "graph g : App {\n  operator o {\n    var n : long = 0\n    on in { out = . }\n  }\n  in -> o -> out\n}\n";
    let (hir, bag) = check_source(long);
    assert!(
        !bag.has_errors(),
        "long is now canonical: {:#?}",
        bag.diagnostics()
    );
    assert_eq!(
        hir.expect("one App").app.nodes[0].vars[0].ty,
        DataType::LONG
    );
    let overflow = "graph g : App {\n  operator o {\n    var n : int = 2147483648\n    on in { out = . }\n  }\n  in -> o -> out\n}\n";
    let (_, bag) = check_source(overflow);
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == E_PORT_TYPE
                && diagnostic.message().contains("requires int32"))
    );
}

#[test]
fn ports_are_used_with_their_direction() {
    // `o.in` as an edge source, a handler on `out`, and an
    // emission into `in` all refuse.
    let source = "graph g : App {\n  operator o {\n    on out { in = . }\n  }\n  o.in -> o\n}\n";
    let (_, bag) = check_source(source);
    let directions = bag
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code() == E_PORT_DIRECTION)
        .count();
    assert_eq!(
        directions,
        2,
        "handler + edge source: {:#?}",
        bag.diagnostics()
    );
    let emission =
        "graph g : App {\n  operator o {\n    on in { in = . }\n  }\n  in -> o -> out\n}\n";
    let (_, bag) = check_source(emission);
    assert!(diagnostic_codes(&bag).contains(&E_PORT_DIRECTION));
}

#[test]
fn boundary_ports_carry_direction() {
    let source =
        "graph g : App {\n  input feed\n  operator o { on in { out = . } }\n  o.out -> feed\n}\n";
    let (_, bag) = check_source(source);
    let diagnostic = bag
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code() == E_PORT_DIRECTION)
        .expect("an input boundary port cannot drain the graph");
    assert!(
        diagnostic
            .message()
            .contains("its output port is a destination")
    );
}

#[test]
fn wiring_cycles_are_refused() {
    let source = "graph g : App {\n  operator a { on in { out = . } }\n  operator b { on in { out = . } }\n  a -> b\n  b -> a\n}\n";
    let (_, bag) = check_source(source);
    let diagnostic = bag
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code() == E_GRAPH_CYCLE)
        .expect("the cycle must refuse");
    assert!(diagnostic.message().contains("DAG"));
    let chain = "graph g : App {\n  operator a { on in { out = . } }\n  operator b { on in { out = . } }\n  in -> a -> b -> out\n}\n";
    let (_, bag) = check_source(chain);
    assert!(!diagnostic_codes(&bag).contains(&E_GRAPH_CYCLE));

    let session = r#"
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
    let (_, bag) = check_source(session);
    assert!(
        !diagnostic_codes(&bag).contains(&E_GRAPH_CYCLE),
        "the inbound and outbound halves of one Session are a causal boundary: {:#?}",
        bag.diagnostics()
    );
}

#[test]
fn repeated_attributes_and_port_declarations_are_refused() {
    let repeated_attr = "graph g : App {\n  connector c : Process {\n    command = \"cat\" ; command = \"false\"\n    codec = lines\n    idempotent = true; behavior = deterministic\n  }\n  in -> c -> out\n}\n";
    let (_, bag) = check_source(repeated_attr);
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == E_CONTRACT_ATTR
                && diagnostic.message().contains("twice"))
    );
    let repeated_port = "graph g : App {\n  operator o {\n    input event\n    input event\n    on event { out = . }\n  }\n  feed -> o.event\n  input feed\n  o -> out\n}\n";
    let (_, bag) = check_source(repeated_port);
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == E_PORT_CONFLICT
                && diagnostic.message().contains("already declared"))
    );
}

#[test]
fn key_and_lifetime_are_orthogonal_in_all_four_combinations() {
    let policies = [
        ("", false, "persistent"),
        ("instance = { key = data }", true, "persistent"),
        (
            "lifetime = { kind = idle ; after = duration(\"30s\") }",
            false,
            "idle",
        ),
        (
            "instance = { key = data }\n    lifetime = { kind = idle ; after = duration(\"30s\") }",
            true,
            "idle",
        ),
    ];
    for (attributes, keyed, lifetime) in policies {
        let source = format!(
            "graph g : App {{\n  input in: text\n  output out: text\n  operator relay {{\n    input in: text\n    output out: text\n    {attributes}\n    on in {{ out = . }}\n  }}\n  in -> relay -> out\n}}\n"
        );
        let hir = check_clean(&source);
        let policy = &hir.app.nodes[0].instance;
        if attributes.is_empty() {
            assert!(policy.is_none(), "omission is the persistent singleton");
            continue;
        }
        let policy = policy.as_ref().expect("a non-default policy is retained");
        assert_eq!(policy.key.is_some(), keyed);
        match (&policy.lifetime, lifetime) {
            (InstanceLifetime::Persistent, "persistent") => {}
            (InstanceLifetime::Idle(value), "idle") => {
                assert_eq!(value.value, crate::value::Value::Duration(30_000_000_000))
            }
            (actual, expected) => panic!("expected {expected}, got {actual:?}"),
        }
    }
}

#[test]
fn omitted_and_explicit_persistent_lifetimes_share_one_identity() {
    let source = |attribute: &str| {
        format!(
            "graph g : App {{\n  operator relay {{\n    {attribute}\n    on in {{ out = . }}\n  }}\n  in -> relay -> out\n}}\n"
        )
    };
    let omitted = check_clean(&source(""));
    let explicit = check_clean(&source("lifetime = { kind = persistent }"));
    assert_eq!(
        crate::compiler::lower::semantic_hash(&omitted),
        crate::compiler::lower::semantic_hash(&explicit)
    );
    assert_eq!(omitted.app.nodes[0].instance, None);
    assert_eq!(explicit.app.nodes[0].instance, None);
}

#[test]
fn every_executor_kind_uses_the_same_instance_sheet() {
    let source = r#"
graph g : App {
  input in: text
  output out: text
  instance = { key = data }

  connector shell : Process {
instance = { key = data }
lifetime = { kind = idle ; after = duration("5m") }
command = "cat"
codec = lines
idempotent = true
behavior = deterministic
  }
  in -> shell -> out
  shell.err -> nothing
  shell.exit -> nothing
}
"#;
    let hir = check_clean(source);
    let graph = hir.app.instance.as_ref().expect("the Graph owns a policy");
    assert!(graph.key.is_some());
    assert_eq!(graph.lifetime, InstanceLifetime::Persistent);
    let connector = hir.app.nodes[0]
        .instance
        .as_ref()
        .expect("the Connector owns the same sheet");
    assert!(connector.key.is_some());
    assert!(matches!(connector.lifetime, InstanceLifetime::Idle(_)));
}

#[test]
fn every_r4_lifetime_reaches_hir_with_its_duration() {
    enum ExpectedLifetime {
        Fixed,
        SinglePass,
        Tumbling,
    }

    let cases = [
        (
            "lifetime = { kind = fixed ; after = duration(\"30s\") }",
            ExpectedLifetime::Fixed,
        ),
        (
            "lifetime = { kind = single_pass }",
            ExpectedLifetime::SinglePass,
        ),
        (
            "lifetime = { kind = tumbling ; every = duration(\"5m\") }",
            ExpectedLifetime::Tumbling,
        ),
    ];
    for (attribute, expected) in cases {
        let source = format!(
            "graph g : App {{\n  input in: text\n  output out: text\n  operator relay {{\n    {attribute}\n    on in {{ out = . }}\n  }}\n  in -> relay -> out\n}}\n"
        );
        let hir = check_clean(&source);
        let lifetime = &hir.app.nodes[0]
            .instance
            .as_ref()
            .expect("the explicit policy reaches HIR")
            .lifetime;
        assert!(match expected {
            ExpectedLifetime::Fixed => matches!(lifetime, InstanceLifetime::Fixed(_)),
            ExpectedLifetime::SinglePass => {
                matches!(lifetime, InstanceLifetime::SinglePass)
            }
            ExpectedLifetime::Tumbling => {
                matches!(lifetime, InstanceLifetime::Tumbling(_))
            }
        });
    }
}

#[test]
fn closing_is_one_explicit_policy_and_stop_closes_the_current_instance() {
    let source = r#"
graph g : App {
  input in: int
  output out: int
  operator relay {
input in: int
output out: int
closing = { drain_for = duration("2s") ; settle_for = duration("3s") }
on in { out = . ; stop }
  }
  in -> relay -> out
}
"#;
    let hir = check_clean(source);
    let policy = hir.app.nodes[0]
        .instance
        .as_ref()
        .expect("the explicit closing policy is retained");
    let closing = policy.closing.as_ref().expect("Closing is explicit");
    assert_eq!(
        closing.drain_for.value,
        crate::value::Value::Duration(2_000_000_000)
    );
    assert_eq!(
        closing.settle_for.value,
        crate::value::Value::Duration(3_000_000_000)
    );
    assert!(matches!(
        handler_body(&hir.app.nodes[0].handlers[0]).last(),
        Some(Statement::Stop)
    ));

    let (_, targeted) = check_source(&source.replace("stop }", "stop other }"));
    assert!(targeted.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == E_INSTANCE
            && diagnostic.message().contains("targeted `stop` was removed")
    }));
    let recursive = source.replace("on in { out = . ; stop }", "on stop { stop }");
    let (_, recursive) = check_source(&recursive);
    assert!(
        recursive.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == E_INSTANCE
                && diagnostic
                    .message()
                    .contains("cannot request closure again")
        }),
        "got {:#?}",
        recursive.diagnostics()
    );
}

#[test]
fn one_key_projection_may_cross_different_input_records() {
    let source = r#"
graph g : App {
  input alpha: struct { tenant: text, amount: int }
  input beta: struct { tenant: text, note: text }
  output out: text
  operator relay {
input a: struct { tenant: text, amount: int }
input b: struct { tenant: text, note: text }
output out: text
instance = { key = data.tenant }
on a { out = data.tenant }
on b { out = data.tenant }
  }
  alpha -> relay.a
  beta -> relay.b
  relay -> out
}
"#;
    let hir = check_clean(source);
    let key = hir.app.nodes[0]
        .instance
        .as_ref()
        .and_then(|policy| policy.key.as_ref())
        .expect("the common projection is total on both ports");
    assert_eq!(key.ty, DataType::Text);
}

#[test]
fn composite_and_sys_keys_use_the_positive_surface() {
    let source = r#"
graph g : App {
  input in: text
  output out: text
  operator relay {
input in: text
output out: text
instance = { key = { request = meta.sys.id ; tenant = data } }
on in { out = . }
  }
  in -> relay -> out
}
"#;
    let hir = check_clean(source);
    let key = hir.app.nodes[0]
        .instance
        .as_ref()
        .and_then(|policy| policy.key.as_ref())
        .expect("the composite key checks");
    assert!(key.ty.is_instance_key());
    assert!(!key.ty.is_canonical_key());
}

#[test]
fn instance_sheets_refuse_malformed_values_before_lowering() {
    let cases = [
        ("instance = {}", "requires exactly one `key`"),
        ("instance = { key = 1 ; key = 2 }", "declared twice"),
        (
            "lifetime = { kind = idle ; after = duration(\"0s\") }",
            "must be positive",
        ),
        ("lifetime = { kind = mystery }", "unknown lifetime kind"),
        ("closing = {}", "requires `drain_for`"),
        (
            "closing = { drain_for = duration(\"1s\") }",
            "requires `settle_for`",
        ),
        (
            "closing = { drain_for = duration(\"0s\") ; settle_for = duration(\"1s\") }",
            "`closing.drain_for` must be positive",
        ),
        (
            "closing = { drain_for = duration(\"1s\") ; settle_for = duration(\"1s\") ; magic = 1 }",
            "has no `magic` field",
        ),
        ("instance = { key = data }", "canonical scalar/enum leaves"),
    ];
    for (attribute, expected) in cases {
        let source = format!(
            "graph g : App {{\n  input in: variant\n  output out: variant\n  operator o {{\n    input in: variant\n    output out: variant\n    {attribute}\n    on in {{ out = . }}\n  }}\n  in -> o -> out\n}}\n"
        );
        let (_, bag) = check_source(&source);
        assert!(
            bag.diagnostics().iter().any(|diagnostic| {
                diagnostic.code() == E_INSTANCE && diagnostic.message().contains(expected)
            }),
            "expected `{expected}` for {source:?}, got {:#?}",
            bag.diagnostics()
        );
    }
}

#[test]
fn superseded_instance_spellings_have_exactly_one_migration_diagnostic() {
    let sources = [
        (
            "named partition",
            "graph g : App { partition p { key = 1 ; lifetime = persistent } operator o { on in { out = . } } in -> o -> out }",
            "`partition`",
        ),
        (
            "plural instance attribute",
            "graph g : App { operator o { instances = { key = data } ; on in { out = . } } in -> o -> out }",
            "`instances` was superseded",
        ),
        (
            "Graph per",
            "graph g : App per p { operator o { on in { out = . } } in -> o -> out }",
            "`per`",
        ),
        (
            "Operator inline per",
            "graph g : App { operator o per (key = 1, lifetime = persistent) { on in { out = . } } in -> o -> out }",
            "`per`",
        ),
        (
            "Connector per",
            "graph g : App { connector p : Process per p { command = \"cat\" ; codec = lines ; idempotent = true; behavior = deterministic } in -> p -> out }",
            "`per`",
        ),
        (
            "join by",
            "type JoinExpiry = union { JoinExpired {} } graph g : App { operator j { input a: int ; input b: int ; output out: int ; output expired: JoinExpiry ; on (a, b) by a.data within duration(\"1s\") { out = a.data + b.data } } in -> j.a ; in -> j.b ; j -> out ; j.expired -> nothing }",
            "join `by`",
        ),
        (
            "targeted stop",
            "graph g : App { operator o { on in { stop p } } in -> o -> out }",
            "targeted `stop`",
        ),
    ];
    for (label, source, expected) in sources {
        let (_, bag) = check_source(source);
        assert_eq!(bag.diagnostics().len(), 1, "{label}: {bag:#?}");
        assert_eq!(bag.diagnostics()[0].code(), E_INSTANCE, "{label}");
        assert!(
            bag.diagnostics()[0].message().contains(expected),
            "{label}: {bag:#?}"
        );
    }
}
