use super::*;
use crate::connector::ContractId;
use crate::journal::retry_protocol::{
    self as rp, AttemptId, Batch, CloseReason, Fact, Packet, Pin,
};
use crate::plan::SemanticHash;
use alloc::collections::BTreeSet;

/// Lossless key 58 payload. An empty packet must be omitted by its carrier.
#[must_use]
pub fn retry_packet(packet: &Packet) -> Cbor {
    rp::encoding::packet_tree(packet, retained_pin, envelope)
}

fn retained_pin(pin: &Pin) -> Cbor {
    list([
        id(pin.envelope),
        Cbor::Bytes(pin.prepared.bytes().to_vec()),
        list(
            pin.settings
                .iter()
                .map(|(name, value)| list([word(name), canonical_value(value)])),
        ),
        optional(pin.key.as_ref(), |value| word(value)),
        integer(pin.opened_at),
    ])
}

fn attempt(operation: &Cbor, ordinal: &Cbor, zero: bool) -> Result<AttemptId, Error> {
    let ordinal = uint(ordinal)?;
    if ordinal == 0 && !zero {
        return Err(Error::Invalid("Retry attempt must be positive"));
    }
    Ok(AttemptId {
        operation: rp::OperationId::from_id(read_id(operation)?),
        ordinal,
    })
}

fn batch(operation: &Cbor, ordinal: &Cbor, call: &Cbor, emissions: &Cbor) -> Result<Batch, Error> {
    let emissions = items(emissions)?
        .iter()
        .map(read_id)
        .collect::<Result<Vec<_>, _>>()?;
    if emissions.is_empty() || emissions.iter().collect::<BTreeSet<_>>().len() != emissions.len() {
        return Err(Error::Invalid("Retry batch must be nonempty and unique"));
    }
    Ok(Batch {
        attempt: attempt(operation, ordinal, false)?,
        call: CallId::new(read_id(call)?),
        emissions,
    })
}

