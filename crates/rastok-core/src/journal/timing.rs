//! Retained scheduler evidence, deliberately outside the causal projection.
//! Arm order belongs to recovery: sorting equal-due arms by reference would
//! silently change which timeout wins after a restart.

use crate::canonical_cbor::{self, DecodeError, Value};
use crate::envelope::Id;
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

/// Extend-only record-retention key, not a canonical causal-record field.
pub const TIMER_SCHEDULE_KEY: u64 = 56;

/// One dated obligation, in the existing timer-kind/reference namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimerArm {
    /// Existing `journal::timer_kind` tag; no new scheduler protocol.
    pub kind: u8,
    /// Exact obligation, never a freshly derived restart reference.
    pub reference: Id,
    /// Absolute signed nanosecond instant on the run's logical timeline.
    pub due_at: i64,
}

/// An immutable schedule accepted with one journal record. The journal owns
/// it; the scheduler is only a disposable execution projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimerSchedule {
    armed_at: i64,
    arms: Vec<TimerArm>,
}

impl TimerSchedule {
    /// Validate before acceptance. An absent schedule is `None`, not an empty
    /// record field. Repeated references are ordered renewals, not a set.
    ///
    /// # Errors
    /// Empty schedules, unknown kinds or deadlines preceding their arm.
    pub fn try_new(armed_at: i64, arms: Vec<TimerArm>) -> Result<Self, DecodeError> {
        if arms.is_empty() {
            return Err(DecodeError::Malformed(
                "an empty timer schedule must be omitted",
            ));
        }
        for arm in &arms {
            if !(super::timer_kind::CALL_TIMEOUT..=super::timer_kind::SESSION_HANDSHAKE)
                .contains(&arm.kind)
            {
                return Err(DecodeError::Malformed(
                    "unknown timer kind in retained schedule",
                ));
            }
            if arm.due_at < armed_at {
                return Err(DecodeError::Malformed(
                    "a retained deadline precedes its arm",
                ));
            }
        }
        Ok(Self { armed_at, arms })
    }

    /// The explicit logical instant at acceptance, never OS or restart time.
    #[must_use]
    pub const fn armed_at(&self) -> i64 {
        self.armed_at
    }

    /// Actual arm order. Together with record order it breaks equal-date ties.
    #[must_use]
    pub fn arms(&self) -> &[TimerArm] {
        &self.arms
    }

    /// Canonical retained field `{56: [armed_at, [[kind, ref, due_at], ...]]}`.
    /// These bytes must be integrity-protected by storage, but must not be
    /// appended to the causal seal or mistaken for a complete journal backup.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        canonical_cbor::encode(&Value::Map(BTreeMap::from([(
            TIMER_SCHEDULE_KEY,
            Value::Array(vec![
                integer(self.armed_at),
                Value::Array(
                    self.arms
                        .iter()
                        .map(|arm| {
                            Value::Array(vec![
                                Value::Uint(u64::from(arm.kind)),
                                Value::Bytes(arm.reference.as_bytes().to_vec()),
                                integer(arm.due_at),
                            ])
                        })
                        .collect(),
                ),
            ]),
        )])))
    }

    /// Strict reader for the allocated retained field, independent of std.
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
        let Some(Value::Array(schedule)) = fields.remove(&TIMER_SCHEDULE_KEY) else {
            return Err(DecodeError::Malformed("missing timer schedule key 56"));
        };
        let [at, Value::Array(rows)] = schedule.as_slice() else {
            return Err(DecodeError::Malformed("a timer schedule has two fields"));
        };
        let armed_at = signed(at)?;
        let mut arms = Vec::with_capacity(rows.len());
        for row in rows {
            let Value::Array(row) = row else {
                return Err(DecodeError::Malformed("a timer arm must be an array"));
            };
            let [Value::Uint(kind), Value::Bytes(reference), due] = row.as_slice() else {
                return Err(DecodeError::Malformed(
                    "a timer arm has kind, reference and due instant",
                ));
            };
            arms.push(TimerArm {
                kind: u8::try_from(*kind)
                    .map_err(|_| DecodeError::Malformed("timer kind exceeds u8"))?,
                reference: Id::from_bytes(reference.as_slice().try_into().map_err(|_| {
                    DecodeError::Malformed("timer reference must have sixteen bytes")
                })?),
                due_at: signed(due)?,
            });
        }
        Self::try_new(armed_at, arms)
    }
}

