//! The typed Rastok projections onto canonical CBOR — the
//! `tagged_value` registry of `identity.md` §3, applied. This module
//! knows the tag numbers; [`crate::canonical_cbor`] knows only the
//! byte discipline. Golden traces and the plan package both seal
//! through here — one registry, everywhere.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use crate::envelope::{Data, Encoding, Envelope, Role, Scalar};
use crate::value::Value;

// The tree type keeps its historical local name: `Cbor` the tree
// against `Value` the logical domain — two vocabularies in one
// file need distinct names.
use crate::canonical_cbor::{Value as Cbor, encode};

/// The canonical envelope form, keys 1–9. Absent fields are omitted keys,
/// never nulls.
#[must_use]
pub fn canonical_envelope(envelope: &Envelope) -> Cbor {
    let sys = envelope.meta().sys();
    let mut pairs = BTreeMap::from([(1, Cbor::Bytes(sys.id().as_bytes().to_vec()))]);
    if let Some(parent) = sys.parent_id() {
        pairs.insert(2, Cbor::Bytes(parent.as_bytes().to_vec()));
    }
    pairs.insert(3, Cbor::Bytes(sys.origin_id().as_bytes().to_vec()));
    if sys.role() != Role::Default {
        pairs.insert(4, Cbor::Text(sys.role().to_string()));
    }
    pairs.insert(5, Cbor::Uint(sys.graph_version()));
    // Canonical form key 6 is RETIRED (extend-only): the content type moved
    // to the ext zone, sealed in key 9 (D-2026-08-10-4).
    pairs.insert(8, canonical_data(envelope.data()));
    let mut app_meta = Vec::new();
    for (field, value) in envelope.meta().app() {
        app_meta.push((String::from("app"), field.clone(), value));
    }
    for (field, value) in envelope.meta().ext() {
        app_meta.push((String::from("ext"), field.clone(), value));
    }
    if !app_meta.is_empty() {
        app_meta.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        pairs.insert(
            9,
            Cbor::Array(
                app_meta
                    .into_iter()
                    .map(|(namespace, field, value)| {
                        Cbor::Array(vec![
                            Cbor::Text(namespace),
                            Cbor::Text(field),
                            canonical_meta_value(value),
                        ])
                    })
                    .collect(),
            ),
        );
    }
    Cbor::Map(pairs)
}

/// `data` (canonical form key 8): `[encoding_tag, bytes]`. Encoded payloads
/// seal their original bytes, never re-encoded. A materialized value
/// has no storage bytes, so its canonical bytes are its own
/// `tagged_value` CBOR under `raw` — deterministic and injective,
/// this build's materialization rule.
#[must_use]
pub fn canonical_data(data: &Data) -> Cbor {
    match data {
        Data::Encoded {
            encoding, bytes, ..
        } => Cbor::Array(vec![
            Cbor::Uint(match encoding {
                Encoding::Raw => 1,
                Encoding::Avro => 2,
                Encoding::ArrowIpc => 3,
            }),
            Cbor::Bytes(bytes.clone()),
        ]),
        Data::Materialized(value) => Cbor::Array(vec![
            Cbor::Uint(1),
            Cbor::Bytes(encode(&canonical_value(value))),
        ]),
    }
}

