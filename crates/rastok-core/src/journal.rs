//! Durable causal history and recovery model — the M0 in-memory
//! slice.
//!
//! Governing rules: [`delivery-semantics.md`] for the three causal
//! record kinds and the outbound WAL, [`envelope.md`] §2 for the id
//! recipes, [`identity.md`] §3 for the fields each record seals. This
//! module exposes the *logical* journal API — typed records and
//! semantic queries; a physical layout comes later and must never
//! leak into these types. The record slice itself is an
//! implementation detail of [`MemJournal`], for tests and goldens —
//! consumers of the trait ask semantic questions instead.
//!
//! The journal is append-only: closing a call adds a new fact, it
//! never rewrites the open one. Records are written by the runtime
//! mechanics only; Drivers and mocks supply events, never records.
//!
//! [`delivery-semantics.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/delivery-semantics.md
//! [`envelope.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/envelope.md
//! [`identity.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/identity.md

use crate::boundary::{
    CallRef, Completion, InboundCallRef, InboundEnvelopeProvenance, InboundSession,
    InboundSubmission, Intent, SessionOpenIntent, SessionRef, SessionWriteIntent, SessionWriteRef,
    SubmissionSequence,
};
use crate::envelope::{Envelope, Id, Meta, MetaSys};
use crate::identity::{InstanceAddress, InstanceRef};
use crate::node;
use crate::plan::{GraphVersion, OwnerPath};
use crate::value::Value;
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

/// Deterministic identity recipes — the injective blake3 framing of
/// `envelope.md` §2, with its domain tags.
pub mod ids {
    use super::{
        CallRef, Id, InboundEnvelopeProvenance, InstanceRef, SessionRef, SessionWriteRef, node,
    };
    use alloc::string::ToString;
    use alloc::vec::Vec;

    /// Domain tag of a root derivation: 2 fields.
    const ROOT: u8 = 0x01;
    /// Domain tag of an emission derivation: 5 fields.
    const EMISSION: u8 = 0x02;
    /// Domain tag of the internal call-identity derivation. Outside
    /// the envelope id domains (`0x01`–`0x04` are reserved by the
    /// spec); a call ref is never an envelope id.
    const CALL: u8 = 0x10;
    /// Domain tag of a logical session reference.
    const SESSION: u8 = 0x11;
    /// Domain tag of one write obligation within a session.
    const SESSION_WRITE: u8 = 0x12;
    /// Domain tag of a delivery reference.
    const DELIVERY: u8 = 0x20;
    /// Domain tag of an instance-lifetime obligation. Outside envelope-id
    /// domains; the reference couples address and journal-derived generation.
    const INSTANCE_LIFETIME: u8 = 0x21;
    /// Domain tag of one bounded Closing phase reference.
    const INSTANCE_CLOSING: u8 = 0x22;

    /// The root id of an inbound envelope:
    /// `blake3(connector_path ‖ inbound_seq)`. The single-process
    /// recipe; a non-zero replica joins the derivation when multi-host
    /// semantics land.
    #[must_use]
    pub fn root_id(provenance: &InboundEnvelopeProvenance, seq: u64) -> Id {
        Id::derive(
            ROOT,
            &[
                provenance.connector_path().to_string().as_bytes(),
                &seq.to_le_bytes(),
            ],
        )
    }

    /// The id of an emitted envelope:
    /// `blake3(node_path ‖ trigger ‖ parent_id ‖ emission_port ‖ emission_index)`.
    /// Paths, triggers, and ports contribute UTF-8; the parent its 16
    /// raw bytes; the index its u32 little-endian bytes.
    #[must_use]
    pub fn emission_id(
        node_path: &node::Path,
        trigger: &node::Trigger,
        parent: Id,
        port: &node::Port,
        index: u32,
    ) -> Id {
        Id::derive(
            EMISSION,
            &[
                node_path.to_string().as_bytes(),
                trigger.to_string().as_bytes(),
                parent.as_bytes(),
                port.name().as_bytes(),
                &index.to_le_bytes(),
            ],
        )
    }

    /// The delivery reference (envelope.md §2): one envelope delivered
    /// along one canonical edge is one delivery, and this is its
    /// identity — what `timers` (identity.md §3 key 37) seals, what
    /// the scheduler resolves, and what the call identity derives
    /// from. A fan-out of one envelope is one distinct reference per
    /// receiving edge, two ports of one connector included.
    #[must_use]
    pub fn delivery_ref(edge: &str, delivery: Id) -> Id {
        Id::derive(DELIVERY, &[edge.as_bytes(), delivery.as_bytes()])
    }

    /// The stable 16-byte timer reference for one instance generation.
    #[must_use]
    pub fn instance_lifetime_ref(instance: &InstanceRef) -> Id {
        Id::derive(
            INSTANCE_LIFETIME,
            &[
                instance.address().as_bytes(),
                &instance.generation().to_le_bytes(),
            ],
        )
    }

    /// Stable reference of one bounded Closing phase. `phase` is the
    /// extend-only timer-kind number (drain or settlement), so the two
    /// obligations of one generation cannot collide.
    #[must_use]
    pub fn instance_closing_ref(instance: &InstanceRef, phase: u8) -> Id {
        Id::derive(
            INSTANCE_CLOSING,
            &[
                instance.address().as_bytes(),
                &instance.generation().to_le_bytes(),
                &[phase],
            ],
        )
    }

    /// The internal identity of an outbound call (envelope.md §2):
    /// one delivery carrying one prepared request — derived from the
    /// delivery reference and the digest of the exact prepared bytes,
    /// recomputable from any record that carries them (OUTBOUND keys
    /// 40, 43, 47), never stored as a new causal field. The digest
    /// says WHAT was sent; the reference says BY WHERE — routing
    /// never salts the digest.
    #[must_use]
    pub fn call_ref(delivery_ref: Id, request_digest: &[u8; 32]) -> CallRef {
        CallRef::new(Id::derive(CALL, &[delivery_ref.as_bytes(), request_digest]))
    }

    /// Logical reference of one inbound-opened session. The opening root is
    /// already journal-derived; the subtype frame keeps the recipe disjoint
    /// from outbound openings under the shared session domain.
    #[must_use]
    pub fn inbound_session_ref(provenance: &InboundEnvelopeProvenance, opening: Id) -> SessionRef {
        let subtype = [1];
        let mut parts = Vec::with_capacity(
            provenance
                .connector_path()
                .segments()
                .len()
                .saturating_add(2),
        );
        parts.push(subtype.as_slice());
        parts.extend(
            provenance
                .connector_path()
                .segments()
                .iter()
                .map(|segment| segment.as_bytes()),
        );
        parts.push(opening.as_bytes());
        SessionRef::new(Id::derive(SESSION, &parts))
    }

    /// Logical reference of one outbound-opened session.
    #[must_use]
    pub fn outbound_session_ref(
        connector: &node::Path,
        emitting: Id,
        request_digest: &[u8; 32],
    ) -> SessionRef {
        let subtype = [2];
        let mut parts = Vec::with_capacity(connector.segments().len().saturating_add(3));
        parts.push(subtype.as_slice());
        parts.extend(
            connector
                .segments()
                .iter()
                .map(|segment| segment.as_bytes()),
        );
        parts.push(emitting.as_bytes());
        parts.push(request_digest);
        SessionRef::new(Id::derive(SESSION, &parts))
    }

    /// Identity of one graph-to-world session write.
    #[must_use]
    pub fn session_write_ref(
        session: SessionRef,
        emitting: Id,
        message_digest: &[u8; 32],
    ) -> SessionWriteRef {
        SessionWriteRef::new(Id::derive(
            SESSION_WRITE,
            &[session.id().as_bytes(), emitting.as_bytes(), message_digest],
        ))
    }
}

/// Extend-only timer-kind registry stored in transaction key 37.
pub mod timer_kind {
    /// One delivered Connector call budget.
    pub const CALL_TIMEOUT: u8 = 1;
    /// One join correlation bound (reserved until joins execute).
    pub const JOIN_WITHIN: u8 = 2;
    /// One logical instance generation lifetime.
    pub const INSTANCE_LIFETIME: u8 = 3;
    /// One Saga deadline (reserved until Saga execution).
    pub const SAGA_TIMEOUT: u8 = 4;
    /// One Loop backoff wakeup (reserved until Loop execution).
    pub const LOOP_BACKOFF: u8 = 5;
    /// End of an instance's admitted-work drain interval.
    pub const INSTANCE_DRAIN: u8 = 6;
    /// End of an instance's cancellation settlement interval.
    pub const INSTANCE_SETTLEMENT: u8 = 7;
    /// One admitted inbound Call response deadline.
    pub const INBOUND_CALL_TIMEOUT: u8 = 8;
    /// One client Session opening-handshake deadline.
    pub const SESSION_HANDSHAKE: u8 = 9;
}

/// One causal record.
#[derive(Debug, Clone, PartialEq)]
pub enum Record {
    /// The world entered: an envelope was born at ingress.
    Inbound(InboundRecord),
    /// One node fired: the atomic commit packet.
    Transaction(TransactionRecord),
    /// The graph acted on the world: one fact of a call's WAL — the
    /// open, then separately its closure. The past never rewrites.
    Outbound(OutboundRecord),
}

/// An accepted inbound envelope and its record (`identity.md` §3,
/// keys 20–28). The record autonomously reconstructs its root id from
/// `(connector_path, replica, seq)`.
#[derive(Debug, Clone, PartialEq)]
pub struct InboundRecord {
    /// Stable provenance of the external root stream.
    pub provenance: InboundEnvelopeProvenance,
    /// Source port of the root: an App input when [`Self::connector_path`] is
    /// absent, otherwise that Connector's output. Replay re-routes from this
    /// causal fact. Projects as key 24, omitted for conventional `in`.
    pub port: node::Port,
    /// Connector output which introduced this root. `None` is an explicit
    /// App-boundary injection. Replay therefore never guesses the source
    /// endpoint from provenance text.
    pub connector_path: Option<node::Path>,
    /// The atomic per-provenance sequence.
    pub seq: u64,
    /// The envelope born here.
    pub envelope: Envelope,
    /// Every target instance selected before this inbound fact committed.
    /// Singleton owners are explicit because generation is never derivable
    /// from a route's static endpoint alone.
    pub routes: Vec<RouteTarget>,
    /// Exchange-specific opening/message fact. Omitted for Event ingress;
    /// boxed so the ordinary record does not pay for the larger Call payload.
    pub exchange: Option<Box<InboundExchangeRecord>>,
}

/// Exchange-specific meaning of one committed Connector-origin root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboundExchangeRecord {
    /// One bounded inbound Call opened at the exact source instance.
    Call(InboundCallIngressRecord),
    /// One Session opening or message.
    Session(SessionIngressRecord),
}

/// Causal opening of one inbound Call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundCallIngressRecord {
    /// Root-derived Call reference.
    pub reference: InboundCallRef,
    /// Exact Connector generation that owns the request and its terminal exit.
    pub instance: InstanceRef,
    /// Dated obligations armed by this Call-opening fact. The tuple registry
    /// is shared with transaction timers; ack-mode Calls carry none.
    pub timers: Vec<(u8, Id)>,
}

/// Exchange-specific terminal reference carried by one Connector transaction.
///
/// Call and Session termination are categorically exclusive. The extension is
/// boxed because ordinary Operator and outbound-Call transactions carry
/// neither; adding a rare exchange must not widen every retained record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundExchangeExitRecord {
    /// Terminal transaction of one admitted inbound Call.
    Call(InboundCallRef),
    /// Terminal transaction of one inbound Session.
    Session(SessionRef),
}

/// Resolved session meaning of one committed inbound fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionIngressRecord {
    /// This record opened the named session.
    Open(SessionRef),
    /// This record delivered one message in the named live session.
    Message(SessionRef),
}

/// One pre-commit target selection for a single inbound envelope.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RouteTarget {
    /// Canonical edge text carrying the envelope.
    pub edge: String,
    /// Complete target instance reference returned by destination admission.
    pub target: InstanceRef,
}

/// One consumed input position — the cursor is per
/// `(graph edge × source instance reference)`, not per port: several edges can
/// feed one port.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConsumedPosition {
    /// The graph edge the input arrived on, canonically rendered.
    pub edge: String,
    /// Complete upstream instance reference which owns this edge cursor.
    pub source: InstanceRef,
    /// The position on that edge.
    pub seq: u64,
}

