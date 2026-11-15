//! A handler filter ends before its Dispatch without changing admission,
//! narrowing, plan identity or the journal.

#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::compiler::compile;
use rastok::connector::contract::ContractCatalog;
use rastok::journal::Record;
use rastok::plan::{self, GraphVersion};

fn assert_narrowed_values(context: &str) {
    for (input_type, target, output_type, present, absent, expected, fallback) in [
        (
            "text?", "text", "text", "\"A\"", "null", "\"A\"", "\"none\"",
        ),
        (
            "Event",
            "Present",
            "Payload",
            "Present { note = \"A\" }",
            "Absent {}",
            "{ note = \"A\" }",
            "{ note = \"none\" }",
        ),
    ] {
        let read = "data";
        let (handler, result_type, yes, no) = match context {
            "and" => (
                format!("on in {{ out = data is {target} && [{read}] == [expected] }}"),
                "bool",
                "true",
                "false",
            ),
            "ternary" => (
                format!("on in {{ out = data is {target} ? {read} : fallback }}"),
                output_type,
                expected,
                fallback,
            ),
            "handler" => (
                format!("on in where data is {target} {{ out = {read} }}"),
                output_type,
                expected,
                "nothing",
            ),
            "match" => (
                format!("on in match {{ data is {target} => out = {read}; _ => nothing }}"),
                output_type,
                expected,
                "nothing",
            ),
            _ => panic!("unknown test context"),
        };
        let source = format!(
            r#"
type Payload = struct {{ note: text }}
type Event = union {{ Present {{ note: text }}, Absent {{}} }}
val expected: {output_type} = {expected}
val fallback: {output_type} = {fallback}
graph app : App {{
  input in: {input_type}
  output out: {result_type}
  operator choose {{
    input in: {input_type}
    output out: {result_type}
    {handler}
  }}
  in -> choose -> out
}}
lab narrowed {{
  scenario present_and_absent {{
    app.in = {present}
    expect app.out == {yes}
    app.in = {absent}
    expect app.out == {no}
  }}
}}
"#
        );
        let compiled = compile(&[&source], &ContractCatalog::try_m0().unwrap())
            .unwrap_or_else(|error| panic!("{context}, {input_type}: {error:?}"));
        let bytes = plan::encode(&compiled.plan, &compiled.hash);
        let (decoded, _) =
            plan::decode(&bytes).expect("narrowed expressions validate after transport");
        let runs = rastok::lab::run(&decoded, GraphVersion(1), &compiled.labs[0]);
        assert_eq!(runs.len(), 1);
        assert!(
            runs[0].report.passed(),
            "{context}, {input_type}: {:?}",
            runs[0].report
        );
    }
}

#[test]
fn a_positive_is_converts_a_whole_value_on_the_right_of_and() {
    assert_narrowed_values("and");
}

#[test]
fn a_positive_is_converts_a_whole_value_in_a_ternary() {
    assert_narrowed_values("ternary");
}

#[test]
fn a_positive_is_converts_a_whole_value_in_a_handler() {
    assert_narrowed_values("handler");
}

#[test]
fn a_positive_is_converts_a_whole_value_in_a_match_arm() {
    assert_narrowed_values("match");
}

