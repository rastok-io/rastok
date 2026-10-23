//! Public Retry admission preserves the closed host, policy and safety rules.

#![allow(clippy::expect_used, clippy::panic)]

use rastok::compiler::compile;
use rastok::connector::contract::ContractCatalog;
use rastok::diagnostics::Bag;

fn refuse(source: &str) -> Bag {
    compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue"),
    )
    .expect_err("an unsupported Retry must refuse before execution")
}

fn graph(contract: &str, attributes: &str) -> String {
    let mut settings = attributes.to_owned();
    let finite = contract.starts_with("Process") || contract == "HttpClient" || contract == "File";
    if finite && !attributes.contains("behavior") {
        settings.push_str("\nbehavior = deterministic");
    }
    if contract.starts_with("Process") && !attributes.contains("command") {
        settings.push_str("\ncommand = \"cat\"");
    }
    if contract.starts_with("Process") && !attributes.contains("codec") {
        settings.push_str("\ncodec = lines");
    }
    if contract == "HttpClient" {
        settings.push_str("\nurl = \"https://example.invalid\"\ncodec = text");
    }
    if contract == "File" {
        settings.push_str("\npath = \"input.txt\"");
    }
    let stderr = if contract.starts_with("Process") {
        "call.err -> nothing"
    } else {
        ""
    };
    format!(
        "graph app : App {{\nconnector call : {contract} + Retry {{\n{settings}\n}}\nin -> call\ncall.out -> nothing\n{stderr}\ncall.exit -> nothing\n}}"
    )
}

fn assert_code(bag: &Bag, code: &str, fragment: &str) {
    assert!(
        bag.diagnostics().iter().any(|diagnostic| {
            diagnostic.code().as_str() == code && diagnostic.message().contains(fragment)
        }),
        "expected {code} containing {fragment:?}: {bag:?}"
    );
}

fn admitted(source: &str) {
    compile(&[source], &ContractCatalog::try_m0().expect("catalogue"))
        .expect("a supported Retry passes independent plan validation");
}

#[test]
fn valid_retry_executes_through_the_public_compiler_and_lab() {
    let source = graph(
        "Process",
        "command = \"cat\"; codec = lines; behavior = deterministic; timeout = duration(\"1s\")\nRetry.attempts = 3\nRetry.on = [SpawnFailed]",
    );
    compile(&[&source], &ContractCatalog::try_m0().expect("catalogue"))
        .expect("the qualified Call Retry is executable");
    let compilation = compile(
        &[include_str!("fixtures/ui-retry.rkg")],
        &ContractCatalog::try_m0().expect("catalogue"),
    )
    .expect("typed Retry through the public compiler");
    let runs = rastok::lab::run_program(
        &compilation.plan,
        compilation.hash,
        rastok::plan::GraphVersion(1),
        &compilation.labs[0],
        None,
    );
    assert_eq!(runs.len(), 1);
    assert!(runs[0].report.passed(), "{:?}", runs[0].report);
}

#[test]
fn retry_cannot_inherit_repeat_safety_from_effect() {
    let bag = refuse(&graph(
        "Process",
        "effect = idempotent\nRetry.attempts = 3\nRetry.on = [TimeoutExpired]",
    ));
    assert_code(&bag, "E-RETRY-EFFECT", "idempotent = true");
    let fault = bag
        .diagnostics()
        .iter()
        .find(|d| d.code().as_str() == "E-RETRY-EFFECT")
        .expect("unsafe selection");
    for (name, value) in [
        ("owner", "call"),
        ("case", "TimeoutExpired"),
        ("visibility", "outcome_unknown"),
        ("idempotent", "false"),
    ] {
        assert!(
            fault
                .fields()
                .iter()
                .any(|(key, actual)| *key == name && actual == value),
            "{fault:?}"
        );
    }
}

#[test]
fn retry_refuses_non_call_profiles_and_same_owner_loop() {
    for contract in ["HttpServer", "WebSocketClient", "WebSocketServer"] {
        let bag = refuse(&graph(contract, "Retry.attempts = 2; Retry.on = []"));
        assert_code(&bag, "E-RETRY-HOST", "outbound Call");
    }
    let bag = refuse(&graph(
        "Process",
        "mode = service; Retry.attempts = 2; Retry.on = [SpawnFailed]",
    ));
    assert_code(&bag, "E-RETRY-HOST", "outbound Call");
    let source = graph(
        "Process + Loop",
        "Retry.attempts = 2; Retry.on = [SpawnFailed]",
    );
    assert_code(&refuse(&source), "E-EXTENSION-CONFLICT", "Loop");
    let bag =
        refuse("graph app : App { graph inner { connector job : Process + Retry + Loop {} } }");
    let conflict = bag
        .diagnostics()
        .iter()
        .find(|d| d.code().as_str() == "E-EXTENSION-CONFLICT")
        .expect("named conflict");
    for (name, value) in [("owner", "inner.job"), ("left", "Loop"), ("right", "Retry")] {
        assert!(
            conflict
                .fields()
                .iter()
                .any(|(key, actual)| *key == name && actual == value),
            "{conflict:?}"
        );
    }
}

