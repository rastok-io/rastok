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

/// The default stdout cap for a job run, per the contract sheet's
/// `max_output = 8MiB` default.
const DEFAULT_MAX_OUTPUT: usize = 8 * 1024 * 1024;

/// Prepares the exact protocol bytes of a request under a connector's
/// contract — pure, shared by the machine and the adapter: the digest
/// the journal seals and the bytes the child receives come from this
/// one function, so they cannot diverge. `codec = lines` frames text
/// as a single line (one trailing newline ensured) and passes raw
/// bytes verbatim; anything else refuses before any effect.
///
/// # Errors
///
/// The typed-decode reason when the data cannot be framed.
pub fn prepare(spec: &crate::plan::ConnectorSpec, data: &Data) -> Result<Vec<u8>, String> {
    let _ = spec; // one codec exists in this build; jsonl defers at check
    frame_request(data)
}

/// Parses a declared size ("8MiB", "512KiB", "100B", bare bytes).
fn parse_size(text: &str) -> Option<usize> {
    let text = text.trim();
    let split = text.find(|c: char| !c.is_ascii_digit())?;
    let (digits, suffix) = text.split_at(split);
    let base: usize = digits.parse().ok()?;
    let factor = match suffix.trim() {
        "B" => 1,
        "KiB" => 1024,
        "MiB" => 1024 * 1024,
        "GiB" => 1024 * 1024 * 1024,
        _ => return None,
    };
    base.checked_mul(factor)
}

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
        let framed = match prepare(spec, request.data()) {
            Ok(bytes) => bytes,
            Err(message) => return failure("DecodeError", message),
        };
        let max_output = match spec.attribute("max_output") {
            Some(AttrSetting::Value(cel::Value::Text(text))) => match parse_size(text) {
                Some(size) => size,
                None => {
                    return failure("SpawnFailed", format!("max_output `{text}` is not a size"));
                }
            },
            _ => DEFAULT_MAX_OUTPUT,
        };
        let deadline = match spec.attribute("timeout") {
            Some(AttrSetting::Value(cel::Value::Duration(nanos))) => u64::try_from(*nanos)
                .ok()
                .map(|nanos| std::time::Instant::now() + std::time::Duration::from_nanos(nanos)),
            _ => None,
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
        // Pipes are drained concurrently or they deadlock: a child
        // may block on stderr before closing stdout, or on stdout
        // before reading stdin. Every pipe gets its own thread; the
        // parent only watches the exit and the clock.
        let stdin = child.stdin.take();
        let writer = std::thread::spawn(move || {
            if let Some(mut stdin) = stdin {
                // A child that exits without draining stdin is a
                // broken pipe, not a fault.
                let _ = stdin.write_all(&framed);
            }
        });
        let cap = u64::try_from(max_output)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let stdout_reader = child.stdout.take().map(|pipe| {
            std::thread::spawn(move || {
                let mut buffer = Vec::new();
                let _ = pipe.take(cap).read_to_end(&mut buffer);
                buffer
            })
        });
        let stderr_reader = child.stderr.take().map(|pipe| {
            std::thread::spawn(move || {
                let mut buffer = Vec::new();
                let _ = pipe.take(cap).read_to_end(&mut buffer);
                buffer
            })
        });
        let join_pipes =
            |writer: std::thread::JoinHandle<()>,
             stdout_reader: Option<std::thread::JoinHandle<Vec<u8>>>,
             stderr_reader: Option<std::thread::JoinHandle<Vec<u8>>>| {
                let _ = writer.join();
                let stdout = stdout_reader
                    .and_then(|handle| handle.join().ok())
                    .unwrap_or_default();
                let stderr = stderr_reader
                    .and_then(|handle| handle.join().ok())
                    .unwrap_or_default();
                (stdout, stderr)
            };
        // Watch the exit against the declared budget: a mute child is
        // killed at the deadline and answers with the typed timeout —
        // the machine's virtual budget cannot reach a blocked host.
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if let Some(deadline) = deadline
                        && std::time::Instant::now() >= deadline
                    {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = join_pipes(writer, stdout_reader, stderr_reader);
                        return CallOutcome::Failure(TypedError {
                            kind: String::from("TimeoutExpired"),
                            data: Data::materialized(Value::Map(BTreeMap::new())),
                        });
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = join_pipes(writer, stdout_reader, stderr_reader);
                    return failure("ExecFailed", error.to_string());
                }
            }
        };
        let (stdout, stderr_bytes) = join_pipes(writer, stdout_reader, stderr_reader);
        if stdout.len() > max_output {
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
        let stderr = String::from_utf8_lossy(&stderr_bytes).into_owned();
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
    fn a_mute_child_is_killed_at_its_declared_budget() {
        // The machine's virtual clock cannot reach a blocked host:
        // the adapter itself enforces the wall-clock deadline.
        let source = r#"
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
        let started = std::time::Instant::now();
        let (_, outputs, errors) = run(source, "salut");
        assert!(outputs.is_empty());
        assert_eq!(errors.len(), 1);
        let Data::Materialized(Value::Map(payload)) = errors[0].data() else {
            panic!("expected the typed error payload");
        };
        assert_eq!(
            payload.get("kind"),
            Some(&Value::Scalar(Scalar::Text(String::from("TimeoutExpired"))))
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "the deadline actually fired"
        );
    }

    #[test]
    fn a_stderr_flood_cannot_deadlock_the_pipes() {
        // 200 KiB of stderr overflows the OS pipe buffer: without a
        // concurrent drain the child blocks before closing stdout.
        let source = r#"
graph flood : App {
  connector noisy : Process {
    mode = job ; command = "sh"
    args = ["-c", "head -c 200000 /dev/zero >&2; echo done"]
    codec = lines
    effect = idempotent ; behavior = deterministic
    timeout = duration("30s")
  }

  operator shout {
    on in { out = data.text }
  }

  . -> shout -> noisy -> .
}
"#;
        let (_, outputs, errors) = run(source, "salut");
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            outputs[0].data(),
            &Data::materialized(Value::Scalar(Scalar::Text(String::from("done\n"))))
        );
    }

    #[test]
    fn the_declared_output_cap_is_enforced() {
        let source = r#"
graph capped : App {
  connector wide : Process {
    mode = job ; command = "sh"
    args = ["-c", "head -c 100 /dev/zero"]
    codec = lines
    effect = idempotent ; behavior = deterministic
    max_output = "50B"
  }

  operator shout {
    on in { out = data.text }
  }

  . -> shout -> wide -> .
}
"#;
        let (_, outputs, errors) = run(source, "salut");
        assert!(outputs.is_empty());
        let Data::Materialized(Value::Map(payload)) = errors[0].data() else {
            panic!("expected the typed error payload");
        };
        assert_eq!(
            payload.get("kind"),
            Some(&Value::Scalar(Scalar::Text(String::from("OutputOverflow"))))
        );
    }

    #[test]
    fn an_unframeable_request_never_opens_a_call() {
        // The prepared request is sealed before the open fact: an
        // unframeable payload refuses as a typed decode error with
        // NO outbound record — the journal never promises an effect
        // that cannot be attempted.
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
        let (journal, outputs, errors) = run(source, "salut");
        assert!(outputs.is_empty());
        let Data::Materialized(Value::Map(payload)) = errors[0].data() else {
            panic!("expected the typed error payload");
        };
        assert_eq!(
            payload.get("kind"),
            Some(&Value::Scalar(Scalar::Text(String::from("DecodeError"))))
        );
        assert!(
            !journal
                .records()
                .iter()
                .any(|record| matches!(record, Record::Outbound(_))),
            "no open fact for an unattemptable effect"
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
