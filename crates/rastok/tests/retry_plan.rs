//! Retry policy is independently verifiable before execution. A forged
//! guarantee is never supplied by a CallOpen.

#![allow(clippy::unwrap_used)]

use rastok::node::Port;
use rastok::{compiler, contract::ContractCatalog, datatype::DataType, expr, plan, value::Value};

fn compile(settings: &str) -> compiler::Compilation {
    compiler::compile(
        &[&format!(
            r#"
graph app : App {{
  input in: text
  output out: text
  connector call : Process {{
    command = "cat"; codec = lines; behavior = deterministic
    timeout = duration("1s")
    {settings}
  }}
  in -> call -> out
  call.err -> nothing
  call.exit -> nothing
}}
"#
        )],
        &ContractCatalog::try_m0().unwrap(),
    )
    .unwrap()
}

fn constant(source: &str) -> plan::PlanConstant {
    let expr = expr::compile(source, &expr::Signature::new()).unwrap();
    let resolved = expr::eval(&expr, &std::collections::BTreeMap::new())
        .unwrap()
        .into_value()
        .unwrap();
    plan::PlanConstant { expr, resolved }
}

fn retry_plan() -> plan::ExecutionPlan {
    let mut raw = compile("idempotent = true").plan.plan().clone();
    let owner = rastok::node::NodeIndex::new(1);
    raw.nodes[owner.index()]
        .extensions
        .push(plan::ExtensionSpec::Retry(Box::new(plan::RetrySpec {
            attempts: constant("3"),
            cases: vec!["TimeoutExpired".into()],
            delay: Some(plan::DelayPolicy::Fixed {
                after: constant("duration(\"100ms\")"),
            }),
            entry: Port::new("in").unwrap(),
            terminal: Port::new("exit").unwrap(),
        })));
    raw
}

#[test]
fn omission_and_explicit_false_are_one_plan_and_identity() {
    let omitted = compile("");
    let explicit = compile("idempotent = false");
    assert_eq!(omitted.hash, explicit.hash);
    assert_eq!(omitted.plan, explicit.plan);
    assert!(!omitted.plan.nodes[1].connector().unwrap().is_idempotent());
    assert_ne!(omitted.hash, compile("idempotent = true").hash);
}

