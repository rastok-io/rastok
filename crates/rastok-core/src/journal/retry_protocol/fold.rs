//! Pure attempt protocol. Spatial admission, owner ancestry, output typing and
//! the exact carrier are independently checked when accepting journal records.

use super::*;
use crate::boundary::Intent;
use crate::canonical;
use crate::instance;

use crate::journal::{DeadlineKind, Emission, ids, timing::Deadlines};
use alloc::collections::BTreeSet;
use core::fmt;

/// Why a candidate packet or Call fact cannot extend the retained prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Invalid bound, case selection, delay or original repetition guarantee.
    Policy,
    /// Missing or contradictory original plan/Contract/entry/envelope binding.
    OriginalBinding,
    /// Opening delivery, generation or operation identity does not agree.
    OpeningIdentity,
    /// The exact opening delivery already owns an operation.
    DuplicateOperation,
    /// No operation has been opened for this reference.
    UnknownOperation,
    /// Facts are on the wrong carrier or break an atomic pair.
    PacketShape,
    /// Phase, attempt ordinal or Closing contradicts this transition.
    Transition,
    /// A fresh admission changed DATA/metadata, its lineage or its cursor.
    AdmissionIdentity,
    /// An attempted Call changed the pinned request, settings, key or owner.
    PinnedCall,
    /// The terminal case is outside the original Contract.
    Terminal,
    /// Closure time or original delay evidence is missing or contradictory.
    Timing,
    /// Receipt/publication does not describe the exact whole emission batch.
    Batch,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Policy => "invalid Retry policy or repetition guarantee",
            Self::OriginalBinding => {
                "Retry requires its original validated plan and Contract binding"
            }
            Self::OpeningIdentity => "Retry operation contradicts its exact opening admission",
            Self::DuplicateOperation => "Retry operation was already opened",
            Self::UnknownOperation => "Retry fact has no original operation",
            Self::PacketShape => "Retry packet has an invalid carrier or broken atomic pair",
            Self::Transition => "Retry fact contradicts the retained phase, ordinal or Closing",
            Self::AdmissionIdentity => "Retry admission changed pinned state, lineage or delivery",
            Self::PinnedCall => "Retry Call contradicts the original prepared operation",
            Self::Terminal => "Retry terminal is outside the original Contract",
            Self::Timing => "Retry deadline contradicts the original winning closure",
            Self::Batch => "Retry batch does not match its exact ordered producer emissions",
        })
    }
}

impl core::error::Error for Error {}

/// Authority supplied by original plan validation and the opening delivery.
/// It is deliberately not serialized in each subsequent attempt.
#[derive(Debug)]
pub struct Opening<'a> {
    /// Exact owner generation of the original delivery.
    pub instance: &'a instance::Generation,
    /// Canonical delivery admitted by the original journal/route authority.
    pub delivery: Id,
    /// Original validated plan hash.
    pub plan: SemanticHash,
    /// Original exact Contract.
    pub contract: &'a ContractId,
    /// Its resolved Call profile.
    pub call: CallSemantics,
    /// Its checked immutable Retry policy.
    pub policy: &'a Policy,
    /// Original envelope, fetched from the retained opening delivery.
    pub envelope: &'a Envelope,
}

/// Winning EGRESS closure inputs, before its Connector transaction exists.
#[derive(Debug)]
pub struct Settlement<'a> {
    /// Call's retained preparation; later attempt expressions are never used.
    pub intent: &'a Intent,
    /// Terminal re-derived from the validated completion/timeout/abandonment.
    pub case: &'a str,
    /// Explicit VM logical instant accepted with this winning closure.
    pub closed_at: i64,
    /// Exact original retained arm, or absence for a final/non-delayed attempt.
    pub schedule: Option<&'a Deadlines>,
}

/// Reconstructed phase; only a journal transition can move to its successor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Opened but never admitted.
    Initial,
    /// Admitted, no EGRESS open yet.
    Admitted,
    /// EGRESS open; recovery must not recall this effect.
    CallOpen,
    /// Winning closure retained, completion not yet committed.
    Settled,
    /// Exact complete batch retained for its manager, not yet acknowledged.
    Held,
    /// Consumed/advanced, waiting for original due time and capacity.
    AwaitingAdmission,
    /// Final publication or exact-generation stop closed the operation.
    Closed,
}

