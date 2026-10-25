//! The canonical registry: Rastok types and values projected onto CBOR.
//!
//! CBOR represents binary items with a head byte naming the item's kind
//! and an argument (a value or length), using the shortest permitted form.
//! It is a standard, schema-free substrate that can be audited without
//! Rastok. Our profile fixes every representation used by an identity,
//! causal seal, plan hash or golden; see [identity.md §3].
//!
//! This module owns the tag registry. [`crate::canonical_cbor`] owns only
//! the byte discipline. The codec is handwritten because the canonical
//! value ↔ bytes bijection must remain stable across builds: a third-party
//! library may change between equally legal CBOR encodings on an upgrade,
//! which would change our seals. [D-2026-08-03-1] fixes that responsibility
//! here, rather than delegating identity to a library's encoding choices.
//!
//! [identity.md §3]: https://github.com/rastok-io/rastok/blob/main/docs/spec/identity.md#3-the-canonical-record-form
//! [D-2026-08-03-1]: https://github.com/rastok-io/rastok/blob/main/docs/spec/decisions.md#d-2026-08-03-1--canonical-bytes-are-hand-rolled-deterministic-cbor

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use crate::datatype::{
    ArrayType, BinaryType, ByteLength, DataType, DateUnit, DecimalType, EnumType, FloatType,
    IntegerType, IntegerWidth, IntervalUnit, MapType, StructType, TaggedCase, TaggedUnionType,
    TemporalType, TimeUnit, TimeZone, TypeField,
};
use crate::envelope::{Data, Encoding, Envelope, Scalar};
use crate::value::Value;

// CBOR item trees and logical values are distinct domains.
use crate::canonical_cbor::{Value as Cbor, encode};

/// The canonical envelope form, keys 1–6. Absent fields are omitted keys,
/// never nulls.
#[must_use]
pub fn envelope(envelope: &Envelope) -> Cbor {
    let sys = envelope.meta().sys();
    let mut pairs = BTreeMap::from([(1, Cbor::Bytes(sys.id().as_bytes().to_vec()))]);
    if let Some(parent) = sys.parent_id() {
        pairs.insert(2, Cbor::Bytes(parent.as_bytes().to_vec()));
    }
    pairs.insert(3, Cbor::Bytes(sys.origin_id().as_bytes().to_vec()));
    pairs.insert(4, Cbor::Uint(sys.graph_version()));
    pairs.insert(5, data(envelope.data()));
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
            6,
            Cbor::Array(
                app_meta
                    .into_iter()
                    .map(|(namespace, field, value)| {
                        Cbor::Array(vec![
                            Cbor::Text(namespace),
                            Cbor::Text(field),
                            meta_value(value),
                        ])
                    })
                    .collect(),
            ),
        );
    }
    Cbor::Map(pairs)
}

/// `data` (canonical form key 5): `[encoding_tag, bytes]`. Encoded payloads
/// seal their original bytes, never re-encoded. A value created in the
/// graph uses its canonical tagged-value bytes under `ccbor`, distinct from
/// external `raw` bytes even when those bytes happen to encode the same
/// tagged value.
#[must_use]
pub fn data(data: &Data) -> Cbor {
    match data {
        Data::Encoded {
            encoding, bytes, ..
        } => Cbor::Array(vec![
            Cbor::Uint(match encoding {
                Encoding::Raw => 1,
                Encoding::Ccbor => 2,
                Encoding::ArrowIpc => 3,
            }),
            Cbor::Bytes(bytes.to_vec()),
        ]),
        Data::Materialized(value) => Cbor::Array(vec![
            Cbor::Uint(2),
            Cbor::Bytes(encode(&self::value(value))),
        ]),
    }
}

