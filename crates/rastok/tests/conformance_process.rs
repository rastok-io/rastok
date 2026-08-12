// A test harness drives the OS on purpose; the frontier lints
// guard the core modules, not the evidence.
#![allow(clippy::expect_used)]
//! `Process` re-expressed on the conformance harness: the profile is
//! the whole suite — probes, provocations, and the stated gap — and
//! the battery holds the adapter to the ten contract cases. A second
//! test turns the harness against a lying adapter to prove the
//! battery itself bites.

use rastok::boundary::{CallOutcome, Intent, Partner, Response, TypedError};
use rastok::conformance::{
    AdapterFactory, InterleaveProbe, Profile, Provocation, assert_conformant, verify,
};
use rastok::connectors::Process;
use rastok::contract::Registry;
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
    on in { out = data.a }
  }

  operator rb {
    on in { out = data.b }
  }

  . -> ra -> left -> .
  . -> rb -> right -> .
}
"#;

struct ProcessFactory;

impl AdapterFactory for ProcessFactory {
    fn make<'plan>(&mut self, plan: &'plan ValidatedExecutionPlan) -> Box<dyn Partner + 'plan> {
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
                kind: "ExecFailed",
                source: EXEC_FAILED.into(),
                input: "salut",
            },
            Provocation {
                kind: "SpawnFailed",
                source: SPAWN_FAILED.into(),
                input: "salut",
            },
            Provocation {
                kind: "DecodeError",
                source: DECODE_ERROR.into(),
                input: "salut",
            },
            Provocation {
                kind: "OutputOverflow",
                source: OUTPUT_OVERFLOW.into(),
                input: "salut",
            },
        ],
        unprovable: vec![(
            "StderrLine",
            "the M0 job adapter folds stderr into the ExecFailed detail; \
             StderrLine events arrive with the service mode",
        )],
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
        &Registry::try_m0().expect("the standard catalogue holds"),
        &mut ProcessFactory,
    );
    assert_conformant(&report, &["StderrLine"]);
    assert_eq!(
        report.proven_kinds,
        vec![
            "ExecFailed",
            "SpawnFailed",
            "DecodeError",
            "OutputOverflow",
            "TimeoutExpired",
        ]
    );
}

/// Answers every started call with an error kind no contract
/// declares — the outcome gate must refuse it, and the harness must
/// call that a violation.
struct Liar {
    pending: Vec<Response>,
}

impl Partner for Liar {
    fn start(&mut self, intent: &Intent, _request: &Envelope) {
        self.pending.push(Response {
            call_ref: intent.call_ref,
            outcome: CallOutcome::Failure(TypedError {
                kind: String::from("Gremlin"),
                data: Data::materialized(Value::Map(std::collections::BTreeMap::new())),
            }),
        });
    }

    fn events(&mut self) -> Vec<Response> {
        std::mem::take(&mut self.pending)
    }
}

struct LiarFactory;

impl AdapterFactory for LiarFactory {
    fn make<'plan>(&mut self, _plan: &'plan ValidatedExecutionPlan) -> Box<dyn Partner + 'plan> {
        Box::new(Liar {
            pending: Vec::new(),
        })
    }
}

#[test]
fn an_undeclared_kind_fails_the_battery() {
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
        &Registry::try_m0().expect("the standard catalogue holds"),
        &mut LiarFactory,
    );
    assert!(!report.conformant());
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.contains("the outcome gate refused a reply")),
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

impl Partner for Swapper {
    fn start(&mut self, intent: &Intent, _request: &Envelope) {
        let echoed = String::from_utf8_lossy(intent.prepared.bytes()).into_owned();
        self.pending.push(Response {
            call_ref: intent.call_ref,
            outcome: CallOutcome::Success(Data::materialized(Value::Scalar(
                rastok::envelope::Scalar::Text(echoed),
            ))),
        });
    }

    fn events(&mut self) -> Vec<Response> {
        let mut batch = std::mem::take(&mut self.pending);
        if batch.len() >= 2 {
            let crossed = batch[0].outcome.clone();
            batch[0].outcome = batch[1].outcome.clone();
            batch[1].outcome = crossed;
        }
        batch
    }
}

struct SwapperFactory;

impl AdapterFactory for SwapperFactory {
    fn make<'plan>(&mut self, _plan: &'plan ValidatedExecutionPlan) -> Box<dyn Partner + 'plan> {
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
        &Registry::try_m0().expect("the standard catalogue holds"),
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
