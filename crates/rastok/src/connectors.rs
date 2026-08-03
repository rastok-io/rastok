//! The real adapters behind connector contracts — where the graph's
//! intent becomes an actual effect on the machine.
//!
//! An adapter is a [`Partner`]: the dispatcher hands it journaled
//! intents and collects its answers. Nothing else changes shape — the
//! Lab's scripted worlds and these adapters are interchangeable
//! behind the same trait, which is the point: a simulation exercises
//! exactly the machinery a run does, minus this file.

use crate::boundary::{CallOutcome, Intent, Partner, Response, TypedError};
use crate::cel;
use crate::envelope::{Data, Encoding, Envelope, Scalar, Value};
use crate::plan::{AttrSetting, ExecutionPlan, NodeKind};
use std::collections::BTreeMap;
use std::io::Read;
use std::io::Write;
use std::process::{Command, Stdio};

/// The stdout cap for a job run, per the contract sheet's
/// `max_output = 8MiB` default. Declared sizes join with the size
/// parser.
const MAX_OUTPUT: usize = 8 * 1024 * 1024;

/// The `Process` adapter, job mode: one envelope, one spawn. stdin
/// carries the codec-framed request; the complete stdout at exit
/// becomes the one reply. The process boundary is the correlation.
///
/// The run is synchronous: a job either exits or trips its output
/// cap. Wall-clock budget enforcement stays with the machine's
/// discrete clock — the `timeout` attribute arms the journal-side
/// budget, and a hung child is bounded by `max_output`, not killed
/// mid-run by this adapter yet.
///
/// Job-mode stderr travels inside `ExecFailed` on failure; on a
/// successful exit it is dropped in this build — `StderrLine` events
/// belong to the service-mode stream machinery.
#[derive(Debug)]
pub struct Process<'plan> {
    plan: &'plan ExecutionPlan,
    answers: Vec<Response>,
}

impl<'plan> Process<'plan> {
    /// An adapter serving every `Process` connector of one plan.
    #[must_use]
    pub fn new(plan: &'plan ExecutionPlan) -> Self {
        Self {
            plan,
            answers: Vec::new(),
        }
    }

