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
use crate::envelope::{Data, Envelope, Value};
use crate::plan::{AttrSetting, NodeKind};
use std::collections::BTreeMap;
use std::io::Read;
use std::io::Write;
use std::process::{Command, Stdio};

/// The job profile's RESOLVED settings — one pure conversion from
/// the travelling spec, judged in full BEFORE any effect
/// (D-2026-08-12-1, review rounds): the contract must be `Process`,
/// every Defaulted attribute must be present (the checker injected
/// it; absence is a violated invariant), every value must inhabit
/// its domain — the codec exhaustively, with NO fallback — and
/// nothing is silently dropped or re-defaulted. The adapter
/// consumes THIS, never the raw attribute table.
#[derive(Debug)]
struct ProcessSettings {
    command: String,
    args: Vec<String>,
    codec: crate::boundary::RequestCodec,
    max_output: u64,
}

/// A violated `Process` invariant — typed, so the refusal names the
/// exact door instead of concatenating prose at the fault site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessSettingsError {
    /// The spec belongs to another contract entirely.
    NotProcess(String),
    /// A Defaulted attribute did not travel in the plan.
    Unresolved(&'static str),
    /// A mode this build has no adapter for.
    UnknownMode(String),
    /// `on_timeout` is fixed to `kill` on a job.
    OnTimeoutNotKill,
    /// The command is not text.
    CommandNotText,
    /// An argument is not text — nothing is dropped silently.
    ArgNotText(String),
    /// A codec word outside the dispatch domain.
    UnknownCodec(String),
    /// `max_output` is not a declared byte size.
    NotASize(String),
    /// A budget this host's `Vec` cannot even EXPRESS
    /// (`> isize::MAX` on this target) — representability, not a
    /// memory promise.
    HostCapacity(u64),
    /// An error kind this adapter really produces is missing from
    /// the spec's declared set — its response would be refused at
    /// the PlanGate after consumption, leaving the call open
    /// forever.
    UndeclaredError(&'static str),

    /// `timeout` is not a duration.
    TimeoutNotDuration,
}

impl core::fmt::Display for ProcessSettingsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotProcess(contract) => write!(
                f,
                "the plan hands a `{contract}` connector to the Process adapter"
            ),
            Self::Unresolved(name) => write!(
                f,
                "the plan violates the Process contract: `{name}` is missing from the \
                 travelling sheet (the compiler resolves every attribute before a plan leaves)"
            ),
            Self::UnknownMode(word) => {
                write!(f, "the `{word}` mode has no adapter in this build")
            }
            Self::OnTimeoutNotKill => {
                write!(
                    f,
                    "`on_timeout` is fixed to `kill` for `Process(mode = job)`"
                )
            }
            Self::CommandNotText => write!(f, "the command attribute is not text"),
            Self::ArgNotText(value) => write!(
                f,
                "an argument is not text: {value} — nothing is dropped silently"
            ),
            Self::UnknownCodec(word) => write!(
                f,
                "`{word}` is outside the codec dispatch domain (lines | jsonl)"
            ),
            Self::NotASize(text) => write!(f, "max_output `{text}` is not a size"),
            Self::HostCapacity(size) => write!(
                f,
                "this host cannot express a {size}-byte buffer (beyond its Vec limit)"
            ),
            Self::UndeclaredError(kind) => write!(
                f,
                "the adapter produces `{kind}` but the spec's error_set does not declare it — \
                 its response would strand the call open"
            ),

            Self::TimeoutNotDuration => write!(f, "the timeout attribute is not a duration"),
        }
    }
}

/// Every failure THIS adapter can hand the machine, with its payload
/// SHAPE sealed beside its kind — one authority: a new emission site
/// cannot exist without a variant, `into_outcome` is the only
/// `TypedError` builder, and every kind is proven declared at
/// binding (no real response can be refused at the PlanGate after
/// the dispatcher consumed the event — a stranded open call).
#[derive(Debug, Clone, PartialEq)]
enum ProcessFailure {
    /// The OS refused a launch — or an intent reached an unbound
    /// path (the edge defense).
    SpawnFailed { message: String },
    /// The reply's CONTENT is invalid: JSON, UTF-8, framing — the
    /// offending detail rides the payload.
    DecodeError { message: String },
    /// A pipe READ failed mid-run: the capture is incomplete for an
    /// I/O reason, not a content one. The payload is CLOSED and
    /// empty — the OS detail is scheduling-adjacent telemetry, and
    /// two racing pump failures must not journal their order
    /// (D-2026-08-12-1, review round).
    CaptureFailed {},
    /// The run ended without a clean success. ONE shape
    /// (D-2026-08-12-1, review round): a wait/OS bookkeeping fault
    /// reports `exit_code = -1` with the fault in `stderr` — the
    /// spec's `{ exit_code, stderr }`, never a second form.
    ExecFailed { exit_code: i64, stderr: String },
    /// The AGGREGATE capture budget or the host's capture capacity
    /// was crossed. No `stream`, no exact `size`: the two pump
    /// threads have no total order, so naming the crossing stream
    /// or a drained byte-count would journal a thread race
    /// (D-2026-08-12-1, review round). `limit` is the declared
    /// budget.
    OutputOverflow {
        limit: u64,
        reason: CaptureOverflowReason,
    },
}

