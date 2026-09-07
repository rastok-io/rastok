//! M1.7 L3: one finite progress verdict is proved from the ordinary Graph,
//! independently of the name or number of terminal outputs.

#![allow(clippy::disallowed_methods, clippy::expect_used, clippy::panic)]

use rastok::compiler::compile;
use rastok::contract::ContractCatalog;
use rastok::journal::Record;
use rastok::plan::GraphVersion;

fn compile_one(source: &str) -> Result<rastok::compiler::Compilation, rastok::diagnostics::Bag> {
    compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
}

fn loop_source(boundary: &str, handlers: &str) -> String {
    format!(
        "graph finite : App + Loop {{\n  input in: int\n{boundary}\n  Loop.fuel = 5\n{handlers}\n}}\n"
    )
}

fn refusal(source: &str, expected: &str) {
    let bag = compile_one(source).expect_err("the Loop control proof must refuse");
    let diagnostics = bag.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{bag:#?}");
    assert_eq!(diagnostics[0].code().as_str(), "E-LOOP-CONTROL");
    assert!(
        diagnostics[0]
            .message()
            .starts_with("cannot prove that Loop `")
            && diagnostics[0]
                .message()
                .contains("` always terminates at `"),
        "the user-facing proof failure should use plain English: {bag:#?}"
    );
    assert!(
        diagnostics[0].message().contains(expected),
        "wanted {expected:?}, got {bag:#?}"
    );
    assert_eq!(
        diagnostics[0]
            .fields()
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>(),
        ["owner", "path", "detail"]
    );
}

fn run_lab(source: &str) -> Vec<rastok::lab::ScenarioRun> {
    let compilation = compile_one(source).expect("the public Loop lab checks");
    let lab = compilation
        .labs
        .first()
        .expect("the source carries one lab");
    let runs = rastok::lab::run(&compilation.plan, GraphVersion(1), lab);
    for run in &runs {
        assert!(run.report.passed(), "{:#?}", run.report.failures);
    }
    runs
}

const SINGLETON_LOOP_LAB: &str = r#"
graph singleton : App + Loop {
  input in: struct {
    label: text
    n: int
  }
  output done: text
  Loop.measure = data.n
  Loop.delay = { kind = fixed; after = duration("1s") }

  on in where data.n <= 0 { done = data.label }
  on in where data.n > 0 {
    next = { label = data.label; n = data.n - 1 }
  }
}

lab singleton {
  simulation finite on app {
    scenario concurrent_roots_share_no_loop_state {
      app.in = { label = "first"; n = 1 }
      expect nothing app.done
      app.in = { label = "second"; n = 2 }
      expect nothing app.done
      advance duration("1s")
      expect app.done count == 1 where data == "first"
      advance duration("1s")
      expect app.done count == 1 where data == "second"
    }
  }
}
"#;

const KEYED_RETRY_LOOP_LAB: &str = r#"
type Attempt = struct {
  tenant: text
  attempt: int
}

graph keyed_retry : App + Loop {
  input in: Attempt
  output done: text
  output failed: text
  instance = { key = data.tenant }
  Loop.fuel = 2
  Loop.delay = { kind = fixed; after = duration("1s") }

  graph audit {
    input in: Attempt
    operator observe {
      input in: Attempt
      output out: text
      on in { out = data.tenant }
    }
    . -> observe
    observe.out -> nothing
  }

  connector shell : Process {
    mode = job
    command = "cat"
    codec = jsonl
    effect = idempotent
    behavior = probabilistic
    timeout = duration("1s")
    max_output = "1 MiB"
  }

  . -> audit
  . -> shell
  shell.out -> nothing
  shell.err -> nothing
  on shell.exit where data is TimeoutExpired {
    next = {
      tenant = Loop.state.data.tenant
      attempt = Loop.state.data.attempt + 1
    }
  }
  on shell.exit where data is Exited          { done = Loop.state.data.tenant }
  on shell.exit where data is CaptureFailed  { failed = "capture" }
  on shell.exit where data is DecodeFailed   { failed = "decode" }
  on shell.exit where data is OutcomeUnknown { failed = "unknown" }
  on shell.exit where data is OutputOverflow { failed = "overflow" }
  on shell.exit where data is SpawnFailed    { failed = "spawn" }
}

lab keyed_retry {
  simulation retry on app {
    mock shell {
      on in where data is Attempt && data.attempt == 0 { out = nothing }
      on in {
        out = { frame = 1 }
        out = { frame = 2 }
        exit = Exited { exit_code = 0; signal = null }
      }
    }

    scenario two_keys_retry_independently {
      app.in = { tenant = "alice"; attempt = 0 }
      expect nothing app.done
      app.in = { tenant = "bob"; attempt = 0 }
      expect nothing app.done
      advance duration("1s")
      expect nothing app.done
      advance duration("1s")
      expect app.done count == 1 where data == "alice"
      expect app.done count == 1 where data == "bob"
      expect nothing app.failed
    }
  }
}
"#;

const LOOP_CLOSING_LAB: &str = r#"
graph closing : App + Loop {
  input in: struct {
    label: text
    n: int
  }
  output done: text
  lifetime = { kind = fixed; after = duration("1s") }
  Loop.fuel = 2
  Loop.delay = { kind = fixed; after = duration("2s") }

  on in where data.n <= 0 { done = data.label }
  on in where data.n > 0 {
    next = { label = data.label; n = data.n - 1 }
  }
}

lab closing {
  simulation generations on app {
    scenario a_stale_wakeup_cannot_enter_the_successor {
      app.in = { label = "old"; n = 1 }
      expect nothing app.done
      advance duration("1s")
      expect nothing app.done
      app.in = { label = "new"; n = 0 }
      expect app.done count == 1 where data == "new"
      advance duration("1s")
      expect nothing app.done
      expect nothing app.expired
    }
  }
}
"#;

const LOOP_EXHAUSTION_LAB: &str = r#"
graph exhaustion : App + Loop {
  input in: int
  Loop.fuel = 1
  on in { next = data + 1 }
}

lab exhaustion {
  simulation bounded on app {
    scenario the_last_proposed_state_is_observable {
      app.in = 7
      expect app.expired count == 1 where data is LoopExpired
    }
  }
}
"#;

#[test]
fn public_lab_runs_independent_roots_in_one_singleton_loop_instance() {
    let runs = run_lab(SINGLETON_LOOP_LAB);
    assert_eq!(runs.len(), 1);
    let loop_opens = runs[0]
        .records
        .iter()
        .filter(|record| {
            matches!(
                record,
                Record::Transaction(transaction)
                    if matches!(
                        transaction.loop_fact(),
                        Some(rastok::journal::LoopFact::Open { .. })
                    )
            )
        })
        .count();
    assert_eq!(
        loop_opens, 2,
        "one singleton instance carries two Loop runs"
    );
}

#[test]
fn public_lab_composes_keyed_retry_nested_graphs_and_process_frames() {
    let runs = run_lab(KEYED_RETRY_LOOP_LAB);
    assert_eq!(runs.len(), 1);
    let records = &runs[0].records;

    let audit_firings = records
        .iter()
        .filter(|record| {
            matches!(
                record,
                Record::Transaction(transaction)
                    if transaction.owner_path.to_string() == "audit.observe"
                        && matches!(transaction.trigger, rastok::node::Trigger::Port(_))
            )
        })
        .count();
    assert_eq!(
        audit_firings, 4,
        "the nested DAG runs once per body application"
    );

    let successful_multiframe_calls = records
        .iter()
        .filter(|record| {
            matches!(
                record,
                Record::Transaction(transaction)
                    if transaction.owner_path.to_string() == "shell"
                        && transaction
                            .emissions
                            .iter()
                            .filter(|emission| emission.port.name() == "out")
                            .count()
                            == 2
            )
        })
        .count();
    assert_eq!(
        successful_multiframe_calls, 2,
        "each keyed retry may observe many Process frames, but only exit decides progress"
    );
}

#[test]
fn the_keyed_retry_ui_tour_remains_executable() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/keyed-retry.rkg");
    let source = std::fs::read_to_string(path).expect("the public UI tour is readable");
    let runs = run_lab(&source);
    assert_eq!(runs.len(), 1);
}

