// A test harness drives the OS on purpose; the frontier lints
// guard the core modules, not the evidence.
#![allow(clippy::expect_used)]
//! `Process` re-expressed on the conformance harness: the profile is
//! the whole suite — probes, provocations, and the stated gap — and
//! the battery holds the Driver to the ten contract cases. A second
//! test turns the harness against a lying Driver to prove the
//! battery itself bites.

use rastok::boundary::{
    Completion, Intent, OutboundConnectorDriver, PortOutput, ProcessExit, Response,
};
use rastok::conformance::{
    InterleaveProbe, OutboundConnectorDriverFactory, Profile, Provocation, assert_conformant,
    verify,
};
use rastok::connectors::Process;
use rastok::contract::ContractCatalog;
use rastok::envelope::{Data, Envelope, Value};
use rastok::plan::ValidatedExecutionPlan;
use rastok::stdlib::PROCESS;

const HAPPY: &str = r#"
graph happy : App {
  connector echo : Process {
    mode = job ; command = "cat"
    codec = lines
    effect = idempotent ; behavior = deterministic
  }

  operator shout {
    input in: struct { text: text }
    on in where data.text != "" {
      out = data.text
    }
  }

  . -> shout -> echo -> .
}
"#;

const EXEC_FAILED: &str = r#"
graph failing : App {
  connector boom : Process {
    mode = job ; command = "sh"
    args = ["-c", "echo broken >&2; exit 3"]
    codec = lines
    effect = idempotent ; behavior = deterministic
  }

  operator shout {
    input in: struct { text: text }
    on in { out = data.text }
  }

  . -> shout -> boom -> .
}
"#;

const SPAWN_FAILED: &str = r#"
graph missing : App {
  connector ghost : Process {
    mode = job ; command = "/nonexistent/rastok-no-such-binary"
    codec = lines
    effect = idempotent ; behavior = deterministic
  }

  operator shout {
    input in: struct { text: text }
    on in { out = data.text }
  }

  . -> shout -> ghost -> .
}
"#;

const DECODE_ERROR: &str = r#"
graph garbling : App {
  connector garble : Process {
    mode = job ; command = "perl"
    args = ["-e", "print chr(255)"]
    codec = lines
    effect = idempotent ; behavior = deterministic
  }

  operator shout {
    input in: struct { text: text }
    on in { out = data.text }
  }

  . -> shout -> garble -> .
}
"#;

const OUTPUT_OVERFLOW: &str = r#"
graph flooding : App {
  connector leaky : Process {
    mode = job ; command = "sh"
    args = ["-c", "head -c 51 /dev/zero; sleep 30"]
    codec = lines
    effect = idempotent ; behavior = deterministic
    max_output = "50B"
    timeout = duration("15s")
  }

  operator shout {
    input in: struct { text: text }
    on in { out = data.text }
  }

  . -> shout -> leaky -> .
}
"#;

const MUTE: &str = r#"
graph mute : App {
  connector hang : Process {
    mode = job ; command = "sh"
    args = ["-c", "sleep 30"]
    codec = lines
    effect = idempotent ; behavior = deterministic
    timeout = duration("200ms")
  }

  operator shout {
    input in: struct { text: text }
    on in { out = data.text }
  }

  . -> shout -> hang -> .
}
"#;

const PAIR: &str = r#"
graph pair : App {
  connector left : Process {
    mode = job ; command = "cat"
    codec = lines
    effect = idempotent ; behavior = deterministic
  }

  connector right : Process {
    mode = job ; command = "cat"
    codec = lines
    effect = idempotent ; behavior = deterministic
  }

  operator ra {
    input in: struct { a: text, b: text }
    on in { out = data.a }
  }

  operator rb {
    input in: struct { a: text, b: text }
    on in { out = data.b }
  }

  . -> ra -> left -> .
  . -> rb -> right -> .
}
"#;

struct ProcessFactory;

impl OutboundConnectorDriverFactory for ProcessFactory {
    fn make<'plan>(
        &mut self,
        plan: &'plan ValidatedExecutionPlan,
    ) -> Box<dyn OutboundConnectorDriver + 'plan> {
        Box::new(Process::try_new(plan).expect("the plan binds"))
    }
}

