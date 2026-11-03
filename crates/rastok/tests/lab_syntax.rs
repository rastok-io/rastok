//! Lab syntax and observation semantics stay outside application identity.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic
)]

use rastok::compiler::{self, Compilation};
use rastok::connector::contract::ContractCatalog;
use rastok::diagnostics::Code;
use rastok::lab;
use rastok::plan::GraphVersion;

const APP: &str = "graph demo : App { input in: text; output out: text; in -> out }\n";

fn compile(source: &str) -> Compilation {
    compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap()
}

fn reports(compilation: &Compilation) -> Vec<lab::ScenarioRun> {
    compilation
        .labs
        .iter()
        .flat_map(|corpus| {
            lab::run_program(
                &compilation.plan,
                compilation.hash,
                GraphVersion(1),
                corpus,
                None,
            )
        })
        .collect()
}

#[test]
fn direct_scenarios_use_the_lab_name_and_repeated_expectations_keep_the_window() {
    let source = format!(
        r#"{APP}
lab smoke {{
  scenario roundtrip {{
    app.in = "hello"
    expect app.out
    expect 1 app.out
    expect app.out == "hello"
    expect app.out where data == "hello"
    expect 0 app.out where data == "other"
    advance duration("1s")
    expect app.out == nothing
    expect 0 app.out
    expect 0 app.out == nothing
  }}
}}
"#
    );
    let compilation = compile(&source);
    let runs = reports(&compilation);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].report.simulation, "smoke");
    assert!(runs[0].report.passed(), "{:#?}", runs[0].report);
    assert_eq!(compilation.hash, compile(APP).hash);
    assert_eq!(compilation.plan, compile(APP).plan);
    let definition = &compilation.scenario_definitions[0];
    assert_eq!(definition.simulation, "smoke");
    assert_eq!(definition.expectations.len(), 8);
    for source in definition.expectations.values() {
        assert!(compilation.sources[0].text[source.span.range()].starts_with("expect "));
    }
}

#[test]
fn implicit_and_explicit_app_targets_are_equivalent() {
    let mut hash = None;
    for target in ["", ": app", ": App", ": demo"] {
        let source = format!(
            "{APP}lab tests {{ simulation world {target} {{ scenario check {{ app.in = \"x\"\nexpect app.out == \"x\" }} }} }}"
        );
        let compilation = compile(&source);
        assert!(reports(&compilation)[0].report.passed());
        assert_eq!(*hash.get_or_insert(compilation.hash), compilation.hash);
    }
    let error = compiler::compile(
        &[&format!(
            "{APP}lab tests {{ simulation world : child {{}} }}"
        )],
        &ContractCatalog::try_m0().unwrap(),
    )
    .unwrap_err();
    assert!(
        error
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == Code::new("E-SIM-TARGET"))
    );
}

#[test]
fn nothing_and_value_counts_refuse_contradictions_before_execution() {
    for expectation in [
        "expect 2 app.out == nothing",
        "expect 0 app.out == \"x\"",
        "expect 18446744073709551616 app.out",
    ] {
        let source = format!("{APP}lab tests {{ scenario invalid {{ {expectation} }} }}");
        let error = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap_err();
        assert!(
            error
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == Code::new("E-EXPR")),
            "{expectation}: {error:?}"
        );
    }
}

#[test]
fn equality_uses_the_ports_contextual_type_checker() {
    let source = r#"
type Done = struct { event_id: text, status: int }
graph demo : App { input in: Done; output out: Done; in -> out }
lab tests { scenario done {
  app.in = { event_id = "evt_1", status = 201 }
  expect app.out == { event_id = "evt_1", status = 201 }
} }
"#;
    assert!(reports(&compile(source))[0].report.passed());
    for expectation in [
        r#"expect app.out == "wrong type""#,
        r#"expect app.out == { event_id = "evt_1", status = "wrong type" }"#,
    ] {
        let original = r#"expect app.out == { event_id = "evt_1", status = 201 }"#;
        let source = source.replace(original, expectation);
        let error = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap_err();
        assert!(
            error
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == Code::new("E-PORT-TYPE")),
            "{error:?}"
        );
    }
}