/// A `tagged_value` for a materialized value: `[type_tag, value]` per
/// the fixed registry.
pub fn canonical_value(value: &Value) -> Cbor {
    match value {
        Value::Null => Cbor::Array(vec![Cbor::Uint(9), Cbor::Null]),
        Value::Bool(v) => Cbor::Array(vec![Cbor::Uint(1), Cbor::Bool(*v)]),
        Value::Int(v) => int_tagged(*v),
        Value::Uint(v) => Cbor::Array(vec![Cbor::Uint(23), Cbor::Uint(*v)]),
        Value::Double(v) => Cbor::Array(vec![Cbor::Uint(5), Cbor::Double((*v).into())]),
        Value::Text(v) => Cbor::Array(vec![Cbor::Uint(8), Cbor::Text(v.clone())]),
        Value::Decimal { unscaled, scale } => decimal_tagged(*unscaled, *scale),
        Value::Bytes(v) => Cbor::Array(vec![Cbor::Uint(7), Cbor::Bytes(v.clone())]),
        Value::Duration(v) => Cbor::Array(vec![Cbor::Uint(16), int_cbor(*v)]),
        Value::Timestamp(v) => Cbor::Array(vec![Cbor::Uint(12), int_cbor(*v)]),
        Value::Date(v) => Cbor::Array(vec![Cbor::Uint(11), int_cbor(*v)]),
        Value::Time(v) => Cbor::Array(vec![Cbor::Uint(14), int_cbor(*v)]),
        Value::TimestampNtz(v) => Cbor::Array(vec![Cbor::Uint(13), int_cbor(*v)]),
        Value::Interval {
            months,
            days,
            nanoseconds,
        } => Cbor::Array(vec![
            Cbor::Uint(24),
            Cbor::Array(vec![
                int_cbor(*months),
                int_cbor(*days),
                int_cbor(*nanoseconds),
            ]),
        ]),
        Value::Uuid(v) => Cbor::Array(vec![Cbor::Uint(15), Cbor::Bytes(v.to_vec())]),
        Value::Struct(fields) => {
            // Canonical sorted pairs — construction order is not
            // identity material.
            let mut sorted: Vec<_> = fields.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            Cbor::Array(vec![
                Cbor::Uint(17),
                Cbor::Array(
                    sorted
                        .into_iter()
                        .map(|(field, value)| {
                            Cbor::Array(vec![Cbor::Text(field.clone()), canonical_value(value)])
                        })
                        .collect(),
                ),
            ])
        }
        Value::Map(entries) => {
            // Sorted by the KEY'S canonical tagged bytes, not by
            // Rust's string order — CBOR text encodes its length
            // first, so "b" precedes "aa".
            let mut pairs: Vec<(Vec<u8>, Cbor)> = entries
                .iter()
                .map(|(key, value)| {
                    let tagged_key = Cbor::Array(vec![Cbor::Uint(8), Cbor::Text(key.clone())]);
                    (
                        encode(&tagged_key),
                        Cbor::Array(vec![tagged_key, canonical_value(value)]),
                    )
                })
                .collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            Cbor::Array(vec![
                Cbor::Uint(18),
                Cbor::Array(pairs.into_iter().map(|(_, pair)| pair).collect()),
            ])
        }
        Value::KeyedMap(entries) => {
            let mut pairs: Vec<(Vec<u8>, Cbor)> = entries
                .iter()
                .map(|(key, value)| {
                    let tagged_key = canonical_value(key);
                    (
                        encode(&tagged_key),
                        Cbor::Array(vec![tagged_key, canonical_value(value)]),
                    )
                })
                .collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            Cbor::Array(vec![
                Cbor::Uint(18),
                Cbor::Array(pairs.into_iter().map(|(_, pair)| pair).collect()),
            ])
        }
        Value::Array(items) => Cbor::Array(vec![
            Cbor::Uint(19),
            Cbor::Array(items.iter().map(canonical_value).collect()),
        ]),
        Value::Enum(symbol) => Cbor::Array(vec![Cbor::Uint(20), Cbor::Text(symbol.clone())]),
        Value::StructuralUnion { branch, value } => Cbor::Array(vec![
            Cbor::Uint(21),
            Cbor::Array(vec![Cbor::Uint(u64::from(*branch)), canonical_value(value)]),
        ]),
        Value::TaggedUnion(tagged) => Cbor::Array(vec![
            Cbor::Uint(21),
            Cbor::Array(vec![
                Cbor::Text(tagged.case.clone()),
                canonical_value(&Value::Struct(tagged.payload.clone())),
            ]),
        ]),
    }
}