#[test]
fn retry_attributes_fail_by_name_and_exact_source_span() {
    for (attributes, code, fragment) in [
        (
            "Retry.attempts = 0; Retry.on = [SpawnFailed]",
            "E-RETRY",
            "positive",
        ),
        (
            "Retry.attempts = 2; Retry.cases = [SpawnFailed]",
            "E-RETRY",
            "Retry.on",
        ),
        (
            "Retry.attempts = 2; Retry.on = [SpawnFailed, SpawnFailed]",
            "E-RETRY-CASES",
            "twice",
        ),
        (
            "Retry.attempts = 2; Retry.on = [SpwanFailed]",
            "E-RETRY-CASES",
            "SpwanFailed",
        ),
        (
            "idempotent = \"true\"; Retry.attempts = 2; Retry.on = [SpawnFailed]",
            "E-RETRY",
            "boolean",
        ),
    ] {
        let source = graph("Process", attributes);
        let bag = refuse(&source);
        assert_code(&bag, code, fragment);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|d| d.code().as_str() == code)
            .expect("diagnostic");
        assert!(
            diagnostic
                .spans()
                .iter()
                .any(|span| attributes.contains(&source[span.range()])),
            "{bag:?}"
        );
    }
}

#[test]
fn retry_uses_lexical_constants_without_invocation_state() {
    admitted(
        r#"
val limit = 3
val safe = true
graph app : App {
  val gap = duration("10ms")
  graph inner {
    connector call : Process + Retry {
      command = "cat"; codec = lines; behavior = deterministic
      val extra = 1
      idempotent = safe
      Retry.attempts = limit + extra
      Retry.on = [TimeoutExpired, SpawnFailed]
      Retry.delay = { kind = exponential; first = gap; ceiling = duration("20ms") }
      timeout = duration("1s")
    }
    in -> call -> out
    call.err -> nothing; call.exit -> nothing
  }
  in -> inner -> out
}
"#,
    );
    for expression in [
        "data.attempts",
        "meta.app.limit",
        "instance.key",
        "env(\"RETRIES\")",
        "secret(\"retry-limit\")",
    ] {
        let bag = refuse(&graph(
            "Process",
            &format!(
                "Retry.attempts = {expression}; Retry.on = [SpawnFailed]; timeout = duration(\"1s\")"
            ),
        ));
        assert_code(&bag, "E-EXPR", "");
        assert!(
            !bag.diagnostics()
                .iter()
                .any(|d| d.code().as_str() == "E-CHECK-DEFERRED"),
            "a nonconstant policy must not be mistaken for a valid candidate: {bag:?}"
        );
    }
}

#[test]
fn retry_distinguishes_absent_false_true_and_fixed_idempotence() {
    for guarantee in ["", "idempotent = false;"] {
        admitted(&graph(
            "Process",
            &format!(
                "{guarantee} Retry.attempts = 2; Retry.on = [SpawnFailed]; timeout = duration(\"1s\")"
            ),
        ));
        let bag = refuse(&graph(
            "Process",
            &format!(
                "{guarantee} Retry.attempts = 2; Retry.on = [TimeoutExpired]; timeout = duration(\"1s\")"
            ),
        ));
        assert_code(&bag, "E-RETRY-EFFECT", "TimeoutExpired");
    }
    for cases in [
        "[SpawnFailed, TimeoutExpired]",
        "[TimeoutExpired, SpawnFailed]",
    ] {
        admitted(&graph(
            "Process",
            &format!(
                "idempotent = true; Retry.attempts = 2; Retry.on = {cases}; timeout = duration(\"1s\")"
            ),
        ));
    }
    for mode in ["read", "replace"] {
        admitted(&graph(
            "File",
            &format!(
                "mode = {mode}; Retry.attempts = 2; Retry.on = [IoFailed]; timeout = duration(\"1s\")"
            ),
        ));
    }
    let bag = refuse(&graph(
        "File",
        "mode = read; idempotent = false; Retry.attempts = 2; Retry.on = []; timeout = duration(\"1s\")",
    ));
    assert_code(&bag, "E-RETRY", "File fixes");
}

#[test]
fn retry_selects_server_errors_from_the_versioned_http_contract() {
    admitted(&graph(
        "HttpClient",
        "Retry.attempts = 2; Retry.on = [ConnectFailed, DnsFailed, TlsFailed]; timeout = duration(\"1s\")",
    ));
    admitted(&graph(
        "HttpClient",
        "idempotent = true; Retry.attempts = 3; Retry.on = [ServerError]; timeout = duration(\"1s\")",
    ));
    let bag = refuse(&graph(
        "Process",
        "idempotent = true; Retry.attempts = 3; Retry.on = [ServerError]; timeout = duration(\"1s\")",
    ));
    assert_code(&bag, "E-RETRY-CASES", "ServerError");
}

