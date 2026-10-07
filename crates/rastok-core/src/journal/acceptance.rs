//! Cross-record deadline acceptance, shared by the live journal and recovery.
//! The record codec and deadline constructor check shape and individual dates;
//! this layer relates unsealed instants to sealed Call, transaction and recurrence facts.

use super::DeadlineKind;
use super::deadline::{Deadlines, Error};
use crate::envelope::Id;

pub(crate) fn validate_call(record: &super::CallRecord) -> Result<(), Error> {
    let context = if let Some(packet) = record.retry_record.as_deref() {
        let Some(context) = packet.context else {
            return Err(Error::MissingRetryContext);
        };
        if context.ordinal == 0 || !packet.facts.is_empty() {
            return Err(Error::InvalidRetryContext);
        }
        Some(context)
    } else {
        None
    };
    match (&record.status, record.closed_at) {
        (super::CallStatus::Open, None) => {}
        (super::CallStatus::Closed(_) | super::CallStatus::Abandoned, Some(at))
            if at >= record.intent.logical_time => {}
        (super::CallStatus::Open, Some(_)) => return Err(Error::UnexpectedClosureInstant),
        (_, None) => return Err(Error::MissingClosureInstant),
        (_, Some(_)) => return Err(Error::ClosurePrecedesOpening),
    }
    if let Some(schedule) = record.deadlines.as_deref() {
        let Some(context) = context else {
            return Err(Error::DeadlineWithoutRetryContext);
        };
        let Some(next) = context.ordinal.checked_add(1) else {
            return Err(Error::RetryOrdinalOverflow);
        };
        let reference =
            super::retry_protocol::admission_ref(context.operation, next, record.intent.call_id);
        if !matches!(record.status, super::CallStatus::Closed(_))
            || record.closed_at != Some(schedule.created_at())
            || !matches!(schedule.items(), [deadline] if deadline.kind == super::DeadlineKind::RecurrenceDelay && deadline.reference == reference)
        {
            return Err(Error::DeadlineMismatch);
        }
    }
    Ok(())
}

pub(crate) fn require_deadline_instants(
    schedule: &Deadlines,
    deadline_ids: &[(DeadlineKind, Id)],
) -> Result<(), Error> {
    for (kind, reference) in deadline_ids {
        if !schedule
            .items()
            .iter()
            .any(|deadline| deadline.kind == *kind && deadline.reference == *reference)
        {
            return Err(Error::SealedDeadlineWithoutInstant);
        }
    }
    Ok(())
}

pub(crate) fn validate_transaction(record: &super::TransactionRecord) -> Result<(), Error> {
    if let Some(schedule) = record.deadlines.as_deref() {
        if record
            .logical_time
            .is_some_and(|at| at != schedule.created_at())
        {
            return Err(Error::FiringTimeDisagrees);
        }
        require_deadline_instants(schedule, &record.deadline_ids)?;
    }
    let facts = record
        .loop_protocol()
        .map_or(&[][..], |packet| packet.facts.as_slice());
    let delayed = facts.iter().filter_map(|fact| match fact {
        super::loop_protocol::Fact::Advance {
            run,
            to_iteration,
            command,
            due_at: Some(due_at),
            ..
        } => Some((
            super::loop_protocol::admission_ref(*run, *to_iteration, *command),
            *due_at,
        )),
        _ => None,
    });
    for (reference, due_at) in delayed {
        if !record
            .deadline_ids
            .contains(&(super::DeadlineKind::RecurrenceDelay, reference))
            || !record.deadlines.as_deref().is_some_and(|schedule| {
                // A Loop admission reference already fixes the run and
                // iteration: unlike an idle lifetime, it cannot be renewed.
                schedule
                    .items()
                    .iter()
                    .filter(|deadline| {
                        deadline.kind == super::DeadlineKind::RecurrenceDelay
                            && deadline.reference == reference
                            && deadline.due_at == due_at
                    })
                    .count()
                    == 1
            })
        {
            return Err(Error::LoopDeadlineMismatch);
        }
    }
    if let Some(schedule) = record.deadlines.as_deref() {
        for deadline in schedule.items() {
            if deadline.kind == super::DeadlineKind::RecurrenceDelay && !facts.iter().any(|fact| {
                matches!(fact, super::loop_protocol::Fact::Advance {
                    run, to_iteration, command, due_at: Some(due_at), ..
                } if super::loop_protocol::admission_ref(*run, *to_iteration, *command) == deadline.reference
                    && *due_at == deadline.due_at)
            }) {
                return Err(Error::LoopDeadlineMismatch);
            }
        }
    }
    Ok(())
}