/// One emission of a transaction, in text order.
#[derive(Debug, Clone, PartialEq)]
pub struct Emission {
    /// The emission port.
    pub port: node::Port,
    /// The per-port emission index, in handler-text order.
    pub index: u32,
    /// The emitted envelope.
    pub envelope: Envelope,
}

/// The atomic commit packet of one node firing (`identity.md` §3,
/// keys 30–39, 48–50 and 52): consumed input, state delta, and emissions commit
/// together or not at all. [`Journal::commit`] canonicalizes the
/// sorted collections; emission order is text order and stays as
/// given.
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionRecord {
    /// The firing lifecycle/transaction owner. Ordinary Node transactions
    /// carry the same segments as their node path; Graph lifecycle records
    /// use the preserved lexical Graph owner, including the empty root path.
    pub owner_path: OwnerPath,
    /// Complete firing instance reference. It is mandatory even for a
    /// persistent singleton owner.
    pub instance: InstanceRef,
    /// Enclosing Graph generation, present on a nested owner's `l:start`.
    /// The address carries ancestor key frames but deliberately not ancestor
    /// generations; recovery needs this explicit causal edge.
    pub parent_instance: Option<InstanceRef>,
    /// A close request won atomically with this firing. The reference is the
    /// root of the closing scope; descendants may finish already-admitted
    /// work, while later external routes reserve the successor generation.
    pub close_scope: Option<InstanceRef>,
    /// What fired it.
    pub trigger: node::Trigger,
    /// Causal envelope for a lifecycle transaction that consumes no mailbox
    /// cursor. Port-triggered and call-completion transactions derive their
    /// cause from `consumed` and omit this field.
    pub cause: Option<crate::envelope::Id>,
    /// Inbound exchange terminated by this Connector transaction. The rare
    /// extension is boxed so its absence does not widen every retained
    /// transaction. The transaction's last emission must be the sole
    /// conventional `exit`.
    pub exchange_exit: Option<Box<InboundExchangeExitRecord>>,
    /// The consumed input positions; canonically sorted at commit.
    pub consumed: Vec<ConsumedPosition>,
    /// Envelope ids contributing to a presence-completing join, in the
    /// handler's declared port order. Empty for every non-join firing. This
    /// preserves the non-trigger roots without pretending an envelope has
    /// more than one parent.
    pub join_inputs: Vec<crate::envelope::Id>,
    /// The state changes; canonically sorted by field at commit.
    pub state_delta: Vec<(String, Value)>,
    /// The emissions, in text/emission-index order.
    pub emissions: Vec<Emission>,
    /// Non-singleton target selections made for those emissions before this
    /// record committed. The envelope id disambiguates repeated emissions on
    /// the same port and edge.
    pub routes: Vec<RoutedDelivery>,
    /// The dated obligations this firing arms — `(timer_kind, ref)`
    /// per identity.md §3 key 37: kind 1 = call budget, ref = the
    /// derived budget identity `budget_ref(edge, envelope_id)`
    /// (erratum E-2026-08-04-2). Sorted at commit; the due instant
    /// is never sealed.
    pub timers: Vec<(u8, crate::envelope::Id)>,
}

/// One routed delivery whose target instance was selected atomically with
/// the transaction that emitted it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RoutedDelivery {
    /// The emitted envelope being delivered.
    pub delivery: crate::envelope::Id,
    /// Canonical edge text carrying it.
    pub edge: String,
    /// Complete target instance reference selected before producer commit.
    pub target: InstanceRef,
}

/// One external-world WAL fact. Call and Session subforms remain under the
/// same OUTBOUND causal record kind; neither can be mistaken for the other.
#[derive(Debug, Clone, PartialEq)]
pub enum OutboundRecord {
    /// One fact of a Call's open/close WAL.
    Call(CallRecord),
    /// One fact of a Session opening or message-write WAL.
    Session(SessionRecord),
}

/// One fact of a Call's WAL (`delivery-semantics.md` §4): the open fact, or
/// its closure — two records, never one rewritten.
#[derive(Debug, Clone, PartialEq)]
pub struct CallRecord {
    /// The canonical call fields.
    pub intent: Intent,
    /// Which fact this record states.
    pub status: OutboundStatus,
}

/// One causal OUTBOUND fact inside the Session fold.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRecord {
    /// Connector that owns the logical session.
    pub connector_path: node::Path,
    /// Session whose fold this fact advances.
    pub session_ref: SessionRef,
    /// Exact transition stated by this record.
    pub status: SessionStatus,
}

/// Append-only states of outbound Session work.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionStatus {
    /// Journal-before-effect opening intent for a client-like session.
    Opening(SessionOpenIntent),
    /// The opening effect succeeded and messages may flow.
    Opened,
    /// One graph-to-world message became an open write obligation.
    WriteOpen(SessionWriteIntent),
    /// The Driver locally accepted the message bytes.
    WriteSettled(SessionWriteRef),
    /// Recovery cannot prove the message's external visibility; it must not
    /// be replayed blindly.
    WriteAbandoned(SessionWriteRef),
}

/// The sealed causal states of an outbound call. A late reply after
/// closure is a discarded observation — recorded elsewhere, never a
/// sealed status.
#[derive(Debug, Clone, PartialEq)]
pub enum OutboundStatus {
    /// The effect may be executing; its outcome is not yet causal.
    Open,
    /// The single causal closure.
    Closed(Closure),
    /// Resolved when recovery or bounded Closing can no longer establish a
    /// terminal observation. The open fact remains byte-identical.
    Abandoned,
}

/// What closed a call — the driver's completion or the timeout,
/// exactly one. Completion outputs are still drafts; the connector's
/// transaction builds their envelopes atomically.
#[derive(Debug, Clone, PartialEq)]
pub enum Closure {
    /// The driver answered with ordered outputs and one terminal exit.
    Response(Completion),
    /// The scheduler's budget expired first.
    Timeout,
}

/// Who won the race to close a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseVerdict {
    /// This closure is the single causal one.
    Won,
    /// Someone closed first; the caller's event becomes a discarded
    /// observation.
    AlreadyClosed,
}

/// The result of one accepting inbound transaction.
#[derive(Debug, Clone, PartialEq)]
pub struct CommittedInbound {
    /// The allocated or validated sequence.
    pub seq: u64,
    /// The envelope born here — its spine minted by the runtime.
    pub envelope: Envelope,
    /// The committed record.
    pub record: InboundRecord,
}

/// A complete ingress envelope whose causal identity is fixed but whose
/// accepting record has not committed yet.
///
/// This token lets routing evaluate `meta.sys` without either inventing a
/// provisional identity or mutating the journal before destination admission
/// succeeds. [`Journal::commit_inbound`] revalidates the sequence atomically.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedInbound {
    submission: InboundSubmission,
    connector_path: Option<node::Path>,
    seq: u64,
    envelope: Envelope,
    exchange: Option<InboundExchangeRecord>,
}

impl PreparedInbound {
    /// The sequence reserved by the current journal view.
    #[must_use]
    pub const fn seq(&self) -> u64 {
        self.seq
    }

    /// The immutable complete envelope visible to route-key expressions.
    #[must_use]
    pub const fn envelope(&self) -> &Envelope {
        &self.envelope
    }

    /// Resolved session fact, including the ref derived for an opening.
    #[must_use]
    pub const fn session(&self) -> Option<SessionIngressRecord> {
        match &self.exchange {
            Some(InboundExchangeRecord::Session(session)) => Some(*session),
            Some(InboundExchangeRecord::Call(_)) | None => None,
        }
    }

    /// Marks a Connector-origin prepared root as one bounded inbound Call.
    /// The root id is the Call ref; `instance` is selected before commit.
    ///
    /// # Errors
    ///
    /// A boundary root, Session fact, or second Call marker is contradictory.
    pub(crate) fn into_call(
        mut self,
        instance: InstanceRef,
        timers: Vec<(u8, Id)>,
    ) -> Result<Self, Error> {
        if self.connector_path.is_none() || self.exchange.is_some() {
            return Err(Error::CorruptInboundCall);
        }
        self.exchange = Some(InboundExchangeRecord::Call(InboundCallIngressRecord {
            reference: InboundCallRef::from_root(self.envelope.meta().sys().id()),
            instance,
            timers,
        }));
        Ok(self)
    }
}

/// A journal fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// An explicit submission sequence does not match the expected
    /// next number — a fixture diverged from the journal.
    SequenceMismatch {
        /// What the journal expected.
        expected: u64,
        /// What the submission carried.
        got: u64,
    },
    /// A per-provenance sequence exhausted its range; monotonicity
    /// cannot continue.
    SequenceOverflow,
    /// A route or transaction named a generation other than the one derived
    /// from the journal's lifecycle fold for that address.
    InstanceGenerationMismatch {
        /// The journal-derived reference.
        expected: InstanceRef,
        /// The transported claim.
        got: InstanceRef,
    },
    /// A transaction claims an actor other than the owner sealed into its
    /// instance address.
    InstanceOwnerMismatch {
        /// Owner derived from the instance address.
        expected: OwnerPath,
        /// Owner transported by the transaction.
        got: OwnerPath,
    },
    /// `l:start` attempted to reopen an active generation, or `l:stop`
    /// attempted to close an address whose generation never started.
    InvalidInstanceLifecycle {
        /// The addressed generation.
        instance: InstanceRef,
        /// Whether the journal fold was active before the transition.
        active: bool,
    },
    /// The parent reference is missing, unexpected, or does not name an
    /// enclosing owner. It belongs exactly on a non-root owner's `l:start`.
    InvalidParentInstance {
        /// The child lifecycle reference.
        instance: InstanceRef,
        /// The transported parent claim, if any.
        parent: Option<InstanceRef>,
    },
    /// A parent `l:stop` was attempted while at least one journaled child
    /// lifecycle obligation remained open.
    OpenChildInstances {
        /// Parent generation that must remain active.
        parent: InstanceRef,
    },
    /// A close request did not name the firing generation itself.
    InvalidCloseScope {
        /// Firing generation.
        instance: InstanceRef,
        /// Transported close root.
        scope: InstanceRef,
    },
    /// A second close request tried to govern a generation already covered by
    /// an active durable close root. The first causal request is authoritative.
    InstanceAlreadyClosing {
        /// Generation named by the losing request.
        instance: InstanceRef,
        /// Existing root whose Closing scope already covers it.
        scope: InstanceRef,
    },
    /// `l:stop` was attempted before a durable close request existed.
    StopWithoutClose {
        /// Generation whose lifecycle was truncated.
        instance: InstanceRef,
    },
    /// A call ref matching no journaled call.
    UnknownCall,
    /// An inbound Call marker contradicts its root or source instance.
    CorruptInboundCall,
    /// The same inbound Call root was opened more than once.
    DuplicateInboundCall,
    /// A transaction names no open inbound Call.
    UnknownInboundCall,
    /// An inbound Call terminal transaction omitted, repeated, or followed
    /// its conventional `exit` emission.
    InvalidInboundCallExit,
    /// An intent whose call ref is already journaled.
    DuplicateOpen,
    /// An intent whose call ref does not derive from its own
    /// canonical fields — a forged or corrupted causal fact.
    CorruptIntent,
    /// A session opening ref does not derive from its canonical fields.
    CorruptSessionOpen,
    /// A session write ref does not derive from its canonical fields.
    CorruptSessionWrite,
    /// A session ref is unknown or no longer live.
    UnknownSession,
    /// An opening attempted to reuse a session ref already present in the
    /// journal fold.
    DuplicateSessionOpen,
    /// A session write ref has no matching open fact.
    UnknownSessionWrite,
    /// A session write was opened twice or settled more than once.
    DuplicateSessionWriteTransition,
    /// A terminal was attempted while one or more message writes were still
    /// causally open.
    OpenSessionWrites,
    /// A session terminal transaction omitted, repeated, or followed its
    /// conventional `exit` emission.
    InvalidSessionExit,
    /// A session terminal envelope omitted or contradicted the logical
    /// `meta.ext.session_id` projection owned by its `SessionRef`.
    SessionProjectionMismatch,
    /// The backing store failed to commit — the durable-journal
    /// escape hatch; the in-memory journal never raises it.
    Storage(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SequenceMismatch { expected, got } => {
                write!(f, "expected inbound seq {expected}, got {got}")
            }
            Self::SequenceOverflow => f.write_str("the inbound sequence range is exhausted"),
            Self::InstanceGenerationMismatch { expected, got } => write!(
                f,
                "expected instance generation {} for address {:?}, got {} for {:?}",
                expected.generation(),
                expected.address(),
                got.generation(),
                got.address()
            ),
            Self::InstanceOwnerMismatch { expected, got } => {
                write!(f, "expected transaction owner `{expected}`, got `{got}`")
            }
            Self::InvalidInstanceLifecycle { instance, active } => write!(
                f,
                "instance generation {} for address {:?} is {}",
                instance.generation(),
                instance.address(),
                if *active {
                    "already active"
                } else {
                    "not active"
                }
            ),
            Self::InvalidParentInstance { instance, parent } => write!(
                f,
                "instance generation {} for owner `{}` has invalid lifecycle parent {:?}",
                instance.generation(),
                instance.address().owner(),
                parent
            ),
            Self::OpenChildInstances { parent } => write!(
                f,
                "instance generation {} for owner `{}` still has active children",
                parent.generation(),
                parent.address().owner()
            ),
            Self::InvalidCloseScope { instance, scope } => write!(
                f,
                "instance {:?} cannot close unrelated scope {:?}",
                instance, scope
            ),
            Self::InstanceAlreadyClosing { instance, scope } => write!(
                f,
                "instance {:?} is already governed by Closing scope {:?}",
                instance, scope
            ),
            Self::StopWithoutClose { instance } => write!(
                f,
                "instance generation {} for owner `{}` cannot stop before Closing was requested",
                instance.generation(),
                instance.address().owner()
            ),
            Self::UnknownCall => f.write_str("no outbound record for this call"),
            Self::CorruptInboundCall => {
                f.write_str("the inbound Call marker contradicts its committed root or source")
            }
            Self::DuplicateInboundCall => {
                f.write_str("the inbound Call is already present in the journal fold")
            }
            Self::UnknownInboundCall => f.write_str("no open inbound Call has this reference"),
            Self::InvalidInboundCallExit => f.write_str(
                "an inbound Call terminal transaction requires exactly one final `exit` emission",
            ),
            Self::DuplicateOpen => f.write_str("this call is already journaled"),
            Self::CorruptIntent => {
                f.write_str("the call ref does not derive from the intent's canonical fields")
            }
            Self::CorruptSessionOpen => f.write_str(
                "the session ref does not derive from the opening intent's canonical fields",
            ),
            Self::CorruptSessionWrite => f.write_str(
                "the session write ref does not derive from the intent's canonical fields",
            ),
            Self::UnknownSession => f.write_str("the session is unknown or already terminal"),
            Self::DuplicateSessionOpen => f.write_str("the session opening is already journaled"),
            Self::UnknownSessionWrite => f.write_str("no open session write has this reference"),
            Self::DuplicateSessionWriteTransition => {
                f.write_str("the session write already made this lifecycle transition")
            }
            Self::OpenSessionWrites => {
                f.write_str("the session still has unsettled message writes")
            }
            Self::InvalidSessionExit => f.write_str(
                "a session terminal transaction requires exactly one final `exit` emission",
            ),
            Self::SessionProjectionMismatch => f.write_str(
                "a session terminal envelope must project its logical `session_id` exactly",
            ),
            Self::Storage(detail) => write!(f, "the journal store failed: {detail}"),
        }
    }
}