/// Why a capture stopped — a CLOSED domain, rendered to text only
/// at the envelope frontier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureOverflowReason {
    /// The declared aggregate budget was crossed.
    Budget,
    /// The host's capture allocation refused (`try_reserve`).
    Capacity,
}

impl CaptureOverflowReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Budget => "budget",
            Self::Capacity => "capacity",
        }
    }
}

/// One drain's bound AND the pump channel's capacity — shared, so a
/// drain can always empty what a full channel holds and hand back
/// control.
const CHANNEL_CAPACITY: usize = 64;

/// The machine-produced kinds a bound Process world can emit beside
/// the adapter's own — required at binding for the same
/// stranded-call reason. The spelling is the machine's own constant:
/// one authority across the crates.
const MACHINE_EMITS: &[&str] = &[crate::runtime::vm::TIMEOUT_EXPIRED];

/// Seals variant ↔ canonical name ↔ binding list in ONE declaration:
/// `kind()` is generated as an exhaustive match, so a new variant
/// cannot exist without a name here — and `KINDS` is derived from
/// the same list, never a parallel one.
macro_rules! process_failure_names {
    ($(($variant:ident, $name:literal)),+ $(,)?) => {
        impl ProcessFailure {
            /// Every kind the ADAPTER emits — derived from the one
            /// declaration below.
            const KINDS: &'static [&'static str] = &[$($name),+];

            /// The canonical kind — EXHAUSTIVE: a new variant fails
            /// to compile here until it joins the declaration.
            const fn kind(&self) -> &'static str {
                match self {
                    $(Self::$variant { .. } => $name),+
                }
            }
        }
    };
}

process_failure_names!(
    (SpawnFailed, "SpawnFailed"),
    (DecodeError, "DecodeError"),
    (CaptureFailed, "CaptureFailed"),
    (ExecFailed, "ExecFailed"),
    (OutputOverflow, "OutputOverflow"),
);

impl ProcessFailure {
    fn into_outcome(self) -> CallOutcome {
        let kind = self.kind();
        let payload = match self {
            Self::CaptureFailed {} => BTreeMap::new(),
            Self::SpawnFailed { message } | Self::DecodeError { message } => {
                let mut payload = BTreeMap::new();
                payload.insert(String::from("message"), Value::Text(message));
                payload
            }
            Self::ExecFailed { exit_code, stderr } => {
                let mut payload = BTreeMap::new();
                payload.insert(String::from("exit_code"), Value::Int(exit_code));
                payload.insert(String::from("stderr"), Value::Text(stderr));
                payload
            }
            Self::OutputOverflow { limit, reason } => {
                let mut payload = BTreeMap::new();
                payload.insert(
                    String::from("limit"),
                    Value::Int(i64::try_from(limit).unwrap_or(i64::MAX)),
                );
                payload.insert(
                    String::from("reason"),
                    Value::Text(String::from(reason.as_str())),
                );
                payload
            }
        };
        CallOutcome::Failure(TypedError {
            kind: String::from(kind),
            data: Data::materialized(Value::Map(payload)),
        })
    }
}

