//! Release regressions expressed through the same CLI as the supplied programs.
//! Ignored tests assert the intended correction, not the current defect. Tests
//! for behavior that must remain unchanged run immediately, including scenarios
//! whose specified result is failure. No test starts a physical Connector.

#![allow(clippy::disallowed_types, clippy::disallowed_methods)]

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn rastok() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rastok"));
    command
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/alpha_0_1_1"))
        // The synthetic deployment tests must not depend on the caller's environment.
        .env_remove("R139_TOKEN")
        .env_remove("R139_TOKEN_VERSION");
    command
}

#[allow(clippy::expect_used)] // Process failures must fail this test harness.
fn cli(arguments: &[&str]) -> Output {
    rastok().args(arguments).output().expect("run the CLI")
}

#[allow(clippy::expect_used)]
fn run_with_input(file: &str, input: &str) -> Output {
    let mut child = rastok()
        .args(["run", file])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run the CLI with piped input");
    // Dropping the pipe delivers EOF; pending deadlines alone do not keep run alive.
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(input.as_bytes())
        .expect("write the input");
    child.wait_with_output().expect("collect the CLI output")
}

#[allow(clippy::expect_used)]
fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).expect("UTF-8 stdout")
}

#[allow(clippy::expect_used)]
fn stderr(output: &Output) -> &str {
    std::str::from_utf8(&output.stderr).expect("UTF-8 stderr")
}

#[track_caller]
fn assert_status(output: &Output, expected: i32) {
    assert_eq!(
        output.status.code(),
        Some(expected),
        "stdout:\n{}\nstderr:\n{}",
        stdout(output),
        stderr(output)
    );
}

#[track_caller]
fn assert_check_passes(file: &str) {
    let output = cli(&["check", file]);
    assert_status(&output, 0);
    assert!(stdout(&output).starts_with("ok: "), "{output:?}");
    assert!(stderr(&output).is_empty(), "{output:?}");
}

#[track_caller]
fn assert_scenarios_pass(file: &str, scenarios: &[&str]) {
    assert_check_passes(file);
    let output = cli(&["simulate", file]);
    assert_status(&output, 0);
    assert!(stderr(&output).is_empty(), "{output:?}");
    let mut expected = String::new();
    for scenario in scenarios {
        expected.push_str(&format!("PASS {scenario}\n"));
    }
    expected.push_str(&format!("{} scenario(s), 0 failed\n", scenarios.len()));
    assert_eq!(stdout(&output), expected);
}

#[track_caller]
fn assert_check_refuses(file: &str, code: &str, details: &[&str]) {
    let output = cli(&["check", file]);
    assert_status(&output, 1);
    assert!(stdout(&output).is_empty(), "{output:?}");
    let diagnostic = stderr(&output);
    assert!(
        contains_diagnostic(diagnostic, code, details),
        "no {code} diagnostic contains all of {details:?}:\n{diagnostic}"
    );
}

fn contains_diagnostic(rendered: &str, code: &str, details: &[&str]) -> bool {
    let prefix = format!("error[{code}]:");
    // Keep line endings when separating reports so fields can be matched as complete lines.
    rendered.split_inclusive("\n\n").any(|diagnostic| {
        diagnostic.starts_with(&prefix) && details.iter().all(|detail| diagnostic.contains(detail))
    })
}

#[test]
fn diagnostic_details_are_matched_with_their_code() {
    let rendered = concat!(
        "error[E-PARSE-EXPECTED]: expected a value\n",
        "  --> split-value.rkg:1:1\n",
        "\n",
        "error[E-EXPR]: a value ends with its line\n",
        "  = detail: open a parenthesis on the same line\n",
    );
    let details = [
        "a value ends with its line",
        "open a parenthesis on the same line",
    ];
    assert!(!contains_diagnostic(rendered, "E-PARSE-EXPECTED", &details));
    assert!(contains_diagnostic(rendered, "E-EXPR", &details));
}