/// A `tagged_value` for writable metadata (flat by type).
pub fn canonical_meta_value(value: &crate::envelope::MetaValue) -> Cbor {
    match value {
        crate::envelope::MetaValue::Scalar(scalar) => scalar_tagged(scalar),
        crate::envelope::MetaValue::Array(items) => Cbor::Array(vec![
            Cbor::Uint(19),
            Cbor::Array(items.iter().map(scalar_tagged).collect()),
        ]),
        crate::envelope::MetaValue::Map(entries) => {
            let mut pairs: Vec<(Vec<u8>, Cbor)> = entries
                .iter()
                .map(|(key, values)| {
                    let tagged_key = Cbor::Array(vec![Cbor::Uint(8), Cbor::Text(key.clone())]);
                    let tagged_values = Cbor::Array(vec![
                        Cbor::Uint(19),
                        Cbor::Array(values.iter().map(scalar_tagged).collect()),
                    ]);
                    (
                        encode(&tagged_key),
                        Cbor::Array(vec![tagged_key, tagged_values]),
                    )
                })
                .collect();
            pairs.sort_by(|left, right| left.0.cmp(&right.0));
            Cbor::Array(vec![
                Cbor::Uint(18),
                Cbor::Array(pairs.into_iter().map(|(_, pair)| pair).collect()),
            ])
        }
    }
}

