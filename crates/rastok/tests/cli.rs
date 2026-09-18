// A test harness drives the OS on purpose; the frontier lints
// guard the core modules, not the evidence.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
//! The binary, end to end: the M0 minimal-evidence chain driven the
//! way a user drives it — `check`, `run` against a real child
//! process, `simulate` against the scripted lab.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Connector probes release a chosen ephemeral port before the child binds it.
/// A concurrently starting UI listener can claim that port, so all test-owned
/// loopback listeners share this guard. Pure and subprocess-only CLI tests
/// remain parallel; this does not claim a port lease against unrelated hosts.
/// The mutex contains no shared data. A failed probe must not poison the
/// remaining probes into reporting its assertion as their own failure.
#[cfg(any(
    feature = "hosted-http12",
    feature = "hosted-websocket",
    feature = "hosted-ui"
))]
static LIVE_NETWORK_CLI_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn example() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/echo.rkg")
}

#[cfg(feature = "hosted-ui")]
fn keyed_retry_example() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ui-two-retries.rkg")
}

fn rastok() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rastok"))
}

#[test]
fn new_creates_one_executable_graph_with_a_lab_without_overwriting() {
    let dir = std::env::temp_dir().join(format!("rastok-new-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temporary directory");
    let path = dir.join("hello-world.rkg");
    let _ = std::fs::remove_file(&path);
    let created = rastok()
        .current_dir(&dir)
        .args(["new", "hello-world"])
        .output()
        .unwrap();
    assert!(created.status.success(), "{created:?}");
    let source = std::fs::read_to_string(&path).unwrap();
    assert!(source.contains("graph hello_world : App"));
    assert!(source.contains("lab hello_world"));
    let simulated = rastok().arg("simulate").arg(&path).output().unwrap();
    assert!(simulated.status.success(), "{simulated:?}");
    assert!(String::from_utf8_lossy(&simulated.stdout).contains("PASS smoke.roundtrip"));
    let repeated = rastok()
        .current_dir(&dir)
        .args(["new", "hello-world"])
        .output()
        .unwrap();
    assert!(!repeated.status.success());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
    for name in [
        "../escape",
        "bad/name",
        "-flag",
        "1name",
        "a\nb",
        "hello.rkg",
    ] {
        let refused = rastok()
            .current_dir(&dir)
            .args(["new", name])
            .output()
            .unwrap();
        assert_eq!(refused.status.code(), Some(2), "{name:?}: {refused:?}");
    }
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(dir).unwrap();
}

#[test]
fn check_accepts_the_example_and_prints_its_hash() {
    let output = rastok()
        .arg("check")
        .arg(example())
        .output()
        .expect("the binary runs");
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("semantic hash"), "{stdout}");
}

#[test]
fn check_renders_a_semantic_refusal_in_the_rustc_shape() {
    let path = std::env::temp_dir().join("rastok-cli-evidence-bad.rkg");
    // A real semantic rule, not a parser accident: an emission
    // leaving by an input port.
    std::fs::write(
        &path,
        "graph g : App {\n  operator o {\n    on in { in = . }\n  }\n  in -> o -> out\n}\n",
    )
    .expect("a writable temp dir");
    let output = rastok()
        .arg("check")
        .arg(&path)
        .output()
        .expect("the binary runs");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[E-PORT-DIRECTION]"), "{stderr}");
    assert!(stderr.contains(":3:"), "line and column: {stderr}");
    assert!(stderr.contains("^^"), "the underline: {stderr}");
}

#[test]
fn check_refuses_the_execution_ui_by_name() {
    let output = rastok()
        .arg("check")
        .arg(example())
        .arg("--ui")
        .output()
        .expect("the binary runs");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--ui is available only for `simulate`"),
        "{stderr}"
    );
}

#[cfg(not(feature = "hosted-ui"))]
#[test]
fn a_reduced_binary_refuses_the_ui_capability_by_name() {
    let output = rastok()
        .arg("simulate")
        .arg(example())
        .arg("--ui")
        .output()
        .expect("the binary runs");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("requires a build with the `hosted-ui` feature"),
        "{stderr}"
    );
}

#[test]
fn a_ui_port_without_the_ui_is_a_usage_error() {
    let output = rastok()
        .arg("simulate")
        .arg(example())
        .args(["--ui-port", "8123"])
        .output()
        .expect("the binary runs");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--ui-port requires --ui"), "{stderr}");
}