impl core::error::Error for Error {}

/// Semantic journal operations and queries — never bare appends, and
/// no exposure of a physical record layout: a durable implementation
/// may hold anything on disk as long as it answers these questions.
pub trait Journal {
    /// Mints the complete ingress envelope without committing it. The result
    /// is a read-only reservation: only [`Self::commit_inbound`] consumes the
    /// sequence.
    ///
    /// # Errors
    ///
    /// Sequence mismatch or exhaustion, as for [`Self::append_inbound`].
    fn prepare_inbound(&self, submission: InboundSubmission) -> Result<PreparedInbound, Error>;

    /// Mints a Connector-origin root without committing it. The source node
    /// is derived from the typed provenance and recorded separately so replay
    /// can route from the same output endpoint.
    ///
    /// # Errors
    ///
    /// The same sequence failures as [`Self::prepare_inbound`], plus Session
    /// lifecycle refusals when `session` carries framing.
    fn prepare_connector_inbound(
        &self,
        submission: InboundSubmission,
        session: Option<InboundSession>,
    ) -> Result<PreparedInbound, Error>;

    /// Mints one session opening/message envelope without committing it. The
    /// journal derives or verifies the logical session ref and overwrites the
    /// reserved `meta.ext.session_id` projection.
    ///
    /// # Errors
    ///
    /// The same sequence failures as [`Self::prepare_inbound`], or an unknown,
    /// terminal, or duplicate session transition.
    fn prepare_session_inbound(
        &self,
        submission: InboundSubmission,
        session: InboundSession,
    ) -> Result<PreparedInbound, Error>;

    /// Atomically commits one prepared ingress and its destination routes.
    /// A durable implementation must compare-and-advance the prepared
    /// sequence in the same transaction as the record.
    ///
    /// # Errors
    ///
    /// A stale sequence, a forged instance generation, or storage failure.
    fn commit_inbound(
        &mut self,
        prepared: PreparedInbound,
        routes: Vec<RouteTarget>,
    ) -> Result<CommittedInbound, Error>;

    /// The one accepting transaction of an inbound submission:
    /// allocate or validate the sequence, derive the root id, mint
    /// `meta.sys` (no parent; the envelope is its own origin), build
    /// the envelope, and commit. Only after this returns may the
    /// caller acknowledge the transport — commit-then-ack.
    ///
    /// # Errors
    ///
    /// [`Error::SequenceMismatch`] when an explicit sequence diverges
    /// from the journal; [`Error::SequenceOverflow`] when monotonicity
    /// cannot continue.
    fn append_inbound(&mut self, submission: InboundSubmission) -> Result<CommittedInbound, Error> {
        self.append_inbound_routed(submission, Vec::new())
    }

    /// Accepts an inbound envelope together with every non-singleton target
    /// selected before the fact commits. This is the runtime entry point;
    /// [`Self::append_inbound`] is the route-free convenience used by journal
    /// tests and callers whose plan has no keyed destination.
    fn append_inbound_routed(
        &mut self,
        submission: InboundSubmission,
        routes: Vec<RouteTarget>,
    ) -> Result<CommittedInbound, Error> {
        let prepared = self.prepare_inbound(submission)?;
        self.commit_inbound(prepared, routes)
    }

    /// Returns the generation a route committing now must seal for this
    /// address. The value is a pure fold of journaled `l:start`/`l:stop`
    /// transactions; no Worker-local counter participates.
    fn instance_ref_for_admission(&self, address: &InstanceAddress) -> Result<InstanceRef, Error>;

    /// Commits one atomic transaction record, canonicalizing its
    /// sorted collections. Durable-before-delivered: a failed commit
    /// must be an error, never a silence.
    ///
    /// # Errors
    ///
    /// Implementation faults; the in-memory journal has none today.
    fn commit(&mut self, transaction: TransactionRecord) -> Result<(), Error>;

    /// Journals an outbound intent as open — before the effect
    /// executes, never after.
    ///
    /// # Errors
    ///
    /// [`Error::DuplicateOpen`] when the call is already journaled;
    /// [`Error::CorruptIntent`] when the call ref does not derive from
    /// the intent's own canonical fields.
    fn open_outbound(&mut self, intent: Intent) -> Result<(), Error>;

    /// Journals one outbound session opening before the Driver acts.
    ///
    /// # Errors
    ///
    /// A corrupt or duplicate session reference.
    fn open_outbound_session(&mut self, intent: SessionOpenIntent) -> Result<(), Error>;

    /// Confirms that one outbound opening established a live session.
    ///
    /// # Errors
    ///
    /// The session is unknown or already terminal.
    fn confirm_session_open(&mut self, session: &SessionRef) -> Result<CloseVerdict, Error>;

    /// Journals one session message before the Driver may offer its bytes.
    ///
    /// # Errors
    ///
    /// The session is unavailable or the write identity is corrupt/duplicate.
    fn open_session_write(&mut self, intent: SessionWriteIntent) -> Result<(), Error>;

    /// Settles one local session write obligation. `abandoned` means recovery
    /// cannot prove external visibility and therefore must not replay it.
    ///
    /// # Errors
    ///
    /// The session/write is unknown.
    fn settle_session_write(
        &mut self,
        write_ref: &SessionWriteRef,
        abandoned: bool,
    ) -> Result<CloseVerdict, Error>;

    /// Settles the race to close a call by appending the closure
    /// fact: the first closure wins and becomes causal; every later
    /// one is told [`CloseVerdict::AlreadyClosed`] and must treat its
    /// event as a discarded observation. The open fact is never
    /// rewritten.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownCall`] when no record carries this call ref.
    fn close_outbound(
        &mut self,
        call_ref: &CallRef,
        closure: Closure,
    ) -> Result<CloseVerdict, Error>;

    /// Declares that bounded Closing could not establish an observed
    /// terminal outcome. This participates in the same close-once race as a
    /// response or timeout and appends an `abandoned` fact on success.
    fn abandon_outbound(&mut self, call_ref: &CallRef) -> Result<CloseVerdict, Error>;

    /// The envelope carrying `id`, wherever the history recorded it.
    fn find_envelope(&self, id: Id) -> Option<Envelope>;

    /// The intent of a journaled call, settled or not.
    fn find_call(&self, call_ref: &CallRef) -> Option<Intent>;

    /// The calls whose open fact has no closure fact yet, in open
    /// order.
    fn open_calls(&self) -> Vec<Intent>;

    /// Admitted inbound Calls whose terminal Connector transaction has not
    /// committed, in canonical reference order.
    fn open_inbound_calls(&self) -> Vec<InboundCallIngressRecord>;

    /// Live sessions in canonical reference order.
    fn live_sessions(&self) -> Vec<SessionRef>;

    /// Session message writes whose open fact has no settlement fact.
    fn open_session_writes(&self) -> Vec<SessionWriteIntent>;
}

/// The M0 in-memory journal. It preserves the logical boundaries a
/// durable implementation needs — typed records, per-provenance
/// sequences, append-only closure — with no claim about files.
#[derive(Debug)]
pub struct MemJournal {
    graph_version: GraphVersion,
    records: Vec<Record>,
    next_seq: BTreeMap<InboundEnvelopeProvenance, u64>,
    /// Replay-derived lifecycle fold, maintained atomically with `records`.
    /// It is an index over causal facts, never a second generation authority.
    instance_lifecycles: BTreeMap<InstanceAddress, InstanceLifecycle>,
    /// Active durable close roots. The set is a replay-derived fast rejection:
    /// ordinary admission never walks an owner chain when no close fact exists.
    active_close_roots: BTreeSet<InstanceRef>,
    /// Replay-derived Session fold. It is an index over the three ordinary
    /// causal record kinds, never a fourth record stream.
    sessions: BTreeMap<SessionRef, SessionLifecycle>,
    /// Replay-derived inbound Call fold. The opening INBOUND fact and terminal
    /// TRANSACTION remain the only authorities.
    inbound_calls: BTreeMap<InboundCallRef, InboundCallLifecycle>,
}

#[derive(Debug, Clone)]
struct InboundCallLifecycle {
    connector_path: node::Path,
    instance: InstanceRef,
    timers: Vec<(u8, Id)>,
    open: bool,
}

#[derive(Debug, Clone, Default)]
struct InstanceLifecycle {
    starts: u64,
    active: bool,
    parent: Option<InstanceRef>,
    closing: bool,
}

