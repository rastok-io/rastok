// A test harness drives the OS on purpose; the frontier lints
// guard the core modules, not the evidence.
#![allow(clippy::expect_used)]
//! `Process` re-expressed on the conformance harness: the profile is
//! the whole suite — probes, provocations, and the stated gap — and
//! the battery holds the Driver to the ten contract cases. A second
//! test turns the harness against a lying Driver to prove the
//! battery itself bites.

use rastok::conformance::{
    DriverTestFixture, ExitCaseTest, InterleaveExpectation, InterleaveProbe, assert_conformant,
    verify,
};
use rastok::connector::ConnectorDirection;
use rastok::connector::ContractId;
use rastok::connector::driver::Driver;
use rastok::connector::driver::Outbound;
use rastok::connector::driver::OutboundCall;
use rastok::connector::exchange::CallOpen;
use rastok::connector::exchange::CallResponse;
use rastok::connector::exchange::CallStart;
use rastok::connector::exchange::Completion;
use rastok::connector::exchange::EnvelopeDraft;
use rastok::connector::exchange::RequestPreparation;
use rastok::connector::exchange::SchedulerCompletion;
use rastok::connector::stdlib::PROCESS;
use rastok::connector::stdlib::PROCESS_CONNECTOR_ID;
use rastok::connector::stdlib::ProcessExit;
use rastok::connectors::{DriverFactory, DriverFactoryError, DriverRegistry, ProcessFactory};
use rastok::contract::ContractCatalog;
use rastok::envelope::{Data, Envelope, Value};
use rastok::plan::ValidatedExecutionPlan;

const HAPPY: &str = r#"
graph happy : App {
  connector echo : Process {
    mode = job ; command = "cat"
    codec = lines
    idempotent = true; behavior = deterministic
  }

  operator shout {
    input in: struct { text: text }
    on in where data.text != "" {
      out = data.text
    }
  }

  in -> shout -> echo -> out
  echo.err -> nothing
  echo.exit -> nothing
}
"#;

const EXEC_FAILED: &str = r#"
graph failing : App {
  connector boom : Process {
    mode = job ; command = "sh"
    args = ["-c", "echo broken >&2; exit 3"]
    codec = lines
    idempotent = true; behavior = deterministic
  }

  operator shout {
    input in: struct { text: text }
    on in { out = data.text }
  }

  in -> shout -> boom -> out
  boom.err -> nothing
  boom.exit -> nothing
}
"#;

const SPAWN_FAILED: &str = r#"
graph missing : App {
  connector ghost : Process {
    mode = job ; command = "/nonexistent/rastok-no-such-binary"
    codec = lines
    idempotent = true; behavior = deterministic
  }

  operator shout {
    input in: struct { text: text }
    on in { out = data.text }
  }

  in -> shout -> ghost -> out
  ghost.err -> nothing
  ghost.exit -> nothing
}
"#;

const DECODE_ERROR: &str = r#"
graph garbling : App {
  connector garble : Process {
    mode = job ; command = "perl"
    args = ["-e", "print chr(255)"]
    codec = lines
    idempotent = true; behavior = deterministic
  }

  operator shout {
    input in: struct { text: text }
    on in { out = data.text }
  }

  in -> shout -> garble -> out
  garble.err -> nothing
  garble.exit -> nothing
}
"#;

const OUTPUT_OVERFLOW: &str = r#"
graph flooding : App {
  connector leaky : Process {
    mode = job ; command = "sh"
    args = ["-c", "head -c 51 /dev/zero; sleep 30"]
    codec = lines
    idempotent = true; behavior = deterministic
    max_output = "50B"
    timeout = duration("15s")
  }

  operator shout {
    input in: struct { text: text }
    on in { out = data.text }
  }

  in -> shout -> leaky -> out
  leaky.err -> nothing
  leaky.exit -> nothing
}
"#;

const MUTE: &str = r#"
graph mute : App {
  connector hang : Process {
    mode = job ; command = "sh"
    args = ["-c", "sleep 30"]
    codec = lines
    idempotent = true; behavior = deterministic
    timeout = duration("200ms")
  }

  operator shout {
    input in: struct { text: text }
    on in { out = data.text }
  }

  in -> shout -> hang -> out
  hang.err -> nothing
  hang.exit -> nothing
}
"#;

const PAIR: &str = r#"
graph pair : App {
  connector left : Process {
    mode = job ; command = "cat"
    codec = lines
    idempotent = true; behavior = deterministic
  }

  connector right : Process {
    mode = job ; command = "cat"
    codec = lines
    idempotent = true; behavior = deterministic
  }

  operator ra {
    input in: struct { a: text, b: text }
    on in { out = data.a }
  }

  operator rb {
    input in: struct { a: text, b: text }
    on in { out = data.b }
  }

  in -> ra -> left -> out
  in -> rb -> right -> out
  left.err -> nothing
  left.exit -> nothing
  right.err -> nothing
  right.exit -> nothing
}
"#;