    fn run(&self, intent: &Intent, request: &Envelope) -> CallOutcome {
        let Some(spec) = self
            .plan
            .nodes
            .iter()
            .find(|node| node.path == intent.connector_path)
            .and_then(|node| match &node.kind {
                NodeKind::Connector(spec) => Some(spec),
                NodeKind::Operator => None,
            })
        else {
            // A journaled intent for an unplanned node cannot happen
            // through the dispatcher; answer with a typed spawn
            // failure rather than trusting that.
            return failure(
                "SpawnFailed",
                "the intent names no planned connector".to_string(),
            );
        };
        let Some(AttrSetting::Value(cel::Value::Text(command))) = spec.attribute("command") else {
            return failure(
                "SpawnFailed",
                "the command attribute is not text".to_string(),
            );
        };
        let args: Vec<String> = match spec.attribute("args") {
            Some(AttrSetting::Value(cel::Value::List(items))) => items
                .iter()
                .filter_map(|item| match item {
                    cel::Value::Text(text) => Some(text.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let framed = match frame_request(request.data()) {
            Ok(bytes) => bytes,
            Err(message) => return failure("DecodeError", message),
        };
        let mut child = match Command::new(command)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => return failure("SpawnFailed", error.to_string()),
        };
        // A pipe writes and reads concurrently or deadlocks: the
        // child may block writing stdout while we block writing
        // stdin. The writer gets its own thread; a child that exits
        // without draining stdin is a broken pipe, not a fault.
        let stdin = child.stdin.take();
        let writer = std::thread::spawn(move || {
            if let Some(mut stdin) = stdin {
                let _ = stdin.write_all(&framed);
            }
        });
        let mut stdout = Vec::new();
        if let Some(pipe) = child.stdout.take() {
            let cap = u64::try_from(MAX_OUTPUT).unwrap_or(u64::MAX);
            if let Err(error) = pipe.take(cap.saturating_add(1)).read_to_end(&mut stdout) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = writer.join();
                return failure("ExecFailed", error.to_string());
            }
        }
        if stdout.len() > MAX_OUTPUT {
            let _ = child.kill();
            let _ = child.wait();
            let _ = writer.join();
            let mut payload = BTreeMap::new();
            payload.insert(
                String::from("size"),
                Value::Scalar(Scalar::Int(i64::try_from(stdout.len()).unwrap_or(i64::MAX))),
            );
            return CallOutcome::Failure(TypedError {
                kind: String::from("OutputOverflow"),
                data: Data::materialized(Value::Map(payload)),
            });
        }
        let status = child.wait();
        let _ = writer.join();
        let mut stderr = String::new();
        if let Some(mut pipe) = child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        let status = match status {
            Ok(status) => status,
            Err(error) => return failure("ExecFailed", error.to_string()),
        };
        if !status.success() {
            let mut payload = BTreeMap::new();
            payload.insert(
                String::from("exit_code"),
                match status.code() {
                    Some(code) => Value::Scalar(Scalar::Int(i64::from(code))),
                    // Killed by a signal: no exit code exists.
                    None => Value::Scalar(Scalar::Int(-1)),
                },
            );
            payload.insert(String::from("stderr"), Value::Scalar(Scalar::Text(stderr)));
            return CallOutcome::Failure(TypedError {
                kind: String::from("ExecFailed"),
                data: Data::materialized(Value::Map(payload)),
            });
        }
        // `codec = lines`, job mode: the whole stdout, one text blob.
        match String::from_utf8(stdout) {
            Ok(text) => CallOutcome::Success(Data::materialized(Value::Scalar(Scalar::Text(text)))),
            Err(_) => failure("DecodeError", "stdout is not valid UTF-8".to_string()),
        }
    }
}

impl Partner for Process<'_> {
    fn start(&mut self, intent: &Intent, request: &Envelope) {
        let outcome = self.run(intent, request);
        self.answers.push(Response {
            call_ref: intent.call_ref,
            outcome,
        });
    }

    fn events(&mut self) -> Vec<Response> {
        std::mem::take(&mut self.answers)
    }
}

/// Frames a request for stdin under `codec = lines`: text data is
/// one line — a trailing newline is ensured, not doubled — and raw
/// bytes pass verbatim. Anything else cannot be framed as a line and
/// refuses as a typed decode error before any spawn.
fn frame_request(data: &Data) -> Result<Vec<u8>, String> {
    match data {
        Data::Encoded {
            encoding: Encoding::Raw,
            bytes,
            ..
        } => Ok(bytes.clone()),
        Data::Materialized(Value::Scalar(Scalar::Text(text))) => {
            let mut bytes = text.clone().into_bytes();
            if bytes.last() != Some(&b'\n') {
                bytes.push(b'\n');
            }
            Ok(bytes)
        }
        Data::Encoded { encoding, .. } => Err(format!(
            "the lines codec frames text; the request is {encoding:?}-encoded"
        )),
        Data::Materialized(other) => Err(format!(
            "the lines codec frames text; the request data is {}",
            value_kind(other)
        )),
    }
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Scalar(Scalar::Bool(_)) => "a bool",
        Value::Scalar(Scalar::Int(_)) => "an int",
        Value::Scalar(Scalar::Double(_)) => "a double",
        Value::Scalar(Scalar::Text(_)) => "text",
        Value::Scalar(Scalar::Bytes(_)) => "bytes",
        Value::Struct(_) => "a struct",
        Value::Array(_) => "an array",
        Value::Map(_) => "a map",
    }
}

