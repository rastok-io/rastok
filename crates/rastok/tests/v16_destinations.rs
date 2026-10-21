//! A destination is a checked obligation, never an implicit discard.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::{compiler, connector::contract::ContractCatalog, plan};

fn compile(source: &str) -> Result<compiler::Compilation, rastok::diagnostics::Bag> {
    compiler::compile(&[source], &ContractCatalog::try_m0().unwrap())
}

#[test]
fn removed_boundary_spelling_refuses_without_moving_explicit_identity() {
    for (ports, old_wiring, new_wiring, hash) in [
        (
            "input in: int; output out: int",
            ". -> worker -> .",
            "in -> worker -> out",
            "61847c2351b663e8b18c83332baa9137f5877f05c2a5b827e0dddc06e373b666",
        ),
        (
            "input request: int; output answer: int",
            ".request -> worker\nworker.out -> .answer",
            "request -> worker\nworker.out -> answer",
            "87ee3cbb9ade925b9f41d796ddd232bc40eecacae0271a3b88f8821ceded1e93",
        ),
    ] {
        let source = |wiring| {
            format!(
                "graph app : App {{\n{ports}\noperator worker {{ input in: int; output out: int; on in {{ out = data }} }}\n{wiring}\n}}"
            )
        };
        let old = compile(&source(old_wiring)).unwrap_err();
        assert!(
            old.diagnostics()
                .iter()
                .all(
                    |diagnostic| diagnostic.code().as_str() == "E-WIRING-MIGRATION"
                        && diagnostic.message().contains("write")
                ),
            "{old:?}"
        );
        let explicit = compile(&source(new_wiring)).unwrap();
        assert_eq!(
            explicit.hash.to_string(),
            hash,
            "identity measured on both accepted spellings before the migration gate"
        );
        assert_eq!(
            plan::decode(&plan::encode(&explicit.plan, &explicit.hash)).unwrap(),
            (explicit.plan, explicit.hash)
        );
    }
}

#[test]
fn a_productible_operator_output_requires_a_written_destination() {
    let errors = compile(
        r#"graph app : App {
          input in: int
          operator worker { input in: int; output out: int; on in { out = data } }
          in -> worker
        }"#,
    )
    .unwrap_err();
    assert!(
        errors.diagnostics().iter().any(|error| {
            error.code().as_str() == "E-UNWIRED-OUTPUT" && error.to_string().contains("worker.out")
        }),
        "{errors:?}"
    );
}

#[test]
fn a_declared_but_unproduced_output_needs_no_fake_discard() {
    let compilation = compile(
        r#"graph app : App {
          input in: int
          operator worker {
            input in: int
            output unused: text
            on in { nothing }
          }
          in -> worker
        }"#,
    )
    .unwrap();
    assert!(compilation.warnings.iter().any(|warning| {
        warning.code().as_str() == "W-DEAD-OUTPUT" && warning.to_string().contains("worker.unused")
    }));
    assert!(compilation.warnings.iter().all(|warning| {
        warning.code().as_str() != "W-DEAD-OUTPUT" || warning.to_string().contains("worker.unused")
    }));
    let bytes = plan::encode(&compilation.plan, &compilation.hash);
    plan::decode(&bytes).unwrap();
}

fn join_expiry_source(bound: &str, disposition: &str) -> String {
    format!(
        r#"type JoinExpiry = union {{ JoinExpired {{}} }}
graph app : App {{
  input left: int
  input right: int
  output expired: JoinExpiry
  operator pair {{
    input left: int
    input right: int
    output expired: JoinExpiry
    on (left, right) {bound} {{ nothing }}
  }}
  left -> pair.left
  right -> pair.right
  {disposition}
}}"#
    )
}

#[test]
fn bounded_join_expiry_requires_an_explicit_disposition() {
    for bound in [
        "within duration(\"1s\")",
        "last 2",
        "within duration(\"1s\") last 2",
    ] {
        let errors = compile(&join_expiry_source(bound, "")).unwrap_err();
        assert!(
            errors.diagnostics().iter().any(|error| {
                error.code().as_str() == "E-UNWIRED-OUTPUT"
                    && error.to_string().contains("pair.expired")
            }),
            "{bound}: {errors:?}"
        );

        for disposition in [
            "pair.expired -> nothing",
            "pair.expired -> expired",
            "pair.expired -> match { JoinExpired -> expired }",
        ] {
            let compilation = compile(&join_expiry_source(bound, disposition)).unwrap();
            assert!(
                !compilation.warnings.iter().any(|warning| {
                    warning.code().as_str() == "W-DEAD-OUTPUT"
                        && warning.to_string().contains("pair.expired")
                }),
                "{bound}: {:?}",
                compilation.warnings
            );
            plan::decode(&plan::encode(&compilation.plan, &compilation.hash)).unwrap();
        }
    }
}