#[test]
fn closed_match_members_preserve_complete_tagged_values() {
    for (opening, path, emission, ending) in [
        ("on in match data", "data", "data", ""),
        ("on in { match data", "data", "data", "}"),
        (
            "on in { val selected = data; match selected",
            "selected",
            "selected",
            "}",
        ),
        ("on in match data", "data", ".", ""),
    ] {
        let source = format!(
            r#"
type Payload = struct {{ name: text }}
type Outcome = union {{ Stored {{ name: text }}, Refused {{ reason: text }} }}
graph app : App {{
  input in: Outcome
  output out: Outcome
  output names: text
  output payload: Payload
  operator relay {{
    input in: Outcome
    output out: Outcome
    output names: text
    output payload: Payload
    var name: text = ""
    {opening} {{
      Stored => {{
        name = name + {path}.name
        names = name
        payload = {path} as Stored
        out = {emission}
      }}
      Refused => out = {emission}
    }} {ending}
  }}
  in -> relay
  relay.out -> out
  relay.names -> names
  relay.payload -> payload
}}
lab tests {{ scenario keep_case_and_read_fields {{
  app.in = Stored {{ name = "first" }}
  expect app.out == Stored {{ name = "first" }}
  expect app.names == "first"
  expect app.payload == {{ name = "first" }}
  app.in = Stored {{ name = "second" }}
  expect app.out == Stored {{ name = "second" }}
  expect app.names == "firstsecond"
  app.in = Refused {{ reason = "full" }}
  expect app.out == Refused {{ reason = "full" }}
}} }}
"#
        );
        let compiled = compile(&[&source], &ContractCatalog::try_m0().unwrap())
            .unwrap_or_else(|error| panic!("{opening}, {emission}: {error:?}"));
        let (plan, _) = plan::decode(&plan::encode(&compiled.plan, &compiled.hash)).unwrap();
        let runs = rastok::lab::run(&plan, GraphVersion(1), &compiled.labs[0]);
        assert_eq!(runs.len(), 1);
        assert!(runs[0].report.passed(), "{opening}: {:?}", runs[0].report);
    }
}

#[test]
fn a_closed_match_keeps_an_enclosing_type_conversion() {
    let source = r#"
type Outcome = union { Stored { name: text }, Refused { reason: text } }
val stored: Outcome = Stored { name = "present" }
graph app : App {
  input in: variant
  output out: Outcome
  output names: text
  operator relay {
    input in: variant
    output out: Outcome
    output names: text
    on in where data is Outcome match data {
      Stored => { names = data.name; out = data }
      Refused => out = data
    }
  }
  in -> relay
  relay.out -> out
  relay.names -> names
}
lab tests { scenario dynamic_case {
  app.in = stored
  expect app.out == Stored { name = "present" }
  expect app.names == "present"
  app.in = "another type"
  expect app.out == nothing
} }
"#;
    let compiled = compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let (plan, _) = plan::decode(&plan::encode(&compiled.plan, &compiled.hash)).unwrap();
    let runs = rastok::lab::run(&plan, GraphVersion(1), &compiled.labs[0]);
    assert_eq!(runs.len(), 1);
    assert!(runs[0].report.passed(), "{:?}", runs[0].report);
}

#[test]
fn closed_match_selection_needs_one_case_and_an_unchanged_path() {
    for (dispatch, code) in [
        (
            "match data { Stored | Refused => out = data.name }",
            "E-EXPR",
        ),
        (
            "match (true ? data : data) { Stored => out = data.name; Refused => nothing }",
            "E-EXPR",
        ),
        (
            "match data { Stored => out = data.name }",
            "E-MATCH-NONEXHAUSTIVE",
        ),
        (
            "match previous { Stored => { previous = Refused { reason = \"full\" }; out = previous.name }; Refused => nothing }",
            "E-EXPR",
        ),
    ] {
        let source = format!(
            r#"
type Outcome = union {{ Stored {{ name: text }}, Refused {{ reason: text }} }}
graph app : App {{
  input in: Outcome
  output out: text
  operator inspect {{
    input in: Outcome
    output out: text
    var previous: Outcome = Stored {{ name = "before" }}
    on in {{ {dispatch} }}
  }}
  in -> inspect -> out
}}
"#
        );
        let bag = compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap_err();
        assert!(
            bag.diagnostics().iter().any(|d| d.code().as_str() == code),
            "{dispatch}: {bag:?}"
        );
    }
}

