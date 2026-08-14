//! The typed Rastok projections onto canonical CBOR — the
//! `tagged_value` registry of `identity.md` §3, applied. This module
//! knows the tag numbers; [`crate::canonical_cbor`] knows only the
//! byte discipline. Golden traces and the plan package both seal
//! through here — one registry, everywhere.

use alloc::vec;
use alloc::vec::Vec;

use crate::envelope::Scalar;
use crate::value::Value;

// The tree type keeps its historical local name: `Cbor` the tree
// against `Value` the logical domain — two vocabularies in one
// file need distinct names.
use crate::canonical_cbor::{Value as Cbor, encode};

/// A `tagged_value` for a materialized value: `[type_tag, value]` per
/// the fixed registry.
pub fn value_tagged(value: &Value) -> Cbor {
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
                            Cbor::Array(vec![Cbor::Text(field.clone()), value_tagged(value)])
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
                        Cbor::Array(vec![tagged_key, value_tagged(value)]),
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
            Cbor::Array(items.iter().map(value_tagged).collect()),
        ]),
        Value::Enum(symbol) => Cbor::Array(vec![Cbor::Uint(20), Cbor::Text(symbol.clone())]),
        Value::StructuralUnion { branch, value } => Cbor::Array(vec![
            Cbor::Uint(21),
            Cbor::Array(vec![Cbor::Uint(u64::from(*branch)), value_tagged(value)]),
        ]),
        Value::TaggedUnion(tagged) => Cbor::Array(vec![
            Cbor::Uint(21),
            Cbor::Array(vec![
                Cbor::Text(tagged.case.clone()),
                value_tagged(&Value::Struct(tagged.payload.clone())),
            ]),
        ]),
    }
}

/// A `tagged_value` for writable metadata (flat by type).
pub fn meta_tagged(value: &crate::envelope::MetaValue) -> Cbor {
    match value {
        crate::envelope::MetaValue::Scalar(scalar) => scalar_tagged(scalar),
        crate::envelope::MetaValue::Array(items) => Cbor::Array(vec![
            Cbor::Uint(19),
            Cbor::Array(items.iter().map(scalar_tagged).collect()),
        ]),
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
    use alloc::string::String;

    /// Map keys sort by their canonical tagged bytes — CBOR text
    /// encodes its length first, so `"b"` precedes `"aa"`, against
    /// Rust's string order.
    #[test]
    fn map_keys_sort_by_canonical_bytes_not_string_order() {
        let mut entries = std::collections::BTreeMap::new();
        entries.insert(String::from("aa"), crate::value::Value::Bool(true));
        entries.insert(String::from("b"), crate::value::Value::Bool(false));
        let Cbor::Array(parts) = value_tagged(&crate::value::Value::Map(entries)) else {
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
    fn the_two_union_families_seal_their_semantic_discriminant() {
        let structural = value_tagged(&crate::value::Value::StructuralUnion {
            branch: 2,
            value: alloc::boxed::Box::new(crate::value::Value::Text(String::from("value"))),
        });
        let tagged = value_tagged(&crate::value::Value::TaggedUnion(alloc::boxed::Box::new(
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
}
