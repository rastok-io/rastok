//! Contracts with the external world.
//!
//! Governing rule: [`delivery-semantics.md`]. In run, the world plays
//! itself; in simulation, the lab plays the world — both through the
//! same traits, and the journal keeps its world-half either way. The
//! [`DriverPoller`] is the loop between the journal and the outbound drivers:
//! it drives open outbound calls and settles their closures; only a
//! won closure ever becomes a VM input.
//!
//! [`delivery-semantics.md`]: https://github.com/rastok-io/rastok/blob/main/docs/spec/delivery-semantics.md

use crate::datatype::{
    DataType, EnumType, StructType, TaggedCase, TaggedUnionType, TypeError, TypeField,
};
use crate::envelope::{self, Envelope};
use crate::instance;

use crate::journal::{CloseVerdict, Closure, Journal};
use crate::node;

use alloc::boxed::Box;
use alloc::collections::{BTreeSet, VecDeque};
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
pub struct IngressEnvelopeProvenance {
    connector_path: node::Path,
    replica: u64,
}

impl IngressEnvelopeProvenance {
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
pub struct CallId(envelope::Id);

impl CallId {
    /// Wraps a derived identity; the recipe lives with the journal's
    /// `ids` module.
    #[must_use]
    pub const fn new(id: envelope::Id) -> Self {
        Self(id)
    }

    /// The derived identity bytes used as an input to other causal recipes.
    #[must_use]
    pub const fn id(self) -> envelope::Id {
        self.0
    }
}

impl fmt::Display for CallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Placement-neutral identity of one admitted inbound Call.
///
/// Unlike [`CallId`], this is not a digest of an outbound request intent. An
/// inbound Call is born as a root envelope, so its root id is already the
/// unique causal reference. The newtype prevents the two directions from
/// being crossed accidentally while keeping the recipe free of a redundant
/// hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InboundCallId(envelope::Id);

impl InboundCallId {
    /// Names the Call opened by one committed root envelope.
    #[must_use]
    pub const fn from_root(root: envelope::Id) -> Self {
        Self(root)
    }

    /// The root identity carried by canonical causal encodings.
    #[must_use]
    pub const fn id(self) -> envelope::Id {
        self.0
    }
}

impl fmt::Display for InboundCallId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Placement-neutral identity of one committed Connector session.
///
/// The value is derived from the opening fact by the journal. It never wraps
/// a socket, address, negotiated protocol, Host, Worker, or library handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(envelope::Id);

impl SessionId {
    /// Wraps a journal-derived identity; the recipe lives in [`crate::journal::ids`].
    #[must_use]
    pub const fn new(id: envelope::Id) -> Self {
        Self(id)
    }

    /// The raw derived identity used by canonical causal encodings.
    #[must_use]
    pub const fn id(self) -> envelope::Id {
        self.0
    }

    /// Parses the sole lowercase hexadecimal `meta.ext.session_id` spelling.
    ///
    /// Causal records still carry the raw id. This parser exists only at the
    /// graph-to-Driver correlation boundary and rejects every non-canonical
    /// spelling instead of normalizing it.
    #[must_use]
    pub fn parse_projection(value: &str) -> Option<Self> {
        if value.len() != 32
            || value
                .bytes()
                .any(|byte| !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase())
        {
            return None;
        }
        let mut bytes = [0u8; 16];
        for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
            let high = (pair[0] as char).to_digit(16)? as u8;
            let low = (pair[1] as char).to_digit(16)? as u8;
            bytes[index] = (high << 4) | low;
        }
        Some(Self(envelope::Id::from_bytes(bytes)))
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Identity of one journaled graph-to-world write inside a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionWriteId(envelope::Id);

impl SessionWriteId {
    /// Wraps a journal-derived write identity.
    #[must_use]
    pub const fn new(id: envelope::Id) -> Self {
        Self(id)
    }

    /// The raw derived identity used by canonical causal encodings.
    #[must_use]
    pub const fn id(self) -> envelope::Id {
        self.0
    }
}

impl fmt::Display for SessionWriteId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// The exact logical protocol or operation descriptor of a request, sealed.
/// What the journal digests is what the Driver decodes and executes without
/// recomputing settings. The sole M1 exception is an attested `SecretName`:
/// its physical material overlays the descriptor inside the authorized Driver
/// and is fixed independently by the immutable run header.
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