/// Reads a `tagged_value` back into the core value domain — the inverse of
/// [`canonical_value`], shared by plan and diagnostic-recording readers.
///
/// # Errors
/// A malformed tagged value. The enclosing format checks canonical re-encoding.
pub fn decode_canonical_value(tree: &Cbor) -> Result<Value, crate::canonical_cbor::DecodeError> {
    match tagged_array(tree, "tagged value")? {
        [Cbor::Uint(1), Cbor::Bool(v)] => Ok(Value::Bool(*v)),
        [Cbor::Uint(2), value] | [Cbor::Uint(3), value] => {
            // Tags 2 (int) and 3 (long) are both the signed domain —
            // the registry keeps them distinct so the seal shows the
            // honest width; the decoded value is one i64 either way.
            Ok(Value::Int(tagged_integer(value)?))
        }
        [Cbor::Uint(5), Cbor::Double(v)] => Ok(Value::Double(v.value())),
        [Cbor::Uint(7), Cbor::Bytes(v)] => Ok(Value::Bytes(v.clone())),
        [Cbor::Uint(8), Cbor::Text(v)] => Ok(Value::Text(v.clone())),
        [Cbor::Uint(9), Cbor::Null] => Ok(Value::Null),
        [Cbor::Uint(23), Cbor::Uint(v)] => Ok(Value::Uint(*v)),
        [Cbor::Uint(10), payload] => match tagged_array(payload, "decimal")? {
            [Cbor::Bytes(unscaled), scale] => {
                if unscaled.is_empty() || unscaled.len() > 16 {
                    return Err(value_decode_error("decimal coefficient"));
                }
                let mut wide = if unscaled[0] & 0x80 != 0 {
                    [0xFF_u8; 16]
                } else {
                    [0x00_u8; 16]
                };
                wide[16 - unscaled.len()..].copy_from_slice(unscaled);
                let value = i128::from_be_bytes(wide);
                let scale_wide = tagged_integer(scale)?;
                // The value registry carries the decimal128 domain. Whether
                // the value is a declared-scale decimal or a schema-less
                // number is decided by its transported DataType, not by a
                // second decoder-local fold.
                let scale =
                    i32::try_from(scale_wide).map_err(|_| value_decode_error("decimal scale"))?;
                if !crate::datatype::decimal_in_domain(value, scale) {
                    return Err(value_decode_error("decimal form"));
                }
                Ok(Value::Decimal {
                    unscaled: value,
                    scale,
                })
            }
            _ => Err(value_decode_error("decimal")),
        },
        [Cbor::Uint(12), value] => Ok(Value::Timestamp(tagged_integer(value)?)),
        [Cbor::Uint(11), value] => Ok(Value::Date(tagged_integer(value)?)),
        [Cbor::Uint(13), value] => Ok(Value::TimestampNtz(tagged_integer(value)?)),
        [Cbor::Uint(14), value] => Ok(Value::Time(tagged_integer(value)?)),
        [Cbor::Uint(15), Cbor::Bytes(value)] => Ok(Value::Uuid(
            value
                .as_slice()
                .try_into()
                .map_err(|_| value_decode_error("uuid length"))?,
        )),
        [Cbor::Uint(16), value] => Ok(Value::Duration(tagged_integer(value)?)),
        [Cbor::Uint(17), Cbor::Array(pairs)] => {
            // The struct literal — bare text keys, declaration order AS
            // SEALED (sorted). Its surrounding DataType supplies the closed
            // field contract; validation checks conformance before encoding.
            let mut fields = alloc::vec::Vec::new();
            for pair in pairs {
                match tagged_array(pair, "struct field")? {
                    [Cbor::Text(name), value] => {
                        fields.push((name.clone(), decode_canonical_value(value)?));
                    }
                    _ => return Err(value_decode_error("struct field")),
                }
            }
            Ok(Value::Struct(fields))
        }
        [Cbor::Uint(18), Cbor::Array(pairs)] => map_value_from_tagged(pairs),
        [Cbor::Uint(19), Cbor::Array(items)] => Ok(Value::Array(
            items
                .iter()
                .map(decode_canonical_value)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        [Cbor::Uint(20), Cbor::Text(symbol)] => Ok(Value::Enum(symbol.clone())),
        [Cbor::Uint(21), payload] => match tagged_array(payload, "union value")? {
            [Cbor::Uint(branch), value] => Ok(Value::StructuralUnion {
                branch: u16::try_from(*branch).map_err(|_| value_decode_error("union branch"))?,
                value: alloc::boxed::Box::new(decode_canonical_value(value)?),
            }),
            [Cbor::Text(case), value] => {
                let Value::Struct(payload) = decode_canonical_value(value)? else {
                    return Err(value_decode_error("tagged-union payload"));
                };
                Ok(Value::TaggedUnion(alloc::boxed::Box::new(
                    crate::value::TaggedUnionValue {
                        case: case.clone(),
                        payload,
                    },
                )))
            }
            _ => Err(value_decode_error("union value")),
        },
        [Cbor::Uint(24), payload] => match tagged_array(payload, "interval")? {
            [months, days, nanoseconds] => Ok(Value::Interval {
                months: tagged_integer(months)?,
                days: tagged_integer(days)?,
                nanoseconds: tagged_integer(nanoseconds)?,
            }),
            _ => Err(value_decode_error("interval")),
        },
        _ => Err(value_decode_error("tagged value")),
    }
}

fn map_value_from_tagged(pairs: &[Cbor]) -> Result<Value, crate::canonical_cbor::DecodeError> {
    let mut decoded = Vec::with_capacity(pairs.len());
    let mut all_text = true;
    for pair in pairs {
        let [key, value] = tagged_array(pair, "map entry")? else {
            return Err(value_decode_error("map entry"));
        };
        let key = decode_canonical_value(key)?;
        all_text &= matches!(key, Value::Text(_));
        decoded.push((key, decode_canonical_value(value)?));
    }
    if all_text {
        let mut map = BTreeMap::new();
        for (key, value) in decoded {
            let Value::Text(key) = key else {
                return Err(value_decode_error("text map key"));
            };
            if map.insert(key, value).is_some() {
                return Err(value_decode_error("duplicate map key"));
            }
        }
        Ok(Value::Map(map))
    } else {
        Ok(Value::KeyedMap(decoded))
    }
}

fn value_decode_error(what: &'static str) -> crate::canonical_cbor::DecodeError {
    crate::canonical_cbor::DecodeError::Malformed(what)
}

fn tagged_array<'a>(
    tree: &'a Cbor,
    what: &'static str,
) -> Result<&'a [Cbor], crate::canonical_cbor::DecodeError> {
    match tree {
        Cbor::Array(values) => Ok(values),
        _ => Err(value_decode_error(what)),
    }
}