#[test]
fn a_closed_match_state_write_restores_the_declared_union() {
    let source = r#"
type Outcome = union { Stored { name: text }, Refused { reason: text } }
graph app : App {
  input in: text
  output out: Outcome
  output names: text
  operator change {
    input in: text
    output out: Outcome
    output names: text
    var previous: Outcome = Stored { name = "before" }
    on in match previous {
      Stored => {
        names = previous.name
        previous = Refused { reason = "full" }
        out = previous
      }
      Refused => out = previous
    }
  }
  in -> change
  change.out -> out
  change.names -> names
}
lab tests { scenario write_ends_selection {
  app.in = "change"
  expect app.names == "before"
  expect app.out == Refused { reason = "full" }
  app.in = "read"
  expect app.names == nothing
  expect app.out == Refused { reason = "full" }
} }
"#;
    let compiled = compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let (plan, _) = plan::decode(&plan::encode(&compiled.plan, &compiled.hash)).unwrap();
    let runs = rastok::lab::run(&plan, GraphVersion(1), &compiled.labs[0]);
    assert_eq!(runs.len(), 1);
    assert!(runs[0].report.passed(), "{:?}", runs[0].report);
}

#[test]
fn inferred_struct_fields_are_wrapped_at_the_typed_destination() {
    let source = r#"
type Receipt = struct { reservation: text }
type Receipts = struct { count: int, flight: Receipt? }
type Nested = struct { copy: Receipts }
graph app : App {
  input in: text
  output out: Receipts
  output nested: Nested
  output optional: Receipts?
  operator book {
    input in: text
    output out: Receipts
    output nested: Nested
    output optional: Receipts?
    var saved: Receipts = { count = 0; flight = null }
    on in {
      val flight = { reservation = data }
      val receipts = { count = 1; flight = flight }
      saved = receipts
      out = saved
      nested = { copy = receipts }
      optional = receipts
    }
  }
  in -> book
  book.out -> out
  book.nested -> nested
  book.optional -> optional
}
lab tests { scenario optional_fields {
  app.in = "first"
  expect app.out where data.flight is Receipt && data.flight.reservation == "first"
  expect app.nested where data.copy.flight is Receipt && data.copy.flight.reservation == "first"
  expect app.optional where data is Receipts && data.flight is Receipt && data.flight.reservation == "first"
  app.in = "second"
  expect app.out where data.flight is Receipt && data.flight.reservation == "second"
} }
"#;
    let compiled = compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let (plan, _) = plan::decode(&plan::encode(&compiled.plan, &compiled.hash)).unwrap();
    let runs = rastok::lab::run(&plan, GraphVersion(1), &compiled.labs[0]);
    assert_eq!(runs.len(), 1);
    assert!(runs[0].report.passed(), "{:?}", runs[0].report);
}

#[test]
fn successive_is_operands_use_every_prior_positive_fact() {
    for predicate in [
        "data is Booked && data.receipt is Receipt && data.receipt.note is text",
        "data is Booked ? (data.receipt is Receipt ? data.receipt.note is text : false) : false",
    ] {
        let source = format!(
            r#"
type Receipt = struct {{ note: text? }}
type Event = union {{ Booked {{ receipt: Receipt? }}, Stopped {{}} }}
graph app : App {{
  input in: Event
  output out: bool
  operator inspect {{
    input in: Event
    output out: bool
    on in {{ out = {predicate} }}
  }}
  in -> inspect -> out
}}
lab tests {{ scenario short_circuit {{
  app.in = Booked {{ receipt = {{ note = "present" }} }}
  expect app.out == true
  app.in = Booked {{ receipt = {{ note = null }} }}
  expect app.out == false
  app.in = Booked {{ receipt = null }}
  expect app.out == false
  app.in = Stopped {{}}
  expect app.out == false
}} }}
"#
        );
        let compiled = compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
        let (plan, _) = plan::decode(&plan::encode(&compiled.plan, &compiled.hash)).unwrap();
        let runs = rastok::lab::run(&plan, GraphVersion(1), &compiled.labs[0]);
        assert_eq!(runs.len(), 1);
        assert!(runs[0].report.passed(), "{predicate}: {:?}", runs[0].report);
    }
}

#[test]
fn is_facts_do_not_escape_into_or_or_a_false_branch() {
    for predicate in [
        "data is Booked || data.receipt is text",
        "data is Booked ? true : data.receipt is text",
    ] {
        let source = format!(
            r#"
type Event = union {{ Booked {{ receipt: text }}, Stopped {{}} }}
graph app : App {{
  input in: Event
  operator inspect {{ input in: Event; on in {{ out = {predicate} }} }}
  in -> inspect -> out
}}
"#
        );
        let bag = compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap_err();
        assert!(
            bag.diagnostics()
                .iter()
                .any(|d| d.code().as_str() == "E-EXPR"),
            "{bag:?}"
        );
    }
}