/// Journal-derived operation state. History owns batches; only the unconsumed
/// batch remains pending here, irrespective of the configured attempt bound.
#[derive(Debug, Clone, PartialEq)]
pub struct Operation {
    /// Exact owner, not a key guessed from DATA or an owner-name prefix.
    pub instance: instance::Generation,
    /// Original plan binding.
    pub plan: SemanticHash,
    /// Original exact Contract.
    pub contract: ContractId,
    /// Recurrent request port from that Contract.
    pub entry: Port,
    /// Pinned prepared operation.
    pub pin: Pin,
    /// Original envelope, used to verify temporal admissions without re-evaluation.
    pub original: Envelope,
    /// Exact opening delivery.
    pub opening: Id,
    /// Current admitted attempt, zero only before the first admission.
    pub ordinal: u64,
    /// Last admission's full envelope.
    pub envelope: Envelope,
    /// Current admitted cursor identity.
    pub delivery: Id,
    /// Current protocol phase.
    pub phase: Phase,
    /// Current Call, never reused by a later attempt.
    pub call: Option<CallId>,
    /// Original winning closure instant of this attempt.
    pub closed_at: Option<i64>,
    /// Original deadline, unaffected by a late manager or recovery.
    pub due_at: Option<i64>,
    /// Exact pending receipt; consumed history is not duplicated here.
    pub pending: Option<Batch>,
    /// Why this operation ended.
    pub closed: Option<CloseReason>,
    admitted_at: Option<i64>,
    call_opened_at: Option<i64>,
    policy: Policy,
    terminal: Option<String>,
    parent: Option<Id>,
    withhold: bool,
}

impl Operation {
    /// Original unsealed instant of this attempt's admission.
    #[must_use]
    pub const fn admitted_at(&self) -> Option<i64> {
        self.admitted_at
    }

    /// Disposition fixed by the winning closure, before any later Closing.
    #[must_use]
    pub const fn withholds(&self) -> bool {
        self.withhold
    }

    /// The preceding terminal, retained by the exact complete batch.
    #[must_use]
    pub const fn terminal_parent(&self) -> Option<Id> {
        self.parent
    }

    /// Policy re-derived from the original validated program.
    #[must_use]
    pub const fn policy(&self) -> &Policy {
        &self.policy
    }
}

/// Pure fold of accepted operation facts. No Driver or clock is accessible.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fold {
    operations: BTreeMap<OperationId, Operation>,
    closing: BTreeSet<instance::Generation>,
}

impl Fold {
    /// Inspect one exact operation.
    #[must_use]
    pub fn operation(&self, reference: OperationId) -> Option<&Operation> {
        self.operations.get(&reference)
    }

    /// Retained operations in canonical identity order.
    pub fn operations(&self) -> impl Iterator<Item = (&OperationId, &Operation)> {
        self.operations.iter()
    }

    /// Observe an already-retained exact-generation Closing fact. Ancestor
    /// cascade is resolved by the lifecycle fold, never a string-prefix test.
    pub fn begin_closing(&mut self, instance: &instance::Generation) {
        self.closing.insert(instance.clone());
    }

    /// Exact-generation Closing has already won in the retained prefix.
    #[must_use]
    pub fn is_closing(&self, instance: &instance::Generation) -> bool {
        self.closing.contains(instance)
    }

    /// Fold one atomic packet. `at` is an explicit retained logical instant;
    /// it can establish admission eligibility but never renews a deadline.
    /// The opening authority is required only for an Opened packet.
    ///
    /// # Errors
    /// Contradictory facts leave the entire fold unchanged.
    pub fn apply(
        &mut self,
        packet: &Packet,
        opening: Option<&Opening<'_>>,
        at: i64,
    ) -> Result<(), Error> {
        let mut next = self.clone();
        next.apply_inner(packet, opening, at)?;
        *self = next;
        Ok(())
    }