fn tagged_integer(tree: &Cbor) -> Result<i64, crate::canonical_cbor::DecodeError> {
    match tree {
        Cbor::Uint(value) => i64::try_from(*value).map_err(|_| value_decode_error("integer range")),
        Cbor::Negative(value) => i64::try_from(*value)
            .map(|value| -1 - value)
            .map_err(|_| value_decode_error("integer range")),
        _ => Err(value_decode_error("integer")),
    }
}

fn scalar_tagged(value: &Scalar) -> Cbor {
    match value {
        Scalar::Bool(v) => Cbor::Array(vec![Cbor::Uint(1), Cbor::Bool(*v)]),
        Scalar::Int(v) => int_tagged(*v),
        Scalar::Double(v) => Cbor::Array(vec![Cbor::Uint(5), Cbor::Double((*v).into())]),
        Scalar::Text(v) => Cbor::Array(vec![Cbor::Uint(8), Cbor::Text(v.clone())]),
        Scalar::Decimal { unscaled, scale } => decimal_tagged(*unscaled, *scale),
        Scalar::Bytes(v) => Cbor::Array(vec![Cbor::Uint(7), Cbor::Bytes(v.clone())]),
    }
}

/// `decimal` (tag 10): `[unscaled (minimal big-endian
/// two's-complement bytes), scale (int)]` — the registry's pinned
/// layout, shared by typed and dynamic decimals (D-2026-08-10-7).
pub fn decimal_tagged(unscaled: i128, scale: i32) -> Cbor {
    Cbor::Array(vec![
        Cbor::Uint(10),
        Cbor::Array(vec![
            Cbor::Bytes(minimal_twos_complement(unscaled)),
            int_cbor(i64::from(scale)),
        ]),
    ])
}

/// Minimal big-endian two's complement — no redundant sign byte;
/// zero is a single `0x00` (the registry's rule).
fn minimal_twos_complement(value: i128) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    let mut start = 0;
    while start < 15 {
        let cut = bytes[start];
        let next_msb = bytes[start + 1] & 0x80;
        let redundant = (cut == 0x00 && next_msb == 0) || (cut == 0xFF && next_msb != 0);
        if !redundant {
            break;
        }
        start += 1;
    }
    bytes[start..].to_vec()
}

/// `int` (tag 2) inside the 32-bit range, `long` (tag 3) beyond — the
/// DSL's `int` is 32-bit; runtime arithmetic that leaves the range
/// seals honestly as a long.
pub fn int_tagged(value: i64) -> Cbor {
    let tag = if i32::try_from(value).is_ok() { 2 } else { 3 };
    Cbor::Array(vec![Cbor::Uint(tag), int_cbor(value)])
}

