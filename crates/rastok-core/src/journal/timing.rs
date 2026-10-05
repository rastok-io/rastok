//! Unsealed deadlines, deliberately outside the causal projection.
//! Arm order belongs to recovery: sorting equal-due arms by reference would
//! silently change which timeout wins after a restart.

use super::{DeadlineKind, deadline_kind_tag};
use crate::canonical_cbor::{self, DecodeError, Value};
use crate::envelope::Id;
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

/// Unsealed record key for the deadlines accepted with one commit.
pub const DEADLINES_KEY: u64 = 56;

/// Every winning outbound Call closure retains its original logical instant.
/// This field, like the due instant, is not part of the causal seal.
pub const CALL_CLOSED_AT_KEY: u64 = 59;

pub(crate) fn validate_call(record: &super::CallRecord) -> Result<(), DecodeError> {
    let context = if let Some(packet) = record.retry_record.as_deref() {
        let Some(context) = packet.context else {
            return Err(DecodeError::Malformed(
                "a Retry Call requires an attempt context",
            ));
        };
        if context.ordinal == 0 || !packet.facts.is_empty() {
            return Err(DecodeError::Malformed(
                "a Call carries only a positive Retry context",
            ));
        }
        Some(context)
    } else {
        None
    };
    match (&record.status, record.closed_at) {
        (super::CallStatus::Open, None) => {}
        (super::CallStatus::Closed(_) | super::CallStatus::Abandoned, Some(at))
            if at >= record.intent.logical_time => {}
        _ => {
            return Err(DecodeError::Malformed(
                "a winning Call closure requires its original instant, no earlier than its opening",
            ));
        }
    }
    if let Some(schedule) = record.deadlines.as_deref() {
        let Some(context) = context else {
            return Err(DecodeError::Malformed(
                "only a Retry closure can arm a recurrence",
            ));
        };
        let Some(next) = context.ordinal.checked_add(1) else {
            return Err(DecodeError::Malformed("Retry admission ordinal overflows"));
        };
        let reference =
            super::retry_protocol::admission_ref(context.operation, next, record.intent.call_id);
        if !matches!(record.status, super::CallStatus::Closed(_))
            || record.closed_at != Some(schedule.created_at())
            || !matches!(schedule.items(), [arm] if arm.kind == super::DeadlineKind::RecurrenceDelay && arm.reference == reference)
        {
            return Err(DecodeError::Malformed(
                "the Retry arm must belong to this winning closure and next admission",
            ));
        }
    }
    Ok(())
}

/// One dated obligation, identified by its kind and exact subject id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deadline {
    /// The journal's typed kind of dated obligation.
    pub kind: DeadlineKind,
    /// Exact obligation, never a freshly derived restart reference.
    pub reference: Id,
    /// Absolute signed nanosecond instant on the run's logical timeline.
    pub due_at: i64,
}

/// An immutable schedule accepted with one journal record. The journal owns
/// it; the scheduler is only a disposable execution projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deadlines {
    /// Creation instant in signed logical nanoseconds, not wall-clock time.
    created_at: i64,
    items: Vec<Deadline>,
}

impl Deadlines {
    /// Validate before acceptance. An absent schedule is `None`, not an empty
    /// record field. Repeated references are ordered renewals, not a set.
    ///
    /// # Errors
    /// Empty schedules or deadlines preceding their creation instant.
    pub fn try_new(created_at: i64, items: Vec<Deadline>) -> Result<Self, DecodeError> {
        if items.is_empty() {
            return Err(DecodeError::Malformed(
                "an empty timer schedule must be omitted",
            ));
        }
        for arm in &items {
            if arm.due_at < created_at {
                return Err(DecodeError::Malformed(
                    "an unsealed deadline precedes its creation",
                ));
            }
        }
        Ok(Self { created_at, items })
    }

    /// The explicit logical instant at acceptance, never OS or restart time.
    #[must_use]
    pub const fn created_at(&self) -> i64 {
        self.created_at
    }

    /// Actual arm order. Together with record order it breaks equal-date ties.
    #[must_use]
    pub fn items(&self) -> &[Deadline] {
        &self.items
    }