impl ProcessSettings {
    fn try_from(spec: &rastok_core::plan::ConnectorSpec) -> Result<Self, ProcessSettingsError> {
        use ProcessSettingsError as E;
        if spec.contract != "Process" {
            return Err(E::NotProcess(spec.contract.clone()));
        }
        for kind in ProcessFailure::KINDS.iter().chain(MACHINE_EMITS) {
            if !spec.error_set.iter().any(|declared| declared == kind) {
                return Err(E::UndeclaredError(kind));
            }
        }
        match spec.attribute("mode") {
            Some(AttrSetting::Word(word)) if word == "job" => {}
            Some(AttrSetting::Word(word)) => return Err(E::UnknownMode(word.clone())),
            _ => return Err(E::Unresolved("mode")),
        }
        match spec.attribute("on_timeout") {
            Some(AttrSetting::Word(word)) if word == "kill" => {}
            Some(_) => return Err(E::OnTimeoutNotKill),
            None => return Err(E::Unresolved("on_timeout")),
        }
        let Some(AttrSetting::Value(crate::value::Value::Text(command))) =
            spec.attribute("command")
        else {
            return Err(E::CommandNotText);
        };
        let args = match spec.attribute("args") {
            Some(AttrSetting::Value(crate::value::Value::Array(items))) => {
                let mut args = Vec::with_capacity(items.len());
                for item in items {
                    match item {
                        crate::value::Value::Text(text) => args.push(text.clone()),
                        other => return Err(E::ArgNotText(format!("{other:?}"))),
                    }
                }
                args
            }
            _ => return Err(E::Unresolved("args")),
        };
        // EXHAUSTIVE: the adapter never re-interprets an unknown or
        // missing codec as `lines` — the fallback lives nowhere on
        // this path.
        let codec = match spec.attribute("codec") {
            Some(AttrSetting::Word(word)) if word == "lines" => {
                crate::boundary::RequestCodec::Lines
            }
            Some(AttrSetting::Word(word)) if word == "jsonl" => {
                crate::boundary::RequestCodec::Jsonl
            }
            Some(AttrSetting::Word(word)) => return Err(E::UnknownCodec(word.clone())),
            _ => return Err(E::Unresolved("codec")),
        };
        let max_output = match spec.attribute("max_output") {
            Some(AttrSetting::Value(crate::value::Value::Text(text))) => {
                match crate::contract::byte_size(text) {
                    Some(size) => {
                        // The SYNTAX bound is target-independent
                        // (contract::MAX_BYTE_SIZE). The binding
                        // proves REPRESENTABILITY on this host — a
                        // `Vec` holds at most `isize::MAX` bytes —
                        // never that the memory exists: allocation
                        // is judged at intake, chunk by chunk,
                        // through `try_reserve`.
                        let vec_limit = u64::try_from(isize::MAX).unwrap_or(u64::MAX);
                        if size > vec_limit {
                            return Err(E::HostCapacity(size));
                        }
                        size
                    }
                    None => return Err(E::NotASize(text.clone())),
                }
            }
            _ => return Err(E::Unresolved("max_output")),
        };
        if let Some(setting) = spec.attribute("timeout")
            && !matches!(
                setting,
                AttrSetting::Value(crate::value::Value::Duration(_))
            )
        {
            return Err(E::TimeoutNotDuration);
        }
        Ok(Self {
            command: command.clone(),
            args,
            codec,
            max_output,
        })
    }
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
    /// The borrow anchoring this world to its VALIDATED plan for
    /// the whole run — the settings below were resolved from it at
    /// binding, and the proof may not be dropped or replaced while
    /// children fly.
    _plan: &'plan crate::plan::ValidatedExecutionPlan,
    /// Every Process connector's settings, resolved ONCE at binding
    /// — before any journaled open, so a configuration defect is a
    /// bind error, never a `SpawnFailed` impersonating the OS.
    settings: std::collections::BTreeMap<crate::node::Path, ProcessSettings>,
    runs: Vec<Run>,
    answers: Vec<Response>,
}

/// Why a plan cannot BIND to the Process adapter — surfaced at
/// construction, before the machine opens anything.
#[derive(Debug)]
pub struct ProcessBindError {
    /// The refusing connector.
    pub node: crate::node::Path,
    reason: ProcessBindReason,
}

/// The two REAL bind authorities, kept apart (D-2026-08-12-1,
/// review round): a Process spec whose settings refuse, or a world
/// this single-Partner adapter cannot cover — the distinction the
/// future BoundExecutionPlan router will inherit.
#[derive(Debug)]
pub enum ProcessBindReason {
    /// The Process spec itself refuses.
    Settings(ProcessSettingsError),
    /// A CALLABLE connector of another contract: its intents would
    /// arrive at this adapter and strand open.
    UnservedConnector {
        /// The foreign contract's name.
        contract: String,
    },
}

impl core::fmt::Display for ProcessBindReason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Settings(error) => error.fmt(f),
            Self::UnservedConnector { contract } => write!(
                f,
                "a callable `{contract}` connector has no adapter in this world — its calls \
                 would strand open (the global binder is the second adapter's design)"
            ),
        }
    }
}