    /// The exact logical bytes to decode and execute. An authorized Driver may
    /// overlay attested secret material without placing it in this carrier.
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
    contract: &crate::connector::ContractId,
    port: &str,
    codec_word: Option<&str>,
    err_codec_word: Option<&str>,
) -> Option<DataType> {
    if !matches!(
        contract.as_str(),
        crate::connector::PROCESS_CONNECTOR_ID | crate::connector::PROCESS_SERVICE_CONNECTOR_ID
    ) {
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
    contract: &crate::connector::ContractId,
    attribute: &str,
) -> Option<crate::plan::EvaluationSite> {
    use crate::plan::EvaluationSite;

    if !matches!(
        contract.as_str(),
        crate::connector::PROCESS_CONNECTOR_ID | crate::connector::PROCESS_SERVICE_CONNECTOR_ID
    ) {
        return None;
    }
    match attribute {
        "idempotent" if contract.as_str() == crate::connector::PROCESS_CONNECTOR_ID => {
            Some(EvaluationSite::Plan)
        }
        "mode" | "codec" | "err_codec" | "behavior" | "max_output" | "max_frame" | "on_timeout" => {
            Some(EvaluationSite::Plan)
        }
        "args" | "command" | "cwd" | "timeout" | "env" | "restart" => {
            Some(EvaluationSite::Instance)
        }
        "correlate" => Some(EvaluationSite::Invocation),
        _ => None,
    }
}

/// Effective logical type of one currently executable `File` port.
///
/// This is the Registry-free mirror of the compiler sheet. `watch` is a
/// known source profile but has no executable row until its inbound protocol
/// is specified.
#[must_use]
pub fn file_port_type(
    contract: &crate::connector::ContractId,
    port: &str,
    mode_word: Option<&str>,
) -> Option<DataType> {
    if contract.as_str() != crate::connector::FILE_CONNECTOR_ID {
        return None;
    }
    match (port, mode_word) {
        ("in", Some("read")) => Some(DataType::Variant),
        ("in", Some("replace")) => Some(DataType::BYTES),
        ("out", Some("read" | "replace")) => Some(DataType::BYTES),
        ("exit", Some("read" | "replace")) => file_exit_type().ok(),
        _ => None,
    }
}

/// Latest evaluation site admitted by the executable `File` contract.
#[must_use]
pub fn file_attribute_site(
    contract: &crate::connector::ContractId,
    attribute: &str,
) -> Option<crate::plan::EvaluationSite> {
    use crate::plan::EvaluationSite;

    if contract.as_str() != crate::connector::FILE_CONNECTOR_ID {
        return None;
    }
    match attribute {
        "mode" | "idempotent" | "behavior" | "max_read" => Some(EvaluationSite::Plan),
        "timeout" => Some(EvaluationSite::Instance),
        "path" => Some(EvaluationSite::Invocation),
        _ => None,
    }
}

fn network_body_type(codec: Option<&str>) -> Option<DataType> {
    match codec {
        Some("json") => Some(DataType::Variant),
        Some("text") => Some(DataType::Text),
        Some("bytes") => Some(DataType::BYTES),
        _ => None,
    }
}

/// Registry-free effective port type for every M1.6 network sheet.
#[must_use]
pub fn network_port_type(
    contract: &crate::connector::ContractId,
    port: &str,
    codec: Option<&str>,
) -> Option<DataType> {
    let body = || network_body_type(codec);
    match (contract.as_str(), port) {
        (crate::connector::HTTP_CLIENT_CONNECTOR_ID, "in" | "out")
        | (crate::connector::HTTP_SERVER_CONNECTOR_ID, "in" | "out")
        | (crate::connector::WEBSOCKET_CLIENT_CONNECTOR_ID, "in" | "out")
        | (crate::connector::WEBSOCKET_SERVER_CONNECTOR_ID, "in" | "out") => body(),
        (crate::connector::HTTP_CLIENT_CONNECTOR_ID, "exit") => http_client_exit_type().ok(),
        (crate::connector::HTTP_SERVER_CONNECTOR_ID, "exit") => http_server_exit_type().ok(),
        (crate::connector::WEBSOCKET_CLIENT_CONNECTOR_ID, "exit")
        | (crate::connector::WEBSOCKET_SERVER_CONNECTOR_ID, "exit") => websocket_exit_type().ok(),
        _ => None,
    }
}

/// Registry-free evaluation-site mirror for the four M1.6 network sheets.
#[must_use]
pub fn network_attribute_site(
    contract: &crate::connector::ContractId,
    attribute: &str,
) -> Option<crate::plan::EvaluationSite> {
    use crate::plan::EvaluationSite::{Instance, Invocation, Plan};

    match contract.as_str() {
        crate::connector::HTTP_CLIENT_CONNECTOR_ID => match attribute {
            "behavior" | "codec" | "idempotent" | "json_profile" | "max_body" | "versions"
            | "zero_rtt" => Some(Plan),
            "timeout" | "tls_ca" | "tls_certificate" | "tls_private_key" => Some(Instance),
            "auth_bearer" | "custom_headers" | "headers" | "idempotency_key" | "method" | "url" => {
                Some(Invocation)
            }
            _ => None,
        },
        crate::connector::HTTP_SERVER_CONNECTOR_ID => match attribute {
            "behavior" | "codec" | "max_body" | "mode" | "path" | "versions" | "zero_rtt" => {
                Some(Plan)
            }
            "auth_bearer" | "custom_headers" | "headers" | "listen" | "timeout"
            | "tls_certificate" | "tls_private_key" => Some(Instance),
            "correlate" => Some(Invocation),
            _ => None,
        },
        crate::connector::WEBSOCKET_CLIENT_CONNECTOR_ID => match attribute {
            "behavior" | "codec" | "max_message" | "versions" | "zero_rtt" => Some(Plan),
            "auth_bearer" | "custom_headers" | "handshake_timeout" | "headers" | "subprotocols"
            | "tls_ca" | "tls_certificate" | "tls_private_key" | "url" => Some(Instance),
            _ => None,
        },
        crate::connector::WEBSOCKET_SERVER_CONNECTOR_ID => match attribute {
            "behavior" | "codec" | "max_message" | "path" | "versions" | "zero_rtt" => Some(Plan),
            "allowed_origins" | "auth_bearer" | "custom_headers" | "handshake_timeout"
            | "headers" | "listen" | "subprotocols" | "tls_certificate" | "tls_private_key" => {
                Some(Instance)
            }
            "correlate" => Some(Invocation),
            _ => None,
        },
        _ => None,
    }
}

/// Registry-free evaluation-site mirror for every built-in Connector.
#[must_use]
pub fn connector_attribute_site(
    contract: &crate::connector::ContractId,
    attribute: &str,
) -> Option<crate::plan::EvaluationSite> {
    process_attribute_site(contract, attribute)
        .or_else(|| file_attribute_site(contract, attribute))
        .or_else(|| network_attribute_site(contract, attribute))
}

/// Which deployment reference forms one Connector setting explicitly admits.
/// The table is Registry-free so a decoded package cannot turn a later-phase
/// expression into an undeclared boot capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootReferencePolicy {
    /// A typed non-secret value may come from `env("NAME")`.
    Environment,
    /// Material may come only from `secret("logical/path")`.
    Secret,
}

impl BootReferencePolicy {
    /// Whether this declared policy admits one exact reference kind.
    #[must_use]
    pub const fn admits(self, reference: &crate::deployment::BootReference) -> bool {
        matches!(
            (self, reference),
            (
                Self::Environment,
                crate::deployment::BootReference::Environment(_)
            ) | (Self::Secret, crate::deployment::BootReference::Secret(_))
        )
    }
}

/// Registry-free boot-reference policy for every currently executable
/// Connector setting. Absence means that `env()` and `secret()` both refuse.
#[must_use]
pub fn connector_boot_reference_policy(
    contract: &crate::connector::ContractId,
    attribute: &str,
) -> Option<BootReferencePolicy> {
    use BootReferencePolicy::{Environment, Secret};

    match contract.as_str() {
        crate::connector::HTTP_CLIENT_CONNECTOR_ID => match attribute {
            // A4 intentionally opens the smallest executable surface. Each
            // later setting joins this table only with its real and scripted
            // Driver consumers in the same gate.
            "auth_bearer" => Some(Secret),
            "url" => Some(Environment),
            _ => None,
        },
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

/// The portable terminal union of one `File` invocation.
///
/// Host error numbers, paths and platform messages deliberately do not enter
/// this causal type. The operation and requested path are already sealed by
/// the outbound intent; the cases describe only portable outcomes.
///
/// # Errors
///
/// Returns the canonical type constructor's precise invariant violation.
pub fn file_exit_type() -> Result<DataType, TypeError> {
    let empty = || StructType::new(Vec::new());
    let completed = TaggedCase::new("Completed", empty()?)?;
    let not_found = TaggedCase::new("NotFound", empty()?)?;
    let not_a_file = TaggedCase::new("NotAFile", empty()?)?;
    let permission_denied = TaggedCase::new("PermissionDenied", empty()?)?;
    let invalid_path = TaggedCase::new("InvalidPath", empty()?)?;
    let read_limit_exceeded = TaggedCase::new(
        "ReadLimitExceeded",
        StructType::new(alloc::vec![TypeField::new("limit", DataType::UINT)])?,
    )?;
    let atomic_replace_unavailable = TaggedCase::new("AtomicReplaceUnavailable", empty()?)?;
    let io_failed = TaggedCase::new("IoFailed", empty()?)?;
    let timeout_expired = TaggedCase::new("TimeoutExpired", empty()?)?;
    let outcome_unknown = TaggedCase::new("OutcomeUnknown", empty()?)?;
    Ok(DataType::TaggedUnion(TaggedUnionType::new(alloc::vec![
        completed,
        not_found,
        not_a_file,
        permission_denied,
        invalid_path,
        read_limit_exceeded,
        atomic_replace_unavailable,
        io_failed,
        timeout_expired,
        outcome_unknown,
    ])?))
}

fn protocol_code_type() -> DataType {
    // The source expression language's ordinary integer is the portable
    // carrier. Drivers prove the protocol-specific u16 range at ingress.
    DataType::INT
}

fn empty_case(name: &str) -> Result<TaggedCase, TypeError> {
    TaggedCase::new(name, StructType::new(Vec::new())?)
}

/// Portable terminal union of one outbound HTTP call.
pub fn http_client_exit_type() -> Result<DataType, TypeError> {
    let completed = TaggedCase::new(
        "Completed",
        StructType::new(alloc::vec![TypeField::new("status", protocol_code_type())])?,
    )?;
    let body_too_large = TaggedCase::new(
        "BodyTooLarge",
        StructType::new(alloc::vec![TypeField::new("limit", DataType::UINT)])?,
    )?;
    Ok(DataType::TaggedUnion(TaggedUnionType::new(alloc::vec![
        completed,
        TaggedCase::new(
            "ServerError",
            StructType::new(alloc::vec![TypeField::new("status", protocol_code_type())])?
        )?,
        empty_case("DnsFailed")?,
        empty_case("ConnectFailed")?,
        empty_case("TlsFailed")?,
        empty_case("ProtocolFailed")?,
        empty_case("TimeoutExpired")?,
        body_too_large,
        empty_case("DecodeFailed")?,
        empty_case("OutcomeUnknown")?,
    ])?))
}

/// Portable terminal union of one admitted HTTP server request.
pub fn http_server_exit_type() -> Result<DataType, TypeError> {
    let completed = TaggedCase::new(
        "Completed",
        StructType::new(alloc::vec![TypeField::new("status", protocol_code_type())])?,
    )?;
    Ok(DataType::TaggedUnion(TaggedUnionType::new(alloc::vec![
        completed,
        empty_case("RequestExpired")?,
        empty_case("Overloaded")?,
        empty_case("PeerDisconnected")?,
        empty_case("OutcomeUnknown")?,
    ])?))
}

/// Portable terminal union shared by WebSocket client and server sessions.
pub fn websocket_exit_type() -> Result<DataType, TypeError> {
    let closed = TaggedCase::new(
        "Closed",
        StructType::new(alloc::vec![
            TypeField::new("code", protocol_code_type()),
            TypeField::new("reason", DataType::Text),
        ])?,
    )?;
    let handshake_rejected = TaggedCase::new(
        "HandshakeRejected",
        StructType::new(alloc::vec![TypeField::new("status", protocol_code_type())])?,
    )?;
    let message_too_large = TaggedCase::new(
        "MessageTooLarge",
        StructType::new(alloc::vec![TypeField::new("limit", DataType::UINT)])?,
    )?;
    Ok(DataType::TaggedUnion(TaggedUnionType::new(alloc::vec![
        closed,
        empty_case("DnsFailed")?,
        empty_case("ConnectFailed")?,
        empty_case("TlsFailed")?,
        handshake_rejected,
        empty_case("ProtocolFailed")?,
        empty_case("TransportLost")?,
        empty_case("TimeoutExpired")?,
        message_too_large,
        empty_case("DecodeFailed")?,
        empty_case("OutcomeUnknown")?,
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

/// The portable terminal facts emitted by `File`.
///
/// This Rust enum is a Driver-side construction aid, not a second logical
/// type. [`FileExit::into_value`] is the sole projection into the ordinary
/// canonical tagged union carried by the `exit` port. Host error numbers,
/// paths, and platform messages never cross this boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileExit {
    /// The requested read or atomic replacement completed.
    Completed,
    /// No filesystem entry exists at the logical path.
    NotFound,
    /// The entry or one of its required path components is not a file.
    NotAFile,
    /// The host denied the operation.
    PermissionDenied,
    /// The logical path has no portable File spelling.
    InvalidPath,
    /// A complete read would exceed the declared bound.
    ReadLimitExceeded {
        /// Declared read bound in bytes.
        limit: u64,
    },
    /// The host cannot provide the contract's atomic replacement primitive.
    AtomicReplaceUnavailable,
    /// A portable classification above cannot describe the I/O failure.
    IoFailed,
    /// The scheduler's invocation budget won the close race.
    TimeoutExpired,
    /// Cancellation was requested, but no terminal observation arrived
    /// within the instance's explicit settlement budget.
    OutcomeUnknown,
}

/// Portable terminal facts of one outbound HTTP call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpClientExit {
    /// A usable response outside 500–599. Non-2xx is not a transport failure.
    Completed {
        /// Three-digit HTTP status code.
        status: u16,
    },
    /// A usable decoded HTTP 500–599 response, after external visibility.
    ServerError {
        /// The observed HTTP status, retained unchanged.
        status: u16,
    },
    /// Name resolution failed before request bytes became visible.
    DnsFailed,
    /// The transport connection could not be established.
    ConnectFailed,
    /// Verified TLS negotiation failed.
    TlsFailed,
    /// HTTP framing or negotiation failed.
    ProtocolFailed,
    /// The scheduler's call budget won the close race.
    TimeoutExpired,
    /// The declared request- or response-body bound was exceeded.
    BodyTooLarge {
        /// Declared body limit.
        limit: u64,
    },
    /// A complete body did not inhabit the selected codec.
    DecodeFailed,
    /// Cancellation settled without a portable terminal observation.
    OutcomeUnknown,
}

/// Portable terminal facts of one admitted HTTP server request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpServerExit {
    /// The protocol response became terminal.
    Completed {
        /// Status returned to the remote caller.
        status: u16,
    },
    /// A held respond-mode request exceeded its budget.
    RequestExpired,
    /// A committed response could not enter the bounded protocol writer.
    Overloaded,
    /// The peer disappeared before the response became terminal.
    PeerDisconnected,
    /// Recovery cannot prove which terminal observation the peer saw.
    OutcomeUnknown,
}

/// Portable terminal facts shared by WebSocket client and server sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebSocketExit {
    /// The close handshake supplied a portable code and UTF-8 reason.
    Closed {
        /// RFC 6455 close code.
        code: u16,
        /// Peer-provided close reason, possibly empty.
        reason: String,
    },
    /// Name resolution failed while opening a client session.
    DnsFailed,
    /// The transport connection could not be established.
    ConnectFailed,
    /// Verified TLS negotiation failed.
    TlsFailed,
    /// The HTTP handshake refused the session.
    HandshakeRejected {
        /// HTTP refusal status.
        status: u16,
    },
    /// WebSocket framing or negotiation failed.
    ProtocolFailed,
    /// An established transport disappeared without a clean close.
    TransportLost,
    /// The session/opening budget won its close race.
    TimeoutExpired,
    /// One complete message exceeded the declared bound.
    MessageTooLarge {
        /// Declared complete-message limit.
        limit: u64,
    },
    /// A complete message did not inhabit the selected codec.
    DecodeFailed,
    /// Recovery cannot prove which terminal observation the peer saw.
    OutcomeUnknown,
}

fn tagged_value(case: &str, payload: Vec<(String, crate::value::Value)>) -> crate::value::Value {
    crate::value::Value::TaggedUnion(alloc::boxed::Box::new(crate::value::TaggedUnionValue {
        case: String::from(case),
        payload,
    }))
}

impl HttpClientExit {
    /// Classifies an already admitted and decoded response. Decode failure
    /// must be selected before this function, even for a server-error status.
    #[must_use]
    pub const fn response(status: u16) -> Self {
        if status >= 500 && status <= 599 {
            Self::ServerError { status }
        } else {
            Self::Completed { status }
        }
    }

    /// Projects the Driver fact into the ordinary canonical tagged value.
    #[must_use]
    pub fn into_value(self) -> crate::value::Value {
        use crate::value::Value;
        match self {
            Self::Completed { status } => tagged_value(
                "Completed",
                alloc::vec![(String::from("status"), Value::Int(i64::from(status)))],
            ),
            Self::ServerError { status } => tagged_value(
                "ServerError",
                alloc::vec![(String::from("status"), Value::Int(i64::from(status)))],
            ),
            Self::DnsFailed => tagged_value("DnsFailed", Vec::new()),
            Self::ConnectFailed => tagged_value("ConnectFailed", Vec::new()),
            Self::TlsFailed => tagged_value("TlsFailed", Vec::new()),
            Self::ProtocolFailed => tagged_value("ProtocolFailed", Vec::new()),
            Self::TimeoutExpired => tagged_value("TimeoutExpired", Vec::new()),
            Self::BodyTooLarge { limit } => tagged_value(
                "BodyTooLarge",
                alloc::vec![(String::from("limit"), Value::Uint(limit))],
            ),
            Self::DecodeFailed => tagged_value("DecodeFailed", Vec::new()),
            Self::OutcomeUnknown => tagged_value("OutcomeUnknown", Vec::new()),
        }
    }
}

impl HttpServerExit {
    /// Projects the Driver fact into the ordinary canonical tagged value.
    #[must_use]
    pub fn into_value(self) -> crate::value::Value {
        use crate::value::Value;
        match self {
            Self::Completed { status } => tagged_value(
                "Completed",
                alloc::vec![(String::from("status"), Value::Int(i64::from(status)))],
            ),
            Self::RequestExpired => tagged_value("RequestExpired", Vec::new()),
            Self::Overloaded => tagged_value("Overloaded", Vec::new()),
            Self::PeerDisconnected => tagged_value("PeerDisconnected", Vec::new()),
            Self::OutcomeUnknown => tagged_value("OutcomeUnknown", Vec::new()),
        }
    }
}

impl WebSocketExit {
    /// Projects the Driver fact into the ordinary canonical tagged value.
    #[must_use]
    pub fn into_value(self) -> crate::value::Value {
        use crate::value::Value;
        match self {
            Self::Closed { code, reason } => tagged_value(
                "Closed",
                alloc::vec![
                    (String::from("code"), Value::Int(i64::from(code))),
                    (String::from("reason"), Value::Text(reason)),
                ],
            ),
            Self::DnsFailed => tagged_value("DnsFailed", Vec::new()),
            Self::ConnectFailed => tagged_value("ConnectFailed", Vec::new()),
            Self::TlsFailed => tagged_value("TlsFailed", Vec::new()),
            Self::HandshakeRejected { status } => tagged_value(
                "HandshakeRejected",
                alloc::vec![(String::from("status"), Value::Int(i64::from(status)))],
            ),
            Self::ProtocolFailed => tagged_value("ProtocolFailed", Vec::new()),
            Self::TransportLost => tagged_value("TransportLost", Vec::new()),
            Self::TimeoutExpired => tagged_value("TimeoutExpired", Vec::new()),
            Self::MessageTooLarge { limit } => tagged_value(
                "MessageTooLarge",
                alloc::vec![(String::from("limit"), Value::Uint(limit))],
            ),
            Self::DecodeFailed => tagged_value("DecodeFailed", Vec::new()),
            Self::OutcomeUnknown => tagged_value("OutcomeUnknown", Vec::new()),
        }
    }
}

impl FileExit {
    /// Projects the Driver fact into the ordinary canonical tagged value.
    #[must_use]
    pub fn into_value(self) -> crate::value::Value {
        use crate::value::{TaggedUnionValue, Value};

        let (case, payload) = match self {
            Self::Completed => ("Completed", Vec::new()),
            Self::NotFound => ("NotFound", Vec::new()),
            Self::NotAFile => ("NotAFile", Vec::new()),
            Self::PermissionDenied => ("PermissionDenied", Vec::new()),
            Self::InvalidPath => ("InvalidPath", Vec::new()),
            Self::ReadLimitExceeded { limit } => (
                "ReadLimitExceeded",
                alloc::vec![(String::from("limit"), Value::Uint(limit))],
            ),
            Self::AtomicReplaceUnavailable => ("AtomicReplaceUnavailable", Vec::new()),
            Self::IoFailed => ("IoFailed", Vec::new()),
            Self::TimeoutExpired => ("TimeoutExpired", Vec::new()),
            Self::OutcomeUnknown => ("OutcomeUnknown", Vec::new()),
        };
        Value::TaggedUnion(alloc::boxed::Box::new(TaggedUnionValue {
            case: String::from(case),
            payload,
        }))
    }
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

/// Frames one HTTP body or complete WebSocket message without transport
/// delimiters. HTTP versions, WebSocket fragmentation and frame headers are
/// physical Driver concerns; this function owns only the sheet's logical
/// `json | text | bytes` codec.
///
/// # Errors
///
/// The payload does not inhabit the selected logical codec.
pub fn network_payload_bytes(codec: &str, data: &envelope::Data) -> Result<Vec<u8>, String> {
    use envelope::{Data, Value};
    match (codec, data) {
        ("bytes", _) => Ok(bytes_payload(data)?.to_vec()),
        ("text", Data::Materialized(Value::Text(text))) => Ok(text.as_bytes().to_vec()),
        ("text", Data::Encoded { encoding, .. }) => Err(alloc::format!(
            "the text codec requires materialized text; the payload is {encoding:?}-encoded"
        )),
        ("text", Data::Materialized(other)) => Err(alloc::format!(
            "the text codec requires text; the payload is {:?}",
            crate::value::ValueKind::of(other)
        )),
        ("json", Data::Materialized(value)) => crate::json::write_value(value)
            .map(String::into_bytes)
            .map_err(|error| alloc::format!("the payload has no JSON form: {}", error.message)),
        ("json", Data::Encoded { encoding, .. }) => Err(alloc::format!(
            "the json codec frames the JSON model; the payload is {encoding:?}-encoded"
        )),
        (unknown, _) => Err(alloc::format!("unknown network body codec `{unknown}`")),
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

/// Projects a machine-decided terminal fact into the File contract.
///
/// This is selected by a bound File Driver; the generic scheduler never
/// dispatches on a concrete Connector name.
#[must_use]
pub fn file_scheduler_completion(completion: SchedulerCompletion) -> Completion {
    Completion::file_exit(match completion {
        SchedulerCompletion::TimeoutExpired => FileExit::TimeoutExpired,
        SchedulerCompletion::OutcomeUnknown => FileExit::OutcomeUnknown,
    })
}

/// Frames one opaque byte request without text or data-model conversion.
///
/// # Errors
///
/// The typed refusal when the payload is neither raw encoded bytes nor the
/// materialized bytes scalar.
pub fn prepare_bytes(data: &envelope::Data) -> Result<PreparedRequest, String> {
    Ok(PreparedRequest::seal(bytes_payload(data)?.to_vec()))
}

/// Borrows one opaque byte payload without framing or sealing it.
///
/// Connector-specific operation framers use this seam when the raw payload is
/// only one field of their final [`PreparedRequest`]. It avoids constructing
/// and hashing a discarded intermediate request.
///
/// # Errors
///
/// The typed refusal when the payload is neither raw encoded bytes nor the
/// materialized bytes scalar.
pub fn bytes_payload(data: &envelope::Data) -> Result<&[u8], String> {
    use envelope::{Data, Encoding, Value};
    match data {
        Data::Encoded {
            encoding: Encoding::Raw,
            bytes,
            ..
        } => Ok(bytes),
        Data::Materialized(Value::Bytes(bytes)) => Ok(bytes),
        Data::Encoded { encoding, .. } => Err(alloc::format!(
            "the bytes codec requires raw bytes; the request is {encoding:?}-encoded"
        )),
        Data::Materialized(other) => Err(alloc::format!(
            "the bytes codec requires bytes; the request data is {:?}",
            crate::value::ValueKind::of(other)
        )),
    }
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
        } => bytes.to_vec(),
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
                "the lines codec frames text; the request data is {:?}",
                crate::value::ValueKind::of(other)
            ));
        }
    };
    Ok(PreparedRequest::seal(bytes))
}

/// Description of an outbound Call opening. The canonical fields
/// (`identity.md` §3, keys 27–30 and 33) are journaled before the effect
/// executes; resolved settings remain execution material whose call-varying
/// semantics must already be covered by the prepared-request digest.
#[derive(Debug, Clone, PartialEq)]
pub struct CallOpen {
    /// The derived internal call identity.
    pub call_id: CallId,
    /// The envelope that opened the call — the cause.
    pub emitting_id: envelope::Id,
    /// The canonical edge of the causing delivery (key 33): with the
    /// emitting id it reconstructs the delivery reference and hence
    /// the call identity — the record stays autonomous.
    pub edge: String,
    /// The calling connector node.
    pub connector_path: node::Path,
    /// The prepared request — the sealed digest is canonical (key
    /// 30); the bytes ride along for the driver and are never
    /// themselves record material.
    pub prepared: PreparedRequest,
    /// Optional partner business-operation key, stable across attempts.
    /// Its presence is not an idempotence guarantee.
    pub idempotency_key: Option<String>,
    /// Effective non-plan Connector attributes resolved for this exact call.
    /// Instance rows are reused from the immutable instance bundle;
    /// invocation rows are evaluated against the admitted envelope before
    /// request preparation. Drivers consume them but never choose defaults or
    /// evaluate expressions.
    pub resolved_settings: alloc::collections::BTreeMap<String, crate::value::Value>,
    /// Logical instant fixed by the VM for the causing transaction. This is
    /// execution context for scripted Drivers, not an EGRESS identity field
    /// and never an invitation for a Driver to read the host clock.
    pub logical_time: i64,
}

impl CallOpen {
    /// The boundary integrity seal over the exact prepared bytes.
    #[must_use]
    pub fn request_digest(&self) -> &[u8; 32] {
        self.prepared.digest()
    }
}

/// Description of one outbound Session opening, journaled before the Driver acts.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionOpen {
    /// Session identity derived from the remaining canonical fields.
    pub session_id: SessionId,
    /// Envelope whose delivery requested the opening.
    pub emitting_id: envelope::Id,
    /// Connector node that will own the session.
    pub connector_path: node::Path,
    /// Exact handshake request bytes and their integrity seal.
    pub prepared: PreparedRequest,
}

impl SessionOpen {
    /// The boundary integrity seal over the exact opening bytes.
    #[must_use]
    pub fn request_digest(&self) -> &[u8; 32] {
        self.prepared.digest()
    }
}

/// Description of one graph-to-world message inside a committed Session.
///
/// Like [`CallOpen`], the exact prepared bytes are journaled by digest before a
/// Driver may act. Unlike a Call, settling this write does not publish a
/// graph-visible `exit`; only the enclosing session has a terminal exit.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionWrite {
    /// Derived identity of this write obligation.
    pub write_ref: SessionWriteId,
    /// Logical session being addressed.
    pub session_id: SessionId,
    /// Envelope whose emission requested the write.
    pub emitting_id: envelope::Id,
    /// Connector node owning the live session.
    pub connector_path: node::Path,
    /// Exact prepared message bytes and their integrity seal.
    pub prepared: PreparedRequest,
}