#[test]
fn diagnostic_details_are_not_combined_across_matching_reports() {
    let rendered = concat!(
        "error[E-PORT-TYPE]: the first producer has the wrong type\n",
        "  = expected: Frame\n",
        "\n",
        "error[E-PORT-TYPE]: the second producer has the wrong type\n",
        "  = found: Other\n",
    );
    assert!(!contains_diagnostic(
        rendered,
        "E-PORT-TYPE",
        &["= expected: Frame", "= found: Other"],
    ));
    assert!(contains_diagnostic(
        rendered,
        "E-PORT-TYPE",
        &["  = expected: Frame\n"],
    ));
    assert!(contains_diagnostic(
        rendered,
        "E-PORT-TYPE",
        &["  = found: Other\n"],
    ));
}

#[test]
fn r150_a_completed_mock_requires_its_body_at_check() {
    assert_check_refuses(
        "R-150-fake-without-body.rkg",
        "E-CONTRACT-REQUIRED",
        &[
            "std/http-client@1",
            "exactly one",
            "= connector:",
            "  = terminal: Completed\n",
            "  = port: out\n",
            "= expected:",
            "= found:",
            "R-150-fake-without-body.rkg:63:13\n",
        ],
    );
}

#[test]
fn r165_a_bare_route_requires_a_handler_for_the_default_input() {
    assert_check_refuses(
        "R-165-route-without-port.rkg",
        "E-HANDLER-MISSING",
        &["annunciator.storage", "= node: annunciator", "= port: in"],
    );
}

#[test]
fn r176_a_known_timestamp_in_a_json_request_refuses_at_check() {
    assert_check_refuses(
        "R-176-date-in-a-json-body.rkg",
        "E-TYPE-INVALID",
        &["orders.in", "field `at`", "timestamp"],
    );
}

#[test]
fn r176_a_variant_request_still_fails_dynamically_with_decode_failed() {
    let file = "R-176-date-through-a-variant.rkg";
    assert_check_passes(file);
    let output = cli(&["simulate", file]);
    assert_status(&output, 1);
    assert!(stderr(&output).is_empty(), "{output:?}");
    assert_eq!(
        stdout(&output),
        concat!(
            "FAIL r176.a_date_in_a_json_body\n",
            "     expect app.ended == \"Completed\"  (R-176-date-through-a-variant.rkg:75)\n",
            "     observed app.ended: \"DecodeFailed\"\n",
            "     expected 1 matching delivery on app.ended, but observed 0\n",
            "     expect app.out == { ok = true }  (R-176-date-through-a-variant.rkg:76)\n",
            "     nothing observed\n",
            "     expected 1 matching delivery on app.out, but observed 0\n",
            "1 scenario(s), 1 failed\n",
        )
    );
}

#[test]
fn r089_a_known_tagged_union_in_a_json_frame_refuses_at_check() {
    assert_check_refuses(
        "R-089-union-in-a-json-frame.rkg",
        "E-TYPE-INVALID",
        &["agent.in", "field `kind`", "tagged union"],
    );
}

#[test]
fn r184_a_guard_narrows_the_value_read_in_its_body() {
    assert_scenarios_pass(
        "R-184-value-after-is.rkg",
        &["r184.a_value_used_after_an_is_test"],
    );
}

#[test]
fn r184_a_ternary_narrows_the_value_read_in_its_true_branch() {
    assert_scenarios_pass("R-184-value-after-is-ternary.rkg", &["r184b.s"]);
}

#[test]
fn r164_one_advance_and_four_advances_deliver_session_replies_at_the_same_instants() {
    assert_scenarios_pass(
        "R-164-session-mock-in-one-advance.rkg",
        &["session.one_advance", "session.four_advances"],
    );
}

#[test]
fn r164_one_advance_and_four_advances_deliver_the_same_mock_replies() {
    assert_scenarios_pass(
        "R-164-advance-and-fake.rkg",
        &[
            "r164.four_samples_in_one_advance",
            "r164.the_same_four_samples_in_four_advances",
        ],
    );
}

#[test]
fn r088_a_retry_mock_observes_the_opening_time_of_its_attempt() {
    assert_scenarios_pass(
        "R-088-stale-now-in-a-fake.rkg",
        &[
            "r088.the_scenario_stops_on_the_attempt",
            "r088.the_attempt_falls_inside_one_advance",
        ],
    );
}

#[test]
fn r166_a_websocket_client_mock_is_a_named_deferral() {
    assert_check_refuses(
        "R-166-fake-websocket-client.rkg",
        "E-CHECK-DEFERRED",
        &["= construct: a WebSocket Session mock"],
    );
}

