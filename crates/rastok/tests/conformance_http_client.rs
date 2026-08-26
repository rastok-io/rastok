//! `HttpClient` through the connector-neutral Call battery.
//!
//! A deterministic loopback HTTP/1.1 peer supplies portable world facts.
//! Protocol-specific H2/H3 tests live beside the private network engine; this
//! file proves that the hosted Driver obeys the generic journal and terminal
//! contract seen by every Call Connector.

#![cfg(feature = "hosted-http12")]
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::expect_used
)]

use rastok::conformance::{
    InterleaveExpectation, InterleaveProbe, Profile, Provocation, assert_conformant, verify,
};
use rastok::connectors::{DriverRegistry, HttpClientConfig, HttpClientFactory};
use rastok::contract::ContractCatalog;
use rastok::envelope::{Data, Value};
use rastok::node;
use rastok::stdlib::HTTP_CLIENT;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

struct LoopbackServer {
    address: SocketAddr,
    stopping: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl LoopbackServer {
    fn start() -> Self {
        let listener =
            TcpListener::bind("127.0.0.1:0").expect("the HTTP conformance listener binds");
        let address = listener
            .local_addr()
            .expect("the HTTP conformance listener has an address");
        let stopping = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&stopping);
        let thread = std::thread::spawn(move || {
            while let Ok((stream, _)) = listener.accept() {
                if observed.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::spawn(move || serve(stream));
            }
        });
        Self {
            address,
            stopping,
            thread: Some(thread),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    fn tls_url(&self, path: &str) -> String {
        format!("https://localhost:{}{path}", self.address.port())
    }
}

impl Drop for LoopbackServer {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("the HTTP conformance server joins");
        }
    }
}

fn serve(mut stream: TcpStream) {
    stream
        .set_read_timeout(Some(std::time::Duration::from_millis(250)))
        .expect("the conformance stream accepts a finite read timeout");
    let mut request = Vec::new();
    let mut chunk = [0u8; 4 * 1024];
    loop {
        let Ok(read) = stream.read(&mut chunk) else {
            return;
        };
        if read == 0 {
            return;
        }
        request.extend_from_slice(&chunk[..read]);
        if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            break;
        }
        if request.len() > 64 * 1024 {
            return;
        }
    }
    let Ok(request) = std::str::from_utf8(&request) else {
        // A TLS ClientHello sent to the plain loopback peer is the stable
        // TlsFailed provocation. Closing here supplies no HTTP ambiguity.
        return;
    };
    let Some(path) = request
        .lines()
        .next()
        .and_then(|line| line.split_ascii_whitespace().nth(1))
    else {
        return;
    };
    let (delay, content_type, body): (_, _, &[u8]) = match path {
        "/happy" => (0, "text/plain", b"happy-response"),
        "/large" => (0, "text/plain", b"too-large"),
        "/bad-json" => (0, "application/json", b"not-json"),
        "/first" => (40, "text/plain", b"first-result"),
        "/second" => (1, "text/plain", b"second-result"),
        "/mute" => {
            std::thread::sleep(std::time::Duration::from_millis(500));
            return;
        }
        _ => (0, "text/plain", b"unknown"),
    };
    if delay != 0 {
        std::thread::sleep(std::time::Duration::from_millis(delay));
    }
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\nx-repeat: a\r\nx-repeat: b\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len(),
    );
    stream
        .write_all(response.as_bytes())
        .expect("the conformance peer writes response headers");
    stream
        .write_all(body)
        .expect("the conformance peer writes the response body");
}

fn probe(name: &str, url: &str, codec: &str, timeout: &str, max_body: &str) -> String {
    let route = if codec == "text" {
        r#"
  operator body {
    input in: struct { text: text }
    on in { out = data.text }
  }
  . -> body -> client -> .
"#
    } else {
        "\n  . -> client -> .\n"
    };
    format!(
        r#"
graph {name} : App {{
  connector client : HttpClient {{
    url = "{url}"
    method = "POST"
    timeout = duration("{timeout}")
    codec = {codec}
    versions = ["http/1.1"]
    max_body = "{max_body}"
    effect = idempotent ; behavior = deterministic
  }}
{route}}}
"#,
    )
}