#[cfg(feature = "hosted-ui")]
#[test]
fn simulate_ui_starts_paused_and_retains_the_completed_run_until_close() {
    use std::io::{BufRead as _, Read as _};

    let _network_guard = LIVE_NETWORK_CLI_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut child = rastok()
        .arg("simulate")
        .arg(example())
        .arg("--ui")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the UI binary starts");
    let stdout = child.stdout.take().expect("stdout is piped");
    let mut stdout = std::io::BufReader::new(stdout);
    let mut url_line = String::new();
    stdout
        .read_line(&mut url_line)
        .expect("the UI URL is printed");
    let url = url_line
        .trim()
        .strip_prefix("ui: http://")
        .expect("the URL uses the documented prefix");
    let (address, token) = url
        .split_once("/?token=")
        .expect("the URL carries one capability");
    let address: std::net::SocketAddr = address.parse().expect("the address parses");

    let snapshot = ui_snapshot(address, token);
    assert_eq!(snapshot["execution"]["state"], "paused");
    assert_eq!(snapshot["mode"], "simulate");
    assert!(
        snapshot["execution"]["legal_commands"]
            .as_array()
            .is_some_and(|commands| commands.iter().any(|command| command == "step")),
        "the primary Step control is usable at the initial scenario boundary: {snapshot}"
    );
    assert!(snapshot["topology"]["activity"].is_null());
    assert!(
        snapshot["topology"]["edges"]
            .as_array()
            .is_some_and(|edges| edges.iter().all(|edge| {
                edge["queued_count"] == "0" && edge["mailbox"].as_array().is_some_and(Vec::is_empty)
            })),
        "a fresh graph starts with no invented edge traffic: {snapshot}"
    );

    let origin = format!("http://{address}");
    let headers = [
        ("Origin", origin.as_str()),
        ("X-Rastok-Controller", "cli-test"),
    ];
    let claim = ui_http(
        address,
        &format!("POST /api/control/claim?token={token}"),
        &headers,
    );
    assert!(claim.starts_with("HTTP/1.1 200"), "{claim}");
    let transactions_before_action = transaction_observation_count(&snapshot);
    let action = ui_http(
        address,
        &format!("POST /api/command/step?token={token}"),
        &headers,
    );
    assert!(action.starts_with("HTTP/1.1 202"), "{action}");
    let after_action = wait_ui_snapshot(address, token, |snapshot| {
        snapshot["scenario"]["current"]["next_action_index"] == 1
    });
    assert_eq!(
        transaction_observation_count(&after_action),
        transactions_before_action,
        "the initial Step applies one authored action without also running a VM transaction"
    );
    assert_eq!(
        after_action["topology"]["activity"]["kind"],
        "scenario-action"
    );
    assert!(after_action["topology"]["activity"]["node"].is_null());
    assert_eq!(after_action["topology"]["activity"]["ordinal"], 1);
    assert!(
        after_action["topology"]["activity"]["routes"]
            .as_array()
            .is_some_and(|routes| routes
                .iter()
                .any(|route| { route["route"] == ".in>relay.in" && route["phase"] == "admitted" })),
        "the first Step names the exact moving envelope route: {after_action}"
    );
    assert_eq!(
        topology_edge(&after_action, ".in>relay.in")["queued_count"],
        "1",
        "the admitted envelope is currently waiting on its exact edge"
    );
    let mailbox = topology_edge(&after_action, ".in>relay.in")["mailbox"]
        .as_array()
        .expect("the exact mailbox FIFO is inspectable");
    assert_eq!(mailbox.len(), 1);
    assert_eq!(mailbox[0]["position"], 0);
    assert_eq!(mailbox[0]["node"], "relay");
    assert_eq!(mailbox[0]["port"], "in");
    assert_eq!(mailbox[0]["envelope_id"], mailbox[0]["origin_id"]);
    let transactions_before = transaction_observation_count(&after_action);
    let step = ui_http(
        address,
        &format!("POST /api/command/step?token={token}"),
        &headers,
    );
    assert!(step.starts_with("HTTP/1.1 202"), "{step}");
    let mut after_step = wait_ui_snapshot(address, token, |snapshot| {
        snapshot["revision"].as_u64() > after_action["revision"].as_u64()
    });
    assert_eq!(
        transaction_observation_count(&after_step),
        transactions_before + 1,
        "one Next transaction command commits one VM transaction"
    );
    assert_eq!(after_step["topology"]["activity"]["kind"], "lifecycle");
    assert_eq!(after_step["topology"]["activity"]["ordinal"], 2);
    assert_eq!(
        after_step["topology"]["activity"]["routes"],
        serde_json::json!([]),
        "a lifecycle Step pulses its owner without inventing envelope motion"
    );
    assert_eq!(
        after_step["topology"]["activity"]["node"], "",
        "the first VM transaction starts the root App instance"
    );
    let latest_record = after_step["history"]["records"]
        .as_array()
        .and_then(|records| records.last())
        .expect("the lifecycle transaction is retained");
    assert_eq!(latest_record["record_kind"], "instance-opened");
    assert_eq!(latest_record["trigger"], "l:start");
    assert_eq!(latest_record["summary"], "Instance opened · App · gen 0");
    assert_eq!(
        topology_edge(&after_step, "relay.out>shell.in")["queued_count"],
        "0",
        "a lifecycle transaction does not invent an emitted envelope"
    );

    for _ in 0..4 {
        if topology_edge(&after_step, "relay.out>shell.in")["queued_count"] == "1" {
            break;
        }
        let prior_revision = after_step["revision"].as_u64();
        let prior_transactions = transaction_observation_count(&after_step);
        let step = ui_http(
            address,
            &format!("POST /api/command/step?token={token}"),
            &headers,
        );
        assert!(step.starts_with("HTTP/1.1 202"), "{step}");
        after_step = wait_ui_snapshot(address, token, |snapshot| {
            snapshot["revision"].as_u64() > prior_revision
        });
        assert_eq!(
            transaction_observation_count(&after_step),
            prior_transactions + 1,
            "each click still advances exactly one VM transaction"
        );
    }
    assert_eq!(after_step["topology"]["activity"]["node"], "relay");
    assert_eq!(
        topology_node(&after_step, "relay")["firing_count"],
        "2",
        "the Node lifecycle start and input handler are both exact transactions"
    );
    assert_eq!(
        topology_edge(&after_step, "relay.out>shell.in")["queued_count"],
        "1",
        "the emitted envelope is visibly waiting in the downstream mailbox"
    );
    assert_eq!(
        topology_edge(&after_step, "relay.out>shell.in")["mailbox"]
            .as_array()
            .map(Vec::len),
        Some(1),
        "the mailbox inspector and its count describe the same current FIFO"
    );

    for _ in 0..16 {
        if after_step["exchanges"]["calls"]
            .as_array()
            .is_some_and(|calls| calls.iter().any(|call| call["completion_retained"] == true))
        {
            break;
        }
        let prior_revision = after_step["revision"].as_u64();
        let step = ui_http(
            address,
            &format!("POST /api/command/step?token={token}"),
            &headers,
        );
        assert!(step.starts_with("HTTP/1.1 202"), "{step}");
        after_step = wait_ui_snapshot(address, token, |snapshot| {
            snapshot["revision"].as_u64() > prior_revision
        });
    }
    assert!(
        after_step["exchanges"]["calls"]
            .as_array()
            .is_some_and(|calls| { calls.iter().any(|call| call["completion_retained"] == true) }),
        "the returned Call is inspectable before its settlement handler runs: {after_step}"
    );
    assert!(
        after_step["history"]["deliveries"]
            .as_array()
            .is_some_and(|deliveries| !deliveries
                .iter()
                .any(|delivery| { delivery["node"].is_null() && delivery["port"] == "out" })),
        "the retained completion has not yet reached the graph output"
    );

    let run = ui_http(
        address,
        &format!("POST /api/command/run-to-completion?token={token}"),
        &headers,
    );
    assert!(run.starts_with("HTTP/1.1 202"), "{run}");

    let completed = wait_ui_snapshot(address, token, |snapshot| {
        snapshot["execution"]["state"] == "completed"
    });
    assert!(
        completed["history"]["records"]
            .as_array()
            .is_some_and(|records| !records.is_empty()),
        "completed history remains inspectable: {completed}"
    );
    assert!(
        completed["topology"]["edges"]
            .as_array()
            .is_some_and(|edges| edges.iter().all(|edge| edge["queued_count"] == "0")),
        "a completed run has no envelopes left in live mailboxes: {completed}"
    );
    assert!(
        topology_edge(&completed, "relay.out>shell.in")["mailbox"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "the inspector does not confuse causal history with live mailbox contents"
    );
    let continuously_driven_records = completed["history"]["records"].clone();
    let continuously_driven_deliveries = completed["history"]["deliveries"].clone();
    let continuously_driven_steps = completed["topology"]["activity"]["ordinal"].clone();
    let reset = ui_http(
        address,
        &format!("POST /api/command/reset-scenario?token={token}"),
        &headers,
    );
    assert!(reset.starts_with("HTTP/1.1 202"), "{reset}");
    let reset = wait_ui_snapshot(address, token, |snapshot| {
        snapshot["execution"]["state"] == "paused"
            && snapshot["history"]["records"] == serde_json::json!([])
    });
    assert_eq!(reset["scenario"]["current"]["next_action_index"], 0);
    assert_eq!(reset["timeline"]["session"], 1);
    assert_eq!(reset["timeline"]["boundary"], 0);
    let retained = ui_http(
        address,
        &format!(
            "GET /api/snapshot?session=0&boundary={}&token={token}",
            completed["timeline"]["boundary"]
        ),
        &[],
    );
    assert!(retained.starts_with("HTTP/1.1 200"), "{retained}");
    let (_, retained_body) = retained.split_once("\r\n\r\n").expect("HTTP body");
    let retained: serde_json::Value = serde_json::from_str(retained_body).expect("JSON frame");
    assert_eq!(retained["history"], completed["history"]);
    assert_eq!(retained["timeline"]["session"], 0);
    assert_eq!(ui_snapshot(address, token)["timeline"]["session"], 1);
    let branch = ui_http(
        address,
        &format!("POST /api/command/branch-from?session=0&boundary=0&token={token}"),
        &headers,
    );
    assert!(branch.starts_with("HTTP/1.1 400"), "{branch}");
    assert!(branch.ends_with("unknown command"), "{branch}");
    let mut manually_stepped = reset;
    for _ in 0..64 {
        manually_stepped = wait_ui_snapshot(address, token, |snapshot| {
            matches!(
                snapshot["execution"]["state"].as_str(),
                Some("paused" | "completed")
            )
        });
        if manually_stepped["execution"]["state"] == "completed" {
            break;
        }
        let prior_revision = manually_stepped["revision"].as_u64();
        let step = ui_http(
            address,
            &format!("POST /api/command/step?token={token}"),
            &headers,
        );
        assert!(step.starts_with("HTTP/1.1 202"), "{step}");
        manually_stepped = wait_ui_snapshot(address, token, |snapshot| {
            snapshot["revision"].as_u64() > prior_revision
        });
    }
    assert_eq!(
        manually_stepped["execution"]["state"], "completed",
        "repeated Step reaches the ordinary scenario verdict: {manually_stepped}"
    );
    assert_eq!(
        manually_stepped["topology"]["activity"]["ordinal"], continuously_driven_steps,
        "Play and repeated Step cross exactly the same number of UI boundaries"
    );
    assert_eq!(
        manually_stepped["history"]["records"], continuously_driven_records,
        "Play and repeated Step retain the same causal sequence"
    );
    assert_eq!(
        manually_stepped["history"]["deliveries"], continuously_driven_deliveries,
        "Play and repeated Step observe the same delivered envelopes"
    );
    let close = ui_http(
        address,
        &format!("POST /api/command/close?token={token}"),
        &headers,
    );
    assert!(close.starts_with("HTTP/1.1 202"), "{close}");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while child
        .try_wait()
        .expect("child status is readable")
        .is_none()
    {
        if std::time::Instant::now() >= deadline {
            child.kill().expect("a stuck UI can be stopped");
            panic!("the UI process did not close");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let mut remainder = String::new();
    stdout
        .read_to_string(&mut remainder)
        .expect("remaining verdict reads");
    assert!(remainder.contains("PASS smoke.roundtrip"), "{remainder}");
}

#[cfg(feature = "hosted-ui")]
#[test]
fn simulation_step_keeps_a_silent_call_paused_and_reaches_the_next_lab_action() {
    use std::io::BufRead as _;

    let _network_guard = LIVE_NETWORK_CLI_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut child = rastok()
        .arg("simulate")
        .arg(keyed_retry_example())
        .arg("--ui")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the UI binary starts");
    let stdout = child.stdout.take().expect("stdout is piped");
    let mut stdout = std::io::BufReader::new(stdout);
    let mut url_line = String::new();
    stdout
        .read_line(&mut url_line)
        .expect("the UI URL is printed");
    let url = url_line
        .trim()
        .strip_prefix("ui: http://")
        .expect("the URL uses the documented prefix");
    let (address, token) = url
        .split_once("/?token=")
        .expect("the URL carries one capability");
    let address: std::net::SocketAddr = address.parse().expect("the address parses");
    let origin = format!("http://{address}");
    let headers = [
        ("Origin", origin.as_str()),
        ("X-Rastok-Controller", "silent-call-cli-test"),
    ];
    let claim = ui_http(
        address,
        &format!("POST /api/control/claim?token={token}"),
        &headers,
    );
    assert!(claim.starts_with("HTTP/1.1 200"), "{claim}");

    let mut snapshot = ui_snapshot(address, token);
    for _ in 0..16 {
        if snapshot["execution"]["awaiting_outbound_calls"] == 1 {
            break;
        }
        let prior_revision = snapshot["revision"].as_u64();
        let step = ui_http(
            address,
            &format!("POST /api/command/step?token={token}"),
            &headers,
        );
        assert!(step.starts_with("HTTP/1.1 202"), "{step}");
        snapshot = wait_ui_snapshot(address, token, |candidate| {
            candidate["revision"].as_u64() > prior_revision
        });
    }
    assert_eq!(snapshot["execution"]["awaiting_outbound_calls"], 1);
    assert_eq!(snapshot["execution"]["state"], "paused");
    assert_eq!(snapshot["topology"]["activity"]["kind"], "call-boundary");
    assert_eq!(snapshot["topology"]["activity"]["node"], "model");
    assert_eq!(snapshot["topology"]["activity"]["motion"], "outbound");
    assert_eq!(
        snapshot["topology"]["activity"]["detail"],
        "Call opened · model"
    );
    assert!(
        snapshot["execution"]["legal_commands"]
            .as_array()
            .is_some_and(|commands| commands.iter().any(|command| command == "step")),
        "a silent Call cannot strand manual stepping: {snapshot}"
    );

    let prior_revision = snapshot["revision"].as_u64();
    let step = ui_http(
        address,
        &format!("POST /api/command/step?token={token}"),
        &headers,
    );
    assert!(step.starts_with("HTTP/1.1 202"), "{step}");
    let after_expect = wait_ui_snapshot(address, token, |candidate| {
        candidate["revision"].as_u64() > prior_revision
    });
    assert_eq!(after_expect["execution"]["state"], "paused");
    assert_eq!(after_expect["scenario"]["current"]["next_action_index"], 2);
    assert_eq!(after_expect["scenario"]["current"]["last_action"], "expect");
    assert_eq!(after_expect["execution"]["awaiting_outbound_calls"], 1);

    let close = ui_http(
        address,
        &format!("POST /api/command/close?token={token}"),
        &headers,
    );
    assert!(close.starts_with("HTTP/1.1 202"), "{close}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while child
        .try_wait()
        .expect("child status is readable")
        .is_none()
    {
        if std::time::Instant::now() >= deadline {
            child.kill().expect("a stuck UI can be stopped");
            panic!("the UI process did not close");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn run_refuses_the_simulation_only_debugger_before_effects() {
    let output = rastok()
        .arg("run")
        .arg(example())
        .arg("--ui")
        .output()
        .expect("the binary runs");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty(), "no server or Driver starts");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--ui is available only for `simulate`")
    );
}

#[cfg(feature = "hosted-ui")]
#[allow(clippy::expect_used)]
fn ui_snapshot(address: std::net::SocketAddr, token: &str) -> serde_json::Value {
    let response = ui_http(address, &format!("GET /api/snapshot?token={token}"), &[]);
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .expect("the HTTP response has a body");
    serde_json::from_str(body).expect("the snapshot is JSON")
}

#[cfg(feature = "hosted-ui")]
fn wait_ui_snapshot(
    address: std::net::SocketAddr,
    token: &str,
    condition: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let snapshot = ui_snapshot(address, token);
        if condition(&snapshot) {
            return snapshot;
        }
        assert!(std::time::Instant::now() < deadline, "the UI view advances");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[cfg(feature = "hosted-ui")]
fn transaction_observation_count(snapshot: &serde_json::Value) -> usize {
    snapshot["history"]["records"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|record| record["event"] == "rastok.transaction.committed")
        .count()
}

#[cfg(feature = "hosted-ui")]
#[allow(clippy::panic)]
fn topology_edge<'a>(snapshot: &'a serde_json::Value, route: &str) -> &'a serde_json::Value {
    snapshot["topology"]["edges"]
        .as_array()
        .and_then(|edges| edges.iter().find(|edge| edge["route"] == route))
        .unwrap_or_else(|| panic!("topology has no edge `{route}`: {snapshot}"))
}

#[cfg(feature = "hosted-ui")]
#[allow(clippy::panic)]
fn topology_node<'a>(snapshot: &'a serde_json::Value, path: &str) -> &'a serde_json::Value {
    snapshot["topology"]["nodes"]
        .as_array()
        .and_then(|nodes| nodes.iter().find(|node| node["path"] == path))
        .unwrap_or_else(|| panic!("topology has no node `{path}`: {snapshot}"))
}

#[cfg(feature = "hosted-ui")]
#[allow(clippy::expect_used)]
fn ui_http(address: std::net::SocketAddr, request_line: &str, headers: &[(&str, &str)]) -> String {
    use std::io::{Read as _, Write as _};

    let mut stream = std::net::TcpStream::connect(address).expect("the UI accepts loopback");
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .expect("the UI probe is bounded");
    let mut request = format!("{request_line} HTTP/1.1\r\nHost: {address}\r\n");
    for (name, value) in headers {
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("Content-Length: 0\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .expect("the UI request writes");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("the UI response reads");
    response
}

#[test]
fn run_echoes_a_line_through_a_real_child_process() {
    let mut child = rastok()
        .arg("run")
        .arg(example())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary runs");
    child
        .stdin
        .take()
        .expect("a piped stdin")
        .write_all(b"salut\n")
        .expect("stdin accepts");
    let output = child.wait_with_output().expect("the run completes");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "out: salut\n");
}

#[test]
fn run_can_bind_the_real_file_driver_with_an_explicit_root() {
    let root = std::env::temp_dir().join(format!("rastok-cli-file-root-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the File root is writable");
    std::fs::write(root.join("artifact.bin"), b"file payload").expect("the fixture writes");
    let source_path = root.join("file.rkg");
    std::fs::write(
        &source_path,
        r#"graph file_cli : App {
  input in: variant
  output out: bytes
  connector file : File {
    mode = read
    path = "artifact.bin"
  }
  in -> file -> out
  file.exit -> nothing
}
"#,
    )
    .expect("the graph writes");

    let mut child = rastok()
        .arg("run")
        .arg(&source_path)
        .args(["--file-root"])
        .arg(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary runs");
    child
        .stdin
        .take()
        .expect("a piped stdin")
        .write_all(b"read\n")
        .expect("stdin accepts");
    let output = child.wait_with_output().expect("the run completes");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "out: file payload\n"
    );
    std::fs::remove_dir_all(root).expect("the File root is removable");
}

#[test]
fn a_recorded_file_run_replays_after_the_world_and_source_are_removed() {
    let root =
        std::env::temp_dir().join(format!("rastok-cli-recorded-file-{}", std::process::id()));
    std::fs::create_dir(&root).expect("unique recording fixture");
    let source = root.join("file.rkg");
    let artifact = root.join("input.bin");
    let recording = root.join("run.rkrec");
    std::fs::write(&artifact, b"original world\n").unwrap();
    std::fs::write(
        &source,
        r#"graph file_read : App {
  input in: variant
  output out: bytes
  connector file : File { mode = read ; path = "input.bin" }
  in -> file -> out
  file.exit -> nothing
}"#,
    )
    .unwrap();
    let mut child = rastok()
        .arg("run")
        .arg(&source)
        .arg("--file-root")
        .arg(&root)
        .arg("--record")
        .arg(&recording)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"read\n").unwrap();
    let original = child.wait_with_output().unwrap();
    assert!(
        original.status.success(),
        "{}",
        String::from_utf8_lossy(&original.stderr)
    );
    let original_bytes = std::fs::read(&recording).unwrap();
    let rejected = rastok()
        .arg("run")
        .arg(&source)
        .arg("--record")
        .arg(&recording)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert_eq!(
        std::fs::read(&recording).unwrap(),
        original_bytes,
        "existing evidence is never overwritten"
    );
    std::fs::remove_file(&source).unwrap();
    std::fs::remove_file(&artifact).unwrap();
    let replay = rastok()
        .arg("replay")
        .arg(&recording)
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(
        replay.status.success(),
        "{}",
        String::from_utf8_lossy(&replay.stderr)
    );
    assert_eq!(replay.stdout, original.stdout);
    let captured = rastok::recording::Recording::read(&recording).unwrap();
    assert!(captured.finished());
    let seal = rastok::golden::project(captured.records()).seal_hex();
    assert!(String::from_utf8_lossy(&original.stderr).contains(&seal));
    assert!(String::from_utf8_lossy(&replay.stderr).contains(&seal));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(&recording).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn retry_is_public_in_simulation_and_real_recording_without_recalling_the_world() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ui-retry.rkg");
    let simulated = rastok().arg("simulate").arg(&fixture).output().unwrap();
    assert!(simulated.status.success(), "{simulated:?}");
    assert!(String::from_utf8_lossy(&simulated.stdout).contains("PASS retained.second_attempt"));

    let root = std::env::temp_dir().join(format!("rastok-cli-retry-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let source = root.join("retry.rkg");
    let recording = root.join("retry.rkrec");
    let missing = root.join("unavailable-child");
    std::fs::write(
        &source,
        format!(
            r#"
graph app : App {{
  input in: text
  output exit: variant
  connector call : Process + Retry {{
    command = {:?}; codec = lines; behavior = deterministic
    timeout = duration("1s")
    Retry.attempts = 2; Retry.on = [SpawnFailed]
    Retry.delay = {{ kind = fixed; after = duration("1ms") }}
  }}
  in -> call
  call.out -> nothing; call.err -> nothing; call.exit -> exit
}}
"#,
            missing.to_string_lossy()
        ),
    )
    .unwrap();
    let mut child = rastok()
        .arg("run")
        .arg(&source)
        .arg("--record")
        .arg(&recording)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"same request\n")
        .unwrap();
    // EOF requests Closing. Keep admission open until final publication;
    // otherwise the already-admitted request correctly drains as one Call.
    let stdout = child.stdout.take().unwrap();
    let (sent, received) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        use std::io::BufRead as _;
        let mut stdout = std::io::BufReader::new(stdout);
        let mut line = String::new();
        let result = stdout.read_line(&mut line);
        let _ = sent.send((result, line));
    });
    let first = received.recv_timeout(std::time::Duration::from_secs(30));
    if first.is_err() {
        let _ = child.kill();
    }
    let (read, output) = first.expect("bounded Retry publishes its terminal");
    assert!(
        read.unwrap() > 0,
        "the graph publishes a final output before Closing"
    );
    reader.join().unwrap();
    drop(child.stdin.take());
    let original = child.wait_with_output().unwrap();
    assert!(original.status.success(), "{original:?}");
    let captured = rastok::recording::Recording::read(&recording).unwrap();
    assert!(captured.finished());
    use rastok::journal::{OutboundRecord, OutboundStatus, Record, retry_protocol::Fact};
    assert_eq!(
        captured
            .records()
            .iter()
            .filter(|record| matches!(record,
        Record::Outbound(OutboundRecord::Call(call)) if call.status == OutboundStatus::Open))
            .count(),
        2
    );
    let facts: Vec<_> = captured
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(tx) => tx.retry_record.as_ref(),
            _ => None,
        })
        .flat_map(|packet| &packet.facts)
        .collect();
    assert_eq!(
        facts
            .iter()
            .filter(|fact| matches!(fact, Fact::Held(_)))
            .count(),
        1
    );
    assert_eq!(
        facts
            .iter()
            .filter(|fact| matches!(fact, Fact::Consumed(_)))
            .count(),
        1
    );
    assert_eq!(
        facts
            .iter()
            .filter(|fact| matches!(fact, Fact::Published(_)))
            .count(),
        1
    );
    std::fs::remove_file(&source).unwrap();
    let replay = rastok()
        .arg("replay")
        .arg(&recording)
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(replay.status.success(), "{replay:?}");
    assert_eq!(replay.stdout, output.as_bytes());
    let seal = rastok::golden::project(captured.records()).seal_hex();
    assert!(String::from_utf8_lossy(&replay.stderr).contains(&seal));
    assert!(!missing.exists());
    std::fs::remove_file(&recording).unwrap();
    std::fs::remove_dir(&root).unwrap();
}

