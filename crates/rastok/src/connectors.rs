// The adapter layer IS the OS boundary: spawning, threads, and the
// clock are its business — the frontier lints stop at this file.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
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
use crate::envelope::{Data, Envelope, Scalar, Value};
use crate::plan::{AttrSetting, ExecutionPlan, NodeKind};
use std::collections::BTreeMap;
use std::io::Read;
use std::io::Write;
use std::process::{Command, Stdio};

/// The default stdout cap for a job run, per the contract sheet's
/// `max_output = 8MiB` default.
const DEFAULT_MAX_OUTPUT: usize = 8 * 1024 * 1024;

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
/// The adapter is asynchronous, as the Partner contract demands: a
/// partner answers or stays silent — it never times itself out. Only
/// the scheduler mints TimeoutExpired; when the machine closes a
/// call, [`Partner::cancel`] kills the child. Every pipe drains on
/// its own detached thread through a channel, so a descendant that
/// inherits the pipes can never hang this process: readers are never
/// joined, only drained.
///
/// Job-mode stderr travels inside `ExecFailed` on failure; on a
/// successful exit it is dropped in this build — `StderrLine` events
/// belong to the service-mode stream machinery.
#[derive(Debug)]
pub struct Process<'plan> {
    plan: &'plan ExecutionPlan,
    runs: Vec<Run>,
    answers: Vec<Response>,
}

/// One in-flight child.
#[derive(Debug)]
struct Run {
    call_ref: crate::boundary::CallRef,
    codec: crate::boundary::RequestCodec,
    child: std::process::Child,
    chunks: std::sync::mpsc::Receiver<Chunk>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    max_output: usize,
    overflowed: bool,
    out_eof: bool,
    err_eof: bool,
    exit: Option<std::process::ExitStatus>,
    group_killed: bool,
}

/// One drained slice of a pipe, or its end-of-stream marker — the
/// deterministic completion protocol: a reply is final when both
/// streams have marked EOF, not after a scheduling-dependent grace.
#[derive(Debug)]
enum Chunk {
    Out(Vec<u8>),
    Err(Vec<u8>),
    OutEof,
    ErrEof,
}

impl<'plan> Process<'plan> {
    /// An adapter serving every `Process` connector of one plan.
    #[must_use]
    pub fn new(plan: &'plan ExecutionPlan) -> Self {
        Self {
            plan,
            runs: Vec::new(),
            answers: Vec::new(),
        }
    }

    fn launch(&mut self, intent: &Intent, request: &Envelope) -> Result<Run, CallOutcome> {
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
            return Err(failure(
                "SpawnFailed",
                "the intent names no planned connector".to_string(),
            ));
        };
        let Some(AttrSetting::Value(cel::Value::Text(command))) = spec.attribute("command") else {
            return Err(failure(
                "SpawnFailed",
                "the command attribute is not text".to_string(),
            ));
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
        // The machine prepared and sealed the bytes; the adapter
        // TRANSPORTS them — recomputing would reopen the gap between
        // sealed and sent.
        let framed = intent.prepared.bytes().to_vec();
        let _ = request;
        let max_output = match spec.attribute("max_output") {
            Some(AttrSetting::Value(cel::Value::Text(text))) => match parse_size(text) {
                Some(size) => size,
                None => {
                    return Err(failure(
                        "SpawnFailed",
                        format!("max_output `{text}` is not a size"),
                    ));
                }
            },
            _ => DEFAULT_MAX_OUTPUT,
        };
        let mut builder = Command::new(command);
        builder
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Its own process group: a kill reaches every descendant, so
        // no orphan can keep the pipes — or a pump thread — alive.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut builder, 0);
        let mut child = match builder.spawn() {
            Ok(child) => child,
            Err(error) => return Err(failure("SpawnFailed", error.to_string())),
        };
        // Detached pipe pumps: chunks flow through the channel; the
        // threads die with their pipes and are never waited on.
        let stdin = child.stdin.take();
        std::thread::spawn(move || {
            if let Some(mut stdin) = stdin {
                // A child that exits without draining stdin is a
                // broken pipe, not a fault.
                let _ = stdin.write_all(&framed);
            }
        });
        // Bounded channel: the pumps block when the collector lags,
        // so an unread flood costs at most the channel's window —
        // the cap is a memory bound, not just a reply bound.
        let (sender, chunks) = std::sync::mpsc::sync_channel(64);
        match child.stdout.take() {
            Some(pipe) => {
                let sender = sender.clone();
                std::thread::spawn(move || pump_pipe(pipe, &sender, Chunk::Out, Chunk::OutEof));
            }
            None => {
                let _ = sender.send(Chunk::OutEof);
            }
        }
        match child.stderr.take() {
            Some(pipe) => {
                std::thread::spawn(move || pump_pipe(pipe, &sender, Chunk::Err, Chunk::ErrEof));
            }
            None => {
                let _ = sender.send(Chunk::ErrEof);
            }
        }
        Ok(Run {
            call_ref: intent.call_ref,
            codec: crate::boundary::RequestCodec::of(spec),
            child,
            chunks,
            stdout: Vec::new(),
            stderr: Vec::new(),
            max_output,
            overflowed: false,
            out_eof: false,
            err_eof: false,
            exit: None,
            group_killed: false,
        })
    }
}

