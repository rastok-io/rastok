// A test harness drives the OS on purpose; the frontier lints
// guard the core modules, not the evidence.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
//! The binary, end to end: the M0 minimal-evidence chain driven the
//! way a user drives it — `check`, `run` against a real child
//! process, `simulate` against the scripted lab.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn example() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/echo.rkg")
}

fn rastok() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rastok"))
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
        "graph g : App {\n  operator o {\n    on in { in = . }\n  }\n  . -> o -> .\n}\n",
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
  . -> file -> .
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

#[cfg(feature = "hosted-http12")]
#[test]
fn run_polls_an_inbound_http_server_without_a_stdin_submission() {
    use std::io::Read as _;

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
  server -> .
}}
"#,
        ),
    )
    .expect("the HTTP CLI source writes");
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
    effect = idempotent ; behavior = deterministic
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
            Err(error) => panic!("the standalone WebSocketServer never started: {error}"),
        }
    };
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .expect("the WebSocket peer read is bounded");
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(5)))
        .expect("the WebSocket peer write is bounded");
    let request = format!("ws://{address}/echo");
    let (mut peer, response) = tungstenite::client::client(request, stream)
        .expect("an independent peer completes HTTP/1.1 Upgrade");
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
        stdout.contains("data(8)"),
        "the diff names the key: {stdout}"
    );
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
    effect = idempotent
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
