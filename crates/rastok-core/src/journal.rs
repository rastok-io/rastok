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
//! [`delivery-semantics.md`]: https://github.com/rastok-io/rastok/blob/main/docs/spec/delivery-semantics.md
//! [`envelope.md`]: https://github.com/rastok-io/rastok/blob/main/docs/spec/envelope.md
//! [`identity.md`]: https://github.com/rastok-io/rastok/blob/main/docs/spec/identity.md

use crate::connector::exchange::CallId;
use crate::connector::exchange::CallOpen;
use crate::connector::exchange::Completion;
use crate::connector::exchange::InboundCallId;
use crate::connector::exchange::InboundSession;
use crate::connector::exchange::IngressProposal;
use crate::connector::exchange::IngressProvenance;
use crate::connector::exchange::IngressSequence;
use crate::connector::exchange::SessionId;
use crate::connector::exchange::SessionOpen;
use crate::connector::exchange::SessionWrite;
use crate::connector::exchange::SessionWriteId;
use crate::envelope::{Envelope, Id, IdKind, Meta, MetaSys};
use crate::instance;

use crate::node;
use crate::observation::{
    ExchangeCorrelation, RecordCorrelation, RecordObservation, RecordObservationKind,
};
use crate::plan::GraphVersion;
use crate::value::Value;
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

pub(crate) mod acceptance;
pub(crate) mod admission;
pub mod codec;
pub mod deadline;
pub mod instance_stop;
pub mod loop_protocol;
pub mod retry_protocol;

/// Deterministic identity recipes — the injective blake3 framing of
/// `envelope.md` §2, with its IdKind registry.
pub mod ids {
    use super::{CallId, Id, IdKind, IngressProvenance, SessionId, SessionWriteId, node};
    use crate::instance;
    use alloc::string::ToString;
    use alloc::vec::Vec;

    /// The root id of an inbound envelope:
    /// `blake3(connector_path ‖ inbound_seq)`. The single-process
    /// recipe; a non-zero replica joins the derivation when multi-host
    /// semantics land.
    #[must_use]
    pub fn root_id(provenance: &IngressProvenance, seq: u64) -> Id {
        Id::derive(
            IdKind::Root,
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
            IdKind::Emission,
            &[
                node_path.to_string().as_bytes(),
                trigger.to_string().as_bytes(),
                parent.as_bytes(),
                port.name().as_bytes(),
                &index.to_le_bytes(),
            ],
        )
    }

    /// The id of an envelope emitted by a Graph owner rather than an
    /// executable Node. Existing Node recipes keep their exact bytes; this
    /// form only makes the already-specified root-relative owner path
    /// available to Extension managers; Graphs have no application handlers.
    #[must_use]
    pub fn graph_emission_id(
        owner: &node::OwnerPath,
        trigger: &node::Trigger,
        parent: Id,
        port: &node::Port,
        index: u32,
    ) -> Id {
        Id::derive(
            IdKind::Emission,
            &[
                owner.to_string().as_bytes(),
                trigger.to_string().as_bytes(),
                parent.as_bytes(),
                port.name().as_bytes(),
                &index.to_le_bytes(),
            ],
        )
    }

    /// The delivery reference (envelope.md §2): one envelope delivered
    /// along one canonical edge is one delivery, and this is its
    /// identity — what `deadline_ids` (identity.md §3 key 24) seals, what
    /// the scheduler resolves, and what the call identity derives
    /// from. A fan-out of one envelope is one distinct reference per
    /// receiving edge, two ports of one connector included.
    #[must_use]
    pub fn delivery_id(edge: &str, delivery: Id) -> Id {
        Id::derive(IdKind::Delivery, &[edge.as_bytes(), delivery.as_bytes()])
    }

    /// The stable 16-byte deadline reference for one instance generation.
    #[must_use]
    pub fn instance_lifetime_id(instance: &instance::Generation) -> Id {
        Id::derive(
            IdKind::InstanceLifetime,
            &[
                instance.address().as_bytes(),
                &instance.generation().to_le_bytes(),
            ],
        )
    }

    /// Stable reference of one bounded Closing phase. `phase` is the
    /// extend-only deadline-kind number (drain or settlement), so the two
    /// obligations of one generation cannot collide.
    #[must_use]
    pub fn instance_closing_id(instance: &instance::Generation, phase: super::DeadlineKind) -> Id {
        Id::derive(
            IdKind::InstanceClosing,
            &[
                instance.address().as_bytes(),
                &instance.generation().to_le_bytes(),
                &[u8::from(phase)],
            ],
        )
    }

    /// The internal identity of an outbound call (envelope.md §2):
    /// one delivery carrying one prepared request — derived from the
    /// delivery reference and the digest of the exact prepared bytes,
    /// recomputable from any record that carries them (EGRESS keys
    /// 27, 30, 33), never stored as a new causal field. The digest
    /// says WHAT was sent; the reference says BY WHERE — routing
    /// never salts the digest.
    #[must_use]
    pub fn call_id(delivery_id: Id, request_digest: &[u8; 32]) -> CallId {
        CallId::new(Id::derive(
            IdKind::Call,
            &[delivery_id.as_bytes(), request_digest],
        ))
    }

    /// Logical reference of one inbound-opened session. The opening root is
    /// already journal-derived; the subtype frame keeps the recipe disjoint
    /// from outbound openings under the shared session domain.
    #[must_use]
    pub fn inbound_session_id(provenance: &IngressProvenance, opening: Id) -> SessionId {
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
        SessionId::new(Id::derive(IdKind::Session, &parts))
    }

    /// Logical reference of one outbound-opened session.
    #[must_use]
    pub fn outbound_session_id(
        connector: &node::Path,
        emitting: Id,
        request_digest: &[u8; 32],
    ) -> SessionId {
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
        SessionId::new(Id::derive(IdKind::Session, &parts))
    }

    /// Identity of one graph-to-world session write.
    #[must_use]
    pub fn session_write_id(
        session: SessionId,
        emitting: Id,
        message_digest: &[u8; 32],
    ) -> SessionWriteId {
        SessionWriteId::new(Id::derive(
            IdKind::SessionWrite,
            &[session.id().as_bytes(), emitting.as_bytes(), message_digest],
        ))
    }
}

/// The closed kinds of dated obligations recorded in key 24.
/// Explicit discriminants define their wire bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum DeadlineKind {
    /// One delivered Connector call budget.
    CallTimeout = 1,
    /// One join correlation bound.
    JoinWithin = 2,
    /// One logical instance generation lifetime.
    InstanceLifetime = 3,
    /// One Loop or Retry delayed re-entry wakeup, keyed by its admission id.
    RecurrenceDelay = 4,
    /// End of an instance's admitted-work drain interval.
    InstanceDrain = 5,
    /// End of an instance's cancellation settlement interval.
    InstanceSettlement = 6,
    /// One admitted inbound Call response deadline.
    InboundCallTimeout = 7,
    /// One client Session opening-handshake deadline.
    SessionHandshake = 8,
}

impl From<DeadlineKind> for u8 {
    fn from(kind: DeadlineKind) -> Self {
        kind as u8
    }
}

/// A byte which does not name an allocated deadline kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownDeadlineKind(pub u8);

impl fmt::Display for UnknownDeadlineKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown deadline kind byte {}", self.0)
    }
}

impl core::error::Error for UnknownDeadlineKind {}

impl TryFrom<u8> for DeadlineKind {
    type Error = UnknownDeadlineKind;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::CallTimeout),
            2 => Ok(Self::JoinWithin),
            3 => Ok(Self::InstanceLifetime),
            4 => Ok(Self::RecurrenceDelay),
            5 => Ok(Self::InstanceDrain),
            6 => Ok(Self::InstanceSettlement),
            7 => Ok(Self::InboundCallTimeout),
            8 => Ok(Self::SessionHandshake),
            other => Err(UnknownDeadlineKind(other)),
        }
    }
}

/// One causal record. Each owned packet is indirect so journal growth reserves
/// handles, not the largest packet in every unused slot. This physical layout
/// changes neither the three record kinds nor their canonical projection.
#[derive(Debug, Clone, PartialEq)]
pub enum Record {
    /// The world entered: an envelope was born at ingress.
    Ingress(Box<IngressRecord>),
    /// One node fired: the atomic commit packet.
    Transaction(Box<TransactionRecord>),
    /// The graph acted on the world: one fact of a call's WAL — the
    /// open, then separately its closure. The past never rewrites.
    Egress(EgressRecord),
}

impl Record {
    /// Required unsealed deadline evidence when present. This is not a lossy
    /// observation and never enters the causal seal. Outbound effects inherit
    /// budgets created by their accepting delivery, not their Driver start.
    #[must_use]
    pub fn deadlines(&self) -> Option<&deadline::Deadlines> {
        match self {
            Self::Ingress(record) => record.deadlines.as_deref(),
            Self::Transaction(record) => record.deadlines.as_deref(),
            Self::Egress(EgressRecord::Call(record)) => record.deadlines.as_deref(),
            Self::Egress(_) => None,
        }
    }
}

pub(crate) fn recorded_envelope(records: &[Record], id: Id) -> Option<&Envelope> {
    records.iter().find_map(|record| match record {
        Record::Ingress(inbound) if inbound.envelope.meta().sys().id() == id => {
            Some(&inbound.envelope)
        }
        Record::Transaction(transaction) => transaction
            .emissions
            .iter()
            .map(|emission| &emission.envelope)
            .find(|envelope| envelope.meta().sys().id() == id)
            .or_else(|| {
                transaction.loop_protocol().and_then(|packet| {
                    packet
                        .envelopes()
                        .find(|envelope| envelope.meta().sys().id() == id)
                })
            })
            .or_else(|| {
                transaction.retry_record.as_deref().and_then(|packet| {
                    packet
                        .envelopes()
                        .find(|envelope| envelope.meta().sys().id() == id)
                })
            }),
        Record::Ingress(_) | Record::Egress(_) => None,
    })
}

fn recorded_origin(records: &[Record], id: Id) -> Option<Id> {
    recorded_envelope(records, id).map(|envelope| envelope.meta().sys().origin_id())
}

fn inbound_exchange_context(
    records: &[Record],
    exchange: InboundExchangeExitRecord,
) -> (Option<Id>, Option<node::Path>) {
    let inbound = records.iter().rev().find_map(|record| {
        let Record::Ingress(inbound) = record else {
            return None;
        };
        let matches = match (exchange, inbound.exchange.as_deref()) {
            (
                InboundExchangeExitRecord::Call(expected),
                Some(InboundExchangeRecord::Call(actual)),
            ) => expected == actual.reference,
            (
                InboundExchangeExitRecord::Session(expected),
                Some(InboundExchangeRecord::Session(
                    SessionIngressRecord::Open(actual) | SessionIngressRecord::Message(actual),
                )),
            ) => expected == *actual,
            _ => false,
        };
        matches.then(|| {
            (
                Some(inbound.envelope.meta().sys().origin_id()),
                inbound.connector_path.clone(),
            )
        })
    });
    if let Some(context) = inbound {
        return context;
    }
    let InboundExchangeExitRecord::Session(session) = exchange else {
        return (None, None);
    };
    let connector = records.iter().rev().find_map(|record| match record {
        Record::Egress(EgressRecord::Session(recorded)) if recorded.session_id == session => {
            Some(recorded.connector_path.clone())
        }
        _ => None,
    });
    (session_origin(records, session, None), connector)
}

fn session_origin(
    records: &[Record],
    session: SessionId,
    write: Option<SessionWriteId>,
) -> Option<Id> {
    records.iter().rev().find_map(|record| {
        let Record::Egress(EgressRecord::Session(recorded)) = record else {
            return None;
        };
        if recorded.session_id != session {
            return None;
        }
        let emitting = match (&recorded.status, write) {
            (SessionStatus::Opening(intent), None) => Some(intent.emitting_id),
            (SessionStatus::WriteOpen(intent), Some(expected)) if intent.write_ref == expected => {
                Some(intent.emitting_id)
            }
            _ => None,
        }?;
        recorded_origin(records, emitting)
    })
}

fn transaction_origin(records: &[Record], transaction: &TransactionRecord) -> Option<Id> {
    transaction
        .emissions
        .first()
        .map(|emission| emission.envelope.meta().sys().origin_id())
        .or_else(|| {
            transaction
                .cause
                .and_then(|id| recorded_origin(records, id))
        })
        .or_else(|| {
            transaction
                .join_inputs
                .first()
                .and_then(|id| recorded_origin(records, *id))
        })
        .or_else(|| {
            transaction
                .exchange_exit
                .as_deref()
                .and_then(|exchange| inbound_exchange_context(records, *exchange).0)
        })
}

