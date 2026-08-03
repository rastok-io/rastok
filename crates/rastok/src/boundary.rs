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
use crate::journal::{CloseVerdict, Closure, Journal};
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
    /// What the partner reports — success or a typed failure.
    pub outcome: CallOutcome,
}

/// What a call produced: the boundary can say failure, because every
/// connector contract has an error set beside its reply.
#[derive(Debug, Clone, PartialEq)]
pub enum CallOutcome {
    /// The reply payload, bound for the connector's `out` port.
    Success(envelope::Data),
    /// A member of the connector's typed error set, bound for `err`.
    Failure(TypedError),
}

/// One member of a connector's declared error set.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedError {
    /// The error kind, from the contract sheet's error set.
    pub kind: String,
    /// The typed payload of the error.
    pub data: envelope::Data,
}

/// Judges a partner's outcome against the calling connector's
/// contract before it may become a causal fact. The plan owns the
/// error sets; the dispatcher only asks.
pub trait OutcomeGate {
    /// `Ok` to admit; `Err(reason)` to refuse the closure.
    ///
    /// # Errors
    ///
    /// The refusal reason, surfaced with the rejected response.
    fn admit(&self, connector: &node::Path, outcome: &CallOutcome) -> Result<(), String>;
}

/// The gate that admits everything — for worlds whose outcomes are
/// validated elsewhere, and for tests of the dispatch mechanics.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenGate;

impl OutcomeGate for OpenGate {
    fn admit(&self, _: &node::Path, _: &CallOutcome) -> Result<(), String> {
        Ok(())
    }
}