    fn apply_inner(
        &mut self,
        packet: &Packet,
        opening: Option<&Opening<'_>>,
        at: i64,
    ) -> Result<(), Error> {
        match (packet.context, packet.facts.as_slice()) {
            (None, [fact @ Fact::Opened { .. }]) => {
                self.open(fact, opening.ok_or(Error::OriginalBinding)?, at)
            }
            (None, [fact @ Fact::Opened { .. }, admit @ Fact::Admitted { .. }])
                if fact.operation() == admit.operation() =>
            {
                self.open(fact, opening.ok_or(Error::OriginalBinding)?, at)?;
                self.admit(admit, at)
            }
            (None, [fact @ Fact::Admitted { .. }]) => self.admit(fact, at),
            (Some(context), [Fact::Held(batch)]) if context == batch.attempt => {
                self.produce(batch, true)
            }
            (
                Some(context),
                [
                    Fact::Published(batch),
                    Fact::Close {
                        attempt,
                        reason: CloseReason::Final,
                    },
                ],
            ) if context == batch.attempt && context == *attempt => {
                self.produce(batch, false)?;
                self.close(*attempt, CloseReason::Final)
            }
            (None, [Fact::Consumed(batch), Fact::Advance { from, to, call }])
                if batch.attempt == *from && batch.call == *call =>
            {
                self.consume(batch)?;
                let state = self.state_mut(*from)?;
                if from.ordinal.checked_add(1) != Some(*to) || *to > state.policy.attempts {
                    return Err(Error::Transition);
                }
                let instance = state.instance.clone();
                if self.closing.contains(&instance) {
                    return Err(Error::Transition);
                }
                self.state_mut(*from)?.phase = Phase::AwaitingAdmission;
                Ok(())
            }
            (
                None,
                [
                    Fact::Consumed(batch),
                    Fact::Close {
                        attempt,
                        reason: CloseReason::Stopped,
                    },
                ],
            ) if batch.attempt == *attempt => {
                self.consume(batch)?;
                self.close(*attempt, CloseReason::Stopped)
            }
            (
                None,
                [
                    Fact::Close {
                        attempt,
                        reason: CloseReason::Stopped,
                    },
                ],
            ) => self.close(*attempt, CloseReason::Stopped),
            _ => Err(Error::PacketShape),
        }
    }

    fn open(&mut self, fact: &Fact, authority: &Opening<'_>, at: i64) -> Result<(), Error> {
        let Fact::Opened {
            operation,
            opening,
            entry,
            plan,
            contract,
            pin,
        } = fact
        else {
            return Err(Error::PacketShape);
        };
        if self.operations.contains_key(operation) {
            return Err(Error::DuplicateOperation);
        }
        if *operation != operation_ref(authority.instance, entry, *opening) {
            return Err(Error::OpeningIdentity);
        }
        if *plan != authority.plan
            || *opening != authority.delivery
            || contract != authority.contract
            || authority.policy.call != authority.call
            || entry.name() != authority.call.request()
            || pin.envelope != authority.envelope.meta().sys().id()
            || pin.opened_at > at
            || self.closing.contains(authority.instance)
        {
            return Err(Error::OriginalBinding);
        }
        self.operations.insert(
            *operation,
            Operation {
                instance: authority.instance.clone(),
                plan: *plan,
                contract: contract.clone(),
                entry: entry.clone(),
                pin: pin.clone(),
                original: authority.envelope.clone(),
                opening: *opening,
                ordinal: 0,
                envelope: authority.envelope.clone(),
                delivery: *opening,
                phase: Phase::Initial,
                call: None,
                closed_at: None,
                due_at: None,
                pending: None,
                closed: None,
                admitted_at: None,
                call_opened_at: None,
                policy: authority.policy.clone(),
                terminal: None,
                parent: None,
                withhold: false,
            },
        );
        Ok(())
    }

    fn state_mut(&mut self, attempt: AttemptId) -> Result<&mut Operation, Error> {
        let state = self
            .operations
            .get_mut(&attempt.operation)
            .ok_or(Error::UnknownOperation)?;
        if state.ordinal != attempt.ordinal || state.phase == Phase::Closed {
            return Err(Error::Transition);
        }
        Ok(state)
    }

    fn admit(&mut self, fact: &Fact, at: i64) -> Result<(), Error> {
        let Fact::Admitted {
            attempt,
            envelope,
            delivery,
        } = fact
        else {
            return Err(Error::PacketShape);
        };
        let state = self
            .operations
            .get_mut(&attempt.operation)
            .ok_or(Error::UnknownOperation)?;
        if self.closing.contains(&state.instance)
            || state.ordinal.checked_add(1) != Some(attempt.ordinal)
            || attempt.ordinal > state.policy.attempts
        {
            return Err(Error::Transition);
        }
        match state.phase {
            Phase::Initial if attempt.ordinal == 1 => {
                if envelope.meta().sys() != state.original.meta().sys()
                    || !same_state(envelope, &state.original)
                    || *delivery != state.opening
                {
                    return Err(Error::AdmissionIdentity);
                }
                if at < state.pin.opened_at {
                    return Err(Error::Timing);
                }
            }
            Phase::AwaitingAdmission => {
                let call = state.call.ok_or(Error::Transition)?;
                let id = admission_ref(attempt.operation, attempt.ordinal, call);
                let sys = envelope.meta().sys();
                let initial = state.original.meta();
                if sys.id() != id
                    || sys.parent_id() != state.parent
                    || sys.origin_id() != initial.sys().origin_id()
                    || sys.role() != initial.sys().role()
                    || sys.graph_version() != initial.sys().graph_version()
                    || !same_state(envelope, &state.original)
                    || *delivery
                        != ids::delivery_id(&admission_edge(attempt.operation, &state.entry), id)
                {
                    return Err(Error::AdmissionIdentity);
                }
                if state.closed_at.is_none_or(|closed| at < closed)
                    || state.due_at.is_some_and(|due| at < due)
                {
                    return Err(Error::Timing);
                }
            }
            _ => return Err(Error::Transition),
        }
        state.ordinal = attempt.ordinal;
        state.admitted_at = Some(at);
        state.delivery = *delivery;
        state.envelope = envelope.clone();
        state.phase = Phase::Admitted;
        state.call = None;
        state.call_opened_at = None;
        state.closed_at = None;
        state.due_at = None;
        state.terminal = None;
        state.parent = None;
        state.withhold = false;
        Ok(())
    }