#[test]
fn an_unbounded_join_does_not_make_an_unwritten_expired_port_productible() {
    let compilation = compile(&join_expiry_source("", "")).unwrap();
    assert!(compilation.warnings.iter().any(|warning| {
        warning.code().as_str() == "W-DEAD-OUTPUT" && warning.to_string().contains("pair.expired")
    }));
    plan::decode(&plan::encode(&compilation.plan, &compilation.hash)).unwrap();
}

#[test]
fn a_forged_plan_cannot_erase_a_bounded_joins_expiry_disposition() {
    for bound in ["within duration(\"1s\")", "last 2"] {
        let compilation = compile(&join_expiry_source(bound, "pair.expired -> nothing")).unwrap();
        let mut forged = compilation.plan.into_inner();
        for node in &mut forged.nodes {
            if let plan::NodeKind::Graph(graph) = &mut node.body {
                graph
                    .wiring
                    .retain(|rule| rule.source.port.name() != "expired");
            }
        }
        let error = plan::ValidatedExecutionPlan::try_new(forged).unwrap_err();
        assert!(
            matches!(&error, plan::PlanValidationError::UnwiredOutput { node, port }
            if node.to_string() == "pair" && port.name() == "expired"),
            "{error}"
        );
    }
}

#[test]
fn a_nested_graph_cannot_hide_its_bounded_joins_expiry() {
    let source = r#"type JoinExpiry = union { JoinExpired {} }
graph app : App {
  input in: int
  graph inner {
    input left: int
    input right: int
    output expired: JoinExpiry
    operator pair {
      input left: int
      input right: int
      output expired: JoinExpiry
      on (left, right) within duration("1s") { nothing }
    }
    left -> pair.left
    right -> pair.right
    pair.expired -> expired
  }
  in -> inner.left
}"#;
    let errors = compile(source).unwrap_err();
    assert!(
        errors.diagnostics().iter().any(|error| {
            error.code().as_str() == "E-UNWIRED-OUTPUT"
                && error.to_string().contains("inner.expired")
        }),
        "{errors:?}"
    );
}

#[test]
fn bounded_join_expiry_is_observable_for_time_and_count_evictions() {
    for (bound, actions) in [
        (
            "within duration(\"1s\")",
            "app.left = 1\nadvance duration(\"1s\")",
        ),
        ("last 2", "app.left = 1\napp.left = 2\napp.left = 3"),
    ] {
        let source = format!(
            "{}\nlab app {{ simulation proof on app {{ scenario eviction {{\n{actions}\nexpect app.expired count == 1 where data is JoinExpired\n}} }} }}",
            join_expiry_source(bound, "pair.expired -> expired"),
        );
        let compilation = compile(&source).unwrap();
        let runs = rastok::lab::run(
            &compilation.plan,
            plan::GraphVersion(1),
            &compilation.labs[0],
        );
        assert_eq!(runs.len(), 1);
        assert!(
            runs[0].report.passed(),
            "{bound}: {:?}",
            runs[0].report.failures
        );
    }
}

#[test]
fn deleting_a_discard_from_a_package_cannot_turn_it_into_an_implicit_sink() {
    let compilation = compile(
        r#"graph app : App {
          input in: int
          operator worker { input in: int; output out: int; on in { out = data } }
          in -> worker
          worker.out -> nothing
        }"#,
    )
    .unwrap();
    let mut forged = compilation.plan.into_inner();
    let plan::NodeKind::Graph(graph) = &mut forged.nodes[0].body else {
        panic!("App Graph")
    };
    graph.wiring.retain(|rule| !matches!(&rule.selection, plan::RouteSelection::All(set) if set == &[plan::Destination::Discard]));
    let error = plan::ValidatedExecutionPlan::try_new(forged).unwrap_err();
    assert!(error.to_string().contains("worker.out"), "{error}");
}

#[test]
fn a_live_nested_boundary_cannot_hide_behind_another_fanout_destination() {
    let errors = compile(
        r#"graph app : App {
          input in: int
          output out: int
          graph inner {
            input in: int
            output abandoned: int
            output out: int
            operator worker { input in: int; output out: int; on in { out = data } }
            in -> worker
            worker.out -> { abandoned; out }
          }
          in -> inner
          inner.out -> out
        }"#,
    )
    .unwrap_err();
    assert!(
        errors.diagnostics().iter().any(|error| {
            error.code().as_str() == "E-UNWIRED-OUTPUT"
                && error.to_string().contains("inner.abandoned")
        }),
        "{errors:?}"
    );
}