impl SessionWrite {
    /// The boundary integrity seal over the exact message bytes.
    #[must_use]
    pub fn message_digest(&self) -> &[u8; 32] {
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
    /// External observations owned by this exact boundary output.
    ///
    /// The VM validates and overlays these fields after ordinary request-zone
    /// inheritance. Drivers never mint `meta.sys` or write `meta.app`.
    pub ext: envelope::Namespace,
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
            ext: envelope::Namespace::new(),
        }));
        outputs.extend(err.into_iter().map(|data| PortOutput {
            port: node::Port::conventional_err(),
            data,
            ext: envelope::Namespace::new(),
        }));
        outputs.push(PortOutput {
            port: node::Port::conventional_exit(),
            data: envelope::Data::materialized(exit.into_value()),
            ext: envelope::Namespace::new(),
        });
        Self { outputs }
    }

    /// Builds a completion containing only its terminal Process fact.
    #[must_use]
    pub fn process_exit(exit: ProcessExit) -> Self {
        Self::process(Vec::new(), Vec::new(), exit)
    }

    /// Builds one File completion: an optional complete read followed by the
    /// sole terminal fact. Replace success deliberately supplies no `out`.
    #[must_use]
    pub fn file(out: Option<envelope::Data>, exit: FileExit) -> Self {
        let mut outputs = Vec::with_capacity(usize::from(out.is_some()).saturating_add(1));
        if let Some(data) = out {
            outputs.push(PortOutput {
                port: node::Port::conventional_out(),
                data,
                ext: envelope::Namespace::new(),
            });
        }
        outputs.push(PortOutput {
            port: node::Port::conventional_exit(),
            data: envelope::Data::materialized(exit.into_value()),
            ext: envelope::Namespace::new(),
        });
        Self { outputs }
    }

    /// Builds a File completion containing only its terminal fact.
    #[must_use]
    pub fn file_exit(exit: FileExit) -> Self {
        Self::file(None, exit)
    }

    fn terminal_value(value: crate::value::Value) -> Self {
        Self {
            outputs: alloc::vec![PortOutput {
                port: node::Port::conventional_exit(),
                data: envelope::Data::materialized(value),
                ext: envelope::Namespace::new(),
            }],
        }
    }

    /// Builds one usable HTTP response followed by its sole terminal fact.
    #[must_use]
    pub fn http_client(
        out: envelope::Data,
        ext: envelope::Namespace,
        exit: HttpClientExit,
    ) -> Self {
        Self {
            outputs: alloc::vec![
                PortOutput {
                    port: node::Port::conventional_out(),
                    data: out,
                    ext,
                },
                PortOutput {
                    port: node::Port::conventional_exit(),
                    data: envelope::Data::materialized(exit.into_value()),
                    ext: envelope::Namespace::new(),
                },
            ],
        }
    }

    /// Builds an HTTP-client completion containing only its terminal fact.
    #[must_use]
    pub fn http_client_exit(exit: HttpClientExit) -> Self {
        Self::terminal_value(exit.into_value())
    }

    /// Builds an HTTP-server completion containing only its terminal fact.
    #[must_use]
    pub fn http_server_exit(exit: HttpServerExit) -> Self {
        Self::terminal_value(exit.into_value())
    }

    /// Builds a WebSocket completion containing only its terminal fact.
    #[must_use]
    pub fn websocket_exit(exit: WebSocketExit) -> Self {
        Self::terminal_value(exit.into_value())
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
    pub call_id: CallId,
    /// Ordered ordinary outputs plus the terminal invocation fact.
    pub completion: Completion,
}