impl Reader {
    /// Decode a sparse Retry packet using the same envelope prefix as records.
    ///
    /// # Errors
    /// Empty/unknown fields, malformed tags and references, noncanonical pins,
    /// or missing parents. The protocol fold still validates carriers and state.
    pub fn retry_packet(&mut self, value: &Cbor) -> Result<Packet, Error> {
        let Cbor::Map(fields) = value else {
            return Err(Error::Invalid("Retry packet map"));
        };
        if fields.is_empty() || fields.keys().any(|key| !matches!(key, 1 | 2)) {
            return Err(Error::Invalid("Retry packet fields"));
        }
        let context = fields
            .get(&1)
            .map(|value| {
                let [operation, ordinal] = array(value)?;
                attempt(operation, ordinal, false)
            })
            .transpose()?;
        let facts = if let Some(value) = fields.get(&2) {
            let values = items(value)?;
            if values.is_empty() {
                return Err(Error::Invalid("empty Retry facts must be omitted"));
            }
            values
                .iter()
                .map(|value| self.retry_fact(value))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        Ok(Packet { context, facts })
    }

    fn retry_fact(&mut self, value: &Cbor) -> Result<Fact, Error> {
        match items(value)?.first().map(uint).transpose()? {
            Some(1) => {
                let [_, operation, opening, entry, plan, contract, pin] = array(value)?;
                let [source, prepared, settings, key, at] = array(pin)?;
                let envelope = read_id(source)?;
                if !self.envelopes.contains_key(&envelope) {
                    return Err(Error::Invalid(
                        "Retry pin envelope is not in retained prefix",
                    ));
                }
                let mut resolved = BTreeMap::new();
                let mut previous = None;
                for setting in items(settings)? {
                    let [name, value] = array(setting)?;
                    let name = text(name)?;
                    if previous.is_some_and(|previous| previous >= name) {
                        return Err(Error::Invalid("Retry pin settings are not canonical"));
                    }
                    previous = Some(name);
                    resolved.insert(
                        name.to_owned(),
                        decode_canonical_value(value).map_err(Error::Read)?,
                    );
                }
                Ok(Fact::Opened {
                    operation: rp::OperationId::from_id(read_id(operation)?),
                    opening: read_id(opening)?,
                    entry: read_port(entry)?,
                    plan: SemanticHash::from_bytes(
                        bytes(plan)?
                            .try_into()
                            .map_err(|_| Error::Invalid("Retry original plan hash"))?,
                    ),
                    contract: ContractId::parse(text(contract)?)
                        .map_err(|_| Error::Invalid("Retry ContractId"))?,
                    pin: Pin {
                        envelope,
                        prepared: PreparedRequest::seal(bytes(prepared)?.to_vec()),
                        settings: resolved,
                        key: read_optional(key, |value| Ok(text(value)?.to_owned()))?,
                        opened_at: signed(at)?,
                    },
                })
            }
            Some(2) => {
                let [_, operation, ordinal, envelope, delivery] = array(value)?;
                Ok(Fact::Admitted {
                    attempt: attempt(operation, ordinal, false)?,
                    envelope: self.envelope(envelope)?,
                    delivery: read_id(delivery)?,
                })
            }
            Some(tag @ (3 | 4 | 6)) => {
                let [_, operation, ordinal, call, emissions] = array(value)?;
                let batch = batch(operation, ordinal, call, emissions)?;
                Ok(match tag {
                    3 => Fact::Held(batch),
                    4 => Fact::Consumed(batch),
                    _ => Fact::Published(batch),
                })
            }
            Some(5) => {
                let [_, operation, ordinal, next, call] = array(value)?;
                let from = attempt(operation, ordinal, false)?;
                let to = uint(next)?;
                if from.ordinal.checked_add(1) != Some(to) {
                    return Err(Error::Invalid(
                        "Retry Advance must name the following attempt",
                    ));
                }
                Ok(Fact::Advance {
                    from,
                    to,
                    call: CallId::new(read_id(call)?),
                })
            }
            Some(7) => {
                let [_, operation, ordinal, reason] = array(value)?;
                let reason = match uint(reason)? {
                    1 => CloseReason::Final,
                    2 => CloseReason::Stopped,
                    _ => return Err(Error::Invalid("Retry close reason")),
                };
                Ok(Fact::Close {
                    attempt: attempt(operation, ordinal, reason == CloseReason::Stopped)?,
                    reason,
                })
            }
            _ => Err(Error::Invalid("Retry fact tag")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical_cbor::{decode, encode};
    use alloc::vec;

    fn fixture() -> (Reader, Envelope, rp::OperationId, Packet) {
        let source = Envelope::new(
            Meta::new(MetaSys::root(Id::from_bytes([1; 16]), 1)),
            Data::materialized(Value::Text("request".into())),
        );
        let mut reader = Reader::default();
        reader.envelope(&envelope(&source)).unwrap();
        let operation = rp::OperationId::from_id(Id::from_bytes([2; 16]));
        let packet = Packet {
            context: None,
            facts: vec![Fact::Opened {
                operation,
                opening: Id::from_bytes([3; 16]),
                entry: Port::new("in").unwrap(),
                plan: SemanticHash::from_bytes([4; 32]),
                contract: ContractId::parse(crate::connector::PROCESS_CONNECTOR_ID).unwrap(),
                pin: Pin {
                    envelope: source.meta().sys().id(),
                    prepared: PreparedRequest::seal(vec![0, 1, 255]),
                    settings: BTreeMap::from([
                        ("command".into(), Value::Text("cat".into())),
                        ("timeout".into(), Value::Duration(100)),
                    ]),
                    key: Some("stable-key".into()),
                    opened_at: i64::MIN,
                },
            }],
        };
        (reader, source, operation, packet)
    }

    #[test]
    fn every_retry_tag_round_trips_without_losing_the_pin_or_emission_order() {
        let (mut reader, source, operation, opened) = fixture();
        let attempt = AttemptId {
            operation,
            ordinal: 1,
        };
        let batch = Batch {
            attempt,
            call: CallId::new(Id::from_bytes([5; 16])),
            emissions: vec![Id::from_bytes([9; 16]), Id::from_bytes([7; 16])],
        };
        let manager = |facts| Packet {
            context: None,
            facts,
        };
        let packets = [
            opened,
            manager(vec![Fact::Admitted {
                attempt,
                envelope: source,
                delivery: Id::from_bytes([3; 16]),
            }]),
            Packet {
                context: Some(attempt),
                facts: vec![],
            },
            Packet {
                context: Some(attempt),
                facts: vec![Fact::Held(batch.clone())],
            },
            manager(vec![
                Fact::Consumed(batch.clone()),
                Fact::Advance {
                    from: attempt,
                    to: 2,
                    call: batch.call,
                },
            ]),
            Packet {
                context: Some(attempt),
                facts: vec![
                    Fact::Published(batch.clone()),
                    Fact::Close {
                        attempt,
                        reason: CloseReason::Final,
                    },
                ],
            },
            manager(vec![
                Fact::Consumed(batch),
                Fact::Close {
                    attempt,
                    reason: CloseReason::Stopped,
                },
            ]),
            manager(vec![Fact::Close {
                attempt: AttemptId {
                    operation,
                    ordinal: 0,
                },
                reason: CloseReason::Stopped,
            }]),
        ];
        for packet in packets {
            let encoded = encode(&retry_packet(&packet));
            let (tree, read) = decode(&encoded).unwrap();
            assert_eq!(read, encoded.len());
            let decoded = reader.retry_packet(&tree).unwrap();
            assert_eq!(decoded, packet);
            assert_eq!(encode(&retry_packet(&decoded)), encoded);
        }
    }

    #[test]
    fn causal_and_retained_pins_share_the_registry_but_not_request_bytes_or_time() {
        let (mut reader, _, _, packet) = fixture();
        assert_eq!(reader.retry_packet(&retry_packet(&packet)).unwrap(), packet);
        let mut later = packet.clone();
        let Fact::Opened { pin, .. } = &mut later.facts[0] else {
            unreachable!()
        };
        pin.opened_at = i64::MAX;
        assert_eq!(later.tree(), packet.tree());
        assert_ne!(retry_packet(&later), retry_packet(&packet));

        let Cbor::Map(fields) = packet.tree() else {
            unreachable!()
        };
        assert_eq!(fields.len(), 1);
        let facts = items(&fields[&2]).unwrap();
        let fact = items(&facts[0]).unwrap();
        assert_eq!(fact[0], Cbor::Uint(1));
        let pin = items(&fact[6]).unwrap();
        assert_eq!(pin.len(), 4, "clock must not leak into the causal pin");
        assert_eq!(
            pin[1],
            Cbor::Bytes(PreparedRequest::seal(vec![0, 1, 255]).digest().to_vec())
        );
        let Fact::Opened { pin, .. } = &mut later.facts[0] else {
            unreachable!()
        };
        pin.prepared = PreparedRequest::seal(vec![0, 1, 254]);
        assert_ne!(later.tree(), packet.tree());
    }

    #[test]
    fn independent_batch_projection_pins_tags_references_and_order() {
        let (_, _, operation, _) = fixture();
        let attempt = AttemptId {
            operation,
            ordinal: 3,
        };
        let batch = Batch {
            attempt,
            call: CallId::new(Id::from_bytes([5; 16])),
            emissions: vec![Id::from_bytes([9; 16]), Id::from_bytes([7; 16])],
        };
        for (tag, fact) in [
            (3, Fact::Held(batch.clone())),
            (4, Fact::Consumed(batch.clone())),
            (6, Fact::Published(batch.clone())),
        ] {
            let packet = Packet {
                context: Some(attempt),
                facts: vec![fact],
            };
            let expected = Cbor::Map(BTreeMap::from([
                (1, list([id(operation.id()), Cbor::Uint(3)])),
                (
                    2,
                    list([list([
                        Cbor::Uint(tag),
                        id(operation.id()),
                        Cbor::Uint(3),
                        id(batch.call.id()),
                        list([id(batch.emissions[0]), id(batch.emissions[1])]),
                    ])]),
                ),
            ]));
            assert_eq!(packet.tree(), expected);
            assert_eq!(retry_packet(&packet), expected);
        }
    }

    #[test]
    fn retry_packet_refuses_unknown_empty_and_ambiguous_shapes() {
        let (_, _, operation, _) = fixture();
        let field = |value| Cbor::Map(BTreeMap::from([(2, list([value]))]));
        let invalid = [
            Cbor::Null,
            Cbor::Map(BTreeMap::new()),
            Cbor::Map(BTreeMap::from([(3, Cbor::Uint(1))])),
            Cbor::Map(BTreeMap::from([(2, list([]))])),
            Cbor::Map(BTreeMap::from([(
                1,
                list([id(operation.id()), Cbor::Uint(0)]),
            )])),
            field(list([Cbor::Uint(8)])),
            field(list([
                Cbor::Uint(3),
                id(operation.id()),
                Cbor::Uint(1),
                id(operation.id()),
                list([]),
            ])),
            field(list([
                Cbor::Uint(4),
                id(operation.id()),
                Cbor::Uint(1),
                id(operation.id()),
                list([id(operation.id()), id(operation.id())]),
            ])),
            field(list([
                Cbor::Uint(5),
                id(operation.id()),
                Cbor::Uint(1),
                Cbor::Uint(3),
                id(operation.id()),
            ])),
            field(list([
                Cbor::Uint(5),
                id(operation.id()),
                Cbor::Uint(u64::MAX),
                Cbor::Uint(0),
                id(operation.id()),
            ])),
            field(list([
                Cbor::Uint(7),
                id(operation.id()),
                Cbor::Uint(0),
                Cbor::Uint(1),
            ])),
            field(list([
                Cbor::Uint(7),
                id(operation.id()),
                Cbor::Uint(1),
                Cbor::Uint(3),
            ])),
            field(list([
                Cbor::Uint(7),
                id(operation.id()),
                Cbor::Uint(1),
                Cbor::Uint(2),
                Cbor::Null,
            ])),
        ];
        for tree in invalid {
            assert!(Reader::default().retry_packet(&tree).is_err(), "{tree:?}");
        }
    }

    #[test]
    fn retry_pins_require_a_retained_source_and_canonical_settings() {
        let (_, _, _, packet) = fixture();
        assert_eq!(
            Reader::default().retry_packet(&retry_packet(&packet)),
            Err(Error::Invalid(
                "Retry pin envelope is not in retained prefix"
            ))
        );
        for duplicate in [false, true] {
            let (mut reader, _, _, _) = fixture();
            let mut tree = retry_packet(&packet);
            let Cbor::Map(fields) = &mut tree else {
                unreachable!()
            };
            let Cbor::Array(facts) = fields.get_mut(&2).unwrap() else {
                unreachable!()
            };
            let Cbor::Array(fact) = &mut facts[0] else {
                unreachable!()
            };
            let Cbor::Array(pin) = &mut fact[6] else {
                unreachable!()
            };
            let Cbor::Array(settings) = &mut pin[2] else {
                unreachable!()
            };
            if duplicate {
                settings[1] = settings[0].clone();
            } else {
                settings.reverse();
            }
            assert_eq!(
                reader.retry_packet(&tree),
                Err(Error::Invalid("Retry pin settings are not canonical"))
            );
        }
        let (mut reader, _, _, packet) = fixture();
        assert!(
            reader.retry_packet(&packet.tree()).is_err(),
            "a causal digest is not a retained request"
        );
    }
}