#[test]
fn a_newline_ends_an_expectation_without_consuming_the_next_action() {
    let source = format!(
        "{APP}lab smoke {{ scenario separate {{\napp.in = \"first\"\nexpect app.out\napp.in = \"second\"\nexpect app.out == \"second\"\n}} }}"
    );
    let compilation = compile(&source);
    assert_eq!(
        compilation.labs[0].simulations[0].scenarios[0].steps.len(),
        4
    );
    assert!(reports(&compilation)[0].report.passed());
}

#[test]
fn lab_fragments_share_defaults_before_scenarios_are_checked() {
    let app = r#"
graph demo : App {
  input in: text
  output out: text
  connector shell : Process {
    mode = job; command = "cat"; codec = lines
    idempotent = true; behavior = deterministic
  }
  in -> shell -> out
  shell.err -> nothing
  shell.exit -> nothing
}
"#;
    let fragments = [
        app,
        "lab smoke { mock shell { on in { out = \"shared\"; exit = Exited { exit_code = 0, signal = null } } } }",
        "lab smoke { scenario direct { app.in = \"x\"\nexpect app.out == \"shared\" } }",
        "lab smoke { simulation explicit { scenario inherited { app.in = \"x\"\nexpect app.out == \"shared\" } } }",
    ];
    let compilation = compiler::compile(&fragments, &ContractCatalog::try_m0().unwrap()).unwrap();
    let runs = reports(&compilation);
    assert_eq!(runs.len(), 2);
    assert!(runs.iter().all(|run| run.report.passed()), "{runs:#?}");
}

#[test]
fn mock_overrides_replace_the_whole_mock_at_the_nearest_level() {
    let source = r#"
graph demo : App {
  input in: text
  output out: text
  connector shell : Process {
    mode = job; command = "cat"; codec = lines
    idempotent = true; behavior = deterministic
  }
  in -> shell -> out
  shell.err -> nothing
  shell.exit -> nothing
}
lab defaults {
  mock shell { on in { out = "lab"; exit = Exited { exit_code = 0, signal = null } } }
  scenario direct { app.in = "x"
    expect app.out == "lab"
  }
  scenario direct_override {
    mock shell { on in { out = "scenario"; exit = Exited { exit_code = 0, signal = null } } }
    app.in = "x"
    expect app.out == "scenario"
  }
  simulation inherited {
    scenario shared { app.in = "x"
      expect app.out == "lab"
    }
  }
  simulation alternate {
    mock shell { on in { out = "simulation"; exit = Exited { exit_code = 0, signal = null } } }
    scenario shared { app.in = "x"
      expect app.out == "simulation"
    }
    scenario override {
      mock shell { on in { out = "scenario"; exit = Exited { exit_code = 0, signal = null } } }
      app.in = "x"
      expect app.out == "scenario"
    }
  }
}
"#;
    let compilation = compile(source);
    let runs = reports(&compilation);
    assert_eq!(runs.len(), 5);
    assert!(runs.iter().all(|run| run.report.passed()), "{runs:#?}");
}

#[test]
fn counts_are_exact_and_filters_count_only_matching_deliveries() {
    let source = r#"
graph demo : App {
  input in: text
  output out: text
  operator emit {
    input in: text
    output out: text
    on in { out = "one"; out = "two"; out = "one" }
  }
  in -> emit -> out
}
lab tests {
  scenario counts {
    app.in = "go"
    expect 3 app.out
    expect 2 app.out == "one"
    expect 2 app.out where data == "one"
    expect app.out == "two"
  }
  scenario default_is_one {
    app.in = "go"
    expect app.out
  }
}
"#;
    let runs = reports(&compile(source));
    assert!(runs[0].report.passed(), "{:#?}", runs[0].report);
    assert_eq!(runs[1].report.failures.len(), 1);
    assert!(runs[1].report.failures[0].contains("expected 1 matching delivery"));
    assert!(runs[1].report.failures[0].contains("observed 3"));
}