#[test]
fn r166_a_websocket_server_mock_is_a_named_deferral() {
    assert_check_refuses(
        "R-166-fake-websocket-server.rkg",
        "E-CHECK-DEFERRED",
        &["= construct: a WebSocket Session mock"],
    );
}

#[test]
fn r092_an_exit_expectation_observes_each_retry_attempt() {
    assert_scenarios_pass(
        "R-092-expect-on-exit.rkg",
        &["r092.an_expectation_on_exit_sees_every_attempt"],
    );
}

#[test]
fn r139_mock_secrets_require_a_profile_and_name_missing_environment_values() {
    let file = "R-139-secret-and-fake.rkg";
    let profile = "R-139-secret-and-fake.deployment.json";
    let missing_profile = cli(&["simulate", file]);
    let missing_environment = cli(&["simulate", file, "--deployment", profile]);
    let supplied = rastok()
        .args(["simulate", file, "--deployment", profile])
        .env("R139_TOKEN", "lab-only-not-a-credential")
        .env("R139_TOKEN_VERSION", "fixture-v1")
        .output()
        .expect("simulate with the synthetic deployment profile");

    assert_status(&missing_profile, 1);
    assert!(stderr(&missing_profile).is_empty(), "{missing_profile:?}");
    for detail in [
        "AGENT_TOKEN",
        "agent.auth_bearer",
        "--deployment",
        "secrets",
        "synthetic",
    ] {
        assert!(
            stdout(&missing_profile).contains(detail),
            "{missing_profile:?}"
        );
    }
    assert_status(&missing_environment, 1);
    assert!(
        stdout(&missing_environment).is_empty(),
        "{missing_environment:?}"
    );
    assert!(stderr(&missing_environment).contains(
        "secret `AGENT_TOKEN`: the environment variable `R139_TOKEN` named by the deployment profile is not set"
    ), "{missing_environment:?}");
    assert_status(&supplied, 0);
    assert!(stderr(&supplied).is_empty(), "{supplied:?}");
    assert_eq!(
        stdout(&supplied),
        "PASS r139.a_fake_stands_for_the_connector\n1 scenario(s), 0 failed\n"
    );
}

#[test]
fn r002_bare_enum_symbols_use_the_expected_type() {
    assert_scenarios_pass("R-002-enum-value.rkg", &["r002.an_enum_value_is_written"]);
}

#[test]
fn r009_empty_collections_use_the_expected_element_types() {
    assert_scenarios_pass(
        "R-009-typed-empty-list.rkg",
        &["r009.an_empty_list_takes_the_type_of_its_place"],
    );
}

#[test]
fn r009_an_empty_struct_is_not_a_map_and_the_refusal_names_the_map_syntax() {
    assert_check_refuses(
        "R-009-empty-struct-in-a-map-place.rkg",
        "E-PORT-TYPE",
        &["counts", "struct", "{:}"],
    );
}

#[test]
fn r003_a_closed_match_arm_knows_its_case_payload() {
    assert_scenarios_pass(
        "R-003-closed-match-arm.rkg",
        &["r003.the_arm_of_a_closed_match_knows_its_case"],
    );
}

#[test]
fn r152_a_second_is_test_uses_the_first_test_narrowing() {
    assert_scenarios_pass(
        "R-152-is-after-is.rkg",
        &["r152.an_is_test_on_a_field_reached_through_an_is_test"],
    );
}

#[test]
fn r152_lab_predicates_use_the_same_narrowing_rule() {
    assert_scenarios_pass(
        "R-152-is-after-is-in-the-lab.rkg",
        &["r152.an_optional_field_under_a_union_case"],
    );
}

#[test]
fn r087_a_state_assignment_can_contain_an_is_ternary() {
    assert_scenarios_pass(
        "R-087-is-in-a-var-assignment.rkg",
        &["r087.an_is_test_written_inside_a_ternary"],
    );
}

#[test]
fn r086_an_integer_converts_exactly_to_a_declared_decimal() {
    assert_scenarios_pass(
        "R-086-integer-to-decimal.rkg",
        &["r086.an_integer_becomes_an_exact_decimal"],
    );
}

