//! Loop expiry uses the host's ordinary terminal, never a synthetic port.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::{compiler, connector::contract::ContractCatalog, plan};

const OPERATOR: &str = r#"
graph app : App {
  input in: int
  output out: int
  output err: union { LoopExpired { last_state: int } }
  operator count : Loop {
    input in: int
    output out: int
    output err: union { LoopExpired { last_state: int } }
    Loop.fuel = 2
    on in match {
      data <= 0 => out = data
      _ => Loop.next = data - 1
    }
  }
  in -> count -> out
  count.err -> err
}
"#;

#[test]
fn loop_uses_declared_host_terminals_without_contributing_ports() {
    let compiled = compiler::compile(&[OPERATOR], &ContractCatalog::try_m0().unwrap()).unwrap();
    let (decoded, hash) = plan::decode(&plan::encode(&compiled.plan, &compiled.hash)).unwrap();
    assert_eq!(hash, compiled.hash);
    let node = &decoded.nodes[1];
    let names: Vec<_> = node.ports.iter().map(|port| port.port.name()).collect();
    assert_eq!(names.len(), 3);
    assert!(names.contains(&"in") && names.contains(&"out") && names.contains(&"err"));
    let terminals = &node.loop_spec().unwrap().terminals;
    assert_eq!(
        terminals.iter().map(|port| port.name()).collect::<Vec<_>>(),
        ["err", "out"]
    );
}

#[test]
fn productive_expiry_requires_a_declared_compatible_output() {
    let source = OPERATOR.replace(
        "    output err: union { LoopExpired { last_state: int } }\n",
        "",
    );
    let errors = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap_err();
    let rendered = format!("{errors:?}");
    assert!(rendered.contains("E-UNWIRED-OUTPUT"), "{rendered}");
    assert!(rendered.contains("prove finiteness"), "{rendered}");
}

#[test]
fn a_proved_loop_does_not_resolve_an_unused_default_expiry_target() {
    let source = r#"
graph app : App {
  input in: int
  output out: text
  operator count : Loop {
    input in: int
    output out: text
    output err: int
    Loop.terminal = [out]
    Loop.measure = Loop.state.data
    on in match {
      data > 0 => { err = data; Loop.next = data - 1 }
      _ => out = "liftoff"
    }
  }
  in -> count -> out
  count.err -> nothing
}
"#;
    let compiled = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let (decoded, _) = plan::decode(&plan::encode(&compiled.plan, &compiled.hash)).unwrap();
    assert!(decoded.nodes[1].loop_spec().unwrap().expired.is_none());
}

#[test]
fn contributed_expiry_wiring_has_a_named_migration() {
    let source = OPERATOR.replace("count.err -> err", "count.Loop.expired -> err");
    let errors = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap_err();
    let rendered = format!("{errors:?}");
    assert!(rendered.contains("E-WIRING-MIGRATION"), "{rendered}");
    assert!(rendered.contains("count.err"), "{rendered}");
}

#[test]
fn a_loop_call_extends_only_its_existing_exit() {
    let source = r#"
graph app : App {
  input in: text
  connector job : Process + Loop {
    command = "cat"
    codec = lines
    idempotent = true
    behavior = deterministic
    timeout = duration("1s")
    Loop.fuel = 2
  }
  in -> job
  job.out -> nothing
  job.err -> nothing
  job.exit -> match {
    TimeoutExpired -> job.Loop.again
    _ -> nothing
  }
}
"#;
    let compiled = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let (decoded, _) = plan::decode(&plan::encode(&compiled.plan, &compiled.hash)).unwrap();
    let node = &decoded.nodes[1];
    assert_eq!(node.ports.len(), 4);
    let exit = node
        .ports
        .iter()
        .find(|port| port.port.name() == "exit")
        .unwrap();
    let Some(rastok::datatype::DataType::TaggedUnion(union)) = &exit.ty else {
        panic!("exit union")
    };
    assert!(
        union
            .cases()
            .iter()
            .any(|case| case.name() == "LoopExpired")
    );
    let forged = source.replace(
        "TimeoutExpired -> job.Loop.again",
        "TimeoutExpired | LoopExpired -> job.Loop.again",
    );
    assert!(compiler::compile(&[&forged], &ContractCatalog::try_m0().unwrap()).is_err());

    for override_attribute in ["Loop.expired = exit", "Loop.terminal = [exit]"] {
        let changed = source.replace(
            "Loop.fuel = 2",
            &format!("Loop.fuel = 2; {override_attribute}"),
        );
        let errors =
            compiler::compile(&[&changed], &ContractCatalog::try_m0().unwrap()).unwrap_err();
        assert!(
            errors
                .diagnostics()
                .iter()
                .any(|error| error.code().as_str() == "E-LOOP-TERMINAL"),
            "{errors:?}"
        );
    }

    let forged_driver = format!(
        r#"{source}
lab app {{ simulation forged on app {{
  mock job {{ on in {{ exit = LoopExpired {{ last_state = data }} }} }}
  scenario attempt {{ app.in = "request" }}
}} }}"#
    );
    let compiled =
        compiler::compile(&[&forged_driver], &ContractCatalog::try_m0().unwrap()).unwrap();
    let runs = rastok::lab::run(&compiled.plan, plan::GraphVersion(1), &compiled.labs[0]);
    assert!(
        !runs[0].report.passed(),
        "a Driver cannot manufacture manager exhaustion"
    );
    assert!(
        runs[0]
            .report
            .failures
            .iter()
            .any(|failure| failure.contains("contract") || failure.contains("Contract")),
        "{:?}",
        runs[0].report.failures
    );
}