fn pair_probe(server: &LoopbackServer) -> String {
    format!(
        r#"
graph pair : App {{
  connector left : HttpClient {{
    url = "{}" ; timeout = duration("1s")
    codec = text ; versions = ["http/1.1"]
    effect = idempotent ; behavior = deterministic
  }}
  connector right : HttpClient {{
    url = "{}" ; timeout = duration("1s")
    codec = text ; versions = ["http/1.1"]
    effect = idempotent ; behavior = deterministic
  }}
  operator a {{
    input in: struct {{ a: text, b: text }}
    on in {{ out = data.a }}
  }}
  operator b {{
    input in: struct {{ a: text, b: text }}
    on in {{ out = data.b }}
  }}
  . -> a -> left -> .
  . -> b -> right -> .
}}
"#,
        server.url("/first"),
        server.url("/second"),
    )
}

fn unused_address() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("the refusal address can be reserved");
    listener.local_addr().expect("the refusal address is known")
}

fn registry() -> DriverRegistry {
    DriverRegistry::default()
        .try_with(Box::new(
            HttpClientFactory::try_new(HttpClientConfig::default())
                .expect("the HttpClient id is canonical"),
        ))
        .expect("HttpClient is registered once")
}

#[test]
fn http_client_passes_the_connector_neutral_call_battery() {
    let server = LoopbackServer::start();
    let unreachable = unused_address();
    let profile = Profile {
        contract: &HTTP_CLIENT,
        source: probe("happy", &server.url("/happy"), "text", "1s", "1KiB"),
        input: "request",
        happy_exit_case: "Completed",
        provocations: vec![
            Provocation {
                exit_case: "ConnectFailed",
                source: probe(
                    "connect_failed",
                    &format!("http://{unreachable}/missing"),
                    "json",
                    "1s",
                    "1KiB",
                ),
                input: "request",
            },
            Provocation {
                exit_case: "TlsFailed",
                source: probe("tls_failed", &server.tls_url("/tls"), "json", "1s", "1KiB"),
                input: "request",
            },
            Provocation {
                exit_case: "ProtocolFailed",
                source: probe(
                    "protocol_failed",
                    "ftp://example.invalid/request",
                    "json",
                    "1s",
                    "1KiB",
                ),
                input: "request",
            },
            Provocation {
                exit_case: "BodyTooLarge",
                source: probe("large", &server.url("/large"), "text", "1s", "4B"),
                input: "x",
            },
            Provocation {
                exit_case: "DecodeFailed",
                source: probe("bad_json", &server.url("/bad-json"), "json", "1s", "1KiB"),
                input: "request",
            },
        ],
        unprovable: vec![
            (
                "DnsFailed",
                "a portable test cannot replace the host resolver through the public Driver",
            ),
            (
                "OutcomeUnknown",
                "bounded Closing, not the HttpClient Driver, mints this case after settlement",
            ),
        ],
        mute_source: Some(probe("mute", &server.url("/mute"), "text", "50ms", "1KiB")),
        interleave: Some(InterleaveProbe {
            source: pair_probe(&server),
            a: "first-request",
            b: "second-request",
            expected: vec![
                InterleaveExpectation {
                    connector: node::Path::new(["left"]).expect("the left path is valid"),
                    data: Data::materialized(Value::Text(String::from("first-result"))),
                },
                InterleaveExpectation {
                    connector: node::Path::new(["right"]).expect("the right path is valid"),
                    data: Data::materialized(Value::Text(String::from("second-result"))),
                },
            ],
        }),
    };
    let report = verify(
        &profile,
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        &registry(),
    );

    assert_conformant(&report, &["DnsFailed", "OutcomeUnknown"]);
    assert_eq!(
        report.proven_cases,
        [
            "Completed",
            "ConnectFailed",
            "TlsFailed",
            "ProtocolFailed",
            "BodyTooLarge",
            "DecodeFailed",
            "TimeoutExpired",
        ],
    );
}