/// Result of framing one invocation before its EGRESS record opens.
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
    /// A Call exchange view proven at bind time disappeared later, or a
    /// Session-only Driver was routed through the Call machine.
    UnavailableCall(node::Path),
    /// No outbound Session binding owns this planned Connector path.
    UnboundSession(node::Path),
    /// A Session exchange view proven at bind time disappeared later.
    UnavailableSession(node::Path),
    /// An inbound role view proven at bind time disappeared later.
    UnavailableInbound(crate::connector::ContractId),
    /// An inbound Call view proven at bind time disappeared later.
    UnavailableInboundCall(crate::connector::ContractId),
    /// A caller attempted to decide a proposal other than the one retained
    /// by that Connector family.
    UnknownIngressProposal {
        /// Family whose retained proposal was addressed.
        connector: crate::connector::ContractId,
        /// Proposal reference supplied by the caller.
        reference: IngressProposalHandle,
    },
    /// The inbound proposal's session framing contradicts the bound
    /// contract's categorical exchange lifecycle.
    InboundExchangeMismatch {
        /// Driver family that proposed the fact.
        connector: crate::connector::ContractId,
        /// Exchange required by the validated plan.
        exchange: crate::connector::ConnectorExchange,
    },
    /// A family proposed a root from a Connector node it did not bind.
    InboundSourceMismatch {
        /// Driver family that made the proposal.
        connector: crate::connector::ContractId,
        /// Proposed Connector node.
        source: node::Path,
    },
    /// One Driver answered a call started through another Connector family.
    CrossFamilyResponse {
        /// Correlated call.
        call_id: CallId,
        /// Family that started it.
        expected: crate::connector::ContractId,
        /// Family that answered it.
        actual: crate::connector::ContractId,
    },
    /// One Driver reported a terminal fact for an inbound Call admitted by
    /// another Connector family.
    CrossFamilyInboundCall {
        /// Correlated inbound Call.
        call_id: InboundCallId,
        /// Family that admitted it.
        expected: crate::connector::ContractId,
        /// Family that reported the observation.
        actual: crate::connector::ContractId,
    },
    /// A Driver reported a terminal fact for no admitted inbound Call.
    UncorrelatedInboundCall(InboundCallId),
    /// One Driver reported an event for a session owned by another family.
    CrossFamilySession {
        /// Correlated logical session.
        session: SessionId,
        /// Family that admitted or opened it.
        expected: crate::connector::ContractId,
        /// Family that reported the observation.
        actual: crate::connector::ContractId,
    },
    /// A Driver reported an observation for no admitted/opened session.
    UncorrelatedSession(SessionId),
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
            Self::UnavailableCall(connector) => write!(
                formatter,
                "the Call exchange Driver bound for `{connector}` is no longer available"
            ),
            Self::UnboundSession(connector) => {
                write!(formatter, "no Session Driver binding owns `{connector}`")
            }
            Self::UnavailableSession(connector) => write!(
                formatter,
                "the Session exchange Driver bound for `{connector}` is no longer available"
            ),
            Self::UnavailableInbound(id) => write!(
                formatter,
                "the inbound Driver role bound for `{id}` is no longer available"
            ),
            Self::UnavailableInboundCall(id) => write!(
                formatter,
                "the inbound Call Driver bound for `{id}` is no longer available"
            ),
            Self::UnknownIngressProposal {
                connector,
                reference,
            } => write!(
                formatter,
                "Driver `{connector}` has no retained inbound proposal {}",
                reference.get()
            ),
            Self::InboundExchangeMismatch {
                connector,
                exchange,
            } => write!(
                formatter,
                "Driver `{connector}` proposed inbound framing outside its {exchange} exchange"
            ),
            Self::InboundSourceMismatch { connector, source } => write!(
                formatter,
                "Driver `{connector}` proposed an inbound root from unbound source `{source}`"
            ),
            Self::CrossFamilyResponse {
                call_id,
                expected,
                actual,
            } => write!(
                formatter,
                "Driver `{actual}` answered call {call_id} started by `{expected}`"
            ),
            Self::CrossFamilyInboundCall {
                call_id,
                expected,
                actual,
            } => write!(
                formatter,
                "Driver `{actual}` terminated inbound call {call_id} admitted by `{expected}`"
            ),
            Self::UncorrelatedInboundCall(call_id) => {
                write!(formatter, "no bound Driver owns inbound call {call_id}")
            }
            Self::CrossFamilySession {
                session,
                expected,
                actual,
            } => write!(
                formatter,
                "Driver `{actual}` reported session {session} owned by `{expected}`"
            ),
            Self::UncorrelatedSession(session) => {
                write!(formatter, "no bound Driver owns session {session}")
            }
        }
    }
}

impl core::error::Error for DriverRouteError {}

/// Judges a driver's completion against the calling connector's resolved
/// contract before it may become a causal fact. The dispatcher only asks.
pub trait CompletionCheck {
    /// `Ok` to admit; `Err(reason)` to refuse the closure.
    ///
    /// # Errors
    ///
    /// The refusal reason, surfaced with the rejected response.
    fn admit(&self, connector: &node::Path, completion: &Completion) -> Result<(), String>;
}

/// Accepts every completion — for worlds whose completions are
/// validated elsewhere, and for tests of the dispatch mechanics.
#[derive(Debug, Clone, Copy, Default)]
pub struct AcceptAnyCompletion;

impl CompletionCheck for AcceptAnyCompletion {
    fn admit(&self, _: &node::Path, _: &Completion) -> Result<(), String> {
        Ok(())
    }
}

/// A settled outbound Call completion awaiting its Connector transaction.
///
/// The Call closure is already journaled; this carrier adds no record or
/// envelope. The VM builds and commits the output emissions.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundaryObservation {
    /// The settled call.
    pub call_id: CallId,
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
pub struct IngressSubmission {
    /// Stable external origin shared by roots from this logical replica.
    pub provenance: IngressEnvelopeProvenance,
    /// Source port of the draft. Direct App injection uses `in` or another
    /// boundary input; a Driver proposal uses one output of the Connector
    /// named by [`Self::provenance`]. Journaled for exact replay.
    pub port: node::Port,
    /// The payload and birth metadata.
    pub draft: Draft,
    /// How the sequence is chosen.
    pub sequence: SubmissionSequence,
}

