//! Graph Call control is derived from complete Contract applications, not
//! independent guesses about response frames and terminal exits.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::compiler;
use rastok::contract::ContractCatalog;
use rastok::journal::loop_protocol::{Fold, Phase};
use rastok::plan::{self, GraphVersion};

const HTTP_GRAPH: &str = r#"
graph app : App + Loop {
  input in: text
  output err: union { LoopExpired { last_state: text } }
  output out: text
  Loop.fuel = 2
  connector request : HttpClient {
    url = "https://example.invalid"
    codec = text
    effect = idempotent
    behavior = probabilistic
    timeout = duration("1s")
  }
  operator judge {
    input in: text
    output accepted: text
    output revised: text
    on in match {
      data == "ready" => accepted = data
      _ => revised = data
    }
  }
  in -> request
  request.out -> judge
  request.exit -> match { Completed -> nothing; _ -> Loop.again }
  judge.accepted -> out
  judge.revised -> Loop.next
}
"#;

#[test]
fn the_lab_resolves_nested_mocks_and_observations_by_exact_path() {
    let nested = HTTP_GRAPH.replace("graph app : App + Loop", "graph worker : Loop");
    let source = format!(
        r#"
graph app : App {{
  input in: text
  output out: text
  {nested}
  in -> worker -> out
  worker.err -> nothing
}}
lab app {{
  simulation nested on app {{
    mock worker.request {{ on in {{ out = "ready"; exit = Completed {{ status = 200 }} }} }}
    scenario one {{
      app.in = "request"
      expect worker.judge.in count == 1 where data == "ready"
      expect app.out count == 1 where data == "ready"
    }}
  }}
}}
"#
    );
    let compiled = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let runs = rastok::lab::run(&compiled.plan, GraphVersion(1), &compiled.labs[0]);
    assert!(runs[0].report.passed(), "{:?}", runs[0].report.failures);
    let unknown = source.replace("mock worker.request", "mock wrong.request");
    let error = compiler::compile(&[&unknown], &ContractCatalog::try_m0().unwrap()).unwrap_err();
    assert!(
        error
            .diagnostics()
            .iter()
            .any(
                |diagnostic| diagnostic.code().as_str() == "E-RESOLVE-UNKNOWN"
                    && diagnostic.message().contains("wrong.request")
            ),
        "{error:?}"
    );
}

#[test]
fn a_graph_call_correlates_its_single_response_with_its_terminal_case() {
    for (reply, assertion, phase, iteration) in [
        ("ready", "expect app.out count == 1", Phase::Closed, 0),
        ("retry", "expect nothing app.out", Phase::Expired, 1),
    ] {
        let source = format!(
            r#"{HTTP_GRAPH}
lab app {{
  simulation calls on app {{
    mock request {{ on in {{ out = "{reply}"; exit = Completed {{ status = 200 }} }} }}
    scenario response {{
      app.in = "request"
      {assertion}
    }}
  }}
}}
"#
        );
        let compiled = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap())
            .unwrap_or_else(|error| panic!("{reply}: {error:?}"));
        let (plan, _) = plan::decode(&plan::encode(&compiled.plan, &compiled.hash)).unwrap();
        let runs = rastok::lab::run(&plan, GraphVersion(1), &compiled.labs[0]);
        assert!(runs[0].report.passed(), "{:?}", runs[0].report.failures);
        let fold = Fold::from_records(&runs[0].records).unwrap();
        let (_, run) = fold.runs().next().unwrap();
        assert_eq!((run.phase, run.iteration), (phase, iteration));
        assert_eq!(fold.runs().count(), 1, "the Call is not a second Loop");
    }
}

#[test]
fn a_graph_call_cannot_compete_with_its_response_verdict() {
    let source = HTTP_GRAPH.replace("Completed -> nothing", "Completed -> Loop.done");
    let error = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap_err();
    assert!(
        format!("{error:?}").contains("exactly one verdict"),
        "{error:?}"
    );
}

#[test]
fn a_streamed_graph_response_cannot_decide_once_per_frame() {
    let source = HTTP_GRAPH
        .replace("HttpClient", "Process")
        .replace("url = \"https://example.invalid\"", "command = \"cat\"")
        .replace("codec = text", "codec = lines")
        .replace("Completed -> nothing", "Exited -> nothing")
        .replace(
            "request.out -> judge",
            "request.out -> judge\n  request.err -> nothing",
        );
    let error = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap_err();
    assert!(
        format!("{error:?}").contains("exactly one verdict"),
        "{error:?}"
    );
}