pub(crate) fn require_timers(
    schedule: &TimerSchedule,
    timers: &[(u8, Id)],
) -> Result<(), DecodeError> {
    for (kind, reference) in timers {
        if !schedule
            .arms
            .iter()
            .any(|arm| arm.kind == *kind && arm.reference == *reference)
        {
            return Err(DecodeError::Malformed(
                "a sealed timer has no retained deadline",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_transaction(record: &super::TransactionRecord) -> Result<(), DecodeError> {
    if let Some(schedule) = record.timer_schedule.as_deref() {
        if record
            .logical_time
            .is_some_and(|at| at != schedule.armed_at())
        {
            return Err(DecodeError::Malformed(
                "observable firing time disagrees with its retained schedule",
            ));
        }
        require_timers(schedule, &record.timers)?;
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
            .timers
            .contains(&(super::timer_kind::LOOP_DELAY, reference))
            || !record.timer_schedule.as_deref().is_some_and(|schedule| {
                // A Loop admission reference already fixes the run and
                // iteration: unlike an idle lifetime, it cannot be renewed.
                schedule
                    .arms
                    .iter()
                    .filter(|arm| {
                        arm.kind == super::timer_kind::LOOP_DELAY
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
    if let Some(schedule) = record.timer_schedule.as_deref() {
        for arm in &schedule.arms {
            if arm.kind == super::timer_kind::LOOP_DELAY && !facts.iter().any(|fact| {
                matches!(fact, super::loop_protocol::Fact::Advance {
                    run, to_iteration, command, due_at: Some(due_at), ..
                } if super::loop_protocol::admission_ref(*run, *to_iteration, *command) == arm.reference
                    && *due_at == arm.due_at)
            }) {
                return Err(DecodeError::Malformed("a retained Loop deadline has no matching Advance"));
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

    fn arm(kind: u8, due_at: i64) -> TimerArm {
        TimerArm {
            kind,
            reference: Id::from_bytes([kind; 16]),
            due_at,
        }
    }

    #[test]
    fn retention_has_an_exact_key_and_signed_instant_encoding() {
        let schedule = TimerSchedule::try_new(-1, vec![arm(5, 100)]).unwrap();
        let mut expected = vec![0xa1, 0x18, 0x38, 0x82, 0x20, 0x81, 0x83, 0x05, 0x50];
        expected.extend_from_slice(&[5; 16]);
        expected.extend_from_slice(&[0x18, 0x64]);
        assert_eq!(schedule.encode(), expected);
        assert_eq!(TimerSchedule::decode(&expected).unwrap(), schedule);
    }

    #[test]
    fn every_registered_timer_retains_its_date_and_actual_arm_order() {
        let mut arms: Vec<_> = (1..=9).rev().map(|kind| arm(kind, i64::MAX)).collect();
        arms.push(arm(9, i64::MAX)); // a later renewal is not sorted or deduplicated
        let schedule = TimerSchedule::try_new(i64::MIN, arms.clone()).unwrap();
        let restored = TimerSchedule::decode(&schedule.encode()).unwrap();
        assert_eq!(restored.armed_at(), i64::MIN);
        assert_eq!(restored.arms(), arms);
        let reversed = TimerSchedule::try_new(i64::MIN, arms.into_iter().rev().collect()).unwrap();
        assert_ne!(schedule.encode(), reversed.encode());
    }

    #[test]
    fn no_empty_unknown_or_backdated_schedule_is_constructible() {
        assert!(TimerSchedule::try_new(0, vec![]).is_err());
        for kind in [0, 10, u8::MAX] {
            assert!(TimerSchedule::try_new(0, vec![arm(kind, 1)]).is_err());
        }
        assert!(TimerSchedule::try_new(0, vec![arm(1, -1)]).is_err());
        assert!(TimerSchedule::try_new(i64::MAX, vec![arm(1, i64::MAX)]).is_ok());
    }

    #[test]
    fn malformed_retention_is_refused_without_repair() {
        let valid = TimerSchedule::try_new(0, vec![arm(1, 1)]).unwrap().encode();
        let mut trailing = valid.clone();
        trailing.push(0);
        assert!(TimerSchedule::decode(&trailing).is_err());
        let mut nonminimal = valid.clone();
        nonminimal.splice(1..3, [0x19, 0, 56]);
        assert!(TimerSchedule::decode(&nonminimal).is_err());
        let mut wrong_key = valid;
        wrong_key[2] = 55;
        assert!(TimerSchedule::decode(&wrong_key).is_err());
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
                TIMER_SCHEDULE_KEY,
                Value::Array(vec![Value::Uint(0), Value::Array(vec![row])]),
            )])));
            assert!(TimerSchedule::decode(&bytes).is_err());
        }
    }
}
