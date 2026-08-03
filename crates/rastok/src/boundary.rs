//! Contracts with the external world.
//!
//! Governing rule: [`delivery-semantics.md`]. In run, the world plays
//! itself; in simulation, the lab plays the world — both through the
//! same traits, and the journal keeps its world-half either way. The
//! [`Dispatcher`] is the loop between the journal and the partners:
//! it drives open outbound calls and settles their closures; only a
//! won closure ever becomes a VM input.
//!
//! [`delivery-semantics.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/delivery-semantics.md

use crate::envelope::{self, Envelope};
use crate::journal::{CloseVerdict, Closure, Journal, OutboundStatus, Record, ids};
use crate::node;
use std::collections::BTreeSet;
use std::fmt;

/// Opaque ingress identity: exactly `(connector_path, replica)`.
/// Opaque so the replica is never inlined anywhere else — the
/// multi-host door stays open. In single-process execution the
/// replica is 0.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IngressAuthority {
    connector_path: node::Path,
    replica: u64,
}

impl IngressAuthority {
    /// The single-process authority for one ingress connector.
    #[must_use]
    pub fn single(connector_path: node::Path) -> Self {
        Self {
            connector_path,
            replica: 0,
        }
    }

    /// The ingress connector's root-relative path — root-recipe input.
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

/// Canonical journaled outbound-call intent (`identity.md` §3, keys
/// 40–43): journaled open before the effect executes.
#[derive(Debug, Clone, PartialEq)]
pub struct Intent {
    /// The derived internal call identity.
    pub call_ref: CallRef,
    /// The envelope that opened the call — the cause.
    pub emitting_id: envelope::Id,
    /// The calling connector node.
    pub connector_path: node::Path,
    /// The boundary integrity seal over the request bytes.
    pub request_digest: [u8; 32],
    /// Names the business operation; stable across retries. `None`
    /// only for `compensable` without a key.
    pub idempotency_key: Option<String>,
    /// The declared effect class of the call.
    pub effect: node::EffectClass,
}

/// Protocol-correlated partner answer. A partner answers or stays
/// silent — never a timeout: timeouts are the scheduler's.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    /// The call this answers, correlated by the connector's own
    /// protocol.
    pub call_ref: CallRef,
    /// The answer payload.
    pub data: envelope::Data,
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

/// A draft proposed for atomic acceptance: ingress births drafts,
/// never envelopes — a root envelope cannot pre-exist its sequence.
#[derive(Debug, Clone, PartialEq)]
pub struct InboundSubmission {
    /// Who proposes it.
    pub authority: IngressAuthority,
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
    /// The content-type hint, when the transport provides one.
    pub mime_type: Option<String>,
    /// The writable `meta.app` namespace seeded at birth.
    pub app: envelope::Namespace,
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
    /// A call's timeout budget expired.
    CallTimeout(CallRef),
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
pub trait Ingress {
    /// The next submission, if one is waiting.
    fn next(&mut self) -> Option<InboundSubmission>;
}

/// Starts external calls and supplies their answers. A partner
/// answers or stays silent — never a timeout and never an unknown
/// outcome: budgets belong to the scheduler, triage to the recovery
/// fold. Correlation uses the connector's own protocol.
pub trait Partner {
    /// Executes the effect of an already-journaled open call. The
    /// request envelope rides along because the intent seals only the
    /// digest of the request, not its bytes.
    fn start(&mut self, intent: &Intent, request: &Envelope);