    /// Canonical unsealed field `{56: [created_at, [[kind, ref, due_at], ...]]}`.
    /// These bytes must be integrity-protected by storage, but must not be
    /// appended to the causal seal or mistaken for a complete journal backup.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        canonical_cbor::encode(&Value::Map(BTreeMap::from([(
            DEADLINES_KEY,
            Value::Array(vec![
                integer(self.created_at),
                Value::Array(
                    self.items
                        .iter()
                        .map(|arm| {
                            Value::Array(vec![
                                Value::Uint(u64::from(deadline_kind_tag(arm.kind))),
                                Value::Bytes(arm.reference.as_bytes().to_vec()),
                                integer(arm.due_at),
                            ])
                        })
                        .collect(),
                ),
            ]),
        )])))
    }

    /// Strict reader for the allocated unsealed field, independent of std.
    ///
    /// # Errors
    /// Noncanonical CBOR, trailing bytes, wrong fields/arity, invalid integer
    /// or reference widths, and the same schedule violations as `try_new`.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let (value, consumed) = canonical_cbor::decode(bytes)?;
        if consumed != bytes.len() {
            return Err(DecodeError::Malformed(
                "trailing bytes after timer retention",
            ));
        }
        let Value::Map(mut fields) = value else {
            return Err(DecodeError::Malformed("timer retention must be a map"));
        };
        if fields.len() != 1 {
            return Err(DecodeError::Malformed(
                "timer retention requires only key 56",
            ));
        }
        let Some(Value::Array(schedule)) = fields.remove(&DEADLINES_KEY) else {
            return Err(DecodeError::Malformed("missing timer schedule key 56"));
        };
        let [at, Value::Array(rows)] = schedule.as_slice() else {
            return Err(DecodeError::Malformed("a timer schedule has two fields"));
        };
        let created_at = signed(at)?;
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let Value::Array(row) = row else {
                return Err(DecodeError::Malformed("a timer arm must be an array"));
            };
            let [Value::Uint(kind), Value::Bytes(reference), due] = row.as_slice() else {
                return Err(DecodeError::Malformed(
                    "a timer arm has kind, reference and due instant",
                ));
            };
            items.push(Deadline {
                kind: DeadlineKind::try_from(
                    u8::try_from(*kind)
                        .map_err(|_| DecodeError::Malformed("deadline kind exceeds u8"))?,
                )
                .map_err(|_| DecodeError::Malformed("unknown deadline kind"))?,
                reference: Id::from_bytes(reference.as_slice().try_into().map_err(|_| {
                    DecodeError::Malformed("timer reference must have sixteen bytes")
                })?),
                due_at: signed(due)?,
            });
        }
        Self::try_new(created_at, items)
    }
}