#[test]
fn public_lab_observes_fuel_exhaustion_and_closing_generation_fencing() {
    assert_eq!(run_lab(LOOP_EXHAUSTION_LAB).len(), 1);
    assert_eq!(run_lab(LOOP_CLOSING_LAB).len(), 1);
}

#[test]
fn terminal_outputs_keep_domain_names_and_stop_is_explicit_termination() {
    let named = loop_source("  output exit: int", "  on in { exit = data }");
    compile_one(&named).expect("a domain-named output terminates the run");

    let several = loop_source(
        "  output result: int\n  output audit: int",
        "  on in { result = data; audit = data }",
    );
    compile_one(&several).expect("several atomic terminal outputs are one verdict");

    let stopped = loop_source("", "  on in { stop }");
    compile_one(&stopped).expect("stop terminates while closing the Loop owner instance");
}

#[test]
fn a_wired_loop_requires_explicit_terminals_and_the_v16_transport() {
    let relayed = r#"
graph finite : App + Loop {
  input in: int
  output result: int
  output audit: int
  Loop.fuel = 5
  operator split {
    input in: int
    output result: int
    output audit: int
    on in { result = data; audit = data }
  }
  in -> split
  split.result -> result
  split.audit -> audit
}
"#;
    let bag = compile_one(relayed).expect_err("a wired Loop declares its terminal set");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:#?}");
    assert_eq!(bag.diagnostics()[0].code().as_str(), "E-LOOP-TERMINAL");

    let explicit = relayed.replace(
        "Loop.fuel = 5",
        "Loop.fuel = 5\n  Loop.terminal = [result, audit]",
    );
    let bag = compile_one(&explicit)
        .expect_err("the new Loop protocol must not silently execute as a v15 Graph handler");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:#?}");
    assert!(
        bag.diagnostics()[0]
            .message()
            .contains("a command-based Loop Graph requires the categorical v16 plan"),
        "{bag:#?}"
    );
}

