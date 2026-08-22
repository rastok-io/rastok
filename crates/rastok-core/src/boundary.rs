//! Contracts with the external world.
//!
//! Governing rule: [`delivery-semantics.md`]. In run, the world plays
//! itself; in simulation, the lab plays the world — both through the
//! same traits, and the journal keeps its world-half either way. The
//! [`Dispatcher`] is the loop between the journal and the outbound drivers:
//! it drives open outbound calls and settles their closures; only a
//! won closure ever becomes a VM input.
//!
//! [`delivery-semantics.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/delivery-semantics.md

use crate::datatype::{
    DataType, EnumType, StructType, TaggedCase, TaggedUnionType, TypeError, TypeField,
};
use crate::envelope::{self, Envelope};
use crate::identity::InstanceRef;
use crate::journal::{CloseVerdict, Closure, Journal};
use crate::node;
use crate::plan::OwnerSlot;
use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// Provenance shared by root envelopes entering through one logical
/// Connector replica: exactly `(connector_path, replica)`.
///
/// This is neither a network address nor the identity of one submission. The
/// journal combines it with an accepted inbound sequence to mint one root;
/// keeping the replica opaque here leaves multi-host placement out of causal
/// identity. In single-process execution the replica is 0.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InboundEnvelopeProvenance {
    connector_path: node::Path,
    replica: u64,
}

impl InboundEnvelopeProvenance {
    /// The single-process provenance of one inbound Connector.
    #[must_use]
    pub fn single(connector_path: node::Path) -> Self {
        Self {
            connector_path,
            replica: 0,
        }
    }

    /// The inbound Connector's root-relative path — root-recipe input.
    #[must_use]
    pub fn connector_path(&self) -> &node::Path {
        &self.connector_path
    }

    /// The logical replica id; 0 in single-process execution.
    #[must_use]
    pub fn replica(&self) -> u64 {
        self.replica
    }
}

/// Deterministic internal call identity — a derivation of the
/// canonical call fields, never a new serialized causal field: it is
/// always recomputable from the record that carries those fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CallRef(envelope::Id);

impl CallRef {
    /// Wraps a derived identity; the recipe lives with the journal's
    /// `ids` module.
    #[must_use]
    pub const fn new(id: envelope::Id) -> Self {
        Self(id)
    }
}

impl fmt::Display for CallRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The exact protocol bytes of a request, sealed: what the journal
/// digests and what the driver receives are THE SAME VALUE — the
/// machine prepares once, the Driver transports, nobody recomputes.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedRequest {
    bytes: alloc::vec::Vec<u8>,
    digest: [u8; 32],
}

impl PreparedRequest {
    /// Seals prepared bytes with their blake3 digest.
    #[must_use]
    pub fn seal(bytes: alloc::vec::Vec<u8>) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&bytes);
        let mut digest = [0u8; 32];
        hasher.finalize_xof().fill(&mut digest);
        Self { bytes, digest }
    }

    /// The exact bytes to send.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The sealed digest of those bytes.
    #[must_use]
    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
}

/// The request-framing codec of a connector — read once from the
/// spec, dispatched here (D-2026-08-11-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestCodec {
    /// Text lines — opaque payloads pass through.
    Lines,
    /// One JSON value per line, the JSON model exactly.
    Jsonl,
    /// One opaque byte payload.
    Bytes,
}

/// Effective logical type of one currently executable Connector port.
///
/// The compiler sheet owns the source-facing rule. This core mirror lets a
/// package validator prove the same table without a compiler ContractCatalog.
#[must_use]
pub fn process_port_type(
    contract: &crate::connector::ConnectorId,
    port: &str,
    codec_word: Option<&str>,
    err_codec_word: Option<&str>,
) -> Option<DataType> {
    if contract.as_str() != crate::connector::PROCESS_CONNECTOR_ID {
        return None;
    }
    match (port, codec_word) {
        ("in" | "out", Some("lines")) => Some(DataType::Text),
        ("in" | "out", Some("bytes")) => Some(DataType::BYTES),
        ("in", Some("jsonl")) => Some(DataType::Variant),
        ("out", Some("jsonl")) => Some(DataType::Variant),
        ("err", Some("lines" | "jsonl" | "bytes")) => match err_codec_word {
            Some("lines") => Some(DataType::Text),
            Some("bytes") => Some(DataType::BYTES),
            _ => None,
        },
        ("exit", Some("lines" | "jsonl" | "bytes")) => process_exit_type().ok(),
        _ => None,
    }
}

/// Latest evaluation site admitted by the currently executable `Process`
/// contract for one attribute.
///
/// The compiler sheet owns the source-facing rule. This core mirror lets a
/// package validator reject a forged late binding without linking the
/// compiler ContractCatalog. Keep it in product agreement with the sheet, just like
/// [`process_port_type`].
#[must_use]
pub fn process_attribute_site(
    contract: &crate::connector::ConnectorId,
    attribute: &str,
) -> Option<crate::plan::EvaluationSite> {
    use crate::plan::EvaluationSite;

    if contract.as_str() != crate::connector::PROCESS_CONNECTOR_ID {
        return None;
    }
    match attribute {
        "mode" | "codec" | "err_codec" | "effect" | "behavior" | "max_output" | "on_timeout" => {
            Some(EvaluationSite::Plan)
        }
        "args" | "command" | "cwd" | "timeout" | "env" | "restart" => {
            Some(EvaluationSite::Instance)
        }
        "correlate" => Some(EvaluationSite::Invocation),
        _ => None,
    }
}

/// The canonical logical type of the conventional `Process.exit` port.
///
/// B5 fixes the migration target beside the old boundary facts; v7 transports
/// this exact union as the ordinary type of the port.
///
/// # Errors
///
/// Returns the canonical type constructor's precise invariant violation.
pub fn process_exit_type() -> Result<DataType, TypeError> {
    let empty = || StructType::new(Vec::new());
    let optional_int = DataType::INT.optional()?;
    let optional_text = DataType::Text.optional()?;
    let exited = TaggedCase::new(
        "Exited",
        StructType::new(alloc::vec![
            TypeField::new("exit_code", DataType::INT),
            TypeField::new("signal", optional_int),
        ])?,
    )?;
    let spawn_failed = TaggedCase::new(
        "SpawnFailed",
        StructType::new(alloc::vec![TypeField::new("message", DataType::Text)])?,
    )?;
    let timeout_expired = TaggedCase::new("TimeoutExpired", empty()?)?;
    let decode_failed = TaggedCase::new(
        "DecodeFailed",
        StructType::new(alloc::vec![
            TypeField::new("line", optional_text),
            TypeField::new("message", DataType::Text),
        ])?,
    )?;
    let capture_failed = TaggedCase::new("CaptureFailed", empty()?)?;
    let outcome_unknown = TaggedCase::new("OutcomeUnknown", empty()?)?;
    let reason = DataType::Enum(EnumType::new(alloc::vec![
        String::from("budget"),
        String::from("capacity"),
    ])?);
    let output_overflow = TaggedCase::new(
        "OutputOverflow",
        StructType::new(alloc::vec![
            TypeField::new("limit", DataType::UINT),
            TypeField::new("reason", reason),
        ])?,
    )?;
    Ok(DataType::TaggedUnion(TaggedUnionType::new(alloc::vec![
        exited,
        spawn_failed,
        timeout_expired,
        decode_failed,
        capture_failed,
        outcome_unknown,
        output_overflow,
    ])?))
}