fn failure(kind: &str, message: String) -> CallOutcome {
    let mut payload = BTreeMap::new();
    payload.insert(
        String::from("message"),
        Value::Scalar(Scalar::Text(message)),
    );
    CallOutcome::Failure(TypedError {
        kind: String::from(kind),
        data: Data::materialized(Value::Map(payload)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use crate::compiler::compile;
    use crate::envelope::Namespace;
    use crate::journal::{MemJournal, OutboundStatus, Record};
    use crate::plan::GraphVersion;
    use crate::runtime::vm::Vm;
    use crate::stdlib::Registry;

    const CAT_PROBE: &str = r#"
graph real : App {
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

    fn submission(text: &str) -> InboundSubmission {
        let mut data = std::collections::BTreeMap::new();
        data.insert(
            String::from("text"),
            Value::Scalar(Scalar::Text(String::from(text))),
        );
        InboundSubmission {
            authority: IngressAuthority::single(crate::node::Path::new(["cli"]).unwrap()),
            draft: Draft {
                data: Data::materialized(Value::Map(data)),
                mime_type: None,
                app: Namespace::new(),
            },
            sequence: SubmissionSequence::Next,
        }
    }

    fn run(source: &str, text: &str) -> (MemJournal, Vec<Envelope>, Vec<Envelope>) {
        let compilation = compile(&[source], &Registry::m0()).expect("a valid plan");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        let mut world = Process::new(&compilation.plan);
        vm.inject(&mut journal, submission(text)).unwrap();
        vm.drain(&mut journal).unwrap();
        vm.pump(&mut journal, &mut world).unwrap();
        vm.drain(&mut journal).unwrap();
        assert!(vm.is_stable());
        (journal, vm.outputs().to_vec(), vm.errors().to_vec())
    }

    #[test]
    fn the_world_actually_answers() {
        let (journal, outputs, errors) = run(CAT_PROBE, "salut");
        assert!(errors.is_empty());
        assert_eq!(outputs.len(), 1);
        // `cat` echoed the framed line; the reply is the whole
        // stdout, one text blob.
        assert_eq!(
            outputs[0].data(),
            &Data::materialized(Value::Scalar(Scalar::Text(String::from("salut\n"))))
        );
        // The same normative sequence as the scripted world — the
        // adapter changed, the machine did not.
        let closed = journal.records().iter().any(|record| {
            matches!(
                record,
                Record::Outbound(o) if matches!(o.status, OutboundStatus::Closed(_))
            )
        });
        assert!(closed, "the call closed against a real child process");
    }

    #[test]
    fn two_real_runs_journal_identically() {
        let (once, first, _) = run(CAT_PROBE, "salut");
        let (twice, second, _) = run(CAT_PROBE, "salut");
        assert_eq!(once.records(), twice.records());
        assert_eq!(first, second);
    }

    #[test]
    fn a_nonzero_exit_is_a_typed_exec_failure() {
        let source = r#"
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
        let (_, outputs, errors) = run(source, "salut");
        assert!(outputs.is_empty());
        assert_eq!(errors.len(), 1);
        let Data::Materialized(Value::Map(payload)) = errors[0].data() else {
            panic!("expected the typed error payload");
        };
        assert_eq!(
            payload.get("kind"),
            Some(&Value::Scalar(Scalar::Text(String::from("ExecFailed"))))
        );
        let Some(Value::Map(detail)) = payload.get("detail") else {
            panic!("expected the ExecFailed detail");
        };
        assert_eq!(
            detail.get("exit_code"),
            Some(&Value::Scalar(Scalar::Int(3)))
        );
        assert_eq!(
            detail.get("stderr"),
            Some(&Value::Scalar(Scalar::Text(String::from("broken\n"))))
        );
    }

    #[test]
    fn an_unknown_binary_is_a_typed_spawn_failure() {
        let source = r#"
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
        let (_, outputs, errors) = run(source, "salut");
        assert!(outputs.is_empty());
        assert_eq!(errors.len(), 1);
        let Data::Materialized(Value::Map(payload)) = errors[0].data() else {
            panic!("expected the typed error payload");
        };
        assert_eq!(
            payload.get("kind"),
            Some(&Value::Scalar(Scalar::Text(String::from("SpawnFailed"))))
        );
    }

    #[test]
    fn an_unframeable_request_is_a_decode_error_without_a_spawn() {
        // The operator ships the whole map; the lines codec frames
        // text — the refusal is typed and no child ever runs.
        let source = r#"
graph unframeable : App {
  connector echo : Process {
    mode = job ; command = "cat"
    codec = lines
    effect = idempotent ; behavior = deterministic
  }

  operator shout {
    on in { out = envelope }
  }

  . -> shout -> echo -> .
}
"#;
        let (_, outputs, errors) = run(source, "salut");
        assert!(outputs.is_empty());
        assert_eq!(errors.len(), 1);
        let Data::Materialized(Value::Map(payload)) = errors[0].data() else {
            panic!("expected the typed error payload");
        };
        assert_eq!(
            payload.get("kind"),
            Some(&Value::Scalar(Scalar::Text(String::from("DecodeError"))))
        );
    }

    #[test]
    fn framing_ensures_exactly_one_trailing_newline() {
        let framed = frame_request(&Data::materialized(Value::Scalar(Scalar::Text(
            String::from("ligne"),
        ))))
        .unwrap();
        assert_eq!(framed, b"ligne\n");
        let already = frame_request(&Data::materialized(Value::Scalar(Scalar::Text(
            String::from("ligne\n"),
        ))))
        .unwrap();
        assert_eq!(already, b"ligne\n");
        let raw = frame_request(&Data::encoded(Encoding::Raw, *b"verbatim")).unwrap();
        assert_eq!(raw, b"verbatim");
    }
}
