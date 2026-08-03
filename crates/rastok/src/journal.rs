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
//! mechanics only; adapters and mocks supply events, never records.
//!
//! [`delivery-semantics.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/delivery-semantics.md
//! [`envelope.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/envelope.md
//! [`identity.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/identity.md

use crate::boundary::{
    CallOutcome, CallRef, InboundSubmission, IngressAuthority, Intent, SubmissionSequence,
};
use crate::cel;
use crate::envelope::{Envelope, Id, Meta, MetaSys};
use crate::node;
use crate::plan::GraphVersion;
use std::collections::BTreeMap;
use std::fmt;

/// Deterministic identity recipes — the injective blake3 framing of
/// `envelope.md` §2, with its domain tags.
pub mod ids {
    use super::{CallRef, Id, IngressAuthority, node};

    /// Domain tag of a root derivation: 2 fields.
    const ROOT: u8 = 0x01;
    /// Domain tag of an emission derivation: 5 fields.
    const EMISSION: u8 = 0x02;
    /// Domain tag of the internal call-identity derivation. Outside
    /// the envelope id domains (`0x01`–`0x04` are reserved by the
    /// spec); a call ref is never an envelope id.
    const CALL: u8 = 0x10;

    /// The root id of an inbound envelope:
    /// `blake3(connector_path ‖ inbound_seq)`. The single-process
    /// recipe; a non-zero replica joins the derivation when multi-host
    /// semantics land.
    #[must_use]
    pub fn root_id(authority: &IngressAuthority, seq: u64) -> Id {
        Id::derive(
            ROOT,
            &[
                authority.connector_path().to_string().as_bytes(),
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

    /// The internal identity of an outbound call, derived from its
    /// canonical fields — recomputable from any record that carries
    /// them, never stored as a new causal field.
    #[must_use]
    pub fn call_ref(
        emitting_id: Id,
        connector_path: &node::Path,
        request_digest: &[u8; 32],
    ) -> CallRef {
        CallRef::new(Id::derive(
            CALL,
            &[
                emitting_id.as_bytes(),
                connector_path.to_string().as_bytes(),
                request_digest,
            ],
        ))
    }
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
/// keys 20–23). The record autonomously reconstructs its root id from
/// `(connector_path, replica, seq)`.
#[derive(Debug, Clone, PartialEq)]
pub struct InboundRecord {
    /// Who accepted it.
    pub authority: IngressAuthority,
    /// The atomic per-authority sequence.
    pub seq: u64,
    /// The envelope born here.
    pub envelope: Envelope,
}

/// One consumed input position — the cursor is per
/// `(graph edge × source partition)`, not per port: several edges can
/// feed one port. The M0 single-cell slice omits the partition.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConsumedPosition {
    /// The graph edge the input arrived on, canonically rendered.
    pub edge: String,
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
/// keys 30–37): consumed input, state delta, and emissions commit
/// together or not at all. [`Journal::commit`] canonicalizes the
/// sorted collections; emission order is text order and stays as
/// given.
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionRecord {
    /// The firing node.
    pub node_path: node::Path,
    /// What fired it.
    pub trigger: node::Trigger,
    /// The consumed input positions; canonically sorted at commit.
    pub consumed: Vec<ConsumedPosition>,
    /// The state changes; canonically sorted by field at commit.
    pub state_delta: Vec<(String, cel::Value)>,
    /// The emissions, in text/emission-index order.
    pub emissions: Vec<Emission>,
    /// The dated obligations this firing arms — `(timer_kind, ref)`
    /// per identity.md §3 key 37: kind 1 = call budget, ref = the
    /// delivered envelope's id. Sorted at commit; the due instant is
    /// never sealed.
    pub timers: Vec<(u8, crate::envelope::Id)>,
}

/// One fact of a call's WAL (`delivery-semantics.md` §4): the open
/// fact, or its closure — two records, never one rewritten.
#[derive(Debug, Clone, PartialEq)]
pub struct OutboundRecord {
    /// The canonical call fields.
    pub intent: Intent,
    /// Which fact this record states.
    pub status: OutboundStatus,
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
    /// Resolved by the recovery fold — the fold is its only minter.
    Abandoned,
}

/// What closed a call — the partner's outcome or the timeout,
/// exactly one. The outcome carries the raw reply or typed error;
/// the connector's own transaction builds the envelope.
#[derive(Debug, Clone, PartialEq)]
pub enum Closure {
    /// The partner answered — with a reply or a typed failure.
    Response(CallOutcome),
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
    /// A per-authority sequence exhausted its range; monotonicity
    /// cannot continue.
    SequenceOverflow,
    /// A call ref matching no journaled call.
    UnknownCall,
    /// An intent whose call ref is already journaled.
    DuplicateOpen,
    /// An intent whose call ref does not derive from its own
    /// canonical fields — a forged or corrupted causal fact.
    CorruptIntent,
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
            Self::UnknownCall => f.write_str("no outbound record for this call"),
            Self::DuplicateOpen => f.write_str("this call is already journaled"),
            Self::CorruptIntent => {
                f.write_str("the call ref does not derive from the intent's canonical fields")
            }
            Self::Storage(detail) => write!(f, "the journal store failed: {detail}"),
        }
    }
}

impl std::error::Error for Error {}

/// Semantic journal operations and queries — never bare appends, and
/// no exposure of a physical record layout: a durable implementation
/// may hold anything on disk as long as it answers these questions.
pub trait Journal {
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
    fn append_inbound(&mut self, submission: InboundSubmission) -> Result<CommittedInbound, Error>;

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

    /// The envelope carrying `id`, wherever the history recorded it.
    fn find_envelope(&self, id: Id) -> Option<Envelope>;

    /// The intent of a journaled call, settled or not.
    fn find_call(&self, call_ref: &CallRef) -> Option<Intent>;

    /// The calls whose open fact has no closure fact yet, in open
    /// order.
    fn open_calls(&self) -> Vec<Intent>;
}

/// The M0 in-memory journal. It preserves the logical boundaries a
/// durable implementation needs — typed records, per-authority
/// sequences, append-only closure — with no claim about files.
#[derive(Debug)]
pub struct MemJournal {
    graph_version: GraphVersion,
    records: Vec<Record>,
    next_seq: BTreeMap<IngressAuthority, u64>,
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
            Record::Outbound(outbound) if outbound.intent.call_ref == *call_ref => {
                Some(&outbound.status)
            }
            _ => None,
        })
    }
}

