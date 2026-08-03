//! Durable causal history and recovery model — the M0 in-memory
//! slice.
//!
//! Governing rules: [`delivery-semantics.md`] for the three causal
//! record kinds and the outbound WAL, [`envelope.md`] §2 for the id
//! recipes, [`identity.md`] §3 for the fields each record seals. This
//! module exposes the *logical* journal API — typed records, ordering,
//! and semantic operations; a physical layout comes later and must
//! never leak into these types.
//!
//! Records are written by the runtime mechanics only; adapters and
//! mocks supply events, never records.
//!
//! [`delivery-semantics.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/delivery-semantics.md
//! [`envelope.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/envelope.md
//! [`identity.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/identity.md

use crate::boundary::{CallRef, InboundSubmission, IngressAuthority, Intent, SubmissionSequence};
use crate::cel;
use crate::envelope::{Envelope, Id, Meta, MetaSys};
use crate::node;
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
    /// The graph acted on the world: one external call's WAL record.
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
/// together or not at all.
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionRecord {
    /// The firing node.
    pub node_path: node::Path,
    /// What fired it.
    pub trigger: node::Trigger,
    /// The consumed input positions, sorted by `(edge, seq)`.
    pub consumed: Vec<ConsumedPosition>,
    /// The state changes, a sorted pair list by field path.
    pub state_delta: Vec<(String, cel::Value)>,
    /// The emissions, in text/emission-index order.
    pub emissions: Vec<Emission>,
}

/// One external call's WAL record: journaled open before the effect
/// executes (`delivery-semantics.md` §4).
#[derive(Debug, Clone, PartialEq)]
pub struct OutboundRecord {
    /// The canonical call fields.
    pub intent: Intent,
    /// The sealed causal state.
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

/// What closed a call — a response or the timeout, exactly one.
#[derive(Debug, Clone, PartialEq)]
pub enum Closure {
    /// The partner answered; the response envelope is the closure.
    Response(Envelope),
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
    /// A call ref matching no open or closed outbound record.
    UnknownCall,
    /// An intent whose call ref is already journaled.
    DuplicateOpen,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SequenceMismatch { expected, got } => {
                write!(f, "expected inbound seq {expected}, got {got}")
            }
            Self::UnknownCall => f.write_str("no outbound record for this call"),
            Self::DuplicateOpen => f.write_str("this call is already journaled"),
        }
    }
}

impl std::error::Error for Error {}

/// Semantic journal operations — never bare appends. The API is
/// logical: typed records and ordering, no physical layout.
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
    /// from the journal.
    fn append_inbound(&mut self, submission: InboundSubmission) -> Result<CommittedInbound, Error>;

    /// Commits one atomic transaction record.
    fn commit(&mut self, transaction: TransactionRecord);

    /// Journals an outbound intent as open — before the effect
    /// executes, never after.
    ///
    /// # Errors
    ///
    /// [`Error::DuplicateOpen`] when the call is already journaled.
    fn open_outbound(&mut self, intent: Intent) -> Result<(), Error>;

    /// Settles the race to close a call: the first closure wins and
    /// becomes causal; every later one is told [`CloseVerdict::AlreadyClosed`]
    /// and must treat its event as a discarded observation.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownCall`] when no record carries this call ref.
    fn close_outbound(
        &mut self,
        call_ref: &CallRef,
        closure: Closure,
    ) -> Result<CloseVerdict, Error>;

    /// The records in causal order.
    fn records(&self) -> &[Record];
}

/// The M0 in-memory journal. It preserves the logical boundaries a
/// durable implementation needs — typed records, per-authority
/// sequences, close-once — with no claim about files.
#[derive(Debug, Default)]
pub struct MemJournal {
    graph_version: u64,
    records: Vec<Record>,
    next_seq: BTreeMap<IngressAuthority, u64>,
}

impl MemJournal {
    /// An empty journal stamping envelopes with `graph_version`.
    /// Embedded mode re-seeds the manifest at boot, so simulations
    /// deterministically run as version 1.
    #[must_use]
    pub fn new(graph_version: u64) -> Self {
        Self {
            graph_version,
            records: Vec::new(),
            next_seq: BTreeMap::new(),
        }
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
        let id = ids::root_id(&submission.authority, seq);
        let mut sys = MetaSys::root(id, self.graph_version);
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
        self.next_seq.insert(submission.authority, seq + 1);
        self.records.push(Record::Inbound(record.clone()));
        Ok(CommittedInbound {
            seq,
            envelope,
            record,
        })
    }

    fn commit(&mut self, transaction: TransactionRecord) {
        self.records.push(Record::Transaction(transaction));
    }

    fn open_outbound(&mut self, intent: Intent) -> Result<(), Error> {
        let duplicate = self.records.iter().any(|record| {
            matches!(record, Record::Outbound(outbound) if outbound.intent.call_ref == intent.call_ref)
        });
        if duplicate {
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
        for record in &mut self.records {
            let Record::Outbound(outbound) = record else {
                continue;
            };
            if outbound.intent.call_ref != *call_ref {
                continue;
            }
            return Ok(match outbound.status {
                OutboundStatus::Open => {
                    outbound.status = OutboundStatus::Closed(closure);
                    CloseVerdict::Won
                }
                _ => CloseVerdict::AlreadyClosed,
            });
        }
        Err(Error::UnknownCall)
    }

    fn records(&self) -> &[Record] {
        &self.records
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::{Draft, Response};
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

    fn intent(journal_path: &str, digest: u8) -> Intent {
        let connector_path = node::Path::new([journal_path]).unwrap();
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
        let mut journal = MemJournal::new(1);
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
        let mut journal = MemJournal::new(1);
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
        let mut journal = MemJournal::new(1);
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
        assert_eq!(run(MemJournal::new(1)), run(MemJournal::new(1)));
    }

    #[test]
    fn a_call_closes_exactly_once() {
        let mut journal = MemJournal::new(1);
        let intent = intent("echo", 7);
        let call = intent.call_ref;
        journal.open_outbound(intent.clone()).unwrap();
        assert_eq!(
            journal.open_outbound(intent).unwrap_err(),
            Error::DuplicateOpen
        );
        let response = Closure::Timeout;
        assert_eq!(
            journal.close_outbound(&call, response).unwrap(),
            CloseVerdict::Won
        );
        // The loser becomes a discarded observation.
        let late = Closure::Response(
            MemJournal::new(1)
                .append_inbound(submission("late", SubmissionSequence::Next))
                .unwrap()
                .envelope,
        );
        assert_eq!(
            journal.close_outbound(&call, late).unwrap(),
            CloseVerdict::AlreadyClosed
        );
        let Record::Outbound(record) = &journal.records()[0] else {
            panic!("expected the outbound record");
        };
        assert_eq!(record.status, OutboundStatus::Closed(Closure::Timeout));
        // A response for an unknown call is a fault, not a silence.
        let ghost = Response {
            call_ref: ids::call_ref(
                Id::from_bytes([9; 16]),
                &node::Path::new(["ghost"]).unwrap(),
                &[0; 32],
            ),
            data: Data::encoded(Encoding::Raw, *b"?"),
        };
        assert_eq!(
            journal.close_outbound(&ghost.call_ref, Closure::Timeout),
            Err(Error::UnknownCall)
        );
    }

    #[test]
    fn records_keep_causal_order() {
        let mut journal = MemJournal::new(1);
        let committed = journal
            .append_inbound(submission("inbound", SubmissionSequence::Next))
            .unwrap();
        journal.commit(TransactionRecord {
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
        });
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
    }
}