    /// Accept the admitted attempt's first EGRESS open. Recovery never calls
    /// this as permission to offer the same effect to a Driver again.
    ///
    /// # Errors
    /// Unadmitted/repeated Call, wrong owner/cursor, or changed pinned material.
    pub fn open_call(&mut self, attempt: AttemptId, intent: &Intent) -> Result<(), Error> {
        let state = self.state_mut(attempt)?;
        if state.phase != Phase::Admitted {
            return Err(Error::Transition);
        }
        Self::check_intent(state, intent)?;
        state.call = Some(intent.call_id);
        state.call_opened_at = Some(intent.logical_time);
        state.phase = Phase::CallOpen;
        Ok(())
    }

    fn check_intent(state: &Operation, intent: &Intent) -> Result<(), Error> {
        if intent.connector_path.segments() != state.instance.address().owner().segments()
            || intent.prepared != state.pin.prepared
            || intent.resolved_settings != state.pin.settings
            || intent.idempotency_key != state.pin.key
            || state.admitted_at.is_none_or(|at| intent.logical_time < at)
            || intent.emitting_id != state.envelope.meta().sys().id()
            || ids::delivery_id(&intent.edge, intent.emitting_id) != state.delivery
            || ids::call_id(state.delivery, state.pin.prepared.digest()) != intent.call_id
        {
            return Err(Error::PinnedCall);
        }
        Ok(())
    }

    /// Retain the unique winning terminal and its original deadline. Closing
    /// already present suppresses recurrence; later Closing cannot change this
    /// committed disposition into a graph publication.
    ///
    /// # Errors
    /// Nonwinning/mismatched Call, unknown terminal, or inconsistent time/arm.
    pub fn settle(&mut self, attempt: AttemptId, settlement: &Settlement<'_>) -> Result<(), Error> {
        let mut next = self.clone();
        let state = next
            .operations
            .get_mut(&attempt.operation)
            .ok_or(Error::UnknownOperation)?;
        if state.ordinal != attempt.ordinal
            || state.phase != Phase::CallOpen
            || state.call != Some(settlement.intent.call_id)
            || state.call_opened_at != Some(settlement.intent.logical_time)
        {
            return Err(Error::Transition);
        }
        Self::check_intent(state, settlement.intent)?;
        if !state
            .policy
            .call
            .terminal_cases()
            .iter()
            .any(|(case, _)| *case == settlement.case)
        {
            return Err(Error::Terminal);
        }
        if state
            .call_opened_at
            .is_none_or(|opened| settlement.closed_at < opened)
        {
            return Err(Error::Timing);
        }
        let withhold = state.policy.repeats(attempt.ordinal, settlement.case)
            && !next.closing.contains(&state.instance);
        let due = withhold
            .then(|| {
                state
                    .policy
                    .due_after(attempt.ordinal, settlement.closed_at)
            })
            .flatten();
        match (due, settlement.schedule) {
            (None, None) => {}
            (Some(due), Some(schedule)) if schedule.created_at() == settlement.closed_at => {
                let [arm] = schedule.items() else {
                    return Err(Error::Timing);
                };
                if arm.kind != DeadlineKind::RecurrenceDelay
                    || arm.due_at != due
                    || arm.reference
                        != admission_ref(
                            attempt.operation,
                            attempt.ordinal + 1,
                            settlement.intent.call_id,
                        )
                {
                    return Err(Error::Timing);
                }
            }
            _ => return Err(Error::Timing),
        }
        state.phase = Phase::Settled;
        state.withhold = withhold;
        state.closed_at = Some(settlement.closed_at);
        state.due_at = due;
        state.terminal = Some(String::from(settlement.case));
        *self = next;
        Ok(())
    }