#[test]
fn expiry_and_pending_commands_recover_on_the_resolved_host_terminal() {
    use rastok::connector::exchange::EnvelopeDraft;
    use rastok::connector::exchange::IngressProposal;
    use rastok::connector::exchange::IngressProvenance;
    use rastok::connector::exchange::IngressSequence;
    use rastok::envelope::{Data, Namespace, Value};
    use rastok::journal::{
        MemJournal,
        loop_protocol::{LoopView, Phase},
    };
    use rastok::plan::GraphVersion;
    use rastok::runtime::execution::Execution;

    for source in [
        OPERATOR.to_string(),
        OPERATOR
            .replace("output err:", "output failed:")
            .replace(
                "Loop.fuel = 2",
                "Loop.fuel = 2; Loop.terminal = [out, failed]; Loop.expired = failed",
            )
            .replace("count.err -> err", "count.failed -> failed"),
    ] {
        let compiled = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
        let mut execution = Execution::new(&compiled.plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        execution
            .accept_input(
                &mut journal,
                IngressProposal {
                    provenance: IngressProvenance::single(
                        rastok::node::Path::new(["test"]).unwrap(),
                    ),
                    draft: EnvelopeDraft {
                        port: rastok::node::Port::new("in").unwrap(),
                        data: Data::materialized(Value::Int(4)),
                        ext: Namespace::new(),
                    },
                    sequence: IngressSequence::Next,
                    session: None,
                },
            )
            .unwrap();
        for _ in 0..30 {
            let mut restored = Execution::new(&compiled.plan, GraphVersion(1));
            restored.recover_from_journal(&journal).unwrap();
            assert_eq!(restored.inspection(), execution.inspection());
            execution = restored;
            if execution.is_stable() {
                break;
            }
            assert!(execution.step(&mut journal).unwrap());
        }
        assert!(execution.is_stable());
        assert_eq!(execution.outputs().len(), 1);
        let port = if source.contains("Loop.expired = failed") {
            "failed"
        } else {
            "err"
        };
        assert_eq!(execution.outputs()[0].port.name(), port);
        let Data::Materialized(Value::TaggedUnion(value)) = execution.outputs()[0].envelope.data()
        else {
            panic!("expiry union")
        };
        assert_eq!(value.case, "LoopExpired");
        assert_eq!(value.payload, [("last_state".to_string(), Value::Int(2))]);
        assert!(
            LoopView::from_records(journal.records())
                .unwrap()
                .runs()
                .all(|(_, run)| run.phase == Phase::Expired)
        );
    }
}

#[test]
fn invalid_expiry_selectors_refuse_by_their_terminal_contract() {
    for target in ["in", "missing", "out", "[err]", "42", "env(\"PORT\")"] {
        let source = OPERATOR.replace(
            "Loop.fuel = 2",
            &format!("Loop.fuel = 2; Loop.expired = {target}"),
        );
        let errors =
            compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap_err();
        let code = if target == "out" {
            "E-UNWIRED-OUTPUT"
        } else {
            "E-LOOP-TERMINAL"
        };
        assert!(
            errors
                .diagnostics()
                .iter()
                .any(|error| error.code().as_str() == code),
            "{target}: {errors:?}"
        );
    }
    let source = OPERATOR.replace(
        "Loop.fuel = 2",
        "Loop.fuel = 2; Loop.terminal = [out]; Loop.expired = err",
    );
    let errors = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap_err();
    assert!(
        errors
            .diagnostics()
            .iter()
            .any(|error| error.code().as_str() == "E-LOOP-TERMINAL")
    );
}

#[test]
fn registry_free_validation_rejects_missing_and_forged_expiry_targets() {
    let compiled = compiler::compile(&[OPERATOR], &ContractCatalog::try_m0().unwrap()).unwrap();
    for mutation in 0..4 {
        let mut raw = compiled.plan.plan().clone();
        let plan::ExtensionSpec::Loop(spec) = &mut raw.nodes[1].extensions[0] else {
            panic!("Loop owner")
        };
        match mutation {
            0 => spec.expired = None,
            1 => spec.expired.as_mut().unwrap().port = rastok::node::Port::new("in").unwrap(),
            2 => spec.expired.as_mut().unwrap().ty = rastok::datatype::DataType::Text,
            _ => spec.expired.as_mut().unwrap().port = rastok::node::Port::new("out").unwrap(),
        }
        assert!(
            plan::ValidatedExecutionPlan::try_new(raw).is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn conventional_terminals_leave_progress_outputs_as_frames() {
    let source = OPERATOR
        .replace(
            "    Loop.fuel = 2",
            "    output progress: int\n    Loop.fuel = 2",
        )
        .replace(
            "_ => Loop.next = data - 1",
            "_ => { progress = data; Loop.next = data - 1 }",
        )
        .replace(
            "count.err -> err",
            "count.err -> err\ncount.progress -> nothing",
        );
    let compiled = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let spec = compiled.plan.nodes[1].loop_spec().unwrap();
    assert_eq!(
        spec.terminals
            .iter()
            .map(|port| port.name())
            .collect::<Vec<_>>(),
        ["err", "out"]
    );
}

#[test]
fn parent_case_routing_must_cover_productible_exhaustion() {
    let source = OPERATOR.replace("count.err -> err", "count.err -> match { _ -> err }");
    compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let source = OPERATOR
        .replace(
            "union { LoopExpired { last_state: int } }",
            "union { Failed {}\n LoopExpired { last_state: int } }",
        )
        .replace("count.err -> err", "count.err -> match { Failed -> err }");
    let errors = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap_err();
    assert!(
        errors
            .diagnostics()
            .iter()
            .any(|error| error.code().as_str() == "E-MATCH-NONEXHAUSTIVE"
                && error.to_string().contains("LoopExpired")),
        "{errors:?}"
    );
}