fn observation_for_record(records: &[Record], record: &Record) -> RecordObservation {
    match record {
        Record::Ingress(inbound) => {
            let mut correlation =
                RecordCorrelation::new(Some(inbound.envelope.meta().sys().origin_id()))
                    .with_connector(inbound.connector_path.clone());
            match inbound.exchange.as_deref() {
                Some(InboundExchangeRecord::Call(call)) => {
                    correlation = correlation
                        .with_instance(call.instance.clone())
                        .with_exchange(Some(ExchangeCorrelation::InboundCall(call.reference)));
                }
                Some(InboundExchangeRecord::Session(
                    SessionIngressRecord::Open(session) | SessionIngressRecord::Message(session),
                )) => {
                    correlation = correlation.with_exchange(Some(ExchangeCorrelation::Session {
                        session: *session,
                        write: None,
                    }));
                }
                None => {}
            }
            RecordObservation::new(RecordObservationKind::IngressCommitted, correlation)
        }
        Record::Transaction(transaction) => {
            let exchange = transaction
                .exchange_exit
                .as_deref()
                .map(|exchange| match exchange {
                    InboundExchangeExitRecord::Call(reference) => {
                        ExchangeCorrelation::InboundCall(*reference)
                    }
                    InboundExchangeExitRecord::Session(reference) => ExchangeCorrelation::Session {
                        session: *reference,
                        write: None,
                    },
                });
            let connector = transaction
                .exchange_exit
                .as_deref()
                .and_then(|exchange| inbound_exchange_context(records, *exchange).1);
            let correlation = RecordCorrelation::new(transaction_origin(records, transaction))
                .with_owner(transaction.owner_path.clone())
                .with_instance(transaction.instance.clone())
                .with_connector(connector)
                .with_exchange(exchange);
            RecordObservation::new(RecordObservationKind::TransactionCommitted, correlation)
        }
        Record::Egress(EgressRecord::Call(call)) => {
            let kind = match call.status {
                CallStatus::Open => RecordObservationKind::CallOpened,
                CallStatus::Closed(_) => RecordObservationKind::CallClosed,
                CallStatus::Abandoned => RecordObservationKind::CallAbandoned,
            };
            let correlation =
                RecordCorrelation::new(recorded_origin(records, call.intent.emitting_id))
                    .with_connector(Some(call.intent.connector_path.clone()))
                    .with_exchange(Some(ExchangeCorrelation::OutboundCall(call.intent.call_id)));
            RecordObservation::new(kind, correlation)
        }
        Record::Egress(EgressRecord::Session(session)) => {
            let (kind, write, direct_emitting) = match &session.status {
                SessionStatus::Opening(intent) => (
                    RecordObservationKind::SessionOpening,
                    None,
                    Some(intent.emitting_id),
                ),
                SessionStatus::Opened => (RecordObservationKind::SessionOpened, None, None),
                SessionStatus::WriteOpen(intent) => (
                    RecordObservationKind::SessionWriteOpened,
                    Some(intent.write_ref),
                    Some(intent.emitting_id),
                ),
                SessionStatus::WriteSettled(reference) => (
                    RecordObservationKind::SessionWriteSettled,
                    Some(*reference),
                    None,
                ),
                SessionStatus::WriteAbandoned(reference) => (
                    RecordObservationKind::SessionWriteAbandoned,
                    Some(*reference),
                    None,
                ),
            };
            let origin = direct_emitting
                .and_then(|id| recorded_origin(records, id))
                .or_else(|| session_origin(records, session.session_id, write));
            let correlation = RecordCorrelation::new(origin)
                .with_connector(Some(session.connector_path.clone()))
                .with_exchange(Some(ExchangeCorrelation::Session {
                    session: session.session_id,
                    write,
                }));
            RecordObservation::new(kind, correlation)
        }
    }
}

pub(crate) fn observations_for_records_from(
    records: &[Record],
    start: usize,
) -> Vec<RecordObservation> {
    records
        .iter()
        .enumerate()
        .skip(start)
        .map(|(index, record)| observation_for_record(&records[..index], record))
        .collect()
}

/// An accepted inbound envelope and its record (`identity.md` §3,
/// keys 8–16). The record autonomously reconstructs its root id from
/// `(connector_path, replica, seq)`.
#[derive(Debug, Clone, PartialEq)]
pub struct IngressRecord {
    /// Exact-instance commands retained atomically with this root (key 42).
    pub instance_commands: Option<Box<instance_stop::InstanceCommands>>,
    /// Unsealed creation and due instants (key 41), atomically accepted with
    /// this ingress. Omitted when this record creates no deadline.
    pub deadlines: Option<Box<deadline::Deadlines>>,
    /// Stable provenance of the external root stream.
    pub provenance: IngressProvenance,
    /// Source port of the root: an App input when [`Self::connector_path`] is
    /// absent, otherwise that Connector's output. Replay re-routes from this
    /// causal fact. Projects as key 12, omitted for conventional `in`.
    pub port: node::Port,
    /// Connector output which introduced this root. `None` is an explicit
    /// App-boundary injection. Replay therefore never guesses the source
    /// endpoint from provenance text.
    pub connector_path: Option<node::Path>,
    /// The atomic per-provenance sequence.
    pub seq: u64,
    /// The envelope born here.
    pub envelope: Envelope,
    /// Every target instance selected before this INGRESS fact committed.
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
    pub reference: InboundCallId,
    /// Exact Connector generation that owns the request and its terminal exit.
    pub instance: instance::Generation,
    /// Dated obligations created by this Call-opening fact. The tuple registry
    /// is shared with transaction deadlines; ack-mode Calls carry none.
    pub deadline_ids: Vec<(DeadlineKind, Id)>,
}

/// Exchange-specific terminal reference carried by one Connector transaction.
///
/// Call and Session termination are categorically exclusive. The extension is
/// boxed because ordinary Operator and outbound-Call transactions carry
/// neither; adding a rare exchange must not widen every retained record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundExchangeExitRecord {
    /// Terminal transaction of one admitted inbound Call.
    Call(InboundCallId),
    /// Terminal transaction of one inbound Session.
    Session(SessionId),
}

/// Resolved session meaning of one committed INGRESS fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionIngressRecord {
    /// This record opened the named session.
    Open(SessionId),
    /// This record delivered one message in the named live session.
    Message(SessionId),
}

/// One pre-commit target selection for a single inbound envelope.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RouteTarget {
    /// Canonical edge text carrying the envelope.
    pub edge: String,
    /// Complete target instance reference returned by destination admission.
    pub target: instance::Generation,
}

/// One consumed input position — the cursor is per
/// `(graph edge × source instance reference)`, not per port: several edges can
/// feed one port.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConsumedPosition {
    /// The graph edge the input arrived on, canonically rendered.
    pub edge: String,
    /// Complete upstream instance reference which owns this edge cursor.
    pub source: instance::Generation,
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

/// Placement-neutral identity of one finite Loop run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LoopRunId(Id);

impl LoopRunId {
    /// Wraps an identity derived by [`loop_protocol::run_ref`].
    #[must_use]
    pub const fn new(id: Id) -> Self {
        Self(id)
    }

    /// Raw causal identity used by the canonical record projection.
    #[must_use]
    pub const fn id(self) -> Id {
        self.0
    }
}

impl fmt::Display for LoopRunId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// The atomic commit packet of one node firing (`identity.md` §3,
/// keys 17–26, 34–36, 38–40 and 42–43): consumed input, state delta, and emissions commit
/// together or not at all. [`Journal::commit`] canonicalizes the
/// sorted collections; emission order is text order and stays as
/// given.
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionRecord {
    /// Sparse pinned-operation facts/correlation (causal key 43).
    pub retry_record: Option<Box<retry_protocol::RetryRecord>>,
    /// Exact-instance command production or winning consumption (key 42).
    pub instance_commands: Option<Box<instance_stop::InstanceCommands>>,
    /// Unsealed creation and due instants (key 41), outside causal identity.
    /// Absence is not evidence from which recovery may invent a deadline.
    pub deadlines: Option<Box<deadline::Deadlines>>,
    /// The firing lifecycle/transaction owner. Ordinary Node transactions
    /// carry the same segments as their node path; Graph lifecycle records
    /// use the preserved lexical Graph owner, including the empty root path.
    pub owner_path: node::OwnerPath,
    /// Complete firing instance reference. It is mandatory even for a
    /// persistent singleton owner.
    pub instance: instance::Generation,
    /// Enclosing Graph generation, present on a nested owner's `l:start`.
    /// The address carries ancestor key frames but deliberately not ancestor
    /// generations; recovery needs this explicit causal edge.
    pub parent_instance: Option<instance::Generation>,
    /// A close request won atomically with this firing. The reference is the
    /// root of the closing scope; descendants may finish already-admitted
    /// work, while later external routes reserve the successor generation.
    pub close_scope: Option<instance::Generation>,
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
    /// One sparse Loop protocol packet. Boxing keeps its absence to one
    /// pointer in every retained non-Loop transaction.
    pub loop_record: Option<Box<loop_protocol::LoopRecord>>,
    /// The transaction's fixed civil time when one of its checked
    /// expressions can observe `now()`. Omitted for handlers that do not read
    /// time, so old causal facts and time-insensitive programs keep their
    /// exact canonical form.
    pub observed_time: Option<crate::time::Timestamp>,
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
    /// The dated obligations this firing creates — `(DeadlineKind, id)`
    /// per identity.md §3 key 24: kind 1 = call budget, ref = the
    /// derived budget identity `budget_ref(edge, envelope_id)`
    /// (erratum E-2026-08-04-2). Sorted at commit; the due instant
    /// is never sealed.
    pub deadline_ids: Vec<(DeadlineKind, crate::envelope::Id)>,
}

impl TransactionRecord {
    /// Command-based Loop packet, absent on all other transactions.
    #[must_use]
    pub fn loop_protocol(&self) -> Option<&loop_protocol::LoopRecord> {
        self.loop_record.as_deref()
    }
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
    pub target: instance::Generation,
}

/// One external-world WAL fact. Call and Session subforms remain under the
/// same EGRESS causal record kind; neither can be mistaken for the other.
#[derive(Debug, Clone, PartialEq)]
pub enum EgressRecord {
    /// One fact of a Call's open/close WAL.
    Call(Box<CallRecord>),
    /// One fact of a Session opening or message-write WAL.
    Session(Box<SessionRecord>),
}

/// One fact of a Call's WAL (`delivery-semantics.md` §4): the open fact, or
/// its closure — two records, never one rewritten.
#[derive(Debug, Clone, PartialEq)]
pub struct CallRecord {
    /// Attempt correlation (causal key 43), absent outside Retry.
    pub retry_record: Option<Box<retry_protocol::RetryRecord>>,
    /// Winning logical closure instant (unsealed key 44); absent only on Open.
    pub closed_at: Option<crate::time::LogicalTime>,
    /// Original closure-based recurrence deadline (unsealed 41, causal presence 24).
    pub deadlines: Option<Box<deadline::Deadlines>>,
    /// The canonical call fields.
    pub intent: CallOpen,
    /// Which fact this record states.
    pub status: CallStatus,
}

/// One causal EGRESS fact inside the Session fold.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRecord {
    /// Connector that owns the logical session.
    pub connector_path: node::Path,
    /// Session whose fold this fact advances.
    pub session_id: SessionId,
    /// Exact transition stated by this record.
    pub status: SessionStatus,
}

/// Append-only states of outbound Session work.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionStatus {
    /// Journal-before-effect opening intent for a client-like session.
    Opening(SessionOpen),
    /// The opening effect succeeded and messages may flow.
    Opened,
    /// One graph-to-world message became an open write obligation.
    WriteOpen(SessionWrite),
    /// The Driver locally accepted the message bytes.
    WriteSettled(SessionWriteId),
    /// Recovery cannot prove the message's external visibility; it must not
    /// be replayed blindly.
    WriteAbandoned(SessionWriteId),
}

/// The sealed causal states of an outbound call. A late reply after
/// closure is a discarded observation — recorded elsewhere, never a
/// sealed status.
#[derive(Debug, Clone, PartialEq)]
pub enum CallStatus {
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

/// Complete result returned by the journal after an ingress commit: its
/// sequence, envelope and record. The Driver receives only their portable
/// identifiers in [`crate::connector::exchange::AdmittedIngress`].
#[derive(Debug, Clone, PartialEq)]
pub struct CommittedIngress {
    /// The allocated or validated sequence.
    pub seq: u64,
    /// The admitted envelope, with its four runtime-assigned `sys` fields.
    pub envelope: Envelope,
    /// The committed record.
    pub record: IngressRecord,
}

/// A complete ingress envelope whose causal identity is fixed but whose
/// accepting record has not committed yet.
///
/// This token lets routing evaluate `meta.sys` without either inventing a
/// provisional identity or mutating the journal before destination admission
/// succeeds. [`Journal::commit_ingress`] revalidates the sequence atomically.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedIngress {
    instance_commands: Option<Box<instance_stop::InstanceCommands>>,
    submission: IngressProposal,
    connector_path: Option<node::Path>,
    seq: u64,
    envelope: Envelope,
    exchange: Option<InboundExchangeRecord>,
    deadlines: Option<Box<deadline::Deadlines>>,
}

impl PreparedIngress {
    /// Retain commands with their accepting root, never in a Worker-only queue.
    #[must_use]
    pub fn with_instance_commands(mut self, packet: instance_stop::InstanceCommands) -> Self {
        self.instance_commands = Some(Box::new(packet));
        self
    }
    /// Attach validated timing evidence before committing the accepting fact.
    /// A durable Journal stores it atomically with the root and its routes.
    #[must_use]
    pub fn with_deadlines(mut self, schedule: deadline::Deadlines) -> Self {
        self.deadlines = Some(Box::new(schedule));
        self
    }
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
        instance: instance::Generation,
        deadline_ids: Vec<(DeadlineKind, Id)>,
    ) -> Result<Self, Error> {
        if self.connector_path.is_none() || self.exchange.is_some() {
            return Err(Error::CorruptInboundCall);
        }
        self.exchange = Some(InboundExchangeRecord::Call(InboundCallIngressRecord {
            reference: InboundCallId::from_root(self.envelope.meta().sys().id()),
            instance,
            deadline_ids,
        }));
        Ok(self)
    }
}

