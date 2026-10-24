//! Requests, responses and ingress proposals exchanged with Connector Drivers.
//!
//! Drivers supply drafts and observations. Execution assigns envelope identities
//! and commits journal facts before returning admission or starting effects.

use crate::connector::stdlib::{
    FileExit, HttpClientExit, HttpServerExit, ProcessExit, WebSocketExit,
};
use crate::envelope;
use crate::node;
use alloc::{string::String, vec::Vec};
use core::fmt;

/// Provenance shared by root envelopes entering through one logical
/// Connector replica: exactly `(connector_path, replica)`.
///
/// This is neither a network address nor the identity of one submission. The
/// journal combines it with an accepted inbound sequence to identify one root;
/// keeping the replica opaque here leaves multi-host placement out of causal
/// identity. In single-process execution the replica is 0.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IngressProvenance {
    connector_path: node::Path,
    replica: u64,
}

impl IngressProvenance {
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
    /// Prepared request bytes and their integrity digest. The causal form
    /// stores the digest (key 30); the lossless journal codec also records
    /// the bytes for recovery.
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
    /// Logical instant fixed by the Execution for the causing transaction. This is
    /// execution context for scripted Drivers, not an EGRESS identity field
    /// and never an invitation for a Driver to read the host clock.
    pub logical_time: crate::time::LogicalTime,
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

/// A port, payload and external metadata supplied for an ingress or a
/// Connector output, before the runtime constructs the envelope.
///
/// A draft has no `meta.sys` or `meta.app`. Ingress admission or the Connector
/// transaction assigns its causal identity and commits the resulting envelope.
/// It is not a journal [`crate::journal::Emission`].
#[derive(Debug, Clone, PartialEq)]
pub struct EnvelopeDraft {
    /// App input or Connector output carrying this draft.
    pub port: node::Port,
    /// Payload representation supplied by the source.
    pub data: envelope::Data,
    /// External metadata supplied with this draft.
    ///
    /// For a Connector completion, the Execution validates and overlays these fields
    /// after ordinary request-zone inheritance. Drivers never assign `meta.sys`
    /// or write `meta.app`.
    pub ext: envelope::Namespace,
}

/// Ordered output drafts for one Call completion. Exactly one draft targets
/// `exit`, and it is last; Drivers and Lab mocks use the same checked form.
#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    outputs: Vec<EnvelopeDraft>,
}

impl Completion {
    /// Checks that the drafts contain exactly one final `exit` output.
    ///
    /// # Errors
    ///
    /// Refuses a missing, repeated, or non-terminal `exit` output.
    pub fn try_new(outputs: Vec<EnvelopeDraft>) -> Result<Self, CompletionError> {
        // `exit` is the boundary protocol's conventional terminal port, not a
        // Execution routing shortcut. The resolved port table owns its type and the
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
    pub fn outputs(&self) -> &[EnvelopeDraft] {
        &self.outputs
    }

    /// Gives the ordered drafts to the Execution after the close race is won.
    #[must_use]
    pub fn into_outputs(self) -> Vec<EnvelopeDraft> {
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
        outputs.extend(out.into_iter().map(|data| EnvelopeDraft {
            port: node::Port::conventional_out(),
            data,
            ext: envelope::Namespace::new(),
        }));
        outputs.extend(err.into_iter().map(|data| EnvelopeDraft {
            port: node::Port::conventional_err(),
            data,
            ext: envelope::Namespace::new(),
        }));
        outputs.push(EnvelopeDraft {
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
            outputs.push(EnvelopeDraft {
                port: node::Port::conventional_out(),
                data,
                ext: envelope::Namespace::new(),
            });
        }
        outputs.push(EnvelopeDraft {
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
            outputs: alloc::vec![EnvelopeDraft {
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
                EnvelopeDraft {
                    port: node::Port::conventional_out(),
                    data: out,
                    ext,
                },
                EnvelopeDraft {
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
pub struct CallResponse {
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

/// A settled outbound Call completion awaiting its Connector transaction.
///
/// The Call closure is already journaled; this carrier adds no record or
/// envelope. The Execution builds and commits the output emissions.
#[derive(Debug, Clone, PartialEq)]
pub struct ClosedCall {
    /// The settled call.
    pub call_id: CallId,
    /// The connector node that owns the coming transaction.
    pub connector_path: node::Path,
    /// The request envelope's id — the transaction's parent.
    pub emitting_id: envelope::Id,
    /// What the call produced, in publication order with `exit` last.
    pub completion: Completion,
}

/// Selects the sequence number used to identify an ingress envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngressSequence {
    /// Allocate the next number — the live mode.
    Next,
    /// Replay an explicit number — fixtures replay recorded seqs; a
    /// mismatch with the expected next number is refused.
    Explicit(u64),
}

/// An ingress proposed for atomic admission. The runtime derives the root
/// envelope's identity from its provenance and sequence before committing it.
#[derive(Debug, Clone, PartialEq)]
pub struct IngressProposal {
    /// Stable external origin shared by roots from this logical replica.
    pub provenance: IngressProvenance,
    /// App input or Connector output, payload and external metadata.
    pub draft: EnvelopeDraft,
    /// How the sequence is chosen.
    pub sequence: IngressSequence,
    /// Opens or addresses a Session; absent for Event and Call ingress.
    pub session: Option<InboundSession>,
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
/// assigns the envelope and any Call or Session identity.
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

/// A Driver's pending ingress and its local handle. The Driver keeps it until
/// admission succeeds or returns a final rejection; backpressure leaves it pending.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingIngress {
    /// Stable while the proposal remains pending in the Driver.
    pub handle: IngressProposalHandle,
    /// Ingress whose provenance names the bound source Connector.
    pub proposal: IngressProposal,
}

/// Admission acknowledgement returned to the Driver only after commit.
/// It carries the envelope and exchange identities needed by the transport;
/// [`crate::journal::CommittedIngress`] instead returns the full journal result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmittedIngress {
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

/// Result of attempting one proposal against bounded runtime admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngressAdmission {
    /// The causal fact committed.
    Accepted(AdmittedIngress),
    /// No causal fact committed; the Driver must retain the proposal and
    /// withhold its application acknowledgement.
    Pending,
    /// No causal fact committed; the Driver receives a terminal refusal.
    Rejected(IngressRejection),
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
/// The Execution decides that the request budget elapsed; the Driver owns only the
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
    /// observation through [`crate::connector::driver::InboundCall::poll_completions`].
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
    /// terminal observation through [`crate::connector::driver::InboundCall::poll_completions`].
    Started,
    /// No protocol bytes were written; finite local capacity kept the same
    /// timeout request pending for a later offer.
    Pending,
}

/// Result of offering one already-journaled call to its bounded Driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallStart {
    /// The Driver owns the effect and must eventually answer or be cancelled.
    Started,
    /// No effect started; bounded local capacity kept the call pending.
    /// The poller will offer the same journaled Call opening again.
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
        exit: EnvelopeDraft,
    },
}

#[cfg(test)]
mod tests;
