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
use rastok::connectors::{DriverRegistry, HttpClientConfig, HttpClientFactory, RuntimeBindings};
use rastok::contract::ContractCatalog;
use rastok::deployment::{
    BootBindings, DeploymentProfile, EnvironmentName, SecretBinding, SecretMaterial, SecretName,
    SecretProvider, SecretProviderError, SecretProviderId, SecretVersion,
};
use rastok::envelope::{Data, Namespace, Value};
use rastok::journal::MemJournal;
use rastok::node;
use rastok::plan::GraphVersion;
use rastok::runtime::vm::Vm;
use rastok::stdlib::HTTP_CLIENT;
use std::collections::BTreeMap;
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
    let authorized = request.lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("authorization") && value.trim() == "Bearer deployment-token"
        })
    });
    let (status, delay, content_type, body): (_, _, _, &[u8]) = match path {
        "/happy" => (200, 0, "text/plain", b"happy-response"),
        "/authenticated" if authorized => (200, 0, "text/plain", b"authenticated"),
        "/authenticated" => (401, 0, "text/plain", b"unauthorized"),
        "/large" => (200, 0, "text/plain", b"too-large"),
        "/bad-json" => (200, 0, "application/json", b"not-json"),
        "/server-error" => (503, 0, "text/plain", b"temporarily-unavailable"),
        "/projected-json" => (
            503,
            0,
            "application/json",
            br#"{"usage":{"tokens":12,"extra":true},"extra":"OMITTED-WIRE-DATA"}"#,
        ),
        "/first" => (200, 40, "text/plain", b"first-result"),
        "/second" => (200, 1, "text/plain", b"second-result"),
        "/mute" => {
            std::thread::sleep(std::time::Duration::from_millis(500));
            return;
        }
        _ => (200, 0, "text/plain", b"unknown"),
    };
    if delay != 0 {
        std::thread::sleep(std::time::Duration::from_millis(delay));
    }
    let response = format!(
        "HTTP/1.1 {status} OK\r\ncontent-type: {content_type}\r\nx-repeat: a\r\nx-repeat: b\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
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
  in -> body -> client -> out
  client.exit -> nothing
"#
    } else {
        "\n  in -> client -> out\n  client.exit -> nothing\n"
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
    idempotent = true; behavior = deterministic
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
    idempotent = true; behavior = deterministic
  }}
  connector right : HttpClient {{
    url = "{}" ; timeout = duration("1s")
    codec = text ; versions = ["http/1.1"]
    idempotent = true; behavior = deterministic
  }}
  operator a {{
    input in: struct {{ a: text, b: text }}
    on in {{ out = data.a }}
  }}
  operator b {{
    input in: struct {{ a: text, b: text }}
    on in {{ out = data.b }}
  }}
  in -> a -> left -> out
  in -> b -> right -> out
  left.exit -> nothing
  right.exit -> nothing
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

struct FixedSecretProvider;

impl SecretProvider for FixedSecretProvider {
    fn resolve(&self, binding: &SecretBinding) -> Result<SecretMaterial, SecretProviderError> {
        assert_eq!(binding.reference().as_str(), "providers/http/token");
        assert_eq!(binding.version().as_str(), "v3");
        SecretMaterial::new(b"deployment-token".to_vec())
    }
}

fn authenticated_boot(url: String) -> BootBindings {
    let reference = SecretName::new("providers/http/token").expect("logical secret reference");
    let binding = SecretBinding::new(
        reference.clone(),
        SecretProviderId::new("test.local/test@1").expect("stable provider id"),
        "HTTP_TEST_TOKEN",
        SecretVersion::new("v3").expect("stable secret version"),
    )
    .expect("versioned secret binding");
    let profile = DeploymentProfile::new(
        BTreeMap::from([(
            EnvironmentName::new("SERVICE_URL").expect("portable environment name"),
            Value::Text(url),
        )]),
        BTreeMap::from([(reference, binding)]),
        None,
    )
    .expect("typed deployment profile");
    BootBindings::resolve(profile, &FixedSecretProvider).expect("all boot bindings resolve")
}