#[derive(Debug, Clone)]
struct SessionLifecycle {
    connector_path: node::Path,
    phase: SessionPhase,
    writes: BTreeMap<SessionWriteRef, bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionPhase {
    Opening,
    Open,
    Exited,
}

impl MemJournal {
    /// An empty journal stamping envelopes with `graph_version`.
    /// Embedded mode re-seeds the manifest at boot, so simulations
    /// deterministically run as version 1.
    #[must_use]
    pub fn new(graph_version: GraphVersion) -> Self {
        Self {
            graph_version,
            records: Vec::new(),
            next_seq: BTreeMap::new(),
            instance_lifecycles: BTreeMap::new(),
            active_close_roots: BTreeSet::new(),
            sessions: BTreeMap::new(),
            inbound_calls: BTreeMap::new(),
        }
    }

    /// The records in causal order — an implementation detail of the
    /// in-memory journal, for tests and the golden projection; the
    /// trait exposes semantic queries instead.
    #[must_use]
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    fn statuses_of(&self, call_ref: &CallRef) -> impl Iterator<Item = &OutboundStatus> {
        self.records.iter().filter_map(move |record| match record {
            Record::Outbound(OutboundRecord::Call(outbound))
                if outbound.intent.call_ref == *call_ref =>
            {
                Some(&outbound.status)
            }
            _ => None,
        })
    }

    /// Cached projection of the pure lifecycle fold for one logical address.
    /// `starts` is also the next generation ordinal whenever no generation is
    /// active. A durable implementation rebuilds the same projection while
    /// replaying `l:start`/`l:stop`; no Worker-local counter participates.
    fn instance_lifecycle(&self, address: &InstanceAddress) -> InstanceLifecycle {
        self.instance_lifecycles
            .get(address)
            .cloned()
            .unwrap_or_default()
    }

    fn active_instance_ref(&self, address: &InstanceAddress) -> Option<InstanceRef> {
        let lifecycle = self.instance_lifecycle(address);
        lifecycle
            .active
            .then(|| InstanceRef::new(address.clone(), lifecycle.starts.saturating_sub(1)))
    }

    fn instance_is_within(&self, child: &InstanceRef, ancestor: &InstanceRef) -> bool {
        let mut cursor = Some(child.clone());
        while let Some(reference) = cursor {
            if reference == *ancestor {
                return true;
            }
            let lifecycle = self.instance_lifecycle(reference.address());
            if !lifecycle.active || lifecycle.starts.saturating_sub(1) != reference.generation() {
                return false;
            }
            cursor = lifecycle.parent;
        }
        false
    }

    /// Outermost durable close root governing this active generation.
    fn closing_scope(&self, instance: &InstanceRef) -> Option<InstanceRef> {
        if self.active_close_roots.is_empty() {
            return None;
        }
        let mut cursor = Some(instance.clone());
        let mut found = None;
        while let Some(reference) = cursor {
            let lifecycle = self.instance_lifecycle(reference.address());
            if !lifecycle.active || lifecycle.starts.saturating_sub(1) != reference.generation() {
                return found;
            }
            if lifecycle.closing {
                found = Some(reference.clone());
            }
            cursor = lifecycle.parent;
        }
        found
    }

    fn route_generation_is_valid(
        &self,
        source: Option<&InstanceRef>,
        target: &InstanceRef,
    ) -> Result<bool, Error> {
        let admission = self.instance_ref_for_admission(target.address())?;
        if target == &admission {
            return Ok(true);
        }
        let Some(source) = source else {
            return Ok(false);
        };
        let Some(scope) = self.closing_scope(source) else {
            return Ok(false);
        };
        Ok(
            self.active_instance_ref(target.address()).as_ref() == Some(target)
                && self.instance_is_within(source, &scope)
                && self.instance_is_within(target, &scope),
        )
    }

    fn has_active_children(&self, parent: &InstanceRef) -> bool {
        self.instance_lifecycles
            .values()
            .any(|lifecycle| lifecycle.active && lifecycle.parent.as_ref() == Some(parent))
    }

    fn route_was_reserved(&self, target: &InstanceRef) -> bool {
        self.records.iter().any(|record| match record {
            Record::Inbound(inbound) => {
                inbound.routes.iter().any(|route| &route.target == target)
                    || matches!(
                        inbound.exchange.as_deref(),
                        Some(InboundExchangeRecord::Call(call)) if &call.instance == target
                    )
            }
            Record::Transaction(transaction) => transaction
                .routes
                .iter()
                .any(|route| &route.target == target),
            Record::Outbound(_) => false,
        })
    }

    fn prepare_inbound_fact(
        &self,
        submission: InboundSubmission,
        session: Option<InboundSession>,
        connector_path: Option<node::Path>,
    ) -> Result<PreparedInbound, Error> {
        let next = self
            .next_seq
            .get(&submission.provenance)
            .copied()
            .unwrap_or(0);
        let seq = match submission.sequence {
            SubmissionSequence::Next => next,
            SubmissionSequence::Explicit(seq) if seq == next => seq,
            SubmissionSequence::Explicit(seq) => {
                return Err(Error::SequenceMismatch {
                    expected: next,
                    got: seq,
                });
            }
        };
        seq.checked_add(1).ok_or(Error::SequenceOverflow)?;
        let id = ids::root_id(&submission.provenance, seq);
        let resolved_exchange = match session {
            None => None,
            Some(InboundSession::Open) => {
                let reference = ids::inbound_session_ref(&submission.provenance, id);
                if self.sessions.contains_key(&reference) {
                    return Err(Error::DuplicateSessionOpen);
                }
                Some(InboundExchangeRecord::Session(SessionIngressRecord::Open(
                    reference,
                )))
            }
            Some(InboundSession::Message(reference)) => {
                let Some(lifecycle) = self.sessions.get(&reference) else {
                    return Err(Error::UnknownSession);
                };
                if lifecycle.phase != SessionPhase::Open
                    || lifecycle.connector_path != *submission.provenance.connector_path()
                {
                    return Err(Error::UnknownSession);
                }
                Some(InboundExchangeRecord::Session(
                    SessionIngressRecord::Message(reference),
                ))
            }
        };
        let mut ext = submission.draft.ext.clone();
        if let Some(InboundExchangeRecord::Session(session)) = &resolved_exchange {
            let reference = match *session {
                SessionIngressRecord::Open(reference)
                | SessionIngressRecord::Message(reference) => reference,
            };
            ext.insert(
                String::from("session_id"),
                crate::envelope::MetaValue::Scalar(crate::envelope::Scalar::Text(
                    reference.to_string(),
                )),
            );
        }
        let sys = MetaSys::root(id, self.graph_version.0);
        let envelope = Envelope::new(Meta::new(sys).with_ext(ext), submission.draft.data.clone());
        Ok(PreparedInbound {
            submission,
            connector_path,
            seq,
            envelope,
            exchange: resolved_exchange,
        })
    }
}

impl Journal for MemJournal {
    fn prepare_inbound(&self, submission: InboundSubmission) -> Result<PreparedInbound, Error> {
        self.prepare_inbound_fact(submission, None, None)
    }

    fn prepare_connector_inbound(
        &self,
        submission: InboundSubmission,
        session: Option<InboundSession>,
    ) -> Result<PreparedInbound, Error> {
        let connector_path = submission.provenance.connector_path().clone();
        self.prepare_inbound_fact(submission, session, Some(connector_path))
    }

    fn prepare_session_inbound(
        &self,
        submission: InboundSubmission,
        session: InboundSession,
    ) -> Result<PreparedInbound, Error> {
        self.prepare_inbound_fact(submission, Some(session), None)
    }

    fn commit_inbound(
        &mut self,
        prepared: PreparedInbound,
        mut routes: Vec<RouteTarget>,
    ) -> Result<CommittedInbound, Error> {
        let expected = self
            .next_seq
            .get(&prepared.submission.provenance)
            .copied()
            .unwrap_or(0);
        if prepared.seq != expected {
            return Err(Error::SequenceMismatch {
                expected,
                got: prepared.seq,
            });
        }
        let following = prepared.seq.checked_add(1).ok_or(Error::SequenceOverflow)?;
        match prepared.exchange.as_ref() {
            Some(InboundExchangeRecord::Session(SessionIngressRecord::Open(reference))) => {
                if self.sessions.contains_key(reference) {
                    return Err(Error::DuplicateSessionOpen);
                }
            }
            Some(InboundExchangeRecord::Session(SessionIngressRecord::Message(reference))) => {
                let Some(lifecycle) = self.sessions.get(reference) else {
                    return Err(Error::UnknownSession);
                };
                if lifecycle.phase != SessionPhase::Open
                    || lifecycle.connector_path != *prepared.submission.provenance.connector_path()
                {
                    return Err(Error::UnknownSession);
                }
            }
            Some(InboundExchangeRecord::Call(call)) => {
                let root = prepared.envelope.meta().sys().id();
                let Some(connector_path) = prepared.connector_path.as_ref() else {
                    return Err(Error::CorruptInboundCall);
                };
                if call.reference != InboundCallRef::from_root(root)
                    || call.instance.address().owner().segments() != connector_path.segments()
                {
                    return Err(Error::CorruptInboundCall);
                }
                if self.inbound_calls.contains_key(&call.reference) {
                    return Err(Error::DuplicateInboundCall);
                }
                let expected_timeout = (timer_kind::INBOUND_CALL_TIMEOUT, call.reference.id());
                if !(call.timers.is_empty() || call.timers.as_slice() == [expected_timeout]) {
                    return Err(Error::CorruptInboundCall);
                }
                let expected = self.instance_ref_for_admission(call.instance.address())?;
                if call.instance != expected {
                    return Err(Error::InstanceGenerationMismatch {
                        expected,
                        got: call.instance.clone(),
                    });
                }
            }
            None => {}
        }
        for route in &routes {
            let expected = self.instance_ref_for_admission(route.target.address())?;
            if route.target != expected {
                return Err(Error::InstanceGenerationMismatch {
                    expected,
                    got: route.target.clone(),
                });
            }
        }
        routes.sort();
        let record = InboundRecord {
            provenance: prepared.submission.provenance.clone(),
            port: prepared.submission.port,
            connector_path: prepared.connector_path,
            seq: prepared.seq,
            envelope: prepared.envelope.clone(),
            routes,
            // Exchange framing is absent on Event records. Box the extension
            // so `None` does not widen every causal `Record` retained by an
            // embedded journal.
            exchange: prepared.exchange.clone().map(Box::new),
        };
        match prepared.exchange {
            Some(InboundExchangeRecord::Session(SessionIngressRecord::Open(reference))) => {
                self.sessions.insert(
                    reference,
                    SessionLifecycle {
                        connector_path: prepared.submission.provenance.connector_path().clone(),
                        phase: SessionPhase::Open,
                        writes: BTreeMap::new(),
                    },
                );
            }
            Some(InboundExchangeRecord::Call(call)) => {
                self.inbound_calls.insert(
                    call.reference,
                    InboundCallLifecycle {
                        connector_path: prepared.submission.provenance.connector_path().clone(),
                        instance: call.instance,
                        timers: call.timers,
                        open: true,
                    },
                );
            }
            Some(InboundExchangeRecord::Session(SessionIngressRecord::Message(_))) | None => {}
        }
        self.next_seq
            .insert(prepared.submission.provenance, following);
        self.records.push(Record::Inbound(record.clone()));
        Ok(CommittedInbound {
            seq: prepared.seq,
            envelope: prepared.envelope,
            record,
        })
    }