fn state_narrowing_source(
    context: &str,
    state: &str,
    predicate: &str,
    body: &str,
    output: &str,
    expectations: &str,
) -> String {
    let handler = match context {
        "handler" => format!("on in where {predicate} {{ {body} }}"),
        "match" => format!("on in match {{ {predicate} => {{ {body} }}; _ => nothing }}"),
        _ => panic!("unknown test context"),
    };
    format!(
        r#"
type State = union {{ Ready {{ note: text }}, Empty {{}} }}
graph app : App {{
  input in: text?
  output out: {output}
  operator change {{
    input in: text?
    output out: {output}
    {state}
    {handler}
  }}
  in -> change -> out
}}
lab state {{
  scenario write_then_read {{
    app.in = "input"
    {expectations}
  }}
}}
"#
    )
}

fn assert_state_narrowing_runs(
    state: &str,
    predicate: &str,
    body: &str,
    output: &str,
    expectations: &str,
) {
    for context in ["handler", "match"] {
        let source = state_narrowing_source(context, state, predicate, body, output, expectations);
        let compiled = compile(&[&source], &ContractCatalog::try_m0().unwrap())
            .unwrap_or_else(|error| panic!("{context}: {error:?}"));
        let bytes = plan::encode(&compiled.plan, &compiled.hash);
        let (decoded, hash) = plan::decode(&bytes).expect("state reads validate after transport");
        assert_eq!(hash, compiled.hash);
        let runs = rastok::lab::run(&decoded, GraphVersion(1), &compiled.labs[0]);
        assert_eq!(runs.len(), 1);
        assert!(runs[0].report.passed(), "{context}: {:?}", runs[0].report);
    }
}

#[test]
fn a_state_write_ends_narrowing_even_when_the_new_value_has_the_tested_type() {
    for context in ["handler", "match"] {
        for (state, path, writes) in [
            ("var prev: text? = \"seed\"", "prev", ["null", "\"new\""]),
            (
                "var prev: struct { field: text? } = { field = \"seed\" }",
                "prev.field",
                ["{ field = null }", "{ field = \"new\" }"],
            ),
        ] {
            for write in writes {
                let source = state_narrowing_source(
                    context,
                    state,
                    &format!("{path} is text"),
                    &format!("prev = {write}; out = {path}"),
                    "text",
                    "",
                );
                let error = compile(&[&source], &ContractCatalog::try_m0().unwrap())
                    .expect_err("the write ends the fact, regardless of its new value");
                assert!(
                    error.diagnostics().iter().any(|diagnostic| {
                        diagnostic.code().as_str() == "E-PORT-TYPE"
                            && diagnostic.phase() == rastok::diagnostics::Phase::Check
                    }),
                    "{context}, {path}, {write}: {error:?}",
                );
            }
        }
    }
}

#[test]
fn a_state_write_restores_the_optional_type_for_later_reads() {
    assert_state_narrowing_runs(
        "var prev: text? = \"seed\"",
        "prev is text",
        "prev = null; out = prev",
        "text?",
        "expect app.out == null",
    );
}

#[test]
fn a_state_write_restores_null_comparisons() {
    assert_state_narrowing_runs(
        "var prev: text? = \"seed\"",
        "prev is text",
        "prev = null; out = prev == null",
        "bool",
        "expect app.out == true",
    );
}

#[test]
fn a_state_write_ends_facts_on_its_member_paths() {
    assert_state_narrowing_runs(
        "var prev: struct { field: text? } = { field = \"seed\" }",
        "prev.field is text",
        "prev = { field = null }; out = prev.field",
        "text?",
        "expect app.out == null",
    );
}

