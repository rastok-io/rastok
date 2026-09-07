//! A destination is a checked obligation, never an implicit discard.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::{compiler, contract::ContractCatalog, plan};

fn compile(source: &str) -> Result<compiler::Compilation, rastok::diagnostics::Bag> {
    compiler::compile(&[source], &ContractCatalog::try_m0().unwrap())
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
    forged.edges.retain(|edge| edge.to.is_some());
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
    assert_eq!(decoded.boundary_routes, compilation.plan.boundary_routes);
    assert!(!decoded.boundary_routes.is_empty());

    let mut forged = decoded.clone().into_inner();
    forged.boundary_routes.retain(|rule| {
        !(matches!(&rule.source.owner, plan::EndpointOwner::Graph(path) if path.to_string() == "inner")
            && rule.source.port.name() == "second")
    });
    // Remove the matching flattened edge as well: flat routing alone cannot
    // tell that one sibling boundary disposition has been erased.
    forged.edges.retain(|edge| {
        !edge
            .to
            .as_ref()
            .is_some_and(|target| target.node.is_none() && target.port.name() == "auxiliary")
    });
    let error = plan::ValidatedExecutionPlan::try_new(forged).unwrap_err();
    assert!(
        matches!(&error, plan::PlanValidationError::InvalidRouting(error)
        if error.kind == plan::RouteErrorKind::EmptyDestinations && error.path.contains("second")),
        "{error}"
    );

    let mut forged = decoded.clone().into_inner();
    forged.boundary_routes.clear();
    assert!(matches!(plan::ValidatedExecutionPlan::try_new(forged),
        Err(plan::PlanValidationError::InvalidRouting(error))
        if error.kind == plan::RouteErrorKind::PhysicalEdgeMismatch));

    let mut forged = decoded.into_inner();
    let inner = forged
        .owners
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
        effect = idempotent
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