    fn commit(&mut self, mut transaction: TransactionRecord) -> Result<(), Error> {
        let expected_owner = transaction.instance.address().owner();
        if &transaction.owner_path != expected_owner {
            return Err(Error::InstanceOwnerMismatch {
                expected: expected_owner.clone(),
                got: transaction.owner_path,
            });
        }
        let is_start = matches!(
            transaction.trigger,
            node::Trigger::Lifecycle(node::Lifecycle::Start)
        );
        let is_stop = matches!(
            transaction.trigger,
            node::Trigger::Lifecycle(node::Lifecycle::Stop)
        );
        let is_reply = matches!(
            transaction.trigger,
            node::Trigger::Lifecycle(node::Lifecycle::Reply)
        );
        let is_session_exit = matches!(
            transaction.trigger,
            node::Trigger::Lifecycle(node::Lifecycle::Exit)
        );
        if is_reply
            != matches!(
                transaction.exchange_exit.as_deref(),
                Some(InboundExchangeExitRecord::Call(_))
            )
        {
            return Err(Error::InvalidInboundCallExit);
        }
        if is_session_exit
            != matches!(
                transaction.exchange_exit.as_deref(),
                Some(InboundExchangeExitRecord::Session(_))
            )
        {
            return Err(Error::InvalidSessionExit);
        }
        let mut lifecycle = self.instance_lifecycle(transaction.instance.address());
        let expected = if is_start {
            InstanceRef::new(transaction.instance.address().clone(), lifecycle.starts)
        } else {
            InstanceRef::new(
                transaction.instance.address().clone(),
                lifecycle.starts.saturating_sub(1),
            )
        };
        if transaction.instance != expected {
            return Err(Error::InstanceGenerationMismatch {
                expected,
                got: transaction.instance,
            });
        }
        if (is_start && lifecycle.active) || (!is_start && !lifecycle.active) {
            return Err(Error::InvalidInstanceLifecycle {
                instance: transaction.instance,
                active: lifecycle.active,
            });
        }
        if let Some(scope) = &transaction.close_scope
            && (is_start || is_stop || scope != &transaction.instance)
        {
            return Err(Error::InvalidCloseScope {
                instance: transaction.instance,
                scope: scope.clone(),
            });
        }
        if transaction.close_scope.is_some()
            && let Some(scope) = self.closing_scope(&transaction.instance)
        {
            return Err(Error::InstanceAlreadyClosing {
                instance: transaction.instance,
                scope,
            });
        }
        if is_stop && self.closing_scope(&transaction.instance).is_none() {
            return Err(Error::StopWithoutClose {
                instance: transaction.instance,
            });
        }
        let child_owner = transaction.instance.address().owner();
        let valid_parent = match (
            &transaction.parent_instance,
            is_start,
            child_owner.is_root(),
        ) {
            (None, true, true) | (None, false, _) => true,
            (Some(parent), true, false) => {
                let parent_owner = parent.address().owner();
                child_owner.segments().starts_with(parent_owner.segments())
                    && child_owner.segments().len() > parent_owner.segments().len()
            }
            _ => false,
        };
        if !valid_parent {
            return Err(Error::InvalidParentInstance {
                instance: transaction.instance,
                parent: transaction.parent_instance,
            });
        }
        if let Some(parent) = &transaction.parent_instance {
            let Some(expected_parent) = self.active_instance_ref(parent.address()) else {
                return Err(Error::InvalidParentInstance {
                    instance: transaction.instance,
                    parent: transaction.parent_instance,
                });
            };
            if parent != &expected_parent {
                return Err(Error::InstanceGenerationMismatch {
                    expected: expected_parent,
                    got: parent.clone(),
                });
            }
            if self.closing_scope(parent).is_some()
                && !self.route_was_reserved(&transaction.instance)
            {
                return Err(Error::InvalidParentInstance {
                    instance: transaction.instance,
                    parent: transaction.parent_instance,
                });
            }
        }
        if matches!(
            transaction.trigger,
            node::Trigger::Lifecycle(node::Lifecycle::Stop)
        ) && self.has_active_children(&transaction.instance)
        {
            return Err(Error::OpenChildInstances {
                parent: transaction.instance,
            });
        }
        for route in &transaction.routes {
            if !self.route_generation_is_valid(Some(&transaction.instance), &route.target)? {
                let expected = self.instance_ref_for_admission(route.target.address())?;
                return Err(Error::InstanceGenerationMismatch {
                    expected,
                    got: route.target.clone(),
                });
            }
        }
        if let Some(InboundExchangeExitRecord::Session(session_ref)) =
            transaction.exchange_exit.as_deref().copied()
        {
            let Some(session) = self.sessions.get(&session_ref) else {
                return Err(Error::UnknownSession);
            };
            if session.phase == SessionPhase::Exited
                || session.connector_path.segments() != transaction.owner_path.segments()
            {
                return Err(Error::UnknownSession);
            }
            if session.writes.values().any(|settled| !settled) {
                return Err(Error::OpenSessionWrites);
            }
            let exits = transaction
                .emissions
                .iter()
                .filter(|emission| emission.port.name() == "exit")
                .count();
            if exits != 1
                || transaction
                    .emissions
                    .last()
                    .is_none_or(|emission| emission.port.name() != "exit")
            {
                return Err(Error::InvalidSessionExit);
            }
            let projected = transaction
                .emissions
                .last()
                .and_then(|emission| emission.envelope.meta().ext().get("session_id"));
            if !matches!(
                projected,
                Some(crate::envelope::MetaValue::Scalar(
                    crate::envelope::Scalar::Text(value)
                )) if value == &session_ref.to_string()
            ) {
                return Err(Error::SessionProjectionMismatch);
            }
        }
        if let Some(InboundExchangeExitRecord::Call(call_ref)) =
            transaction.exchange_exit.as_deref().copied()
        {
            let Some(call) = self.inbound_calls.get(&call_ref) else {
                return Err(Error::UnknownInboundCall);
            };
            if !call.open
                || call.connector_path.segments() != transaction.owner_path.segments()
                || call.instance != transaction.instance
            {
                return Err(Error::UnknownInboundCall);
            }
            if transaction.trigger != node::Trigger::Lifecycle(node::Lifecycle::Reply)
                || transaction.cause != Some(call_ref.id())
                || transaction.consumed.len() > 1
            {
                return Err(Error::InvalidInboundCallExit);
            }
            let exits = transaction
                .emissions
                .iter()
                .filter(|emission| emission.port.name() == "exit")
                .count();
            if exits != 1
                || transaction
                    .emissions
                    .last()
                    .is_none_or(|emission| emission.port.name() != "exit")
            {
                return Err(Error::InvalidInboundCallExit);
            }
        }
        // Canonicalize the declaratively sorted collections so an
        // invalid ordering is not constructible through the journal.
        transaction.consumed.sort();
        transaction.state_delta.sort_by(|a, b| a.0.cmp(&b.0));
        transaction.routes.sort();
        transaction.timers.sort();
        if is_start {
            lifecycle.starts = lifecycle
                .starts
                .checked_add(1)
                .ok_or(Error::SequenceOverflow)?;
            lifecycle.active = true;
            lifecycle.parent = transaction.parent_instance.clone();
            lifecycle.closing = false;
        } else if is_stop {
            self.active_close_roots.remove(&transaction.instance);
            lifecycle.active = false;
            lifecycle.parent = None;
            lifecycle.closing = false;
        } else if transaction.close_scope.is_some() {
            self.active_close_roots.insert(transaction.instance.clone());
            lifecycle.closing = true;
        }
        let address = transaction.instance.address().clone();
        let exchange_exit = transaction.exchange_exit.as_deref().copied();
        self.records.push(Record::Transaction(transaction));
        self.instance_lifecycles.insert(address, lifecycle);
        if let Some(InboundExchangeExitRecord::Session(session_ref)) = exchange_exit
            && let Some(session) = self.sessions.get_mut(&session_ref)
        {
            session.phase = SessionPhase::Exited;
        }
        if let Some(InboundExchangeExitRecord::Call(call_ref)) = exchange_exit
            && let Some(call) = self.inbound_calls.get_mut(&call_ref)
        {
            call.open = false;
        }
        Ok(())
    }

    fn instance_ref_for_admission(&self, address: &InstanceAddress) -> Result<InstanceRef, Error> {
        let lifecycle = self.instance_lifecycle(address);
        let closing = lifecycle.active
            && !self.active_close_roots.is_empty()
            && self
                .closing_scope(&InstanceRef::new(
                    address.clone(),
                    lifecycle.starts.saturating_sub(1),
                ))
                .is_some();
        let generation = if lifecycle.active && !closing {
            lifecycle.starts.saturating_sub(1)
        } else {
            lifecycle.starts
        };
        Ok(InstanceRef::new(address.clone(), generation))
    }

    fn open_outbound(&mut self, intent: Intent) -> Result<(), Error> {
        // The autonomy check: the call identity must recompute from
        // the fields this record seals (keys 40, 43, 47).
        let derived = ids::call_ref(
            ids::delivery_ref(&intent.edge, intent.emitting_id),
            intent.request_digest(),
        );
        if derived != intent.call_ref {
            return Err(Error::CorruptIntent);
        }
        if self.statuses_of(&intent.call_ref).next().is_some() {
            return Err(Error::DuplicateOpen);
        }
        self.records
            .push(Record::Outbound(OutboundRecord::Call(CallRecord {
                intent,
                status: OutboundStatus::Open,
            })));
        Ok(())
    }

    fn open_outbound_session(&mut self, intent: SessionOpenIntent) -> Result<(), Error> {
        let derived = ids::outbound_session_ref(
            &intent.connector_path,
            intent.emitting_id,
            intent.request_digest(),
        );
        if derived != intent.session_ref {
            return Err(Error::CorruptSessionOpen);
        }
        if self.sessions.contains_key(&intent.session_ref) {
            return Err(Error::DuplicateSessionOpen);
        }
        let reference = intent.session_ref;
        let connector_path = intent.connector_path.clone();
        self.records
            .push(Record::Outbound(OutboundRecord::Session(SessionRecord {
                connector_path: connector_path.clone(),
                session_ref: reference,
                status: SessionStatus::Opening(intent),
            })));
        self.sessions.insert(
            reference,
            SessionLifecycle {
                connector_path,
                phase: SessionPhase::Opening,
                writes: BTreeMap::new(),
            },
        );
        Ok(())
    }

    fn confirm_session_open(&mut self, session_ref: &SessionRef) -> Result<CloseVerdict, Error> {
        let Some(session) = self.sessions.get_mut(session_ref) else {
            return Err(Error::UnknownSession);
        };
        match session.phase {
            SessionPhase::Opening => session.phase = SessionPhase::Open,
            SessionPhase::Open => return Ok(CloseVerdict::AlreadyClosed),
            SessionPhase::Exited => return Err(Error::UnknownSession),
        }
        let connector_path = session.connector_path.clone();
        self.records
            .push(Record::Outbound(OutboundRecord::Session(SessionRecord {
                connector_path,
                session_ref: *session_ref,
                status: SessionStatus::Opened,
            })));
        Ok(CloseVerdict::Won)
    }

    fn open_session_write(&mut self, intent: SessionWriteIntent) -> Result<(), Error> {
        let derived = ids::session_write_ref(
            intent.session_ref,
            intent.emitting_id,
            intent.message_digest(),
        );
        if derived != intent.write_ref {
            return Err(Error::CorruptSessionWrite);
        }
        let Some(session) = self.sessions.get_mut(&intent.session_ref) else {
            return Err(Error::UnknownSession);
        };
        if session.phase != SessionPhase::Open || session.connector_path != intent.connector_path {
            return Err(Error::UnknownSession);
        }
        if session.writes.contains_key(&intent.write_ref) {
            return Err(Error::DuplicateSessionWriteTransition);
        }
        session.writes.insert(intent.write_ref, false);
        self.records
            .push(Record::Outbound(OutboundRecord::Session(SessionRecord {
                connector_path: intent.connector_path.clone(),
                session_ref: intent.session_ref,
                status: SessionStatus::WriteOpen(intent),
            })));
        Ok(())
    }

    fn settle_session_write(
        &mut self,
        write_ref: &SessionWriteRef,
        abandoned: bool,
    ) -> Result<CloseVerdict, Error> {
        let Some((session_ref, session)) = self
            .sessions
            .iter_mut()
            .find(|(_, session)| session.writes.contains_key(write_ref))
        else {
            return Err(Error::UnknownSessionWrite);
        };
        let Some(settled) = session.writes.get_mut(write_ref) else {
            return Err(Error::UnknownSessionWrite);
        };
        if *settled {
            return Ok(CloseVerdict::AlreadyClosed);
        }
        *settled = true;
        let status = if abandoned {
            SessionStatus::WriteAbandoned(*write_ref)
        } else {
            SessionStatus::WriteSettled(*write_ref)
        };
        self.records
            .push(Record::Outbound(OutboundRecord::Session(SessionRecord {
                connector_path: session.connector_path.clone(),
                session_ref: *session_ref,
                status,
            })));
        Ok(CloseVerdict::Won)
    }