#[test]
fn a_narrowed_read_before_a_state_write_keeps_its_fact() {
    assert_state_narrowing_runs(
        "var prev: text? = \"seed\"",
        "prev is text",
        "val saved = prev; out = prev; prev = null; out = saved",
        "text",
        "expect 2 app.out == \"seed\"",
    );
}

#[test]
fn a_state_assignment_reads_its_right_hand_side_before_ending_the_fact() {
    assert_state_narrowing_runs(
        "var prev: text? = \"seed\"",
        "prev is text",
        "prev = prev + \"!\"; out = prev",
        "text?",
        "expect app.out == \"seed!\"",
    );
}

#[test]
fn writing_another_cell_does_not_end_a_narrowing_fact() {
    assert_state_narrowing_runs(
        "var prev: text? = \"seed\"; var previous: text? = \"other\"",
        "previous is text",
        "prev = null; out = previous",
        "text",
        "expect app.out == \"other\"",
    );
}

#[test]
fn a_state_write_does_not_end_an_input_narrowing_fact() {
    assert_state_narrowing_runs(
        "var prev: text? = \"seed\"",
        "data is text && prev is text",
        "prev = null; out = data",
        "text",
        "expect app.out == \"input\"",
    );
}

#[test]
fn a_state_write_can_change_a_tagged_case_before_emitting_the_state() {
    assert_state_narrowing_runs(
        "var prev: State = Ready { note = \"seed\" }",
        "prev is Ready",
        "prev = Empty {}; out = prev",
        "State",
        "expect app.out == Empty {}",
    );
}

#[test]
fn a_state_write_restores_the_policy_shape_of_the_declared_union() {
    for context in ["handler", "match"] {
        let source = state_narrowing_source(
            context,
            "var prev: State = Ready { note = 1 }",
            "prev is Ready",
            "prev = Empty {}; out = prev",
            "State",
            "expect app.out == Empty {}",
        )
        .replace(
            "note: text",
            "note: int @measure(id = items, unit = \"item\")",
        );
        let compiled = compile(&[&source], &ContractCatalog::try_m0().unwrap())
            .unwrap_or_else(|error| panic!("{context}: {error:?}"));
        let (decoded, _) = plan::decode(&plan::encode(&compiled.plan, &compiled.hash)).unwrap();
        let runs = rastok::lab::run(&decoded, GraphVersion(1), &compiled.labs[0]);
        assert!(runs[0].report.passed(), "{context}: {:?}", runs[0].report);
    }
}

#[test]
fn an_explicit_conversion_still_returns_the_selected_value() {
    // A tagged case is already narrowed to its payload in a guarded body;
    // the explicit conversion control uses the optional scalar case below.
    let source = include_str!("fixtures/alpha_0_1_1/R-184-value-after-is-ternary.rkg")
        .replace("? id :", "? id as text :");
    let compiled = compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let runs = rastok::lab::run(&compiled.plan, GraphVersion(1), &compiled.labs[0]);
    assert!(runs[0].report.passed(), "{:?}", runs[0].report);
}