fn int_cbor(value: i64) -> Cbor {
    if value >= 0 {
        Cbor::Uint(u64::try_from(value).unwrap_or_else(|_| unreachable!("non-negative")))
    } else {
        // Major type 1 carries −1−n; `−1 − value` stays in range for
        // every negative i64 (`i64::MIN` maps to `i64::MAX`).
        Cbor::Negative(
            u64::try_from(-1 - value).unwrap_or_else(|_| unreachable!("negative maps into u64")),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeMap;
    use alloc::string::String;

    /// Map keys sort by their canonical tagged bytes — CBOR text
    /// encodes its length first, so `"b"` precedes `"aa"`, against
    /// Rust's string order.
    #[test]
    fn map_keys_sort_by_canonical_bytes_not_string_order() {
        let mut entries = std::collections::BTreeMap::new();
        entries.insert(String::from("aa"), crate::value::Value::Bool(true));
        entries.insert(String::from("b"), crate::value::Value::Bool(false));
        let Cbor::Array(parts) = canonical_value(&crate::value::Value::Map(entries)) else {
            panic!("a tagged value");
        };
        let Cbor::Array(pairs) = &parts[1] else {
            panic!("the pair list");
        };
        let Cbor::Array(first) = &pairs[0] else {
            panic!("a pair");
        };
        let Cbor::Array(key) = &first[0] else {
            panic!("a tagged key");
        };
        assert_eq!(
            key[1],
            Cbor::Text(String::from("b")),
            "shorter text is byte-smaller"
        );
    }

    #[test]
    fn lossless_header_metadata_reuses_the_canonical_map_and_array_tags() {
        let headers = crate::envelope::MetaValue::Map(BTreeMap::from([
            (
                String::from("aa"),
                alloc::vec![crate::envelope::Scalar::Bytes(alloc::vec![2])],
            ),
            (
                String::from("b"),
                alloc::vec![
                    crate::envelope::Scalar::Bytes(alloc::vec![3]),
                    crate::envelope::Scalar::Bytes(alloc::vec![4]),
                ],
            ),
        ]));
        let Cbor::Array(tagged) = canonical_meta_value(&headers) else {
            panic!("metadata map has one tagged form");
        };
        assert_eq!(tagged[0], Cbor::Uint(18));
        let Cbor::Array(entries) = &tagged[1] else {
            panic!("metadata map contains canonical pairs");
        };
        let Cbor::Array(first) = &entries[0] else {
            panic!("metadata map entry is a pair");
        };
        assert_eq!(
            first[0],
            Cbor::Array(alloc::vec![Cbor::Uint(8), Cbor::Text(String::from("b"))]),
            "canonical bytes, rather than Rust string order, own map ordering",
        );
        assert!(matches!(first[1], Cbor::Array(ref value) if value[0] == Cbor::Uint(19)));
    }

    #[test]
    fn the_two_union_families_seal_their_semantic_discriminant() {
        let structural = canonical_value(&crate::value::Value::StructuralUnion {
            branch: 2,
            value: alloc::boxed::Box::new(crate::value::Value::Text(String::from("value"))),
        });
        let tagged = canonical_value(&crate::value::Value::TaggedUnion(alloc::boxed::Box::new(
            crate::value::TaggedUnionValue {
                case: String::from("Exited"),
                payload: alloc::vec![(String::from("exit_code"), crate::value::Value::Int(0))],
            },
        )));

        let Cbor::Array(structural) = structural else {
            panic!("a tagged structural union");
        };
        let Cbor::Array(structural_body) = &structural[1] else {
            panic!("a structural union body");
        };
        assert_eq!(structural_body[0], Cbor::Uint(2));

        let Cbor::Array(tagged) = tagged else {
            panic!("a tagged named union");
        };
        let Cbor::Array(tagged_body) = &tagged[1] else {
            panic!("a named union body");
        };
        assert_eq!(tagged_body[0], Cbor::Text(String::from("Exited")));
        assert!(matches!(
            tagged_body[1],
            Cbor::Array(ref payload) if payload.first() == Some(&Cbor::Uint(17))
        ));
    }

    #[test]
    fn every_completed_scalar_family_has_one_canonical_value_tag() {
        let values = [
            (crate::value::Value::Date(0), 11),
            (crate::value::Value::TimestampNtz(0), 13),
            (crate::value::Value::Time(0), 14),
            (crate::value::Value::Uuid([0; 16]), 15),
            (
                crate::value::Value::Interval {
                    months: 1,
                    days: 2,
                    nanoseconds: 3,
                },
                24,
            ),
        ];
        for (value, expected_tag) in values {
            let Cbor::Array(tagged) = canonical_value(&value) else {
                panic!("a tagged value");
            };
            assert_eq!(tagged.first(), Some(&Cbor::Uint(expected_tag)));
        }
    }
}