/// Session meaning of one inbound proposal before the accepting commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundSession {
    /// The accepting commit opens a new logical session and derives its ref.
    Open,
    /// One message belongs to an already committed live session.
    Message(SessionId),
}

/// Driver-local identity of one bounded inbound proposal.
///
/// This value is deliberately non-causal: the Driver retains the proposal
/// until the runtime decides it, while the accepting journal transaction
/// mints the envelope and any Call or Session reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IngressProposalHandle(u64);

impl IngressProposalHandle {
    /// Creates one Driver-local proposal reference.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The local numeric value, useful to a Driver's bounded queue.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// One bounded world fact offered without pre-minting an envelope.
#[derive(Debug, Clone, PartialEq)]
pub struct IngressProposal {
    /// Stable while the proposal remains pending in the Driver.
    pub reference: IngressProposalHandle,
    /// Draft and provenance to commit if admission succeeds. The provenance
    /// names the exact Connector node whose output introduces the root; the
    /// runtime verifies it against the bound family before committing.
    pub submission: IngressSubmission,
    /// Session framing when this fact opens or belongs to a session. `None`
    /// is an Event or Call ingress.
    pub session: Option<InboundSession>,
}

/// Positive result returned only after the accepting journal commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngressAcceptance {
    /// Id of the committed root envelope.
    pub envelope: envelope::Id,
    /// Exchange-specific reference created by the accepting fact. Event
    /// ingress has none; Call and Session are mutually exclusive.
    pub exchange: Option<InboundExchangeId>,
}

/// Exchange-specific identity returned by a committed inbound admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundExchangeId {
    /// The root opened one bounded Call obligation.
    Call(InboundCallId),
    /// The root opened or addressed one logical Session.
    Session(SessionId),
}

/// Portable contract-level reasons why a proposed fact was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngressRejection {
    /// The proposed graph boundary input does not exist.
    UnknownPort,
    /// The payload violates an ingress or destination type.
    InvalidPayload,
    /// The external metadata violates the declared vocabulary or type.
    InvalidMetadata,
    /// The named session is unknown or already terminal.
    SessionUnavailable,
}

/// A final decision delivered to the Driver for its retained proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngressDecision {
    /// The causal fact committed; peer-visible application acceptance may now
    /// be sent.
    Accepted(IngressAcceptance),
    /// The proposal was refused without minting an envelope.
    Rejected(IngressRejection),
}

/// Result of attempting one proposal against bounded runtime admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngressAdmission {
    /// The causal fact committed.
    Accepted(IngressAcceptance),
    /// No causal fact committed; the Driver must retain the proposal and
    /// withhold its application acknowledgement.
    Pending,
    /// No causal fact committed; the Driver receives a terminal refusal.
    Rejected(IngressRejection),
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

/// The work selected when a scheduler deadline is reached. Each variant
/// carries the exact delivery, run or instance to which the obligation belongs;
/// the scheduler does not infer a current generation when it fires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Obligation {
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
        instance: instance::Generation,
    },
    /// An admitted inbound Call reached the request deadline published by
    /// its Connector contract. The Driver owns the protocol-side timeout
    /// response; this event only transfers the scheduler's won request.
    InboundCallBudget {
        /// Stable reference of the admitted root Call.
        call_id: InboundCallId,
        /// Exact source generation retaining the request.
        instance: instance::Generation,
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
        instance: instance::Generation,
    },
    /// One committed Loop continuation reached its re-admission date. The
    /// journal fold remains authoritative for the state, ordinal and fuel;
    /// these fields only identify the exact obligation to re-check.
    LoopDelay {
        /// Owner carrying the Loop Extension.
        owner: node::OwnerSlot,
        /// Exact generation in which the continuation was selected.
        instance: instance::Generation,
        /// Durable Loop run whose fold must still be open.
        run: crate::journal::LoopRunId,
        /// Ordinal of the manager-produced state awaiting admission.
        iteration: u64,
        /// Re-entry envelope id, also used as deadline kind 4's stable id (RecurrenceDelay).
        state: envelope::Id,
    },
    /// Original closure-based eligibility, on the same dated scheduler as Loop.
    RetryDelay {
        /// Exact generation retaining the operation.
        instance: instance::Generation,
        /// Predecessor whose winning closure created this deadline.
        attempt: crate::journal::retry_protocol::AttemptId,
        /// Following admission reference (deadline kind 4, RecurrenceDelay).
        admission: envelope::Id,
    },
    /// One owner lifetime reached a scheduler-owned deadline. The complete
    /// instance reference is the generation fence; idle additionally compares
    /// the causal admission id, while fixed and tumbling never move after
    /// `l:start`.
    InstanceLifetime {
        /// Plan owner whose policy scheduled the event.
        owner: node::OwnerSlot,
        /// Exact generation whose deadline is being observed.
        instance: instance::Generation,
        /// Envelope whose arrival created this deadline and causes `l:stop`.
        cause: envelope::Id,
        /// Which profile determines the stale-event rule.
        deadline: InstanceDeadline,
    },
    /// One phase boundary of the explicit bounded Closing protocol.
    InstanceClosing {
        /// Root generation whose policy owns the whole closing subtree.
        instance: instance::Generation,
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
    /// `OutcomeUnknown` and their EGRESS records become `abandoned`.
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

/// Boot, crash, or restart — supervision observations entering the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleObservation {
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
    /// The next retained proposal, if one is waiting. Repeated polls before a
    /// decision must describe the same proposal reference and bytes.
    fn proposal(&mut self) -> Option<IngressProposal>;

    /// Releases one retained proposal after a committed acceptance or a
    /// contract rejection. Pending admission deliberately makes no callback.
    fn decide(&mut self, reference: IngressProposalHandle, decision: IngressDecision);

    /// Inbound Call completion view, when this bound contract is Call-shaped.
    /// Event and Session Drivers leave the default absent.
    fn call(&mut self) -> Option<&mut dyn InboundCallConnectorDriver> {
        None
    }
}

/// One terminal observation for an admitted inbound Call.
///
/// The Driver never commits this observation directly. The runtime correlates it,
/// validates the typed completion and commits the Connector transaction; a
/// duplicate or late observation cannot create a second `exit`.
#[derive(Debug, Clone, PartialEq)]
pub struct InboundCallObservation {
    /// Admitted root Call this observation terminates.
    pub call_id: InboundCallId,
    /// Ordinary ordered outputs with exactly one terminal `exit` last.
    pub completion: Completion,
}

/// One graph-produced response offered to the Driver that owns an admitted
/// inbound Call.
///
/// The response envelope already exists in a committed graph transaction;
/// this value is therefore a delivery capability, not a second causal
/// request or an outbound Call. The Driver may start the protocol write only
/// after receiving this value from the runtime.
#[derive(Debug, Clone, PartialEq)]
pub struct InboundCallResponse {
    /// Admitted root Call selected by lineage or the contract correlator.
    pub call_id: InboundCallId,
    /// Exact inbound Connector endpoint that owns the open request.
    pub connector_path: node::Path,
    /// Already-journaled response envelope delivered to that endpoint.
    pub response: envelope::Envelope,
}

/// One scheduler-owned expiry request for an admitted inbound Call.
///
/// The VM decides that the request budget elapsed; the Driver owns only the
/// protocol action (normally a 504 response) and reports which terminal fact
/// the peer-visible race actually produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundCallTimeout {
    /// Admitted root Call whose budget expired.
    pub call_id: InboundCallId,
    /// Exact inbound Connector endpoint retaining the protocol request.
    pub connector_path: node::Path,
}

/// Result of offering one already-journaled inbound-Call response to its
/// bounded Driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundCallResponseStart {
    /// The Driver owns the response attempt and must report one terminal
    /// observation through [`InboundCallConnectorDriver::events`].
    Started,
    /// No protocol bytes were written; finite local capacity kept the same
    /// response pending for a later offer.
    Pending,
}

/// Result of offering one scheduler-owned inbound-Call timeout to its
/// bounded Driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundCallTimeoutStart {
    /// The Driver owns the timeout attempt and will report the winning
    /// terminal observation through [`InboundCallConnectorDriver::events`].
    Started,
    /// No protocol bytes were written; finite local capacity kept the same
    /// timeout request pending for a later offer.
    Pending,
}

/// Terminal-observation half of an inbound Call Driver.
///
/// Admission itself remains on [`InboundConnectorDriver`]. Keeping this as a
/// categorical view makes an Event source unable to manufacture Call exits.
pub trait InboundCallConnectorDriver {
    /// Starts one graph response whose producing transaction is already
    /// durable. Ack-only Drivers return [`InboundCallResponseStart::Pending`]
    /// only on plans that should have been rejected before binding.
    fn start_response(&mut self, response: &InboundCallResponse) -> InboundCallResponseStart;

    /// Starts the protocol-side timeout action selected by the scheduler.
    /// A graph response already owned by the Driver may win this race; the
    /// sole later event states which outcome became terminal.
    fn start_timeout(&mut self, timeout: &InboundCallTimeout) -> InboundCallTimeoutStart;

    /// Terminal facts observed since the previous bounded drain.
    fn events(&mut self) -> Vec<InboundCallObservation>;
}

/// The outbound world role. Its exchange view is categorical: a bound object
/// exposes Call or Session mechanics, never a bag of optional operations.
pub trait OutboundConnectorDriver {
    /// Call protocol view, when this bound contract is Call-shaped.
    fn call(&mut self) -> Option<&mut dyn CallConnectorDriver>;

    /// Session protocol view, when this bound contract is Session-shaped.
    fn session(&mut self) -> Option<&mut dyn SessionConnectorDriver>;
}

/// Result of offering one already-journaled call to its bounded Driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallStart {
    /// The Driver owns the effect and must eventually answer or be cancelled.
    Started,
    /// No effect started; bounded local capacity kept the call pending.
    /// The dispatcher will offer the same journaled intent again.
    Pending,
}

/// Result of offering one journaled Session action to its bounded Driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStart {
    /// The Driver owns the opening or write and must report its observation.
    Started,
    /// No protocol bytes became visible; bounded capacity kept the action
    /// pending and the machine may offer the same intent again.
    Pending,
}