    /// The answers that arrived since the last drain.
    fn events(&mut self) -> Vec<Response>;
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

/// The loop between the journal and the partners. It starts every
/// journaled open call exactly once — journal-before-effect: the
/// record always exists before the partner hears — and settles
/// closures through the journal's close-once race. Mocks replace the
/// partner, never this bookkeeping.
#[derive(Debug, Default)]
pub struct Dispatcher {
    started: BTreeSet<CallRef>,
}

impl Dispatcher {
    /// A dispatcher with no started calls.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Drives one round: starts newly journaled open calls, collects
    /// the partner's answers, builds each response's outcome envelope
    /// — emitted by the connector node under `p:in`, parented on the
    /// request, inheriting its origin — and settles the close race.
    /// Returns the outcome envelopes that won; a losing or
    /// uncorrelated answer is a discarded observation.
    pub fn pump(&mut self, journal: &mut dyn Journal, partner: &mut dyn Partner) -> Vec<Envelope> {
        // Start phase: every open record not yet started.
        let pending: Vec<Intent> = journal
            .records()
            .iter()
            .filter_map(|record| match record {
                Record::Outbound(outbound)
                    if outbound.status == OutboundStatus::Open
                        && !self.started.contains(&outbound.intent.call_ref) =>
                {
                    Some(outbound.intent.clone())
                }
                _ => None,
            })
            .collect();
        for intent in pending {
            let Some(request) = find_envelope(journal, intent.emitting_id) else {
                continue;
            };
            let request = request.clone();
            self.started.insert(intent.call_ref);
            partner.start(&intent, &request);
        }
        // Settle phase: only a won closure becomes a VM input.
        let mut outcomes = Vec::new();
        for response in partner.events() {
            let Some(intent) = open_intent(journal, &response.call_ref) else {
                continue; // uncorrelated or already settled: discarded observation
            };
            let Some(request) = find_envelope(journal, intent.emitting_id) else {
                continue;
            };
            let trigger = node::Trigger::Port(
                node::Port::new("in").unwrap_or_else(|_| unreachable!("`in` is a valid port")),
            );
            let port =
                node::Port::new("out").unwrap_or_else(|_| unreachable!("`out` is a valid port"));
            let id = ids::emission_id(
                &intent.connector_path,
                &trigger,
                intent.emitting_id,
                &port,
                0,
            );
            let sys = envelope::MetaSys::emission(
                id,
                request.meta().sys(),
                request.meta().sys().graph_version(),
            );
            let outcome = Envelope::new(envelope::Meta::new(sys), response.data);
            if journal.close_outbound(&response.call_ref, Closure::Response(outcome.clone()))
                == Ok(CloseVerdict::Won)
            {
                outcomes.push(outcome);
            }
        }
        outcomes
    }
}

/// The envelope carrying `id`, wherever the journal recorded it.
fn find_envelope(journal: &dyn Journal, id: envelope::Id) -> Option<&Envelope> {
    journal.records().iter().find_map(|record| match record {
        Record::Inbound(inbound) if inbound.envelope.meta().sys().id() == id => {
            Some(&inbound.envelope)
        }
        Record::Transaction(transaction) => transaction
            .emissions
            .iter()
            .map(|emission| &emission.envelope)
            .find(|envelope| envelope.meta().sys().id() == id),
        Record::Outbound(outbound) => match &outbound.status {
            OutboundStatus::Closed(Closure::Response(envelope))
                if envelope.meta().sys().id() == id =>
            {
                Some(envelope)
            }
            _ => None,
        },
        Record::Inbound(_) => None,
    })
}

/// The intent of a still-open call.
fn open_intent(journal: &dyn Journal, call_ref: &CallRef) -> Option<Intent> {
    journal.records().iter().find_map(|record| match record {
        Record::Outbound(outbound)
            if outbound.intent.call_ref == *call_ref && outbound.status == OutboundStatus::Open =>
        {
            Some(outbound.intent.clone())
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::{Data, Encoding, Namespace};
    use crate::journal::MemJournal;

    #[test]
    fn the_authority_stays_opaque_but_answers_the_recipes() {
        let authority = IngressAuthority::single(node::Path::new(["inbound"]).unwrap());
        assert_eq!(authority.connector_path().to_string(), "inbound");
        assert_eq!(authority.replica(), 0);
    }

    /// A scripted partner: records what it was asked to start,
    /// answers from a script.
    #[derive(Default)]
    struct Scripted {
        started: Vec<(Intent, Envelope)>,
        replies: Vec<Response>,
    }

    impl Partner for Scripted {
        fn start(&mut self, intent: &Intent, request: &Envelope) {
            self.started.push((intent.clone(), request.clone()));
        }

        fn events(&mut self) -> Vec<Response> {
            std::mem::take(&mut self.replies)
        }
    }

    fn seeded_journal() -> (MemJournal, Intent) {
        let mut journal = MemJournal::new(1);
        let committed = journal
            .append_inbound(InboundSubmission {
                authority: IngressAuthority::single(node::Path::new(["inbound"]).unwrap()),
                draft: Draft {
                    data: Data::encoded(Encoding::Raw, *b"salut"),
                    mime_type: None,
                    app: Namespace::new(),
                },
                sequence: SubmissionSequence::Next,
            })
            .unwrap();
        let connector_path = node::Path::new(["echo"]).unwrap();
        let emitting_id = committed.envelope.meta().sys().id();
        let request_digest = [7; 32];
        let intent = Intent {
            call_ref: ids::call_ref(emitting_id, &connector_path, &request_digest),
            emitting_id,
            connector_path,
            request_digest,
            idempotency_key: None,
            effect: node::EffectClass::Idempotent,
        };
        journal.open_outbound(intent.clone()).unwrap();
        (journal, intent)
    }

    #[test]
    fn the_record_exists_before_the_partner_hears() {
        let (mut journal, intent) = seeded_journal();
        let mut partner = Scripted::default();
        let mut dispatcher = Dispatcher::new();
        dispatcher.pump(&mut journal, &mut partner);
        assert_eq!(partner.started.len(), 1);
        assert_eq!(partner.started[0].0.call_ref, intent.call_ref);
        // A second pump never re-starts an already-started call.
        dispatcher.pump(&mut journal, &mut partner);
        assert_eq!(partner.started.len(), 1);
    }

    #[test]
    fn a_won_closure_becomes_the_only_vm_input() {
        let (mut journal, intent) = seeded_journal();
        let mut partner = Scripted::default();
        let mut dispatcher = Dispatcher::new();
        dispatcher.pump(&mut journal, &mut partner);
        partner.replies = vec![
            Response {
                call_ref: intent.call_ref,
                data: Data::encoded(Encoding::Raw, *b"SALUT"),
            },
            // A duplicate answer for the same call: the race loser.
            Response {
                call_ref: intent.call_ref,
                data: Data::encoded(Encoding::Raw, *b"LATE"),
            },
        ];
        let outcomes = dispatcher.pump(&mut journal, &mut partner);
        assert_eq!(outcomes.len(), 1, "only the won closure feeds the VM");
        let outcome = &outcomes[0];
        // The outcome is the connector's emission under p:in, parented
        // on the request, inheriting its origin.
        let expected_id = ids::emission_id(
            &intent.connector_path,
            &node::Trigger::Port(node::Port::new("in").unwrap()),
            intent.emitting_id,
            &node::Port::new("out").unwrap(),
            0,
        );
        assert_eq!(outcome.meta().sys().id(), expected_id);
        assert_eq!(outcome.meta().sys().parent_id(), Some(intent.emitting_id));
        assert_eq!(outcome.meta().sys().origin_id(), intent.emitting_id);
    }

    #[test]
    fn an_uncorrelated_answer_is_a_discarded_observation() {
        let (mut journal, _) = seeded_journal();
        let mut partner = Scripted::default();
        let mut dispatcher = Dispatcher::new();
        partner.replies = vec![Response {
            call_ref: ids::call_ref(
                envelope::Id::from_bytes([9; 16]),
                &node::Path::new(["ghost"]).unwrap(),
                &[0; 32],
            ),
            data: Data::encoded(Encoding::Raw, *b"?"),
        }];
        let outcomes = dispatcher.pump(&mut journal, &mut partner);
        assert!(outcomes.is_empty());
    }
}
