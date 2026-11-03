//! Graph Call control is derived from complete Contract applications, not
//! independent guesses about response frames and terminal exits.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::compiler;
use rastok::connector::contract::ContractCatalog;
use rastok::journal::loop_protocol::{LoopView, Phase};
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
    idempotent = true
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
  request.exit -> match { Completed | ServerError -> nothing; _ -> Loop.again }
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
  simulation nested {{
    mock worker.request {{ on in {{ out = "ready"; exit = Completed {{ status = 200 }} }} }}
    scenario one {{
      app.in = "request"
      expect worker.judge.in where data == "ready"
      expect app.out where data == "ready"
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
        ("ready", "expect app.out", Phase::Closed, 0),
        ("retry", "expect app.out == nothing", Phase::Expired, 1),
    ] {
        let source = format!(
            r#"{HTTP_GRAPH}
lab app {{
  simulation calls {{
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
        let fold = LoopView::from_records(&runs[0].records).unwrap();
        let (_, run) = fold.runs().next().unwrap();
        assert_eq!((run.phase, run.iteration), (phase, iteration));
        assert_eq!(fold.runs().count(), 1, "the Call is not a second Loop");
    }
}

#[test]
fn a_graph_call_cannot_compete_with_its_response_verdict() {
    let source = HTTP_GRAPH.replace(
        "Completed | ServerError -> nothing",
        "Completed | ServerError -> Loop.done",
    );
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
        .replace("Completed | ServerError -> nothing", "Exited -> nothing")
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
  simulation calls {{
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
        let fold = LoopView::from_records(&runs[0].records).unwrap();
        let (_, run) = fold.runs().next().unwrap();
        assert_eq!((run.iteration, run.phase), (1, Phase::Expired), "{mode}");
    }
}

#[test]
fn a_graph_driver_cannot_hide_a_missing_promised_response() {
    let source = format!(
        r#"{HTTP_GRAPH}
lab app {{
  simulation broken {{
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
        .replace("Completed | ServerError -> nothing", "Exited -> nothing")
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
fn loop_repeat_warning_includes_sibling_calls_not_only_the_command_source() {
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
    idempotent = true; behavior = deterministic
    timeout = duration("1s")
  }
  in -> { decide; call }
  decide.out -> Loop.again
  call.out -> nothing
  call.err -> nothing
  call.exit -> nothing
}
"#;
    for (guarantee, command, warns) in [
        ("false", "Loop.again", true),
        ("true", "Loop.again", false),
        ("false", "Loop.next", false),
    ] {
        let source = source
            .replace("idempotent = true", &format!("idempotent = {guarantee}"))
            .replace(
                "decide.out -> Loop.again",
                &format!("decide.out -> {command}"),
            );
        let compiled = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
        assert_eq!(
            compiled
                .warnings
                .iter()
                .any(|diagnostic| diagnostic.code().as_str() == "W-LOOP-REPEAT"),
            warns
        );
        assert_eq!(
            !compiled.plan.loop_repeat_warnings().unwrap().is_empty(),
            warns
        );
        if warns {
            let evidence = compiled.plan.loop_repeat_warnings().unwrap();
            assert_eq!(evidence.len(), 1);
            assert_eq!(evidence[0].connector.to_string(), "call");
            assert_eq!(evidence[0].source.port.name(), "out");
            assert_eq!(
                compiled.plan.nodes[evidence[0].source.node.index()]
                    .path
                    .to_string(),
                "decide"
            );
            let diagnostic = compiled
                .warnings
                .iter()
                .find(|item| item.code().as_str() == "W-LOOP-REPEAT")
                .unwrap();
            assert!(diagnostic.message().contains("decide.out -> Loop.again"));
            assert!(diagnostic.message().contains("Call `call`"));
        }
    }
}

#[test]
fn loop_repeat_warning_keeps_terminal_case_correlation() {
    let source = r#"
graph app : App + Loop {
  input in: text
  output err: union { LoopExpired { last_state: text } }
  Loop.fuel = 2
  connector call : Process {
    command = "cat"; codec = lines
    idempotent = true; behavior = deterministic
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
        let source = source
            .replace("SpawnFailed", terminal)
            .replace("idempotent = true", "idempotent = false");
        let compiled = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
        assert_eq!(
            compiled
                .warnings
                .iter()
                .any(|diagnostic| diagnostic.code().as_str() == "W-LOOP-REPEAT"),
            !allowed,
            "{terminal}"
        );
        assert_eq!(
            !compiled.plan.loop_repeat_warnings().unwrap().is_empty(),
            !allowed
        );
    }
}