#[test]
fn a_faulting_filter_does_not_satisfy_a_zero_count() {
    let source = r#"
graph demo : App { input in: variant; output out: variant; in -> out }
lab tests { scenario invalid_conversion {
  app.in = "not an integer"
  expect 0 app.out where (data as int) == 1
} }
"#;
    let runs = reports(&compile(source));
    assert!(!runs[0].report.passed());
    assert_eq!(runs[0].report.failures.len(), 1);
    assert!(runs[0].report.failures[0].contains("predicate faulted"));
}

#[test]
fn tagged_completion_and_envelope_equality_use_the_observed_port_type() {
    let source = r#"
graph demo : App {
  output out: text
  connector listener : HttpServer { listen = "127.0.0.1:8080"; codec = text }
  listener -> out
  listener.exit -> nothing
}
lab tests { scenario accepted {
  listener.out = { meta.ext.http_method = "POST", data = "request" }
  expect listener.exit == Completed { status = 202 }
  expect listener.exit == { data = Completed { status = 202 } }
  expect app.out == { meta.ext.http_method = "POST", data = "request" }
} }
"#;
    let runs = reports(&compile(source));
    assert!(runs[0].report.passed(), "{:#?}", runs[0].report);
    let invalid = source.replace(
        "Completed { status = 202 }",
        "Completed { status = \"wrong\" }",
    );
    let error = compiler::compile(&[&invalid], &ContractCatalog::try_m0().unwrap()).unwrap_err();
    assert!(
        error
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == Code::new("E-PORT-TYPE")),
        "{error:?}"
    );
}

#[test]
fn equality_reuses_record_map_and_frontend_values_without_conflating_their_types() {
    let source = r#"
graph demo : App { input in: variant; output out: variant; in -> out }
lab tests { scenario values {
  app.in = { a = 1, b = "two" }
  expect app.out == { b = "two", a = 1 }
  app.in = { "key": 3 }
  expect app.out == { "key": 3 }
  expect app.out == json! { "key": 3 }
  app.in = text! {hello}
  expect app.out == text! {hello}
  app.in = base64! {aGVsbG8=}
  expect app.out == base64! {aGVsbG8=}
} }
"#;
    assert!(reports(&compile(source))[0].report.passed());
}

#[test]
fn equality_contextualizes_structural_union_branches_in_payloads_and_envelopes() {
    let source = r#"
type Choice = text | int
graph demo : App { input in: Choice; output out: Choice; in -> out }
lab tests { scenario choices {
  app.in = "text branch"
  expect app.out == "text branch"
  expect app.out == { data = "text branch" }
  app.in = 42
  expect app.out == 42
  expect app.out == { data = 42 }
} }
"#;
    assert!(reports(&compile(source))[0].report.passed());
}

#[test]
fn duplicate_scenarios_and_implicit_simulation_collisions_refuse_by_name() {
    for body in [
        "scenario same {} scenario same {}",
        "scenario direct {} simulation tests {}",
        "simulation same {} simulation same {}",
        "simulation grouped { scenario same {} scenario same {} }",
    ] {
        let source = format!("{APP}lab tests {{ {body} }}");
        let error = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap_err();
        assert!(
            error
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == Code::new("E-DUP-LAB-IDENTITY")),
            "{error:?}"
        );
    }
}

#[test]
fn envelope_patterns_check_only_written_fields_but_reject_wrong_field_types() {
    let source = r#"
type Pair = struct { first: int, second: text }
graph demo : App { input in: Pair; output out: Pair; in -> out }
lab tests { scenario fields {
  app.in = { first = 1, second = "kept" }
  expect app.out == { data.first = 1 }
  expect app.out == { data.second = "kept" }
} }
"#;
    assert!(reports(&compile(source))[0].report.passed());
    let invalid = source.replace(
        "expect app.out == { data.first = 1 }",
        "expect app.out == { data.first = \"wrong\" }",
    );
    let error = compiler::compile(&[&invalid], &ContractCatalog::try_m0().unwrap()).unwrap_err();
    assert!(
        error
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == Code::new("E-PORT-TYPE"))
    );
}