fn process_registry() -> DriverRegistry {
    DriverRegistry::default()
        .try_with(Box::new(
            ProcessFactory::try_new().expect("the Process id is canonical"),
        ))
        .expect("Process is registered once")
}

fn process_profile() -> DriverTestFixture {
    DriverTestFixture {
        contract: &PROCESS,
        source: HAPPY.into(),
        input: "salut",
        happy_exit_case: "Exited",
        provocations: vec![
            ExitCaseTest {
                exit_case: "Exited",
                source: EXEC_FAILED.into(),
                input: "salut",
            },
            ExitCaseTest {
                exit_case: "SpawnFailed",
                source: SPAWN_FAILED.into(),
                input: "salut",
            },
            ExitCaseTest {
                exit_case: "DecodeFailed",
                source: DECODE_ERROR.into(),
                input: "salut",
            },
            ExitCaseTest {
                exit_case: "OutputOverflow",
                source: OUTPUT_OVERFLOW.into(),
                input: "salut",
            },
        ],
        untestable: vec![
            (
                "CaptureFailed",
                "a pipe READ error cannot be provoked portably from a real child; \
                 the pump seam is unit-tested with an injected failing reader",
            ),
            (
                "OutcomeUnknown",
                "bounded Closing, not the Process Driver, produces this case after \
                 cancellation remains unobserved through settlement",
            ),
        ],
        mute_source: Some(MUTE.into()),
        interleave: Some(InterleaveProbe {
            source: PAIR.into(),
            a: "volt 231",
            b: "volt 232",
            expected: vec![
                InterleaveExpectation {
                    connector: rastok::node::Path::new(["left"]).expect("the probe path is valid"),
                    data: Data::materialized(Value::Text(String::from("volt 231"))),
                },
                InterleaveExpectation {
                    connector: rastok::node::Path::new(["right"]).expect("the probe path is valid"),
                    data: Data::materialized(Value::Text(String::from("volt 232"))),
                },
            ],
        }),
    }
}

#[test]
fn process_passes_the_whole_battery() {
    let report = verify(
        &process_profile(),
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        &process_registry(),
    );
    assert_conformant(&report, &["CaptureFailed", "OutcomeUnknown"]);
    assert_eq!(
        report.proven_cases,
        vec![
            "Exited",
            "SpawnFailed",
            "DecodeFailed",
            "OutputOverflow",
            "TimeoutExpired",
        ]
    );
}

/// Answers every started call with a terminal case no contract
/// declares — the completion gate must refuse it, and the harness must
/// call that a violation.
struct Liar {
    pending: Vec<CallResponse>,
}

impl OutboundCall for Liar {
    fn prepare_request(
        &mut self,
        _: &rastok::node::Path,
        data: &Data,
        _: &std::collections::BTreeMap<String, rastok::value::Value>,
    ) -> RequestPreparation {
        rastok::connector::stdlib::prepare_process_request(
            rastok::connector::stdlib::RequestCodec::Lines,
            data,
        )
    }

    fn scheduler_completion(
        &mut self,
        _: &rastok::node::Path,
        completion: SchedulerCompletion,
    ) -> Completion {
        rastok::connector::stdlib::process_scheduler_completion(completion)
    }

    fn start(&mut self, intent: &CallOpen, _request: &Envelope) -> CallStart {
        self.pending.push(CallResponse {
            call_id: intent.call_id,
            completion: Completion::try_new(vec![EnvelopeDraft {
                port: rastok::node::Port::conventional_exit(),
                data: Data::materialized(Value::TaggedUnion(Box::new(
                    rastok::value::TaggedUnionValue {
                        case: String::from("Gremlin"),
                        payload: vec![],
                    },
                ))),
                ext: rastok::envelope::Namespace::new(),
            }])
            .expect("the structural terminal barrier holds"),
        });
        CallStart::Started
    }

    fn poll_responses(&mut self) -> Vec<CallResponse> {
        std::mem::take(&mut self.pending)
    }
}

impl Driver for Liar {
    fn inbound(&mut self) -> Option<&mut dyn rastok::connector::driver::Ingress> {
        None
    }

    fn outbound(&mut self) -> Option<&mut dyn Outbound> {
        Some(self)
    }
}

struct LiarFactory(ContractId);

impl DriverFactory for LiarFactory {
    fn contract_id(&self) -> &ContractId {
        &self.0
    }

    fn roles(&self) -> ConnectorDirection {
        ConnectorDirection::Outbound
    }

    fn create<'plan>(
        &self,
        _plan: &'plan ValidatedExecutionPlan,
        _nodes: &[rastok::node::Path],
    ) -> Result<Box<dyn Driver + 'plan>, DriverFactoryError> {
        Ok(Box::new(Liar {
            pending: Vec::new(),
        }))
    }
}