impl Journal for MemJournal {
    fn append_inbound(&mut self, submission: InboundSubmission) -> Result<CommittedInbound, Error> {
        let next = self
            .next_seq
            .get(&submission.authority)
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
        let following = seq.checked_add(1).ok_or(Error::SequenceOverflow)?;
        let id = ids::root_id(&submission.authority, seq);
        let mut sys = MetaSys::root(id, self.graph_version.0);
        if let Some(mime_type) = submission.draft.mime_type {
            sys = sys.with_mime_type(mime_type);
        }
        let envelope = Envelope::new(
            Meta::new(sys).with_app(submission.draft.app),
            submission.draft.data,
        );
        let record = InboundRecord {
            authority: submission.authority.clone(),
            seq,
            envelope: envelope.clone(),
        };
        self.next_seq.insert(submission.authority, following);
        self.records.push(Record::Inbound(record.clone()));
        Ok(CommittedInbound {
            seq,
            envelope,
            record,
        })
    }

    fn commit(&mut self, mut transaction: TransactionRecord) -> Result<(), Error> {
        // Canonicalize the declaratively sorted collections so an
        // invalid ordering is not constructible through the journal.
        transaction.consumed.sort();
        transaction.state_delta.sort_by(|a, b| a.0.cmp(&b.0));
        self.records.push(Record::Transaction(transaction));
        Ok(())
    }

    fn open_outbound(&mut self, intent: Intent) -> Result<(), Error> {
        let derived = ids::call_ref(
            intent.emitting_id,
            &intent.connector_path,
            &intent.request_digest,
        );
        if derived != intent.call_ref {
            return Err(Error::CorruptIntent);
        }
        if self.statuses_of(&intent.call_ref).next().is_some() {
            return Err(Error::DuplicateOpen);
        }
        self.records.push(Record::Outbound(OutboundRecord {
            intent,
            status: OutboundStatus::Open,
        }));
        Ok(())
    }

    fn close_outbound(
        &mut self,
        call_ref: &CallRef,
        closure: Closure,
    ) -> Result<CloseVerdict, Error> {
        let mut intent = None;
        for record in &self.records {
            let Record::Outbound(outbound) = record else {
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
        self.records.push(Record::Outbound(OutboundRecord {
            intent,
            status: OutboundStatus::Closed(closure),
        }));
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
            Record::Outbound(outbound) if outbound.intent.call_ref == *call_ref => {
                Some(outbound.intent.clone())
            }
            _ => None,
        })
    }

