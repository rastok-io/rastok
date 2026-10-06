//! Unsealed deadlines, deliberately outside the causal projection.
//! Arm order belongs to recovery: sorting equal-due arms by reference would
//! silently change which timeout wins after a restart.

use super::codec::{integer, signed};
use super::{DeadlineKind, deadline_kind_tag};
use crate::canonical_cbor::{self, Value};
use crate::envelope::Id;
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

/// Unsealed record key for the deadlines accepted with one commit.
pub const DEADLINES_KEY: u64 = 56;

/// Every winning outbound Call closure retains its original logical instant.
/// This field, like the due instant, is not part of the causal seal.
pub const CALL_CLOSED_AT_KEY: u64 = 59;

/// A deadline violates an acceptance invariant, independently of its encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// No obligations were supplied; the optional field must instead be absent.
    EmptySchedule,
    /// An obligation is already due before its recorded creation.
    DeadlinePrecedesCreation,
    /// A winning Call closure has no logical instant.
    MissingClosureInstant,
    /// An open Call incorrectly carries a closure instant.
    UnexpectedClosureInstant,
    /// A Call closes earlier than its original opening.
    ClosurePrecedesOpening,
    /// A Retry Call packet is missing its attempt context.
    MissingRetryContext,
    /// A Call carries facts or a nonpositive attempt ordinal.
    InvalidRetryContext,
    /// The next attempt ordinal cannot be represented.
    RetryOrdinalOverflow,
    /// A recurrence deadline is attached to a Call without Retry.
    ArmWithoutRetryContext,
    /// A Retry deadline does not identify this closure's next admission.
    ArmMismatch,
    /// A sealed obligation has no corresponding unsealed deadline.
    SealedDeadlineWithoutInstant,
    /// The observable transaction time disagrees with deadline creation.
    FiringTimeDisagrees,
    /// Loop Advance facts and recurrence deadlines do not agree exactly.
    LoopDeadlineMismatch,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::EmptySchedule => "empty deadlines must be omitted",
            Self::DeadlinePrecedesCreation => "a deadline precedes its creation",
            Self::MissingClosureInstant => "a winning Call closure requires its logical instant",
            Self::UnexpectedClosureInstant => "an open Call cannot carry a closure instant",
            Self::ClosurePrecedesOpening => "a Call closure precedes its opening",
            Self::MissingRetryContext => "a Retry Call requires an attempt context",
            Self::InvalidRetryContext => "a Call carries only a positive Retry context",
            Self::RetryOrdinalOverflow => "Retry admission ordinal overflows",
            Self::ArmWithoutRetryContext => "only a Retry closure can create a recurrence deadline",
            Self::ArmMismatch => "the Retry deadline must identify this closure's next admission",
            Self::SealedDeadlineWithoutInstant => "a sealed deadline id has no unsealed deadline",
            Self::FiringTimeDisagrees => "observable firing time disagrees with deadline creation",
            Self::LoopDeadlineMismatch => "Loop Advance and its exact recurrence deadline disagree",
        })
    }
}

impl core::error::Error for Error {}

/// Reading deadlines can fail at the byte boundary or at their typed invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// The canonical carrier is malformed.
    Read(canonical_cbor::DecodeError),
    /// The decoded instants violate an acceptance invariant.
    Invalid(Error),
}

impl From<canonical_cbor::DecodeError> for DecodeError {
    fn from(error: canonical_cbor::DecodeError) -> Self {
        Self::Read(error)
    }
}

impl From<Error> for DecodeError {
    fn from(error: Error) -> Self {
        Self::Invalid(error)
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(error) => error.fmt(f),
            Self::Invalid(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for DecodeError {}

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
    pub fn try_new(created_at: i64, items: Vec<Deadline>) -> Result<Self, Error> {
        if items.is_empty() {
            return Err(Error::EmptySchedule);
        }
        for arm in &items {
            if arm.due_at < created_at {
                return Err(Error::DeadlinePrecedesCreation);
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
            return Err(
                canonical_cbor::DecodeError::Malformed("trailing bytes after deadlines").into(),
            );
        }
        let Value::Map(mut fields) = value else {
            return Err(canonical_cbor::DecodeError::Malformed("deadlines must be a map").into());
        };
        if fields.len() != 1 {
            return Err(
                canonical_cbor::DecodeError::Malformed("deadlines require only key 56").into(),
            );
        }
        let Some(Value::Array(schedule)) = fields.remove(&DEADLINES_KEY) else {
            return Err(canonical_cbor::DecodeError::Malformed("missing deadlines key 56").into());
        };
        let [at, Value::Array(rows)] = schedule.as_slice() else {
            return Err(canonical_cbor::DecodeError::Malformed("deadlines have two fields").into());
        };
        let created_at = signed(at)?;
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let Value::Array(row) = row else {
                return Err(
                    canonical_cbor::DecodeError::Malformed("a deadline must be an array").into(),
                );
            };
            let [Value::Uint(kind), Value::Bytes(reference), due] = row.as_slice() else {
                return Err(canonical_cbor::DecodeError::Malformed(
                    "a deadline has kind, reference and due instant",
                )
                .into());
            };
            items.push(Deadline {
                kind: DeadlineKind::try_from(u8::try_from(*kind).map_err(|_| {
                    canonical_cbor::DecodeError::Malformed("deadline kind exceeds u8")
                })?)
                .map_err(|_| canonical_cbor::DecodeError::Malformed("unknown deadline kind"))?,
                reference: Id::from_bytes(reference.as_slice().try_into().map_err(|_| {
                    canonical_cbor::DecodeError::Malformed(
                        "deadline reference must have sixteen bytes",
                    )
                })?),
                due_at: signed(due)?,
            });
        }
        Self::try_new(created_at, items).map_err(DecodeError::Invalid)
    }
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
                        Err(DecodeError::Read(canonical_cbor::DecodeError::Malformed(
                            "unknown deadline kind"
                        )))
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
        assert_eq!(Deadlines::try_new(0, vec![]), Err(Error::EmptySchedule));
        for kind in [0, 4, 10, u8::MAX] {
            assert_eq!(
                DeadlineKind::try_from(kind),
                Err(super::super::UnknownDeadlineKind(kind))
            );
        }
        assert_eq!(
            Deadlines::try_new(0, vec![arm(1, -1)]),
            Err(Error::DeadlinePrecedesCreation)
        );
        assert!(Deadlines::try_new(i64::MAX, vec![arm(1, i64::MAX)]).is_ok());
    }

    #[test]
    fn decoding_distinguishes_carrier_errors_from_deadline_invariants() {
        for (rows, expected) in [
            (vec![], Error::EmptySchedule),
            (
                vec![Value::Array(vec![
                    Value::Uint(1),
                    Value::Bytes(vec![0; 16]),
                    Value::Negative(0),
                ])],
                Error::DeadlinePrecedesCreation,
            ),
        ] {
            let bytes = canonical_cbor::encode(&Value::Map(BTreeMap::from([(
                DEADLINES_KEY,
                Value::Array(vec![Value::Uint(0), Value::Array(rows)]),
            )])));
            assert_eq!(
                Deadlines::decode(&bytes),
                Err(DecodeError::Invalid(expected))
            );
        }
        assert!(matches!(Deadlines::decode(&[]), Err(DecodeError::Read(_))));
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