/// Starts external calls and supplies their answers. A Call driver
/// answers or stays silent — never a timeout and never an unknown
/// outcome: budgets belong to the scheduler, triage to the recovery
/// fold. Correlation uses the connector's own protocol.
pub trait CallConnectorDriver {
    /// Frames one invocation under this bound contract before the machine
    /// opens its EGRESS record. The Driver chooses no defaults: binding has
    /// already resolved every effective non-plan setting for `connector`.
    fn prepare_request(
        &mut self,
        connector: &node::Path,
        data: &envelope::Data,
        resolved_settings: &alloc::collections::BTreeMap<String, crate::value::Value>,
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
    fn cancel(&mut self, call_id: &CallId) {
        let _ = call_id;
    }

    /// Offers an already-journaled open call for execution. A bounded Driver
    /// may leave it pending without starting an effect; the same intent is
    /// then offered again. The request envelope rides along because the
    /// intent seals only the digest of the request, not its bytes.
    fn start(&mut self, intent: &CallOpen, request: &Envelope) -> CallStart;

    /// The answers that arrived since the last drain.
    fn events(&mut self) -> Vec<Response>;
}

impl<T> OutboundConnectorDriver for T
where
    T: CallConnectorDriver,
{
    fn call(&mut self) -> Option<&mut dyn CallConnectorDriver> {
        Some(self)
    }

    fn session(&mut self) -> Option<&mut dyn SessionConnectorDriver> {
        None
    }
}

/// One observation emitted by a Session Driver after a journaled action.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionObservation {
    /// A journaled outbound opening established the logical session.
    Opened(SessionId),
    /// One journaled message write was accepted by the local protocol stack.
    WriteSettled {
        /// Session that owns the message.
        session: SessionId,
        /// Write obligation that settled.
        write: SessionWriteId,
    },
    /// The session reached its sole terminal fact. The output must target the
    /// conventional `exit` port and is validated against the resolved sheet.
    Exited {
        /// Session that became terminal.
        session: SessionId,
        /// Typed terminal payload proposed for ordinary graph emission.
        exit: PortOutput,
    },
}

/// Synchronous effect boundary for one long-lived exchange.
///
/// Inbound peer messages use [`InboundConnectorDriver`]'s retained proposal
/// protocol over the same bound object. These methods cover the outbound
/// opening/write half without adding a third role or async to the core.
pub trait SessionConnectorDriver {
    /// Frames one opening handshake or returns its typed terminal refusal.
    fn prepare_session_open(
        &mut self,
        connector: &node::Path,
        data: &envelope::Data,
        resolved_settings: &alloc::collections::BTreeMap<String, crate::value::Value>,
    ) -> RequestPreparation;

    /// Projects one machine-decided session terminal through this contract.
    fn scheduler_session_completion(
        &mut self,
        connector: &node::Path,
        completion: SchedulerCompletion,
    ) -> Completion;

    /// Executes an already-journaled opening intent.
    fn open_session(&mut self, intent: &SessionOpen, request: &Envelope) -> SessionStart;

    /// Frames one complete graph-to-world session message.
    fn prepare_session_write(
        &mut self,
        connector: &node::Path,
        session: SessionId,
        data: &envelope::Data,
        resolved_settings: &alloc::collections::BTreeMap<String, crate::value::Value>,
    ) -> RequestPreparation;

    /// Offers one already-journaled complete message to the protocol stack.
    fn write_session(&mut self, intent: &SessionWrite, message: &Envelope) -> SessionStart;

    /// Requests protocol closure. Default: no local action is needed.
    fn close_session(&mut self, session: SessionId) -> SessionStart {
        let _ = session;
        SessionStart::Started
    }

    /// Requests forced local shutdown at the retained Closing settlement
    /// deadline. Unlike protocol close, this handoff cannot wait for peer
    /// cooperation or a free message slot. It is idempotent for this exact
    /// resource and may be repeated after recovery of the closing scope.
    ///
    /// This does not prove an exit or release journal correlation. A physical
    /// Driver must retain its cleanup request without blocking the core; a
    /// resource-free scripted world has nothing to stop.
    fn abort_session(&mut self, session: SessionId) {
        let _ = session;
    }

    /// Releases local resources after the Session's terminal transaction
    /// commits, including a scheduler-owned unknown outcome. This is not a
    /// second protocol close and cannot publish another terminal or start a
    /// grace period. The exact reference fences any successor resource.
    ///
    /// Handoff must not block on the peer or child. A physical Driver owns
    /// any remaining cleanup; a resource-free scripted world has none.
    fn release_session(&mut self, session: SessionId) {
        let _ = session;
    }

    /// Session observations accumulated since the previous drain.
    fn session_events(&mut self) -> Vec<SessionObservation>;
}

/// Verified Session routing table consumed by the generic boundary pump.
pub trait SessionConnectorBindings {
    /// Frames one outbound session opening through its exact Driver.
    fn prepare_session_open(
        &mut self,
        connector: &node::Path,
        data: &envelope::Data,
        resolved_settings: &alloc::collections::BTreeMap<String, crate::value::Value>,
    ) -> Result<RequestPreparation, DriverRouteError>;

    /// Projects a scheduler-owned session terminal through the bound Driver.
    fn scheduler_session_completion(
        &mut self,
        connector: &node::Path,
        completion: SchedulerCompletion,
    ) -> Result<Completion, DriverRouteError>;

    /// Executes an already-journaled opening intent.
    fn open_session(
        &mut self,
        intent: &SessionOpen,
        request: &Envelope,
    ) -> Result<SessionStart, DriverRouteError>;

    /// Frames one outbound message for an existing logical session.
    fn prepare_session_write(
        &mut self,
        connector: &node::Path,
        session: SessionId,
        data: &envelope::Data,
        resolved_settings: &alloc::collections::BTreeMap<String, crate::value::Value>,
    ) -> Result<RequestPreparation, DriverRouteError>;

    /// Offers one already-journaled write to the protocol stack.
    fn write_session(
        &mut self,
        intent: &SessionWrite,
        message: &Envelope,
    ) -> Result<SessionStart, DriverRouteError>;

    /// Requests closure through the owning Driver.
    fn close_session(
        &mut self,
        connector: &node::Path,
        session: SessionId,
    ) -> Result<SessionStart, DriverRouteError>;

    /// Requests forced cleanup through the exact Session's owning Driver,
    /// without forgetting its correlation before terminal commit.
    fn abort_session(
        &mut self,
        connector: &node::Path,
        session: SessionId,
    ) -> Result<(), DriverRouteError>;

    /// Collects Session observations from every bound Session family.
    fn session_events(&mut self) -> Result<Vec<SessionObservation>, DriverRouteError>;

    /// Notifies the owning Driver after the terminal transaction commits,
    /// then releases routing-table ownership. An already forgotten reference
    /// is inert; a missing bound Driver is an integrity fault.
    fn forget_session(&mut self, session: SessionId) -> Result<(), DriverRouteError>;
}

/// The verified outbound routing table consumed by the core VM.
///
/// A single Driver implements this automatically. The application-level
/// `RuntimeBindings` implementation selects among several contract families
/// and returns a typed error if its pre-VM proof is ever contradicted. The VM
/// therefore never matches `Process`, `File`, or another concrete contract.
/// VM entry points borrow this object-safe boundary dynamically: the scheduler
/// and recovery code are shared across Driver bindings rather than duplicated
/// for each binding implementation in an embedded image.
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
        resolved_settings: &alloc::collections::BTreeMap<String, crate::value::Value>,
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
    fn cancel(&mut self, connector: &node::Path, call_id: &CallId) -> Result<(), DriverRouteError>;

    /// Starts one already-journaled call through its owning binding.
    ///
    /// # Errors
    ///
    /// The planned path has no stable outbound route.
    fn start(
        &mut self,
        intent: &CallOpen,
        request: &Envelope,
    ) -> Result<CallStart, DriverRouteError>;

    /// Collects all answers since the previous round.
    ///
    /// # Errors
    ///
    /// A binding invariant was contradicted while polling.
    fn events(&mut self) -> Result<Vec<Response>, DriverRouteError>;

    /// Frames one Session opening through the exchange-specific binding.
    fn prepare_session_open(
        &mut self,
        connector: &node::Path,
        data: &envelope::Data,
        resolved_settings: &alloc::collections::BTreeMap<String, crate::value::Value>,
    ) -> Result<RequestPreparation, DriverRouteError> {
        let _ = (data, resolved_settings);
        Err(DriverRouteError::UnboundSession(connector.clone()))
    }

    /// Projects a scheduler-owned Session terminal through its contract.
    fn scheduler_session_completion(
        &mut self,
        connector: &node::Path,
        completion: SchedulerCompletion,
    ) -> Result<Completion, DriverRouteError> {
        let _ = completion;
        Err(DriverRouteError::UnboundSession(connector.clone()))
    }

    /// Offers one already-journaled Session opening.
    fn open_session(
        &mut self,
        intent: &SessionOpen,
        request: &Envelope,
    ) -> Result<SessionStart, DriverRouteError> {
        let _ = request;
        Err(DriverRouteError::UnboundSession(
            intent.connector_path.clone(),
        ))
    }

    /// Frames one complete Session message.
    fn prepare_session_write(
        &mut self,
        connector: &node::Path,
        session: SessionId,
        data: &envelope::Data,
        resolved_settings: &alloc::collections::BTreeMap<String, crate::value::Value>,
    ) -> Result<RequestPreparation, DriverRouteError> {
        let _ = (session, data, resolved_settings);
        Err(DriverRouteError::UnboundSession(connector.clone()))
    }

    /// Offers one already-journaled Session write.
    fn write_session(
        &mut self,
        intent: &SessionWrite,
        message: &Envelope,
    ) -> Result<SessionStart, DriverRouteError> {
        let _ = message;
        Err(DriverRouteError::UnboundSession(
            intent.connector_path.clone(),
        ))
    }

    /// Requests closure of one live Session.
    fn close_session(
        &mut self,
        connector: &node::Path,
        session: SessionId,
    ) -> Result<SessionStart, DriverRouteError> {
        let _ = session;
        Err(DriverRouteError::UnboundSession(connector.clone()))
    }

    /// Requests forced local shutdown after a retained Closing deadline.
    fn abort_session(
        &mut self,
        connector: &node::Path,
        session: SessionId,
    ) -> Result<(), DriverRouteError> {
        let _ = session;
        Err(DriverRouteError::UnboundSession(connector.clone()))
    }

    /// Collects Session observations from every bound family.
    fn session_events(&mut self) -> Result<Vec<SessionObservation>, DriverRouteError> {
        Ok(Vec::new())
    }

    /// Releases one committed terminal Session's local resources and routing
    /// state. It must not perform another protocol exchange.
    fn forget_session(&mut self, session: SessionId) -> Result<(), DriverRouteError> {
        let _ = session;
        Ok(())
    }
}

