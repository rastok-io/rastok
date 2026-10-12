//! Gallery composition: generic JSON, exact Session ownership and child reuse.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};

use rastok::journal::{EgressRecord, Record, SessionStatus};
use rastok::{compiler::compile, contract::ContractCatalog, plan::GraphVersion};

const GATEWAY: &str = include_str!("../../../examples/agent-gateway.rkg");

#[test]
fn resident_gateway_steps_deliver_opening_before_frames_like_the_batch_lab() {
    let compilation = compile(&[GATEWAY], &ContractCatalog::try_m0().unwrap()).unwrap();
    let lab = &compilation.labs[0];
    let simulation = &lab.simulations[0];
    let batch = rastok::lab::run(&compilation.plan, GraphVersion(1), lab);
    for (scenario, expected) in simulation.scenarios.iter().zip(batch) {
        let mut session = rastok::lab::Simulator::try_new(
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
        assert!(
            session.is_complete(),
            "stepping must not stall on a resident resource"
        );
        let actual = session.finish();
        assert!(actual.report.passed(), "{:?}", actual.report);
        assert_eq!(
            rastok::golden::diff(
                &rastok::golden::project(&actual.records),
                &rastok::golden::project(&expected.records),
            ),
            None,
            "stepping and batch retain the same causal history"
        );
    }
}

#[test]
fn resident_gateway_scenarios_keep_protocol_frames_on_their_socket() {
    let compilation = compile(&[GATEWAY], &ContractCatalog::try_m0().unwrap()).unwrap();
    let runs = rastok::lab::run(&compilation.plan, GraphVersion(1), &compilation.labs[0]);
    assert_eq!(
        runs,
        rastok::lab::run(&compilation.plan, GraphVersion(1), &compilation.labs[0])
    );
    assert_eq!(runs.len(), 2);
    for run in &runs {
        assert!(run.report.passed(), "{:?}", run.report);
    }

    let statuses: Vec<_> = runs[0]
        .records
        .iter()
        .filter_map(|record| match record {
            Record::Egress(EgressRecord::Session(session))
                if session.connector_path.to_string() == "conversation.model" =>
            {
                Some(session)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        statuses
            .iter()
            .filter(|r| matches!(r.status, SessionStatus::Opening(_)))
            .count(),
        2,
        "nine client messages must use exactly two resident resources"
    );
    let mut writes = BTreeMap::new();
    for record in &statuses {
        if matches!(record.status, SessionStatus::WriteOpen(_)) {
            *writes.entry(record.session_id).or_insert(0) += 1;
        }
    }
    let mut counts: Vec<_> = writes.into_values().collect();
    counts.sort();
    assert_eq!(
        counts,
        [3, 4],
        "the first input belongs to Opening; Bob's later four writes reuse his child"
    );

    // Regression: an external Process frame has its own Session metadata.
    // Routing it must retain the already-selected conversation ancestors,
    // not instantiate another conversation using that new session_id.
    let mut remembered = BTreeSet::new();
    let mut replying = BTreeSet::new();
    for record in &runs[0].records {
        if let Record::Transaction(transaction) = record {
            if transaction.owner_path.to_string() != "conversation.route_reply" {
                continue;
            }
            if transaction
                .state_delta
                .iter()
                .any(|(name, _)| name == "socket")
            {
                remembered.insert(transaction.instance.clone());
            }
            if !transaction.emissions.is_empty() {
                replying.insert(transaction.instance.clone());
            }
        }
    }
    assert_eq!(remembered.len(), 2);
    assert_eq!(
        replying, remembered,
        "a child reply stays in its admitting conversation"
    );
}

/// This marked local-network integration uses a shell JSONL fixture, never
/// Codex, credentials, a network provider or a paid model invocation.
#[cfg(all(unix, feature = "hosted-websocket"))]
#[test]
#[allow(clippy::disallowed_types, clippy::disallowed_methods)]
fn live_websockets_reuse_distinct_resident_children_and_preserve_json() {
    use std::io;
    use std::net::{TcpListener, TcpStream};
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};
    use tungstenite::{Message, WebSocket};

    struct GatewayProcess {
        child: Child,
        directory: std::path::PathBuf,
    }
    impl Drop for GatewayProcess {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            if std::thread::panicking() {
                eprintln!(
                    "gateway stderr: {}",
                    std::fs::read_to_string(self.directory.join("stderr")).unwrap_or_default()
                );
                eprintln!(
                    "gateway stdout: {}",
                    std::fs::read_to_string(self.directory.join("stdout")).unwrap_or_default()
                );
            }
            // Each shell leaves its read loop on EOF when the CLI closes its
            // pipe. The fixture cannot leave an agent process running.
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }
    fn peer(address: std::net::SocketAddr) -> WebSocket<TcpStream> {
        // Allow cold executable loading on the host, independently of the
        // five-second handshake and the bounded per-message I/O below.
        let deadline = Instant::now() + Duration::from_secs(30);
        let stream = loop {
            match TcpStream::connect(address) {
                Ok(stream) => break stream,
                Err(error)
                    if error.kind() == io::ErrorKind::ConnectionRefused
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("local gateway did not bind: {error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        tungstenite::client::client(format!("ws://{address}/agent"), stream)
            .unwrap()
            .0
    }
    fn exchange(peer: &mut WebSocket<TcpStream>, request: serde_json::Value) -> serde_json::Value {
        peer.send(Message::text(request.to_string())).unwrap();
        let response: serde_json::Value = serde_json::from_str(
            peer.read()
                .unwrap_or_else(|error| panic!("no reply to {request}: {error}"))
                .to_text()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            response["request"], request,
            "wire JSON is forwarded without imposing a DSL struct"
        );
        response["pid"].clone()
    }

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let directory =
        std::env::temp_dir().join(format!("rastok-resident-gateway-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let script =
        r#"while IFS= read -r line; do printf '{"pid":%d,"request":%s}\n' "$$" "$line"; done"#;
    let source = GATEWAY
        .replace("127.0.0.1:8443", &address.to_string())
        .replace("command = \"codex\"", "command = \"/bin/sh\"")
        .replace(
            "args = [\"app-server\", \"--listen\", \"stdio://\"]",
            &format!(
                "args = [\"-c\", {}]",
                serde_json::to_string(script).unwrap()
            ),
        );
    let file = directory.join("gateway.rkg");
    std::fs::write(&file, source).unwrap();
    let stderr = std::fs::File::create(directory.join("stderr")).unwrap();
    let stdout = std::fs::File::create(directory.join("stdout")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_rastok"))
        .args(["run"])
        .arg(&file)
        .stdin(Stdio::piped())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .unwrap();
    let _gateway = GatewayProcess { child, directory };
    let mut alice = peer(address);
    let mut bob = peer(address);
    let alice_pid = exchange(
        &mut alice,
        serde_json::json!({"id": 0, "method": "initialize", "params": {"clientInfo": {"name": "alice", "version": "1"}}}),
    );
    let bob_pid = exchange(
        &mut bob,
        serde_json::json!({"id": 0, "method": "initialize", "params": {"clientInfo": {"name": "bob", "version": "1"}}}),
    );
    assert_ne!(
        alice_pid, bob_pid,
        "one resident child per socket conversation"
    );
    assert_eq!(
        exchange(&mut alice, serde_json::json!({"method": "initialized"})),
        alice_pid
    );
    assert_eq!(
        exchange(&mut bob, serde_json::json!({"method": "initialized"})),
        bob_pid
    );
    // Client replies have no method. This is a transparent bridge, not a
    // parser that silently rejects the server-request half of JSON-RPC.
    assert_eq!(
        exchange(
            &mut alice,
            serde_json::json!({"id": 7, "result": {"decision": "decline"}})
        ),
        alice_pid
    );
    alice.close(None).unwrap();
    drop(alice);
    assert_eq!(
        exchange(
            &mut bob,
            serde_json::json!({"id": 1, "method": "turn/start", "params": {"input": [{"type": "text", "text": "still here"}]}})
        ),
        bob_pid
    );
    bob.close(None).unwrap();
}