#[test]
fn the_iteration_state_is_the_loops_only_input() {
    let source = r#"
graph finite : App + Loop {
  input in: int
  input interrupt: int
  output out: int
  Loop.fuel = 5
  on in { out = data }
}
"#;
    let bag = compile_one(source).expect_err("a second input would be a second run protocol");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:#?}");
    assert_eq!(bag.diagnostics()[0].code().as_str(), "E-LOOP");
    assert!(
        bag.diagnostics()[0]
            .message()
            .contains("one iteration input")
    );
}

#[test]
fn several_outputs_must_belong_to_one_terminal_transaction() {
    let source = r#"
graph finite : App + Loop {
  input in: int
  output early: int
  output late: int
  Loop.fuel = 5

  operator relay {
    input in: int
    output out: int
    on in { out = data }
  }
  . -> relay
  relay.out -> nothing
  on in { early = data }
  on relay.out { late = data }
}
"#;
    refusal(source, "more than one terminal transaction");
}

#[test]
fn complementary_guards_prove_one_progress_branch_and_ignore_declaration_order() {
    let forward = loop_source(
        "  output done: int",
        "  on in where data <= 0 { done = data }\n  on in where data > 0 { next = data - 1 }",
    );
    let reversed = loop_source(
        "  output done: int",
        "  on in where data > 0 { next = data - 1 }\n  on in where data <= 0 { done = data }",
    );
    let forward = compile_one(&forward).expect("the complementary partition is total");
    let reversed = compile_one(&reversed).expect("source order is not control semantics");
    assert_eq!(forward.hash, reversed.hash);
    assert_eq!(forward.plan, reversed.plan);
}

