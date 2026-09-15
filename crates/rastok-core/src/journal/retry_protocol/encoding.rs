use super::{Batch, CloseReason, Fact, Packet, Pin};
use crate::canonical_cbor::Value as Cbor;
use crate::envelope::{Envelope, Id};
use crate::identity::encoding::value_tagged;
use alloc::collections::BTreeMap;
use alloc::string::ToString;
use alloc::vec;

fn id(value: Id) -> Cbor {
    Cbor::Bytes(value.as_bytes().to_vec())
}

pub(super) fn causal_pin(pin: &Pin) -> Cbor {
    Cbor::Array(vec![
        id(pin.envelope),
        Cbor::Bytes(pin.prepared.digest().to_vec()),
        Cbor::Array(
            pin.settings
                .iter()
                .map(|(name, value)| {
                    Cbor::Array(vec![Cbor::Text(name.clone()), value_tagged(value)])
                })
                .collect(),
        ),
        pin.key
            .as_ref()
            .map_or(Cbor::Null, |key| Cbor::Text(key.clone())),
    ])
}

/// Fact tags/order are shared by the lossless carrier and the causal seal;
/// only operation pins and envelope payload representations differ.
pub(crate) fn packet_tree(
    packet: &Packet,
    pin: fn(&Pin) -> Cbor,
    envelope: fn(&Envelope) -> Cbor,
) -> Cbor {
    let mut fields = BTreeMap::new();
    if let Some(attempt) = packet.context {
        fields.insert(
            1,
            Cbor::Array(vec![
                id(attempt.operation.id()),
                Cbor::Uint(attempt.ordinal),
            ]),
        );
    }
    if !packet.facts.is_empty() {
        fields.insert(
            2,
            Cbor::Array(
                packet
                    .facts
                    .iter()
                    .map(|fact| match fact {
                        Fact::Opened {
                            operation,
                            opening,
                            entry,
                            plan,
                            contract,
                            pin: value,
                        } => Cbor::Array(vec![
                            Cbor::Uint(1),
                            id(operation.id()),
                            id(*opening),
                            Cbor::Text(entry.to_string()),
                            Cbor::Bytes(plan.as_bytes().to_vec()),
                            Cbor::Text(contract.to_string()),
                            pin(value),
                        ]),
                        Fact::Admitted {
                            attempt,
                            envelope: value,
                            delivery,
                        } => Cbor::Array(vec![
                            Cbor::Uint(2),
                            id(attempt.operation.id()),
                            Cbor::Uint(attempt.ordinal),
                            envelope(value),
                            id(*delivery),
                        ]),
                        Fact::Held(batch) => batch_tree(3, batch),
                        Fact::Consumed(batch) => batch_tree(4, batch),
                        Fact::Advance { from, to, call } => Cbor::Array(vec![
                            Cbor::Uint(5),
                            id(from.operation.id()),
                            Cbor::Uint(from.ordinal),
                            Cbor::Uint(*to),
                            id(call.id()),
                        ]),
                        Fact::Published(batch) => batch_tree(6, batch),
                        Fact::Close { attempt, reason } => Cbor::Array(vec![
                            Cbor::Uint(7),
                            id(attempt.operation.id()),
                            Cbor::Uint(attempt.ordinal),
                            Cbor::Uint(match reason {
                                CloseReason::Final => 1,
                                CloseReason::Stopped => 2,
                            }),
                        ]),
                    })
                    .collect(),
            ),
        );
    }
    Cbor::Map(fields)
}

fn batch_tree(tag: u64, batch: &Batch) -> Cbor {
    Cbor::Array(vec![
        Cbor::Uint(tag),
        id(batch.attempt.operation.id()),
        Cbor::Uint(batch.attempt.ordinal),
        id(batch.call.id()),
        Cbor::Array(batch.emissions.iter().copied().map(id).collect()),
    ])
}