impl<T> OutboundConnectorBindings for T
where
    T: CallConnectorDriver + ?Sized,
{
    fn prepare_request(
        &mut self,
        connector: &node::Path,
        data: &envelope::Data,
        resolved_settings: &alloc::collections::BTreeMap<String, crate::value::Value>,
    ) -> Result<RequestPreparation, DriverRouteError> {
        Ok(CallConnectorDriver::prepare_request(
            self,
            connector,
            data,
            resolved_settings,
        ))
    }

    fn scheduler_completion(
        &mut self,
        connector: &node::Path,
        completion: SchedulerCompletion,
    ) -> Result<Completion, DriverRouteError> {
        Ok(CallConnectorDriver::scheduler_completion(
            self, connector, completion,
        ))
    }

    fn cancel(
        &mut self,
        _connector: &node::Path,
        call_id: &CallId,
    ) -> Result<(), DriverRouteError> {
        CallConnectorDriver::cancel(self, call_id);
        Ok(())
    }

    fn start(
        &mut self,
        intent: &CallOpen,
        request: &Envelope,
    ) -> Result<CallStart, DriverRouteError> {
        Ok(CallConnectorDriver::start(self, intent, request))
    }

    fn events(&mut self) -> Result<Vec<Response>, DriverRouteError> {
        Ok(CallConnectorDriver::events(self))
    }
}

/// Supplies lifecycle observations.
pub trait Supervisor {
    /// The next supervision observation, if one is pending.
    fn next_event(&mut self) -> Option<LifecycleObservation>;
}

/// One settled pump round: what may enter the VM, and what may not —
/// visibly, never silently.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PollBatch {
    /// The won closures, routable to the VM.
    pub events: Vec<BoundaryObservation>,
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
pub enum PollFault {
    /// A journaled intent whose request envelope cannot be found:
    /// an integrity fault in the causal history.
    MissingRequest(CallId),
    /// The verified Driver routing table was contradicted.
    Driver(DriverRouteError),
    /// The journal refused an operation.
    Journal(crate::journal::Error),
}