/// The semantic terminal facts emitted by `Process`.
///
/// This Rust enum is a Driver-side construction aid, not a second logical
/// type. [`ProcessExit::into_value`] is the sole projection into the ordinary
/// canonical [`crate::value::Value::TaggedUnion`] carried by the `exit` port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessExit {
    /// The child status was observed; non-zero is data, not an automatic
    /// failure classification. `exit_code = -1` denotes a signal-only status.
    Exited {
        /// Portable process exit code, or `-1` when only a signal exists.
        exit_code: i32,
        /// Unix signal when available.
        signal: Option<i32>,
    },
    /// The OS refused to create the child.
    SpawnFailed {
        /// Boundary diagnostic.
        message: String,
    },
    /// The scheduler's call budget won the close race.
    TimeoutExpired,
    /// Captured stdout or stderr did not inhabit its selected framing.
    DecodeFailed {
        /// Framing or decoding diagnostic.
        message: String,
        /// Offending JSONL line when one exists.
        line: Option<String>,
    },
    /// A pipe or status capture could not be completed.
    CaptureFailed,
    /// Cancellation was requested, but no terminal observation arrived
    /// within the instance's explicit settlement budget.
    OutcomeUnknown,
    /// The aggregate capture bound or representable host capacity was hit.
    OutputOverflow {
        /// Declared aggregate byte limit.
        limit: u64,
        /// Why the bounded capture stopped.
        reason: OutputOverflowReason,
    },
}

/// Why a bounded Process capture stopped.
///
/// Keeping this closed beside [`ProcessExit`] prevents a Driver from
/// constructing an enum value outside the logical `ProcessExit` contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputOverflowReason {
    /// The declared aggregate byte budget was crossed.
    Budget,
    /// The host refused capture capacity.
    Capacity,
}

impl OutputOverflowReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Budget => "budget",
            Self::Capacity => "capacity",
        }
    }
}

impl ProcessExit {
    /// Projects the Driver fact into the ordinary canonical tagged value.
    #[must_use]
    pub fn into_value(self) -> crate::value::Value {
        use crate::value::{TaggedUnionValue, Value};

        let optional_int = |value: Option<i32>| Value::StructuralUnion {
            branch: u16::from(value.is_some()),
            value: alloc::boxed::Box::new(
                value.map_or(Value::Null, |value| Value::Int(i64::from(value))),
            ),
        };
        let optional_text = |value: Option<String>| Value::StructuralUnion {
            branch: u16::from(value.is_some()),
            value: alloc::boxed::Box::new(value.map_or(Value::Null, Value::Text)),
        };
        let (case, payload) = match self {
            Self::Exited { exit_code, signal } => (
                "Exited",
                alloc::vec![
                    (String::from("exit_code"), Value::Int(i64::from(exit_code))),
                    (String::from("signal"), optional_int(signal)),
                ],
            ),
            Self::SpawnFailed { message } => (
                "SpawnFailed",
                alloc::vec![(String::from("message"), Value::Text(message))],
            ),
            Self::TimeoutExpired => ("TimeoutExpired", Vec::new()),
            Self::DecodeFailed { message, line } => (
                "DecodeFailed",
                alloc::vec![
                    (String::from("line"), optional_text(line)),
                    (String::from("message"), Value::Text(message)),
                ],
            ),
            Self::CaptureFailed => ("CaptureFailed", Vec::new()),
            Self::OutcomeUnknown => ("OutcomeUnknown", Vec::new()),
            Self::OutputOverflow { limit, reason } => (
                "OutputOverflow",
                alloc::vec![
                    (String::from("limit"), Value::Uint(limit)),
                    (
                        String::from("reason"),
                        Value::Enum(String::from(reason.as_str()))
                    ),
                ],
            ),
        };
        Value::TaggedUnion(alloc::boxed::Box::new(TaggedUnionValue {
            case: String::from(case),
            payload,
        }))
    }
}

/// Frames one request under the given codec.
///
/// # Errors
/// The typed decode refusal, when the payload has no frame under
/// this codec.
pub fn prepare_request(
    codec: RequestCodec,
    data: &envelope::Data,
) -> Result<PreparedRequest, String> {
    match codec {
        RequestCodec::Lines => prepare_lines(data),
        RequestCodec::Jsonl => prepare_jsonl(data),
        RequestCodec::Bytes => prepare_bytes(data),
    }
}

/// Frames one Process request or returns its exact typed pre-call refusal.
///
/// This helper is shared by the real Driver and scripted Process worlds; it
/// is not a VM fallback. The selected Driver binding calls it explicitly.
#[must_use]
pub fn prepare_process_request(codec: RequestCodec, data: &envelope::Data) -> RequestPreparation {
    match prepare_request(codec, data) {
        Ok(prepared) => RequestPreparation::Prepared(prepared),
        Err(message) => {
            RequestPreparation::Rejected(Completion::process_exit(ProcessExit::DecodeFailed {
                message,
                line: None,
            }))
        }
    }
}

/// Projects a machine-decided terminal fact into the Process contract.
///
/// Like [`prepare_process_request`], this is selected by a bound Process
/// Driver and is never concrete-name dispatch in the VM.
#[must_use]
pub fn process_scheduler_completion(completion: SchedulerCompletion) -> Completion {
    Completion::process_exit(match completion {
        SchedulerCompletion::TimeoutExpired => ProcessExit::TimeoutExpired,
        SchedulerCompletion::OutcomeUnknown => ProcessExit::OutcomeUnknown,
    })
}