#[test]
fn a_checked_integer_measure_can_replace_the_operational_fuel_cap() {
    let source = r#"
graph finite : App + Loop {
  input in: int
  output done: int
  Loop.measure = data

  on in where data <= 0 { done = data }
  on in where data > 0  { next = data - 1 }
}
"#;
    let compilation = compile_one(source).expect("the affine countdown is proved finite");
    let spec = compilation.plan.owners[0].loop_spec().unwrap();
    assert!(spec.fuel.is_none());
    let proof = spec.proof.as_ref().expect("the proof travels");
    assert_eq!(proof.decreases, [1]);

    let structured = r#"
graph finite : App + Loop {
  input in: struct {
    n: int
    label: text
  }
  output done: text
  Loop.measure = data.n

  on in where data.n <= 0 { done = data.label }
  on in where data.n > 0 {
    next = {
      n = data.n - 1
      label = data.label
    }
  }
}
"#;
    let compilation = compile_one(structured).expect("a struct field can be the ranking measure");
    let spec = compilation.plan.owners[0].loop_spec().unwrap();
    assert_eq!(
        spec.proof.as_ref().expect("the proof travels").decreases,
        [1]
    );
}

#[test]
fn a_measure_that_does_not_prove_finiteness_fails_at_its_source() {
    for (next, expected) in [
        ("data", "strictly decrease"),
        ("data + 1", "strictly decrease"),
    ] {
        let source = format!(
            r#"
graph finite : App + Loop {{
  input in: int
  output done: int
  Loop.measure = data

  on in where data <= 0 {{ done = data }}
  on in where data > 0  {{ next = {next} }}
}}
"#
        );
        let bag = compile_one(&source).expect_err("an unproved recurrence must fail closed");
        assert_eq!(bag.diagnostics().len(), 1, "{bag:#?}");
        assert_eq!(bag.diagnostics()[0].code().as_str(), "E-LOOP");
        assert!(
            bag.diagnostics()[0]
                .message()
                .starts_with("this Loop cannot be proved finite:"),
            "{bag:#?}"
        );
        assert!(
            bag.diagnostics()[0].message().contains(expected),
            "{bag:#?}"
        );
    }
}

#[test]
fn a_measure_uses_a_signed_well_founded_carrier_and_a_checked_invariant() {
    let unsigned = r#"
graph finite : App + Loop {
  input in: number
  output done: number
  Loop.measure = data
  on in where data == 0 { done = data }
  on in where data > 0 { next = data - 1 }
}
"#;
    let bag = compile_one(unsigned).expect_err("the first proof carrier is signed integer");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:#?}");
    assert_eq!(bag.diagnostics()[0].code().as_str(), "E-LOOP");
    assert!(
        bag.diagnostics()[0]
            .message()
            .contains("signed integer expression"),
        "{bag:#?}"
    );

    let invariant = r#"
graph finite : App + Loop {
  input in: int
  output done: int
  Loop.measure = data
  Loop.invariant = data >= 0
  on in where data <= 0 { done = data }
  on in where data > 0 { next = data - 1 }
}
"#;
    let bag = compile_one(invariant).expect_err("an unproved invariant must fail closed");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:#?}");
    assert_eq!(bag.diagnostics()[0].code().as_str(), "E-LOOP");
    assert!(
        bag.diagnostics()[0]
            .message()
            .contains("invariant cannot be established"),
        "{bag:#?}"
    );
}

#[test]
fn silence_repeated_next_and_mixed_verdicts_refuse_by_one_code() {
    refusal(
        &loop_source("", "  on in { val held = data }"),
        "finish silently",
    );
    refusal(
        &loop_source("", "  on in { next = data; next = data }"),
        "more than once",
    );
    refusal(
        &loop_source("  output exit: int", "  on in { next = data; exit = data }"),
        "mixes `next` continuation",
    );
}

