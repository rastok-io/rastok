//! `Process` re-expressed on the conformance harness: the profile is
//! the whole suite — probes, provocations, and the stated gap — and
//! the battery holds the adapter to the ten contract cases. A second
//! test turns the harness against a lying adapter to prove the
//! battery itself bites.

use rastok::boundary::{CallOutcome, Intent, Partner, Response, TypedError};
use rastok::conformance::{AdapterFactory, Profile, Provocation, assert_conformant, verify};
use rastok::connectors::Process;
use rastok::envelope::{Data, Envelope, Value};
use rastok::plan::ExecutionPlan;
use rastok::stdlib::{PROCESS, Registry};

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

struct ProcessFactory;

impl AdapterFactory for ProcessFactory {
    fn make<'plan>(&mut self, plan: &'plan ExecutionPlan) -> Box<dyn Partner + 'plan> {
        Box::new(Process::new(plan))
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
    }
}

#[test]
fn process_passes_the_whole_battery() {
    let report = verify(&process_profile(), &Registry::m0(), &mut ProcessFactory);
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
    fn make<'plan>(&mut self, _plan: &'plan ExecutionPlan) -> Box<dyn Partner + 'plan> {
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
    };
    let report = verify(&profile, &Registry::m0(), &mut LiarFactory);
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