#[test]
fn the_gallery_file_pipeline_replaces_once_and_replay_does_not_touch_the_world() {
    let root = std::env::temp_dir().join(format!("rastok-gallery-file-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let recording = root.join("copy.rkrec");
    std::fs::write(root.join("input.txt"), b"alpha\nbeta\n").unwrap();
    let mut child = rastok()
        .arg("run")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/file-pipeline.rkg"))
        .arg("--file-root")
        .arg(&root)
        .arg("--record")
        .arg(&recording)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"input.txt\n")
        .unwrap();
    let original = child.wait_with_output().unwrap();
    assert!(original.status.success(), "{original:?}");
    assert_eq!(
        std::fs::read(root.join("copy.txt")).unwrap(),
        b"alpha\nbeta\n"
    );
    std::fs::write(root.join("input.txt"), b"changed world").unwrap();
    std::fs::write(root.join("copy.txt"), b"do not overwrite").unwrap();
    let replayed = rastok()
        .arg("replay")
        .arg(&recording)
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(replayed.status.success(), "{replayed:?}");
    assert_eq!(original.stdout, replayed.stdout);
    assert_eq!(
        std::fs::read(root.join("input.txt")).unwrap(),
        b"changed world"
    );
    assert_eq!(
        std::fs::read(root.join("copy.txt")).unwrap(),
        b"do not overwrite"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn recording_personal_data_refuses_before_creating_a_file() {
    let root =
        std::env::temp_dir().join(format!("rastok-cli-record-personal-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let source = root.join("personal.rkg");
    let recording = root.join("forbidden.rkrec");
    std::fs::write(
        &source,
        "graph g : App { input in: text @personal ; output out: text @personal ; in -> out }",
    )
    .unwrap();
    let output = rastok()
        .arg("run")
        .arg(&source)
        .arg("--record")
        .arg(&recording)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("persistent retention/erasure"));
    assert!(!recording.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(feature = "hosted-http12")]
#[test]
fn run_polls_an_inbound_http_server_without_a_stdin_submission() {
    use std::io::Read as _;

    let _network_guard = LIVE_NETWORK_CLI_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let reserved = std::net::TcpListener::bind("127.0.0.1:0").expect("a local port is available");
    let address = reserved
        .local_addr()
        .expect("the local port has an address");
    drop(reserved);
    let root = std::env::temp_dir().join(format!("rastok-cli-http-server-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the HTTP CLI fixture directory is writable");
    let source = root.join("server.rkg");
    std::fs::write(
        &source,
        format!(
            r#"graph http_cli : App {{
  connector server : HttpServer {{
    listen = "{address}"
    codec = text
    versions = ["http/1.1"]
  }}
  server -> out
  server.exit -> nothing
}}
"#,
        ),
    )
    .expect("the HTTP CLI source writes");
    // Validate the fixture before timing its I/O boundary. Cold executable
    // loading is not an HTTP response/handshake budget.
    let checked = rastok().arg("check").arg(&source).output().unwrap();
    assert!(checked.status.success(), "{checked:?}");
    let mut child = rastok()
        .arg("run")
        .arg(&source)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the hosted binary starts");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut stream = loop {
        match std::net::TcpStream::connect(address) {
            Ok(stream) => break stream,
            Err(_) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => panic!("the standalone HttpServer never started: {error}"),
        }
    };
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .expect("the client read is bounded");
    stream
        .write_all(
            b"POST / HTTP/1.1\r\nHost: localhost\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\nping",
        )
        .expect("the HTTP request writes");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("the committed acknowledgement returns");
    assert!(
        response.starts_with(b"HTTP/1.1 202"),
        "unexpected standalone response: {:?}",
        String::from_utf8_lossy(&response)
    );
    child.kill().expect("the long-lived server can be stopped");
    let _ = child.wait();
    std::fs::remove_dir_all(root).expect("the HTTP CLI fixture is removable");
}

#[cfg(feature = "hosted-websocket")]
#[test]
fn run_composes_a_live_websocket_gateway_with_process() {
    let _network_guard = LIVE_NETWORK_CLI_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let reserved =
        std::net::TcpListener::bind("127.0.0.1:0").expect("a local WebSocket port is available");
    let address = reserved
        .local_addr()
        .expect("the local WebSocket port has an address");
    drop(reserved);
    let root = std::env::temp_dir().join(format!(
        "rastok-cli-websocket-gateway-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the WebSocket fixture directory is writable");
    let source = root.join("gateway.rkg");
    std::fs::write(
        &source,
        format!(
            r#"graph websocket_cli : App {{
  connector echo : Process {{
    command = "cat"
    mode = job
    codec = lines
    timeout = duration("5s")
    max_output = "1MiB"
    idempotent = true; behavior = deterministic
  }}
  connector socket : WebSocketServer {{
    listen = "{address}"
    path = "/echo"
    codec = text
    handshake_timeout = duration("5s")
    max_message = "1MiB"
    correlate = meta.ext.session_id
    versions = ["http/1.1"]
  }}
  socket.out -> echo.in
  echo.out -> socket.in
  socket.exit -> nothing
  echo.err -> nothing
  echo.exit -> nothing
}}
"#,
        ),
    )
    .expect("the WebSocket gateway source writes");
    let checked = rastok().arg("check").arg(&source).output().unwrap();
    assert!(checked.status.success(), "{checked:?}");
    let mut child = rastok()
        .arg("run")
        .arg(&source)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the hosted binary starts");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let stream = loop {
        match std::net::TcpStream::connect(address) {
            Ok(stream) => break stream,
            Err(_) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => {
                let status = child.try_wait().expect("the gateway status is readable");
                if status.is_none() {
                    child.kill().expect("the late gateway can be stopped");
                }
                let output = child
                    .wait_with_output()
                    .expect("the gateway output is readable");
                panic!(
                    "the standalone WebSocketServer never started: {error}; \
                     status before cleanup: {status:?}; stderr: {}",
                    String::from_utf8_lossy(&output.stderr),
                );
            }
        }
    };
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .expect("the WebSocket peer read is bounded");
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(5)))
        .expect("the WebSocket peer write is bounded");
    let request = format!("ws://{address}/echo");
    let (mut peer, response) = match tungstenite::client::client(request, stream) {
        Ok(upgrade) => upgrade,
        Err(error) => {
            let status = child.try_wait().expect("the gateway status is readable");
            if status.is_none() {
                child.kill().expect("the failed gateway can be stopped");
            }
            let output = child
                .wait_with_output()
                .expect("the gateway output is readable");
            panic!(
                "an independent peer failed HTTP/1.1 Upgrade: {error:?}; \
                 gateway status before cleanup: {status:?}; stderr: {}",
                String::from_utf8_lossy(&output.stderr),
            );
        }
    };
    assert_eq!(
        response.status(),
        tungstenite::http::StatusCode::SWITCHING_PROTOCOLS
    );
    peer.send(tungstenite::Message::text("hello"))
        .expect("the peer sends one complete text message");
    assert_eq!(
        peer.read().expect("the gateway returns one message"),
        tungstenite::Message::text("hello")
    );
    peer.close(None).expect("the peer closes cleanly");

    child.kill().expect("the long-lived gateway can be stopped");
    let _ = child.wait();
    std::fs::remove_dir_all(root).expect("the WebSocket fixture is removable");
}