    fn open_calls(&self) -> Vec<Intent> {
        let mut open: Vec<Intent> = Vec::new();
        for record in &self.records {
            let Record::Outbound(outbound) = record else {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::Draft;
    use crate::envelope::{Data, Encoding, Namespace};

    fn authority(path: &str) -> IngressAuthority {
        IngressAuthority::single(node::Path::new([path]).unwrap())
    }

    fn submission(path: &str, sequence: SubmissionSequence) -> InboundSubmission {
        InboundSubmission {
            authority: authority(path),
            draft: Draft {
                data: Data::encoded(Encoding::Raw, *b"hello"),
                mime_type: Some(String::from("text/plain")),
                app: Namespace::new(),
            },
            sequence,
        }
    }

    fn intent(connector: &str, digest: u8) -> Intent {
        let connector_path = node::Path::new([connector]).unwrap();
        let emitting_id = Id::derive(0x02, &[b"cause"]);
        let request_digest = [digest; 32];
        Intent {
            call_ref: ids::call_ref(emitting_id, &connector_path, &request_digest),
            emitting_id,
            connector_path,
            request_digest,
            idempotency_key: Some(String::from("op-1")),
            effect: node::EffectClass::Idempotent,
        }
    }

    #[test]
    fn the_id_recipes_are_pinned_byte_for_byte() {
        let root = ids::root_id(&authority("inbound"), 7);
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
        assert_eq!(sys.id(), ids::root_id(&authority("inbound"), 0));
        assert_eq!(sys.parent_id(), None);
        assert_eq!(sys.origin_id(), sys.id());
        assert_eq!(sys.graph_version(), 1);
        assert_eq!(sys.mime_type(), Some("text/plain"));
        assert_eq!(committed.seq, 0);
    }

    #[test]
    fn sequences_are_monotone_per_authority() {
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
        let intent = intent("echo", 7);
        let call = intent.call_ref;
        journal.open_outbound(intent.clone()).unwrap();
        let open_before = journal.records()[0].clone();
        assert_eq!(
            journal.close_outbound(&call, Closure::Timeout).unwrap(),
            CloseVerdict::Won
        );
        // The normative sequence: two consecutive facts, open first,
        // byte-identical to what it was before the closure.
        assert_eq!(journal.records().len(), 2);
        assert_eq!(journal.records()[0], open_before);
        let Record::Outbound(open) = &journal.records()[0] else {
            panic!("expected the open fact");
        };
        assert_eq!(open.status, OutboundStatus::Open);
        let Record::Outbound(closed) = &journal.records()[1] else {
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
            Id::from_bytes([9; 16]),
            &node::Path::new(["ghost"]).unwrap(),
            &[0; 32],
        );
        assert_eq!(
            journal.close_outbound(&ghost, Closure::Timeout),
            Err(Error::UnknownCall)
        );
    }

    #[test]
    fn a_forged_intent_is_refused() {
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut forged = intent("echo", 7);
        forged.call_ref = ids::call_ref(
            Id::from_bytes([9; 16]),
            &node::Path::new(["other"]).unwrap(),
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
        journal
            .commit(TransactionRecord {
                node_path: node::Path::new(["shout"]).unwrap(),
                trigger: node::Trigger::Port(node::Port::new("in").unwrap()),
                consumed: vec![
                    ConsumedPosition {
                        edge: String::from("b"),
                        seq: 1,
                    },
                    ConsumedPosition {
                        edge: String::from("a"),
                        seq: 2,
                    },
                ],
                state_delta: vec![
                    (String::from("z"), cel::Value::Int(1)),
                    (String::from("a"), cel::Value::Int(2)),
                ],
                emissions: vec![],
                timers: vec![],
            })
            .unwrap();
        let Record::Transaction(transaction) = &journal.records()[0] else {
            panic!("expected the transaction");
        };
        assert_eq!(transaction.consumed[0].edge, "a");
        assert_eq!(transaction.state_delta[0].0, "a");
    }

    #[test]
    fn records_keep_causal_order() {
        let mut journal = MemJournal::new(GraphVersion(1));
        let committed = journal
            .append_inbound(submission("inbound", SubmissionSequence::Next))
            .unwrap();
        journal
            .commit(TransactionRecord {
                node_path: node::Path::new(["shout"]).unwrap(),
                trigger: node::Trigger::Port(node::Port::new("in").unwrap()),
                consumed: vec![ConsumedPosition {
                    edge: String::from(".in>shout.in"),
                    seq: 0,
                }],
                state_delta: vec![(String::from("count"), cel::Value::Int(1))],
                emissions: vec![Emission {
                    port: node::Port::new("out").unwrap(),
                    index: 0,
                    envelope: committed.envelope.clone(),
                }],
                timers: vec![],
            })
            .unwrap();
        journal.open_outbound(intent("echo", 1)).unwrap();
        let kinds: Vec<u8> = journal
            .records()
            .iter()
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
}