#[test]
fn r137_inline_records_are_list_elements_in_outputs_and_expectations() {
    assert_scenarios_pass(
        "R-137-record-inside-a-list.rkg",
        &["r137.records_written_in_place_inside_a_list"],
    );
}

#[test]
fn r091_a_bare_stop_ends_at_its_newline() {
    assert_scenarios_pass(
        "R-091-stop-then-another-arm.rkg",
        &["r091.a_bare_stop_followed_by_another_arm"],
    );
}

#[track_caller]
fn assert_unpaired_join_warning(file: &str, details: &[&str], simulation: &str) {
    let checked = cli(&["check", file]);
    let simulated = cli(&["simulate", file]);
    assert_status(&checked, 0);
    assert!(stdout(&checked).starts_with("ok: "), "{checked:?}");
    let diagnostic = stderr(&checked);
    assert_eq!(
        diagnostic.matches("warning[W-JOIN-UNPAIRED]:").count(),
        1,
        "{diagnostic}"
    );
    for detail in ["= node:", "= connector:", "= port:", "= case:"]
        .iter()
        .chain(details)
    {
        assert!(
            diagnostic.contains(detail),
            "missing {detail:?}:\n{diagnostic}"
        );
    }
    // The warning does not repair the authored pairing: the same scenario still fails.
    assert_status(&simulated, 1);
    assert_eq!(stdout(&simulated), simulation);
}

#[test]
fn r169_http_join_warns_when_a_frame_outlives_its_unrouted_terminal() {
    assert_unpaired_join_warning(
        "R-169-join-http.rkg",
        &["pair", "catalog.out", "catalog.exit", "ServerError"],
        concat!(
            "FAIL r169.a_join_pairs_each_answer_with_its_own_end\n",
            "     expect app.seen == { name = \"item b\"; status = 200 }  (R-169-join-http.rkg:89)\n",
            "     observed app.seen: {\"name\":\"error page\",\"status\":200}\n",
            "     expected 1 matching delivery on app.seen, but observed 0\n",
            "1 scenario(s), 1 failed\n",
        ),
    );
}

#[test]
fn r169_process_join_warns_when_frames_do_not_pair_with_one_exit() {
    assert_unpaired_join_warning(
        "R-169-join-process.rkg",
        &["pair", "tool.out", "tool.exit"],
        concat!(
            "FAIL r169p.a_run_that_writes_nothing_shifts_the_next_one\n",
            "     expect app.out == { size = 2; code = 0 }  (R-169-join-process.rkg:69)\n",
            "     observed app.out: {\"code\":7,\"size\":2}\n",
            "     expected 1 matching delivery on app.out, but observed 0\n",
            "1 scenario(s), 1 failed\n",
        ),
    );
}

#[test]
fn r169_a_join_bound_without_an_instance_key_does_not_prove_pairing() {
    assert_unpaired_join_warning(
        "R-169-join-bounded-no-key.rkg",
        &["pair", "catalog.out", "catalog.exit", "ServerError"],
        concat!(
            "FAIL r169.second_call_inside_the_window\n",
            "     expect app.seen == { name = \"item b\"; status = 200 }  (R-169-join-bounded-no-key.rkg:100)\n",
            "     observed app.seen: {\"name\":\"error page\",\"status\":200}\n",
            "     expected 1 matching delivery on app.seen, but observed 0\n",
            "PASS r169.second_call_after_the_window\n",
            "2 scenario(s), 1 failed\n",
        ),
    );
}

#[test]
fn r169_a_keyed_and_bounded_join_remains_accepted_without_a_warning() {
    assert_scenarios_pass(
        "R-169-join-keyed-and-bounded.rkg",
        &[
            "r169.second_call_inside_the_window",
            "r169.second_call_after_the_window",
        ],
    );
}

#[test]
fn r090_different_structs_are_reported_by_name_and_differing_fields() {
    assert_check_refuses(
        "R-090-two-structs.rkg",
        "E-PORT-TYPE",
        &["Draft", "Review", "hash", "reviewer"],
    );
}

#[test]
fn r090_an_inferred_output_reports_its_actual_structural_difference() {
    assert_check_refuses(
        "R-090-untyped-output.rkg",
        "E-PORT-TYPE",
        &["Frame", "Other", "turn", "note"],
    );
}