#[test]
fn simulate_runs_the_lab_and_reports_every_scenario() {
    let output = rastok()
        .arg("simulate")
        .arg(example())
        .output()
        .expect("the binary runs");
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS smoke.roundtrip"), "{stdout}");
    assert!(stdout.contains("PASS smoke.silence_expires"), "{stdout}");
    assert!(stdout.contains("3 scenario(s), 0 failed"), "{stdout}");
}

#[test]
fn simulate_resolves_boot_refs_from_a_profile_without_cli_secret_values() {
    let root = std::env::temp_dir().join(format!(
        "rastok-cli-deployment-profile-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the deployment fixture directory is writable");
    let source = root.join("app.rkg");
    let profile = root.join("deployment.json");
    std::fs::write(
        &source,
        r#"graph deployed : App {
  connector client : HttpClient {
    url = env("SERVICE_URL")
    auth_bearer = secret("providers/http/token")
    method = "POST"
    timeout = duration("1s")
    codec = text
    idempotent = true; behavior = deterministic
  }
  in -> client -> out
  client.exit -> nothing
}
lab deployed {
  simulation boot on app {
    mock client {
      on in {
        out = "ok"
        exit = Completed { status = 200 }
      }
    }
    scenario configured "boot refs bind before execution" {
      app.in = "request"
      expect "reply" app.out count == 1 where data == "ok"
    }
  }
}
"#,
    )
    .expect("the deployment graph writes");
    std::fs::write(
        &profile,
        r#"{
  "environment":[{"name":"SERVICE_URL","value":"https://example.test"}],
  "secrets":[{
    "reference":"providers/http/token",
    "environment":"RASTOK_TEST_API_TOKEN",
    "version":"v1"
  }]
}"#,
    )
    .expect("the deployment profile writes");

    let output = rastok()
        .arg("simulate")
        .arg(&source)
        .arg("--deployment")
        .arg(&profile)
        .env("RASTOK_TEST_API_TOKEN", "physical-cli-secret")
        .env("RASTOK_TEST_API_TOKEN_VERSION", "v1")
        .output()
        .expect("the binary runs with an out-of-band secret");
    assert!(output.status.success(), "{output:?}");
    let inspected = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(inspected.contains("PASS boot.configured"), "{inspected}");
    assert!(!inspected.contains("physical-cli-secret"));
    std::fs::remove_dir_all(root).expect("the deployment fixture is removable");
}