#[test]
fn guarded_families_must_be_disjoint_and_exhaustive() {
    refusal(
        &loop_source(
            "  output done: int",
            "  on in where data < 0 { done = data }",
        ),
        "without a progress branch",
    );
    refusal(
        &loop_source(
            "  output done: int",
            "  on in { done = data }\n  on in where data >= 0 { next = data }",
        ),
        "same input value",
    );
    refusal(
        &loop_source(
            "  output done: int",
            "  on in where data >= 0 { done = data }\n  on in where data >= 0 { done = data }",
        ),
        "declared twice",
    );
}

#[test]
fn expired_is_manager_owned() {
    refusal(
        &loop_source(
            "",
            "  on in { expired = LoopExpired { last_state = data } }",
        ),
        "produced only by the Loop manager",
    );
}

#[test]
fn a_call_decides_on_exactly_once_exit_not_on_a_zero_or_more_stream() {
    let bounded = r#"
graph finite : App + Loop {
  input in: text
  Loop.fuel = 3
  closing = { drain_for = duration("1s"); settle_for = duration("1s") }

  connector shell : Process {
    mode = job
    command = "cat"
    codec = lines
    effect = idempotent
    behavior = deterministic
    timeout = duration("1s")
    max_output = "1 MiB"
  }

  . -> shell
  shell.out -> nothing
  shell.err -> nothing
  on shell.exit { stop }
}
"#;
    compile_one(bounded).expect("the exactly-once Call exit supplies one terminal verdict");

    let implicit_closing = bounded.replace(
        "  closing = { drain_for = duration(\"1s\"); settle_for = duration(\"1s\") }\n",
        "",
    );
    let bag = compile_one(&implicit_closing)
        .expect_err("a Loop stop cannot invent an implicit Connector grace period");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:#?}");
    assert_eq!(bag.diagnostics()[0].code().as_str(), "E-INSTANCE");
    assert!(bag.diagnostics()[0].message().contains("requires `closing"));

    let repeated = bounded
        .replace("  shell.out -> nothing\n", "  output result: text\n")
        .replace(
            "  on shell.exit { stop }",
            "  on shell.out { result = data }\n  on shell.exit { stop }",
        );
    refusal(&repeated, "zero-or-more Connector output");

    let unbounded = bounded.replace("    timeout = duration(\"1s\")\n", "");
    refusal(&unbounded, "positive plan-constant `timeout`");
}

#[test]
fn every_case_of_a_typed_exit_union_is_a_finite_guard_domain() {
    let branches = [
        "  on shell.exit where data is CaptureFailed { stop }",
        "  on shell.exit where data is DecodeFailed { stop }",
        "  on shell.exit where data is Exited { result = data.exit_code }",
        "  on shell.exit where data is OutcomeUnknown { stop }",
        "  on shell.exit where data is OutputOverflow { stop }",
        "  on shell.exit where data is SpawnFailed { stop }",
        "  on shell.exit where data is TimeoutExpired { next = Loop.state.data }",
    ]
    .join("\n");
    let source = format!(
        r#"
graph finite : App + Loop {{
  input in: text
  output result: int
  Loop.fuel = 3
  closing = {{ drain_for = duration("1s"); settle_for = duration("1s") }}
  connector shell : Process {{
    mode = job
    command = "cat"
    codec = lines
    effect = idempotent
    behavior = deterministic
    timeout = duration("1s")
    max_output = "1 MiB"
  }}
  . -> shell
  shell.out -> nothing
  shell.err -> nothing
{branches}
}}
"#
    );
    compile_one(&source).expect("all typed exit cases form one exhaustive partition");

    let missing = source.replace(
        "  on shell.exit where data is TimeoutExpired { next = Loop.state.data }\n",
        "",
    );
    refusal(&missing, "without a progress branch");
}
