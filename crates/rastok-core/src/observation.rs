//! Typed, non-causal facts retained beside the journal.
//!
//! These values correlate causal records for inspection and export without
//! becoming replay input or causal identity. Hosted exporters may discard a
//! projection of this stream; the retained stream itself remains available to
//! read-only inspection at the selected journal prefix.

use crate::boundary::{CallId, InboundCallId, SessionId, SessionWriteId};
use crate::envelope::Id;
use crate::identity::InstanceGeneration;
use crate::node;
use crate::plan::OwnerPath;

/// The causal transition summarized by one retained observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordObservationKind {
    /// One root envelope entered the graph.
    InboundCommitted,
    /// One atomic Node or Graph firing committed.
    TransactionCommitted,
    /// One outbound Call became open before external work.
    CallOpened,
    /// One outbound Call acquired its sole causal closure.
    CallClosed,
    /// Recovery could no longer establish an outbound Call's outcome.
    CallAbandoned,
    /// One outbound Session opening became durable before external work.
    SessionOpening,
    /// The external opening established a live Session.
    SessionOpened,
    /// One Session message write became durable before external work.
    SessionWriteOpened,
    /// One Session message write settled locally.
    SessionWriteSettled,
    /// Recovery could not establish one Session write's visibility.
    SessionWriteAbandoned,
}

/// Exchange identity carried by an observation when one exists.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExchangeCorrelation {
    /// A graph-to-world bounded Call.
    OutboundCall(CallId),
    /// A world-to-graph bounded Call.
    InboundCall(InboundCallId),
    /// A long-lived Session and, for write facts, its exact write.
    Session {
        /// Logical Session identity.
        session: SessionId,
        /// Message-write identity, absent for opening and terminal facts.
        write: Option<SessionWriteId>,
    },
}

/// Placement-neutral correlation extracted from one causal record.
///
/// Optional fields express facts the causal record genuinely does not own;
/// they are never synthesized from host placement or exporter state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordCorrelation {
    origin_id: Option<Id>,
    owner: Option<OwnerPath>,
    instance: Option<InstanceGeneration>,
    connector: Option<node::Path>,
    exchange: Option<ExchangeCorrelation>,
}

impl RecordCorrelation {
    pub(crate) const fn new(origin_id: Option<Id>) -> Self {
        Self {
            origin_id,
            owner: None,
            instance: None,
            connector: None,
            exchange: None,
        }
    }

    pub(crate) fn with_owner(mut self, owner: OwnerPath) -> Self {
        self.owner = Some(owner);
        self
    }

    pub(crate) fn with_instance(mut self, instance: InstanceGeneration) -> Self {
        self.instance = Some(instance);
        self
    }

    pub(crate) fn with_connector(mut self, connector: Option<node::Path>) -> Self {
        self.connector = connector;
        self
    }

    pub(crate) fn with_exchange(mut self, exchange: Option<ExchangeCorrelation>) -> Self {
        self.exchange = exchange;
        self
    }

    /// Root flow identity used as the trace correlation when known.
    #[must_use]
    pub const fn origin_id(&self) -> Option<Id> {
        self.origin_id
    }

    /// Logical owner of a transaction, when this is a transaction fact.
    #[must_use]
    pub const fn owner(&self) -> Option<&OwnerPath> {
        self.owner.as_ref()
    }

    /// Exact logical instance generation, when the causal fact owns one.
    #[must_use]
    pub const fn instance(&self) -> Option<&InstanceGeneration> {
        self.instance.as_ref()
    }

    /// Connector owning the exchange, when present in causal history.
    #[must_use]
    pub const fn connector(&self) -> Option<&node::Path> {
        self.connector.as_ref()
    }

    /// Typed Call or Session correlation, when this fact belongs to one.
    #[must_use]
    pub const fn exchange(&self) -> Option<&ExchangeCorrelation> {
        self.exchange.as_ref()
    }
}

/// One retained, non-causal projection of a causal record.
///
/// Stream position is the surrounding journal observation order rather than a
/// duplicated field. Future measure and cost observations can extend this
/// typed plane without changing any causal record bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordObservation {
    kind: RecordObservationKind,
    correlation: RecordCorrelation,
}

impl RecordObservation {
    pub(crate) const fn new(kind: RecordObservationKind, correlation: RecordCorrelation) -> Self {
        Self { kind, correlation }
    }

    /// Causal transition summarized by this observation.
    #[must_use]
    pub const fn kind(&self) -> RecordObservationKind {
        self.kind
    }

    /// Placement-neutral correlation derived from the causal fact.
    #[must_use]
    pub const fn correlation(&self) -> &RecordCorrelation {
        &self.correlation
    }
}