#[test]
fn simulate_fails_loudly_on_a_broken_expectation() {
    let source = std::fs::read_to_string(example()).expect("the example reads");
    let path = std::env::temp_dir().join("rastok-cli-evidence-broken.rkg");
    std::fs::write(
        &path,
        source.replace("data == \"salut\"", "data == \"autre\""),
    )
    .expect("a writable temp dir");
    let output = rastok()
        .arg("simulate")
        .arg(&path)
        .output()
        .expect("the binary runs");
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("FAIL smoke.roundtrip"), "{stdout}");
    assert!(stdout.contains("the echo"), "the label names it: {stdout}");
}

#[test]
fn simulate_blesses_then_verifies_a_golden_directory() {
    let dir = std::env::temp_dir().join("rastok-cli-golden");
    let _ = std::fs::remove_dir_all(&dir);
    // Bless…
    let output = rastok()
        .args(["simulate"])
        .arg(example())
        .args(["--golden"])
        .arg(&dir)
        .args(["--bless"])
        .output()
        .expect("the binary runs");
    assert!(output.status.success(), "{output:?}");
    assert!(dir.join("smoke.roundtrip.trace").exists());
    // …verify…
    let output = rastok()
        .args(["simulate"])
        .arg(example())
        .args(["--golden"])
        .arg(&dir)
        .output()
        .expect("the binary runs");
    assert!(output.status.success(), "{output:?}");
    // …and a staled golden is named, not hex-dumped.
    let path = dir.join("smoke.roundtrip.trace");
    let stored = std::fs::read_to_string(&path).expect("the blessed trace");
    std::fs::write(&path, stored.replace("73616c7574", "6175747265")).expect("the alteration");
    let output = rastok()
        .args(["simulate"])
        .arg(example())
        .args(["--golden"])
        .arg(&dir)
        .output()
        .expect("the binary runs");
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("FAIL smoke.roundtrip"), "{stdout}");
    assert!(stdout.contains("golden divergence"), "{stdout}");
    assert!(
        stdout.contains("expected \"autre\"; got \"salut\""),
        "{stdout}"
    );
    assert!(stdout.contains("cause: INBOUND"), "{stdout}");
    assert!(stdout.contains("sequence 0"), "{stdout}");
    assert!(
        stdout.contains("data(8)"),
        "the diff names the key: {stdout}"
    );
    for malformed in ["a\n", "€1\n", "zz\n"] {
        std::fs::write(&path, malformed).expect("malformed golden probe");
        let output = rastok()
            .arg("simulate")
            .arg(example())
            .arg("--golden")
            .arg(&dir)
            .output()
            .expect("the binary runs");
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains("the stored golden is unreadable by this build"),
            "malformed hexadecimal refuses without a parsing panic: {output:?}"
        );
    }
}