/// Kills a child and, on Unix, its whole process group — the group
/// id is the child's pid, set at spawn.
fn kill_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // `kill -- -PGID` without libc: forbid(unsafe_code) holds.
        // Stderr is discarded — signalling a group whose leader is
        // already a zombie is a normal race, noisy on macOS.
        let _ = Command::new("kill")
            .args(["-9", "--", &format!("-{}", child.id())])
            .stderr(Stdio::null())
            .stdout(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Reads one pipe to its end in bounded chunks, sending each through
/// the channel and closing with the EOF marker; a closed receiver
/// ends the pump.
fn pump_pipe<P: Read>(
    mut pipe: P,
    sender: &std::sync::mpsc::SyncSender<Chunk>,
    wrap: fn(Vec<u8>) -> Chunk,
    eof: Chunk,
) {
    let mut buffer = [0u8; 8192];
    loop {
        match pipe.read(&mut buffer) {
            Ok(0) | Err(_) => {
                let _ = sender.send(eof);
                return;
            }
            Ok(read) => {
                if sender.send(wrap(buffer[..read].to_vec())).is_err() {
                    return;
                }
            }
        }
    }
}

impl Run {
    /// Drains arrived chunks, checking the cap per chunk; past the
    /// cap nothing more is stored — the bound is enforced at intake.
    fn collect(&mut self) -> bool {
        for chunk in self.chunks.try_iter() {
            match chunk {
                Chunk::Out(bytes) => {
                    if !self.overflowed {
                        self.stdout.extend_from_slice(&bytes);
                        if self.stdout.len() > self.max_output {
                            self.overflowed = true;
                        }
                    }
                }
                Chunk::Err(bytes) => {
                    if self.stderr.len() < self.max_output {
                        self.stderr.extend_from_slice(&bytes);
                    }
                }
                Chunk::OutEof => self.out_eof = true,
                Chunk::ErrEof => self.err_eof = true,
            }
        }
        self.overflowed
    }

    /// Whether the reply is final: the child exited and both streams
    /// marked EOF — a protocol, not a grace delay. The markers are
    /// guaranteed: at the leader's exit [`Process::events`] kills the
    /// remaining group, so the pipes close and the EOFs follow.
    fn finished(&self) -> bool {
        self.exit.is_some() && self.out_eof && self.err_eof
    }

    /// The reply of a finished run.
    fn settle(mut self, status: std::process::ExitStatus) -> Response {
        let overflowed = self.collect() || self.overflowed;
        let outcome = if overflowed {
            let mut payload = BTreeMap::new();
            payload.insert(
                String::from("size"),
                Value::Scalar(Scalar::Int(
                    i64::try_from(self.stdout.len()).unwrap_or(i64::MAX),
                )),
            );
            CallOutcome::Failure(TypedError {
                kind: String::from("OutputOverflow"),
                data: Data::materialized(Value::Map(payload)),
            })
        } else if !status.success() {
            let mut payload = BTreeMap::new();
            payload.insert(
                String::from("exit_code"),
                match status.code() {
                    Some(code) => Value::Scalar(Scalar::Int(i64::from(code))),
                    // Killed by a signal: no exit code exists.
                    None => Value::Scalar(Scalar::Int(-1)),
                },
            );
            payload.insert(
                String::from("stderr"),
                Value::Scalar(Scalar::Text(
                    String::from_utf8_lossy(&self.stderr).into_owned(),
                )),
            );
            CallOutcome::Failure(TypedError {
                kind: String::from("ExecFailed"),
                data: Data::materialized(Value::Map(payload)),
            })
        } else {
            match String::from_utf8(std::mem::take(&mut self.stdout)) {
                Ok(text) => match self.codec {
                    // `codec = lines`, job mode: the whole stdout,
                    // one text blob.
                    crate::boundary::RequestCodec::Lines => {
                        CallOutcome::Success(Data::materialized(Value::Scalar(Scalar::Text(text))))
                    }
                    // `codec = jsonl`, job mode: the decoded lines,
                    // `data = array<variant>` (D-2026-08-11-1). The
                    // house reader owns the numbers; an unparseable
                    // line — an interior empty one included — is a
                    // typed DecodeError carrying the offending line.
                    crate::boundary::RequestCodec::Jsonl => match decode_jsonl(&text) {
                        Ok(items) => CallOutcome::Success(Data::materialized(Value::Array(items))),
                        Err(message) => failure("DecodeError", message),
                    },
                },
                Err(_) => failure("DecodeError", "stdout is not valid UTF-8".to_string()),
            }
        };
        Response {
            call_ref: self.call_ref,
            outcome,
        }
    }
}

/// Decodes a complete jsonl stdout into its line values
/// (D-2026-08-11-1): each `\n`-framed line through the house
/// reader — exact numbers, duplicate keys refused; the trailing
/// newline's empty tail is framing; an interior empty or trailing
/// garbage line refuses with the offending line in the message.
fn decode_jsonl(stdout: &str) -> Result<Vec<Value>, String> {
    let mut items = Vec::new();
    // EMPTY stdout is the empty array; a lone `\n` is one empty
    // line and refuses below — the framing is judged before the
    // terminator is stripped, so no invalid line is silently lost.
    if stdout.is_empty() {
        return Ok(items);
    }
    let body = stdout.strip_suffix('\n').unwrap_or(stdout);
    for line in body.split('\n') {
        let (value, consumed) = rastok_core::json::parse_value(line)
            .map_err(|error| format!("line `{line}` does not parse: {}", error.message))?;
        // JSON's own whitespace alphabet, exactly (space, tab, CR —
        // LF cannot appear inside a line): a non-breaking space is
        // trailing content, not padding.
        if line[consumed..]
            .bytes()
            .all(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
        {
            let value = rastok_core::runtime::to_envelope_value(&value)
                .map_err(|fault| format!("line `{line}` is not representable: {fault}"))?;
            items.push(value);
        } else {
            return Err(format!(
                "line `{line}` carries trailing content after its JSON value"
            ));
        }
    }
    Ok(items)
}

impl Partner for Process<'_> {
    fn start(&mut self, intent: &Intent, request: &Envelope) {
        match self.launch(intent, request) {
            Ok(run) => self.runs.push(run),
            Err(outcome) => self.answers.push(Response {
                call_ref: intent.call_ref,
                outcome,
            }),
        }
    }

    fn events(&mut self) -> Vec<Response> {
        let mut answers = std::mem::take(&mut self.answers);
        let mut still_running = Vec::new();
        for mut run in self.runs.drain(..) {
            if run.collect() {
                // The cap is a hard guarantee: tripped at intake, the
                // whole process group dies now.
                kill_group(&mut run.child);
                let mut payload = BTreeMap::new();
                payload.insert(
                    String::from("size"),
                    Value::Scalar(Scalar::Int(
                        i64::try_from(run.stdout.len()).unwrap_or(i64::MAX),
                    )),
                );
                answers.push(Response {
                    call_ref: run.call_ref,
                    outcome: CallOutcome::Failure(TypedError {
                        kind: String::from("OutputOverflow"),
                        data: Data::materialized(Value::Map(payload)),
                    }),
                });
                continue;
            }
            if run.exit.is_none() {
                match run.child.try_wait() {
                    Ok(Some(status)) => run.exit = Some(status),
                    Ok(None) => {}
                    Err(error) => {
                        let call_ref = run.call_ref;
                        kill_group(&mut run.child);
                        answers.push(Response {
                            call_ref,
                            outcome: failure("ExecFailed", error.to_string()),
                        });
                        continue;
                    }
                }
            }
            match run.exit {
                // Complete by protocol: exit seen and both EOFs in.
                Some(status) if run.finished() => answers.push(run.settle(status)),
                Some(_) => {
                    // The leader exited: "stdout complete at exit"
                    // means the job is OVER — any descendant still
                    // holding the pipes dies with the group now, so
                    // the EOF markers are guaranteed to arrive on a
                    // later poll. Deterministic, no timeout needed.
                    if !run.group_killed {
                        run.group_killed = true;
                        kill_group(&mut run.child);
                    }
                    still_running.push(run);
                }
                None => still_running.push(run),
            }
        }
        self.runs = still_running;
        answers
    }

    fn cancel(&mut self, call_ref: &crate::boundary::CallRef) {
        if let Some(position) = self.runs.iter().position(|run| &run.call_ref == call_ref) {
            let mut run = self.runs.swap_remove(position);
            // The whole process GROUP dies: descendants inherited the
            // pipes with the group, and the group takes them down —
            // the pump threads then see EOF and exit.
            kill_group(&mut run.child);
        }
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
            port: crate::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Map(data)),
                ext: Namespace::new(),
            },
            sequence: SubmissionSequence::Next,
        }
    }

    /// Drives one submission to stability against real children: the
    /// wall clock maps onto the virtual clock in small steps, so
    /// declared budgets expire for real and the scheduler — never
    /// the adapter — mints the timeouts.
    fn run(source: &str, text: &str) -> (MemJournal, Vec<Envelope>, Vec<Envelope>) {
        let compilation = compile(&[source], &Registry::m0()).expect("a valid plan");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        let mut world = Process::new(&compilation.plan);
        vm.inject(&mut journal, submission(text)).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            vm.drain(&mut journal).unwrap();
            let report = vm.pump(&mut journal, &mut world).unwrap();
            vm.drain(&mut journal).unwrap();
            if vm.is_stable() {
                break;
            }
            if report.settled == 0 {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the run wedged: neither settled nor expired"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
                vm.advance(&mut journal, &mut world, vm.now() + 10_000_000)
                    .unwrap();
            }
        }
        (
            journal,
            vm.outputs()
                .iter()
                .map(|output| output.envelope.clone())
                .collect(),
            vm.errors().to_vec(),
        )
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
        // The harness maps the wall clock onto the virtual clock, so
        // the scheduler's budget expires for real and cancel kills
        // the mute child.
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
        let (journal, outputs, errors) = run(source, "salut");
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
            "the budget actually fired"
        );
        // The scheduler minted it: the closure is a Timeout fact,
        // never a partner-fabricated failure response.
        assert!(journal.records().iter().any(|record| matches!(
            record,
            Record::Outbound(o)
                if matches!(o.status, OutboundStatus::Closed(crate::journal::Closure::Timeout))
        )));
    }

    #[test]
    fn a_leaders_exit_terminates_its_group_without_any_timeout() {
        // The eighth review's repro: the leader exits immediately, a
        // background descendant inherits the pipes, and NO budget is
        // declared. "stdout complete at exit" means the job is over —
        // the group dies at the exit and the reply arrives without
        // waiting on anyone.
        let source = r#"
graph orphaning : App {
  connector fork : Process {
    mode = job ; command = "sh"
    args = ["-c", "echo done; sleep 30 & exit 0"]
    codec = lines
    effect = idempotent ; behavior = deterministic
  }

  operator shout {
    on in { out = data.text }
  }

  . -> shout -> fork -> .
}
"#;
        let started = std::time::Instant::now();
        let (_, outputs, errors) = run(source, "salut");
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            outputs[0].data(),
            &Data::materialized(Value::Scalar(Scalar::Text(String::from("done\n"))))
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "the group died at the leader's exit; nothing waited on the descendant"
        );
    }

    #[test]
    fn a_descendant_holding_the_pipes_cannot_wedge_the_cap() {
        // The sixth review's hang: the direct child dies, its
        // descendant keeps the pipes open. The cap must fire from
        // the collected bytes without waiting on any pipe.
        let source = r#"
graph wedge : App {
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
        let started = std::time::Instant::now();
        let (_, outputs, errors) = run(source, "salut");
        assert!(outputs.is_empty());
        let Data::Materialized(Value::Map(payload)) = errors[0].data() else {
            panic!("expected the typed error payload");
        };
        assert_eq!(
            payload.get("kind"),
            Some(&Value::Scalar(Scalar::Text(String::from("OutputOverflow"))))
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "the cap is a hard guarantee, not a hope"
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
    on in { out = . }
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

    /// The framing table of D-2026-08-11-1, exactly: empty stdout
    /// is the empty array, a lone `\n` is one empty line and
    /// refuses, JSON's own whitespace alphabet pads a line and
    /// nothing else does.
    #[test]
    fn the_jsonl_framing_table_holds() {
        use rastok_core::envelope::{Scalar, Value};
        assert_eq!(super::decode_jsonl(""), Ok(vec![]), "empty stdout");
        assert!(
            super::decode_jsonl("\n").is_err(),
            "a lone newline is one EMPTY line, refused — never silently lost"
        );
        assert_eq!(
            super::decode_jsonl("1\n"),
            Ok(vec![Value::Scalar(Scalar::Int(1))])
        );
        assert!(
            super::decode_jsonl("1\n\n").is_err(),
            "an interior empty line refuses"
        );
        assert_eq!(
            super::decode_jsonl("1 \t\r\n"),
            Ok(vec![Value::Scalar(Scalar::Int(1))]),
            "JSON's whitespace pads a line"
        );
        assert!(
            super::decode_jsonl("1\u{a0}\n").is_err(),
            "a non-breaking space is trailing content, not padding"
        );
    }
}
