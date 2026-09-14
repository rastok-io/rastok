//! Loop control, proof and public Lab behavior across Operator and Graph hosts.

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
    let outputs: Vec<_> = boundary
        .lines()
        .filter_map(|line| line.trim().strip_prefix("output "))
        .filter_map(|line| line.split_once(':').map(|(port, _)| port.trim()))
        .collect();
    let terminals = format!(
        "Loop.terminal = [{}err]",
        outputs
            .iter()
            .map(|name| format!("{name}, "))
            .collect::<String>()
    );
    let dispositions = outputs
        .iter()
        .map(|port| format!("finite.{port} -> nothing"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "graph app : App {{ input in: int\noperator finite : Loop {{\n  input in: int\noutput err: union {{ LoopExpired {{ last_state: int }} }}\n{boundary}\n  Loop.fuel = 5\n{terminals}\n{handlers}\n}}\nin -> finite\n{dispositions}\nfinite.err -> nothing\n}}\n"
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
  Loop.terminal = [done]

  input in: struct {
    label: text
    n: int
  }
  output done: text
  Loop.measure = Loop.state.data.n
  Loop.delay = { kind = fixed; after = duration("1s") }

  operator decide {
    input in: struct { label: text, n: int }
    output done: text
    output revised: struct { label: text, n: int }
    on in match {
      data.n > 0 => revised = { label = data.label; n = data.n - 1 }
      _ => done = data.label
    }
  }
  in -> decide
  decide.done -> done
  decide.revised -> Loop.next
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
type ProcessExit = union {
  CaptureFailed {}
  DecodeFailed { line: text?, message: text }
  Exited { exit_code: int, signal: int? }
  OutcomeUnknown {}
  OutputOverflow { limit: uint, reason: enum { budget, capacity } }
  SpawnFailed { message: text }
  TimeoutExpired {}
}

graph keyed_retry : App + Loop {
  input in: Attempt
  output err: union { LoopExpired { last_state: Attempt } }
  output done: text
  output failed: text
  instance = { key = data.tenant }
  Loop.fuel = 2
  Loop.terminal = [done, failed, err]
  Loop.delay = { kind = fixed; after = duration("1s") }

  graph audit {
    input in: Attempt
    operator observe {
      input in: Attempt
      output out: text
      on in { out = data.tenant }
    }
    in -> observe
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

  in -> { audit; shell }
  shell.out -> nothing
  shell.err -> nothing
  operator decide {
    input in: ProcessExit
    output retry: Attempt
    output done: text
    output failed: text
    on in match data {
      TimeoutExpired => retry = {
        tenant = Loop.state.data.tenant
        attempt = Loop.state.data.attempt + 1
      }
      Exited => done = Loop.state.data.tenant
      CaptureFailed => failed = "capture"
      DecodeFailed => failed = "decode"
      OutcomeUnknown => failed = "unknown"
      OutputOverflow => failed = "overflow"
      SpawnFailed => failed = "spawn"
    }
  }
  shell.exit -> decide
  decide.retry -> Loop.next
  decide.done -> done
  decide.failed -> failed
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
  output err: union { LoopExpired { last_state: struct {
    label: text
    n: int
  } } }

  Loop.terminal = [done, err]

  input in: struct {
    label: text
    n: int
  }
  output done: text
  lifetime = { kind = fixed; after = duration("1s") }
  Loop.fuel = 2
  Loop.delay = { kind = fixed; after = duration("2s") }

  operator decide {
    input in: struct { label: text, n: int }
    output done: text
    output revised: struct { label: text, n: int }
    on in match {
      data.n > 0 => revised = { label = data.label; n = data.n - 1 }
      _ => done = data.label
    }
  }
  in -> decide
  decide.done -> done
  decide.revised -> Loop.next
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
      expect nothing app.err
    }
  }
}
"#;

const LOOP_EXHAUSTION_LAB: &str = r#"
graph exhaustion : App + Loop {
  input in: int
  output err: union { LoopExpired { last_state: int } }
  Loop.fuel = 1
  operator revise {
    input in: int
    output out: int
    on in { out = data + 1 }
  }
  in -> revise
  revise.out -> Loop.next
}

lab exhaustion {
  simulation bounded on app {
    scenario the_last_proposed_state_is_observable {
      app.in = 7
      expect app.err count == 1 where data is LoopExpired
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
                    if transaction.loop_protocol().is_some_and(|packet| packet.facts.iter().any(|fact|
                        matches!(fact, rastok::journal::loop_protocol::Fact::Open { .. })
                    ))
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
    assert_eq!(runs.len(), 2);
    let lifecycle = runs
        .iter()
        .find(|r| r.report.scenario == "lifetime")
        .expect("the lifetime scenario");
    let generations: std::collections::BTreeSet<_> = lifecycle
        .records
        .iter()
        .filter_map(|r| match r {
            Record::Transaction(tx) if tx.owner_path.to_string().is_empty() => {
                Some(tx.instance.generation())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        generations,
        [0, 1].into_iter().collect(),
        "idle expiry opens a fresh generation, not another Loop iteration"
    );
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
fn a_wired_loop_resolves_custom_terminals_and_round_trips() {
    let relayed = r#"
graph finite : App + Loop {
  input in: int
  output err: union { LoopExpired { last_state: int } }
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
    assert_eq!(bag.diagnostics()[0].code().as_str(), "E-LOOP-CONTROL");

    let explicit = relayed.replace(
        "Loop.fuel = 5",
        "Loop.fuel = 5\n  Loop.terminal = [result, audit, err]",
    );
    let compilation = compile_one(&explicit).expect("the wired Loop has one terminal transaction");
    let (decoded, hash) =
        rastok::plan::decode(&rastok::plan::encode(&compilation.plan, &compilation.hash)).unwrap();
    assert_eq!(decoded, compilation.plan);
    assert_eq!(hash, compilation.hash);
}

#[test]
fn a_loop_with_ambiguous_inputs_requires_an_explicit_entry() {
    let source = r#"
graph finite : App + Loop {
  input request: int
  output err: union { LoopExpired { last_state: int } }
  input other: int
  output out: int
  Loop.fuel = 5
  request -> out
  other -> nothing
}
"#;
    let bag = compile_one(source).expect_err("the admission port must be unambiguous");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:#?}");
    assert_eq!(bag.diagnostics()[0].code().as_str(), "E-LOOP-ENTRY");
    compile_one(&source.replace("Loop.fuel = 5", "Loop.fuel = 5\nLoop.entry = request"))
        .expect("an explicit recurrent entry resolves the ambiguity");
}

#[test]
fn several_outputs_must_belong_to_one_terminal_transaction() {
    let source = r#"
graph finite : App + Loop {
  input in: int
  output err: union { LoopExpired { last_state: int } }
  output early: int
  output late: int
  Loop.fuel = 5
  Loop.terminal = [early, late, err]

  operator relay {
    input in: int
    output out: int
    on in { out = data }
  }
  operator second {
    input in: int
    output out: int
    on in { out = data }
  }
  in -> { relay; second }
  relay.out -> early
  second.out -> late
}
"#;
    refusal(source, "exactly one verdict");
}

#[test]
fn closed_dispatch_arms_prove_one_branch_and_ignore_declaration_order() {
    let forward = loop_source(
        "  output done: int",
        "  on in match data > 0 { false => done = data; true => Loop.next = data - 1 }",
    );
    let reversed = loop_source(
        "  output done: int",
        "  on in match data > 0 { true => Loop.next = data - 1; false => done = data }",
    );
    let forward = compile_one(&forward).expect("the complementary partition is total");
    let reversed = compile_one(&reversed).expect("source order is not control semantics");
    assert_eq!(forward.hash, reversed.hash);
    assert_eq!(forward.plan, reversed.plan);
}

#[test]
fn a_checked_integer_measure_can_replace_the_operational_fuel_cap() {
    let source = loop_source(
        "output done: int",
        "on in match { data > 0 => Loop.next = data - 1; _ => done = data }",
    )
    .replace("Loop.fuel = 5", "Loop.measure = Loop.state.data");
    let compilation = compile_one(&source).expect("the affine countdown is proved finite");
    let spec = compilation
        .plan
        .owners
        .iter()
        .find_map(|owner| owner.loop_spec())
        .unwrap();
    assert!(spec.fuel.is_none());
    let proof = spec.proof.as_ref().expect("the proof travels");
    assert_eq!(proof.decreases, [1]);

    let structured = r#"
graph finite : App + Loop {
  Loop.terminal = [done]

  input in: struct {
    n: int
    label: text
  }
  output done: text
  Loop.measure = Loop.state.data.n
  operator decide {
    input in: struct { n: int, label: text }
    output done: text
    output revised: struct { n: int, label: text }
    on in match {
      data.n > 0 => revised = { n = data.n - 1; label = data.label }
      _ => done = data.label
    }
  }
  in -> decide
  decide.done -> done
  decide.revised -> Loop.next
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
        let source = loop_source(
            "output done: int",
            &format!("on in match {{ data > 0 => Loop.next = {next}; _ => done = data }}"),
        )
        .replace("Loop.fuel = 5", "Loop.measure = Loop.state.data");
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
    let unsigned = loop_source("output done: number", "on in { done = data }")
        .replace("input in: int", "input in: number")
        .replace("Loop.fuel = 5", "Loop.measure = Loop.state.data")
        .replace("finite.err -> nothing", "");
    let bag = compile_one(&unsigned).expect_err("the first proof carrier is signed integer");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:#?}");
    assert_eq!(bag.diagnostics()[0].code().as_str(), "E-LOOP");
    assert!(
        bag.diagnostics()[0]
            .message()
            .contains("signed integer expression"),
        "{bag:#?}"
    );

    let invariant = loop_source(
        "output done: int",
        "on in match { data > 0 => Loop.next = data - 1; _ => done = data }",
    )
    .replace(
        "Loop.fuel = 5",
        "Loop.measure = Loop.state.data\nLoop.invariant = Loop.state.data >= 0",
    );
    let bag = compile_one(&invariant).expect_err("an unproved invariant must fail closed");
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
        "exactly one of Next, Again, Done, or stop",
    );
    refusal(
        &loop_source("", "  on in { Loop.next = data; Loop.next = data }"),
        "exactly one of Next, Again, Done, or stop",
    );
    refusal(
        &loop_source(
            "  output exit: int",
            "  on in { Loop.next = data; exit = data }",
        ),
        "exactly one of Next, Again, Done, or stop",
    );
}

#[test]
fn a_filter_cannot_leave_a_loop_silent_and_duplicate_handlers_refuse() {
    refusal(
        &loop_source(
            "  output done: int",
            "  on in where data < 0 { done = data }",
        ),
        "exactly one of Next, Again, Done, or stop",
    );
    for handlers in [
        "on in { done = data }\non in where data >= 0 { Loop.next = data }",
        "on in where data >= 0 { done = data }\non in where data >= 0 { done = data }",
    ] {
        let bag = compile_one(&loop_source("output done: int", handlers)).unwrap_err();
        assert!(
            bag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code().as_str() == "E-HANDLER-DUPLICATE"),
            "{bag:?}"
        );
    }
}

#[test]
fn expired_is_manager_owned() {
    let source = loop_source(
        "",
        "on in { Loop.expired = LoopExpired { last_state = data } }",
    );
    let bag = compile_one(&source).unwrap_err();
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.message().contains("Loop.expired")),
        "{bag:?}"
    );
}

#[test]
fn a_call_decides_on_exactly_once_exit_not_on_a_zero_or_more_stream() {
    let bounded = r#"
graph finite : App + Loop {
  input in: text
  output err: union { LoopExpired { last_state: text } }
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

  in -> shell
  shell.out -> nothing
  shell.err -> nothing
  shell.exit -> Loop.done
}
"#;
    compile_one(bounded).expect("the exactly-once Call exit supplies one terminal verdict");

    let implicit_closing = bounded.replace(
        "  closing = { drain_for = duration(\"1s\"); settle_for = duration(\"1s\") }\n",
        "",
    );
    compile_one(&implicit_closing)
        .expect("Done closes a run, not its instance: it needs no Closing grace period");

    let repeated = bounded.replace(
        "  shell.out -> nothing\n",
        "  output result: text\n  Loop.terminal = [result, err]\n  shell.out -> result\n",
    );
    refusal(&repeated, "exactly one verdict");

    let unbounded = bounded.replace("    timeout = duration(\"1s\")\n", "");
    refusal(&unbounded, "positive plan-constant timeout");
}

#[test]
fn every_case_of_a_typed_exit_union_is_a_finite_guard_domain() {
    let branches = [
        "CaptureFailed -> Loop.done",
        "DecodeFailed -> Loop.done",
        "Exited -> Loop.done",
        "OutcomeUnknown -> Loop.done",
        "OutputOverflow -> Loop.done",
        "SpawnFailed -> Loop.done",
        "TimeoutExpired -> Loop.again",
    ]
    .join("\n");
    let source = format!(
        r#"
graph finite : App + Loop {{
  input in: text
  output err: union {{ LoopExpired {{ last_state: text }} }}
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
  in -> shell
  shell.out -> nothing
  shell.err -> nothing
  shell.exit -> match {{
{branches}
  }}
}}
"#
    );
    compile_one(&source).expect("all typed exit cases form one exhaustive partition");

    let missing = source.replace("TimeoutExpired -> Loop.again", "");
    let bag = compile_one(&missing).unwrap_err();
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-MATCH-NONEXHAUSTIVE"),
        "{bag:?}"
    );
}