/// One routable boundary fact for the VM: which connector, which
/// port, which cause. The VM owns what follows — the connector's
/// transaction builds the outcome envelope under the emission recipe;
/// the dispatcher never mints envelopes.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundaryEvent {
    /// The settled call.
    pub call_ref: CallRef,
    /// The connector node that owns the coming transaction.
    pub connector_path: node::Path,
    /// The request envelope's id — the transaction's parent.
    pub emitting_id: envelope::Id,
    /// `out` for a success, `err` for a typed failure.
    pub port: node::Port,
    /// What the call produced.
    pub outcome: CallOutcome,
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
    /// A delivery's call budget expired. The budget runs from the
    /// DELIVERY into the connector's mailbox (execution-model §8) —
    /// queue time counts — so the reference is the delivered
    /// envelope's id, resolvable whether the call is still queued or
    /// already open.
    DeliveryBudget {
        /// The connector whose budget this is (the mailbox to search).
        connector: node::Path,
        /// The canonical edge the envelope traveled — with the id,
        /// the obligation's identity: two ports of one connector are
        /// two obligations.
        edge: String,
        /// The delivered envelope's id.
        delivery: envelope::Id,
    },
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
    /// Answers the outcome gate refused, with the reason. The call
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
            Self::Journal(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for DispatchError {}

/// The loop between the journal and the partners. It starts every
/// journaled open call exactly once — journal-before-effect: the
/// record always exists before the partner hears — and settles
/// closures through the journal's close-once race into routable
/// [`BoundaryEvent`]s. The VM owns everything after: the connector's
/// transaction builds the outcome envelope; the dispatcher never
/// mints one. Mocks replace the partner, never this bookkeeping.
///
/// Restart discipline: re-calling a partner is a recovery decision
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
    /// settles the partner's answers through the close-once race.
    ///
    /// # Errors
    ///
    /// [`DispatchError::MissingRequest`] when the causal history lacks
    /// a journaled request — an integrity fault; journal refusals are
    /// propagated, never absorbed.
    pub fn pump(
        &mut self,
        journal: &mut dyn Journal,
        partner: &mut dyn Partner,
        gate: &dyn OutcomeGate,
    ) -> Result<PumpBatch, DispatchError> {
        // Start phase: every open call not yet started.
        for intent in journal.open_calls() {
            if self.started.contains(&intent.call_ref) {
                continue;
            }
            let request = journal
                .find_envelope(intent.emitting_id)
                .ok_or(DispatchError::MissingRequest(intent.call_ref))?;
            self.started.insert(intent.call_ref);
            partner.start(&intent, &request);
        }
        // Settle phase: won closures become routable events; losers
        // and strays stay visible.
        let mut batch = PumpBatch::default();
        let open = journal.open_calls();
        for response in partner.events() {
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
            if let Err(reason) = gate.admit(&intent.connector_path, &response.outcome) {
                batch.rejected.push((response, reason));
                continue;
            }
            let closure = Closure::Response(response.outcome.clone());
            match journal
                .close_outbound(&response.call_ref, closure)
                .map_err(DispatchError::Journal)?
            {
                CloseVerdict::Won => {
                    let port_name = match &response.outcome {
                        CallOutcome::Success(_) => "out",
                        CallOutcome::Failure(_) => "err",
                    };
                    let port = node::Port::new(port_name)
                        .unwrap_or_else(|_| unreachable!("default port names are valid"));
                    self.started.remove(&response.call_ref);
                    batch.events.push(BoundaryEvent {
                        call_ref: response.call_ref,
                        connector_path: intent.connector_path.clone(),
                        emitting_id: intent.emitting_id,
                        port,
                        outcome: response.outcome,
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
    use super::*;
    use crate::envelope::{Data, Encoding, Namespace};
    use crate::journal::{MemJournal, ids};
    use crate::plan::GraphVersion;

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
        let mut journal = MemJournal::new(GraphVersion(1));
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
        dispatcher
            .pump(&mut journal, &mut partner, &OpenGate)
            .unwrap();
        assert_eq!(partner.started.len(), 1);
        assert_eq!(partner.started[0].0.call_ref, intent.call_ref);
        // A second pump never re-starts an already-started call.
        dispatcher
            .pump(&mut journal, &mut partner, &OpenGate)
            .unwrap();
        assert_eq!(partner.started.len(), 1);
    }

    #[test]
    fn a_resumed_dispatcher_re_calls_nothing() {
        let (mut journal, _) = seeded_journal();
        let mut partner = Scripted::default();
        // Restart over an existing journal: the open call counts as
        // started; re-calls are the recovery fold's decision.
        let mut dispatcher = Dispatcher::resume(&journal);
        dispatcher
            .pump(&mut journal, &mut partner, &OpenGate)
            .unwrap();
        assert!(partner.started.is_empty());
    }

    #[test]
    fn won_closures_become_routable_events() {
        let (mut journal, intent) = seeded_journal();
        let mut partner = Scripted::default();
        let mut dispatcher = Dispatcher::new();
        dispatcher
            .pump(&mut journal, &mut partner, &OpenGate)
            .unwrap();
        partner.replies = vec![
            Response {
                call_ref: intent.call_ref,
                outcome: CallOutcome::Success(Data::encoded(Encoding::Raw, *b"SALUT")),
            },
            // A duplicate answer for the same call: the race loser.
            Response {
                call_ref: intent.call_ref,
                outcome: CallOutcome::Success(Data::encoded(Encoding::Raw, *b"LATE")),
            },
        ];
        let batch = dispatcher
            .pump(&mut journal, &mut partner, &OpenGate)
            .unwrap();
        assert_eq!(batch.events.len(), 1, "only the won closure feeds the VM");
        assert_eq!(batch.discarded.len(), 1, "the loser stays visible");
        assert!(batch.uncorrelated.is_empty());
        let event = &batch.events[0];
        assert_eq!(event.connector_path.to_string(), "echo");
        assert_eq!(event.emitting_id, intent.emitting_id);
        assert_eq!(event.port.name(), "out");
        assert!(matches!(&event.outcome, CallOutcome::Success(_)));
        // The dispatcher minted no envelope: the journal's closure
        // fact carries the raw outcome, and the connector's own
        // transaction will build the envelope.
        assert!(journal.open_calls().is_empty());
    }

    #[test]
    fn a_typed_failure_routes_to_err() {
        let (mut journal, intent) = seeded_journal();
        let mut partner = Scripted::default();
        let mut dispatcher = Dispatcher::new();
        dispatcher
            .pump(&mut journal, &mut partner, &OpenGate)
            .unwrap();
        partner.replies = vec![Response {
            call_ref: intent.call_ref,
            outcome: CallOutcome::Failure(TypedError {
                kind: String::from("ExecFailed"),
                data: Data::encoded(Encoding::Raw, *b"exit 1"),
            }),
        }];
        let batch = dispatcher
            .pump(&mut journal, &mut partner, &OpenGate)
            .unwrap();
        assert_eq!(batch.events.len(), 1);
        assert_eq!(batch.events[0].port.name(), "err");
        assert!(
            matches!(&batch.events[0].outcome, CallOutcome::Failure(error) if error.kind == "ExecFailed")
        );
    }

    #[test]
    fn an_uncorrelated_answer_is_a_visible_fault() {
        let (mut journal, _) = seeded_journal();
        let mut partner = Scripted::default();
        let mut dispatcher = Dispatcher::new();
        partner.replies = vec![Response {
            call_ref: ids::call_ref(
                envelope::Id::from_bytes([9; 16]),
                &node::Path::new(["ghost"]).unwrap(),
                &[0; 32],
            ),
            outcome: CallOutcome::Success(Data::encoded(Encoding::Raw, *b"?")),
        }];
        let batch = dispatcher
            .pump(&mut journal, &mut partner, &OpenGate)
            .unwrap();
        assert!(batch.events.is_empty());
        assert_eq!(batch.uncorrelated.len(), 1, "correlation faults surface");
    }
}
