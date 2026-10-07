use super::*;
use crate::journal::deadline::{Deadline, Deadlines};
use alloc::vec;

fn deadline(kind: u8, due_at: i64) -> Deadline {
    Deadline {
        kind: journal::DeadlineKind::try_from(kind).unwrap(),
        reference: Id::from_bytes([kind; 16]),
        due_at,
    }
}

#[test]
fn deadline_kinds_keep_their_explicit_bytes_and_reject_unknown_wire_tags() {
    use journal::DeadlineKind;
    let registry = [
        (1, DeadlineKind::CallTimeout),
        (2, DeadlineKind::JoinWithin),
        (3, DeadlineKind::InstanceLifetime),
        (4, DeadlineKind::RecurrenceDelay),
        (5, DeadlineKind::InstanceDrain),
        (6, DeadlineKind::InstanceSettlement),
        (7, DeadlineKind::InboundCallTimeout),
        (8, DeadlineKind::SessionHandshake),
    ];
    for byte in 0..=u8::MAX {
        let tree = list([
            Cbor::Uint(0),
            list([list([
                Cbor::Uint(u64::from(byte)),
                id(Id::from_bytes([0; 16])),
                Cbor::Uint(1),
            ])]),
        ]);
        match registry.iter().find(|(tag, _)| *tag == byte) {
            Some((_, kind)) => {
                assert_eq!(DeadlineKind::try_from(byte), Ok(*kind));
                assert_eq!(u8::from(*kind), byte);
                let decoded = read_schedule(&tree).unwrap().unwrap();
                assert_eq!(decoded.items()[0].kind, *kind);
                assert_eq!(schedule(Some(&decoded)), tree);
            }
            None => {
                assert_eq!(
                    DeadlineKind::try_from(byte),
                    Err(journal::UnknownDeadlineKind(byte))
                );
                assert_eq!(
                    read_schedule(&tree),
                    Err(Error::Invalid("unknown deadline kind"))
                );
            }
        }
    }
}

#[test]
fn deadlines_have_an_exact_inline_signed_instant_encoding() {
    let deadlines = Deadlines::try_new(-1, vec![deadline(5, 100)]).unwrap();
    let mut expected = vec![0x82, 0x20, 0x81, 0x83, 0x05, 0x50];
    expected.extend_from_slice(&[5; 16]);
    expected.extend_from_slice(&[0x18, 0x64]);
    assert_eq!(
        crate::canonical_cbor::encode(&schedule(Some(&deadlines))),
        expected
    );
    let (tree, used) = crate::canonical_cbor::decode(&expected).unwrap();
    assert_eq!(used, expected.len());
    assert_eq!(read_schedule(&tree).unwrap().as_deref(), Some(&deadlines));
    assert_eq!(schedule(None), Cbor::Null);
    assert_eq!(read_schedule(&Cbor::Null), Ok(None));
}

#[test]
fn every_registered_deadline_keeps_its_due_time_and_creation_order() {
    let mut items: Vec<_> = [8, 7, 6, 5, 4, 3, 2, 1]
        .into_iter()
        .map(|kind| deadline(kind, i64::MAX))
        .collect();
    items.push(deadline(8, i64::MAX)); // a renewal is not sorted or deduplicated
    let deadlines = Deadlines::try_new(i64::MIN, items.clone()).unwrap();
    let tree = schedule(Some(&deadlines));
    let restored = read_schedule(&tree).unwrap().unwrap();
    assert_eq!(restored.created_at(), i64::MIN);
    assert_eq!(restored.items(), items);
    let reversed = Deadlines::try_new(i64::MIN, items.into_iter().rev().collect()).unwrap();
    assert_ne!(tree, schedule(Some(&reversed)));
}

#[test]
fn no_empty_unknown_or_backdated_schedule_is_constructible() {
    use journal::deadline::Error as DeadlineError;
    assert_eq!(
        Deadlines::try_new(0, vec![]),
        Err(DeadlineError::EmptySchedule)
    );
    for kind in [0, 9, 10, u8::MAX] {
        assert_eq!(
            journal::DeadlineKind::try_from(kind),
            Err(journal::UnknownDeadlineKind(kind))
        );
    }
    assert_eq!(
        Deadlines::try_new(0, vec![deadline(1, -1)]),
        Err(DeadlineError::DeadlinePrecedesCreation)
    );
    assert!(Deadlines::try_new(i64::MAX, vec![deadline(1, i64::MAX)]).is_ok());
}

#[test]
fn decoding_distinguishes_carrier_errors_from_deadline_invariants() {
    use journal::deadline::Error as DeadlineError;
    for (rows, expected) in [
        (vec![], DeadlineError::EmptySchedule),
        (
            vec![list([
                Cbor::Uint(1),
                id(Id::from_bytes([0; 16])),
                Cbor::Negative(0),
            ])],
            DeadlineError::DeadlinePrecedesCreation,
        ),
    ] {
        assert_eq!(
            read_schedule(&list([Cbor::Uint(0), list(rows)])),
            Err(Error::Deadlines(expected))
        );
    }
    assert_eq!(
        read_schedule(&Cbor::Bool(true)),
        Err(Error::Invalid("array"))
    );
}

#[test]
fn malformed_deadlines_are_refused_without_repair() {
    let valid = schedule(Some(&Deadlines::try_new(0, vec![deadline(1, 1)]).unwrap()));
    // No byte-string or map wrapper is a second spelling for this field.
    assert!(read_schedule(&Cbor::Bytes(crate::canonical_cbor::encode(&valid))).is_err());
    assert!(read_schedule(&Cbor::Map(BTreeMap::from([(41, valid.clone())]))).is_err());
    for row in [
        list([Cbor::Uint(1), Cbor::Bytes(vec![0; 15]), Cbor::Uint(1)]),
        list([
            Cbor::Uint(1),
            id(Id::from_bytes([0; 16])),
            Cbor::Uint(u64::MAX),
        ]),
        list([
            Cbor::Uint(1),
            id(Id::from_bytes([0; 16])),
            Cbor::Negative(u64::MAX),
        ]),
        list([Cbor::Uint(1), id(Id::from_bytes([0; 16])), Cbor::Bool(true)]),
        list([Cbor::Uint(256), id(Id::from_bytes([0; 16])), Cbor::Uint(1)]),
        list([Cbor::Uint(1), id(Id::from_bytes([0; 16]))]),
    ] {
        assert!(read_schedule(&list([Cbor::Uint(0), list([row])])).is_err());
    }
    for created_at in [
        Cbor::Uint(u64::MAX),
        Cbor::Negative(u64::MAX),
        Cbor::Bool(true),
    ] {
        assert!(
            read_schedule(&list([
                created_at,
                list([list([
                    Cbor::Uint(1),
                    id(Id::from_bytes([0; 16])),
                    Cbor::Uint(1)
                ])])
            ]))
            .is_err()
        );
    }
    let nonminimal = [0x82, 0x18, 0x00, 0x80];
    assert!(crate::canonical_cbor::decode(&nonminimal).is_err());
}

#[test]
fn deadlines_are_inline_values_not_nested_cbor_documents() {
    let deadlines = Deadlines::try_new(
        -1,
        vec![Deadline {
            kind: journal::DeadlineKind::CallTimeout,
            reference: Id::from_bytes([5; 16]),
            due_at: 100,
        }],
    )
    .unwrap();
    assert_eq!(
        schedule(Some(&deadlines)),
        list([
            Cbor::Negative(0),
            list([list([
                Cbor::Uint(1),
                id(Id::from_bytes([5; 16])),
                Cbor::Uint(100)
            ])]),
        ]),
    );
}