    fn close_outbound(
        &mut self,
        call_ref: &CallRef,
        closure: Closure,
    ) -> Result<CloseVerdict, Error> {
        let mut intent = None;
        for record in &self.records {
            let Record::Outbound(OutboundRecord::Call(outbound)) = record else {
                continue;
            };
            if outbound.intent.call_ref != *call_ref {
                continue;
            }
            match outbound.status {
                OutboundStatus::Open => intent = Some(outbound.intent.clone()),
                OutboundStatus::Closed(_) | OutboundStatus::Abandoned => {
                    return Ok(CloseVerdict::AlreadyClosed);
                }
            }
        }
        let Some(intent) = intent else {
            return Err(Error::UnknownCall);
        };
        // Append the closure as a new fact; the open fact stays
        // byte-identical — the attempt trace survives.
        self.records
            .push(Record::Outbound(OutboundRecord::Call(CallRecord {
                intent,
                status: OutboundStatus::Closed(closure),
            })));
        Ok(CloseVerdict::Won)
    }

    fn abandon_outbound(&mut self, call_ref: &CallRef) -> Result<CloseVerdict, Error> {
        let mut intent = None;
        for record in &self.records {
            let Record::Outbound(OutboundRecord::Call(outbound)) = record else {
                continue;
            };
            if outbound.intent.call_ref != *call_ref {
                continue;
            }
            match outbound.status {
                OutboundStatus::Open => intent = Some(outbound.intent.clone()),
                OutboundStatus::Closed(_) | OutboundStatus::Abandoned => {
                    return Ok(CloseVerdict::AlreadyClosed);
                }
            }
        }
        let Some(intent) = intent else {
            return Err(Error::UnknownCall);
        };
        self.records
            .push(Record::Outbound(OutboundRecord::Call(CallRecord {
                intent,
                status: OutboundStatus::Abandoned,
            })));
        Ok(CloseVerdict::Won)
    }

    fn find_envelope(&self, id: Id) -> Option<Envelope> {
        self.records.iter().find_map(|record| match record {
            Record::Inbound(inbound) if inbound.envelope.meta().sys().id() == id => {
                Some(inbound.envelope.clone())
            }
            Record::Transaction(transaction) => transaction
                .emissions
                .iter()
                .map(|emission| &emission.envelope)
                .find(|envelope| envelope.meta().sys().id() == id)
                .cloned(),
            _ => None,
        })
    }

    fn find_call(&self, call_ref: &CallRef) -> Option<Intent> {
        self.records.iter().find_map(|record| match record {
            Record::Outbound(OutboundRecord::Call(outbound))
                if outbound.intent.call_ref == *call_ref =>
            {
                Some(outbound.intent.clone())
            }
            _ => None,
        })
    }

    fn open_calls(&self) -> Vec<Intent> {
        let mut open: Vec<Intent> = Vec::new();
        for record in &self.records {
            let Record::Outbound(OutboundRecord::Call(outbound)) = record else {
                continue;
            };
            match outbound.status {
                OutboundStatus::Open => open.push(outbound.intent.clone()),
                OutboundStatus::Closed(_) | OutboundStatus::Abandoned => {
                    open.retain(|intent| intent.call_ref != outbound.intent.call_ref);
                }
            }
        }
        open
    }

    fn open_inbound_calls(&self) -> Vec<InboundCallIngressRecord> {
        self.inbound_calls
            .iter()
            .filter(|(_, lifecycle)| lifecycle.open)
            .map(|(reference, lifecycle)| InboundCallIngressRecord {
                reference: *reference,
                instance: lifecycle.instance.clone(),
                timers: lifecycle.timers.clone(),
            })
            .collect()
    }

    fn live_sessions(&self) -> Vec<SessionRef> {
        self.sessions
            .iter()
            .filter_map(|(reference, lifecycle)| {
                (lifecycle.phase == SessionPhase::Open).then_some(*reference)
            })
            .collect()
    }