/// A `tagged_value` for a materialized value: `[type_tag, value]` per
/// the fixed registry.
pub fn value(value: &Value) -> Cbor {
    match value {
        Value::Null => Cbor::Array(vec![Cbor::Uint(7), Cbor::Null]),
        Value::Bool(v) => Cbor::Array(vec![Cbor::Uint(1), Cbor::Bool(*v)]),
        Value::Int(v) => int(*v),
        Value::Uint(v) => Cbor::Array(vec![Cbor::Uint(20), Cbor::Uint(*v)]),
        Value::Double(v) => Cbor::Array(vec![Cbor::Uint(4), Cbor::Double((*v).into())]),
        Value::Text(v) => Cbor::Array(vec![Cbor::Uint(6), Cbor::Text(v.clone())]),
        Value::Decimal { unscaled, scale } => decimal(*unscaled, *scale),
        Value::Bytes(v) => Cbor::Array(vec![Cbor::Uint(5), Cbor::Bytes(v.clone())]),
        Value::Duration(v) => Cbor::Array(vec![Cbor::Uint(14), signed(*v)]),
        Value::Timestamp(v) => Cbor::Array(vec![Cbor::Uint(22), timestamp(*v)]),
        Value::Date(v) => Cbor::Array(vec![Cbor::Uint(9), signed(*v)]),
        Value::Time(v) => Cbor::Array(vec![Cbor::Uint(12), signed(*v)]),
        Value::TimestampNtz(v) => Cbor::Array(vec![Cbor::Uint(23), timestamp(*v)]),
        Value::Interval {
            months,
            days,
            nanoseconds,
        } => Cbor::Array(vec![
            Cbor::Uint(21),
            Cbor::Array(vec![signed(*months), signed(*days), signed(*nanoseconds)]),
        ]),
        Value::Uuid(v) => Cbor::Array(vec![Cbor::Uint(13), Cbor::Bytes(v.to_vec())]),
        Value::Struct(fields) => {
            // Canonical sorted pairs — construction order is not
            // identity material.
            let mut sorted: Vec<_> = fields.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            Cbor::Array(vec![
                Cbor::Uint(15),
                Cbor::Array(
                    sorted
                        .into_iter()
                        .map(|(field, value)| {
                            Cbor::Array(vec![Cbor::Text(field.clone()), self::value(value)])
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
                    let tagged_key = Cbor::Array(vec![Cbor::Uint(6), Cbor::Text(key.clone())]);
                    (
                        encode(&tagged_key),
                        Cbor::Array(vec![tagged_key, self::value(value)]),
                    )
                })
                .collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            Cbor::Array(vec![
                Cbor::Uint(16),
                Cbor::Array(pairs.into_iter().map(|(_, pair)| pair).collect()),
            ])
        }
        Value::KeyedMap(entries) => {
            let mut pairs: Vec<(Vec<u8>, Cbor)> = entries
                .iter()
                .map(|(key, value)| {
                    let tagged_key = self::value(key);
                    (
                        encode(&tagged_key),
                        Cbor::Array(vec![tagged_key, self::value(value)]),
                    )
                })
                .collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            Cbor::Array(vec![
                Cbor::Uint(16),
                Cbor::Array(pairs.into_iter().map(|(_, pair)| pair).collect()),
            ])
        }
        Value::Array(items) => Cbor::Array(vec![
            Cbor::Uint(17),
            Cbor::Array(items.iter().map(self::value).collect()),
        ]),
        Value::Enum(symbol) => Cbor::Array(vec![Cbor::Uint(18), Cbor::Text(symbol.clone())]),
        Value::StructuralUnion { branch, value } => Cbor::Array(vec![
            Cbor::Uint(19),
            Cbor::Array(vec![Cbor::Uint(u64::from(*branch)), self::value(value)]),
        ]),
        Value::TaggedUnion(tagged) => Cbor::Array(vec![
            Cbor::Uint(19),
            Cbor::Array(vec![
                Cbor::Text(tagged.case.clone()),
                self::value(&Value::Struct(tagged.payload.clone())),
            ]),
        ]),
    }
}

/// The normalized civil pair used by timestamp values, observed time and box
/// identities. It carries no value tag; the enclosing field fixes its meaning.
pub fn timestamp(value: crate::time::Timestamp) -> Cbor {
    Cbor::Array(vec![
        signed(value.seconds()),
        Cbor::Uint(u64::from(value.nanoseconds())),
    ])
}

/// Reads one normalized civil pair without accepting the former integer carrier.
///
/// # Errors
/// A malformed pair, seconds outside i64, or a fraction at or above one billion.
pub fn decode_timestamp(
    value: &Cbor,
) -> Result<crate::time::Timestamp, crate::canonical_cbor::DecodeError> {
    let Cbor::Array(pair) = value else {
        return Err(value_decode_error("timestamp pair"));
    };
    let [seconds, Cbor::Uint(nanoseconds)] = pair.as_slice() else {
        return Err(value_decode_error("timestamp pair"));
    };
    let nanoseconds =
        u32::try_from(*nanoseconds).map_err(|_| value_decode_error("timestamp fraction"))?;
    crate::time::Timestamp::try_new(tagged_integer(seconds)?, nanoseconds)
        .map_err(|_| value_decode_error("timestamp fraction"))
}

/// A `tagged_value` for writable metadata (flat by type).
pub fn meta_value(value: &crate::envelope::MetaValue) -> Cbor {
    match value {
        crate::envelope::MetaValue::Scalar(scalar) => scalar_tagged(scalar),
        crate::envelope::MetaValue::Array(items) => Cbor::Array(vec![
            Cbor::Uint(17),
            Cbor::Array(items.iter().map(scalar_tagged).collect()),
        ]),
        crate::envelope::MetaValue::Map(entries) => {
            let mut pairs: Vec<(Vec<u8>, Cbor)> = entries
                .iter()
                .map(|(key, values)| {
                    let tagged_key = Cbor::Array(vec![Cbor::Uint(6), Cbor::Text(key.clone())]);
                    let tagged_values = Cbor::Array(vec![
                        Cbor::Uint(17),
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
                Cbor::Uint(16),
                Cbor::Array(pairs.into_iter().map(|(_, pair)| pair).collect()),
            ])
        }
    }
}

/// Decodes the complete ccbor DATA representation, not a prefix. The value
/// algebra also has a canonical form (field order, integer width, decimals),
/// which is stricter than merely accepting canonical CBOR items.
///
/// # Errors
/// Invalid CBOR, inconsistent values, trailing bytes or a noncanonical value.
pub fn decode_data_value(bytes: &[u8]) -> Result<Value, crate::canonical_cbor::DecodeError> {
    use crate::canonical_cbor::{DecodeError, decode};
    let (tree, used) = decode(bytes)?;
    if used != bytes.len() {
        return Err(DecodeError::Malformed("trailing ccbor DATA bytes"));
    }
    let decoded = decode_value(&tree)?;
    decoded
        .check_consistency()
        .map_err(|_| DecodeError::Malformed("inconsistent ccbor value"))?;
    if encode(&value(&decoded)) != bytes {
        return Err(DecodeError::NonCanonical("ccbor value form"));
    }
    Ok(decoded)
}

/// Reads a `tagged_value` back into the core value domain — the inverse of
/// [`value`], shared by plan and diagnostic-recording readers.
///
/// # Errors
/// A malformed tagged value. The enclosing format checks canonical re-encoding.
pub fn decode_value(tree: &Cbor) -> Result<Value, crate::canonical_cbor::DecodeError> {
    match tagged_array(tree, "tagged value")? {
        [Cbor::Uint(1), Cbor::Bool(v)] => Ok(Value::Bool(*v)),
        [Cbor::Uint(2), value] | [Cbor::Uint(3), value] => {
            // Tags 2 (int) and 3 (long) are both the signed domain —
            // the registry keeps them distinct so the seal shows the
            // honest width; the decoded value is one i64 either way.
            Ok(Value::Int(tagged_integer(value)?))
        }
        [Cbor::Uint(4), Cbor::Double(v)] => Ok(Value::Double(v.value())),
        [Cbor::Uint(5), Cbor::Bytes(v)] => Ok(Value::Bytes(v.clone())),
        [Cbor::Uint(6), Cbor::Text(v)] => Ok(Value::Text(v.clone())),
        [Cbor::Uint(7), Cbor::Null] => Ok(Value::Null),
        [Cbor::Uint(20), Cbor::Uint(v)] => Ok(Value::Uint(*v)),
        [Cbor::Uint(8), payload] => match tagged_array(payload, "decimal")? {
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
        [Cbor::Uint(22), value] => Ok(Value::Timestamp(decode_timestamp(value)?)),
        [Cbor::Uint(9), value] => Ok(Value::Date(tagged_integer(value)?)),
        [Cbor::Uint(23), value] => Ok(Value::TimestampNtz(decode_timestamp(value)?)),
        [Cbor::Uint(12), value] => Ok(Value::Time(tagged_integer(value)?)),
        [Cbor::Uint(13), Cbor::Bytes(value)] => Ok(Value::Uuid(
            value
                .as_slice()
                .try_into()
                .map_err(|_| value_decode_error("uuid length"))?,
        )),
        [Cbor::Uint(14), value] => Ok(Value::Duration(tagged_integer(value)?)),
        [Cbor::Uint(15), Cbor::Array(pairs)] => {
            // The struct literal — bare text keys, declaration order AS
            // SEALED (sorted). Its surrounding DataType supplies the closed
            // field contract; validation checks conformance before encoding.
            let mut fields = alloc::vec::Vec::new();
            for pair in pairs {
                match tagged_array(pair, "struct field")? {
                    [Cbor::Text(name), value] => {
                        fields.push((name.clone(), decode_value(value)?));
                    }
                    _ => return Err(value_decode_error("struct field")),
                }
            }
            Ok(Value::Struct(fields))
        }
        [Cbor::Uint(16), Cbor::Array(pairs)] => map_from_canonical(pairs),
        [Cbor::Uint(17), Cbor::Array(items)] => Ok(Value::Array(
            items
                .iter()
                .map(decode_value)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        [Cbor::Uint(18), Cbor::Text(symbol)] => Ok(Value::Enum(symbol.clone())),
        [Cbor::Uint(19), payload] => match tagged_array(payload, "union value")? {
            [Cbor::Uint(branch), value] => Ok(Value::StructuralUnion {
                branch: u16::try_from(*branch).map_err(|_| value_decode_error("union branch"))?,
                value: alloc::boxed::Box::new(decode_value(value)?),
            }),
            [Cbor::Text(case), value] => {
                let Value::Struct(payload) = decode_value(value)? else {
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
        [Cbor::Uint(21), payload] => match tagged_array(payload, "interval")? {
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

fn map_from_canonical(pairs: &[Cbor]) -> Result<Value, crate::canonical_cbor::DecodeError> {
    let mut decoded = Vec::with_capacity(pairs.len());
    let mut all_text = true;
    for pair in pairs {
        let [key, value] = tagged_array(pair, "map entry")? else {
            return Err(value_decode_error("map entry"));
        };
        let key = decode_value(key)?;
        all_text &= matches!(key, Value::Text(_));
        decoded.push((key, decode_value(value)?));
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
        Scalar::Int(v) => int(*v),
        Scalar::Double(v) => Cbor::Array(vec![Cbor::Uint(4), Cbor::Double((*v).into())]),
        Scalar::Text(v) => Cbor::Array(vec![Cbor::Uint(6), Cbor::Text(v.clone())]),
        Scalar::Decimal { unscaled, scale } => decimal(*unscaled, *scale),
        Scalar::Bytes(v) => Cbor::Array(vec![Cbor::Uint(5), Cbor::Bytes(v.clone())]),
    }
}

/// `decimal` (tag 8): `[unscaled (minimal big-endian
/// two's-complement bytes), scale (int)]` — the registry's pinned
/// layout, shared by typed and dynamic decimals (D-2026-08-10-7).
pub fn decimal(unscaled: i128, scale: i32) -> Cbor {
    Cbor::Array(vec![
        Cbor::Uint(8),
        Cbor::Array(vec![
            Cbor::Bytes(minimal_twos_complement(unscaled)),
            signed(i64::from(scale)),
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
pub fn int(value: i64) -> Cbor {
    let tag = if i32::try_from(value).is_ok() { 2 } else { 3 };
    Cbor::Array(vec![Cbor::Uint(tag), signed(value)])
}

/// The untagged CBOR integer shared by values, type parameters and instance segments.
pub(crate) fn signed(value: i64) -> Cbor {
    if value >= 0 {
        Cbor::Uint(value.unsigned_abs())
    } else {
        Cbor::Negative(value.unsigned_abs() - 1)
    }
}

pub(crate) fn type_tree(ty: &DataType) -> Cbor {
    match ty {
        DataType::Null => Cbor::Uint(1),
        DataType::Bool => Cbor::Uint(2),
        DataType::Integer(integer) => {
            let (signedness, width) = match integer {
                IntegerType::Signed(width) => (1, width.bits()),
                IntegerType::Unsigned(width) => (2, width.bits()),
            };
            Cbor::Array(vec![
                Cbor::Uint(3),
                Cbor::Uint(signedness),
                Cbor::Uint(u64::from(width)),
            ])
        }
        DataType::Float(float) => {
            Cbor::Array(vec![Cbor::Uint(4), Cbor::Uint(u64::from(float.bits()))])
        }
        DataType::Decimal(decimal) => Cbor::Array(vec![
            Cbor::Uint(5),
            Cbor::Uint(u64::from(decimal.precision())),
            signed(i64::from(decimal.scale())),
        ]),
        DataType::Number => Cbor::Uint(6),
        DataType::Text => Cbor::Uint(7),
        DataType::MediaType => Cbor::Uint(18),
        DataType::Bytes(binary) => Cbor::Array(vec![
            Cbor::Uint(8),
            match binary {
                BinaryType::Variable => Cbor::Null,
                BinaryType::Fixed(length) => Cbor::Uint(u64::from(length.get())),
            },
        ]),
        DataType::Temporal(temporal) => temporal_type_tree(temporal),
        DataType::Uuid => Cbor::Uint(10),
        DataType::Struct(structure) => {
            Cbor::Array(vec![Cbor::Uint(11), struct_fields_tree(structure)])
        }
        DataType::Map(map) => Cbor::Array(vec![
            Cbor::Uint(12),
            type_tree(map.key()),
            type_tree(map.value()),
        ]),
        DataType::Array(array) => Cbor::Array(vec![
            Cbor::Uint(13),
            type_tree(array.element()),
            array
                .length()
                .map_or(Cbor::Null, |length| Cbor::Uint(u64::from(length.get()))),
        ]),
        DataType::Enum(enumeration) => Cbor::Array(vec![
            Cbor::Uint(14),
            Cbor::Array(
                enumeration
                    .symbols()
                    .iter()
                    .map(|symbol| Cbor::Text(symbol.clone()))
                    .collect(),
            ),
        ]),
        DataType::StructuralUnion(union) => Cbor::Array(vec![
            Cbor::Uint(15),
            Cbor::Array(union.branches().iter().map(type_tree).collect()),
        ]),
        DataType::TaggedUnion(union) => Cbor::Array(vec![
            Cbor::Uint(16),
            Cbor::Array(
                union
                    .cases()
                    .iter()
                    .map(|case| {
                        Cbor::Array(vec![
                            Cbor::Text(case.name().to_string()),
                            struct_fields_tree(case.payload()),
                        ])
                    })
                    .collect(),
            ),
        ]),
        DataType::Variant => Cbor::Uint(17),
    }
}

fn struct_fields_tree(structure: &StructType) -> Cbor {
    Cbor::Array(
        structure
            .fields()
            .iter()
            .map(|field| {
                Cbor::Array(vec![
                    Cbor::Text(field.name().to_string()),
                    type_tree(field.ty()),
                ])
            })
            .collect(),
    )
}

fn temporal_type_tree(temporal: &TemporalType) -> Cbor {
    let time_unit = |unit: TimeUnit| {
        Cbor::Uint(match unit {
            TimeUnit::Second => 1,
            TimeUnit::Millisecond => 2,
            TimeUnit::Microsecond => 3,
            TimeUnit::Nanosecond => 4,
        })
    };
    let mut parts = vec![Cbor::Uint(9)];
    match temporal {
        TemporalType::Date(unit) => {
            parts.push(Cbor::Uint(1));
            parts.push(Cbor::Uint(match unit {
                DateUnit::Day => 1,
                DateUnit::Millisecond => 2,
            }));
        }
        TemporalType::Time(unit) => {
            parts.push(Cbor::Uint(2));
            parts.push(time_unit(*unit));
        }
        TemporalType::Timestamp { unit, zone } => {
            parts.push(Cbor::Uint(3));
            parts.push(time_unit(*unit));
            parts.push(match zone {
                TimeZone::Utc => Cbor::Null,
                TimeZone::Named(name) => Cbor::Text(name.as_str().to_string()),
            });
        }
        TemporalType::TimestampNtz(unit) => {
            parts.push(Cbor::Uint(4));
            parts.push(time_unit(*unit));
        }
        TemporalType::Duration(unit) => {
            parts.push(Cbor::Uint(5));
            parts.push(time_unit(*unit));
        }
        TemporalType::Interval(unit) => {
            parts.push(Cbor::Uint(6));
            parts.push(Cbor::Uint(match unit {
                IntervalUnit::YearMonth => 1,
                IntervalUnit::DayTime => 2,
                IntervalUnit::MonthDayNano => 3,
                IntervalUnit::MonthDayMillisUnsigned => 4,
            }));
        }
    }
    Cbor::Array(parts)
}

pub(crate) fn type_from(tree: &Cbor) -> Result<DataType, TypeDecodeError> {
    match tree {
        Cbor::Uint(1) => Ok(DataType::Null),
        Cbor::Uint(2) => Ok(DataType::Bool),
        Cbor::Uint(6) => Ok(DataType::Number),
        Cbor::Uint(7) => Ok(DataType::Text),
        Cbor::Uint(18) => Ok(DataType::MediaType),
        Cbor::Uint(10) => Ok(DataType::Uuid),
        Cbor::Uint(17) => Ok(DataType::Variant),
        Cbor::Array(parts) => type_parts_from(parts),
        _ => Err(TypeDecodeError::Malformed("data type")),
    }
}

fn type_parts_from(parts: &[Cbor]) -> Result<DataType, TypeDecodeError> {
    let built = match parts {
        [Cbor::Uint(3), Cbor::Uint(signedness), Cbor::Uint(width)] => {
            let width = integer_width_from(*width)?;
            DataType::Integer(match signedness {
                1 => IntegerType::Signed(width),
                2 => IntegerType::Unsigned(width),
                _ => return Err(TypeDecodeError::Malformed("integer signedness")),
            })
        }
        [Cbor::Uint(4), Cbor::Uint(width)] => DataType::Float(match width {
            16 => FloatType::Binary16,
            32 => FloatType::Binary32,
            64 => FloatType::Binary64,
            _ => return Err(TypeDecodeError::Malformed("float width")),
        }),
        [Cbor::Uint(5), Cbor::Uint(precision), scale] => {
            let precision = u32::try_from(*precision)
                .map_err(|_| TypeDecodeError::Malformed("decimal precision"))?;
            let scale = i32::try_from(type_int(scale)?)
                .map_err(|_| TypeDecodeError::Malformed("decimal scale"))?;
            DataType::Decimal(
                DecimalType::new(precision, scale)
                    .map_err(|_| TypeDecodeError::Malformed("decimal type"))?,
            )
        }
        [Cbor::Uint(8), length] => DataType::Bytes(match length {
            Cbor::Null => BinaryType::Variable,
            Cbor::Uint(length) => BinaryType::Fixed(
                ByteLength::new(
                    u32::try_from(*length)
                        .map_err(|_| TypeDecodeError::Malformed("byte length"))?,
                )
                .map_err(|_| TypeDecodeError::Malformed("byte length"))?,
            ),
            _ => return Err(TypeDecodeError::Malformed("byte length")),
        }),
        [Cbor::Uint(9), ..] => DataType::Temporal(temporal_type_from(parts)?),
        [Cbor::Uint(11), fields] => DataType::Struct(struct_fields_from(fields)?),
        [Cbor::Uint(12), key, value] => DataType::Map(
            MapType::new(type_from(key)?, type_from(value)?)
                .map_err(|_| TypeDecodeError::Malformed("map type"))?,
        ),
        [Cbor::Uint(13), element, length] => DataType::Array(match length {
            Cbor::Null => ArrayType::variable(type_from(element)?),
            Cbor::Uint(length) => ArrayType::fixed(
                type_from(element)?,
                u32::try_from(*length).map_err(|_| TypeDecodeError::Malformed("array length"))?,
            )
            .map_err(|_| TypeDecodeError::Malformed("array length"))?,
            _ => return Err(TypeDecodeError::Malformed("array length")),
        }),
        [Cbor::Uint(14), Cbor::Array(symbols)] => DataType::Enum(
            EnumType::new(
                symbols
                    .iter()
                    .map(|symbol| Ok(type_text(symbol, "enum symbol")?.to_string()))
                    .collect::<Result<Vec<_>, TypeDecodeError>>()?,
            )
            .map_err(|_| TypeDecodeError::Malformed("enum type"))?,
        ),
        [Cbor::Uint(15), Cbor::Array(branches)] => DataType::structural_union(
            branches
                .iter()
                .map(type_from)
                .collect::<Result<Vec<_>, _>>()?,
        )
        .map_err(|_| TypeDecodeError::Malformed("structural union"))?,
        [Cbor::Uint(16), Cbor::Array(cases)] => DataType::TaggedUnion(
            TaggedUnionType::new(
                cases
                    .iter()
                    .map(|case| match type_array(case, "tagged-union case")? {
                        [Cbor::Text(name), fields] => {
                            TaggedCase::new(name.clone(), struct_fields_from(fields)?)
                                .map_err(|_| TypeDecodeError::Malformed("tagged-union case"))
                        }
                        _ => Err(TypeDecodeError::Malformed("tagged-union case")),
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            )
            .map_err(|_| TypeDecodeError::Malformed("tagged union"))?,
        ),
        _ => return Err(TypeDecodeError::Malformed("data type")),
    };
    built
        .check_bounds()
        .map_err(|_| TypeDecodeError::Malformed("data type budget"))?;
    Ok(built)
}

fn integer_width_from(width: u64) -> Result<IntegerWidth, TypeDecodeError> {
    match width {
        8 => Ok(IntegerWidth::Bits8),
        16 => Ok(IntegerWidth::Bits16),
        32 => Ok(IntegerWidth::Bits32),
        64 => Ok(IntegerWidth::Bits64),
        _ => Err(TypeDecodeError::Malformed("integer width")),
    }
}

fn struct_fields_from(tree: &Cbor) -> Result<StructType, TypeDecodeError> {
    StructType::new(
        type_array(tree, "struct fields")?
            .iter()
            .map(|field| match type_array(field, "struct field type")? {
                [Cbor::Text(name), ty] => Ok(TypeField::new(name.clone(), type_from(ty)?)),
                _ => Err(TypeDecodeError::Malformed("struct field type")),
            })
            .collect::<Result<Vec<_>, _>>()?,
    )
    .map_err(|_| TypeDecodeError::Malformed("struct type"))
}

fn temporal_type_from(parts: &[Cbor]) -> Result<TemporalType, TypeDecodeError> {
    let time_unit = |code: u64| match code {
        1 => Ok(TimeUnit::Second),
        2 => Ok(TimeUnit::Millisecond),
        3 => Ok(TimeUnit::Microsecond),
        4 => Ok(TimeUnit::Nanosecond),
        _ => Err(TypeDecodeError::Malformed("time unit")),
    };
    match parts {
        [Cbor::Uint(9), Cbor::Uint(1), Cbor::Uint(unit)] => Ok(TemporalType::Date(match unit {
            1 => DateUnit::Day,
            2 => DateUnit::Millisecond,
            _ => return Err(TypeDecodeError::Malformed("date unit")),
        })),
        [Cbor::Uint(9), Cbor::Uint(2), Cbor::Uint(unit)] => {
            Ok(TemporalType::Time(time_unit(*unit)?))
        }
        [Cbor::Uint(9), Cbor::Uint(3), Cbor::Uint(unit), zone] => Ok(TemporalType::Timestamp {
            unit: time_unit(*unit)?,
            zone: match zone {
                Cbor::Null => TimeZone::Utc,
                Cbor::Text(name) => TimeZone::named(name.clone())
                    .map_err(|_| TypeDecodeError::Malformed("timestamp timezone"))?,
                _ => return Err(TypeDecodeError::Malformed("timestamp timezone")),
            },
        }),
        [Cbor::Uint(9), Cbor::Uint(4), Cbor::Uint(unit)] => {
            Ok(TemporalType::TimestampNtz(time_unit(*unit)?))
        }
        [Cbor::Uint(9), Cbor::Uint(5), Cbor::Uint(unit)] => {
            Ok(TemporalType::Duration(time_unit(*unit)?))
        }
        [Cbor::Uint(9), Cbor::Uint(6), Cbor::Uint(unit)] => {
            Ok(TemporalType::Interval(match unit {
                1 => IntervalUnit::YearMonth,
                2 => IntervalUnit::DayTime,
                3 => IntervalUnit::MonthDayNano,
                4 => IntervalUnit::MonthDayMillisUnsigned,
                _ => return Err(TypeDecodeError::Malformed("interval unit")),
            }))
        }
        _ => Err(TypeDecodeError::Malformed("temporal type")),
    }
}

/// A malformed canonical type, independent of any enclosing plan package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TypeDecodeError {
    Malformed(&'static str),
    NotArray(&'static str),
    NotText(&'static str),
}

impl core::fmt::Display for TypeDecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Malformed(what) => f.write_str(what),
            Self::NotArray(what) => write!(f, "{what}: not an array"),
            Self::NotText(what) => write!(f, "{what}: not text"),
        }
    }
}

impl core::error::Error for TypeDecodeError {}

fn type_array<'a>(tree: &'a Cbor, what: &'static str) -> Result<&'a [Cbor], TypeDecodeError> {
    match tree {
        Cbor::Array(items) => Ok(items),
        _ => Err(TypeDecodeError::NotArray(what)),
    }
}

fn type_text<'a>(tree: &'a Cbor, what: &'static str) -> Result<&'a str, TypeDecodeError> {
    match tree {
        Cbor::Text(text) => Ok(text),
        _ => Err(TypeDecodeError::NotText(what)),
    }
}

fn type_int(tree: &Cbor) -> Result<i64, TypeDecodeError> {
    match tree {
        Cbor::Uint(value) => {
            i64::try_from(*value).map_err(|_| TypeDecodeError::Malformed("integer range"))
        }
        Cbor::Negative(value) => i64::try_from(*value)
            .map(|magnitude| -1 - magnitude)
            .map_err(|_| TypeDecodeError::Malformed("integer range")),
        _ => Err(TypeDecodeError::Malformed("integer")),
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
        let Cbor::Array(parts) = self::value(&crate::value::Value::Map(entries)) else {
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
        let Cbor::Array(tagged) = meta_value(&headers) else {
            panic!("metadata map has one tagged form");
        };
        assert_eq!(tagged[0], Cbor::Uint(16));
        let Cbor::Array(entries) = &tagged[1] else {
            panic!("metadata map contains canonical pairs");
        };
        let Cbor::Array(first) = &entries[0] else {
            panic!("metadata map entry is a pair");
        };
        assert_eq!(
            first[0],
            Cbor::Array(alloc::vec![Cbor::Uint(6), Cbor::Text(String::from("b"))]),
            "canonical bytes, rather than Rust string order, own map ordering",
        );
        assert!(matches!(first[1], Cbor::Array(ref value) if value[0] == Cbor::Uint(17)));
    }

    #[test]
    fn the_two_union_families_seal_their_semantic_discriminant() {
        let structural = self::value(&crate::value::Value::StructuralUnion {
            branch: 2,
            value: alloc::boxed::Box::new(crate::value::Value::Text(String::from("value"))),
        });
        let tagged = self::value(&crate::value::Value::TaggedUnion(alloc::boxed::Box::new(
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
            Cbor::Array(ref payload) if payload.first() == Some(&Cbor::Uint(15))
        ));
    }

    #[test]
    fn wide_civil_values_round_trip_and_equal_instants_share_bytes() {
        use crate::time::Timestamp;
        for instant in [
            Timestamp::MIN,
            Timestamp::try_new(-62_135_596_800, 0).unwrap(),
            Timestamp::from_nanoseconds(-1),
            Timestamp::UNIX_EPOCH,
            Timestamp::try_new(253_402_300_799, 999_999_999).unwrap(),
            Timestamp::MAX,
        ] {
            for (value, tag) in [
                (Value::Timestamp(instant), 22),
                (Value::TimestampNtz(instant), 23),
            ] {
                let expected = Cbor::Array(vec![
                    Cbor::Uint(tag),
                    Cbor::Array(vec![
                        signed(instant.seconds()),
                        Cbor::Uint(u64::from(instant.nanoseconds())),
                    ]),
                ]);
                assert_eq!(self::value(&value), expected);
                assert_eq!(decode_value(&expected), Ok(value));
            }
        }
        let microsecond_before_epoch =
            Value::Timestamp(Timestamp::try_new(-1, 999_999_000).unwrap());
        let nanoseconds_before_epoch = Value::Timestamp(Timestamp::from_nanoseconds(-1_000));
        assert_eq!(
            encode(&self::value(&microsecond_before_epoch)),
            encode(&self::value(&nanoseconds_before_epoch))
        );
    }

    #[test]
    fn civil_value_decoding_refuses_retired_tags_and_noncanonical_pairs() {
        for tag in [10, 11] {
            assert!(decode_value(&Cbor::Array(vec![Cbor::Uint(tag), signed(0)])).is_err());
        }
        for tag in [22, 23] {
            for invalid in [
                signed(0),
                Cbor::Array(vec![signed(0)]),
                Cbor::Array(vec![signed(0), signed(0), signed(0)]),
                Cbor::Array(vec![signed(0), signed(-1)]),
                Cbor::Array(vec![signed(0), Cbor::Uint(1_000_000_000)]),
                Cbor::Array(vec![signed(0), Cbor::Uint(u64::MAX)]),
                Cbor::Array(vec![Cbor::Uint(u64::MAX), Cbor::Uint(0)]),
            ] {
                assert!(decode_value(&Cbor::Array(vec![Cbor::Uint(tag), invalid])).is_err());
            }
        }
    }

    #[test]
    fn every_completed_scalar_family_has_one_canonical_value_tag() {
        let values = [
            (crate::value::Value::Date(0), 9),
            (
                crate::value::Value::TimestampNtz(crate::time::Timestamp::UNIX_EPOCH),
                23,
            ),
            (crate::value::Value::Time(0), 12),
            (crate::value::Value::Uuid([0; 16]), 13),
            (
                crate::value::Value::Interval {
                    months: 1,
                    days: 2,
                    nanoseconds: 3,
                },
                21,
            ),
        ];
        for (value, expected_tag) in values {
            let Cbor::Array(tagged) = self::value(&value) else {
                panic!("a tagged value");
            };
            assert_eq!(tagged.first(), Some(&Cbor::Uint(expected_tag)));
        }
    }
}