#[test]
fn a_where_match_is_identical_to_its_explicit_nested_dispatch() {
    let source = r#"
type Event = union {
  Accepted { score: int }
  Ignored {}
}
graph filter : App {
  input in: Event
  output out: int
  operator choose {
    input in: Event
    output out: int
    on in where data is Accepted match {
      data.score > 0 => out = data.score
      _ => out = 0
    }
  }
  in -> choose -> out
}
lab filter {
  simulation decisions {
    scenario filter_then_decide {
      app.in = Ignored {}
      expect app.out == nothing
      app.in = Accepted { score = 7 }
      expect app.out where data == 7
      app.in = Accepted { score = -3 }
      expect app.out where data == 0
    }
  }
}
"#;
    let explicit = source.replace(
        "on in where data is Accepted match {\n      data.score > 0 => out = data.score\n      _ => out = 0\n    }",
        "on in match {\n      data is Accepted => match {\n        data.score > 0 => out = data.score\n        _ => out = 0\n      }\n      _ => nothing\n    }",
    );
    assert_ne!(source, explicit);
    let registry = ContractCatalog::try_m0().unwrap();
    let filtered = compile(&[source], &registry).unwrap();
    let nested = compile(&[&explicit], &registry).unwrap();
    assert_eq!(filtered.hash, nested.hash);
    assert_eq!(filtered.plan, nested.plan);
    let bytes = plan::encode(&filtered.plan, &filtered.hash);
    assert_eq!(bytes, plan::encode(&nested.plan, &nested.hash));
    let (decoded, hash) = plan::decode(&bytes).unwrap();
    assert_eq!(decoded, filtered.plan);
    assert_eq!(hash, filtered.hash);

    let runs = rastok::lab::run(&decoded, GraphVersion(1), &filtered.labs[0]);
    let twins = rastok::lab::run(&nested.plan, GraphVersion(1), &nested.labs[0]);
    assert_eq!(runs.len(), 1);
    assert!(runs[0].report.passed(), "{:?}", runs[0].report);
    assert_eq!(runs[0].records, twins[0].records);
    let decisions: Vec<_> = runs[0]
        .records
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction)
                if transaction.owner_path.to_string() == "choose"
                    && !transaction.consumed.is_empty() =>
            {
                Some(transaction)
            }
            _ => None,
        })
        .collect();
    assert_eq!(decisions.len(), 3, "the false filter consumes too");
    assert!(decisions[0].emissions.is_empty());
}

#[test]
fn a_join_where_match_retains_unmatched_candidates_before_dispatch() {
    for bounds in [
        "",
        "last 3",
        "within duration(\"1s\")",
        "within duration(\"1s\") last 3",
    ] {
        let source = format!(
            r#"
type Expiry = union {{ JoinExpired {{}} }}
graph pairing : App {{
  input left: int
  input right: int
  output out: int
  operator pair {{
    input left: int
    input right: int
    output out: int
    output expired: Expiry
    on (left, right) where left.data == right.data {bounds} match left.data > 1 {{
      true => out = left.data + right.data
      false => out = 0
    }}
  }}
  left -> pair.left
  right -> pair.right
  pair.out -> out
  pair.expired -> nothing
}}
lab pairing {{
  simulation candidates {{
    scenario keep_then_match {{
      app.left = 1
      app.right = 2
      expect app.out == nothing
      app.right = 1
      expect app.out where data == 0
      app.left = 2
      expect app.out where data == 4
    }}
  }}
}}
"#
        );
        let result = compile(&[&source], &ContractCatalog::try_m0().unwrap());
        if bounds.is_empty() {
            let bag = result.expect_err("a content join still requires a retention bound");
            assert_eq!(bag.diagnostics().len(), 1, "{bag:?}");
            assert_eq!(bag.diagnostics()[0].code().as_str(), "E-INSTANCE");
            assert!(
                bag.diagnostics()[0]
                    .message()
                    .contains("`within` or `last`")
            );
            continue;
        }
        let compilation = result.unwrap();
        let bytes = plan::encode(&compilation.plan, &compilation.hash);
        let (decoded, _) = plan::decode(&bytes).unwrap();
        let runs = rastok::lab::run(&decoded, GraphVersion(1), &compilation.labs[0]);
        assert_eq!(runs.len(), 1);
        assert!(runs[0].report.passed(), "{bounds}: {:?}", runs[0].report);
        assert_eq!(
            runs[0]
                .records
                .iter()
                .filter(|record| matches!(record, Record::Transaction(transaction)
                    if transaction.join_inputs.len() == 2))
                .count(),
            2,
            "a false admission must not consume either candidate"
        );
    }
}

#[test]
fn a_malformed_where_before_match_is_an_expression_error_not_a_statement_error() {
    let source = "graph g : App { input in: int; operator o { input in: int; on in where data > match { true => nothing; _ => nothing } } in -> o }";
    let bag = compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap_err();
    assert_eq!(bag.diagnostics().len(), 1, "{bag:?}");
    assert_eq!(bag.diagnostics()[0].code().as_str(), "E-EXPR");
    let span = bag.diagnostics()[0].spans()[0].range();
    assert!(span.start >= source.find("data >").unwrap());
    assert!(span.end <= source.find("match {").unwrap());
}