impl ProcessBindError {
    /// The typed reason — callers can dispatch on it instead of
    /// parsing prose.
    #[must_use]
    pub fn reason(&self) -> &ProcessBindReason {
        &self.reason
    }
}

impl core::fmt::Display for ProcessBindError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "`{}` cannot bind: {}", self.node, self.reason)
    }
}

impl std::error::Error for ProcessBindError {}

/// One in-flight child.
#[derive(Debug)]
struct Run {
    call_ref: crate::boundary::CallRef,
    codec: crate::boundary::RequestCodec,
    child: std::process::Child,
    chunks: std::sync::mpsc::Receiver<Chunk>,
    capture: Capture,
    exit: Option<std::process::ExitStatus>,
    group_killed: bool,
}

/// The CHILD-FREE capture state (`try_reserve` consults the host,
/// so absorption is not pure — the terminal REDUCER is): chunks are
/// absorbed one by one, and the verdict reduces by full fixed
/// PRIORITY over everything observed up to the tripping drain —
/// `Budget` over `Capacity` over `CaptureFailed` — never by the
/// arrival order of two racing pumps (D-2026-08-12-1, review
/// round). `Budget` is a pure function of the observed totals, so
/// it must outrank the environmental facts in EVERY order.
#[derive(Debug)]
struct Capture {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    /// Bytes SEEN on each stream, in u64 — the budget judge, so the
    /// buffers never hold a byte past the cap and a 32-bit host
    /// never allocates toward a bound its `Vec` cannot express.
    observed_out: u64,
    observed_err: u64,
    max_output: u64,
    /// The capture verdict: whether the DECLARED budget or the
    /// host's capture capacity spoke. No stream, no byte-count —
    /// the two pumps have no total order, and the journal must not
    /// record a thread race (D-2026-08-12-1, review round).
    overflow: Option<CaptureOverflowReason>,
    /// A pipe read failed: the capture is incomplete and the reply
    /// refuses as the typed CaptureFailed — truncated bytes are
    /// never presented as the whole.
    read_failure: bool,
    out_eof: bool,
    err_eof: bool,
    /// TEST SEAM: forces `try_reserve` refusal so the Capacity path
    /// is provable without a real memory shortage. Always false in
    /// production construction.
    capacity_refused: bool,
}

impl Capture {
    fn new(max_output: u64) -> Self {
        Self {
            stdout: Vec::new(),
            stderr: Vec::new(),
            observed_out: 0,
            observed_err: 0,
            max_output,
            overflow: None,
            read_failure: false,
            out_eof: false,
            err_eof: false,
            capacity_refused: false,
        }
    }

    /// Absorbs one chunk under the aggregate budget. `Budget` may
    /// UPGRADE an earlier `Capacity` verdict (it is deterministic in
    /// the observed totals); nothing downgrades.
    fn absorb(&mut self, chunk: Chunk) {
        let (bytes, is_out) = match chunk {
            Chunk::Out(bytes) => (bytes, true),
            Chunk::Err(bytes) => (bytes, false),
            Chunk::OutEof => {
                self.out_eof = true;
                return;
            }
            Chunk::ErrEof => {
                self.err_eof = true;
                return;
            }
            Chunk::ReadFailed => {
                self.read_failure = true;
                return;
            }
        };
        if is_out {
            self.observed_out = self.observed_out.saturating_add(bytes.len() as u64);
        } else {
            self.observed_err = self.observed_err.saturating_add(bytes.len() as u64);
        }
        if self.observed_out.saturating_add(self.observed_err) > self.max_output {
            // Deterministic in the totals — outranks Capacity in
            // every order.
            self.overflow = Some(CaptureOverflowReason::Budget);
        }
        if self.overflow.is_some() {
            return;
        }
        let buffer = if is_out {
            &mut self.stdout
        } else {
            &mut self.stderr
        };
        if self.capacity_refused || buffer.try_reserve(bytes.len()).is_err() {
            self.overflow = Some(CaptureOverflowReason::Capacity);
        } else {
            buffer.extend_from_slice(&bytes);
        }
    }

    /// The terminal verdict over everything observed so far — the
    /// fixed-priority reducer: a budget/capacity overflow outranks a
    /// read loss, whatever order the pumps delivered them in.
    fn verdict(&self) -> Option<ProcessFailure> {
        if let Some(reason) = self.overflow {
            return Some(ProcessFailure::OutputOverflow {
                limit: self.max_output,
                reason,
            });
        }
        if self.read_failure {
            return Some(ProcessFailure::CaptureFailed {});
        }
        None
    }
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
    /// A pipe READ failed mid-run — not end-of-stream: the captured
    /// bytes are incomplete and must never be presented as the
    /// whole. Carries NOTHING: the OS detail would race between two
    /// failing pumps (D-2026-08-12-1, review round).
    ReadFailed,
}