/// Frames one opaque byte request without text or data-model conversion.
///
/// # Errors
///
/// The typed refusal when the payload is neither raw encoded bytes nor the
/// materialized bytes scalar.
pub fn prepare_bytes(data: &envelope::Data) -> Result<PreparedRequest, String> {
    use envelope::{Data, Encoding, Value};
    let bytes = match data {
        Data::Encoded {
            encoding: Encoding::Raw,
            bytes,
            ..
        }
        | Data::Materialized(Value::Bytes(bytes)) => bytes.clone(),
        Data::Encoded { encoding, .. } => {
            return Err(alloc::format!(
                "the bytes codec requires raw bytes; the request is {encoding:?}-encoded"
            ));
        }
        Data::Materialized(other) => {
            return Err(alloc::format!(
                "the bytes codec requires bytes; the request data is {other:?}"
            ));
        }
    };
    Ok(PreparedRequest::seal(bytes))
}

/// Frames a request as ONE canonical JSON line (D-2026-08-11-1):
/// the writer's bytes are `request_digest` material. Encoded
/// payloads and byte scalars refuse typed — `jsonl` frames the JSON
/// model, not opaque representations.
///
/// # Errors
/// The typed decode refusal.
pub fn prepare_jsonl(data: &envelope::Data) -> Result<PreparedRequest, String> {
    use envelope::Data;
    let value = match data {
        Data::Materialized(value) => value,
        Data::Encoded { encoding, .. } => {
            return Err(alloc::format!(
                "the jsonl codec frames the JSON model; the request is {encoding:?}-encoded"
            ));
        }
    };
    let mut line = crate::json::write_value(value)
        .map_err(|error| alloc::format!("the request has no JSON form: {}", error.message))?;
    line.push('\n');
    Ok(PreparedRequest::seal(line.into_bytes()))
}

/// Prepares the plain-text request wire: raw bytes or the textual
/// scalar's UTF-8 form, exactly as before the codec split.
pub fn prepare_lines(data: &envelope::Data) -> Result<PreparedRequest, String> {
    use envelope::{Data, Encoding, Value};
    let bytes = match data {
        Data::Encoded {
            encoding: Encoding::Raw,
            bytes,
            ..
        } => bytes.clone(),
        Data::Materialized(Value::Text(text)) => {
            let mut bytes = text.clone().into_bytes();
            if bytes.last() != Some(&b'\n') {
                bytes.push(b'\n');
            }
            bytes
        }
        Data::Encoded { encoding, .. } => {
            return Err(alloc::format!(
                "the lines codec frames text; the request is {encoding:?}-encoded"
            ));
        }
        Data::Materialized(other) => {
            return Err(alloc::format!(
                "the lines codec frames text; the request data is {other:?}"
            ));
        }
    };
    Ok(PreparedRequest::seal(bytes))
}

/// Canonical journaled outbound-call intent (`identity.md` §3, keys
/// 40–43 and 47): journaled open before the effect executes.
#[derive(Debug, Clone, PartialEq)]
pub struct Intent {
    /// The derived internal call identity.
    pub call_ref: CallRef,
    /// The envelope that opened the call — the cause.
    pub emitting_id: envelope::Id,
    /// The canonical edge of the causing delivery (key 47): with the
    /// emitting id it reconstructs the delivery reference and hence
    /// the call identity — the record stays autonomous.
    pub edge: String,
    /// The calling connector node.
    pub connector_path: node::Path,
    /// The prepared request — the sealed digest is canonical (key
    /// 43); the bytes ride along for the driver and are never
    /// themselves record material.
    pub prepared: PreparedRequest,
    /// Names the business operation; stable across retries. `None`
    /// only for `compensable` without a key.
    pub idempotency_key: Option<String>,
    /// The declared effect class of the call.
    pub effect: node::EffectClass,
    /// Effective Connector attributes evaluated once in the selected
    /// instance bundle. They are derived from the sealed plan and instance
    /// context; Drivers consume them but never choose defaults or evaluate
    /// expressions.
    pub instance_settings: alloc::collections::BTreeMap<String, crate::value::Value>,
}

impl Intent {
    /// The boundary integrity seal over the exact prepared bytes.
    #[must_use]
    pub fn request_digest(&self) -> &[u8; 32] {
        self.prepared.digest()
    }
}

/// One ordinary output proposed by an external driver.
///
/// This is deliberately not a journal [`crate::journal::Emission`]: the
/// driver states a port and payload, while the VM alone mints envelopes,
/// identities and the atomic transaction record.
#[derive(Debug, Clone, PartialEq)]
pub struct PortOutput {
    /// Ordinary connector output port.
    pub port: node::Port,
    /// Payload representation returned by the driver.
    pub data: envelope::Data,
}

/// One admitted invocation's ordered boundary completion.
///
/// Construction seals the terminal barrier: exactly one conventional
/// `exit` output exists and it is last. Consequently neither a driver nor a
/// mock can represent an output that follows the invocation's terminal fact.
#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    outputs: Vec<PortOutput>,
}

impl Completion {
    /// Builds one completion after proving its terminal barrier.
    ///
    /// # Errors
    ///
    /// Refuses a missing, repeated, or non-terminal `exit` output.
    pub fn try_new(outputs: Vec<PortOutput>) -> Result<Self, CompletionError> {
        // `exit` is the boundary protocol's conventional terminal port, not a
        // VM routing shortcut. The resolved port table owns its type and the
        // graph still routes it like every other named output.
        let exit_count = outputs
            .iter()
            .filter(|output| output.port.name() == "exit")
            .count();
        match exit_count {
            0 => return Err(CompletionError::MissingExit),
            1 => {}
            count => return Err(CompletionError::MultipleExits(count)),
        }
        if outputs
            .last()
            .is_none_or(|output| output.port.name() != "exit")
        {
            return Err(CompletionError::OutputAfterExit);
        }
        Ok(Self { outputs })
    }

    /// Outputs in publication order, including the terminal `exit` last.
    #[must_use]
    pub fn outputs(&self) -> &[PortOutput] {
        &self.outputs
    }

    /// Gives the ordered drafts to the VM after the close race is won.
    #[must_use]
    pub fn into_outputs(self) -> Vec<PortOutput> {
        self.outputs
    }

    /// Builds the bounded on-exit completion of one `Process(job)`.
    ///
    /// Separating the two frame vectors in this API makes the fixed ordering
    /// (`out*`, then `err*`, then terminal `exit`) structural and prevents an
    /// Driver from accidentally placing lifecycle data on stderr.
    #[must_use]
    pub fn process(out: Vec<envelope::Data>, err: Vec<envelope::Data>, exit: ProcessExit) -> Self {
        let mut outputs = Vec::with_capacity(out.len().saturating_add(err.len()).saturating_add(1));
        outputs.extend(out.into_iter().map(|data| PortOutput {
            port: node::Port::conventional_out(),
            data,
        }));
        outputs.extend(err.into_iter().map(|data| PortOutput {
            port: node::Port::conventional_err(),
            data,
        }));
        outputs.push(PortOutput {
            port: node::Port::conventional_exit(),
            data: envelope::Data::materialized(exit.into_value()),
        });
        Self { outputs }
    }