#[test]
fn nested_dispositions_are_rederived_even_after_flattening() {
    let compilation = compile(
        r#"graph app : App {
          input in: int
          output out: int
          output auxiliary: int
          graph inner {
            input in: int
            output first: int
            output second: int
            operator worker { input in: int; output out: int; on in { out = data } }
            in -> worker
            worker.out -> { first; second }
          }
          in -> inner
          inner.first -> out
          inner.second -> auxiliary
        }"#,
    )
    .unwrap();
    let bytes = plan::encode(&compilation.plan, &compilation.hash);
    let (decoded, _) = plan::decode(&bytes).unwrap();
    assert_eq!(
        decoded.reference_routes().collect::<Vec<_>>(),
        compilation.plan.reference_routes().collect::<Vec<_>>()
    );
    assert!(decoded.reference_routes().next().is_some());

    let inner = rastok::node::NodeIndex::new(
        decoded
            .nodes
            .iter()
            .position(|node| node.path.to_string() == "inner")
            .unwrap() as u32,
    );
    let mut forged = decoded.clone().into_inner();
    let plan::NodeKind::Graph(graph) = &mut forged.nodes[0].body else {
        panic!("App Graph")
    };
    graph
        .wiring
        .retain(|rule| !(rule.source.node == inner && rule.source.port.name() == "second"));
    // Derived indexes are rebuilt from the changed reference wiring.
    let error = plan::ValidatedExecutionPlan::try_new(forged).unwrap_err();
    assert!(
        matches!(&error, plan::PlanValidationError::InvalidRouting(error)
        if error.kind == plan::RouteErrorKind::EmptyDestinations && error.path.contains("second")),
        "{error}"
    );

    let mut forged = decoded.clone().into_inner();
    let plan::NodeKind::Graph(graph) = &mut forged.nodes[1].body else {
        panic!("inner Graph")
    };
    graph.wiring.clear();
    let error = plan::ValidatedExecutionPlan::try_new(forged).unwrap_err();
    assert!(
        matches!(error, plan::PlanValidationError::UnwiredOutput { .. }),
        "{error}"
    );

    let mut forged = decoded.into_inner();
    let inner = forged
        .nodes
        .iter_mut()
        .find(|owner| owner.path.to_string() == "inner")
        .unwrap();
    inner
        .ports
        .iter_mut()
        .find(|port| port.port.name() == "first")
        .unwrap()
        .ty = Some(rastok::datatype::DataType::Text);
    assert!(matches!(plan::ValidatedExecutionPlan::try_new(forged),
        Err(plan::PlanValidationError::InvalidRouting(error))
        if error.kind == plan::RouteErrorKind::BoundaryTypeMismatch));
}

#[test]
fn an_unreachable_graph_suspends_its_output_obligations() {
    let compilation = compile(
        r#"graph app : App {
          input in: int
          output out: int
          in -> out
          graph unreachable {
            input in: int
            operator worker { input in: int; output out: int; on in { out = data } }
            in -> worker
          }
        }"#,
    )
    .unwrap();
    assert!(
        compilation
            .warnings
            .iter()
            .any(|warning| warning.code().as_str() == "W-DEAD-GRAPH")
    );
    let bytes = plan::encode(&compilation.plan, &compilation.hash);
    plan::decode(&bytes).unwrap();
}

#[test]
fn measure_only_expiry_is_dead_but_an_explicit_fuel_cap_can_win() {
    let source = r#"graph app : App {
      input in: int
      output out: int
      operator countdown : Loop {
        input in: int
        output out: int
        Loop.measure = Loop.state.data
        on in match {
          data > 0 => Loop.next = data - 1
          _ => out = data
        }
      }
      in -> countdown -> out
    }"#;
    let compilation = compile(source).unwrap();
    assert!(!compilation.warnings.iter().any(|warning| {
        warning.code().as_str() == "W-DEAD-OUTPUT" && warning.to_string().contains("Loop.expired")
    }));
    plan::decode(&plan::encode(&compilation.plan, &compilation.hash)).unwrap();
    let limited = source.replace("Loop.measure =", "Loop.fuel = 2; Loop.measure =");
    let errors = compile(&limited).unwrap_err();
    assert!(errors.diagnostics().iter().any(|error| {
        error.code().as_str() == "E-UNWIRED-OUTPUT" && error.to_string().contains("Loop.expired")
    }));
}

#[test]
fn a_contract_cardinality_violation_is_not_published_even_without_loop() {
    let source = r#"graph app : App {
      input in: int
      output result: variant
      connector request : HttpClient {
        url = "https://example.test/"
        codec = json
        idempotent = true
        behavior = probabilistic
        timeout = duration("1s")
      }
      in -> request
      request.out -> result
      request.exit -> nothing
    }
    lab app {
      simulation bad_driver on app {
        mock request { on in { exit = Completed { status = 200 } } }
        scenario omitted_frame { app.in = 3 }
      }
    }"#;
    let compilation = compile(source).unwrap();
    let runs = rastok::lab::run(
        &compilation.plan,
        plan::GraphVersion(1),
        &compilation.labs[0],
    );
    assert!(
        runs[0]
            .report
            .failures
            .iter()
            .any(|error| error.contains("ContractViolation")),
        "{:?}",
        runs[0].report.failures
    );
}