#[test]
fn graph_file_profiles_rederive_never_and_exactly_one_frame_production() {
    for (mode, reply, routes) in [
        (
            "read",
            "out = base64! { YQ== }; exit = Completed {}",
            "file.out -> Loop.next\n  file.exit -> match { Completed -> nothing; _ -> Loop.done }",
        ),
        ("replace", "exit = Completed {}", "file.exit -> Loop.again"),
    ] {
        let source = format!(
            r#"
graph app : App + Loop {{
  input in: bytes
  output err: union {{ LoopExpired {{ last_state: bytes }} }}
  Loop.fuel = 2
  connector file : File {{ mode = {mode}; path = "unused.bin"; timeout = duration("1s") }}
  in -> file
  {routes}
}}
lab app {{
  simulation calls on app {{
    mock file {{ on in {{ {reply} }} }}
    scenario bounded {{ app.in = base64! {{ YQ== }} }}
  }}
}}
"#
        );
        let compiled = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
        let (plan, _) = plan::decode(&plan::encode(&compiled.plan, &compiled.hash)).unwrap();
        let runs = rastok::lab::run(&plan, GraphVersion(1), &compiled.labs[0]);
        assert!(
            runs[0].report.passed(),
            "{mode}: {:?}",
            runs[0].report.failures
        );
        let fold = Fold::from_records(&runs[0].records).unwrap();
        let (_, run) = fold.runs().next().unwrap();
        assert_eq!((run.iteration, run.phase), (1, Phase::Expired), "{mode}");
    }
}

#[test]
fn a_graph_driver_cannot_hide_a_missing_promised_response() {
    let source = format!(
        r#"{HTTP_GRAPH}
lab app {{
  simulation broken on app {{
    mock request {{ on in {{ exit = Completed {{ status = 200 }} }} }}
    scenario one {{ app.in = "request" }}
  }}
}}
"#
    );
    let compiled = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let runs = rastok::lab::run(&compiled.plan, GraphVersion(1), &compiled.labs[0]);
    assert!(
        runs[0]
            .report
            .failures
            .iter()
            .any(|error| error.contains("ContractViolation")),
        "{:?}",
        runs[0].report.failures
    );
    assert!(runs[0].records.iter().all(|record| !matches!(record,
        rastok::journal::Record::Transaction(record) if record.loop_protocol().is_some_and(|packet| !packet.produced.is_empty()))));
}

#[test]
fn a_graph_call_needs_a_bounded_application() {
    let source = HTTP_GRAPH
        .replace("HttpClient", "Process")
        .replace("url = \"https://example.invalid\"", "command = \"cat\"")
        .replace("codec = text", "codec = lines")
        .replace("Completed -> nothing", "Exited -> nothing")
        .replace(
            "request.out -> judge",
            "request.err -> nothing\n    request.out -> judge",
        )
        .replace("timeout = duration(\"1s\")", "");
    let error = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap_err();
    assert!(
        format!("{error:?}").contains("positive plan-constant timeout"),
        "{error:?}"
    );
}

#[test]
fn graph_effect_safety_includes_sibling_calls_not_only_the_command_source() {
    let source = r#"
graph app : App + Loop {
  input in: text
  output err: union { LoopExpired { last_state: text } }
  Loop.fuel = 2
  operator decide {
    input in: text
    output out: text
    on in { out = data }
  }
  connector call : Process {
    command = "cat"; codec = lines
    effect = idempotent; behavior = deterministic
    timeout = duration("1s")
  }
  in -> { decide; call }
  decide.out -> Loop.again
  call.out -> nothing
  call.err -> nothing
  call.exit -> nothing
}
"#;
    let compiled = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    for effect in [
        rastok::node::EffectClass::Irreversible,
        rastok::node::EffectClass::Compensable,
    ] {
        let mut forged = compiled.plan.plan().clone();
        let call = forged
            .nodes
            .iter_mut()
            .find_map(|node| match &mut node.execution {
                plan::NodeExecution::Connector(call) => Some(call),
                _ => None,
            })
            .unwrap();
        call.effect = rastok::node::Effect::Class(effect);
        assert!(matches!(
            plan::ValidatedExecutionPlan::try_new(forged),
            Err(plan::PlanValidationError::UnsafeLoopEffect { .. })
        ));
    }
}

#[test]
fn graph_effect_safety_keeps_terminal_case_correlation() {
    let source = r#"
graph app : App + Loop {
  input in: text
  output err: union { LoopExpired { last_state: text } }
  Loop.fuel = 2
  connector call : Process {
    command = "cat"; codec = lines
    effect = idempotent; behavior = deterministic
    timeout = duration("1s")
  }
  in -> call
  call.out -> nothing
  call.err -> nothing
  call.exit -> match { SpawnFailed -> Loop.again; _ -> Loop.done }
}
"#;
    for (terminal, allowed) in [
        ("SpawnFailed", true),
        ("TimeoutExpired", false),
        ("Exited", false),
    ] {
        let source = source.replace("SpawnFailed", terminal);
        let compiled = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
        let mut plan = compiled.plan.plan().clone();
        let plan::NodeExecution::Connector(call) = &mut plan.nodes[0].execution else {
            panic!("Call");
        };
        call.effect = rastok::node::Effect::Class(rastok::node::EffectClass::Irreversible);
        let result = plan::ValidatedExecutionPlan::try_new(plan);
        if allowed {
            result.unwrap(); // Contract proves no effect began; no execution is claimed.
        } else {
            assert!(
                matches!(
                    result,
                    Err(plan::PlanValidationError::UnsafeLoopEffect { .. })
                ),
                "{terminal}: {result:?}"
            );
        }
    }
}