    fn produce(&mut self, batch: &Batch, held: bool) -> Result<(), Error> {
        let state = self.state_mut(batch.attempt)?;
        if state.phase != Phase::Settled || state.call != Some(batch.call) || state.withhold != held
        {
            return Err(Error::Transition);
        }
        if batch.emissions.is_empty()
            || batch
                .emissions
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
                != batch.emissions.len()
        {
            return Err(Error::Batch);
        }
        state.parent = batch.emissions.last().copied();
        if held {
            state.pending = Some(batch.clone());
            state.phase = Phase::Held;
        }
        Ok(())
    }

    fn consume(&mut self, batch: &Batch) -> Result<(), Error> {
        let state = self.state_mut(batch.attempt)?;
        if state.phase != Phase::Held || state.pending.as_ref() != Some(batch) {
            return Err(Error::Batch);
        }
        state.pending = None;
        Ok(())
    }

    fn close(&mut self, attempt: AttemptId, reason: CloseReason) -> Result<(), Error> {
        let state = self
            .operations
            .get_mut(&attempt.operation)
            .ok_or(Error::UnknownOperation)?;
        if state.ordinal != attempt.ordinal || state.pending.is_some() {
            return Err(Error::Transition);
        }
        match reason {
            CloseReason::Final
                if state.phase == Phase::Settled && !state.withhold && state.parent.is_some() => {}
            CloseReason::Stopped
                if self.closing.contains(&state.instance)
                    && matches!(
                        state.phase,
                        Phase::Initial | Phase::AwaitingAdmission | Phase::Held
                    ) => {}
            _ => return Err(Error::Transition),
        }
        state.phase = Phase::Closed;
        state.closed = Some(reason);
        Ok(())
    }

    /// Check a producer packet against the transaction's actual ordered
    /// emissions. Empty physical routes are never a substitute for this fact.
    ///
    /// # Errors
    /// Missing, reordered, duplicated or alien emission/terminal references.
    pub fn validate_emissions(&self, packet: &Packet, emissions: &[Emission]) -> Result<(), Error> {
        let batch = match packet.facts.first() {
            Some(Fact::Held(batch) | Fact::Published(batch)) => batch,
            _ => return Err(Error::PacketShape),
        };
        let state = self
            .operations
            .get(&batch.attempt.operation)
            .ok_or(Error::UnknownOperation)?;
        if state.ordinal != batch.attempt.ordinal
            || state.call != Some(batch.call)
            || batch.emissions.len() != emissions.len()
            || !batch
                .emissions
                .iter()
                .zip(emissions)
                .all(|(id, emission)| *id == emission.envelope.meta().sys().id())
        {
            return Err(Error::Batch);
        }
        let Some(terminal) = emissions.last() else {
            return Err(Error::Batch);
        };
        if terminal.port.name() != state.policy.call.terminal()
            || emissions[..emissions.len() - 1]
                .iter()
                .any(|emission| emission.port.name() == state.policy.call.terminal())
            || terminal
                .envelope
                .data()
                .materialized_value()
                .and_then(|value| state.policy.call.terminal_case(value))
                != state.terminal.as_deref()
        {
            return Err(Error::Batch);
        }
        let frames = &emissions[..emissions.len() - 1];
        if frames.iter().any(|frame| {
            state
                .policy
                .call
                .frame_production(frame.port.name())
                .is_none()
        }) || state
            .policy
            .call
            .frame_ports()
            .iter()
            .any(|(port, production)| {
                !production.accepts_count(
                    state.terminal.as_deref().unwrap_or(""),
                    frames
                        .iter()
                        .filter(|frame| frame.port.name() == *port)
                        .count(),
                )
            })
        {
            return Err(Error::Batch);
        }
        Ok(())
    }
}

// Scalar metadata has IEEE comparison semantics; retained identity instead
// normalizes NaN and distinguishes signed zero, through the existing registry.
fn same_state(left: &Envelope, right: &Envelope) -> bool {
    left.data() == right.data()
        && [left.meta().app(), left.meta().ext()]
            .into_iter()
            .zip([right.meta().app(), right.meta().ext()])
            .all(|(left, right)| {
                left.len() == right.len()
                    && left.iter().zip(right).all(|((ln, lv), (rn, rv))| {
                        ln == rn && canonical::meta_value(lv) == canonical::meta_value(rv)
                    })
            })
}
