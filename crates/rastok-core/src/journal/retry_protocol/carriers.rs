//! The key-58 packet has a specific carrier, not an ignorable annotation.
//! This checks shape and disposition without a plan; the operation fold and
//! spatial recovery separately bind it to the original Contract/admission.

use super::*;
use crate::journal::TransactionRecord;
use crate::node::{Lifecycle, Trigger};
use alloc::collections::BTreeSet;

pub(crate) fn validate_transaction(record: &TransactionRecord) -> Result<(), Error> {
    let manager = record.trigger == Trigger::Lifecycle(Lifecycle::Retry);
    let Some(packet) = record.retry_record.as_deref() else {
        return if manager {
            Err(Error::PacketShape)
        } else {
            Ok(())
        };
    };
    let Some(first) = packet.facts.first() else {
        return Err(Error::PacketShape);
    };
    if packet
        .facts
        .iter()
        .any(|fact| fact.operation() != first.operation())
    {
        return Err(Error::PacketShape);
    }
    match (packet.context, packet.facts.as_slice()) {
        (Some(context), [Fact::Held(batch)]) if context == batch.attempt => {
            if !record.routes.is_empty()
                || record.instance_commands.is_some()
                || record.loop_protocol().is_some_and(|loop_packet| {
                    !loop_packet.facts.is_empty()
                        || !loop_packet.produced.is_empty()
                        || !loop_packet.consumed.is_empty()
                })
            {
                return Err(Error::Batch);
            }
            producer(record, batch)
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
        ) if context == batch.attempt && context == *attempt => producer(record, batch),
        (None, [Fact::Opened { .. }])
        | (None, [Fact::Opened { .. }, Fact::Admitted { .. }])
        | (None, [Fact::Admitted { .. }])
        | (
            None,
            [
                Fact::Close {
                    reason: CloseReason::Stopped,
                    ..
                },
            ],
        ) => manager_carrier(record),
        (None, [Fact::Consumed(batch), Fact::Advance { from, to, call }])
            if batch.attempt == *from
                && from.ordinal > 0
                && from.ordinal.checked_add(1) == Some(*to)
                && batch.call == *call =>
        {
            manager_carrier(record)
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
        ) if batch.attempt == *attempt && attempt.ordinal > 0 => manager_carrier(record),
        _ => Err(Error::PacketShape),
    }
}

fn manager_carrier(record: &TransactionRecord) -> Result<(), Error> {
    if record.trigger != Trigger::Lifecycle(Lifecycle::Retry)
        || record.logical_time.is_none()
        || record.cause.is_none()
        || !record.consumed.is_empty()
        || !record.emissions.is_empty()
        || !record.state_delta.is_empty()
        || !record.join_inputs.is_empty()
        || record.parent_instance.is_some()
        || record.close_scope.is_some()
        || record.exchange_exit.is_some()
        || record.instance_commands.is_some()
        || record.loop_protocol().is_some_and(|packet| {
            !packet.facts.is_empty() || !packet.produced.is_empty() || !packet.consumed.is_empty()
        })
    {
        return Err(Error::PacketShape);
    }
    if !record.routes.is_empty()
        && !record.retry_record.as_deref().is_some_and(|packet| {
            packet
                .facts
                .iter()
                .any(|fact| matches!(fact, Fact::Admitted { .. }))
        })
    {
        return Err(Error::PacketShape);
    }
    Ok(())
}

fn producer(record: &TransactionRecord, batch: &Batch) -> Result<(), Error> {
    if !matches!(
        record.trigger,
        Trigger::Port(_) | Trigger::Lifecycle(Lifecycle::Timeout | Lifecycle::Recover)
    ) || record.consumed.len() != 1
        || record.cause.is_some()
        || record.parent_instance.is_some()
        || record.close_scope.is_some()
        || record.exchange_exit.is_some()
        || !record.state_delta.is_empty()
        || !record.join_inputs.is_empty()
        || batch.attempt.ordinal == 0
        || batch.emissions.is_empty()
        || batch.emissions.len() != record.emissions.len()
        || batch.emissions.iter().collect::<BTreeSet<_>>().len() != batch.emissions.len()
        || !batch
            .emissions
            .iter()
            .zip(&record.emissions)
            .all(|(id, emission)| *id == emission.envelope.meta().sys().id())
    {
        return Err(Error::Batch);
    }
    Ok(())
}