fn drive(vm: &mut Vm<'_>, journal: &mut MemJournal, world: &mut RuntimeBindings<'_>) {
    let mut settled = false;
    for _ in 0..250 {
        vm.run_ready_with_bindings(journal, world)
            .expect("the authenticated graph drains");
        vm.pump(journal, world)
            .expect("the authenticated Driver pumps");
        vm.run_ready_with_bindings(journal, world)
            .expect("the authenticated completion drains");
        if !vm.has_open_obligations() {
            settled = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(settled, "the authenticated loopback call did not settle");
}

#[test]
fn deployment_secret_reaches_real_and_scripted_http_without_entering_artifacts() {
    let server = LoopbackServer::start();
    let source = r#"
graph authenticated : App {
  connector client : HttpClient {
    url = env("SERVICE_URL")
    auth_bearer = secret("providers/http/token")
    method = "POST"
    timeout = duration("1s")
    codec = text
    versions = ["http/1.1"]
    idempotent = true; behavior = deterministic
  }
  in -> client -> out
  client.exit -> nothing
}

lab authenticated {
  simulation scripted on app {
    mock client {
      on in {
        out = "scripted"
        exit = Completed { status = 200 }
      }
    }
    scenario same_plan "the mock consumes the same boot-bound descriptor" {
      app.in = "request"
      expect "scripted response" app.out count == 1 where data == "scripted"
    }
  }
}
"#;
    let compilation = rastok::compiler::compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the boot-bound client compiles");
    let boot = authenticated_boot(server.url("/authenticated"));
    let mut world = RuntimeBindings::try_new_with_boot(&compilation.plan, &registry(), &boot)
        .expect("the real Driver resolves its exact boot inputs");
    let version = GraphVersion(1);
    let mut journal = MemJournal::new_with_deployment(version, boot.attestation().clone());
    let mut vm = Vm::boot_with_deployment(&compilation.plan, version, &boot);
    vm.inject(
        &mut journal,
        rastok::boundary::IngressSubmission {
            provenance: rastok::boundary::IngressEnvelopeProvenance::single(
                node::Path::new(["test"]).expect("valid provenance"),
            ),
            port: node::Port::new("in").expect("valid input port"),
            draft: rastok::boundary::Draft {
                data: Data::materialized(Value::Text(String::from("request"))),
                ext: Namespace::new(),
            },
            sequence: rastok::boundary::SubmissionSequence::Next,
        },
    )
    .expect("the request is admitted");
    drive(&mut vm, &mut journal, &mut world);
    assert!(vm.outputs().iter().any(|output| {
        output.envelope.data().materialized_value()
            == Some(&Value::Text(String::from("authenticated")))
    }));

    let lab_runs =
        rastok::lab::run_with_boot(&compilation.plan, version, &compilation.labs[0], &boot);
    assert!(
        lab_runs.iter().all(|run| run.report.passed()),
        "{lab_runs:#?}"
    );

    let package = rastok::plan::encode(&compilation.plan, &compilation.hash);
    let inspected = format!(
        "{:?}\n{:?}\n{:?}\n{:?}",
        compilation.plan,
        boot,
        journal.records(),
        package
    );
    assert!(!inspected.contains("deployment-token"));
    assert!(inspected.contains("providers/http/token"));
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
                exit_case: "ServerError",
                source: probe(
                    "server_error",
                    &server.url("/server-error"),
                    "text",
                    "1s",
                    "1KiB",
                ),
                input: "request",
            },
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
            "ServerError",
            "ConnectFailed",
            "TlsFailed",
            "ProtocolFailed",
            "BodyTooLarge",
            "DecodeFailed",
            "TimeoutExpired",
        ],
    );
}

#[test]
fn physical_projected_attempts_keep_samples_before_retry_holds_or_publishes() {
    use rastok::accounting::accounting_observations;
    use rastok::journal::Record;
    use rastok::journal::retry_protocol::Fact;
    use rastok::recording::Recording;
    let server = LoopbackServer::start();
    // Only this inert local GET is declared idempotent, not model inference.
    let source = format!(
        r#"
graph app : App {{
  connector client : HttpClient + Retry {{
    url = "{}"; method = "GET"; codec = json; json_profile = projected
    output out: struct {{ usage: struct {{ tokens: uint @measure(id = tokens, unit = "token") }}? }}
    timeout = duration("1s"); versions = ["http/1.1"]
    idempotent = true; behavior = deterministic
    Retry.attempts = 2; Retry.on = [ServerError]
  }}
  in -> client -> out
  client.exit -> nothing
}}
"#,
        server.url("/projected-json")
    );
    let compilation =
        rastok::compiler::compile(&[&source], &ContractCatalog::try_m0().expect("catalogue"))
            .expect("projected Retry");
    let mut world = RuntimeBindings::try_new(&compilation.plan, &registry()).expect("HTTP binder");
    let version = GraphVersion(1);
    let mut journal = MemJournal::new(version);
    let mut vm = Vm::boot_program(&compilation.plan, compilation.hash, version, 0, None);
    vm.inject(
        &mut journal,
        rastok::boundary::IngressSubmission {
            provenance: rastok::boundary::IngressEnvelopeProvenance::single(
                node::Path::new(["wire-test"]).expect("provenance"),
            ),
            port: node::Port::new("in").expect("port"),
            draft: rastok::boundary::Draft {
                data: Data::materialized(Value::Map(BTreeMap::new())),
                ext: Namespace::new(),
            },
            sequence: rastok::boundary::SubmissionSequence::Next,
        },
    )
    .expect("admission");
    drive(&mut vm, &mut journal, &mut world);
    let samples = accounting_observations(&compilation.plan, journal.records(), None)
        .expect("original samples");
    assert_eq!(
        samples.len(),
        2,
        "Held and Published each have one original sample"
    );
    assert_ne!(samples[0].envelope_id(), samples[1].envelope_id());
    assert!(samples.iter().all(|sample| sample.measures().len() == 1
        && sample.measures()[0].quantity().to_string() == "12"
        && sample.measures()[0].unit() == "token"));
    assert_eq!(
        vm.outputs().len(),
        1,
        "only the final response reaches the graph"
    );
    let dispositions: Vec<_> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(tx) => tx.retry_record.as_deref(),
            _ => None,
        })
        .flat_map(|packet| &packet.facts)
        .filter_map(|fact| match fact {
            Fact::Held(_) => Some("held"),
            Fact::Published(_) => Some("published"),
            _ => None,
        })
        .collect();
    assert_eq!(dispositions, ["held", "published"]);
    assert!(!format!("{:?}", journal.records()).contains("OMITTED-WIRE-DATA"));
    let recording =
        Recording::capture(&compilation, &journal, vm.now()).expect("diagnostic recording");
    let decoded =
        Recording::decode(&recording.encode().expect("encode")).expect("independent decode");
    assert_eq!(decoded.records(), journal.records());
    for through in 0..=journal.records().len() {
        let (prefix, _) = decoded
            .inspect(through)
            .expect("every prefix recovers without Driver calls");
        assert_eq!(
            accounting_observations(decoded.plan(), prefix.records(), None)
                .expect("recovered samples"),
            accounting_observations(&compilation.plan, &journal.records()[..through], None)
                .expect("original samples")
        );
    }
}