    /// Builds a completion containing only its terminal Process fact.
    #[must_use]
    pub fn process_exit(exit: ProcessExit) -> Self {
        Self::process(Vec::new(), Vec::new(), exit)
    }
}

/// A malformed invocation completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionError {
    /// No terminal fact was supplied.
    MissingExit,
    /// More than one terminal fact was supplied.
    MultipleExits(usize),
    /// The sole terminal fact was not the final output.
    OutputAfterExit,
}

impl fmt::Display for CompletionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingExit => f.write_str("a completion requires exactly one `exit` output"),
            Self::MultipleExits(count) => {
                write!(
                    f,
                    "a completion carries {count} `exit` outputs; exactly one is required"
                )
            }
            Self::OutputAfterExit => {
                f.write_str("the terminal `exit` must be the completion's final output")
            }
        }
    }
}

impl core::error::Error for CompletionError {}

/// Protocol-correlated driver answer. A driver answers or stays
/// silent — never a timeout: timeouts are the scheduler's.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    /// The call this answers, correlated by the connector's own
    /// protocol.
    pub call_ref: CallRef,
    /// Ordered ordinary outputs plus the terminal invocation fact.
    pub completion: Completion,
}

/// Result of framing one invocation before its outbound record opens.
///
/// A rejection is an ordinary, contract-typed completion. It commits through
/// the same connector transaction as any other completion, but no external
/// effect or outbound-open record exists because there were no protocol bytes
/// to send.
#[derive(Debug, Clone, PartialEq)]
pub enum RequestPreparation {
    /// Exact bytes whose digest becomes call identity material.
    Prepared(PreparedRequest),
    /// The contract's typed terminal response to an unframeable payload.
    Rejected(Completion),
}

/// A terminal fact whose occurrence is decided by the machine, while its
/// typed representation belongs to the Connector contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerCompletion {
    /// The invocation budget won the close-once race.
    TimeoutExpired,
    /// Cancellation was requested but no observation arrived by settlement.
    OutcomeUnknown,
}

/// A route through an already-verified binding became impossible.
///
/// These errors are machine-integrity faults, not domain outcomes. The global
/// binder proves every route before the VM and journal exist; retaining a
/// typed belt here prevents a forged or internally inconsistent caller from
/// becoming a silent no-op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverRouteError {
    /// No outbound binding owns this planned Connector path.
    UnboundOutbound(node::Path),
    /// A role view proven at bind time disappeared later.
    UnavailableOutbound(node::Path),
    /// An inbound role view proven at bind time disappeared later.
    UnavailableInbound(crate::connector::ConnectorId),
    /// One Driver answered a call started through another Connector family.
    CrossFamilyResponse {
        /// Correlated call.
        call_ref: CallRef,
        /// Family that started it.
        expected: crate::connector::ConnectorId,
        /// Family that answered it.
        actual: crate::connector::ConnectorId,
    },
}

impl fmt::Display for DriverRouteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnboundOutbound(connector) => {
                write!(formatter, "no outbound Driver binding owns `{connector}`")
            }
            Self::UnavailableOutbound(connector) => write!(
                formatter,
                "the outbound Driver role bound for `{connector}` is no longer available"
            ),
            Self::UnavailableInbound(id) => write!(
                formatter,
                "the inbound Driver role bound for `{id}` is no longer available"
            ),
            Self::CrossFamilyResponse {
                call_ref,
                expected,
                actual,
            } => write!(
                formatter,
                "Driver `{actual}` answered call {call_ref} started by `{expected}`"
            ),
        }
    }
}

impl core::error::Error for DriverRouteError {}

/// Judges a driver's completion against the calling connector's resolved
/// contract before it may become a causal fact. The dispatcher only asks.
pub trait CompletionGate {
    /// `Ok` to admit; `Err(reason)` to refuse the closure.
    ///
    /// # Errors
    ///
    /// The refusal reason, surfaced with the rejected response.
    fn admit(&self, connector: &node::Path, completion: &Completion) -> Result<(), String>;
}

/// The gate that admits everything — for worlds whose completions are
/// validated elsewhere, and for tests of the dispatch mechanics.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenGate;

impl CompletionGate for OpenGate {
    fn admit(&self, _: &node::Path, _: &Completion) -> Result<(), String> {
        Ok(())
    }
}

/// One routable boundary fact for the VM: which connector, which
/// port, which cause. The VM owns what follows — the connector's
/// transaction builds every output envelope under the emission recipe;
/// the dispatcher never mints envelopes.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundaryEvent {
    /// The settled call.
    pub call_ref: CallRef,
    /// The connector node that owns the coming transaction.
    pub connector_path: node::Path,
    /// The request envelope's id — the transaction's parent.
    pub emitting_id: envelope::Id,
    /// What the call produced, in publication order with `exit` last.
    pub completion: Completion,
}

/// How a submission's sequence number is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionSequence {
    /// Allocate the next number — the live mode.
    Next,
    /// Replay an explicit number — fixtures replay recorded seqs; a
    /// mismatch with the expected next number is refused.
    Explicit(u64),
}

/// A draft proposed for atomic acceptance: inbound Drivers propose drafts,
/// never envelopes — a root envelope cannot pre-exist its sequence.
#[derive(Debug, Clone, PartialEq)]
pub struct InboundSubmission {
    /// Stable external origin shared by roots from this logical replica.
    pub provenance: InboundEnvelopeProvenance,
    /// The graph input the draft enters — `in` or a declared
    /// boundary input. Journaled with the record: replay must
    /// re-route from the same port, so the port is a causal fact.
    pub port: node::Port,
    /// The payload and birth metadata.
    pub draft: Draft,
    /// How the sequence is chosen.
    pub sequence: SubmissionSequence,
}

/// The payload and birth metadata of a submission; the runtime mints
/// `meta.sys` — a draft never carries one.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    /// The payload.
    pub data: envelope::Data,
    /// The external-context zone seeded at birth — the world's
    /// half: the transport's `content_type` and whatever the
    /// boundary's sheet declares. `meta.app` is NEVER seeded at
    /// ingress: the application writes its own zone
    /// (D-2026-08-10-1).
    pub ext: envelope::Namespace,
}