impl<'plan> Process<'plan> {
    /// Binds an adapter to every `Process` connector of one
    /// VALIDATED plan, resolving all settings up front: a
    /// configuration defect refuses HERE — before any journaled
    /// open — and `SpawnFailed` is reserved for the OS actually
    /// refusing a launch (D-2026-08-12-1, review rounds).
    ///
    /// # Errors
    ///
    /// The first connector that cannot bind, with its typed reason:
    /// an unresolved attribute, a value outside its domain, an
    /// undeclared emitted error, a budget beyond this host's `Vec`
    /// REPRESENTABILITY (never a memory promise — allocation is
    /// judged at intake), or a CALLABLE connector of another
    /// contract this single-Partner world cannot serve (a pure
    /// source is skipped).
    pub fn try_new(
        plan: &'plan crate::plan::ValidatedExecutionPlan,
    ) -> Result<Self, ProcessBindError> {
        let mut settings = std::collections::BTreeMap::new();
        for node in &plan.plan().nodes {
            if let NodeKind::Connector(spec) = &node.kind {
                // A SPECIALIZED adapter — but the world it serves
                // has ONE Partner, so a CALLABLE connector of
                // another contract would receive its intents HERE
                // and strand them open: the binding refuses it. A
                // pure source (no incoming edge) is merely skipped.
                // The BoundExecutionPlan router is the second
                // adapter's named design, with obligations BY ROLE:
                // an outbound caller binds to exactly one Partner,
                // an inbound source to exactly one ingress
                // provider, a both-roles connector to both
                // (m1-slice, the connector items).
                if spec.contract != "Process" {
                    let slot = crate::plan::Slot::new(
                        u32::try_from(
                            plan.plan()
                                .nodes
                                .iter()
                                .position(|candidate| candidate.path == node.path)
                                .unwrap_or(usize::MAX),
                        )
                        .unwrap_or(u32::MAX),
                    );
                    let callable = plan
                        .plan()
                        .edges
                        .iter()
                        .any(|edge| edge.to.as_ref().is_some_and(|to| to.node == Some(slot)));
                    if callable {
                        return Err(ProcessBindError {
                            node: node.path.clone(),
                            reason: ProcessBindReason::UnservedConnector {
                                contract: spec.contract.clone(),
                            },
                        });
                    }
                    continue;
                }
                match ProcessSettings::try_from(spec) {
                    Ok(resolved) => {
                        settings.insert(node.path.clone(), resolved);
                    }
                    Err(reason) => {
                        return Err(ProcessBindError {
                            node: node.path.clone(),
                            reason: ProcessBindReason::Settings(reason),
                        });
                    }
                }
            }
        }
        Ok(Self {
            _plan: plan,
            settings,
            runs: Vec::new(),
            answers: Vec::new(),
        })
    }