pub(crate) fn require_deadline_instants(
    schedule: &Deadlines,
    timers: &[(DeadlineKind, Id)],
) -> Result<(), DecodeError> {
    for (kind, reference) in timers {
        if !schedule
            .items
            .iter()
            .any(|arm| arm.kind == *kind && arm.reference == *reference)
        {
            return Err(DecodeError::Malformed(
                "a sealed deadline id has no unsealed deadline",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_transaction(record: &super::TransactionRecord) -> Result<(), DecodeError> {
    if let Some(schedule) = record.deadlines.as_deref() {
        if record
            .logical_time
            .is_some_and(|at| at != schedule.created_at())
        {
            return Err(DecodeError::Malformed(
                "observable firing time disagrees with its unsealed schedule",
            ));
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
                    .items
                    .iter()
                    .filter(|arm| {
                        arm.kind == super::DeadlineKind::RecurrenceDelay
                            && arm.reference == reference
                            && arm.due_at == due_at
                    })
                    .count()
                    == 1
            })
        {
            return Err(DecodeError::Malformed(
                "a delayed Loop must retain its exact accepted deadline",
            ));
        }
    }
    if let Some(schedule) = record.deadlines.as_deref() {
        for arm in &schedule.items {
            if arm.kind == super::DeadlineKind::RecurrenceDelay && !facts.iter().any(|fact| {
                matches!(fact, super::loop_protocol::Fact::Advance {
                    run, to_iteration, command, due_at: Some(due_at), ..
                } if super::loop_protocol::admission_ref(*run, *to_iteration, *command) == arm.reference
                    && *due_at == arm.due_at)
            }) {
                return Err(DecodeError::Malformed("an unsealed Loop deadline has no matching Advance"));
            }
        }
    }
    Ok(())
}

fn integer(value: i64) -> Value {
    if value < 0 {
        Value::Negative(value.unsigned_abs() - 1)
    } else {
        Value::Uint(value as u64)
    }
}

fn signed(value: &Value) -> Result<i64, DecodeError> {
    match value {
        Value::Uint(value) => i64::try_from(*value),
        Value::Negative(value) => i64::try_from(*value).map(|value| -1 - value),
        _ => {
            return Err(DecodeError::Malformed(
                "timer instants must be signed integers",
            ));
        }
    }
    .map_err(|_| DecodeError::Malformed("timer instant exceeds signed nanoseconds"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadline_kinds_keep_their_explicit_bytes_and_reject_unknown_wire_tags() {
        let registry = [
            (1, DeadlineKind::CallTimeout),
            (2, DeadlineKind::JoinWithin),
            (3, DeadlineKind::InstanceLifetime),
            (5, DeadlineKind::RecurrenceDelay),
            (6, DeadlineKind::InstanceDrain),
            (7, DeadlineKind::InstanceSettlement),
            (8, DeadlineKind::InboundCallTimeout),
            (9, DeadlineKind::SessionHandshake),
        ];
        for byte in 0..=u8::MAX {
            let bytes = canonical_cbor::encode(&Value::Map(BTreeMap::from([(
                DEADLINES_KEY,
                Value::Array(vec![
                    Value::Uint(0),
                    Value::Array(vec![Value::Array(vec![
                        Value::Uint(u64::from(byte)),
                        Value::Bytes(vec![0; 16]),
                        Value::Uint(1),
                    ])]),
                ]),
            )])));
            match registry.iter().find(|(tag, _)| *tag == byte) {
                Some((_, kind)) => {
                    assert_eq!(DeadlineKind::try_from(byte), Ok(*kind));
                    assert_eq!(deadline_kind_tag(*kind), byte);
                    let decoded = Deadlines::decode(&bytes).unwrap();
                    assert_eq!(decoded.items()[0].kind, *kind);
                    assert_eq!(decoded.encode(), bytes);
                }
                None => {
                    assert_eq!(
                        DeadlineKind::try_from(byte),
                        Err(super::super::UnknownDeadlineKind(byte))
                    );
                    assert!(matches!(
                        Deadlines::decode(&bytes),
                        Err(DecodeError::Malformed("unknown deadline kind"))
                    ));
                }
            }
        }
    }

    fn arm(kind: u8, due_at: i64) -> Deadline {
        Deadline {
            kind: DeadlineKind::try_from(kind).unwrap(),
            reference: Id::from_bytes([kind; 16]),
            due_at,
        }
    }

    #[test]
    fn deadlines_have_an_exact_key_and_signed_instant_encoding() {
        let schedule = Deadlines::try_new(-1, vec![arm(5, 100)]).unwrap();
        let mut expected = vec![0xa1, 0x18, 0x38, 0x82, 0x20, 0x81, 0x83, 0x05, 0x50];
        expected.extend_from_slice(&[5; 16]);
        expected.extend_from_slice(&[0x18, 0x64]);
        assert_eq!(schedule.encode(), expected);
        assert_eq!(Deadlines::decode(&expected).unwrap(), schedule);
    }

    #[test]
    fn every_registered_deadline_keeps_its_due_time_and_creation_order() {
        let mut items: Vec<_> = [9, 8, 7, 6, 5, 3, 2, 1]
            .into_iter()
            .map(|kind| arm(kind, i64::MAX))
            .collect();
        items.push(arm(9, i64::MAX)); // a later renewal is not sorted or deduplicated
        let schedule = Deadlines::try_new(i64::MIN, items.clone()).unwrap();
        let restored = Deadlines::decode(&schedule.encode()).unwrap();
        assert_eq!(restored.created_at(), i64::MIN);
        assert_eq!(restored.items(), items);
        let reversed = Deadlines::try_new(i64::MIN, items.into_iter().rev().collect()).unwrap();
        assert_ne!(schedule.encode(), reversed.encode());
    }

    #[test]
    fn no_empty_unknown_or_backdated_schedule_is_constructible() {
        assert!(Deadlines::try_new(0, vec![]).is_err());
        for kind in [0, 4, 10, u8::MAX] {
            assert_eq!(
                DeadlineKind::try_from(kind),
                Err(super::super::UnknownDeadlineKind(kind))
            );
        }
        assert!(Deadlines::try_new(0, vec![arm(1, -1)]).is_err());
        assert!(Deadlines::try_new(i64::MAX, vec![arm(1, i64::MAX)]).is_ok());
    }

    #[test]
    fn malformed_deadlines_are_refused_without_repair() {
        let valid = Deadlines::try_new(0, vec![arm(1, 1)]).unwrap().encode();
        let mut trailing = valid.clone();
        trailing.push(0);
        assert!(Deadlines::decode(&trailing).is_err());
        let mut nonminimal = valid.clone();
        nonminimal.splice(1..3, [0x19, 0, 56]);
        assert!(Deadlines::decode(&nonminimal).is_err());
        let mut wrong_key = valid;
        wrong_key[2] = 55;
        assert!(Deadlines::decode(&wrong_key).is_err());
        for row in [
            Value::Array(vec![
                Value::Uint(1),
                Value::Bytes(vec![0; 15]),
                Value::Uint(1),
            ]),
            Value::Array(vec![
                Value::Uint(1),
                Value::Bytes(vec![0; 16]),
                Value::Uint(u64::MAX),
            ]),
            Value::Array(vec![
                Value::Uint(1),
                Value::Bytes(vec![0; 16]),
                Value::Negative(u64::MAX),
            ]),
            Value::Array(vec![
                Value::Uint(1),
                Value::Bytes(vec![0; 16]),
                Value::Bool(true),
            ]),
            Value::Array(vec![
                Value::Uint(256),
                Value::Bytes(vec![0; 16]),
                Value::Uint(1),
            ]),
            Value::Array(vec![Value::Uint(1), Value::Bytes(vec![0; 16])]),
        ] {
            let bytes = canonical_cbor::encode(&Value::Map(BTreeMap::from([(
                DEADLINES_KEY,
                Value::Array(vec![Value::Uint(0), Value::Array(vec![row])]),
            )])));
            assert!(Deadlines::decode(&bytes).is_err());
        }
    }
}