/// One scheduler-owned event with a logical date. Virtual time is
/// discrete and journal-relative; the absolute value is derivable and
/// never sealed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatedEvent {
    /// The logical due instant, in nanoseconds of virtual time.
    pub at: i64,
    /// What comes due.
    pub payload: DatedPayload,
}

/// What a dated event carries. Per-call timeout budgets are
/// scheduler-owned; further obligations (lifetimes, windows) join
/// with their stages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatedPayload {
    /// A delivery's call budget expired. The budget runs from the
    /// DELIVERY into the connector's mailbox (execution-model §8) —
    /// queue time counts — and the obligation resolves by its
    /// delivery reference `(edge, envelope_id)`, whether the call is
    /// still queued or already open.
    DeliveryBudget {
        /// The connector whose budget this is (the mailbox to search).
        connector: node::Path,
        /// Half of the delivery reference: the canonical edge the
        /// envelope traveled. The obligation's identity is
        /// `budget_ref(edge, delivery)` — never the envelope id
        /// alone: two ports of one connector are two obligations.
        edge: String,
        /// The other half of the reference: the delivered envelope's
        /// id.
        delivery: envelope::Id,
        /// Exact connector instance owning the mailbox/call obligation.
        instance: InstanceRef,
    },
    /// One retained join candidate reached its `within` bound. The full
    /// delivery reference makes a consumed candidate's stale event harmless.
    JoinWithin {
        /// Operator owning the join buffer.
        operator: node::Path,
        /// Edge half of the buffered-delivery reference.
        edge: String,
        /// Envelope-id half of the buffered-delivery reference.
        delivery: envelope::Id,
        /// Exact Operator generation that retained it.
        instance: InstanceRef,
    },
    /// One owner lifetime reached a scheduler-owned deadline. The complete
    /// instance reference is the generation fence; idle additionally compares
    /// the causal admission id, while fixed and tumbling never move after
    /// `l:start`.
    InstanceLifetime {
        /// Plan owner whose policy armed the event.
        owner: OwnerSlot,
        /// Exact generation whose deadline is being observed.
        instance: InstanceRef,
        /// Envelope whose arrival armed this deadline and causes `l:stop`.
        cause: envelope::Id,
        /// Which profile determines the stale-event rule.
        deadline: InstanceDeadline,
    },
    /// One phase boundary of the explicit bounded Closing protocol.
    InstanceClosing {
        /// Root generation whose policy owns the whole closing subtree.
        instance: InstanceRef,
        /// Envelope that won the close request.
        cause: envelope::Id,
        /// Drain or settlement deadline.
        deadline: ClosingDeadline,
    },
}

/// Scheduler-owned stages of bounded instance Closing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosingDeadline {
    /// Stop executing admitted ordinary work and request cancellation.
    Drain,
    /// Stop waiting for cancellation observations; unresolved calls become
    /// `OutcomeUnknown` and their outbound records become `abandoned`.
    Settlement,
}

/// Scheduler semantics of a finite dated instance lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceDeadline {
    /// Superseded by any later admitted envelope in the same generation.
    Idle,
    /// Fixed relative to the creating `l:start`.
    Fixed,
    /// Fixed at the end of the address's epoch-aligned processing-time box.
    Tumbling,
}

/// Boot, crash, or restart — supervision facts entering the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleEvent {
    /// The process starts.
    Boot,
    /// The process dies mid-flight.
    Crash,
    /// The process comes back.
    Restart,
}

/// Supplies inbound submissions under backpressure: bounded, births
/// drafts and never envelopes — a root cannot pre-exist its sequence.
pub trait InboundConnectorDriver {
    /// The next submission, if one is waiting.
    fn next(&mut self) -> Option<InboundSubmission>;
}

/// Starts external calls and supplies their answers. A driver
/// answers or stays silent — never a timeout and never an unknown
/// outcome: budgets belong to the scheduler, triage to the recovery
/// fold. Correlation uses the connector's own protocol.
pub trait OutboundConnectorDriver {
    /// Frames one invocation under this bound contract before the machine
    /// opens its outbound record. The Driver chooses no defaults: binding has
    /// already resolved the contract and instance settings for `connector`.
    fn prepare_request(
        &mut self,
        connector: &node::Path,
        data: &envelope::Data,
    ) -> RequestPreparation;

    /// Builds the contract-typed representation of a machine-decided
    /// terminal fact. The scheduler decides that the fact occurred; this
    /// method only projects it into the Connector's ordinary output protocol.
    fn scheduler_completion(
        &mut self,
        connector: &node::Path,
        completion: SchedulerCompletion,
    ) -> Completion;

    /// Withdraws an open call: the machine closed it (a won timeout)
    /// and any in-flight work should stop. Default: nothing — a
    /// scripted world has nothing to stop.
    fn cancel(&mut self, call_ref: &CallRef) {
        let _ = call_ref;
    }

    /// Executes the effect of an already-journaled open call. The
    /// request envelope rides along because the intent seals only the
    /// digest of the request, not its bytes.
    fn start(&mut self, intent: &Intent, request: &Envelope);

    /// The answers that arrived since the last drain.
    fn events(&mut self) -> Vec<Response>;
}

/// The verified outbound routing table consumed by the core VM.
///
/// A single Driver implements this automatically. The application-level
/// `RuntimeBindings` implementation selects among several contract families
/// and returns a typed error if its pre-VM proof is ever contradicted. The VM
/// therefore never matches `Process`, `File`, or another concrete contract.
pub trait OutboundConnectorBindings {
    /// Frames one request through the Driver bound to `connector`.
    ///
    /// # Errors
    ///
    /// The planned path has no stable outbound route.
    fn prepare_request(
        &mut self,
        connector: &node::Path,
        data: &envelope::Data,
    ) -> Result<RequestPreparation, DriverRouteError>;

    /// Projects a scheduler fact through the bound contract.
    ///
    /// # Errors
    ///
    /// The planned path has no stable outbound route.
    fn scheduler_completion(
        &mut self,
        connector: &node::Path,
        completion: SchedulerCompletion,
    ) -> Result<Completion, DriverRouteError>;

    /// Withdraws one call through its owning binding.
    ///
    /// # Errors
    ///
    /// The planned path has no stable outbound route.
    fn cancel(
        &mut self,
        connector: &node::Path,
        call_ref: &CallRef,
    ) -> Result<(), DriverRouteError>;

    /// Starts one already-journaled call through its owning binding.
    ///
    /// # Errors
    ///
    /// The planned path has no stable outbound route.
    fn start(&mut self, intent: &Intent, request: &Envelope) -> Result<(), DriverRouteError>;