    fn launch(&mut self, intent: &Intent, request: &Envelope) -> Result<Run, CallOutcome> {
        // Settings were resolved ONCE at binding; an unknown path
        // here is an integrity breach of the intent, not a config
        // fault.
        let Some(settings) = self.settings.get(&intent.connector_path) else {
            return Err(ProcessFailure::SpawnFailed {
                message: "no Process binding serves this connector (unknown path, or another \
                          contract this specialized adapter does not carry)"
                    .to_string(),
            }
            .into_outcome());
        };
        // The machine prepared and sealed the bytes; the adapter
        // TRANSPORTS them — recomputing would reopen the gap between
        // sealed and sent.
        let framed = intent.prepared.bytes().to_vec();
        let _ = request;
        let mut builder = Command::new(&settings.command);
        builder
            .args(&settings.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Its own process group: a kill reaches every descendant, so
        // no orphan can keep the pipes — or a pump thread — alive.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut builder, 0);
        let mut child = match builder.spawn() {
            Ok(child) => child,
            Err(error) => {
                return Err(ProcessFailure::SpawnFailed {
                    message: error.to_string(),
                }
                .into_outcome());
            }
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
        let (sender, chunks) = std::sync::mpsc::sync_channel(CHANNEL_CAPACITY);
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
            codec: settings.codec,
            child,
            chunks,
            capture: Capture::new(settings.max_output),
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
            Ok(0) => {
                let _ = sender.send(eof);
                return;
            }
            // A signal is not a verdict — read again.
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => {
                // A failed READ is not an EOF: the loss travels as
                // its own fact, then the stream closes.
                let _ = sender.send(Chunk::ReadFailed);
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
    /// Drains arrived chunks into the capture state. `max_output`
    /// is the AGGREGATE budget across stdout and stderr
    /// (D-2026-08-12-1, review round); the verdict reduces by fixed
    /// priority in [`Capture::verdict`].
    fn collect(&mut self) -> bool {
        // BOUNDED: `try_iter` walks a live channel that concurrent
        // pumps keep feeding — one drain takes at most a channel's
        // worth, so the return to the scheduler (and the kill on a
        // terminal fact) stays guaranteed under an infinite
        // producer.
        for chunk in self.chunks.try_iter().take(CHANNEL_CAPACITY) {
            self.capture.absorb(chunk);
        }
        self.capture.verdict().is_some()
    }

    /// Whether the reply is final: the child exited and both streams
    /// marked EOF — a protocol, not a grace delay. The markers are
    /// guaranteed: at the leader's exit [`Process::events`] kills the
    /// remaining group, so the pipes close and the EOFs follow.
    fn finished(&self) -> bool {
        self.exit.is_some() && self.capture.out_eof && self.capture.err_eof
    }

    /// The reply of a finished run.
    fn settle(mut self, status: std::process::ExitStatus) -> Response {
        self.collect();
        let outcome = if let Some(failure) = self.capture.verdict() {
            failure.into_outcome()
        } else if !status.success() {
            ProcessFailure::ExecFailed {
                // Killed by a signal: no exit code exists.
                exit_code: status.code().map_or(-1, i64::from),
                stderr: String::from_utf8_lossy(&self.capture.stderr).into_owned(),
            }
            .into_outcome()
        } else {
            match String::from_utf8(std::mem::take(&mut self.capture.stdout)) {
                Ok(text) => match self.codec {
                    // `codec = lines`, job mode: the whole stdout,
                    // one text blob.
                    crate::boundary::RequestCodec::Lines => {
                        CallOutcome::Success(Data::materialized(Value::Text(text)))
                    }
                    // `codec = jsonl`, job mode: the decoded lines,
                    // `data = array<variant>` (D-2026-08-11-1). The
                    // house reader owns the numbers; an unparseable
                    // line — an interior empty one included — is a
                    // typed DecodeError carrying the offending line.
                    crate::boundary::RequestCodec::Jsonl => match decode_jsonl(&text) {
                        Ok(items) => CallOutcome::Success(Data::materialized(Value::Array(items))),
                        Err(message) => ProcessFailure::DecodeError { message }.into_outcome(),
                    },
                },
                Err(_) => ProcessFailure::DecodeError {
                    message: "stdout is not valid UTF-8".to_string(),
                }
                .into_outcome(),
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
                // A tripped cap or a failed read is a hard verdict:
                // the whole process group dies now, and the answer
                // is the matching typed refusal.
                kill_group(&mut run.child);
                let outcome = run
                    .capture
                    .verdict()
                    .unwrap_or(ProcessFailure::CaptureFailed {})
                    .into_outcome();
                answers.push(Response {
                    call_ref: run.call_ref,
                    outcome,
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
                            // ONE ExecFailed shape: a wait/OS
                            // bookkeeping fault reports exit_code -1
                            // with the fault in stderr.
                            outcome: ProcessFailure::ExecFailed {
                                exit_code: -1,
                                stderr: format!("wait failed: {error}"),
                            }
                            .into_outcome(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use crate::compiler::compile;
    use crate::contract::Registry;
    use crate::envelope::Namespace;
    use crate::journal::{MemJournal, OutboundStatus, Record};
    use crate::plan::GraphVersion;
    use crate::runtime::vm::Vm;

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
        InboundSubmission {
            authority: IngressAuthority::single(crate::node::Path::new(["cli"]).unwrap()),
            port: crate::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Struct(vec![(
                    String::from("text"),
                    Value::Text(String::from(text)),
                )])),
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
        let compilation = compile(
            &[source],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .expect("a valid plan");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        let mut world = Process::try_new(&compilation.plan).expect("the plan binds");
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
            &Data::materialized(Value::Text(String::from("salut\n")))
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
            Some(&Value::Text(String::from("ExecFailed")))
        );
        let Some(Value::Map(detail)) = payload.get("detail") else {
            panic!("expected the ExecFailed detail");
        };
        assert_eq!(detail.get("exit_code"), Some(&Value::Int(3)));
        assert_eq!(
            detail.get("stderr"),
            Some(&Value::Text(String::from("broken\n")))
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
            Some(&Value::Text(String::from("SpawnFailed")))
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
            Some(&Value::Text(String::from("TimeoutExpired")))
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
            &Data::materialized(Value::Text(String::from("done\n")))
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
            Some(&Value::Text(String::from("OutputOverflow")))
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
            &Data::materialized(Value::Text(String::from("done\n")))
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
            Some(&Value::Text(String::from("OutputOverflow")))
        );
    }

    #[test]
    fn a_stderr_flood_counts_against_the_aggregate_budget() {
        // D-2026-08-12-1 (review round): max_output is the AGGREGATE
        // capture budget — stderr crossing it is the SAME typed
        // verdict ({limit, reason} — no stream, no size: no
        // journaled thread race); bytes are never silently dropped.
        let source = r#"
graph hoarse : App {
  connector noisy : Process {
    mode = job ; command = "sh"
    args = ["-c", "head -c 100 /dev/zero >&2; echo done"]
    codec = lines
    effect = idempotent ; behavior = deterministic
    max_output = "50B"
    timeout = duration("15s")
  }

  operator shout {
    on in { out = data.text }
  }

  . -> shout -> noisy -> .
}
"#;
        let (_, outputs, errors) = run(source, "salut");
        assert!(outputs.is_empty(), "{outputs:?}");
        let Data::Materialized(Value::Map(payload)) = errors[0].data() else {
            panic!("expected the typed error payload");
        };
        assert_eq!(
            payload.get("kind"),
            Some(&Value::Text(String::from("OutputOverflow")))
        );
        let Some(Value::Map(detail)) = payload.get("detail") else {
            panic!("the payload rides detail: {payload:?}");
        };
        // No `stream`, no byte-count: the two pumps have no total
        // order, and the journal must not record a thread race —
        // the payload carries the declared limit and the reason.
        assert_eq!(detail.get("stream"), None, "{detail:?}");
        assert_eq!(detail.get("limit"), Some(&Value::Int(50)));
        assert_eq!(
            detail.get("reason"),
            Some(&Value::Text(String::from("budget")))
        );
    }

    #[test]
    fn an_unframeable_request_never_reaches_the_call_boundary() {
        // B2 resolves the whole route to `text`: a structured ingress
        // refuses at the final pin before any mailbox or outbound fact.
        // The adapter-level framing refusal remains a defence for forged
        // plans, not a state a checked graph can normally reach.
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
        let compilation = compile(
            &[source],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .expect("the route resolves to text");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        assert!(
            vm.inject(&mut journal, submission("salut")).is_err(),
            "a struct cannot enter a text route"
        );
        assert!(journal.records().is_empty(), "nothing was journaled");
    }

    /// A scripted reader for the pump seam: each step answers one
    /// `read` call.
    struct ScriptedRead(std::vec::IntoIter<Result<Vec<u8>, std::io::ErrorKind>>);

    impl Read for ScriptedRead {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            match self.0.next() {
                None => Ok(0),
                Some(Ok(bytes)) => {
                    buffer[..bytes.len()].copy_from_slice(&bytes);
                    Ok(bytes.len())
                }
                Some(Err(kind)) => Err(std::io::Error::from(kind)),
            }
        }
    }

    #[test]
    fn the_pump_retries_interrupted_and_reports_read_failures() {
        // Interrupted, then bytes, then EOF: the bytes survive.
        let (sender, receiver) = std::sync::mpsc::sync_channel(8);
        pump_pipe(
            ScriptedRead(
                vec![Err(std::io::ErrorKind::Interrupted), Ok(b"ok".to_vec())].into_iter(),
            ),
            &sender,
            Chunk::Out,
            Chunk::OutEof,
        );
        let chunks: Vec<Chunk> = receiver.try_iter().collect();
        assert!(matches!(&chunks[0], Chunk::Out(bytes) if bytes == b"ok"));
        assert!(matches!(chunks[1], Chunk::OutEof));
        // A definitive error: ReadFailed travels as its own fact,
        // THEN the stream closes.
        let (sender, receiver) = std::sync::mpsc::sync_channel(8);
        pump_pipe(
            ScriptedRead(
                vec![Ok(b"partial".to_vec()), Err(std::io::ErrorKind::BrokenPipe)].into_iter(),
            ),
            &sender,
            Chunk::Out,
            Chunk::OutEof,
        );
        let chunks: Vec<Chunk> = receiver.try_iter().collect();
        assert!(matches!(&chunks[0], Chunk::Out(bytes) if bytes == b"partial"));
        assert!(matches!(chunks[1], Chunk::ReadFailed));
        assert!(matches!(chunks[2], Chunk::OutEof));
    }

    #[test]
    fn the_capture_verdict_is_priority_not_arrival_order() {
        // The reducer's rule (D-2026-08-12-1, review round): among
        // the terminal facts observed up to the tripping drain,
        // OutputOverflow outranks CaptureFailed — whichever order
        // the two pumps delivered them in.
        let both_orders = [
            [Chunk::ReadFailed, Chunk::Out(vec![0u8; 64])],
            [Chunk::Out(vec![0u8; 64]), Chunk::ReadFailed],
        ];
        for chunks in both_orders {
            let mut capture = Capture::new(50);
            for chunk in chunks {
                capture.absorb(chunk);
            }
            assert!(
                matches!(
                    capture.verdict(),
                    Some(ProcessFailure::OutputOverflow {
                        limit: 50,
                        reason: CaptureOverflowReason::Budget,
                    })
                ),
                "priority decides, never arrival order"
            );
        }
        // A read loss alone IS the verdict.
        let mut capture = Capture::new(50);
        capture.absorb(Chunk::ReadFailed);
        assert!(matches!(
            capture.verdict(),
            Some(ProcessFailure::CaptureFailed {})
        ));
        // Under budget, no verdict.
        let mut capture = Capture::new(50);
        capture.absorb(Chunk::Out(vec![0u8; 10]));
        assert_eq!(capture.verdict(), None);
        // FULL priority: Budget > Capacity in BOTH orders — the
        // budget verdict is deterministic in the totals, so a later
        // budget crossing UPGRADES an earlier capacity refusal.
        let mut capture = Capture::new(50);
        capture.capacity_refused = true;
        capture.absorb(Chunk::Out(vec![0u8; 10]));
        assert!(matches!(
            capture.verdict(),
            Some(ProcessFailure::OutputOverflow {
                reason: CaptureOverflowReason::Capacity,
                ..
            })
        ));
        capture.absorb(Chunk::Err(vec![0u8; 64]));
        assert!(
            matches!(
                capture.verdict(),
                Some(ProcessFailure::OutputOverflow {
                    reason: CaptureOverflowReason::Budget,
                    ..
                })
            ),
            "Budget upgrades Capacity, whatever the order"
        );
        let mut capture = Capture::new(50);
        capture.absorb(Chunk::Out(vec![0u8; 64]));
        capture.capacity_refused = true;
        capture.absorb(Chunk::Err(vec![0u8; 8]));
        assert!(matches!(
            capture.verdict(),
            Some(ProcessFailure::OutputOverflow {
                reason: CaptureOverflowReason::Budget,
                ..
            })
        ));
    }

    #[test]
    fn every_adapter_kind_is_in_the_binding_list() {
        // Variant, canonical name and binding list come from ONE
        // declaration (`process_failure_names!` — `kind()` is an
        // exhaustive match, so a new variant cannot compile without
        // joining it); here the declared list meets the sheet.
        assert_eq!(
            ProcessFailure::SpawnFailed {
                message: String::new()
            }
            .kind(),
            "SpawnFailed"
        );
        for kind in ProcessFailure::KINDS {
            assert!(
                crate::stdlib::PROCESS.error_set.contains(kind),
                "`{kind}` must be on the sheet"
            );
        }
        for kind in MACHINE_EMITS {
            assert!(crate::stdlib::PROCESS.error_set.contains(kind));
        }
    }

    /// The framing table of D-2026-08-11-1, exactly: empty stdout
    /// is the empty array, a lone `\n` is one empty line and
    /// refuses, JSON's own whitespace alphabet pads a line and
    /// nothing else does.
    #[test]
    fn the_jsonl_framing_table_holds() {
        use rastok_core::envelope::Value;
        assert_eq!(super::decode_jsonl(""), Ok(vec![]), "empty stdout");
        assert!(
            super::decode_jsonl("\n").is_err(),
            "a lone newline is one EMPTY line, refused — never silently lost"
        );
        assert_eq!(super::decode_jsonl("1\n"), Ok(vec![Value::Int(1)]));
        assert!(
            super::decode_jsonl("1\n\n").is_err(),
            "an interior empty line refuses"
        );
        assert_eq!(
            super::decode_jsonl("1 \t\r\n"),
            Ok(vec![Value::Int(1)]),
            "JSON's whitespace pads a line"
        );
        assert!(
            super::decode_jsonl("1\u{a0}\n").is_err(),
            "a non-breaking space is trailing content, not padding"
        );
    }
}