fn process_profile() -> Profile {
    Profile {
        contract: &PROCESS,
        source: HAPPY.into(),
        input: "salut",
        provocations: vec![
            Provocation {
                exit_case: "Exited",
                source: EXEC_FAILED.into(),
                input: "salut",
            },
            Provocation {
                exit_case: "SpawnFailed",
                source: SPAWN_FAILED.into(),
                input: "salut",
            },
            Provocation {
                exit_case: "DecodeFailed",
                source: DECODE_ERROR.into(),
                input: "salut",
            },
            Provocation {
                exit_case: "OutputOverflow",
                source: OUTPUT_OVERFLOW.into(),
                input: "salut",
            },
        ],
        unprovable: vec![
            (
                "CaptureFailed",
                "a pipe READ error cannot be provoked portably from a real child; \
                 the pump seam is unit-tested with an injected failing reader",
            ),
            (
                "OutcomeUnknown",
                "bounded Closing, not the Process Driver, mints this case after \
                 cancellation remains unobserved through settlement",
            ),
        ],
        mute_source: Some(MUTE.into()),
        interleave: Some(InterleaveProbe {
            source: PAIR.into(),
            a: "volt 231",
            b: "volt 232",
        }),
    }
}

#[test]
fn process_passes_the_whole_battery() {
    let report = verify(
        &process_profile(),
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        &mut ProcessFactory,
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
    pending: Vec<Response>,
}

impl OutboundConnectorDriver for Liar {
    fn start(&mut self, intent: &Intent, _request: &Envelope) {
        self.pending.push(Response {
            call_ref: intent.call_ref,
            completion: Completion::try_new(vec![PortOutput {
                port: rastok::node::Port::conventional_exit(),
                data: Data::materialized(Value::TaggedUnion(Box::new(
                    rastok::value::TaggedUnionValue {
                        case: String::from("Gremlin"),
                        payload: vec![],
                    },
                ))),
            }])
            .expect("the structural terminal barrier holds"),
        });
    }

    fn events(&mut self) -> Vec<Response> {
        std::mem::take(&mut self.pending)
    }
}

struct LiarFactory;

impl OutboundConnectorDriverFactory for LiarFactory {
    fn make<'plan>(
        &mut self,
        _plan: &'plan ValidatedExecutionPlan,
    ) -> Box<dyn OutboundConnectorDriver + 'plan> {
        Box::new(Liar {
            pending: Vec::new(),
        })
    }
}

#[test]
fn a_malformed_terminal_case_fails_the_battery() {
    let profile = Profile {
        contract: &PROCESS,
        source: HAPPY.into(),
        input: "salut",
        provocations: Vec::new(),
        unprovable: Vec::new(),
        mute_source: None,
        interleave: None,
    };
    let report = verify(
        &profile,
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        &mut LiarFactory,
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
    pending: Vec<Response>,
}

impl OutboundConnectorDriver for Swapper {
    fn start(&mut self, intent: &Intent, _request: &Envelope) {
        let echoed = String::from_utf8_lossy(intent.prepared.bytes()).into_owned();
        self.pending.push(Response {
            call_ref: intent.call_ref,
            completion: Completion::process(
                vec![Data::materialized(Value::Text(echoed))],
                vec![],
                ProcessExit::Exited {
                    exit_code: 0,
                    signal: None,
                },
            ),
        });
    }

    fn events(&mut self) -> Vec<Response> {
        let mut batch = std::mem::take(&mut self.pending);
        if batch.len() >= 2 {
            let crossed = batch[0].completion.clone();
            batch[0].completion = batch[1].completion.clone();
            batch[1].completion = crossed;
        }
        batch
    }
}

struct SwapperFactory;

impl OutboundConnectorDriverFactory for SwapperFactory {
    fn make<'plan>(
        &mut self,
        _plan: &'plan ValidatedExecutionPlan,
    ) -> Box<dyn OutboundConnectorDriver + 'plan> {
        Box::new(Swapper {
            pending: Vec::new(),
        })
    }
}

#[test]
fn crossed_concurrent_replies_fail_the_battery() {
    let profile = Profile {
        contract: &PROCESS,
        source: HAPPY.into(),
        input: "salut",
        provocations: Vec::new(),
        unprovable: Vec::new(),
        mute_source: None,
        interleave: Some(InterleaveProbe {
            source: PAIR.into(),
            a: "volt 231",
            b: "volt 232",
        }),
    };
    let report = verify(
        &profile,
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        &mut SwapperFactory,
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