    /// Collects all answers since the previous round.
    ///
    /// # Errors
    ///
    /// A binding invariant was contradicted while polling.
    fn events(&mut self) -> Result<Vec<Response>, DriverRouteError>;
}

impl<T> OutboundConnectorBindings for T
where
    T: OutboundConnectorDriver + ?Sized,
{
    fn prepare_request(
        &mut self,
        connector: &node::Path,
        data: &envelope::Data,
    ) -> Result<RequestPreparation, DriverRouteError> {
        Ok(OutboundConnectorDriver::prepare_request(
            self, connector, data,
        ))
    }

    fn scheduler_completion(
        &mut self,
        connector: &node::Path,
        completion: SchedulerCompletion,
    ) -> Result<Completion, DriverRouteError> {
        Ok(OutboundConnectorDriver::scheduler_completion(
            self, connector, completion,
        ))
    }

    fn cancel(
        &mut self,
        _connector: &node::Path,
        call_ref: &CallRef,
    ) -> Result<(), DriverRouteError> {
        OutboundConnectorDriver::cancel(self, call_ref);
        Ok(())
    }

    fn start(&mut self, intent: &Intent, request: &Envelope) -> Result<(), DriverRouteError> {
        OutboundConnectorDriver::start(self, intent, request);
        Ok(())
    }

    fn events(&mut self) -> Result<Vec<Response>, DriverRouteError> {
        Ok(OutboundConnectorDriver::events(self))
    }
}

/// Supplies one due event at a time: the VM drains to a stable state
/// between events, which may arm new obligations before the target.
pub trait Clock {
    /// The next event due at or before `target`, if any.
    fn next_due(&mut self, target: i64) -> Option<DatedEvent>;
}

/// Supplies lifecycle events.
pub trait Supervisor {
    /// The next supervision fact, if one is pending.
    fn next_event(&mut self) -> Option<LifecycleEvent>;
}

/// One settled pump round: what may enter the VM, and what may not —
/// visibly, never silently.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PumpBatch {
    /// The won closures, routable to the VM.
    pub events: Vec<BoundaryEvent>,
    /// Answers that lost the close race — non-causal observations,
    /// invisible to the graph but never to the system.
    pub discarded: Vec<Response>,
    /// Answers correlating with no journaled call — correlation
    /// faults to surface.
    pub uncorrelated: Vec<Response>,
    /// Answers the completion gate refused, with the reason. The call
    /// stays OPEN — an invalid reply never becomes a causal fact,
    /// and the timeout machinery can still settle the call honestly.
    pub rejected: Vec<(Response, String)>,
}

/// A dispatcher fault — loud, never a skipped iteration.
#[derive(Debug, Clone, PartialEq)]
pub enum DispatchError {
    /// A journaled intent whose request envelope cannot be found:
    /// an integrity fault in the causal history.
    MissingRequest(CallRef),
    /// The verified Driver routing table was contradicted.
    Driver(DriverRouteError),
    /// The journal refused an operation.
    Journal(crate::journal::Error),
}