    fn open_session_writes(&self) -> Vec<SessionWriteIntent> {
        let open: BTreeSet<_> = self
            .sessions
            .values()
            .flat_map(|session| {
                session
                    .writes
                    .iter()
                    .filter_map(|(reference, settled)| (!settled).then_some(*reference))
            })
            .collect();
        self.records
            .iter()
            .filter_map(|record| match record {
                Record::Outbound(OutboundRecord::Session(SessionRecord {
                    status: SessionStatus::WriteOpen(intent),
                    ..
                })) if open.contains(&intent.write_ref) => Some(intent.clone()),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::Draft;
    use crate::envelope::{Data, Encoding, Namespace};
    use alloc::format;
    use alloc::string::ToString;
    use alloc::vec;

    fn instance(owner: &str, generation: u64) -> InstanceRef {
        let owner = crate::plan::OwnerPath::new([owner]).expect("valid owner path");
        InstanceRef::new(InstanceAddress::singleton(&owner), generation)
    }

    fn provenance(path: &str) -> InboundEnvelopeProvenance {
        InboundEnvelopeProvenance::single(node::Path::new([path]).unwrap())
    }

    fn submission(path: &str, sequence: SubmissionSequence) -> InboundSubmission {
        InboundSubmission {
            provenance: provenance(path),
            port: node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::encoded(Encoding::Raw, *b"hello"),
                ext: {
                    let mut ext = Namespace::new();
                    ext.insert(
                        String::from("content_type"),
                        crate::envelope::MetaValue::Scalar(crate::envelope::Scalar::Text(
                            String::from("text/plain"),
                        )),
                    );
                    ext
                },
            },
            sequence,
        }
    }

    fn project_session(envelope: &Envelope, session: SessionRef) -> Envelope {
        let mut ext = envelope.meta().ext().clone();
        ext.insert(
            String::from("session_id"),
            crate::envelope::MetaValue::Scalar(crate::envelope::Scalar::Text(session.to_string())),
        );
        Envelope::new(
            Meta::new(envelope.meta().sys().clone())
                .with_app(envelope.meta().app().clone())
                .with_ext(ext),
            envelope.data().clone(),
        )
    }

    fn intent(connector: &str, digest: u8) -> Intent {
        let connector_path = node::Path::new([connector]).unwrap();
        let emitting_id = Id::derive(0x02, &[b"cause"]);
        let edge = alloc::format!("shout.out>{connector}.in");
        let prepared = crate::boundary::PreparedRequest::seal(vec![digest; 4]);
        Intent {
            call_ref: ids::call_ref(ids::delivery_ref(&edge, emitting_id), prepared.digest()),
            emitting_id,
            edge,
            connector_path,
            prepared,
            idempotency_key: Some(String::from("op-1")),
            effect: node::EffectClass::Idempotent,
            resolved_settings: BTreeMap::new(),
        }
    }

    fn session_open(connector: &str, digest: u8) -> SessionOpenIntent {
        let connector_path = node::Path::new([connector]).unwrap();
        let emitting_id = Id::derive(0x02, &[b"session-cause", &[digest]]);
        let prepared = crate::boundary::PreparedRequest::seal(vec![digest; 4]);
        SessionOpenIntent {
            session_ref: ids::outbound_session_ref(&connector_path, emitting_id, prepared.digest()),
            emitting_id,
            connector_path,
            prepared,
        }
    }

    fn session_write(session_ref: SessionRef, connector: &str, digest: u8) -> SessionWriteIntent {
        let connector_path = node::Path::new([connector]).unwrap();
        let emitting_id = Id::derive(0x02, &[b"message-cause", &[digest]]);
        let prepared = crate::boundary::PreparedRequest::seal(vec![digest; 3]);
        SessionWriteIntent {
            write_ref: ids::session_write_ref(session_ref, emitting_id, prepared.digest()),
            session_ref,
            emitting_id,
            connector_path,
            prepared,
        }
    }

    fn lifecycle_transaction(
        instance: InstanceRef,
        lifecycle: node::Lifecycle,
    ) -> TransactionRecord {
        let parent_instance = (lifecycle == node::Lifecycle::Start
            && !instance.address().owner().is_root())
        .then(|| {
            InstanceRef::new(
                InstanceAddress::singleton(&crate::plan::OwnerPath::root()),
                0,
            )
        });
        TransactionRecord {
            owner_path: instance.address().owner().clone(),
            instance,
            parent_instance,
            close_scope: None,
            exchange_exit: None,
            trigger: node::Trigger::Lifecycle(lifecycle),
            cause: None,
            consumed: vec![],
            join_inputs: vec![],
            state_delta: vec![],
            emissions: vec![],
            routes: vec![],
            timers: vec![],
        }
    }

    fn activate_root(journal: &mut MemJournal) {
        let root = InstanceRef::new(
            InstanceAddress::singleton(&crate::plan::OwnerPath::root()),
            0,
        );
        journal
            .commit(lifecycle_transaction(root, node::Lifecycle::Start))
            .expect("the root lifecycle starts");
    }

    fn activate(journal: &mut MemJournal, reference: InstanceRef) {
        activate_root(journal);
        journal
            .commit(lifecycle_transaction(reference, node::Lifecycle::Start))
            .expect("the child lifecycle starts under the root");
    }

    fn request_close(journal: &mut MemJournal, reference: InstanceRef) {
        let mut record = lifecycle_transaction(reference.clone(), node::Lifecycle::Closing);
        record.close_scope = Some(reference);
        journal.commit(record).expect("Closing is durable");
    }

    #[test]
    fn generations_are_a_journal_fold_and_lifecycle_cannot_double_transition() {
        let mut journal = MemJournal::new(GraphVersion(1));
        let address = InstanceAddress::singleton(
            &crate::plan::OwnerPath::new(["worker"]).expect("valid owner path"),
        );
        let first = InstanceRef::new(address.clone(), 0);
        assert_eq!(journal.instance_ref_for_admission(&address).unwrap(), first);
        activate_root(&mut journal);

        let mut wrong_owner = lifecycle_transaction(first.clone(), node::Lifecycle::Start);
        wrong_owner.owner_path = OwnerPath::new(["other"]).unwrap();
        assert_eq!(
            journal.commit(wrong_owner).unwrap_err(),
            Error::InstanceOwnerMismatch {
                expected: OwnerPath::new(["worker"]).unwrap(),
                got: OwnerPath::new(["other"]).unwrap(),
            }
        );

        let mut missing_parent = lifecycle_transaction(first.clone(), node::Lifecycle::Start);
        missing_parent.parent_instance = None;
        assert_eq!(
            journal.commit(missing_parent).unwrap_err(),
            Error::InvalidParentInstance {
                instance: first.clone(),
                parent: None,
            }
        );

        journal
            .commit(lifecycle_transaction(first.clone(), node::Lifecycle::Start))
            .unwrap();
        let root = InstanceRef::new(
            InstanceAddress::singleton(&crate::plan::OwnerPath::root()),
            0,
        );
        request_close(&mut journal, root.clone());
        let mut duplicate_close = lifecycle_transaction(root.clone(), node::Lifecycle::Closing);
        duplicate_close.close_scope = Some(root.clone());
        assert_eq!(
            journal.commit(duplicate_close).unwrap_err(),
            Error::InstanceAlreadyClosing {
                instance: root.clone(),
                scope: root.clone(),
            },
            "the first durable close request is the only lifecycle transition"
        );
        let mut nested_close = lifecycle_transaction(first.clone(), node::Lifecycle::Closing);
        nested_close.close_scope = Some(first.clone());
        assert_eq!(
            journal.commit(nested_close).unwrap_err(),
            Error::InstanceAlreadyClosing {
                instance: first.clone(),
                scope: root.clone(),
            },
            "a descendant cannot create a second root inside an active close scope"
        );
        assert_eq!(
            journal
                .commit(lifecycle_transaction(root.clone(), node::Lifecycle::Stop))
                .unwrap_err(),
            Error::OpenChildInstances {
                parent: root.clone(),
            },
            "the journal, not scheduler choreography, enforces inner-first stop"
        );
        let second = InstanceRef::new(address.clone(), 1);
        assert_eq!(
            journal.instance_ref_for_admission(&address).unwrap(),
            second,
            "a close request reserves the clean successor before reclamation"
        );
        assert_eq!(
            journal
                .commit(lifecycle_transaction(first.clone(), node::Lifecycle::Start))
                .unwrap_err(),
            Error::InstanceGenerationMismatch {
                expected: second.clone(),
                got: first.clone(),
            },
            "Closing reserves the successor generation before the current one stops"
        );

        journal
            .commit(lifecycle_transaction(first.clone(), node::Lifecycle::Stop))
            .unwrap();
        assert_eq!(
            journal.instance_ref_for_admission(&address).unwrap(),
            second
        );
        assert_eq!(
            journal
                .commit(lifecycle_transaction(first.clone(), node::Lifecycle::Stop))
                .unwrap_err(),
            Error::InvalidInstanceLifecycle {
                instance: first,
                active: false,
            }
        );
        journal
            .commit(lifecycle_transaction(root.clone(), node::Lifecycle::Stop))
            .unwrap();
        let second_root = InstanceRef::new(root.address().clone(), 1);
        journal
            .commit(lifecycle_transaction(
                second_root.clone(),
                node::Lifecycle::Start,
            ))
            .unwrap();
        let mut second_start = lifecycle_transaction(second.clone(), node::Lifecycle::Start);
        second_start.parent_instance = Some(second_root.clone());
        journal.commit(second_start).unwrap();
        assert_eq!(
            journal.instance_ref_for_admission(&address).unwrap(),
            second
        );
        request_close(&mut journal, second.clone());
        journal
            .commit(lifecycle_transaction(second, node::Lifecycle::Stop))
            .unwrap();
        request_close(&mut journal, second_root.clone());
        journal
            .commit(lifecycle_transaction(second_root, node::Lifecycle::Stop))
            .unwrap();
    }

    #[test]
    fn a_committed_route_keeps_its_reserved_generation_until_start_materializes() {
        let mut journal = MemJournal::new(GraphVersion(1));
        let address = InstanceAddress::singleton(
            &crate::plan::OwnerPath::new(["worker"]).expect("valid owner path"),
        );
        let reserved = InstanceRef::new(address.clone(), 0);
        activate_root(&mut journal);
        let prepared = journal
            .prepare_inbound(submission("inbound", SubmissionSequence::Next))
            .unwrap();
        journal
            .commit_inbound(
                prepared,
                vec![RouteTarget {
                    edge: String::from(".in>worker.in"),
                    target: reserved.clone(),
                }],
            )
            .unwrap();

        assert_eq!(
            journal.instance_ref_for_admission(&address).unwrap(),
            reserved,
            "another reservation joins the same not-yet-polled generation"
        );
        journal
            .commit(lifecycle_transaction(
                reserved.clone(),
                node::Lifecycle::Start,
            ))
            .unwrap();
        assert_eq!(
            journal.instance_ref_for_admission(&address).unwrap(),
            reserved
        );
    }

    #[test]
    fn two_edges_derive_two_calls_for_one_envelope_and_one_payload() {
        // The multiport prerequisite, closed: same envelope, same
        // prepared bytes, two receiving edges — two delivery
        // references, two call identities. Salting the digest was
        // rejected; the reference carries the routing.
        let envelope_id = Id::from_bytes([7; 16]);
        let digest = [3u8; 32];
        let a = ids::call_ref(ids::delivery_ref("src.out>c.a", envelope_id), &digest);
        let b = ids::call_ref(ids::delivery_ref("src.out>c.b", envelope_id), &digest);
        assert_ne!(a, b, "two deliveries, two calls");
    }

    #[test]
    fn an_outbound_record_reconstructs_its_call_ref_autonomously() {
        // The autonomy rule of identity.md §3: keys 40 (emitting),
        // 43 (digest), and 47 (edge) suffice — no other record, no
        // out-of-band state.
        let sealed = intent("echo", 7);
        let reconstructed = ids::call_ref(
            ids::delivery_ref(&sealed.edge, sealed.emitting_id),
            sealed.request_digest(),
        );
        assert_eq!(reconstructed, sealed.call_ref);
    }

    #[test]
    fn the_delivery_ref_recipe_is_pinned_byte_for_byte() {
        // envelope.md §2: the delivery reference is
        // derive(0x20, canonical_edge, envelope_id). Any drift here
        // silently unseals every golden's key 37 and every call ref.
        let delivery = Id::from_bytes([7; 16]);
        let reference = ids::delivery_ref("shout.out>echo.in", delivery);
        assert_eq!(
            reference.to_string(),
            "301cd0af333f760b8f5dc13738b821ec",
            "the pinned obligation identity"
        );
    }

    #[test]
    fn the_instance_lifetime_ref_seals_address_and_generation() {
        let address = InstanceAddress::singleton(
            &crate::plan::OwnerPath::new(["worker"]).expect("valid owner path"),
        );
        let first = ids::instance_lifetime_ref(&InstanceRef::new(address.clone(), 0));
        let second = ids::instance_lifetime_ref(&InstanceRef::new(address, 1));
        assert_eq!(first.to_string(), "d8977f984e5356cc4df3aca36c442642");
        assert_ne!(
            first, second,
            "a stale generation has another obligation id"
        );
    }

    #[test]
    fn the_id_recipes_are_pinned_byte_for_byte() {
        let root = ids::root_id(&provenance("inbound"), 7);
        assert_eq!(root.to_string(), "78cf0ba5304d78c7857726f182eb0706");
        let emission = ids::emission_id(
            &node::Path::new(["shout"]).unwrap(),
            &node::Trigger::Port(node::Port::new("in").unwrap()),
            root,
            &node::Port::new("out").unwrap(),
            0,
        );
        assert_eq!(emission.to_string(), "901b0e4b0eb109c225c8c9e90d2eb723");
    }

    #[test]
    fn the_session_recipes_are_pinned_byte_for_byte() {
        let opening = Id::from_bytes([3; 16]);
        let inbound = ids::inbound_session_ref(&provenance("gateway"), opening);
        let connector = node::Path::new(["chat", "socket"]).unwrap();
        let emitting = Id::from_bytes([4; 16]);
        let digest = [5; 32];
        let outbound = ids::outbound_session_ref(&connector, emitting, &digest);
        let write = ids::session_write_ref(outbound, Id::from_bytes([6; 16]), &[7; 32]);
        assert_eq!(
            format!("{inbound}|{outbound}|{write}"),
            "45e41992da920539378d903e21f963a0|835d72e38dc56aaf373d9613449d7354|\
             19b7714f0076f09221da8c3d9a22f2f8"
        );
    }

    #[test]
    fn the_recipes_separate_every_component() {
        let base = ids::emission_id(
            &node::Path::new(["a"]).unwrap(),
            &node::Trigger::Port(node::Port::new("in").unwrap()),
            Id::from_bytes([1; 16]),
            &node::Port::new("out").unwrap(),
            0,
        );
        let variants = [
            ids::emission_id(
                &node::Path::new(["b"]).unwrap(),
                &node::Trigger::Port(node::Port::new("in").unwrap()),
                Id::from_bytes([1; 16]),
                &node::Port::new("out").unwrap(),
                0,
            ),
            ids::emission_id(
                &node::Path::new(["a"]).unwrap(),
                &node::Trigger::Lifecycle(node::Lifecycle::Start),
                Id::from_bytes([1; 16]),
                &node::Port::new("out").unwrap(),
                0,
            ),
            ids::emission_id(
                &node::Path::new(["a"]).unwrap(),
                &node::Trigger::Port(node::Port::new("in").unwrap()),
                Id::from_bytes([2; 16]),
                &node::Port::new("out").unwrap(),
                0,
            ),
            ids::emission_id(
                &node::Path::new(["a"]).unwrap(),
                &node::Trigger::Port(node::Port::new("in").unwrap()),
                Id::from_bytes([1; 16]),
                &node::Port::new("err").unwrap(),
                0,
            ),
            ids::emission_id(
                &node::Path::new(["a"]).unwrap(),
                &node::Trigger::Port(node::Port::new("in").unwrap()),
                Id::from_bytes([1; 16]),
                &node::Port::new("out").unwrap(),
                1,
            ),
        ];
        for variant in variants {
            assert_ne!(variant, base, "every component must separate ids");
        }
    }

    #[test]
    fn an_accepted_inbound_is_its_own_origin() {
        let mut journal = MemJournal::new(GraphVersion(1));
        let committed = journal
            .append_inbound(submission("inbound", SubmissionSequence::Next))
            .unwrap();
        let sys = committed.envelope.meta().sys();
        assert_eq!(sys.id(), ids::root_id(&provenance("inbound"), 0));
        assert_eq!(sys.parent_id(), None);
        assert_eq!(sys.origin_id(), sys.id());
        assert_eq!(sys.graph_version(), 1);
        assert_eq!(
            committed.envelope.meta().ext().get("content_type"),
            Some(&crate::envelope::MetaValue::Scalar(
                crate::envelope::Scalar::Text(String::from("text/plain"))
            ))
        );
        assert_eq!(committed.seq, 0);
    }

    #[test]
    fn sequences_are_monotone_per_inbound_provenance() {
        let mut journal = MemJournal::new(GraphVersion(1));
        for expected in 0..3 {
            let committed = journal
                .append_inbound(submission("inbound", SubmissionSequence::Next))
                .unwrap();
            assert_eq!(committed.seq, expected);
        }
        let other = journal
            .append_inbound(submission("webhook", SubmissionSequence::Next))
            .unwrap();
        assert_eq!(other.seq, 0, "authorities count independently");
    }

    #[test]
    fn explicit_sequences_replay_or_refuse() {
        let mut journal = MemJournal::new(GraphVersion(1));
        journal
            .append_inbound(submission("inbound", SubmissionSequence::Explicit(0)))
            .unwrap();
        let diverged = journal
            .append_inbound(submission("inbound", SubmissionSequence::Explicit(5)))
            .unwrap_err();
        assert_eq!(
            diverged,
            Error::SequenceMismatch {
                expected: 1,
                got: 5
            }
        );
    }

    #[test]
    fn replay_re_derives_identical_ids() {
        let run = |mut journal: MemJournal| {
            journal
                .append_inbound(submission("inbound", SubmissionSequence::Next))
                .unwrap()
                .envelope
                .meta()
                .sys()
                .id()
        };
        assert_eq!(
            run(MemJournal::new(GraphVersion(1))),
            run(MemJournal::new(GraphVersion(1)))
        );
    }

    #[test]
    fn closing_appends_a_fact_and_never_rewrites_the_open() {
        let mut journal = MemJournal::new(GraphVersion(1));
        let echo_intent = intent("echo", 7);
        let call = echo_intent.call_ref;
        journal.open_outbound(echo_intent.clone()).unwrap();
        let open_before = journal.records()[0].clone();
        assert_eq!(
            journal.close_outbound(&call, Closure::Timeout).unwrap(),
            CloseVerdict::Won
        );
        // The normative sequence: two consecutive facts, open first,
        // byte-identical to what it was before the closure.
        assert_eq!(journal.records().len(), 2);
        assert_eq!(journal.records()[0], open_before);
        let Record::Outbound(OutboundRecord::Call(open)) = &journal.records()[0] else {
            panic!("expected the open fact");
        };
        assert_eq!(open.status, OutboundStatus::Open);
        let Record::Outbound(OutboundRecord::Call(closed)) = &journal.records()[1] else {
            panic!("expected the closure fact");
        };
        assert_eq!(closed.status, OutboundStatus::Closed(Closure::Timeout));
        assert_eq!(closed.intent.call_ref, call);
        // The fold answers "open?" correctly.
        assert!(journal.open_calls().is_empty());
        // The race: every later closure loses and appends nothing.
        assert_eq!(
            journal.close_outbound(&call, Closure::Timeout).unwrap(),
            CloseVerdict::AlreadyClosed
        );
        assert_eq!(journal.records().len(), 2);
        // A closure for an unknown call is a fault, not a silence.
        let ghost = ids::call_ref(
            ids::delivery_ref("x.out>ghost.in", Id::from_bytes([9; 16])),
            &[0; 32],
        );
        assert_eq!(
            journal.close_outbound(&ghost, Closure::Timeout),
            Err(Error::UnknownCall)
        );

        let abandoned_intent = intent("slow", 8);
        let abandoned = abandoned_intent.call_ref;
        journal.open_outbound(abandoned_intent).unwrap();
        assert_eq!(
            journal.abandon_outbound(&abandoned).unwrap(),
            CloseVerdict::Won
        );
        assert_eq!(
            journal
                .close_outbound(&abandoned, Closure::Timeout)
                .unwrap(),
            CloseVerdict::AlreadyClosed,
            "an observed response or timeout cannot supersede abandonment"
        );
        assert_eq!(
            journal.abandon_outbound(&abandoned).unwrap(),
            CloseVerdict::AlreadyClosed
        );
        assert!(matches!(
            journal.records().last(),
            Some(Record::Outbound(OutboundRecord::Call(CallRecord {
                status: OutboundStatus::Abandoned,
                ..
            })))
        ));
    }

    #[test]
    fn a_forged_intent_is_refused() {
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut forged = intent("echo", 7);
        forged.call_ref = ids::call_ref(
            ids::delivery_ref("x.out>other.in", Id::from_bytes([9; 16])),
            &[1; 32],
        );
        assert_eq!(
            journal.open_outbound(forged).unwrap_err(),
            Error::CorruptIntent
        );
    }

    #[test]
    fn commit_canonicalizes_the_sorted_collections() {
        let mut journal = MemJournal::new(GraphVersion(1));
        activate(&mut journal, instance("shout", 0));
        journal
            .commit(TransactionRecord {
                owner_path: OwnerPath::new(["shout"]).unwrap(),
                instance: instance("shout", 0),
                parent_instance: None,
                close_scope: None,
                exchange_exit: None,
                trigger: node::Trigger::Port(node::Port::new("in").unwrap()),
                cause: None,
                consumed: vec![
                    ConsumedPosition {
                        edge: String::from("b"),
                        source: instance("source", 1),
                        seq: 1,
                    },
                    ConsumedPosition {
                        edge: String::from("a"),
                        source: instance("source", 2),
                        seq: 2,
                    },
                ],
                join_inputs: vec![],
                state_delta: vec![
                    (String::from("z"), Value::Int(1)),
                    (String::from("a"), Value::Int(2)),
                ],
                emissions: vec![],
                routes: vec![],
                timers: vec![
                    (timer_kind::LOOP_BACKOFF, Id::from_bytes([2; 16])),
                    (timer_kind::CALL_TIMEOUT, Id::from_bytes([1; 16])),
                ],
            })
            .unwrap();
        let Some(Record::Transaction(transaction)) = journal.records().last() else {
            panic!("expected the transaction");
        };
        assert_eq!(transaction.consumed[0].edge, "a");
        assert_eq!(transaction.state_delta[0].0, "a");
        assert_eq!(transaction.timers[0].0, timer_kind::CALL_TIMEOUT);
    }

    #[test]
    fn records_keep_causal_order() {
        let mut journal = MemJournal::new(GraphVersion(1));
        activate(&mut journal, instance("shout", 0));
        let first_observed = journal.records().len();
        let committed = journal
            .append_inbound(submission("inbound", SubmissionSequence::Next))
            .unwrap();
        journal
            .commit(TransactionRecord {
                owner_path: OwnerPath::new(["shout"]).unwrap(),
                instance: instance("shout", 0),
                parent_instance: None,
                close_scope: None,
                exchange_exit: None,
                trigger: node::Trigger::Port(node::Port::new("in").unwrap()),
                cause: None,
                consumed: vec![ConsumedPosition {
                    edge: String::from(".in>shout.in"),
                    source: instance("root", 0),
                    seq: 0,
                }],
                join_inputs: vec![],
                state_delta: vec![(String::from("count"), Value::Int(1))],
                emissions: vec![Emission {
                    port: node::Port::new("out").unwrap(),
                    index: 0,
                    envelope: committed.envelope.clone(),
                }],
                routes: vec![],
                timers: vec![],
            })
            .unwrap();
        journal.open_outbound(intent("echo", 1)).unwrap();
        let kinds: Vec<u8> = journal
            .records()
            .iter()
            .skip(first_observed)
            .map(|record| match record {
                Record::Inbound(_) => 1,
                Record::Transaction(_) => 2,
                Record::Outbound(_) => 3,
            })
            .collect();
        assert_eq!(kinds, [1, 2, 3]);
        // The semantic queries answer without exposing the slice.
        assert!(
            journal
                .find_envelope(committed.envelope.meta().sys().id())
                .is_some()
        );
        assert!(journal.find_call(&intent("echo", 1).call_ref).is_some());
        assert_eq!(journal.open_calls().len(), 1);
    }

    #[test]
    fn inbound_sessions_are_concurrent_correlated_and_silent_after_exit() {
        let mut journal = MemJournal::new(GraphVersion(1));
        let first = journal
            .prepare_session_inbound(
                submission("socket", SubmissionSequence::Next),
                InboundSession::Open,
            )
            .unwrap();
        let first = journal.commit_inbound(first, vec![]).unwrap();
        let Some(InboundExchangeRecord::Session(SessionIngressRecord::Open(first_ref))) =
            first.record.exchange.as_deref().cloned()
        else {
            panic!("the opening commit carries its session ref")
        };
        assert_eq!(
            first.envelope.meta().ext().get("session_id"),
            Some(&crate::envelope::MetaValue::Scalar(
                crate::envelope::Scalar::Text(first_ref.to_string())
            )),
            "the journal, never the peer, projects session identity"
        );

        let second = journal
            .prepare_session_inbound(
                submission("socket", SubmissionSequence::Next),
                InboundSession::Open,
            )
            .unwrap();
        let second = journal.commit_inbound(second, vec![]).unwrap();
        let Some(InboundExchangeRecord::Session(SessionIngressRecord::Open(second_ref))) =
            second.record.exchange.as_deref().cloned()
        else {
            panic!("the second opening carries its session ref")
        };
        assert_ne!(first_ref, second_ref);

        let message = journal
            .prepare_session_inbound(
                submission("socket", SubmissionSequence::Next),
                InboundSession::Message(first_ref),
            )
            .unwrap();
        let message = journal.commit_inbound(message, vec![]).unwrap();
        assert_eq!(
            message.record.exchange.as_deref().cloned(),
            Some(InboundExchangeRecord::Session(
                SessionIngressRecord::Message(first_ref)
            ))
        );
        assert_eq!(
            message.envelope.meta().ext().get("session_id"),
            Some(&crate::envelope::MetaValue::Scalar(
                crate::envelope::Scalar::Text(first_ref.to_string())
            ))
        );
        assert_eq!(journal.live_sessions(), vec![first_ref, second_ref]);

        activate(&mut journal, instance("socket", 0));
        let mut terminal = lifecycle_transaction(instance("socket", 0), node::Lifecycle::Exit);
        terminal.exchange_exit = Some(Box::new(InboundExchangeExitRecord::Session(first_ref)));
        terminal.emissions.push(Emission {
            port: node::Port::conventional_exit(),
            index: 0,
            envelope: first.envelope,
        });
        journal.commit(terminal).unwrap();
        assert_eq!(journal.live_sessions(), vec![second_ref]);
        assert_eq!(
            journal.prepare_session_inbound(
                submission("socket", SubmissionSequence::Next),
                InboundSession::Message(first_ref),
            ),
            Err(Error::UnknownSession),
            "a terminal session can never speak again"
        );
    }

    #[test]
    fn an_inbound_call_opens_on_its_source_instance_and_closes_once() {
        let mut journal = MemJournal::new(GraphVersion(1));
        let source = instance("server", 0);
        let prepared = journal
            .prepare_connector_inbound(submission("server", SubmissionSequence::Next), None)
            .unwrap()
            .into_call(source.clone(), vec![])
            .unwrap();
        let root = prepared.envelope().meta().sys().id();
        let call_ref = InboundCallRef::from_root(root);
        let committed = journal.commit_inbound(prepared, vec![]).unwrap();
        assert_eq!(
            journal.open_inbound_calls(),
            [InboundCallIngressRecord {
                reference: call_ref,
                instance: source.clone(),
                timers: vec![],
            }]
        );
        assert_eq!(
            committed.record.exchange.as_deref(),
            Some(&InboundExchangeRecord::Call(InboundCallIngressRecord {
                reference: call_ref,
                instance: source.clone(),
                timers: vec![],
            }))
        );

        activate(&mut journal, source.clone());
        let unmatched_reply = lifecycle_transaction(source.clone(), node::Lifecycle::Reply);
        assert_eq!(
            journal.commit(unmatched_reply).unwrap_err(),
            Error::InvalidInboundCallExit
        );
        let mut missing_exit = lifecycle_transaction(source.clone(), node::Lifecycle::Reply);
        missing_exit.exchange_exit = Some(Box::new(InboundExchangeExitRecord::Call(call_ref)));
        missing_exit.cause = Some(root);
        assert_eq!(
            journal.commit(missing_exit).unwrap_err(),
            Error::InvalidInboundCallExit
        );

        let mut terminal = lifecycle_transaction(source.clone(), node::Lifecycle::Reply);
        terminal.exchange_exit = Some(Box::new(InboundExchangeExitRecord::Call(call_ref)));
        terminal.cause = Some(root);
        terminal.emissions.push(Emission {
            port: node::Port::conventional_exit(),
            index: 0,
            envelope: committed.envelope,
        });
        let response_position = ConsumedPosition {
            edge: String::from("relay.out->server.in"),
            source: instance("relay", 0),
            seq: 0,
        };
        let mut two_responses = terminal.clone();
        two_responses.consumed = vec![response_position.clone(), response_position];
        assert_eq!(
            journal.commit(two_responses).unwrap_err(),
            Error::InvalidInboundCallExit,
            "one inbound Call can consume at most one graph response"
        );
        journal.commit(terminal.clone()).unwrap();
        assert!(journal.open_inbound_calls().is_empty());
        assert_eq!(
            journal.commit(terminal).unwrap_err(),
            Error::UnknownInboundCall
        );
    }

    #[test]
    fn session_writes_settle_once_and_exit_only_after_every_write() {
        let mut journal = MemJournal::new(GraphVersion(1));
        let opening = session_open("socket", 1);
        let session = opening.session_ref;
        journal.open_outbound_session(opening).unwrap();
        assert_eq!(
            journal.confirm_session_open(&session).unwrap(),
            CloseVerdict::Won
        );
        assert_eq!(
            journal.confirm_session_open(&session).unwrap(),
            CloseVerdict::AlreadyClosed
        );

        let first = session_write(session, "socket", 2);
        let second = session_write(session, "socket", 3);
        journal.open_session_write(first.clone()).unwrap();
        journal.open_session_write(second.clone()).unwrap();
        assert_eq!(journal.open_session_writes().len(), 2);
        assert_eq!(
            journal
                .settle_session_write(&second.write_ref, false)
                .unwrap(),
            CloseVerdict::Won
        );
        assert_eq!(
            journal
                .settle_session_write(&second.write_ref, false)
                .unwrap(),
            CloseVerdict::AlreadyClosed
        );

        activate(&mut journal, instance("socket", 0));
        let envelope = journal
            .append_inbound(submission("world", SubmissionSequence::Next))
            .unwrap()
            .envelope;
        let mut terminal = lifecycle_transaction(instance("socket", 0), node::Lifecycle::Exit);
        terminal.exchange_exit = Some(Box::new(InboundExchangeExitRecord::Session(session)));
        terminal.emissions.push(Emission {
            port: node::Port::conventional_exit(),
            index: 0,
            envelope: project_session(&envelope, session),
        });
        assert_eq!(
            journal.commit(terminal.clone()),
            Err(Error::OpenSessionWrites)
        );

        assert_eq!(
            journal
                .settle_session_write(&first.write_ref, true)
                .unwrap(),
            CloseVerdict::Won,
            "crash ambiguity becomes an abandoned write fact, never a blind replay"
        );
        assert!(journal.open_session_writes().is_empty());
        let mut output_after_exit = terminal.clone();
        output_after_exit.emissions.push(Emission {
            port: node::Port::conventional_out(),
            index: 0,
            envelope: envelope.clone(),
        });
        assert_eq!(
            journal.commit(output_after_exit),
            Err(Error::InvalidSessionExit)
        );
        let mut missing_projection = terminal.clone();
        missing_projection.emissions[0].envelope = envelope;
        assert_eq!(
            journal.commit(missing_projection),
            Err(Error::SessionProjectionMismatch)
        );
        journal.commit(terminal.clone()).unwrap();
        assert_eq!(journal.commit(terminal), Err(Error::UnknownSession));
        assert_eq!(
            journal.open_session_write(session_write(session, "socket", 4)),
            Err(Error::UnknownSession),
            "post-exit writes stay silent at the world boundary"
        );
    }
}
