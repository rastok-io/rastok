//! The standalone teaching page composes existing contracts, not a scheduler.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::Path;

use rastok::journal::{OutboundRecord, Record, SessionStatus};
use rastok::{compiler, contract::ContractCatalog, plan::GraphVersion};

const CHAT: &str = include_str!("../../../examples/agent-chat.rkg");
const HTML: &str = include_str!("../../../examples/agent-chat.html");

fn compile_chat() -> compiler::Compilation {
    compiler::compile_with_assets(
        &[CHAT],
        &ContractCatalog::try_m0().unwrap(),
        Some(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")),
    )
    .unwrap()
}

#[test]
fn the_readable_page_is_the_exact_content_sealed_in_the_plan() {
    let compilation = compile_chat();
    assert_eq!(compilation.plan.assets.len(), 1);
    assert_eq!(compilation.plan.assets[0].bytes, HTML.as_bytes());
    let package = rastok::plan::encode(&compilation.plan, &compilation.hash);
    let (restored, hash) = rastok::plan::decode(&package).unwrap();
    assert_eq!(hash, compilation.hash);
    assert_eq!(restored.assets, compilation.plan.assets);
}

#[test]
fn nicknames_own_distinct_reused_children_and_rejected_messages_open_none() {
    let compilation = compile_chat();
    let run = || {
        rastok::lab::run_program(
            &compilation.plan,
            compilation.hash,
            GraphVersion(1),
            &compilation.labs[0],
            None,
        )
    };
    let runs = run();
    assert_eq!(
        runs,
        run(),
        "all scripted protocol facts replay identically"
    );
    assert_eq!(runs.len(), 3);
    let mut openings = Vec::new();
    for run in &runs {
        assert!(run.report.passed(), "{:?}", run.report);
        openings.push(
            run.records
                .iter()
                .filter(|record| {
                    matches!(record, Record::Outbound(OutboundRecord::Session(session))
                        if session.connector_path.to_string() == "conversation.model"
                        && matches!(session.status, SessionStatus::Opening(_)))
                })
                .count(),
        );
    }
    assert_eq!(openings, [2, 1, 0]);
    let mut writes = BTreeMap::new();
    for record in &runs[0].records {
        if let Record::Outbound(OutboundRecord::Session(session)) = record
            && session.connector_path.to_string() == "conversation.model"
            && matches!(session.status, SessionStatus::WriteOpen(_))
        {
            *writes.entry(session.session_ref).or_insert(0) += 1;
        }
    }
    let mut counts: Vec<_> = writes.into_values().collect();
    counts.sort();
    assert_eq!(counts, [3, 4], "Bob's last turn reuses his original child");
}

#[test]
fn the_chat_lab_has_the_same_causal_history_when_stepped() {
    let compilation = compile_chat();
    let lab = &compilation.labs[0];
    let simulation = &lab.simulations[0];
    let batch = rastok::lab::run(&compilation.plan, GraphVersion(1), lab);
    for (scenario, expected) in simulation.scenarios.iter().zip(batch) {
        let mut session = rastok::lab::ScenarioSession::try_new(
            &compilation.plan,
            GraphVersion(1),
            &simulation.name,
            &simulation.mocks,
            scenario,
            None,
        )
        .unwrap();
        for _ in 0..1000 {
            if session.is_complete() {
                break;
            }
            if session.is_quiescent() {
                session.next_action().unwrap();
            } else {
                session.step_transaction().unwrap();
            }
        }
        assert!(session.is_complete(), "stepping never stalls on a listener");
        let actual = session.finish();
        assert!(actual.report.passed(), "{:?}", actual.report);
        assert_eq!(
            rastok::golden::diff(
                &rastok::golden::project(&actual.records),
                &rastok::golden::project(&expected.records),
            ),
            None,
        );
    }
}

/// Marked local-network integration: real HTTP, WebSockets and harmless shell
/// children. It never launches Codex, uses credentials or invokes a model.
#[cfg(all(unix, feature = "hosted-websocket"))]
#[test]
#[allow(clippy::disallowed_types, clippy::disallowed_methods)]
fn http_remains_available_while_two_children_talk_and_one_quits() {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};
    use tungstenite::{Message, WebSocket};

    struct Server {
        child: Child,
        directory: std::path::PathBuf,
    }
    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            if std::thread::panicking() {
                for name in ["stderr", "stdout"] {
                    eprintln!(
                        "chat {name}: {}",
                        std::fs::read_to_string(self.directory.join(name)).unwrap_or_default()
                    );
                }
            }
            // The shell's bounded read loop ends on pipe EOF; there is no
            // detached subprocess or agent left behind by a failed assertion.
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }
    fn connect(address: SocketAddr) -> TcpStream {
        let deadline = Instant::now() + Duration::from_secs(30);
        let stream = loop {
            match TcpStream::connect(address) {
                Ok(stream) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::ConnectionRefused
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("chat listener did not bind: {error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
    }
    fn page(address: SocketAddr, expected: &str) {
        let mut stream = connect(address);
        write!(
            stream,
            "GET / HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let (headers, body) = response.split_once("\r\n\r\n").unwrap();
        assert!(headers.starts_with("HTTP/1.1 200"), "{headers}");
        assert!(
            headers
                .to_ascii_lowercase()
                .contains("content-type: text/html; charset=utf-8"),
            "{headers}"
        );
        assert_eq!(body, expected);
    }
    fn peer(address: SocketAddr) -> WebSocket<TcpStream> {
        tungstenite::client::client(format!("ws://{address}/chat"), connect(address))
            .unwrap()
            .0
    }
    fn exchange(peer: &mut WebSocket<TcpStream>, frame: serde_json::Value) -> serde_json::Value {
        peer.send(Message::text(frame.to_string())).unwrap();
        serde_json::from_str(peer.read().unwrap().to_text().unwrap()).unwrap()
    }
    fn request(
        peer: &mut WebSocket<TcpStream>,
        name: &str,
        rpc: serde_json::Value,
    ) -> serde_json::Value {
        let reply = exchange(
            peer,
            serde_json::json!({"nickname": name, "kind": "rpc", "rpc": rpc}),
        );
        assert_eq!(reply["kind"], "rpc");
        assert_eq!(reply["rpc"]["request"], rpc);
        reply["rpc"]["pid"].clone()
    }

    let http = TcpListener::bind("127.0.0.1:0").unwrap();
    let websocket = TcpListener::bind("127.0.0.1:0").unwrap();
    let http_address = http.local_addr().unwrap();
    let ws_address = websocket.local_addr().unwrap();
    drop((http, websocket));
    let directory = std::env::temp_dir().join(format!("rastok-chat-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    // If the example's launch spelling changes, refuse the fixture instead
    // of accidentally running its real Codex command during qualification.
    assert_eq!(CHAT.matches("command = \"codex\"").count(), 1);
    assert_eq!(
        CHAT.matches("args = [\"app-server\", \"--listen\", \"stdio://\"]")
            .count(),
        1
    );
    let script =
        r#"while IFS= read -r line; do printf '{"pid":%d,"request":%s}\n' "$$" "$line"; done"#;
    let source = CHAT
        .replace("127.0.0.1:8777", &http_address.to_string())
        .replace("127.0.0.1:8778", &ws_address.to_string())
        .replace("command = \"codex\"", "command = \"/bin/sh\"")
        .replace(
            "args = [\"app-server\", \"--listen\", \"stdio://\"]",
            &format!(
                "args = [\"-c\", {}]",
                serde_json::to_string(script).unwrap()
            ),
        );
    let html = HTML.replace("127.0.0.1:8778", &ws_address.to_string());
    let file = directory.join("chat.rkg");
    std::fs::write(&file, source).unwrap();
    std::fs::write(directory.join("agent-chat.html"), &html).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_rastok"))
        .arg("run")
        .arg(&file)
        .stdin(Stdio::piped())
        .stdout(std::fs::File::create(directory.join("stdout")).unwrap())
        .stderr(std::fs::File::create(directory.join("stderr")).unwrap())
        .spawn()
        .unwrap();
    let server = Server { child, directory };

    page(http_address, &html);
    let mut alice = peer(ws_address);
    let mut bob = peer(ws_address);
    let initialize = serde_json::json!({"id": 0, "method": "initialize", "params": {}});
    let alice_pid = request(&mut alice, "alice", initialize.clone());
    let bob_pid = request(&mut bob, "bob", initialize.clone());
    assert_ne!(alice_pid, bob_pid, "different names own different children");
    let mut duplicate = peer(ws_address);
    assert_eq!(
        exchange(
            &mut duplicate,
            serde_json::json!({"nickname": "alice", "kind": "quit"})
        )["kind"],
        "error"
    );
    duplicate.close(None).unwrap();
    drop(duplicate);
    page(http_address, &html);
    assert_eq!(request(&mut alice, "alice", initialize.clone()), alice_pid);
    assert_eq!(request(&mut bob, "bob", initialize.clone()), bob_pid);
    assert_eq!(
        exchange(
            &mut alice,
            serde_json::json!({"nickname": "alice", "kind": "quit"})
        )["kind"],
        "closing"
    );
    alice.close(None).unwrap();
    drop(alice);
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let output = std::fs::read_to_string(server.directory.join("stdout")).unwrap();
        if output
            .lines()
            .any(|line| line.starts_with("process_exit: "))
        {
            assert_eq!(
                output
                    .lines()
                    .filter(|line| line.starts_with("process_exit: "))
                    .count(),
                1
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Alice's child must reach a terminal"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(request(&mut bob, "bob", initialize), bob_pid);
    page(http_address, &html);
    bob.close(None).unwrap();
}