impl fmt::Display for PollFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRequest(call_id) => {
                write!(f, "no journaled envelope for the request of call {call_id}")
            }
            Self::Driver(error) => error.fmt(f),
            Self::Journal(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for PollFault {}

/// Connects journaled Call openings to outbound Drivers and their observations.
///
/// A fresh round has two halves:
///
/// 1. Offer newly journaled Calls to their Drivers. Journal-before-effect:
///    the opening record exists before a Driver is asked to execute it.
/// 2. Collect completions, check them against the Contract, and settle them
///    through the journal's close-once rule into [`BoundaryObservation`]s.
///
/// A refused settlement keeps its batch for a later round; another batch is
/// not polled until that one is complete. The VM builds the output envelopes
/// and commits the Connector transaction. The poller never creates an envelope.
/// Mocks replace Drivers, never this bookkeeping.
///
/// Restart discipline: an old Call is never executed again by recovery.
/// A later Retry admission is a new, independently authorized Call.
/// A poller over an existing journal is therefore built with
/// [`DriverPoller::resume`], which treats every already-open call as
/// started and re-calls nothing.
#[derive(Debug, Default)]
pub struct DriverPoller {
    started: BTreeSet<CallId>,
    pending_settlement: Option<Box<PendingSettlement>>,
}

/// One handed-off Driver batch, including closures already committed before
/// a later refusal. Do not poll another batch until this one reaches the VM.
#[derive(Debug)]
struct PendingSettlement {
    responses: VecDeque<Response>,
    batch: PollBatch,
}

impl DriverPoller {
    /// A poller for a fresh run: nothing started yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A poller over an existing journal: every currently open
    /// call counts as started, so nothing is blindly re-called; the
    /// recovery fold — not the poller — decides re-calls.
    #[must_use]
    pub fn resume(journal: &dyn Journal) -> Self {
        Self {
            started: journal
                .open_calls()
                .into_iter()
                .map(|intent| intent.call_id)
                .collect(),
            pending_settlement: None,
        }
    }

    /// Drives one round: starts newly journaled open calls, then
    /// settles the driver's answers through the close-once race.
    ///
    /// # Errors
    ///
    /// [`PollFault::MissingRequest`] when the causal history lacks
    /// a journaled request — an integrity fault; journal refusals are
    /// propagated, never absorbed. A refused settlement retains that Driver
    /// batch, including earlier winning closures, until a later pump succeeds.
    pub fn poll_drivers<B: OutboundConnectorBindings + ?Sized>(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut B,
        gate: &dyn CompletionCheck,
        now: i64,
    ) -> Result<PollBatch, PollFault> {
        self.poll_drivers_with_timing(journal, bindings, gate, now, &mut |_, _| Ok(None))
    }

    pub(crate) fn poll_drivers_with_timing<B: OutboundConnectorBindings + ?Sized>(
        &mut self,
        journal: &mut dyn Journal,
        bindings: &mut B,
        gate: &dyn CompletionCheck,
        now: i64,
        timing: &mut impl FnMut(
            &CallOpen,
            &Completion,
        ) -> Result<
            Option<alloc::boxed::Box<crate::journal::deadline::Deadlines>>,
            PollFault,
        >,
    ) -> Result<PollBatch, PollFault> {
        // A timeout or abandonment can close a call outside this response
        // loop. Keep the local start cache equal to the journal's open fold,
        // rather than retaining every historical call for the run's life.
        let open = journal.open_calls();
        let open_refs: BTreeSet<_> = open.iter().map(|intent| intent.call_id).collect();
        self.started.retain(|call_id| open_refs.contains(call_id));
        // A refused batch keeps ownership of its answers. Starting more work
        // or polling the Driver here would grow retention during store pressure.
        if self.pending_settlement.is_none() {
            for intent in &open {
                if self.started.contains(&intent.call_id) {
                    continue;
                }
                let request = journal
                    .find_envelope(intent.emitting_id)
                    .ok_or(PollFault::MissingRequest(intent.call_id))?;
                let start = bindings
                    .start(intent, &request)
                    .map_err(PollFault::Driver)?;
                if start == CallStart::Started {
                    self.started.insert(intent.call_id);
                }
            }
        }
        // Settle phase: won closures become routable events; losers
        // and strays stay visible.
        let mut settlement = match self.pending_settlement.take() {
            Some(pending) => *pending,
            None => PendingSettlement {
                responses: bindings.events().map_err(PollFault::Driver)?.into(),
                batch: PollBatch::default(),
            },
        };
        while let Some(response) = settlement.responses.pop_front() {
            let Some(intent) = open
                .iter()
                .find(|intent| intent.call_id == response.call_id)
            else {
                if journal.find_call(&response.call_id).is_some() {
                    settlement.batch.discarded.push(response);
                } else {
                    settlement.batch.uncorrelated.push(response);
                }
                continue;
            };
            // Validation BEFORE the causal fact: a reply the contract
            // refuses must never close the call — journaled garbage
            // would leave an unclosable obligation.
            if let Err(reason) = gate.admit(&intent.connector_path, &response.completion) {
                settlement.batch.rejected.push((response, reason));
                continue;
            }
            let closure = Closure::Response(response.completion.clone());
            let verdict = timing(intent, &response.completion).and_then(|schedule| {
                journal
                    .close_egress(&response.call_id, closure, now, schedule)
                    .map_err(PollFault::Journal)
            });
            let verdict = match verdict {
                Ok(verdict) => verdict,
                Err(error) => {
                    settlement.responses.push_front(response);
                    self.pending_settlement = Some(Box::new(settlement));
                    return Err(error);
                }
            };
            match verdict {
                CloseVerdict::Won => {
                    self.started.remove(&response.call_id);
                    settlement.batch.events.push(BoundaryObservation {
                        call_id: response.call_id,
                        connector_path: intent.connector_path.clone(),
                        emitting_id: intent.emitting_id,
                        completion: response.completion,
                    });
                }
                CloseVerdict::AlreadyClosed => settlement.batch.discarded.push(response),
            }
        }
        Ok(settlement.batch)
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

    #[test]
    fn codec_refusals_describe_value_kinds_without_formatting_payloads() {
        use crate::envelope::{Data, Value};
        let sentinel = "classified-payload-must-not-escape";
        let payload = Data::materialized(Value::Map(alloc::collections::BTreeMap::from([(
            String::from(sentinel),
            Value::Int(1),
        )])));
        for refusal in [
            network_payload_bytes("text", &payload).expect_err("map is not text"),
            bytes_payload(&payload).expect_err("map is not bytes"),
            prepare_lines(&payload).expect_err("map is not lines"),
        ] {
            assert!(refusal.contains("Map"));
            assert!(!refusal.contains(sentinel));
        }
    }

    #[test]
    fn network_codecs_frame_one_body_or_message_without_transport_delimiters() {
        use crate::envelope::{Data, Encoding, Value};

        assert_eq!(
            network_payload_bytes(
                "json",
                &Data::materialized(Value::Struct(alloc::vec![(
                    String::from("answer"),
                    Value::Int(42),
                )])),
            )
            .unwrap(),
            br#"{"answer":42}"#
        );
        assert_eq!(
            network_payload_bytes(
                "text",
                &Data::materialized(Value::Text(String::from("line\n"))),
            )
            .unwrap(),
            b"line\n"
        );
        assert_eq!(
            network_payload_bytes("bytes", &Data::encoded(Encoding::Raw, [0, 0xff, b'\n']))
                .unwrap(),
            [0, 0xff, b'\n']
        );
        assert!(
            network_payload_bytes("json", &Data::encoded(Encoding::Raw, b"{}".to_vec())).is_err()
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
        let provenance = IngressEnvelopeProvenance::single(node::Path::new(["inbound"]).unwrap());
        assert_eq!(provenance.connector_path().to_string(), "inbound");
        assert_eq!(provenance.replica(), 0);
    }

    /// A scripted driver: records what it was asked to start,
    /// answers from a script.
    #[derive(Default)]
    struct Scripted {
        started: Vec<(CallOpen, Envelope)>,
        replies: Vec<Response>,
    }

    impl CallConnectorDriver for Scripted {
        fn prepare_request(
            &mut self,
            _: &node::Path,
            data: &Data,
            _: &alloc::collections::BTreeMap<String, crate::value::Value>,
        ) -> RequestPreparation {
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

        fn start(&mut self, intent: &CallOpen, request: &Envelope) -> CallStart {
            self.started.push((intent.clone(), request.clone()));
            CallStart::Started
        }

        fn events(&mut self) -> Vec<Response> {
            core::mem::take(&mut self.replies)
        }
    }

    struct StartRefusal {
        refuse: bool,
        pending: bool,
        attempts: usize,
    }

    impl OutboundConnectorBindings for StartRefusal {
        fn prepare_request(
            &mut self,
            _: &node::Path,
            data: &Data,
            _: &alloc::collections::BTreeMap<String, crate::value::Value>,
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

        fn cancel(&mut self, _: &node::Path, _: &CallId) -> Result<(), DriverRouteError> {
            Ok(())
        }

        fn start(
            &mut self,
            intent: &CallOpen,
            _: &Envelope,
        ) -> Result<CallStart, DriverRouteError> {
            self.attempts += 1;
            if self.refuse {
                return Err(DriverRouteError::UnboundOutbound(
                    intent.connector_path.clone(),
                ));
            }
            if self.pending {
                return Ok(CallStart::Pending);
            }
            Ok(CallStart::Started)
        }

        fn events(&mut self) -> Result<Vec<Response>, DriverRouteError> {
            Ok(Vec::new())
        }
    }

    fn seeded_journal() -> (MemJournal, CallOpen) {
        let mut journal = MemJournal::new(GraphVersion(1));
        let committed = journal
            .append_ingress(IngressSubmission {
                provenance: IngressEnvelopeProvenance::single(
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
        let intent = CallOpen {
            call_id: ids::call_id(ids::delivery_id(&edge, emitting_id), prepared.digest()),
            emitting_id,
            edge,
            connector_path,
            prepared,
            idempotency_key: None,
            resolved_settings: alloc::collections::BTreeMap::new(),
            logical_time: 0,
        };
        journal.open_egress(intent.clone(), None).unwrap();
        (journal, intent)
    }

    #[test]
    fn the_record_exists_before_the_driver_hears() {
        let (mut journal, intent) = seeded_journal();
        let mut driver = Scripted::default();
        let mut dispatcher = DriverPoller::new();
        dispatcher
            .poll_drivers(&mut journal, &mut driver, &AcceptAnyCompletion, 0)
            .unwrap();
        assert_eq!(driver.started.len(), 1);
        assert_eq!(driver.started[0].0.call_id, intent.call_id);
        // A second pump never re-starts an already-started call.
        dispatcher
            .poll_drivers(&mut journal, &mut driver, &AcceptAnyCompletion, 0)
            .unwrap();
        assert_eq!(driver.started.len(), 1);
    }

    #[test]
    fn a_refused_start_is_not_remembered_as_started() {
        let (mut journal, _) = seeded_journal();
        let mut bindings = StartRefusal {
            refuse: true,
            pending: false,
            attempts: 0,
        };
        let mut dispatcher = DriverPoller::new();

        assert!(matches!(
            dispatcher.poll_drivers(&mut journal, &mut bindings, &AcceptAnyCompletion, 0),
            Err(PollFault::Driver(DriverRouteError::UnboundOutbound(_)))
        ));
        assert_eq!(bindings.attempts, 1);
        bindings.refuse = false;
        dispatcher
            .poll_drivers(&mut journal, &mut bindings, &AcceptAnyCompletion, 0)
            .unwrap();
        assert_eq!(bindings.attempts, 2, "the open call was retried");
    }

    #[test]
    fn a_backpressured_start_remains_open_and_is_offered_again() {
        let (mut journal, _) = seeded_journal();
        let mut bindings = StartRefusal {
            refuse: false,
            pending: true,
            attempts: 0,
        };
        let mut dispatcher = DriverPoller::new();

        dispatcher
            .poll_drivers(&mut journal, &mut bindings, &AcceptAnyCompletion, 0)
            .unwrap();
        assert_eq!(bindings.attempts, 1);
        assert_eq!(journal.open_calls().len(), 1);

        bindings.pending = false;
        dispatcher
            .poll_drivers(&mut journal, &mut bindings, &AcceptAnyCompletion, 0)
            .unwrap();
        assert_eq!(bindings.attempts, 2, "capacity recovery retries the call");
        dispatcher
            .poll_drivers(&mut journal, &mut bindings, &AcceptAnyCompletion, 0)
            .unwrap();
        assert_eq!(bindings.attempts, 2, "a started call is never duplicated");
    }

    #[test]
    fn a_resumed_dispatcher_re_calls_nothing() {
        let (mut journal, _) = seeded_journal();
        let mut driver = Scripted::default();
        // Restart over an existing journal: the open call counts as
        // started; re-calls are the recovery fold's decision.
        let mut dispatcher = DriverPoller::resume(&journal);
        dispatcher
            .poll_drivers(&mut journal, &mut driver, &AcceptAnyCompletion, 0)
            .unwrap();
        assert!(driver.started.is_empty());
    }

    #[test]
    fn scheduler_closures_are_pruned_from_the_start_cache() {
        let (mut journal, intent) = seeded_journal();
        let mut driver = Scripted::default();
        let mut dispatcher = DriverPoller::new();
        dispatcher
            .poll_drivers(&mut journal, &mut driver, &AcceptAnyCompletion, 0)
            .unwrap();
        assert!(dispatcher.started.contains(&intent.call_id));
        journal
            .close_egress(&intent.call_id, Closure::Timeout, 0, None)
            .unwrap();

        dispatcher
            .poll_drivers(&mut journal, &mut driver, &AcceptAnyCompletion, 0)
            .unwrap();
        assert!(dispatcher.started.is_empty());
    }

    #[test]
    fn won_closures_become_routable_events() {
        let (mut journal, intent) = seeded_journal();
        let mut driver = Scripted::default();
        let mut dispatcher = DriverPoller::new();
        dispatcher
            .poll_drivers(&mut journal, &mut driver, &AcceptAnyCompletion, 0)
            .unwrap();
        driver.replies = vec![
            Response {
                call_id: intent.call_id,
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
                call_id: intent.call_id,
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
            .poll_drivers(&mut journal, &mut driver, &AcceptAnyCompletion, 0)
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
        let mut dispatcher = DriverPoller::new();
        dispatcher
            .poll_drivers(&mut journal, &mut driver, &AcceptAnyCompletion, 0)
            .unwrap();
        driver.replies = vec![Response {
            call_id: intent.call_id,
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
            .poll_drivers(&mut journal, &mut driver, &AcceptAnyCompletion, 0)
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
        let mut dispatcher = DriverPoller::new();
        driver.replies = vec![Response {
            call_id: ids::call_id(
                ids::delivery_id("x.out>ghost.in", envelope::Id::from_bytes([9; 16])),
                &[0; 32],
            ),
            completion: Completion::process_exit(ProcessExit::Exited {
                exit_code: 0,
                signal: None,
            }),
        }];
        let batch = dispatcher
            .poll_drivers(&mut journal, &mut driver, &AcceptAnyCompletion, 0)
            .unwrap();
        assert!(batch.events.is_empty());
        assert_eq!(batch.uncorrelated.len(), 1, "correlation faults surface");
    }

    #[test]
    fn completion_construction_seals_the_terminal_barrier() {
        let out = PortOutput {
            port: node::Port::conventional_out(),
            data: Data::materialized(crate::value::Value::Int(1)),
            ext: envelope::Namespace::new(),
        };
        let exit = PortOutput {
            port: node::Port::conventional_exit(),
            data: Data::materialized(ProcessExit::TimeoutExpired.into_value()),
            ext: envelope::Namespace::new(),
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

    #[test]
    fn file_exit_is_the_exact_portable_terminal_union() {
        let ty = file_exit_type().expect("the standard FileExit type is valid");
        let crate::datatype::DataType::TaggedUnion(union) = &ty else {
            panic!("FileExit must remain a tagged union")
        };
        let cases: Vec<_> = union.cases().iter().map(|case| case.name()).collect();
        assert_eq!(
            cases,
            [
                "AtomicReplaceUnavailable",
                "Completed",
                "InvalidPath",
                "IoFailed",
                "NotAFile",
                "NotFound",
                "OutcomeUnknown",
                "PermissionDenied",
                "ReadLimitExceeded",
                "TimeoutExpired",
            ]
        );
        let limit = union
            .cases()
            .iter()
            .find(|case| case.name() == "ReadLimitExceeded")
            .expect("the bounded-read case");
        assert_eq!(limit.payload().fields().len(), 1);
        assert_eq!(limit.payload().fields()[0].name(), "limit");
        assert_eq!(
            limit.payload().fields()[0].ty(),
            &crate::datatype::DataType::UINT
        );
        assert!(
            union
                .cases()
                .iter()
                .filter(|case| case.name() != "ReadLimitExceeded")
                .all(|case| case.payload().fields().is_empty())
        );
        for exit in [
            FileExit::Completed,
            FileExit::NotFound,
            FileExit::NotAFile,
            FileExit::PermissionDenied,
            FileExit::InvalidPath,
            FileExit::ReadLimitExceeded { limit: 42 },
            FileExit::AtomicReplaceUnavailable,
            FileExit::IoFailed,
            FileExit::TimeoutExpired,
            FileExit::OutcomeUnknown,
        ] {
            let value = exit.into_value();
            assert!(ty.accepts(&value), "{value:?} must inhabit FileExit");
        }
    }

    #[test]
    fn every_network_exit_projection_inhabits_its_closed_union() {
        let client_ty = http_client_exit_type().expect("the HttpClientExit type");
        for exit in [
            HttpClientExit::Completed { status: 418 },
            HttpClientExit::DnsFailed,
            HttpClientExit::ConnectFailed,
            HttpClientExit::TlsFailed,
            HttpClientExit::ProtocolFailed,
            HttpClientExit::TimeoutExpired,
            HttpClientExit::BodyTooLarge { limit: 42 },
            HttpClientExit::DecodeFailed,
            HttpClientExit::OutcomeUnknown,
        ] {
            let value = exit.into_value();
            assert!(
                client_ty.accepts(&value),
                "{value:?} must inhabit HttpClientExit"
            );
        }

        let server_ty = http_server_exit_type().expect("the HttpServerExit type");
        for exit in [
            HttpServerExit::Completed { status: 202 },
            HttpServerExit::RequestExpired,
            HttpServerExit::Overloaded,
            HttpServerExit::PeerDisconnected,
            HttpServerExit::OutcomeUnknown,
        ] {
            let value = exit.into_value();
            assert!(
                server_ty.accepts(&value),
                "{value:?} must inhabit HttpServerExit"
            );
        }

        let websocket_ty = websocket_exit_type().expect("the WebSocketExit type");
        for exit in [
            WebSocketExit::Closed {
                code: 1000,
                reason: String::from("done"),
            },
            WebSocketExit::DnsFailed,
            WebSocketExit::ConnectFailed,
            WebSocketExit::TlsFailed,
            WebSocketExit::HandshakeRejected { status: 403 },
            WebSocketExit::ProtocolFailed,
            WebSocketExit::TransportLost,
            WebSocketExit::TimeoutExpired,
            WebSocketExit::MessageTooLarge { limit: 42 },
            WebSocketExit::DecodeFailed,
            WebSocketExit::OutcomeUnknown,
        ] {
            let value = exit.into_value();
            assert!(
                websocket_ty.accepts(&value),
                "{value:?} must inhabit WebSocketExit"
            );
        }
    }
}