#[test]
fn original_guarantee_must_match_its_closed_expression_and_effective_attribute() {
    for mutation in 0..4 {
        let mut raw = compile("idempotent = true").plan.plan().clone();
        let plan::NodeKind::Connector(call) = &mut raw.nodes[1].body else {
            unreachable!()
        };
        match mutation {
            0 => call.idempotent = None,
            1 => call.idempotent.as_mut().unwrap().resolved = Value::Bool(false),
            2 => {
                call.idempotent.as_mut().unwrap().expr = expr::compile(
                    "data.safe",
                    &expr::Signature::new().with("data.safe", DataType::Bool),
                )
                .unwrap()
            }
            _ => {
                call.attributes
                    .iter_mut()
                    .find(|(name, _)| name == "idempotent")
                    .unwrap()
                    .1 = plan::AttrSetting::Value(Value::Bool(false))
            }
        }
        assert!(
            plan::ValidatedExecutionPlan::try_new(raw).is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn retry_roundtrips_without_any_command_port_or_expired_wrapper() {
    let raw = retry_plan();
    let ordinary = compile("idempotent = true");
    assert_eq!(raw.nodes[1].ports, ordinary.plan.nodes[1].ports);
    let validated = plan::ValidatedExecutionPlan::try_new(raw).unwrap();
    let bytes = plan::encode(&validated, &ordinary.hash);
    let (decoded, _) = plan::decode(&bytes).unwrap();
    assert_eq!(decoded, validated);
    assert_eq!(plan::FORMAT_VERSION, 21);
    // This VM has no bound Call dispatcher: a valid policy cannot bypass
    // the execution gate or reach an unbound Driver.
    let mut vm = rastok::runtime::execution::Execution::new(&validated, plan::GraphVersion(1));
    let mut journal = rastok::journal::MemJournal::new(plan::GraphVersion(1));
    assert!(matches!(
        vm.step(&mut journal),
        Err(
            rastok::runtime::execution::ExecutionFault::UnsupportedExtension {
                extension: rastok::node::Extension::Retry,
                ..
            }
        )
    ));
    assert!(journal.records().is_empty());
}

#[test]
fn hostile_retry_policies_are_rederived_in_core() {
    for mutation in 0..8 {
        let mut raw = retry_plan();
        let owner = rastok::node::NodeIndex::new(1);
        let plan::ExtensionSpec::Retry(spec) = &mut raw.nodes[owner.index()].extensions[0] else {
            unreachable!()
        };
        match mutation {
            0 => spec.attempts.resolved = Value::Int(0),
            1 => spec.attempts = constant("false"),
            2 => spec.cases = vec!["TimeoutExpired".into(), "SpawnFailed".into()],
            3 => spec.cases = vec!["SpawnFailed".into(), "SpawnFailed".into()],
            4 => spec.cases = vec!["RetryExpired".into()],
            5 => spec.entry = Port::new("out").unwrap(),
            6 => spec.terminal = Port::new("out").unwrap(),
            _ => {
                spec.delay = Some(plan::DelayPolicy::Fixed {
                    after: constant("duration(\"0s\")"),
                })
            }
        }
        assert!(
            plan::ValidatedExecutionPlan::try_new(raw).is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn retry_safety_uses_the_original_guarantee_and_terminal_visibility() {
    let mut raw = retry_plan();
    let plan::NodeKind::Connector(call) = &mut raw.nodes[1].body else {
        unreachable!()
    };
    call.idempotent = Some(constant("false"));
    call.attributes
        .iter_mut()
        .find(|(name, _)| name == "idempotent")
        .unwrap()
        .1 = plan::AttrSetting::Value(Value::Bool(false));
    assert!(matches!(
        plan::ValidatedExecutionPlan::try_new(raw.clone()),
        Err(plan::PlanValidationError::InvalidRetryPolicy { .. })
    ));
    let owner = rastok::node::NodeIndex::new(1);
    let plan::ExtensionSpec::Retry(spec) = &mut raw.nodes[owner.index()].extensions[0] else {
        unreachable!()
    };
    spec.cases = vec!["SpawnFailed".into()];
    plan::ValidatedExecutionPlan::try_new(raw).unwrap();
}

#[test]
fn source_retry_reaches_the_same_hir_and_transport_as_other_extensions() {
    use rastok::diagnostics::{Bag, SourceId};
    let checked = |cases: &str, attempts: u64| {
        let source = format!(
            r#"
val safe = true
graph app : App {{
  input in: text
  output out: text
  val gap = duration("10ms")
  connector call : Process + Retry {{
    command = "cat"; codec = lines; behavior = deterministic
    idempotent = safe
    timeout = duration("1s")
    Retry.attempts = {attempts}
    Retry.on = {cases}
    Retry.delay = {{ kind = fixed; after = gap }}
  }}
  in -> call -> out
  call.err -> nothing; call.exit -> nothing
}}
"#
        );
        let catalogue = ContractCatalog::try_m0().unwrap();
        let mut diagnostics = Bag::new();
        let id = SourceId::new(0);
        let tree = compiler::parser::parse(id, &source, &catalogue, &mut diagnostics);
        let unit = compiler::Unit {
            files: vec![(id, tree)],
        };
        let hir = compiler::check::check(&unit, &catalogue, &mut diagnostics, None).unwrap();
        assert!(!diagnostics.has_errors(), "{diagnostics:?}");
        let hash = compiler::lower::semantic_hash(&hir);
        let (raw, _) = compiler::lower::lower(&hir).unwrap();
        let plan = plan::ValidatedExecutionPlan::try_new(raw).unwrap();
        (plan, hash)
    };
    let (normal, hash) = checked("[SpawnFailed, TimeoutExpired]", 3);
    let (reversed, reversed_hash) = checked("[TimeoutExpired, SpawnFailed]", 3);
    assert_eq!(normal, reversed);
    assert_eq!(hash, reversed_hash);
    assert_ne!(hash, checked("[SpawnFailed, TimeoutExpired]", 4).1);
    let spec = normal.nodes[1].retry_spec().unwrap();
    assert_eq!(spec.attempts.resolved, Value::Int(3));
    assert_eq!(
        spec.delay.as_ref().unwrap().duration_after(1),
        Some(10_000_000)
    );
    assert!(
        normal.nodes[1]
            .ports
            .iter()
            .all(|port| !port.port.name().starts_with("Retry"))
    );
    assert!(
        normal.nodes[1]
            .connector()
            .unwrap()
            .attributes
            .iter()
            .all(|(name, _)| !name.starts_with("Retry"))
    );
    let bytes = plan::encode(&normal, &hash);
    assert_eq!(plan::decode(&bytes).unwrap(), (normal, hash));
}