#[test]
fn a_malformed_terminal_case_fails_the_battery() {
    let profile = DriverTestFixture {
        contract: &PROCESS,
        source: HAPPY.into(),
        input: "salut",
        happy_exit_case: "Exited",
        provocations: Vec::new(),
        untestable: Vec::new(),
        mute_source: None,
        interleave: None,
    };
    let drivers = DriverRegistry::default()
        .try_with(Box::new(LiarFactory(
            ContractId::parse(PROCESS_CONNECTOR_ID).expect("the Process id is canonical"),
        )))
        .expect("the liar registers once");
    let report = verify(
        &profile,
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        &drivers,
    );
    assert!(!report.conformant());
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.contains("the completion gate refused a reply")),
        "the gate violation is named: {:#?}",
        report.violations
    );
}

/// Answers both concurrent calls with correct call references but
/// crossed payloads — the correlation defect a single-call battery
/// cannot see.
struct Swapper {
    pending: Vec<CallResponse>,
}

impl OutboundCall for Swapper {
    fn prepare_request(
        &mut self,
        _: &rastok::node::Path,
        data: &Data,
        _: &std::collections::BTreeMap<String, rastok::value::Value>,
    ) -> RequestPreparation {
        rastok::connector::stdlib::prepare_process_request(
            rastok::connector::stdlib::RequestCodec::Lines,
            data,
        )
    }

    fn scheduler_completion(
        &mut self,
        _: &rastok::node::Path,
        completion: SchedulerCompletion,
    ) -> Completion {
        rastok::connector::stdlib::process_scheduler_completion(completion)
    }

    fn start(&mut self, intent: &CallOpen, _request: &Envelope) -> CallStart {
        let framed = String::from_utf8_lossy(intent.prepared.bytes());
        let echoed = framed.strip_suffix('\n').unwrap_or(&framed).to_owned();
        self.pending.push(CallResponse {
            call_id: intent.call_id,
            completion: Completion::process(
                vec![Data::materialized(Value::Text(echoed))],
                vec![],
                ProcessExit::Exited {
                    exit_code: 0,
                    signal: None,
                },
            ),
        });
        CallStart::Started
    }

    fn poll_responses(&mut self) -> Vec<CallResponse> {
        let mut batch = std::mem::take(&mut self.pending);
        if batch.len() >= 2 {
            let crossed = batch[0].completion.clone();
            batch[0].completion = batch[1].completion.clone();
            batch[1].completion = crossed;
        }
        batch
    }
}

impl Driver for Swapper {
    fn inbound(&mut self) -> Option<&mut dyn rastok::connector::driver::Ingress> {
        None
    }

    fn outbound(&mut self) -> Option<&mut dyn Outbound> {
        Some(self)
    }
}

struct SwapperFactory(ContractId);

impl DriverFactory for SwapperFactory {
    fn contract_id(&self) -> &ContractId {
        &self.0
    }

    fn roles(&self) -> ConnectorDirection {
        ConnectorDirection::Outbound
    }

    fn create<'plan>(
        &self,
        _plan: &'plan ValidatedExecutionPlan,
        _nodes: &[rastok::node::Path],
    ) -> Result<Box<dyn Driver + 'plan>, DriverFactoryError> {
        Ok(Box::new(Swapper {
            pending: Vec::new(),
        }))
    }
}

#[test]
fn crossed_concurrent_replies_fail_the_battery() {
    let profile = DriverTestFixture {
        contract: &PROCESS,
        source: HAPPY.into(),
        input: "salut",
        happy_exit_case: "Exited",
        provocations: Vec::new(),
        untestable: Vec::new(),
        mute_source: None,
        interleave: Some(InterleaveProbe {
            source: PAIR.into(),
            a: "volt 231",
            b: "volt 232",
            expected: vec![
                InterleaveExpectation {
                    connector: rastok::node::Path::new(["left"]).expect("the probe path is valid"),
                    data: Data::materialized(Value::Text(String::from("volt 231"))),
                },
                InterleaveExpectation {
                    connector: rastok::node::Path::new(["right"]).expect("the probe path is valid"),
                    data: Data::materialized(Value::Text(String::from("volt 232"))),
                },
            ],
        }),
    };
    let drivers = DriverRegistry::default()
        .try_with(Box::new(SwapperFactory(
            ContractId::parse(PROCESS_CONNECTOR_ID).expect("the Process id is canonical"),
        )))
        .expect("the swapper registers once");
    let report = verify(
        &profile,
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        &drivers,
    );
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.contains("crossed two concurrent replies")),
        "the swap is named: {:#?}",
        report.violations
    );
}

#[test]
fn identical_interleave_expectations_cannot_claim_correlation_proof() {
    let mut profile = process_profile();
    let probe = profile
        .interleave
        .as_mut()
        .expect("the Process profile carries an interleave proof");
    probe.expected[1].data = probe.expected[0].data.clone();

    let report = verify(
        &profile,
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        &process_registry(),
    );
    assert!(
        report
            .violations
            .iter()
            .any(|violation| { violation.contains("expected outputs must differ") })
    );
}