#[test]
fn r091_a_second_match_names_the_final_control_form_rule() {
    assert_check_refuses(
        "R-091-two-matches.rkg",
        "E-PARSE-EXPECTED",
        &[
            "a `match` is the last form of its block",
            "another Operator",
            "nest it in an arm",
        ],
    );
}

#[test]
fn r091_a_case_comparison_suggests_the_is_test() {
    assert_check_refuses(
        "R-091-compare-with-a-case.rkg",
        "E-EXPR",
        &["a tagged case is tested with `is`", "!(data.by is Editor)"],
    );
}

#[test]
fn r091_a_split_value_names_the_same_line_parenthesis_rule() {
    assert_check_refuses(
        "R-091-value-on-two-lines.rkg",
        "E-PARSE-EXPECTED",
        &[
            "a value ends with its line",
            "open a parenthesis on the same line",
        ],
    );
}

#[test]
fn r138_a_second_final_output_names_the_missing_terminal_and_its_arm() {
    assert_check_refuses(
        "R-138-second-final-output.rkg",
        "E-LOOP-CONTROL",
        &[
            "selects no verdict",
            "status",
            "Loop.terminal = [out, status]",
            "R-138-second-final-output.rkg:30:",
        ],
    );
}

#[test]
fn r138_disposal_is_not_a_loop_verdict_and_the_refusal_locates_the_arm() {
    assert_check_refuses(
        "R-138-output-to-nothing.rkg",
        "E-LOOP-CONTROL",
        &[
            "selects no verdict",
            "note",
            "nothing",
            "does not finish",
            "R-138-output-to-nothing.rkg:28:",
        ],
    );
}

#[test]
fn r138_a_loop_arm_with_two_verdicts_names_both() {
    assert_check_refuses(
        "R-138-two-verdicts.rkg",
        "E-LOOP-CONTROL",
        &[
            "selects two verdicts",
            "terminal output `out`",
            "Loop.next",
            "R-138-two-verdicts.rkg:27:",
        ],
    );
}

#[test]
fn r167_after_in_a_mock_names_the_deferred_construct() {
    assert_check_refuses(
        "R-167-after-in-a-fake.rkg",
        "E-PARSE-DEFERRED",
        &[
            "delayed mock action",
            "after",
            "not implemented",
            "= construct:",
        ],
    );
}

#[test]
fn r167_foreach_refusal_explains_the_reserved_word() {
    assert_check_refuses(
        "R-167-foreach.rkg",
        "E-RESOLVE-UNKNOWN",
        &[
            "foreach",
            "reserved word",
            "per-item fan-out",
            "not implemented",
        ],
    );
}

#[test]
fn r180_optional_state_access_explains_presence_and_narrowing() {
    assert_check_refuses(
        "R-180-absent-state.rkg",
        "E-EXPR",
        &["dossier", "may be absent", "is Open", "title"],
    );
}

#[test]
fn r136_computed_member_access_suggests_a_local_value_without_admitting_it() {
    assert_check_refuses(
        "R-136-one-shot-member-access.rkg",
        "E-EXPR",
        &[
            "outside this build's subset",
            "name the value first",
            "val row = data.rows.value[0]",
        ],
    );
}

#[test]
fn r093_duplicate_edges_keep_the_emitted_route_diagnostic() {
    let output = cli(&["check", "R-093-duplicate-edge.rkg"]);
    assert_status(&output, 1);
    assert!(stdout(&output).is_empty(), "{output:?}");
    assert_eq!(
        stderr(&output),
        concat!(
            "error[E-ROUTE-DUPLICATE]: destination `o.in` is selected twice for the same source case\n",
            "  --> R-093-duplicate-edge.rkg:30:3\n",
            "   |\n",
            "30 |   in -> o\n",
            "   |   ^^^^^^^\n",
            "  = source: .in\n",
            "  = case: all\n",
        )
    );
}

#[track_caller]
fn assert_missing_simulation_source(arguments: &[&str]) {
    let output = cli(arguments);
    assert_status(&output, 2);
    assert!(stdout(&output).is_empty(), "{output:?}");
    assert_eq!(
        stderr(&output),
        concat!(
            "error: `rastok simulate` needs at least one source file\n",
            "usage: rastok simulate <file.rkg>... [--deployment <profile.json>] [--compare <dir> | --save <dir>] [--ui [--ui-port <port>]]\n",
        )
    );
}

