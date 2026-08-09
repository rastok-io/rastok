//! The typed Rastok projections onto canonical CBOR — the
//! `tagged_value` registry of `identity.md` §3, applied. This module
//! knows the tag numbers; [`crate::canonical_cbor`] knows only the
//! byte discipline. Golden traces and the plan package both seal
//! through here — one registry, everywhere.

use alloc::vec;
use alloc::vec::Vec;

use crate::envelope::Scalar;

// The tree type keeps its historical local name: `Cbor` the tree
// against `Value` the envelope domain — two vocabularies in one
// file need distinct names.
use crate::canonical_cbor::{Value as Cbor, encode};

/// A `tagged_value` for an envelope value: `[type_tag, value]` per
/// the fixed registry.
pub fn value_tagged(value: &crate::envelope::Value) -> Cbor {
    use crate::envelope::Value;
    match value {
        Value::Scalar(Scalar::Bool(v)) => Cbor::Array(vec![Cbor::Uint(1), Cbor::Bool(*v)]),
        Value::Scalar(Scalar::Int(v)) => int_tagged(*v),
        Value::Scalar(Scalar::Double(v)) => {
            Cbor::Array(vec![Cbor::Uint(5), Cbor::Double((*v).into())])
        }
        Value::Scalar(Scalar::Text(v)) => Cbor::Array(vec![Cbor::Uint(8), Cbor::Text(v.clone())]),
        Value::Scalar(Scalar::Bytes(v)) => Cbor::Array(vec![Cbor::Uint(7), Cbor::Bytes(v.clone())]),
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
    }
}

/// A `tagged_value` for a CEL value (state deltas).
pub fn cel_tagged(value: &crate::cel::Value) -> Cbor {
    match value {
        // State writes resolve the laziness witness before the store
        // (runtime `resolve_payload`), so no delta ever carries one;
        // encoding a marker would seal a lie.
        crate::cel::Value::Payload => unreachable!("a payload witness never reaches a seal"),
        crate::cel::Value::Bool(v) => Cbor::Array(vec![Cbor::Uint(1), Cbor::Bool(*v)]),
        crate::cel::Value::Int(v) => int_tagged(*v),
        crate::cel::Value::Uint(v) => Cbor::Array(vec![Cbor::Uint(23), Cbor::Uint(*v)]),
        crate::cel::Value::Double(v) => Cbor::Array(vec![Cbor::Uint(5), Cbor::Double((*v).into())]),
        crate::cel::Value::Text(v) => Cbor::Array(vec![Cbor::Uint(8), Cbor::Text(v.clone())]),
        crate::cel::Value::Bytes(v) => Cbor::Array(vec![Cbor::Uint(7), Cbor::Bytes(v.clone())]),
        crate::cel::Value::Duration(v) => Cbor::Array(vec![Cbor::Uint(16), int_cbor(*v)]),
        crate::cel::Value::Timestamp(v) => Cbor::Array(vec![Cbor::Uint(12), int_cbor(*v)]),
        crate::cel::Value::Null => Cbor::Array(vec![Cbor::Uint(9), Cbor::Null]),
        crate::cel::Value::List(items) => Cbor::Array(vec![
            Cbor::Uint(19),
            Cbor::Array(items.iter().map(cel_tagged).collect()),
        ]),
        crate::cel::Value::Struct(fields) => {
            // The struct's canonical seal — tag 17, sorted pairs —
            // mirrors `value_tagged`'s: one registry, whatever
            // domain carried the value.
            let mut sorted: Vec<_> = fields.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            Cbor::Array(vec![
                Cbor::Uint(17),
                Cbor::Array(
                    sorted
                        .into_iter()
                        .map(|(field, value)| {
                            Cbor::Array(vec![Cbor::Text(field.clone()), cel_tagged(value)])
                        })
                        .collect(),
                ),
            ])
        }
        crate::cel::Value::Map(entries) => {
            let mut pairs: Vec<(Vec<u8>, Cbor)> = entries
                .iter()
                .map(|(key, value)| {
                    let tagged_key = Cbor::Array(vec![Cbor::Uint(8), Cbor::Text(key.clone())]);
                    (
                        encode(&tagged_key),
                        Cbor::Array(vec![tagged_key, cel_tagged(value)]),
                    )
                })
                .collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            Cbor::Array(vec![
                Cbor::Uint(18),
                Cbor::Array(pairs.into_iter().map(|(_, pair)| pair).collect()),
            ])
        }
    }
}

/// A `tagged_value` for writable metadata (flat by type).
pub fn meta_tagged(value: &crate::envelope::MetaValue) -> Cbor {
    match value {
        crate::envelope::MetaValue::Scalar(scalar) => {
            value_tagged(&crate::envelope::Value::Scalar(scalar.clone()))
        }
        crate::envelope::MetaValue::Array(items) => Cbor::Array(vec![
            Cbor::Uint(19),
            Cbor::Array(
                items
                    .iter()
                    .map(|scalar| value_tagged(&crate::envelope::Value::Scalar(scalar.clone())))
                    .collect(),
            ),
        ]),
    }
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
    use crate::envelope::Scalar;
    use alloc::string::String;

    /// Map keys sort by their canonical tagged bytes — CBOR text
    /// encodes its length first, so `"b"` precedes `"aa"`, against
    /// Rust's string order.
    #[test]
    fn map_keys_sort_by_canonical_bytes_not_string_order() {
        let mut entries = std::collections::BTreeMap::new();
        entries.insert(
            String::from("aa"),
            crate::envelope::Value::Scalar(Scalar::Bool(true)),
        );
        entries.insert(
            String::from("b"),
            crate::envelope::Value::Scalar(Scalar::Bool(false)),
        );
        let Cbor::Array(parts) = value_tagged(&crate::envelope::Value::Map(entries)) else {
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
}