impl fmt::Display for DispatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRequest(call_ref) => {
                write!(
                    f,
                    "no journaled envelope for the request of call {call_ref}"
                )
            }
            Self::Driver(error) => error.fmt(f),
            Self::Journal(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for DispatchError {}

/// The loop between the journal and the outbound drivers. It starts every
/// journaled open call exactly once — journal-before-effect: the
/// record always exists before the driver hears — and settles
/// closures through the journal's close-once race into routable
/// [`BoundaryEvent`]s. The VM owns everything after: the connector's
/// transaction builds the output envelopes; the dispatcher never
/// mints one. Mocks replace the driver, never this bookkeeping.
///
/// Restart discipline: re-calling a driver is a recovery decision
/// (idempotent or keyed calls may be re-called, a compensable call
/// without a key may not — that triage belongs to the recovery fold).
/// A dispatcher over an existing journal is therefore built with
/// [`Dispatcher::resume`], which treats every already-open call as
/// started and re-calls nothing.
#[derive(Debug, Default)]
pub struct Dispatcher {
    started: BTreeSet<CallRef>,
}

impl Dispatcher {
    /// A dispatcher for a fresh run: nothing started yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A dispatcher over an existing journal: every currently open
    /// call counts as started, so nothing is blindly re-called; the
    /// recovery fold — not the dispatcher — decides re-calls.
    #[must_use]
    pub fn resume(journal: &dyn Journal) -> Self {
        Self {
            started: journal
                .open_calls()
                .into_iter()
                .map(|intent| intent.call_ref)
                .collect(),
        }
    }

    /// Drives one round: starts newly journaled open calls, then
    /// settles the driver's answers through the close-once race.
    ///
    /// # Errors
    ///
    /// [`DispatchError::MissingRequest`] when the causal history lacks
    /// a journaled request — an integrity fault; journal refusals are
    /// propagated, never absorbed.
    pub fn pump<B: OutboundConnectorBindings + ?Sized>(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut B,
        gate: &dyn CompletionGate,
    ) -> Result<PumpBatch, DispatchError> {
        // Start phase: every open call not yet started.
        for intent in journal.open_calls() {
            if self.started.contains(&intent.call_ref) {
                continue;
            }
            let request = journal
                .find_envelope(intent.emitting_id)
                .ok_or(DispatchError::MissingRequest(intent.call_ref))?;
            bindings
                .start(&intent, &request)
                .map_err(DispatchError::Driver)?;
            self.started.insert(intent.call_ref);
        }
        // Settle phase: won closures become routable events; losers
        // and strays stay visible.
        let mut batch = PumpBatch::default();
        let open = journal.open_calls();
        for response in bindings.events().map_err(DispatchError::Driver)? {
            let Some(intent) = open
                .iter()
                .find(|intent| intent.call_ref == response.call_ref)
            else {
                if journal.find_call(&response.call_ref).is_some() {
                    batch.discarded.push(response);
                } else {
                    batch.uncorrelated.push(response);
                }
                continue;
            };
            // Validation BEFORE the causal fact: a reply the contract
            // refuses must never close the call — journaled garbage
            // would leave an unclosable obligation.
            if let Err(reason) = gate.admit(&intent.connector_path, &response.completion) {
                batch.rejected.push((response, reason));
                continue;
            }
            let closure = Closure::Response(response.completion.clone());
            match journal
                .close_outbound(&response.call_ref, closure)
                .map_err(DispatchError::Journal)?
            {
                CloseVerdict::Won => {
                    self.started.remove(&response.call_ref);
                    batch.events.push(BoundaryEvent {
                        call_ref: response.call_ref,
                        connector_path: intent.connector_path.clone(),
                        emitting_id: intent.emitting_id,
                        completion: response.completion,
                    });
                }
                CloseVerdict::AlreadyClosed => batch.discarded.push(response),
            }
        }
        Ok(batch)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn prepare_lines_frames_exactly_one_trailing_newline() {
        use crate::envelope::{Data, Encoding, Value};
        let framed =
            prepare_lines(&Data::materialized(Value::Text(String::from("ligne")))).unwrap();
        assert_eq!(framed.bytes(), b"ligne\n");
        let already =
            prepare_lines(&Data::materialized(Value::Text(String::from("ligne\n")))).unwrap();
        assert_eq!(already.bytes(), b"ligne\n");
        let raw = prepare_lines(&Data::encoded(Encoding::Raw, *b"verbatim")).unwrap();
        assert_eq!(raw.bytes(), b"verbatim");
        // The seal is the blake3 of exactly those bytes.
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"verbatim");
        let mut digest = [0u8; 32];
        hasher.finalize_xof().fill(&mut digest);
        assert_eq!(raw.digest(), &digest);
    }

    #[test]
    fn the_bytes_codec_never_falls_back_to_lines() {
        use crate::envelope::{Data, Value};
        let framed = prepare_request(
            RequestCodec::Bytes,
            &Data::materialized(Value::Bytes(alloc::vec![0, 0xff, b'\n'])),
        )
        .unwrap();
        assert_eq!(framed.bytes(), &[0, 0xff, b'\n']);
        assert!(
            prepare_request(
                RequestCodec::Bytes,
                &Data::materialized(Value::Text(String::from("not bytes"))),
            )
            .is_err()
        );
    }

    use super::*;
    use crate::envelope::{Data, Encoding, Namespace};
    use crate::journal::{MemJournal, ids};
    use crate::plan::GraphVersion;
    use alloc::string::ToString;
    use alloc::vec;

    #[test]
    fn inbound_provenance_stays_opaque_but_answers_the_recipes() {
        let provenance = InboundEnvelopeProvenance::single(node::Path::new(["inbound"]).unwrap());
        assert_eq!(provenance.connector_path().to_string(), "inbound");
        assert_eq!(provenance.replica(), 0);
    }

    /// A scripted driver: records what it was asked to start,
    /// answers from a script.
    #[derive(Default)]
    struct Scripted {
        started: Vec<(Intent, Envelope)>,
        replies: Vec<Response>,
    }

    impl OutboundConnectorDriver for Scripted {
        fn prepare_request(&mut self, _: &node::Path, data: &Data) -> RequestPreparation {
            match prepare_lines(data) {
                Ok(prepared) => RequestPreparation::Prepared(prepared),
                Err(message) => RequestPreparation::Rejected(Completion::process_exit(
                    ProcessExit::DecodeFailed {
                        message,
                        line: None,
                    },
                )),
            }
        }

        fn scheduler_completion(
            &mut self,
            _: &node::Path,
            completion: SchedulerCompletion,
        ) -> Completion {
            Completion::process_exit(match completion {
                SchedulerCompletion::TimeoutExpired => ProcessExit::TimeoutExpired,
                SchedulerCompletion::OutcomeUnknown => ProcessExit::OutcomeUnknown,
            })
        }

        fn start(&mut self, intent: &Intent, request: &Envelope) {
            self.started.push((intent.clone(), request.clone()));
        }

        fn events(&mut self) -> Vec<Response> {
            core::mem::take(&mut self.replies)
        }
    }

    struct StartRefusal {
        refuse: bool,
        attempts: usize,
    }

    impl OutboundConnectorBindings for StartRefusal {
        fn prepare_request(
            &mut self,
            _: &node::Path,
            data: &Data,
        ) -> Result<RequestPreparation, DriverRouteError> {
            Ok(prepare_process_request(RequestCodec::Lines, data))
        }

        fn scheduler_completion(
            &mut self,
            _: &node::Path,
            completion: SchedulerCompletion,
        ) -> Result<Completion, DriverRouteError> {
            Ok(process_scheduler_completion(completion))
        }

        fn cancel(&mut self, _: &node::Path, _: &CallRef) -> Result<(), DriverRouteError> {
            Ok(())
        }

        fn start(&mut self, intent: &Intent, _: &Envelope) -> Result<(), DriverRouteError> {
            self.attempts += 1;
            if self.refuse {
                return Err(DriverRouteError::UnboundOutbound(
                    intent.connector_path.clone(),
                ));
            }
            Ok(())
        }

        fn events(&mut self) -> Result<Vec<Response>, DriverRouteError> {
            Ok(Vec::new())
        }
    }

    fn seeded_journal() -> (MemJournal, Intent) {
        let mut journal = MemJournal::new(GraphVersion(1));
        let committed = journal
            .append_inbound(InboundSubmission {
                provenance: InboundEnvelopeProvenance::single(
                    node::Path::new(["inbound"]).unwrap(),
                ),
                port: node::Port::new("in").unwrap(),
                draft: Draft {
                    data: Data::encoded(Encoding::Raw, *b"salut"),
                    ext: Namespace::new(),
                },
                sequence: SubmissionSequence::Next,
            })
            .unwrap();
        let connector_path = node::Path::new(["echo"]).unwrap();
        let emitting_id = committed.envelope.meta().sys().id();
        let edge = String::from("shout.out>echo.in");
        let prepared = PreparedRequest::seal(b"salut\n".to_vec());
        let intent = Intent {
            call_ref: ids::call_ref(ids::delivery_ref(&edge, emitting_id), prepared.digest()),
            emitting_id,
            edge,
            connector_path,
            prepared,
            idempotency_key: None,
            effect: node::EffectClass::Idempotent,
            instance_settings: alloc::collections::BTreeMap::new(),
        };
        journal.open_outbound(intent.clone()).unwrap();
        (journal, intent)
    }

    #[test]
    fn the_record_exists_before_the_driver_hears() {
        let (mut journal, intent) = seeded_journal();
        let mut driver = Scripted::default();
        let mut dispatcher = Dispatcher::new();
        dispatcher
            .pump(&mut journal, &mut driver, &OpenGate)
            .unwrap();
        assert_eq!(driver.started.len(), 1);
        assert_eq!(driver.started[0].0.call_ref, intent.call_ref);
        // A second pump never re-starts an already-started call.
        dispatcher
            .pump(&mut journal, &mut driver, &OpenGate)
            .unwrap();
        assert_eq!(driver.started.len(), 1);
    }

    #[test]
    fn a_refused_start_is_not_remembered_as_started() {
        let (mut journal, _) = seeded_journal();
        let mut bindings = StartRefusal {
            refuse: true,
            attempts: 0,
        };
        let mut dispatcher = Dispatcher::new();

        assert!(matches!(
            dispatcher.pump(&mut journal, &mut bindings, &OpenGate),
            Err(DispatchError::Driver(DriverRouteError::UnboundOutbound(_)))
        ));
        assert_eq!(bindings.attempts, 1);
        bindings.refuse = false;
        dispatcher
            .pump(&mut journal, &mut bindings, &OpenGate)
            .unwrap();
        assert_eq!(bindings.attempts, 2, "the open call was retried");
    }

    #[test]
    fn a_resumed_dispatcher_re_calls_nothing() {
        let (mut journal, _) = seeded_journal();
        let mut driver = Scripted::default();
        // Restart over an existing journal: the open call counts as
        // started; re-calls are the recovery fold's decision.
        let mut dispatcher = Dispatcher::resume(&journal);
        dispatcher
            .pump(&mut journal, &mut driver, &OpenGate)
            .unwrap();
        assert!(driver.started.is_empty());
    }

    #[test]
    fn won_closures_become_routable_events() {
        let (mut journal, intent) = seeded_journal();
        let mut driver = Scripted::default();
        let mut dispatcher = Dispatcher::new();
        dispatcher
            .pump(&mut journal, &mut driver, &OpenGate)
            .unwrap();
        driver.replies = vec![
            Response {
                call_ref: intent.call_ref,
                completion: Completion::process(
                    vec![Data::encoded(Encoding::Raw, *b"SALUT")],
                    vec![],
                    ProcessExit::Exited {
                        exit_code: 0,
                        signal: None,
                    },
                ),
            },
            // A duplicate answer for the same call: the race loser.
            Response {
                call_ref: intent.call_ref,
                completion: Completion::process(
                    vec![Data::encoded(Encoding::Raw, *b"LATE")],
                    vec![],
                    ProcessExit::Exited {
                        exit_code: 0,
                        signal: None,
                    },
                ),
            },
        ];
        let batch = dispatcher
            .pump(&mut journal, &mut driver, &OpenGate)
            .unwrap();
        assert_eq!(batch.events.len(), 1, "only the won closure feeds the VM");
        assert_eq!(batch.discarded.len(), 1, "the loser stays visible");
        assert!(batch.uncorrelated.is_empty());
        let event = &batch.events[0];
        assert_eq!(event.connector_path.to_string(), "echo");
        assert_eq!(event.emitting_id, intent.emitting_id);
        assert_eq!(event.completion.outputs()[0].port.name(), "out");
        assert_eq!(
            event
                .completion
                .outputs()
                .last()
                .map(|output| output.port.name()),
            Some("exit")
        );
        // The dispatcher minted no envelope: the journal's closure
        // fact carries the raw completion, and the connector's own
        // transaction will build the envelopes.
        assert!(journal.open_calls().is_empty());
    }

    #[test]
    fn a_completion_keeps_stderr_separate_from_its_exit() {
        let (mut journal, intent) = seeded_journal();
        let mut driver = Scripted::default();
        let mut dispatcher = Dispatcher::new();
        dispatcher
            .pump(&mut journal, &mut driver, &OpenGate)
            .unwrap();
        driver.replies = vec![Response {
            call_ref: intent.call_ref,
            completion: Completion::process(
                vec![],
                vec![Data::encoded(Encoding::Raw, *b"diagnostic")],
                ProcessExit::Exited {
                    exit_code: 1,
                    signal: None,
                },
            ),
        }];
        let batch = dispatcher
            .pump(&mut journal, &mut driver, &OpenGate)
            .unwrap();
        assert_eq!(batch.events.len(), 1);
        let outputs = batch.events[0].completion.outputs();
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].port.name(), "err");
        assert_eq!(outputs[1].port.name(), "exit");
    }

    #[test]
    fn an_uncorrelated_answer_is_a_visible_fault() {
        let (mut journal, _) = seeded_journal();
        let mut driver = Scripted::default();
        let mut dispatcher = Dispatcher::new();
        driver.replies = vec![Response {
            call_ref: ids::call_ref(
                ids::delivery_ref("x.out>ghost.in", envelope::Id::from_bytes([9; 16])),
                &[0; 32],
            ),
            completion: Completion::process_exit(ProcessExit::Exited {
                exit_code: 0,
                signal: None,
            }),
        }];
        let batch = dispatcher
            .pump(&mut journal, &mut driver, &OpenGate)
            .unwrap();
        assert!(batch.events.is_empty());
        assert_eq!(batch.uncorrelated.len(), 1, "correlation faults surface");
    }

    #[test]
    fn completion_construction_seals_the_terminal_barrier() {
        let out = PortOutput {
            port: node::Port::conventional_out(),
            data: Data::materialized(crate::value::Value::Int(1)),
        };
        let exit = PortOutput {
            port: node::Port::conventional_exit(),
            data: Data::materialized(ProcessExit::TimeoutExpired.into_value()),
        };
        assert_eq!(
            Completion::try_new(vec![out.clone()]),
            Err(CompletionError::MissingExit)
        );
        assert_eq!(
            Completion::try_new(vec![exit.clone(), out]),
            Err(CompletionError::OutputAfterExit)
        );
        assert_eq!(
            Completion::try_new(vec![exit.clone(), exit]),
            Err(CompletionError::MultipleExits(2))
        );
    }

    #[test]
    fn every_process_exit_projection_inhabits_the_one_union() {
        // Populated optional fields pin the Null-first branch convention used
        // by `into_value`; changing canonical union order must break here.
        let exits = [
            ProcessExit::Exited {
                exit_code: -1,
                signal: Some(9),
            },
            ProcessExit::SpawnFailed {
                message: String::from("no executable"),
            },
            ProcessExit::TimeoutExpired,
            ProcessExit::DecodeFailed {
                message: String::from("bad line"),
                line: Some(String::from("{")),
            },
            ProcessExit::CaptureFailed,
            ProcessExit::OutputOverflow {
                limit: 42,
                reason: OutputOverflowReason::Budget,
            },
            ProcessExit::OutputOverflow {
                limit: 42,
                reason: OutputOverflowReason::Capacity,
            },
        ];
        let ty = process_exit_type().expect("the standard ProcessExit type is valid");
        for exit in exits {
            let value = exit.into_value();
            assert!(ty.accepts(&value), "{value:?} must inhabit ProcessExit");
        }
    }
}