#[test]
fn retry_attributes_require_their_owner_and_do_not_supply_a_port_or_command() {
    for source in [
        "graph app : App + Retry {}",
        "graph app : App { operator worker : Retry {} }",
        "graph app : App { graph inner : Retry {} }",
    ] {
        assert_code(&refuse(source), "E-RETRY-HOST", "outbound Call");
    }
    let source = graph("Process", "Retry.attempts = 2; Retry.on = []").replace(" + Retry", "");
    assert_code(&refuse(&source), "E-RETRY", "+ Retry");
    for field in [
        "cases",
        "next",
        "again",
        "done",
        "state",
        "expired",
        "attempts.extra",
    ] {
        let bag = refuse(&graph(
            "Process",
            &format!(
                "Retry.attempts = 2; Retry.on = []; Retry.{field} = 1; timeout = duration(\"1s\")"
            ),
        ));
        assert_code(&bag, "E-RETRY", &format!("Retry.{field}"));
    }
    let source = graph("Process + Retry", "Retry.attempts = 2; Retry.on = []");
    assert_code(&refuse(&source), "E-NODE-ANNOTATION", "twice");
    for extension in ["App", "Saga", "Retri"] {
        let source = graph(
            &format!("Process + {extension}"),
            "Retry.attempts = 2; Retry.on = []",
        );
        assert_code(&refuse(&source), "E-NODE-ANNOTATION", extension);
    }
}

#[test]
fn retry_case_selection_is_not_an_expression_or_a_text_list() {
    for cases in [
        "SpawnFailed",
        "[\"SpawnFailed\"]",
        "[SpawnFailed == SpawnFailed]",
        "[exit.SpawnFailed]",
        "[RetryExpired]",
        "[_]",
        "[SpawnFailed] + [TimeoutExpired]",
    ] {
        let bag = refuse(&graph(
            "Process",
            &format!(
                "idempotent = true; Retry.attempts = 2; Retry.on = {cases}; timeout = duration(\"1s\")"
            ),
        ));
        assert_code(&bag, "E-RETRY-CASES", "");
    }
    admitted(&graph(
        "Process",
        "Retry.attempts = 1; Retry.on = []; timeout = duration(\"1s\")",
    ));
}

#[test]
fn retry_bounds_and_delays_use_the_closed_positive_domains() {
    for attempts in ["0", "-1", "1.5", "true", "\"3\""] {
        let bag = refuse(&graph(
            "Process",
            &format!("Retry.attempts = {attempts}; Retry.on = []; timeout = duration(\"1s\")"),
        ));
        assert_code(&bag, "E-RETRY", "positive integer");
    }
    for (delay, fragment) in [
        ("duration(\"1s\")", "composite"),
        ("{ after = duration(\"1s\") }", "kind"),
        ("{ kind = fixed }", "after"),
        ("{ kind = random }", "fixed or exponential"),
        ("{ kind = fixed; after = duration(\"0s\") }", "positive"),
        (
            "{ kind = fixed; after = duration(\"1s\"); after = duration(\"2s\") }",
            "twice",
        ),
        (
            "{ kind = fixed; after = duration(\"1s\"); jitter = true }",
            "jitter",
        ),
        (
            "{ kind = exponential; first = duration(\"2s\"); ceiling = duration(\"1s\") }",
            "no smaller",
        ),
    ] {
        let bag = refuse(&graph(
            "Process",
            &format!(
                "Retry.attempts = 2; Retry.on = []; Retry.delay = {delay}; timeout = duration(\"1s\")"
            ),
        ));
        assert_code(&bag, "E-RETRY", fragment);
        assert!(
            !bag.diagnostics()
                .iter()
                .any(|d| d.code().as_str() == "E-LOOP"),
            "{bag:?}"
        );
    }
    for delay in [
        "{ kind = fixed; after = duration(\"1s\") }",
        "{ kind = exponential; first = duration(\"1s\"); ceiling = duration(\"1s\") }",
    ] {
        admitted(&graph(
            "Process",
            &format!(
                "Retry.attempts = 2; Retry.on = []; Retry.delay = {delay}; timeout = duration(\"1s\")"
            ),
        ));
    }
    let bag = refuse(&graph(
        "Process",
        "Retry.attempts = 2; Retry.on = []; timeout = duration(\"0s\")",
    ));
    assert_code(&bag, "E-RETRY", "positive Call timeout");
}

#[test]
fn idempotence_without_retry_is_not_lowered_as_an_effect_alias() {
    for guarantee in ["true", "false"] {
        let source = graph(
            "Process",
            &format!("idempotent = {guarantee}; Retry.attempts = 1; Retry.on = []"),
        )
        .replace(" + Retry", "")
        .replace("Retry.attempts = 1; Retry.on = []", "");
        let compilation = compile(&[&source], &ContractCatalog::try_m0().expect("catalogue"))
            .expect("ordinary Call");
        assert_eq!(
            compilation.plan.nodes[1]
                .connector()
                .expect("Call")
                .is_idempotent(),
            guarantee == "true"
        );
    }
    let source = "graph app : App { connector call : Process { mode = service; command = \"cat\"; codec = lines; behavior = deterministic; idempotent = true } }";
    assert_code(&refuse(source), "E-CONTRACT-ATTR", "idempotent");
}