#[test]
fn equality_checks_content_type_metadata_written_by_a_frontend() {
    let source = r#"
graph demo : App { input in: bytes; output out: bytes; in -> out }
lab tests {
  scenario matching {
    app.in = base64!("image/png") { AQI= }
    expect app.out == base64!("image/png") { AQI= }
    expect app.out == { meta.ext.content_type = "image/png" }
  }
  scenario different_type {
    app.in = base64!("image/png") { AQI= }
    expect app.out == base64!("application/octet-stream") { AQI= }
  }
}
"#;
    let runs = reports(&compile(source));
    assert!(runs[0].report.passed(), "{:#?}", runs[0].report);
    assert_eq!(runs[1].report.failures.len(), 1);
}

#[test]
fn labels_legacy_headers_and_nested_scenarios_are_parse_errors_with_positions() {
    for body in [
        "simulation s on app {}",
        "scenario named \"label\" {}",
        "scenario named { expect \"label\" app.out }",
        "scenario outer { scenario inner {} }",
        "simulation outer { simulation inner {} }",
        "scenario outer { simulation inner {} }",
        "scenario named { expect app.out count == 1 }",
    ] {
        let source = format!("{APP}lab tests {{ {body} }}");
        let error = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap_err();
        assert!(
            error.diagnostics().iter().any(|diagnostic| {
                diagnostic.phase() == rastok::diagnostics::Phase::Parse
                    && diagnostic
                        .spans()
                        .iter()
                        .any(|span| span.range().start >= APP.len())
            }),
            "{body}: {error:?}"
        );
    }
}
#[test]
fn a_unit_keeps_one_logical_lab() {
    let catalogue = ContractCatalog::try_m0().unwrap();
    let source = format!("{APP}\nlab first {{}}\nlab second {{}}");
    let bag = compiler::compile(&[&source], &catalogue).unwrap_err();
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-LAB-DUP")
    );
}

#[test]
fn duplicate_mocks_refuse_at_each_scope_without_rejecting_an_override() {
    let app = r#"
graph demo : App {
  connector shell : Process {
    mode = job; command = "cat"; codec = lines
    idempotent = true; behavior = deterministic
  }
  in -> shell -> out
  shell.err -> nothing
  shell.exit -> nothing
}
"#;
    let duplicate = "mock shell {} mock shell {}";
    for body in [
        duplicate.to_string(),
        format!("simulation grouped {{ {duplicate} }}"),
        format!("scenario direct {{ {duplicate} }}"),
    ] {
        let source = format!("{app}lab tests {{ {body} }}");
        let error = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap_err();
        assert!(
            error
                .diagnostics()
                .iter()
                .any(|diagnostic| { diagnostic.code() == Code::new("E-DUP-LAB-IDENTITY") }),
            "{error:?}"
        );
    }
}

#[test]
fn equality_diagnostics_do_not_disclose_classified_values_or_patterns() {
    for expected in ["{ value = 7654321 }", "{ data.value = 7654321 }"] {
        let source = format!(
            r#"
type Protected = struct {{ value: int @sensitive(level = restricted) }}
graph hidden : App {{ input in: Protected; output out: Protected; in -> out }}
lab tests {{ scenario private {{
  app.in = {{ value = 1234567 }}
  expect app.out == {expected}
}} }}
"#
        );
        let compilation = compile(&source);
        let runs = reports(&compilation);
        assert!(!runs[0].report.passed());
        let rendered = rastok::render::scenario_failures(&compilation, &runs[0].report).join("\n");
        assert!(
            rendered.contains("Assertion evidence is redacted"),
            "{rendered}"
        );
        assert!(!rendered.contains("1234567"));
        assert!(!rendered.contains("7654321"));
    }
}