#[test]
fn r078_simulate_with_a_ui_option_but_no_source_is_a_usage_error() {
    assert_missing_simulation_source(&["simulate", "--ui"]);
}

#[test]
fn r078_simulate_alone_names_the_missing_source_and_its_own_usage() {
    assert_missing_simulation_source(&["simulate"]);
}

#[test]
fn r080_each_command_explains_its_own_usage_and_options() {
    for command in ["run", "check", "simulate", "new", "replay", "connectors"] {
        for flag in ["--help", "-h"] {
            let output = cli(&[command, flag]);
            assert_status(&output, 0);
            assert!(stderr(&output).is_empty(), "{command} {flag}: {output:?}");
            let help = stdout(&output);
            assert!(
                help.starts_with(&format!("usage: rastok {command} ")),
                "{help}"
            );
            assert_eq!(help.matches("usage:").count(), 1, "{help}");
            assert!(
                help.contains("\n\n"),
                "the help includes a description: {help}"
            );
            assert!(
                help.contains("-h, --help"),
                "the help lists its options: {help}"
            );
        }
    }
}

#[test]
#[ignore = "R-081, gate P5"]
fn r081_struct_outputs_use_the_lab_json_projection() {
    let output = run_with_input("R-081-struct-output.rkg", "hello\nworld\n");
    assert_status(&output, 0);
    assert!(stderr(&output).is_empty(), "{output:?}");
    assert_eq!(
        stdout(&output),
        "{\"line\":\"hello\",\"size\":5}\n{\"line\":\"world\",\"size\":5}\n"
    );
}

#[test]
#[ignore = "R-081, gate P5"]
fn r081_multiple_outputs_keep_port_names_and_render_their_values() {
    let output = run_with_input("R-081-two-outputs.rkg", "hello\n");
    assert_status(&output, 0);
    assert!(stderr(&output).is_empty(), "{output:?}");
    assert_eq!(stdout(&output), "out: hello\nsize: 5\n");
}

#[test]
#[ignore = "R-081, gate P5"]
fn r081_non_text_stdin_remains_refused_with_the_named_limit() {
    let output = run_with_input("R-081-typed-input.rkg", "{\"id\":\"A\",\"amount\":3}\n");
    assert_status(&output, 1);
    assert!(stdout(&output).is_empty(), "{output:?}");
    assert!(
        stderr(&output)
            .contains("only a `text` App input can be fed from standard input in this build"),
        "{output:?}"
    );
}

#[test]
fn r081_eof_does_not_wait_for_a_pending_deadline_alone() {
    let output = run_with_input("R-081-pending-deadline.rkg", "go\n");
    assert_status(&output, 0);
    assert!(stdout(&output).is_empty(), "{output:?}");
    assert!(stderr(&output).is_empty(), "{output:?}");
}

#[test]
fn r181_a_join_bound_alone_still_does_not_close_its_instance() {
    let file = "R-181-open-instance.rkg";
    assert_check_passes(file);
    let output = cli(&["simulate", file]);
    assert_status(&output, 1);
    assert!(stderr(&output).is_empty(), "{output:?}");
    assert_eq!(
        stdout(&output),
        concat!(
            "FAIL r181.a_decided_request_leaves_its_instance_open\n",
            "     expect app.closed == \"closed\"  (R-181-open-instance.rkg:70)\n",
            "     nothing observed\n",
            "     expected 1 matching delivery on app.closed, but observed 0\n",
            "FAIL r181.an_expired_request_leaves_its_instance_open\n",
            "     expect app.closed == \"closed\"  (R-181-open-instance.rkg:77)\n",
            "     nothing observed\n",
            "     expected 1 matching delivery on app.closed, but observed 0\n",
            "2 scenario(s), 2 failed\n",
        )
    );
}

#[test]
fn r151_a_state_write_is_visible_to_the_next_statement() {
    assert_scenarios_pass(
        "R-151-write-then-read.rkg",
        &["r151.a_write_is_read_by_the_next_statement"],
    );
}