#[test]
fn bless_without_golden_is_a_usage_error() {
    let output = rastok()
        .args(["simulate"])
        .arg(example())
        .args(["--bless"])
        .output()
        .expect("the binary runs");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--bless requires --golden"), "{stderr}");
}

#[test]
fn resolve_openapi_updates_and_checks_one_generated_project_source() {
    let root = std::env::temp_dir().join(format!("rastok-cli-openapi-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the project root is writable");
    std::fs::write(
        root.join("app.rkg"),
        r#"graph api : App {
  connector payment : HttpClient {
    url = "https://api.example.test/payments"
    method = "POST"
    timeout = duration("1s")
    codec = json
    idempotent = true
    behavior = deterministic
  }
}"#,
    )
    .expect("the Rastok source writes");
    std::fs::write(
        root.join("openapi.yaml"),
        r#"openapi: 3.1.0
paths:
  /payments:
    post:
      responses: {}
components:
  schemas:
    Payment:
      type: object
      additionalProperties: false
      required: [id]
      properties:
        id: { type: string, format: uuid }
"#,
    )
    .expect("the OpenAPI source writes");

    let output = rastok()
        .arg("--resolve-open-api")
        .arg(&root)
        .output()
        .expect("the resolver runs");
    assert!(output.status.success(), "{output:?}");
    assert!(root.join("openapi.generated.rkg").exists());

    let checked = rastok()
        .arg("--resolve-open-api")
        .arg(&root)
        .arg("--check")
        .output()
        .expect("check mode runs");
    assert!(checked.status.success(), "{checked:?}");
    std::fs::remove_dir_all(root).expect("the project root is removable");
}