/// A journal fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A retained prefix cannot be reconstructed through the accepting operations.
    InvalidRetainedRecord(&'static str),
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
        expected: instance::Generation,
        /// The transported claim.
        got: instance::Generation,
    },
    /// A transaction claims an actor other than the owner sealed into its
    /// instance address.
    InstanceOwnerMismatch {
        /// Owner derived from the instance address.
        expected: node::OwnerPath,
        /// Owner transported by the transaction.
        got: node::OwnerPath,
    },
    /// `l:start` attempted to reopen an active generation, or `l:stop`
    /// attempted to close an address whose generation never started.
    InvalidInstanceLifecycle {
        /// The addressed generation.
        instance: instance::Generation,
        /// Whether the journal fold was active before the transition.
        active: bool,
    },
    /// The parent reference is missing, unexpected, or does not name an
    /// enclosing owner. It belongs exactly on a non-root owner's `l:start`.
    InvalidParentInstance {
        /// The child lifecycle reference.
        instance: instance::Generation,
        /// The transported parent claim, if any.
        parent: Option<instance::Generation>,
    },
    /// A parent `l:stop` was attempted while at least one journaled child
    /// lifecycle obligation remained open.
    OpenChildInstances {
        /// Parent generation that must remain active.
        parent: instance::Generation,
    },
    /// A close request did not name the firing generation itself.
    InvalidCloseScope {
        /// Firing generation.
        instance: instance::Generation,
        /// Transported close root.
        scope: instance::Generation,
    },
    /// A second close request tried to govern a generation already covered by
    /// an active durable close root. The first causal request is authoritative.
    InstanceAlreadyClosing {
        /// Generation named by the losing request.
        instance: instance::Generation,
        /// Existing root whose Closing scope already covers it.
        scope: instance::Generation,
    },
    /// `l:stop` was attempted before a durable close request existed.
    StopWithoutClose {
        /// Generation whose lifecycle was truncated.
        instance: instance::Generation,
    },
    /// An r2 command packet contradicts the replay-derived protocol.
    InvalidLoopProtocol(loop_protocol::Error),
    /// Retry facts must have their exact manager or producer carrier.
    InvalidRetryProtocol(retry_protocol::Error),
    /// An exact-instance command contradicts its source, target or close fact.
    InvalidInstanceCommand(instance_stop::Error),
    /// Unsealed deadline evidence contradicts the accepted obligations.
    InvalidDeadlines(deadline::Error),
    /// A Call carrier contradicts its closure time or Retry correlation.
    InvalidCallRetention(deadline::Error),
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
    /// `meta.ext.session_id` projection owned by its `SessionId`.
    SessionProjectionMismatch,
    /// The backing store failed to commit — the durable-journal
    /// escape hatch; the in-memory journal never raises it.
    Storage(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRetainedRecord(reason) => write!(f, "invalid journal record: {reason}"),
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
            Self::InvalidLoopProtocol(error) => error.fmt(f),
            Self::InvalidRetryProtocol(error) => error.fmt(f),
            Self::InvalidInstanceCommand(error) => error.fmt(f),
            Self::InvalidDeadlines(error) => write!(f, "invalid deadlines: {error}"),
            Self::InvalidCallRetention(error) => write!(f, "invalid Call retention: {error}"),
            Self::UnknownCall => f.write_str("no EGRESS record for this call"),
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
    /// Immutable civil origin of the run, stored in its unsealed header.
    /// Logical instants are relative to this origin, beginning at zero.
    fn origin(&self) -> crate::time::Timestamp;

    /// Visits one consistent committed prefix in causal order. These are
    /// semantic records, not the store's physical encoding. Recovery never
    /// executes a producer while reading this stream.
    ///
    /// # Errors
    /// Failure to read the complete committed prefix from the store.
    fn visit_records(&self, visit: &mut dyn FnMut(&Record)) -> Result<(), Error>;

    /// Deployment facts fixed before the first causal append. A durable
    /// journal stores this in its immutable run header; it is not a fourth
    /// causal record stream.
    #[must_use]
    fn deployment_attestation(&self) -> Option<&crate::deployment::DeploymentAttestation> {
        None
    }

    /// Personal-data retention capability of this journal implementation.
    /// The default is deliberately closed: carrying a matching profile header
    /// does not by itself prove storage limitation or erasure semantics.
    #[must_use]
    fn privacy_retention(&self) -> Option<crate::deployment::PrivacyRetention> {
        None
    }

    /// Typed non-causal observations retained beside causal history, starting
    /// at one semantic stream offset.
    ///
    /// A physical journal may index or reconstruct this stream however it
    /// chooses. Export queues are disposable consumers of this query; they do
    /// not become another accounting authority. An offset at or beyond the
    /// current end returns an empty suffix.
    #[must_use]
    fn retained_observations_from(&self, start: usize) -> Vec<RecordObservation>;

    /// Returns the complete retained observation stream.
    #[must_use]
    fn retained_observations(&self) -> Vec<RecordObservation> {
        self.retained_observations_from(0)
    }

    /// Constructs the complete ingress envelope without committing it. The result
    /// is a read-only reservation: only [`Self::commit_ingress`] consumes the
    /// sequence.
    ///
    /// # Errors
    ///
    /// Sequence mismatch or exhaustion, as for [`Self::append_ingress`].
    /// Session framing in the proposal also requires a valid lifecycle
    /// transition; its identity is derived or checked and projected into
    /// `meta.ext.session_id` by the journal.
    fn prepare_ingress(&self, submission: IngressProposal) -> Result<PreparedIngress, Error>;

    /// Constructs a Connector-origin root without committing it. The source node
    /// is derived from the typed provenance and recorded separately so replay
    /// can route from the same output endpoint.
    ///
    /// # Errors
    ///
    /// The same sequence failures as [`Self::prepare_ingress`], plus Session
    /// lifecycle refusals when the proposal carries Session framing.
    fn prepare_connector_ingress(
        &self,
        submission: IngressProposal,
    ) -> Result<PreparedIngress, Error>;

    /// Atomically commits one prepared ingress and its destination routes.
    /// A durable implementation must compare-and-advance the prepared
    /// sequence in the same transaction as the record.
    ///
    /// # Errors
    ///
    /// A stale sequence, a forged instance generation, or storage failure.
    fn commit_ingress(
        &mut self,
        prepared: PreparedIngress,
        routes: Vec<RouteTarget>,
    ) -> Result<CommittedIngress, Error>;

    /// The one accepting transaction of an inbound submission:
    /// allocate or validate the sequence, derive the root id, construct
    /// `meta.sys` (no parent; the envelope is its own origin), build
    /// the envelope, and commit. Only after this returns may the
    /// caller acknowledge the transport — commit-then-ack.
    ///
    /// # Errors
    ///
    /// [`Error::SequenceMismatch`] when an explicit sequence diverges
    /// from the journal; [`Error::SequenceOverflow`] when monotonicity
    /// cannot continue.
    fn append_ingress(&mut self, submission: IngressProposal) -> Result<CommittedIngress, Error> {
        self.append_ingress_routed(submission, Vec::new())
    }

    /// Accepts an inbound envelope together with every non-singleton target
    /// selected before the fact commits. This is the runtime entry point;
    /// [`Self::append_ingress`] is the route-free convenience used by journal
    /// tests and callers whose plan has no keyed destination.
    fn append_ingress_routed(
        &mut self,
        submission: IngressProposal,
        routes: Vec<RouteTarget>,
    ) -> Result<CommittedIngress, Error> {
        let prepared = self.prepare_ingress(submission)?;
        self.commit_ingress(prepared, routes)
    }

    /// Returns the generation a route committing now must seal for this
    /// address. The value is a pure fold of journaled `l:start`/`l:stop`
    /// transactions; no Worker-local counter participates.
    fn instance_generation_for_admission(
        &self,
        address: &instance::InstanceAddress,
    ) -> Result<instance::Generation, Error>;

    /// Reconstructs the command-based Loop protocol from committed facts.
    ///
    /// # Errors
    /// A contradictory r2 history. No Worker state contributes to this fold.
    fn loop_protocol(&self) -> Result<loop_protocol::LoopView, Error>;

    /// Reconstruct exact-instance commands from one retained prefix.
    ///
    /// # Errors
    /// An unreadable prefix or a contradictory command/lifecycle fact.
    fn instance_commands(&self) -> Result<instance_stop::InstanceCommandView, Error> {
        let mut fold = instance_stop::InstanceCommandView::default();
        let mut result = Ok(());
        self.visit_records(&mut |record| {
            if result.is_ok() {
                result = fold.apply(record);
            }
        })?;
        result.map_err(Error::InvalidInstanceCommand)?;
        Ok(fold)
    }

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
    fn open_egress(
        &mut self,
        intent: CallOpen,
        attempt: Option<retry_protocol::AttemptId>,
    ) -> Result<(), Error>;

    /// Journals one outbound session opening before the Driver acts.
    ///
    /// # Errors
    ///
    /// A corrupt or duplicate session reference.
    fn open_outbound_session(&mut self, intent: SessionOpen) -> Result<(), Error>;

    /// Confirms that one outbound opening established a live session.
    ///
    /// # Errors
    ///
    /// The session is unknown or already terminal.
    fn confirm_session_open(&mut self, session: &SessionId) -> Result<CloseVerdict, Error>;

    /// Journals one session message before the Driver may offer its bytes.
    ///
    /// # Errors
    ///
    /// The session is unavailable or the write identity is corrupt/duplicate.
    fn open_session_write(&mut self, intent: SessionWrite) -> Result<(), Error>;

    /// Settles one local session write obligation. `abandoned` means recovery
    /// cannot prove external visibility and therefore must not replay it.
    ///
    /// # Errors
    ///
    /// The session/write is unknown.
    fn settle_session_write(
        &mut self,
        write_ref: &SessionWriteId,
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
    fn close_egress(
        &mut self,
        call_id: &CallId,
        closure: Closure,
        closed_at: crate::time::LogicalTime,
        schedule: Option<Box<deadline::Deadlines>>,
    ) -> Result<CloseVerdict, Error>;

    /// Declares that bounded Closing could not establish an observed
    /// terminal outcome. This participates in the same close-once race as a
    /// response or timeout and appends an `abandoned` fact on success.
    fn abandon_egress(
        &mut self,
        call_id: &CallId,
        closed_at: crate::time::LogicalTime,
    ) -> Result<CloseVerdict, Error>;

    /// The envelope carrying `id`, wherever the history recorded it.
    fn find_envelope(&self, id: Id) -> Option<Envelope>;

    /// The intent of a journaled call, settled or not.
    fn find_call(&self, call_id: &CallId) -> Option<CallOpen>;

    /// The calls whose open fact has no closure fact yet, in open
    /// order.
    fn open_calls(&self) -> Vec<CallOpen>;

    /// Admitted inbound Calls whose terminal Connector transaction has not
    /// committed, in canonical reference order.
    fn open_inbound_calls(&self) -> Vec<InboundCallIngressRecord>;

    /// Live sessions in canonical reference order.
    fn live_sessions(&self) -> Vec<SessionId>;

    /// Session message writes whose open fact has no settlement fact.
    fn open_session_writes(&self) -> Vec<SessionWrite>;
}

/// The deliberately simple, linear M1 in-memory journal. It preserves the
/// logical boundaries a durable implementation needs — typed records,
/// per-provenance sequences, append-only closure — with no claim about files.
///
/// `find_call`, `find_envelope`, and `statuses_of` scan records in O(n); the Execution
/// also performs full scans. This is acceptable for in-memory runs of a few
/// hundred records. M2 must maintain indexes atomically with the records,
/// following the existing lifecycle and Session folds, without making an
/// index a second source of truth.
#[derive(Debug)]
pub struct MemJournal {
    graph_version: GraphVersion,
    origin: crate::time::Timestamp,
    deployment_attestation: Option<crate::deployment::DeploymentAttestation>,
    records: Vec<Record>,
    next_seq: BTreeMap<IngressProvenance, u64>,
    /// Replay-derived lifecycle fold, maintained atomically with `records`.
    /// It is an index over causal facts, never a second generation authority.
    admissions: admission::AdmissionView,
    /// Replay-derived Session fold. It is an index over the three ordinary
    /// causal record kinds, never a fourth record stream.
    sessions: BTreeMap<SessionId, SessionLifecycle>,
    /// Replay-derived inbound Call fold. The opening INGRESS fact and terminal
    /// TRANSACTION remain the only authorities.
    inbound_calls: BTreeMap<InboundCallId, InboundCallLifecycle>,
    /// Replay-derived Loop fold, replaced only with the committed record.
    /// The independent LoopView reader reconstructs it from `records`; ordinary
    /// non-Loop journals allocate nothing for this index.
    command_loops: Option<Box<loop_protocol::LoopView>>,
}

#[derive(Debug, Clone)]
struct InboundCallLifecycle {
    connector_path: node::Path,
    instance: instance::Generation,
    deadline_ids: Vec<(DeadlineKind, Id)>,
    open: bool,
}

#[derive(Debug, Clone)]
struct SessionLifecycle {
    connector_path: node::Path,
    phase: SessionPhase,
    writes: BTreeMap<SessionWriteId, bool>,
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
        Self::new_with_origin(graph_version, crate::time::Timestamp::UNIX_EPOCH)
    }

    /// An empty journal with an explicit immutable civil origin.
    #[must_use]
    pub fn new_with_origin(graph_version: GraphVersion, origin: crate::time::Timestamp) -> Self {
        Self {
            graph_version,
            origin,
            deployment_attestation: None,
            records: Vec::new(),
            next_seq: BTreeMap::new(),
            admissions: admission::AdmissionView::default(),
            sessions: BTreeMap::new(),
            inbound_calls: BTreeMap::new(),
            command_loops: None,
        }
    }

    /// An empty run whose deployment facts have already been fixed. This is
    /// the only M1 journal with a personal-data retention capability, and
    /// that capability is deliberately run-scoped: dropping this value drops
    /// its complete in-memory history.
    #[must_use]
    pub fn new_with_deployment(
        graph_version: GraphVersion,
        origin: crate::time::Timestamp,
        attestation: crate::deployment::DeploymentAttestation,
    ) -> Self {
        let mut journal = Self::new_with_origin(graph_version, origin);
        journal.deployment_attestation = Some(attestation);
        journal
    }

    /// The records in causal order — an implementation detail of the
    /// in-memory journal, for tests and the golden projection; the
    /// trait exposes semantic queries instead.
    #[must_use]
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// Deployment version stamped on this journal's admitted envelopes.
    #[must_use]
    pub fn graph_version(&self) -> GraphVersion {
        self.graph_version
    }

    /// Rebuilds a retained prefix through the journal's accepting operations.
    /// No producer or Driver executes. The caller must separately validate
    /// the prefix against its plan with Execution recovery before exposing it.
    ///
    /// # Errors
    /// Invalid roots, sequences, lifecycle, exchange or Loop transitions.
    /// An error never returns a partially reconstructed journal.
    pub fn from_records(
        graph_version: GraphVersion,
        origin: crate::time::Timestamp,
        attestation: Option<crate::deployment::DeploymentAttestation>,
        records: &[Record],
    ) -> Result<Self, Error> {
        let mut journal = attestation.map_or_else(
            || Self::new_with_origin(graph_version, origin),
            |header| Self::new_with_deployment(graph_version, origin, header),
        );
        for record in records {
            let previous = journal.records.len();
            match record {
                Record::Ingress(inbound) => {
                    let session = match inbound.exchange.as_deref() {
                        Some(InboundExchangeRecord::Session(SessionIngressRecord::Open(_))) => {
                            Some(InboundSession::Open)
                        }
                        Some(InboundExchangeRecord::Session(SessionIngressRecord::Message(
                            reference,
                        ))) => Some(InboundSession::Message(*reference)),
                        _ => None,
                    };
                    let submission = IngressProposal {
                        provenance: inbound.provenance.clone(),
                        draft: crate::connector::exchange::EnvelopeDraft {
                            port: inbound.port.clone(),
                            data: inbound.envelope.data().clone(),
                            ext: inbound.envelope.meta().ext().clone(),
                        },
                        sequence: IngressSequence::Explicit(inbound.seq),
                        session,
                    };
                    let mut prepared =
                        journal.prepare_ingress_fact(submission, inbound.connector_path.clone())?;
                    if let Some(InboundExchangeRecord::Call(call)) = inbound.exchange.as_deref() {
                        prepared =
                            prepared.into_call(call.instance.clone(), call.deadline_ids.clone())?;
                    }
                    let metadata_equal =
                        prepared.envelope.meta().app().is_empty()
                            && inbound.envelope.meta().app().is_empty()
                            && prepared
                                .envelope
                                .meta()
                                .ext()
                                .iter()
                                .map(|(name, value)| (name, crate::canonical::meta_value(value)))
                                .eq(inbound.envelope.meta().ext().iter().map(|(name, value)| {
                                    (name, crate::canonical::meta_value(value))
                                }));
                    if !metadata_equal
                        || prepared.envelope.meta().sys() != inbound.envelope.meta().sys()
                        || prepared.exchange.as_ref() != inbound.exchange.as_deref()
                    {
                        return Err(Error::InvalidRetainedRecord("ingress identity or exchange"));
                    }
                    if let Some(schedule) = inbound.deadlines.as_deref() {
                        prepared = prepared.with_deadlines(schedule.clone());
                    }
                    if let Some(packet) = inbound.instance_commands.as_deref() {
                        prepared = prepared.with_instance_commands(packet.clone());
                    }
                    journal.commit_ingress(prepared, inbound.routes.clone())?;
                }
                Record::Transaction(transaction) => journal.commit(*transaction.clone())?,
                Record::Egress(EgressRecord::Call(call)) => {
                    acceptance::validate_call(call).map_err(Error::InvalidCallRetention)?;
                    if call.status != CallStatus::Open {
                        let Some(original) =
                            journal.records.iter().find_map(|record| match record {
                                Record::Egress(EgressRecord::Call(open))
                                    if open.intent.call_id == call.intent.call_id =>
                                {
                                    Some(open)
                                }
                                _ => None,
                            })
                        else {
                            return Err(Error::UnknownCall);
                        };
                        if original.intent != call.intent
                            || original.retry_record != call.retry_record
                        {
                            return Err(Error::InvalidRetainedRecord(
                                "a Call closure changed its original intent or attempt",
                            ));
                        }
                    }
                    match &call.status {
                        CallStatus::Open => journal.open_egress(
                            call.intent.clone(),
                            call.retry_record
                                .as_deref()
                                .and_then(|packet| packet.context),
                        )?,
                        CallStatus::Closed(closure) => {
                            let at = call.closed_at.ok_or({
                                Error::InvalidCallRetention(deadline::Error::MissingClosureInstant)
                            })?;
                            journal.close_egress(
                                &call.intent.call_id,
                                closure.clone(),
                                at,
                                call.deadlines.clone(),
                            )?;
                        }
                        CallStatus::Abandoned => {
                            let at = call.closed_at.ok_or({
                                Error::InvalidCallRetention(deadline::Error::MissingClosureInstant)
                            })?;
                            journal.abandon_egress(&call.intent.call_id, at)?;
                        }
                    }
                }
                Record::Egress(EgressRecord::Session(session)) => match &session.status {
                    SessionStatus::Opening(intent) => {
                        journal.open_outbound_session(intent.clone())?
                    }
                    SessionStatus::Opened => {
                        journal.confirm_session_open(&session.session_id)?;
                    }
                    SessionStatus::WriteOpen(intent) => {
                        journal.open_session_write(intent.clone())?
                    }
                    SessionStatus::WriteSettled(reference) => {
                        journal.settle_session_write(reference, false)?;
                    }
                    SessionStatus::WriteAbandoned(reference) => {
                        journal.settle_session_write(reference, true)?;
                    }
                },
            }
            if journal.records.len() != previous + 1 {
                return Err(Error::InvalidRetainedRecord("duplicate settled fact"));
            }
        }
        Ok(journal)
    }

    fn statuses_of(&self, call_id: &CallId) -> impl Iterator<Item = &CallStatus> {
        self.records.iter().filter_map(move |record| match record {
            Record::Egress(EgressRecord::Call(outbound)) if outbound.intent.call_id == *call_id => {
                Some(&outbound.status)
            }
            _ => None,
        })
    }

    fn route_generation_is_valid(
        &self,
        source: Option<&instance::Generation>,
        target: &instance::Generation,
    ) -> Result<bool, Error> {
        let admission = self.instance_generation_for_admission(target.address())?;
        if target == &admission {
            return Ok(true);
        }
        let Some(source) = source else {
            return Ok(false);
        };
        let Some(scope) = self.admissions.closing_scope(source) else {
            return Ok(false);
        };
        Ok(self
            .admissions
            .active_instance_ref(target.address())
            .as_ref()
            == Some(target)
            && self.admissions.instance_is_within(source, &scope)
            && self.admissions.instance_is_within(target, &scope))
    }

    fn route_was_reserved(&self, target: &instance::Generation) -> bool {
        self.records.iter().any(|record| match record {
            Record::Ingress(inbound) => {
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
            Record::Egress(_) => false,
        })
    }

    fn ancestor_route_was_reserved(
        &self,
        target: &instance::Generation,
        parent: &instance::Generation,
        cause: Option<Id>,
    ) -> bool {
        let Some(cause) = cause else { return false };
        let after_stop = self
            .records
            .iter()
            .rposition(|record| {
                matches!(record,
                    Record::Transaction(record)
                        if record.instance.address() == target.address()
                            && record.trigger == node::Trigger::Lifecycle(node::Lifecycle::Stop)
                )
            })
            .map_or(0, |index| index + 1);
        self.records
            .iter()
            .enumerate()
            .skip(after_stop)
            .any(|(index, record)| {
                let Record::Transaction(record) = record else {
                    return false;
                };
                if !record.routes.iter().any(|route| {
                    route.delivery == cause
                        && route.target.address().owner() != target.address().owner()
                        && target.address().contains_address(route.target.address())
                }) {
                    return false;
                }
                // The reserved leaf also admits its as-yet-unstarted Graph
                // ancestors. The producer must belong to this exact parent
                // generation at its commit, not merely share a lexical path.
                // Its stop may already have committed by the time the child starts.
                let mut source = &record.instance;
                loop {
                    if source == parent {
                        return true;
                    }
                    let ancestor =
                        self.records[..index]
                            .iter()
                            .rev()
                            .find_map(|record| match record {
                                Record::Transaction(record)
                                    if record.instance == *source
                                        && record.trigger
                                            == node::Trigger::Lifecycle(node::Lifecycle::Start) =>
                                {
                                    record.parent_instance.as_ref()
                                }
                                _ => None,
                            });
                    let Some(ancestor) = ancestor else {
                        return false;
                    };
                    source = ancestor;
                }
            })
    }

    fn prepare_ingress_fact(
        &self,
        submission: IngressProposal,
        connector_path: Option<node::Path>,
    ) -> Result<PreparedIngress, Error> {
        let next = self
            .next_seq
            .get(&submission.provenance)
            .copied()
            .unwrap_or(0);
        let seq = match submission.sequence {
            IngressSequence::Next => next,
            IngressSequence::Explicit(seq) if seq == next => seq,
            IngressSequence::Explicit(seq) => {
                return Err(Error::SequenceMismatch {
                    expected: next,
                    got: seq,
                });
            }
        };
        seq.checked_add(1).ok_or(Error::SequenceOverflow)?;
        let id = ids::root_id(&submission.provenance, seq);
        let resolved_exchange = match submission.session {
            None => None,
            Some(InboundSession::Open) => {
                let reference = ids::inbound_session_id(&submission.provenance, id);
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
        Ok(PreparedIngress {
            submission,
            connector_path,
            seq,
            envelope,
            exchange: resolved_exchange,
            deadlines: None,
            instance_commands: None,
        })
    }
}

impl Journal for MemJournal {
    fn origin(&self) -> crate::time::Timestamp {
        self.origin
    }

    fn visit_records(&self, visit: &mut dyn FnMut(&Record)) -> Result<(), Error> {
        for record in &self.records {
            visit(record);
        }
        Ok(())
    }

    fn loop_protocol(&self) -> Result<loop_protocol::LoopView, Error> {
        match self.command_loops.as_deref() {
            Some(fold) => Ok(fold.clone()),
            None => loop_protocol::LoopView::from_records(&self.records)
                .map_err(Error::InvalidLoopProtocol),
        }
    }
    fn instance_commands(&self) -> Result<instance_stop::InstanceCommandView, Error> {
        instance_stop::InstanceCommandView::from_records(&self.records)
            .map_err(Error::InvalidInstanceCommand)
    }
    fn deployment_attestation(&self) -> Option<&crate::deployment::DeploymentAttestation> {
        self.deployment_attestation.as_ref()
    }

    fn privacy_retention(&self) -> Option<crate::deployment::PrivacyRetention> {
        Some(crate::deployment::PrivacyRetention::RunScoped)
    }

    fn retained_observations_from(&self, start: usize) -> Vec<RecordObservation> {
        observations_for_records_from(&self.records, start)
    }

    fn prepare_ingress(&self, submission: IngressProposal) -> Result<PreparedIngress, Error> {
        self.prepare_ingress_fact(submission, None)
    }

    fn prepare_connector_ingress(
        &self,
        submission: IngressProposal,
    ) -> Result<PreparedIngress, Error> {
        let connector_path = submission.provenance.connector_path().clone();
        self.prepare_ingress_fact(submission, Some(connector_path))
    }

    fn commit_ingress(
        &mut self,
        prepared: PreparedIngress,
        mut routes: Vec<RouteTarget>,
    ) -> Result<CommittedIngress, Error> {
        if let (Some(schedule), Some(InboundExchangeRecord::Call(call))) =
            (prepared.deadlines.as_deref(), prepared.exchange.as_ref())
        {
            acceptance::require_deadline_instants(schedule, &call.deadline_ids)
                .map_err(Error::InvalidDeadlines)?;
        }
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
                if call.reference != InboundCallId::from_root(root)
                    || call.instance.address().owner().segments() != connector_path.segments()
                {
                    return Err(Error::CorruptInboundCall);
                }
                if self.inbound_calls.contains_key(&call.reference) {
                    return Err(Error::DuplicateInboundCall);
                }
                let expected_timeout = (DeadlineKind::InboundCallTimeout, call.reference.id());
                if !(call.deadline_ids.is_empty()
                    || call.deadline_ids.as_slice() == [expected_timeout])
                {
                    return Err(Error::CorruptInboundCall);
                }
                let expected = self.instance_generation_for_admission(call.instance.address())?;
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
            let expected = self.instance_generation_for_admission(route.target.address())?;
            if route.target != expected {
                return Err(Error::InstanceGenerationMismatch {
                    expected,
                    got: route.target.clone(),
                });
            }
        }
        routes.sort();
        let record = IngressRecord {
            instance_commands: prepared.instance_commands,
            deadlines: prepared.deadlines,
            provenance: prepared.submission.provenance.clone(),
            port: prepared.submission.draft.port,
            connector_path: prepared.connector_path,
            seq: prepared.seq,
            envelope: prepared.envelope.clone(),
            routes,
            // Exchange framing is absent on Event records. Box the extension
            // so `None` does not widen every causal `Record` retained by an
            // embedded journal.
            exchange: prepared.exchange.clone().map(Box::new),
        };
        if record.instance_commands.is_some() {
            self.instance_commands()?
                .apply(&Record::Ingress(Box::new(record.clone())))
                .map_err(Error::InvalidInstanceCommand)?;
        }
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
                        deadline_ids: call.deadline_ids,
                        open: true,
                    },
                );
            }
            Some(InboundExchangeRecord::Session(SessionIngressRecord::Message(_))) | None => {}
        }
        self.next_seq
            .insert(prepared.submission.provenance, following);
        self.records.push(Record::Ingress(Box::new(record.clone())));
        Ok(CommittedIngress {
            seq: prepared.seq,
            envelope: prepared.envelope,
            record,
        })
    }

    fn commit(&mut self, mut transaction: TransactionRecord) -> Result<(), Error> {
        retry_protocol::validate_transaction(&transaction).map_err(Error::InvalidRetryProtocol)?;
        acceptance::validate_transaction(&transaction, self.origin)
            .map_err(Error::InvalidDeadlines)?;
        if transaction.instance_commands.is_some() {
            self.instance_commands()?
                .apply(&Record::Transaction(Box::new(transaction.clone())))
                .map_err(Error::InvalidInstanceCommand)?;
        }
        let command_loops = if transaction.loop_protocol().is_some()
            || (self.command_loops.is_some()
                && matches!(
                    transaction.trigger,
                    node::Trigger::Lifecycle(node::Lifecycle::Start | node::Lifecycle::Stop)
                )) {
            let mut fold = self.loop_protocol()?;
            fold.apply_transaction(&transaction)
                .map_err(Error::InvalidLoopProtocol)?;
            Some(Box::new(fold))
        } else {
            None
        };
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
        let lifecycle = self.admissions.prepare(&transaction)?;
        if let Some(scope) = &transaction.close_scope
            && (is_start || is_stop || scope != &transaction.instance)
        {
            return Err(Error::InvalidCloseScope {
                instance: transaction.instance,
                scope: scope.clone(),
            });
        }
        if transaction.close_scope.is_some()
            && let Some(scope) = self.admissions.closing_scope(&transaction.instance)
        {
            return Err(Error::InstanceAlreadyClosing {
                instance: transaction.instance,
                scope,
            });
        }
        if is_stop
            && self
                .admissions
                .closing_scope(&transaction.instance)
                .is_none()
        {
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
            let Some(expected_parent) = self.admissions.active_instance_ref(parent.address())
            else {
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
            if self.admissions.closing_scope(parent).is_some()
                && !self.route_was_reserved(&transaction.instance)
                && !self.ancestor_route_was_reserved(
                    &transaction.instance,
                    parent,
                    transaction.cause,
                )
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
        ) && self.admissions.has_active_children(&transaction.instance)
        {
            return Err(Error::OpenChildInstances {
                parent: transaction.instance,
            });
        }
        for route in &transaction.routes {
            if !self.route_generation_is_valid(Some(&transaction.instance), &route.target)? {
                let expected = self.instance_generation_for_admission(route.target.address())?;
                return Err(Error::InstanceGenerationMismatch {
                    expected,
                    got: route.target.clone(),
                });
            }
        }
        if let Some(InboundExchangeExitRecord::Session(session_id)) =
            transaction.exchange_exit.as_deref().copied()
        {
            let Some(session) = self.sessions.get(&session_id) else {
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
                )) if value == &session_id.to_string()
            ) {
                return Err(Error::SessionProjectionMismatch);
            }
        }
        if let Some(InboundExchangeExitRecord::Call(call_id)) =
            transaction.exchange_exit.as_deref().copied()
        {
            let Some(call) = self.inbound_calls.get(&call_id) else {
                return Err(Error::UnknownInboundCall);
            };
            if !call.open
                || call.connector_path.segments() != transaction.owner_path.segments()
                || call.instance != transaction.instance
            {
                return Err(Error::UnknownInboundCall);
            }
            if transaction.trigger != node::Trigger::Lifecycle(node::Lifecycle::Reply)
                || transaction.cause != Some(call_id.id())
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
        transaction.deadline_ids.sort();
        self.admissions.commit(&transaction, lifecycle)?;
        let exchange_exit = transaction.exchange_exit.as_deref().copied();
        self.records
            .push(Record::Transaction(Box::new(transaction)));
        if let Some(fold) = command_loops {
            self.command_loops = Some(fold);
        }
        if let Some(InboundExchangeExitRecord::Session(session_id)) = exchange_exit
            && let Some(session) = self.sessions.get_mut(&session_id)
        {
            session.phase = SessionPhase::Exited;
        }
        if let Some(InboundExchangeExitRecord::Call(call_id)) = exchange_exit
            && let Some(call) = self.inbound_calls.get_mut(&call_id)
        {
            call.open = false;
        }
        Ok(())
    }

    fn instance_generation_for_admission(
        &self,
        address: &instance::InstanceAddress,
    ) -> Result<instance::Generation, Error> {
        self.admissions.instance_generation_for_admission(address)
    }

    fn open_egress(
        &mut self,
        intent: CallOpen,
        attempt: Option<retry_protocol::AttemptId>,
    ) -> Result<(), Error> {
        // The autonomy check: the call identity must recompute from
        // the fields this record seals (keys 27, 30, 33).
        let derived = ids::call_id(
            ids::delivery_id(&intent.edge, intent.emitting_id),
            intent.request_digest(),
        );
        if derived != intent.call_id {
            return Err(Error::CorruptIntent);
        }
        if self.statuses_of(&intent.call_id).next().is_some() {
            return Err(Error::DuplicateOpen);
        }
        let record = CallRecord {
            intent,
            status: CallStatus::Open,
            closed_at: None,
            deadlines: None,
            retry_record: attempt.map(|context| {
                Box::new(retry_protocol::RetryRecord {
                    context: Some(context),
                    facts: Vec::new(),
                })
            }),
        };
        acceptance::validate_call(&record).map_err(Error::InvalidCallRetention)?;
        self.records
            .push(Record::Egress(EgressRecord::Call(Box::new(record))));
        Ok(())
    }

    fn open_outbound_session(&mut self, intent: SessionOpen) -> Result<(), Error> {
        let derived = ids::outbound_session_id(
            &intent.connector_path,
            intent.emitting_id,
            intent.request_digest(),
        );
        if derived != intent.session_id {
            return Err(Error::CorruptSessionOpen);
        }
        if self.sessions.contains_key(&intent.session_id) {
            return Err(Error::DuplicateSessionOpen);
        }
        let reference = intent.session_id;
        let connector_path = intent.connector_path.clone();
        self.records
            .push(Record::Egress(EgressRecord::Session(Box::new(
                SessionRecord {
                    connector_path: connector_path.clone(),
                    session_id: reference,
                    status: SessionStatus::Opening(intent),
                },
            ))));
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

    fn confirm_session_open(&mut self, session_id: &SessionId) -> Result<CloseVerdict, Error> {
        let Some(session) = self.sessions.get_mut(session_id) else {
            return Err(Error::UnknownSession);
        };
        match session.phase {
            SessionPhase::Opening => session.phase = SessionPhase::Open,
            SessionPhase::Open => return Ok(CloseVerdict::AlreadyClosed),
            SessionPhase::Exited => return Err(Error::UnknownSession),
        }
        let connector_path = session.connector_path.clone();
        self.records
            .push(Record::Egress(EgressRecord::Session(Box::new(
                SessionRecord {
                    connector_path,
                    session_id: *session_id,
                    status: SessionStatus::Opened,
                },
            ))));
        Ok(CloseVerdict::Won)
    }

    fn open_session_write(&mut self, intent: SessionWrite) -> Result<(), Error> {
        let derived = ids::session_write_id(
            intent.session_id,
            intent.emitting_id,
            intent.message_digest(),
        );
        if derived != intent.write_ref {
            return Err(Error::CorruptSessionWrite);
        }
        let Some(session) = self.sessions.get_mut(&intent.session_id) else {
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
            .push(Record::Egress(EgressRecord::Session(Box::new(
                SessionRecord {
                    connector_path: intent.connector_path.clone(),
                    session_id: intent.session_id,
                    status: SessionStatus::WriteOpen(intent),
                },
            ))));
        Ok(())
    }

    fn settle_session_write(
        &mut self,
        write_ref: &SessionWriteId,
        abandoned: bool,
    ) -> Result<CloseVerdict, Error> {
        let Some((session_id, session)) = self
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
            .push(Record::Egress(EgressRecord::Session(Box::new(
                SessionRecord {
                    connector_path: session.connector_path.clone(),
                    session_id: *session_id,
                    status,
                },
            ))));
        Ok(CloseVerdict::Won)
    }

    fn close_egress(
        &mut self,
        call_id: &CallId,
        closure: Closure,
        closed_at: crate::time::LogicalTime,
        schedule: Option<Box<deadline::Deadlines>>,
    ) -> Result<CloseVerdict, Error> {
        let mut intent = None;
        for record in &self.records {
            let Record::Egress(EgressRecord::Call(outbound)) = record else {
                continue;
            };
            if outbound.intent.call_id != *call_id {
                continue;
            }
            match outbound.status {
                CallStatus::Open => {
                    intent = Some((outbound.intent.clone(), outbound.retry_record.clone()))
                }
                CallStatus::Closed(_) | CallStatus::Abandoned => {
                    return Ok(CloseVerdict::AlreadyClosed);
                }
            }
        }
        let Some((intent, retry_record)) = intent else {
            return Err(Error::UnknownCall);
        };
        // Append the closure as a new fact; the open fact stays
        // byte-identical — the attempt trace survives.
        let record = CallRecord {
            intent,
            status: CallStatus::Closed(closure),
            closed_at: Some(closed_at),
            deadlines: schedule,
            retry_record,
        };
        acceptance::validate_call(&record).map_err(Error::InvalidCallRetention)?;
        self.records
            .push(Record::Egress(EgressRecord::Call(Box::new(record))));
        Ok(CloseVerdict::Won)
    }

    fn abandon_egress(
        &mut self,
        call_id: &CallId,
        closed_at: crate::time::LogicalTime,
    ) -> Result<CloseVerdict, Error> {
        let mut intent = None;
        for record in &self.records {
            let Record::Egress(EgressRecord::Call(outbound)) = record else {
                continue;
            };
            if outbound.intent.call_id != *call_id {
                continue;
            }
            match outbound.status {
                CallStatus::Open => {
                    intent = Some((outbound.intent.clone(), outbound.retry_record.clone()))
                }
                CallStatus::Closed(_) | CallStatus::Abandoned => {
                    return Ok(CloseVerdict::AlreadyClosed);
                }
            }
        }
        let Some((intent, retry_record)) = intent else {
            return Err(Error::UnknownCall);
        };
        let record = CallRecord {
            intent,
            status: CallStatus::Abandoned,
            closed_at: Some(closed_at),
            deadlines: None,
            retry_record,
        };
        acceptance::validate_call(&record).map_err(Error::InvalidCallRetention)?;
        self.records
            .push(Record::Egress(EgressRecord::Call(Box::new(record))));
        Ok(CloseVerdict::Won)
    }

    fn find_envelope(&self, id: Id) -> Option<Envelope> {
        recorded_envelope(&self.records, id).cloned()
    }

    fn find_call(&self, call_id: &CallId) -> Option<CallOpen> {
        self.records.iter().find_map(|record| match record {
            Record::Egress(EgressRecord::Call(outbound)) if outbound.intent.call_id == *call_id => {
                Some(outbound.intent.clone())
            }
            _ => None,
        })
    }

    fn open_calls(&self) -> Vec<CallOpen> {
        let mut open: Vec<CallOpen> = Vec::new();
        for record in &self.records {
            let Record::Egress(EgressRecord::Call(outbound)) = record else {
                continue;
            };
            match outbound.status {
                CallStatus::Open => open.push(outbound.intent.clone()),
                CallStatus::Closed(_) | CallStatus::Abandoned => {
                    open.retain(|intent| intent.call_id != outbound.intent.call_id);
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
                deadline_ids: lifecycle.deadline_ids.clone(),
            })
            .collect()
    }

    fn live_sessions(&self) -> Vec<SessionId> {
        self.sessions
            .iter()
            .filter_map(|(reference, lifecycle)| {
                (lifecycle.phase == SessionPhase::Open).then_some(*reference)
            })
            .collect()
    }

    fn open_session_writes(&self) -> Vec<SessionWrite> {
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
                Record::Egress(EgressRecord::Session(session)) => match &session.status {
                    SessionStatus::WriteOpen(intent) if open.contains(&intent.write_ref) => {
                        Some(intent.clone())
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connector::exchange::EnvelopeDraft;
    use crate::envelope::{Data, Encoding, Namespace};
    use alloc::format;
    use alloc::string::ToString;
    use alloc::vec;

    fn instance(owner: &str, generation: u64) -> instance::Generation {
        let owner = crate::node::OwnerPath::new([owner]).expect("valid owner path");
        instance::Generation::new(instance::InstanceAddress::singleton(&owner), generation)
    }

    fn provenance(path: &str) -> IngressProvenance {
        IngressProvenance::single(node::Path::new([path]).unwrap())
    }

    fn submission(path: &str, sequence: IngressSequence) -> IngressProposal {
        IngressProposal {
            provenance: provenance(path),
            draft: EnvelopeDraft {
                port: node::Port::new("in").unwrap(),
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
            session: None,
        }
    }

    fn project_session(envelope: &Envelope, session: SessionId) -> Envelope {
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

    fn intent(connector: &str, digest: u8) -> CallOpen {
        let connector_path = node::Path::new([connector]).unwrap();
        let emitting_id = Id::derive(crate::envelope::IdKind::Emission, &[b"cause"]);
        let edge = alloc::format!("shout.out>{connector}.in");
        let prepared = crate::connector::exchange::PreparedRequest::seal(vec![digest; 4]);
        CallOpen {
            call_id: ids::call_id(ids::delivery_id(&edge, emitting_id), prepared.digest()),
            emitting_id,
            edge,
            connector_path,
            prepared,
            idempotency_key: Some(String::from("op-1")),
            resolved_settings: BTreeMap::new(),
            logical_time: crate::time::LogicalTime::ZERO,
        }
    }

    #[test]
    fn call_timing_refusals_keep_their_typed_cause() {
        use deadline::{Deadline, Deadlines, Error as DeadlineError};
        let mut call = CallRecord {
            intent: intent("clocked", 6),
            status: CallStatus::Open,
            closed_at: Some(crate::time::LogicalTime::ZERO),
            deadlines: None,
            retry_record: None,
        };
        assert_eq!(
            acceptance::validate_call(&call),
            Err(DeadlineError::UnexpectedClosureInstant)
        );
        call.status = CallStatus::Closed(Closure::Timeout);
        call.closed_at = None;
        assert_eq!(
            acceptance::validate_call(&call),
            Err(DeadlineError::MissingClosureInstant)
        );
        call.closed_at = Some(crate::time::LogicalTime::from_nanoseconds(-1));
        assert_eq!(
            acceptance::validate_call(&call),
            Err(DeadlineError::ClosurePrecedesOpening)
        );
        call.closed_at = Some(crate::time::LogicalTime::ZERO);
        let attempt = retry_protocol::AttemptId {
            operation: retry_protocol::OperationId::from_id(Id::from_bytes([8; 16])),
            ordinal: 1,
        };
        call.retry_record = Some(Box::new(retry_protocol::RetryRecord {
            context: None,
            facts: vec![],
        }));
        assert_eq!(
            acceptance::validate_call(&call),
            Err(DeadlineError::MissingRetryContext)
        );
        call.retry_record.as_deref_mut().unwrap().context = Some(retry_protocol::AttemptId {
            ordinal: 0,
            ..attempt
        });
        assert_eq!(
            acceptance::validate_call(&call),
            Err(DeadlineError::InvalidRetryContext)
        );
        call.retry_record = None;
        let deadline = Deadline {
            kind: DeadlineKind::RecurrenceDelay,
            reference: retry_protocol::admission_ref(attempt.operation, 2, call.intent.call_id),
            due_at: crate::time::LogicalTime::from_nanoseconds(10),
        };
        call.deadlines = Some(Box::new(
            Deadlines::try_new(crate::time::LogicalTime::ZERO, vec![deadline]).unwrap(),
        ));
        assert_eq!(
            acceptance::validate_call(&call),
            Err(DeadlineError::DeadlineWithoutRetryContext)
        );
        call.retry_record = Some(Box::new(retry_protocol::RetryRecord {
            context: Some(retry_protocol::AttemptId {
                ordinal: u64::MAX,
                ..attempt
            }),
            facts: vec![],
        }));
        assert_eq!(
            acceptance::validate_call(&call),
            Err(DeadlineError::RetryOrdinalOverflow)
        );
        call.retry_record.as_deref_mut().unwrap().context = Some(attempt);
        call.closed_at = Some(crate::time::LogicalTime::from_nanoseconds(1));
        assert_eq!(
            acceptance::validate_call(&call),
            Err(DeadlineError::DeadlineMismatch)
        );
        call.closed_at = Some(crate::time::LogicalTime::ZERO);
        assert_eq!(acceptance::validate_call(&call), Ok(()));
    }

    #[test]
    fn transaction_deadlines_must_match_sealed_ids_time_and_loop_advances() {
        use deadline::{Deadline, Deadlines, Error as DeadlineError};
        let mut record = lifecycle_transaction(instance("clocked", 0), node::Lifecycle::Loop);
        let mut deadline = Deadline {
            kind: DeadlineKind::CallTimeout,
            reference: Id::from_bytes([7; 16]),
            due_at: crate::time::LogicalTime::from_nanoseconds(10),
        };
        record.deadlines = Some(Box::new(
            Deadlines::try_new(crate::time::LogicalTime::ZERO, vec![deadline]).unwrap(),
        ));
        record.observed_time = Some(crate::time::Timestamp::from_nanoseconds(1));
        assert_eq!(
            acceptance::validate_transaction(&record, crate::time::Timestamp::UNIX_EPOCH),
            Err(DeadlineError::FiringTimeDisagrees)
        );
        record.observed_time = Some(crate::time::Timestamp::UNIX_EPOCH);
        record.deadline_ids = vec![(DeadlineKind::CallTimeout, Id::from_bytes([6; 16]))];
        assert_eq!(
            acceptance::validate_transaction(&record, crate::time::Timestamp::UNIX_EPOCH),
            Err(DeadlineError::SealedDeadlineWithoutInstant)
        );
        record.deadline_ids = vec![(deadline.kind, deadline.reference)];
        assert_eq!(
            acceptance::validate_transaction(&record, crate::time::Timestamp::UNIX_EPOCH),
            Ok(())
        );

        let run = LoopRunId::new(Id::from_bytes([9; 16]));
        let command = loop_protocol::CommandProduced::new(
            run,
            0,
            loop_protocol::CommandValue::Again(Id::from_bytes([3; 16])),
        );
        let state = MemJournal::new(GraphVersion(1))
            .prepare_connector_ingress(submission("clocked", IngressSequence::Next))
            .unwrap()
            .envelope()
            .clone();
        record.loop_record = Some(Box::new(loop_protocol::LoopRecord {
            facts: vec![loop_protocol::Fact::Advanced {
                run,
                from_iteration: 0,
                to_iteration: 1,
                remaining: 1,
                command: command.reference,
                state,
                due_at: Some(crate::time::LogicalTime::from_nanoseconds(10)),
            }],
            ..Default::default()
        }));
        assert_eq!(
            acceptance::validate_transaction(&record, crate::time::Timestamp::UNIX_EPOCH),
            Err(DeadlineError::LoopDeadlineMismatch)
        );
        deadline.kind = DeadlineKind::RecurrenceDelay;
        deadline.reference = loop_protocol::admission_ref(run, 1, command.reference);
        record.deadline_ids = vec![(deadline.kind, deadline.reference)];
        record.deadlines = Some(Box::new(
            Deadlines::try_new(crate::time::LogicalTime::ZERO, vec![deadline]).unwrap(),
        ));
        assert_eq!(
            acceptance::validate_transaction(&record, crate::time::Timestamp::UNIX_EPOCH),
            Ok(())
        );
        record.deadlines = Some(Box::new(
            Deadlines::try_new(crate::time::LogicalTime::ZERO, vec![deadline, deadline]).unwrap(),
        ));
        assert_eq!(
            acceptance::validate_transaction(&record, crate::time::Timestamp::UNIX_EPOCH),
            Err(DeadlineError::LoopDeadlineMismatch)
        );
        record.deadlines = Some(Box::new(
            Deadlines::try_new(crate::time::LogicalTime::ZERO, vec![deadline]).unwrap(),
        ));
        record.loop_record = None;
        assert_eq!(
            acceptance::validate_transaction(&record, crate::time::Timestamp::UNIX_EPOCH),
            Err(DeadlineError::LoopDeadlineMismatch)
        );
    }

    #[test]
    fn every_winning_call_closure_retains_its_time_without_retry() {
        for abandoned in [false, true] {
            let mut journal = MemJournal::new(GraphVersion(1));
            let original = intent("clocked", 3);
            let call = original.call_id;
            journal.open_egress(original, None).unwrap();
            let open = journal.records()[0].clone();
            if abandoned {
                journal
                    .abandon_egress(&call, crate::time::LogicalTime::from_nanoseconds(17))
                    .unwrap();
            } else {
                journal
                    .close_egress(
                        &call,
                        Closure::Timeout,
                        crate::time::LogicalTime::from_nanoseconds(17),
                        None,
                    )
                    .unwrap();
            }
            assert_eq!(journal.records()[0], open);
            let Record::Egress(EgressRecord::Call(closed)) = &journal.records()[1] else {
                unreachable!()
            };
            assert_eq!(
                closed.closed_at,
                Some(crate::time::LogicalTime::from_nanoseconds(17))
            );
            assert!(closed.retry_record.is_none());
            assert!(closed.deadlines.is_none());
            assert_eq!(
                journal
                    .close_egress(
                        &call,
                        Closure::Timeout,
                        crate::time::LogicalTime::from_nanoseconds(29),
                        None
                    )
                    .unwrap(),
                CloseVerdict::AlreadyClosed
            );
            let recovered = MemJournal::from_records(
                GraphVersion(1),
                crate::time::Timestamp::UNIX_EPOCH,
                None,
                journal.records(),
            )
            .unwrap();
            assert_eq!(recovered.records(), journal.records());
            let mut reader = codec::Decoder::default();
            for record in journal.records() {
                assert_eq!(reader.decode(&codec::encode(record)).unwrap(), *record);
            }
        }
    }

    #[test]
    fn call_retention_refuses_missing_backdated_or_recorrelated_closures() {
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut original = intent("clocked", 4);
        original.logical_time = crate::time::LogicalTime::from_nanoseconds(11);
        let call = original.call_id;
        journal.open_egress(original, None).unwrap();
        assert!(matches!(
            journal.close_egress(
                &call,
                Closure::Timeout,
                crate::time::LogicalTime::from_nanoseconds(10),
                None
            ),
            Err(Error::InvalidCallRetention(_))
        ));
        assert_eq!(journal.records().len(), 1);
        journal
            .close_egress(
                &call,
                Closure::Timeout,
                crate::time::LogicalTime::from_nanoseconds(11),
                None,
            )
            .unwrap();
        for mutation in 0..3 {
            let mut records = journal.records().to_vec();
            let Record::Egress(EgressRecord::Call(closed)) = &mut records[1] else {
                unreachable!()
            };
            match mutation {
                0 => closed.closed_at = None,
                1 => closed.intent.idempotency_key = Some("changed".into()),
                _ => {
                    closed.retry_record = Some(Box::new(retry_protocol::RetryRecord {
                        context: Some(retry_protocol::AttemptId {
                            operation: retry_protocol::OperationId::from_id(Id::from_bytes(
                                [3; 16],
                            )),
                            ordinal: 1,
                        }),
                        facts: vec![],
                    }))
                }
            }
            assert!(
                MemJournal::from_records(
                    GraphVersion(1),
                    crate::time::Timestamp::UNIX_EPOCH,
                    None,
                    &records
                )
                .is_err()
            );
        }
        let mut encoded = codec::encode(&journal.records()[1]);
        let crate::canonical_cbor::Value::Array(fields) = &mut encoded else {
            unreachable!()
        };
        fields[3] = crate::canonical_cbor::Value::Null;
        assert!(codec::Decoder::default().decode(&encoded).is_err());
    }

    #[test]
    fn retry_call_carriers_retain_only_their_own_next_admission_deadline() {
        let original = intent("retrying", 5);
        let call = original.call_id;
        let operation = retry_protocol::OperationId::from_id(Id::from_bytes([7; 16]));
        let attempt = retry_protocol::AttemptId {
            operation,
            ordinal: 1,
        };
        let deadline = deadline::Deadline {
            kind: DeadlineKind::RecurrenceDelay,
            reference: retry_protocol::admission_ref(operation, 2, call),
            due_at: crate::time::LogicalTime::from_nanoseconds(110),
        };
        let schedule = deadline::Deadlines::try_new(
            crate::time::LogicalTime::from_nanoseconds(10),
            vec![deadline],
        )
        .unwrap();
        let mut journal = MemJournal::new(GraphVersion(1));
        journal.open_egress(original.clone(), None).unwrap();
        assert!(matches!(
            journal.close_egress(
                &call,
                Closure::Timeout,
                crate::time::LogicalTime::from_nanoseconds(10),
                Some(Box::new(schedule.clone()))
            ),
            Err(Error::InvalidCallRetention(_))
        ));
        let mut journal = MemJournal::new(GraphVersion(1));
        journal.open_egress(original, Some(attempt)).unwrap();
        for mutation in 0..2 {
            let mut wrong = deadline;
            if mutation == 0 {
                wrong.reference = Id::from_bytes([9; 16]);
            } else {
                wrong.kind = DeadlineKind::CallTimeout;
            }
            let wrong = deadline::Deadlines::try_new(
                crate::time::LogicalTime::from_nanoseconds(10),
                vec![wrong],
            )
            .unwrap();
            assert!(
                journal
                    .close_egress(
                        &call,
                        Closure::Timeout,
                        crate::time::LogicalTime::from_nanoseconds(10),
                        Some(Box::new(wrong))
                    )
                    .is_err()
            );
        }
        journal
            .close_egress(
                &call,
                Closure::Timeout,
                crate::time::LogicalTime::from_nanoseconds(10),
                Some(Box::new(schedule)),
            )
            .unwrap();
        let recovered = MemJournal::from_records(
            GraphVersion(1),
            crate::time::Timestamp::UNIX_EPOCH,
            None,
            journal.records(),
        )
        .unwrap();
        assert_eq!(recovered.records(), journal.records());
        let mut reader = codec::Decoder::default();
        for record in journal.records() {
            assert_eq!(reader.decode(&codec::encode(record)).unwrap(), *record);
        }
    }

    fn session_open(connector: &str, digest: u8) -> SessionOpen {
        let connector_path = node::Path::new([connector]).unwrap();
        let emitting_id = Id::derive(
            crate::envelope::IdKind::Emission,
            &[b"session-cause", &[digest]],
        );
        let prepared = crate::connector::exchange::PreparedRequest::seal(vec![digest; 4]);
        SessionOpen {
            session_id: ids::outbound_session_id(&connector_path, emitting_id, prepared.digest()),
            emitting_id,
            connector_path,
            prepared,
        }
    }

    fn session_write(session_id: SessionId, connector: &str, digest: u8) -> SessionWrite {
        let connector_path = node::Path::new([connector]).unwrap();
        let emitting_id = Id::derive(
            crate::envelope::IdKind::Emission,
            &[b"message-cause", &[digest]],
        );
        let prepared = crate::connector::exchange::PreparedRequest::seal(vec![digest; 3]);
        SessionWrite {
            write_ref: ids::session_write_id(session_id, emitting_id, prepared.digest()),
            session_id,
            emitting_id,
            connector_path,
            prepared,
        }
    }

    fn lifecycle_transaction(
        instance: instance::Generation,
        lifecycle: node::Lifecycle,
    ) -> TransactionRecord {
        let parent_instance = (lifecycle == node::Lifecycle::Start
            && !instance.address().owner().is_root())
        .then(|| {
            instance::Generation::new(
                instance::InstanceAddress::singleton(&crate::node::OwnerPath::root()),
                0,
            )
        });
        TransactionRecord {
            owner_path: instance.address().owner().clone(),
            instance,
            parent_instance,
            close_scope: None,
            exchange_exit: None,
            retry_record: None,
            loop_record: None,
            observed_time: None,
            deadlines: None,
            instance_commands: None,
            trigger: node::Trigger::Lifecycle(lifecycle),
            cause: None,
            consumed: vec![],
            join_inputs: vec![],
            state_delta: vec![],
            emissions: vec![],
            routes: vec![],
            deadline_ids: vec![],
        }
    }

    fn activate_root(journal: &mut MemJournal) {
        let root = instance::Generation::new(
            instance::InstanceAddress::singleton(&crate::node::OwnerPath::root()),
            0,
        );
        journal
            .commit(lifecycle_transaction(root, node::Lifecycle::Start))
            .expect("the root lifecycle starts");
    }

    #[test]
    fn reserved_descendants_prove_ancestor_admission_but_not_another_key_or_generation() {
        let mut journal = MemJournal::new(GraphVersion(1));
        activate_root(&mut journal);
        let parent = instance::Generation::new(
            instance::InstanceAddress::singleton(&node::OwnerPath::new(["window"]).unwrap()),
            0,
        );
        journal
            .commit(lifecycle_transaction(
                parent.clone(),
                node::Lifecycle::Start,
            ))
            .unwrap();
        let source = instance::Generation::new(
            instance::InstanceAddress::singleton(
                &node::OwnerPath::new(["window", "total"]).unwrap(),
            ),
            0,
        );
        let mut start = lifecycle_transaction(source.clone(), node::Lifecycle::Start);
        start.parent_instance = Some(parent.clone());
        journal.commit(start).unwrap();
        let outside = instance::Generation::new(
            instance::InstanceAddress::singleton(&node::OwnerPath::new(["outside"]).unwrap()),
            0,
        );
        journal
            .commit(lifecycle_transaction(
                outside.clone(),
                node::Lifecycle::Start,
            ))
            .unwrap();
        request_close(&mut journal, parent.clone());
        let owner = node::OwnerPath::new(["window", "bridge"]).unwrap();
        let address = |path: &node::OwnerPath, key: &str| {
            instance::InstanceAddress::new(
                path,
                &[instance::Segment::SelectedKey {
                    scope: owner.clone(),
                    ty: crate::datatype::DataType::Text,
                    value: crate::value::Value::Text(key.into()),
                }],
            )
            .unwrap()
        };
        let child = instance::Generation::new(address(&owner, "alice"), 0);
        let leaf = instance::Generation::new(
            address(
                &node::OwnerPath::new(["window", "bridge", "relay"]).unwrap(),
                "alice",
            ),
            0,
        );
        let cause = Id::from_bytes([1; 16]);
        let mut stop = lifecycle_transaction(source, node::Lifecycle::Stop);
        stop.routes.push(RoutedDelivery {
            delivery: cause,
            edge: "window.total.out>window.bridge.relay.in".into(),
            target: leaf.clone(),
        });
        journal.commit(stop).unwrap();
        let mut external = lifecycle_transaction(outside, node::Lifecycle::Recover);
        external.trigger = node::Trigger::Port(node::Port::new("in").unwrap());
        external.routes.push(RoutedDelivery {
            delivery: Id::from_bytes([2; 16]),
            edge: "outside.out>window.bridge.relay.in".into(),
            target: leaf,
        });
        journal.commit(external).unwrap();
        let mut start = lifecycle_transaction(child.clone(), node::Lifecycle::Start);
        start.parent_instance = Some(parent);
        start.cause = Some(cause);
        for wrong_key in [false, true] {
            let mut forged = start.clone();
            if wrong_key {
                forged.instance = instance::Generation::new(address(&owner, "bob"), 0);
            } else {
                forged.cause = Some(Id::from_bytes([2; 16]));
            }
            let before = journal.records().to_vec();
            assert!(matches!(
                journal.commit(forged),
                Err(Error::InvalidParentInstance { .. })
            ));
            assert_eq!(journal.records(), before);
        }
        journal.commit(start.clone()).unwrap();
        journal
            .commit(lifecycle_transaction(child.clone(), node::Lifecycle::Stop))
            .unwrap();
        start.instance = instance::Generation::new(child.address().clone(), 1);
        let before = journal.records().to_vec();
        assert!(
            matches!(
                journal.commit(start),
                Err(Error::InvalidParentInstance { .. })
            ),
            "the prior generation's descendant reservation is not a second admission"
        );
        assert_eq!(journal.records(), before);
    }

    #[test]
    fn observable_time_must_match_the_retained_acceptance_instant() {
        let reference = instance::Generation::new(
            instance::InstanceAddress::singleton(&crate::node::OwnerPath::root()),
            0,
        );
        let mut record = lifecycle_transaction(reference, node::Lifecycle::Start);
        record.observed_time = Some(crate::time::Timestamp::from_nanoseconds(7));
        record.deadlines = Some(Box::new(
            deadline::Deadlines::try_new(
                crate::time::LogicalTime::from_nanoseconds(8),
                vec![deadline::Deadline {
                    kind: DeadlineKind::JoinWithin,
                    reference: Id::from_bytes([1; 16]),
                    due_at: crate::time::LogicalTime::from_nanoseconds(10),
                }],
            )
            .unwrap(),
        ));
        let mut journal = MemJournal::new(GraphVersion(1));
        assert!(matches!(
            journal.commit(record.clone()),
            Err(Error::InvalidDeadlines(_))
        ));
        assert!(journal.records().is_empty());
        record.observed_time = Some(crate::time::Timestamp::from_nanoseconds(8));
        journal.commit(record).unwrap();
    }

    fn activate(journal: &mut MemJournal, reference: instance::Generation) {
        activate_root(journal);
        journal
            .commit(lifecycle_transaction(reference, node::Lifecycle::Start))
            .expect("the child lifecycle starts under the root");
    }

    fn request_close(journal: &mut MemJournal, reference: instance::Generation) {
        let mut record = lifecycle_transaction(reference.clone(), node::Lifecycle::Close);
        record.close_scope = Some(reference);
        journal.commit(record).expect("Closing is durable");
    }

    #[test]
    fn retained_observations_correlate_records_without_entering_causal_history() {
        let mut journal = MemJournal::new(GraphVersion(1));
        let inbound = journal
            .append_ingress(submission("cli", IngressSequence::Next))
            .expect("the root commits");
        let origin = inbound.envelope.meta().sys().origin_id();
        activate_root(&mut journal);

        let connector_path = node::Path::new(["remote"]).expect("valid connector path");
        let edge = String::from("app.out>remote.in");
        let prepared = crate::connector::exchange::PreparedRequest::seal(vec![1, 2, 3]);
        let call_id = ids::call_id(
            ids::delivery_id(&edge, inbound.envelope.meta().sys().id()),
            prepared.digest(),
        );
        journal
            .open_egress(
                CallOpen {
                    call_id,
                    emitting_id: inbound.envelope.meta().sys().id(),
                    edge,
                    connector_path: connector_path.clone(),
                    prepared,
                    idempotency_key: None,
                    resolved_settings: BTreeMap::new(),
                    logical_time: crate::time::LogicalTime::ZERO,
                },
                None,
            )
            .expect("the Call opens");
        journal
            .abandon_egress(&call_id, crate::time::LogicalTime::ZERO)
            .expect("the Call closes abandoned");

        let prepared = crate::connector::exchange::PreparedRequest::seal(vec![4, 5, 6]);
        let session_id = ids::outbound_session_id(
            &connector_path,
            inbound.envelope.meta().sys().id(),
            prepared.digest(),
        );
        journal
            .open_outbound_session(SessionOpen {
                session_id,
                emitting_id: inbound.envelope.meta().sys().id(),
                connector_path: connector_path.clone(),
                prepared,
            })
            .expect("the Session opens");
        journal
            .confirm_session_open(&session_id)
            .expect("the Session becomes live");
        let prepared = crate::connector::exchange::PreparedRequest::seal(vec![7, 8, 9]);
        let write_ref = ids::session_write_id(
            session_id,
            inbound.envelope.meta().sys().id(),
            prepared.digest(),
        );
        journal
            .open_session_write(SessionWrite {
                write_ref,
                session_id,
                emitting_id: inbound.envelope.meta().sys().id(),
                connector_path: connector_path.clone(),
                prepared,
            })
            .expect("the write opens");
        journal
            .settle_session_write(&write_ref, false)
            .expect("the write settles");

        let observations = journal.retained_observations();
        assert_eq!(journal.records().len(), observations.len());
        assert_eq!(
            observations[0].kind(),
            RecordObservationKind::IngressCommitted
        );
        assert_eq!(
            observations[1].kind(),
            RecordObservationKind::TransactionCommitted
        );
        assert_eq!(
            observations[1].correlation().owner(),
            Some(&node::OwnerPath::root())
        );
        let call = &observations[2..4];
        assert_eq!(call[0].kind(), RecordObservationKind::CallOpened);
        assert_eq!(call[1].kind(), RecordObservationKind::CallAbandoned);
        assert!(call.iter().all(|observation| {
            observation.correlation().origin_id() == Some(origin)
                && observation.correlation().connector() == Some(&connector_path)
                && observation.correlation().exchange()
                    == Some(&ExchangeCorrelation::OutboundCall(call_id))
        }));
        let session = &observations[4..8];
        assert_eq!(session[0].kind(), RecordObservationKind::SessionOpening);
        assert_eq!(session[1].kind(), RecordObservationKind::SessionOpened);
        assert_eq!(session[2].kind(), RecordObservationKind::SessionWriteOpened);
        assert_eq!(
            session[3].kind(),
            RecordObservationKind::SessionWriteSettled
        );
        assert!(session.iter().all(|observation| {
            observation.correlation().origin_id() == Some(origin)
                && observation.correlation().connector() == Some(&connector_path)
        }));
        assert_eq!(
            session[3].correlation().exchange(),
            Some(&ExchangeCorrelation::Session {
                session: session_id,
                write: Some(write_ref),
            })
        );

        assert_eq!(Journal::retained_observations(&journal), observations);
        assert_eq!(journal.retained_observations_from(2), observations[2..]);
        assert!(
            journal
                .retained_observations_from(observations.len())
                .is_empty()
        );
    }

    #[test]
    fn admission_prefix_and_live_index_agree_through_inner_first_closing() {
        let root = instance::Generation::new(
            instance::InstanceAddress::singleton(&node::OwnerPath::root()),
            0,
        );
        let parent = instance("window", 0);
        let child = instance::Generation::new(
            instance::InstanceAddress::singleton(&node::OwnerPath::new(["window", "sum"]).unwrap()),
            0,
        );
        let other = instance("other", 0);
        let parent_next = instance::Generation::new(parent.address().clone(), 1);
        let child_next = instance::Generation::new(child.address().clone(), 1);
        let start = |reference: &instance::Generation, parent: Option<&instance::Generation>| {
            let mut record = lifecycle_transaction(reference.clone(), node::Lifecycle::Start);
            record.parent_instance = parent.cloned();
            record
        };
        let closing = |reference: &instance::Generation| {
            let mut record = lifecycle_transaction(reference.clone(), node::Lifecycle::Close);
            record.close_scope = Some(reference.clone());
            record
        };
        let stop = |reference: &instance::Generation| {
            lifecycle_transaction(reference.clone(), node::Lifecycle::Stop)
        };
        let transitions = [
            (start(&root, None), [0, 0, 0, 0]),
            (start(&parent, Some(&root)), [0, 0, 0, 0]),
            (start(&child, Some(&parent)), [0, 0, 0, 0]),
            (start(&other, Some(&root)), [0, 0, 0, 0]),
            (closing(&parent), [0, 1, 1, 0]),
            (stop(&child), [0, 1, 1, 0]),
            (stop(&parent), [0, 1, 1, 0]),
            (start(&parent_next, Some(&root)), [0, 1, 1, 0]),
            (start(&child_next, Some(&parent_next)), [0, 1, 1, 0]),
            (closing(&child_next), [0, 1, 2, 0]),
            (stop(&child_next), [0, 1, 2, 0]),
            (closing(&parent_next), [0, 2, 2, 0]),
            (stop(&parent_next), [0, 2, 2, 0]),
        ];
        let addresses = [
            root.address(),
            parent.address(),
            child.address(),
            other.address(),
        ];
        let mut live = MemJournal::new(GraphVersion(1));
        let mut generations = Vec::new();
        for (index, (record, expected)) in transitions.into_iter().enumerate() {
            live.commit(record).unwrap();
            if index == 4 {
                let count = live.records().len();
                assert!(matches!(
                    live.commit(stop(&parent)),
                    Err(Error::OpenChildInstances { .. })
                ));
                assert_eq!(live.records().len(), count);
            }
            for (address, generation) in addresses.iter().zip(expected) {
                assert_eq!(
                    live.instance_generation_for_admission(address).unwrap(),
                    instance::Generation::new((*address).clone(), generation),
                    "live record {index}",
                );
            }
            generations.push(expected);
        }

        // A completed journal is available, but the recovery fold must not
        // consult later Closing or Stop facts while rebuilding an earlier route.
        let mut prefix = admission::AdmissionPrefix::default();
        for (index, (record, expected)) in live.records().iter().zip(generations).enumerate() {
            prefix.apply(record).unwrap();
            for (address, generation) in addresses.iter().zip(expected) {
                assert_eq!(
                    admission::InstanceAdmissions::instance_generation_for_admission(
                        &prefix, address
                    )
                    .unwrap(),
                    instance::Generation::new((*address).clone(), generation),
                    "recovery prefix {index}",
                );
            }
        }
        let rebuilt = MemJournal::from_records(
            GraphVersion(1),
            crate::time::Timestamp::UNIX_EPOCH,
            None,
            live.records(),
        )
        .unwrap();
        for address in addresses {
            assert_eq!(
                rebuilt.instance_generation_for_admission(address),
                live.instance_generation_for_admission(address),
            );
        }
    }

    #[test]
    fn generations_are_a_journal_fold_and_lifecycle_cannot_double_transition() {
        let mut journal = MemJournal::new(GraphVersion(1));
        let address = instance::InstanceAddress::singleton(
            &crate::node::OwnerPath::new(["worker"]).expect("valid owner path"),
        );
        let first = instance::Generation::new(address.clone(), 0);
        assert_eq!(
            journal.instance_generation_for_admission(&address).unwrap(),
            first
        );
        activate_root(&mut journal);

        let mut wrong_owner = lifecycle_transaction(first.clone(), node::Lifecycle::Start);
        wrong_owner.owner_path = node::OwnerPath::new(["other"]).unwrap();
        assert_eq!(
            journal.commit(wrong_owner).unwrap_err(),
            Error::InstanceOwnerMismatch {
                expected: node::OwnerPath::new(["worker"]).unwrap(),
                got: node::OwnerPath::new(["other"]).unwrap(),
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
        let root = instance::Generation::new(
            instance::InstanceAddress::singleton(&crate::node::OwnerPath::root()),
            0,
        );
        request_close(&mut journal, root.clone());
        let mut duplicate_close = lifecycle_transaction(root.clone(), node::Lifecycle::Close);
        duplicate_close.close_scope = Some(root.clone());
        assert_eq!(
            journal.commit(duplicate_close).unwrap_err(),
            Error::InstanceAlreadyClosing {
                instance: root.clone(),
                scope: root.clone(),
            },
            "the first durable close request is the only lifecycle transition"
        );
        let mut nested_close = lifecycle_transaction(first.clone(), node::Lifecycle::Close);
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
        let second = instance::Generation::new(address.clone(), 1);
        assert_eq!(
            journal.instance_generation_for_admission(&address).unwrap(),
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
            journal.instance_generation_for_admission(&address).unwrap(),
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
        let second_root = instance::Generation::new(root.address().clone(), 1);
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
            journal.instance_generation_for_admission(&address).unwrap(),
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
        let address = instance::InstanceAddress::singleton(
            &crate::node::OwnerPath::new(["worker"]).expect("valid owner path"),
        );
        let reserved = instance::Generation::new(address.clone(), 0);
        activate_root(&mut journal);
        let prepared = journal
            .prepare_ingress(submission("inbound", IngressSequence::Next))
            .unwrap();
        journal
            .commit_ingress(
                prepared,
                vec![RouteTarget {
                    edge: String::from(".in>worker.in"),
                    target: reserved.clone(),
                }],
            )
            .unwrap();

        assert_eq!(
            journal.instance_generation_for_admission(&address).unwrap(),
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
            journal.instance_generation_for_admission(&address).unwrap(),
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
        let a = ids::call_id(ids::delivery_id("src.out>c.a", envelope_id), &digest);
        let b = ids::call_id(ids::delivery_id("src.out>c.b", envelope_id), &digest);
        assert_ne!(a, b, "two deliveries, two calls");
    }

    #[test]
    fn an_outbound_record_reconstructs_its_call_ref_autonomously() {
        // The autonomy rule of identity.md §3: keys 27 (emitting),
        // 30 (digest), and 33 (edge) suffice — no other record, no
        // out-of-band state.
        let sealed = intent("echo", 7);
        let reconstructed = ids::call_id(
            ids::delivery_id(&sealed.edge, sealed.emitting_id),
            sealed.request_digest(),
        );
        assert_eq!(reconstructed, sealed.call_id);
    }

    #[test]
    fn the_delivery_ref_recipe_is_pinned_byte_for_byte() {
        // envelope.md §2: the delivery reference is
        // derive(0x0b, canonical_edge, envelope_id). Any drift here
        // silently changes every consumed delivery and derived Call id.
        let delivery = Id::from_bytes([7; 16]);
        let reference = ids::delivery_id("shout.out>echo.in", delivery);
        assert_eq!(
            reference.to_string(),
            "b26fc16a16876c6d531bfc0b5ee3442b",
            "the pinned obligation identity"
        );
    }

    #[test]
    fn the_instance_lifetime_ref_seals_address_and_generation() {
        let address = instance::InstanceAddress::singleton(
            &crate::node::OwnerPath::new(["worker"]).expect("valid owner path"),
        );
        let first = ids::instance_lifetime_id(&instance::Generation::new(address.clone(), 0));
        let second = ids::instance_lifetime_id(&instance::Generation::new(address, 1));
        assert_eq!(first.to_string(), "77901038ea63d061071656d3c7637a9d");
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
        let inbound = ids::inbound_session_id(&provenance("gateway"), opening);
        let connector = node::Path::new(["chat", "socket"]).unwrap();
        let emitting = Id::from_bytes([4; 16]);
        let digest = [5; 32];
        let outbound = ids::outbound_session_id(&connector, emitting, &digest);
        let write = ids::session_write_id(outbound, Id::from_bytes([6; 16]), &[7; 32]);
        assert_eq!(
            format!("{inbound}|{outbound}|{write}"),
            "5ea1374ac75e3968f604f31cbe68e2ca|7ddc2c74566fc89f912afee39073d7be|\
             47998f535730ffa5def6866dec6795cd"
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
            .append_ingress(submission("inbound", IngressSequence::Next))
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
                .append_ingress(submission("inbound", IngressSequence::Next))
                .unwrap();
            assert_eq!(committed.seq, expected);
        }
        let other = journal
            .append_ingress(submission("webhook", IngressSequence::Next))
            .unwrap();
        assert_eq!(other.seq, 0, "authorities count independently");
    }

    #[test]
    fn explicit_sequences_replay_or_refuse() {
        let mut journal = MemJournal::new(GraphVersion(1));
        journal
            .append_ingress(submission("inbound", IngressSequence::Explicit(0)))
            .unwrap();
        let diverged = journal
            .append_ingress(submission("inbound", IngressSequence::Explicit(5)))
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
                .append_ingress(submission("inbound", IngressSequence::Next))
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
        let call = echo_intent.call_id;
        journal.open_egress(echo_intent.clone(), None).unwrap();
        let open_before = journal.records()[0].clone();
        assert_eq!(
            journal
                .close_egress(
                    &call,
                    Closure::Timeout,
                    crate::time::LogicalTime::ZERO,
                    None
                )
                .unwrap(),
            CloseVerdict::Won
        );
        // The normative sequence: two consecutive facts, open first,
        // byte-identical to what it was before the closure.
        assert_eq!(journal.records().len(), 2);
        assert_eq!(journal.records()[0], open_before);
        let Record::Egress(EgressRecord::Call(open)) = &journal.records()[0] else {
            panic!("expected the open fact");
        };
        assert_eq!(open.status, CallStatus::Open);
        let Record::Egress(EgressRecord::Call(closed)) = &journal.records()[1] else {
            panic!("expected the closure fact");
        };
        assert_eq!(closed.status, CallStatus::Closed(Closure::Timeout));
        assert_eq!(closed.intent.call_id, call);
        // The fold answers "open?" correctly.
        assert!(journal.open_calls().is_empty());
        // The race: every later closure loses and appends nothing.
        assert_eq!(
            journal
                .close_egress(
                    &call,
                    Closure::Timeout,
                    crate::time::LogicalTime::ZERO,
                    None
                )
                .unwrap(),
            CloseVerdict::AlreadyClosed
        );
        assert_eq!(journal.records().len(), 2);
        // A closure for an unknown call is a fault, not a silence.
        let ghost = ids::call_id(
            ids::delivery_id("x.out>ghost.in", Id::from_bytes([9; 16])),
            &[0; 32],
        );
        assert_eq!(
            journal.close_egress(
                &ghost,
                Closure::Timeout,
                crate::time::LogicalTime::ZERO,
                None
            ),
            Err(Error::UnknownCall)
        );

        let abandoned_intent = intent("slow", 8);
        let abandoned = abandoned_intent.call_id;
        journal.open_egress(abandoned_intent, None).unwrap();
        assert_eq!(
            journal
                .abandon_egress(&abandoned, crate::time::LogicalTime::ZERO)
                .unwrap(),
            CloseVerdict::Won
        );
        assert_eq!(
            journal
                .close_egress(
                    &abandoned,
                    Closure::Timeout,
                    crate::time::LogicalTime::ZERO,
                    None
                )
                .unwrap(),
            CloseVerdict::AlreadyClosed,
            "an observed response or timeout cannot supersede abandonment"
        );
        assert_eq!(
            journal
                .abandon_egress(&abandoned, crate::time::LogicalTime::ZERO)
                .unwrap(),
            CloseVerdict::AlreadyClosed
        );
        assert!(matches!(
            journal.records().last(),
            Some(Record::Egress(EgressRecord::Call(call)))
                if call.status == CallStatus::Abandoned
        ));
    }

    #[test]
    fn a_forged_intent_is_refused() {
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut forged = intent("echo", 7);
        forged.call_id = ids::call_id(
            ids::delivery_id("x.out>other.in", Id::from_bytes([9; 16])),
            &[1; 32],
        );
        assert_eq!(
            journal.open_egress(forged, None).unwrap_err(),
            Error::CorruptIntent
        );
    }

    #[test]
    fn commit_canonicalizes_the_sorted_collections() {
        let mut journal = MemJournal::new(GraphVersion(1));
        activate(&mut journal, instance("shout", 0));
        journal
            .commit(TransactionRecord {
                owner_path: node::OwnerPath::new(["shout"]).unwrap(),
                instance: instance("shout", 0),
                parent_instance: None,
                close_scope: None,
                exchange_exit: None,
                retry_record: None,
                loop_record: None,
                observed_time: None,
                deadlines: None,
                instance_commands: None,
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
                deadline_ids: vec![
                    (DeadlineKind::RecurrenceDelay, Id::from_bytes([2; 16])),
                    (DeadlineKind::CallTimeout, Id::from_bytes([1; 16])),
                ],
            })
            .unwrap();
        let Some(Record::Transaction(transaction)) = journal.records().last() else {
            panic!("expected the transaction");
        };
        assert_eq!(transaction.consumed[0].edge, "a");
        assert_eq!(transaction.state_delta[0].0, "a");
        assert_eq!(transaction.deadline_ids[0].0, DeadlineKind::CallTimeout);
    }

    #[test]
    fn records_keep_causal_order() {
        let mut journal = MemJournal::new(GraphVersion(1));
        activate(&mut journal, instance("shout", 0));
        let first_observed = journal.records().len();
        let committed = journal
            .append_ingress(submission("inbound", IngressSequence::Next))
            .unwrap();
        journal
            .commit(TransactionRecord {
                owner_path: node::OwnerPath::new(["shout"]).unwrap(),
                instance: instance("shout", 0),
                parent_instance: None,
                close_scope: None,
                exchange_exit: None,
                retry_record: None,
                loop_record: None,
                observed_time: None,
                deadlines: None,
                instance_commands: None,
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
                deadline_ids: vec![],
            })
            .unwrap();
        journal.open_egress(intent("echo", 1), None).unwrap();
        let kinds: Vec<u8> = journal
            .records()
            .iter()
            .skip(first_observed)
            .map(|record| match record {
                Record::Ingress(_) => 1,
                Record::Transaction(_) => 2,
                Record::Egress(_) => 3,
            })
            .collect();
        assert_eq!(kinds, [1, 2, 3]);
        // The semantic queries answer without exposing the slice.
        assert!(
            journal
                .find_envelope(committed.envelope.meta().sys().id())
                .is_some()
        );
        assert!(journal.find_call(&intent("echo", 1).call_id).is_some());
        assert_eq!(journal.open_calls().len(), 1);
    }

    #[test]
    fn inbound_sessions_are_concurrent_correlated_and_silent_after_exit() {
        let mut journal = MemJournal::new(GraphVersion(1));
        let first = journal
            .prepare_ingress(IngressProposal {
                session: Some(InboundSession::Open),
                ..submission("socket", IngressSequence::Next)
            })
            .unwrap();
        let first = journal.commit_ingress(first, vec![]).unwrap();
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
            .prepare_ingress(IngressProposal {
                session: Some(InboundSession::Open),
                ..submission("socket", IngressSequence::Next)
            })
            .unwrap();
        let second = journal.commit_ingress(second, vec![]).unwrap();
        let Some(InboundExchangeRecord::Session(SessionIngressRecord::Open(second_ref))) =
            second.record.exchange.as_deref().cloned()
        else {
            panic!("the second opening carries its session ref")
        };
        assert_ne!(first_ref, second_ref);

        let message = journal
            .prepare_ingress(IngressProposal {
                session: Some(InboundSession::Message(first_ref)),
                ..submission("socket", IngressSequence::Next)
            })
            .unwrap();
        let message = journal.commit_ingress(message, vec![]).unwrap();
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
        let mut expected = vec![first_ref, second_ref];
        expected.sort();
        assert_eq!(journal.live_sessions(), expected);

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
            journal.prepare_ingress(IngressProposal {
                session: Some(InboundSession::Message(first_ref)),
                ..submission("socket", IngressSequence::Next)
            }),
            Err(Error::UnknownSession),
            "a terminal session can never speak again"
        );
    }

    #[test]
    fn an_ingress_schedule_commits_atomically_with_its_exact_obligations() {
        use deadline::{Deadline, Deadlines};
        let mut journal = MemJournal::new(GraphVersion(1));
        let prepared = journal
            .prepare_connector_ingress(submission("server", IngressSequence::Next))
            .unwrap();
        let root = prepared.envelope().meta().sys().id();
        let deadline_id = (DeadlineKind::InboundCallTimeout, root);
        let prepared = prepared
            .into_call(instance("server", 0), vec![deadline_id])
            .unwrap();
        let mut deadline = Deadline {
            kind: deadline_id.0,
            reference: root,
            due_at: crate::time::LogicalTime::from_nanoseconds(12),
        };
        let schedule = Deadlines::try_new(
            crate::time::LogicalTime::from_nanoseconds(7),
            vec![deadline],
        )
        .unwrap();
        deadline.reference = Id::from_bytes([255; 16]);
        let forged = prepared.clone().with_deadlines(
            Deadlines::try_new(
                crate::time::LogicalTime::from_nanoseconds(7),
                vec![deadline],
            )
            .unwrap(),
        );
        assert!(matches!(
            journal.commit_ingress(forged, vec![]),
            Err(Error::InvalidDeadlines(_))
        ));
        assert!(journal.records().is_empty());
        assert!(journal.open_inbound_calls().is_empty());
        let accepted = prepared.with_deadlines(schedule.clone());
        let stale = accepted.clone();
        let committed = journal.commit_ingress(accepted, vec![]).unwrap();
        assert_eq!(
            committed.seq, 0,
            "failed timing validation spends no sequence"
        );
        assert_eq!(committed.record.deadlines.as_deref(), Some(&schedule));
        assert_eq!(journal.records()[0].deadlines(), Some(&schedule));
        let before = journal.records().to_vec();
        assert!(matches!(
            journal.commit_ingress(stale, vec![]),
            Err(Error::SequenceMismatch { .. })
        ));
        assert_eq!(
            journal.records(),
            before,
            "failed acceptance cannot append timing alone"
        );
    }

    #[test]
    fn an_inbound_call_opens_on_its_source_instance_and_closes_once() {
        let mut journal = MemJournal::new(GraphVersion(1));
        let source = instance("server", 0);
        let prepared = journal
            .prepare_connector_ingress(submission("server", IngressSequence::Next))
            .unwrap()
            .into_call(source.clone(), vec![])
            .unwrap();
        let root = prepared.envelope().meta().sys().id();
        let call_id = InboundCallId::from_root(root);
        let committed = journal.commit_ingress(prepared, vec![]).unwrap();
        assert_eq!(
            journal.open_inbound_calls(),
            [InboundCallIngressRecord {
                reference: call_id,
                instance: source.clone(),
                deadline_ids: vec![],
            }]
        );
        assert_eq!(
            committed.record.exchange.as_deref(),
            Some(&InboundExchangeRecord::Call(InboundCallIngressRecord {
                reference: call_id,
                instance: source.clone(),
                deadline_ids: vec![],
            }))
        );

        activate(&mut journal, source.clone());
        let unmatched_reply = lifecycle_transaction(source.clone(), node::Lifecycle::Reply);
        assert_eq!(
            journal.commit(unmatched_reply).unwrap_err(),
            Error::InvalidInboundCallExit
        );
        let mut missing_exit = lifecycle_transaction(source.clone(), node::Lifecycle::Reply);
        missing_exit.exchange_exit = Some(Box::new(InboundExchangeExitRecord::Call(call_id)));
        missing_exit.cause = Some(root);
        assert_eq!(
            journal.commit(missing_exit).unwrap_err(),
            Error::InvalidInboundCallExit
        );

        let mut terminal = lifecycle_transaction(source.clone(), node::Lifecycle::Reply);
        terminal.exchange_exit = Some(Box::new(InboundExchangeExitRecord::Call(call_id)));
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
        let session = opening.session_id;
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
            .append_ingress(submission("world", IngressSequence::Next))
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
