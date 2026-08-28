//! M1.7 L3: one finite progress verdict is proved from the ordinary Graph,
//! independently of the name or number of terminal outputs.

#![allow(clippy::disallowed_methods, clippy::expect_used, clippy::panic)]

use rastok::compiler::compile;
use rastok::contract::ContractCatalog;

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

#[test]
fn terminal_outputs_keep_domain_names_and_stop_is_explicit_termination() {
    let named = loop_source("  output exit: int", "  on in { exit = data }");
    compile_one(&named).expect("a domain-named output terminates the run");

    let several = loop_source(
        "  output result: int\n  output audit: int",
        "  on in { result = data; audit = data }",
    );
    compile_one(&several).expect("several atomic terminal outputs are one verdict");

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
  . -> split
  split.result -> .result
  split.audit -> .audit
}
"#;
    compile_one(relayed).expect("one Operator transaction may terminate on several outputs");

    let stopped = loop_source("", "  on in { stop }");
    compile_one(&stopped).expect("stop terminates while closing the Loop owner instance");
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
