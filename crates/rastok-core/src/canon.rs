//! The canonical byte discipline — deterministic CBOR per the
//! RFC 8949 §4.2 profile of `identity.md` §3: shortest-form
//! integers, definite lengths, fixed unsigned map keys emitted in
//! table order, binary64 held at fixed width with the one canonical
//! NaN. One implementation serves every canonical form in the
//! project: the golden projection (std layer, evidence) and the plan
//! blob (this kernel, transport). The rationale for a hand-rolled
//! codec over a library lives with the golden projection's
//! documentation and in `decisions.md` (D-2026-08-03-1).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::envelope::Scalar;

/// The canonical CBOR subset the tables need. Map keys are the fixed
/// unsigned registry keys, emitted in table order — which is sorted
/// order, satisfying §4.2 at zero runtime cost.
#[derive(Debug, Clone, PartialEq)]
pub enum Cbor {
    /// An unsigned integer (major type 0).
    Uint(u64),
    /// A negative integer (major type 1); non-negatives use `Uint`.
    Int(i64),
    /// A byte string (major type 2).
    Bytes(Vec<u8>),
    /// A UTF-8 text string (major type 3).
    Text(String),
    /// A definite-length array (major type 4).
    Array(Vec<Cbor>),
    /// A definite-length map with fixed unsigned keys, emitted in
    /// table order — which is sorted order (major type 5).
    Map(Vec<(u64, Cbor)>),
    /// A boolean (major type 7).
    Bool(bool),
    /// Null (major type 7).
    Null,
    /// Always IEEE-754 binary64 — width fixed by the type tag, the
    /// stated exception to shortest-form. NaN canonicalizes to the
    /// one quiet NaN; `-0.0` stays distinct from `+0.0`.
    Double(f64),
}

/// Encodes one canonical value to its deterministic bytes.
#[must_use]
pub fn encode(value: &Cbor) -> Vec<u8> {
    let mut out = Vec::new();
    write(value, &mut out);
    out
}

fn write(value: &Cbor, out: &mut Vec<u8>) {
    match value {
        Cbor::Uint(n) => head(0, *n, out),
        Cbor::Int(n) => {
            debug_assert!(*n < 0, "non-negatives encode as Uint");
            let magnitude = u64::try_from(-1 - *n).unwrap_or_else(|_| unreachable!("i64 fits"));
            head(1, magnitude, out);
        }
        Cbor::Bytes(bytes) => {
            head(2, bytes.len() as u64, out);
            out.extend_from_slice(bytes);
        }
        Cbor::Text(text) => {
            head(3, text.len() as u64, out);
            out.extend_from_slice(text.as_bytes());
        }
        Cbor::Array(items) => {
            head(4, items.len() as u64, out);
            for item in items {
                write(item, out);
            }
        }
        Cbor::Map(pairs) => {
            debug_assert!(
                pairs.windows(2).all(|window| window[0].0 < window[1].0),
                "table order is sorted order"
            );
            head(5, pairs.len() as u64, out);
            for (key, item) in pairs {
                head(0, *key, out);
                write(item, out);
            }
        }
        Cbor::Bool(b) => out.push(if *b { 0xf5 } else { 0xf4 }),
        Cbor::Null => out.push(0xf6),
        Cbor::Double(d) => {
            let bits = if d.is_nan() {
                0x7ff8_0000_0000_0000
            } else {
                d.to_bits()
            };
            out.push(0xfb);
            out.extend_from_slice(&bits.to_be_bytes());
        }
    }
}

/// A shortest-form head: major type + argument.
fn head(major: u8, argument: u64, out: &mut Vec<u8>) {
    let major = major << 5;
    if argument < 24 {
        out.push(major | u8::try_from(argument).unwrap_or_else(|_| unreachable!("< 24")));
    } else if argument <= u64::from(u8::MAX) {
        out.push(major | 24);
        out.push(u8::try_from(argument).unwrap_or_else(|_| unreachable!("checked")));
    } else if argument <= u64::from(u16::MAX) {
        out.push(major | 25);
        out.extend_from_slice(
            &u16::try_from(argument)
                .unwrap_or_else(|_| unreachable!("checked"))
                .to_be_bytes(),
        );
    } else if argument <= u64::from(u32::MAX) {
        out.push(major | 26);
        out.extend_from_slice(
            &u32::try_from(argument)
                .unwrap_or_else(|_| unreachable!("checked"))
                .to_be_bytes(),
        );
    } else {
        out.push(major | 27);
        out.extend_from_slice(&argument.to_be_bytes());
    }
}

/// Decodes one canonical value; returns the tree and the bytes
/// consumed. Only the subset the tables emit is accepted.
pub fn decode(bytes: &[u8]) -> Option<(Cbor, usize)> {
    let first = *bytes.first()?;
    let major = first >> 5;
    match (major, first) {
        (_, 0xf4) => Some((Cbor::Bool(false), 1)),
        (_, 0xf5) => Some((Cbor::Bool(true), 1)),
        (_, 0xf6) => Some((Cbor::Null, 1)),
        (_, 0xfb) => {
            let raw: [u8; 8] = bytes.get(1..9)?.try_into().ok()?;
            Some((Cbor::Double(f64::from_bits(u64::from_be_bytes(raw))), 9))
        }
        (0, _) => {
            let (value, used) = argument(bytes)?;
            Some((Cbor::Uint(value), used))
        }
        (1, _) => {
            let (value, used) = argument(bytes)?;
            Some((Cbor::Int(-1 - i64::try_from(value).ok()?), used))
        }
        (2, _) => {
            let (length, used) = argument(bytes)?;
            let length = usize::try_from(length).ok()?;
            let payload = bytes.get(used..used + length)?;
            Some((Cbor::Bytes(payload.to_vec()), used + length))
        }
        (3, _) => {
            let (length, used) = argument(bytes)?;
            let length = usize::try_from(length).ok()?;
            let payload = bytes.get(used..used + length)?;
            Some((
                Cbor::Text(String::from_utf8(payload.to_vec()).ok()?),
                used + length,
            ))
        }
        (4, _) => {
            let (count, mut used) = argument(bytes)?;
            let mut items = Vec::new();
            for _ in 0..count {
                let (item, item_used) = decode(bytes.get(used..)?)?;
                items.push(item);
                used += item_used;
            }
            Some((Cbor::Array(items), used))
        }
        (5, _) => {
            let (count, mut used) = argument(bytes)?;
            let mut pairs = Vec::new();
            for _ in 0..count {
                let (key, key_used) = decode(bytes.get(used..)?)?;
                used += key_used;
                let Cbor::Uint(key) = key else {
                    return None;
                };
                let (value, value_used) = decode(bytes.get(used..)?)?;
                used += value_used;
                pairs.push((key, value));
            }
            Some((Cbor::Map(pairs), used))
        }
        _ => None,
    }
}

/// A shortest-form head's argument and consumed length.
fn argument(bytes: &[u8]) -> Option<(u64, usize)> {
    let info = bytes.first()? & 0x1f;
    match info {
        0..=23 => Some((u64::from(info), 1)),
        24 => Some((u64::from(*bytes.get(1)?), 2)),
        25 => Some((
            u64::from(u16::from_be_bytes(bytes.get(1..3)?.try_into().ok()?)),
            3,
        )),
        26 => Some((
            u64::from(u32::from_be_bytes(bytes.get(1..5)?.try_into().ok()?)),
            5,
        )),
        27 => Some((u64::from_be_bytes(bytes.get(1..9)?.try_into().ok()?), 9)),
        _ => None,
    }
}

/// A `tagged_value` for an envelope value: `[type_tag, value]` per
/// the fixed registry.
pub fn value_tagged(value: &crate::envelope::Value) -> Cbor {
    use crate::envelope::Value;
    match value {
        Value::Scalar(Scalar::Bool(v)) => Cbor::Array(vec![Cbor::Uint(1), Cbor::Bool(*v)]),
        Value::Scalar(Scalar::Int(v)) => int_tagged(*v),
        Value::Scalar(Scalar::Double(v)) => Cbor::Array(vec![Cbor::Uint(5), Cbor::Double(*v)]),
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
        crate::cel::Value::Bool(v) => Cbor::Array(vec![Cbor::Uint(1), Cbor::Bool(*v)]),
        crate::cel::Value::Int(v) => int_tagged(*v),
        crate::cel::Value::Uint(v) => Cbor::Array(vec![Cbor::Uint(3), Cbor::Uint(*v)]),
        crate::cel::Value::Double(v) => Cbor::Array(vec![Cbor::Uint(5), Cbor::Double(*v)]),
        crate::cel::Value::Text(v) => Cbor::Array(vec![Cbor::Uint(8), Cbor::Text(v.clone())]),
        crate::cel::Value::Bytes(v) => Cbor::Array(vec![Cbor::Uint(7), Cbor::Bytes(v.clone())]),
        crate::cel::Value::Duration(v) => Cbor::Array(vec![Cbor::Uint(16), int_cbor(*v)]),
        crate::cel::Value::Timestamp(v) => Cbor::Array(vec![Cbor::Uint(12), int_cbor(*v)]),
        crate::cel::Value::Null => Cbor::Array(vec![Cbor::Uint(9), Cbor::Null]),
        crate::cel::Value::List(items) => Cbor::Array(vec![
            Cbor::Uint(19),
            Cbor::Array(items.iter().map(cel_tagged).collect()),
        ]),
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
        Cbor::Int(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::Scalar;

    /// RFC 8949 Appendix A vectors, restricted to the subset the
    /// tables use — the encoder is pinned to the standard
    /// independently of any trace.
    #[test]
    fn the_encoder_matches_the_rfc_8949_vectors() {
        let cases: Vec<(Cbor, &[u8])> = vec![
            (Cbor::Uint(0), &[0x00]),
            (Cbor::Uint(23), &[0x17]),
            (Cbor::Uint(24), &[0x18, 0x18]),
            (Cbor::Uint(100), &[0x18, 0x64]),
            (Cbor::Uint(1000), &[0x19, 0x03, 0xe8]),
            (Cbor::Uint(1_000_000), &[0x1a, 0x00, 0x0f, 0x42, 0x40]),
            (
                Cbor::Uint(1_000_000_000_000),
                &[0x1b, 0x00, 0x00, 0x00, 0xe8, 0xd4, 0xa5, 0x10, 0x00],
            ),
            (Cbor::Int(-1), &[0x20]),
            (Cbor::Int(-10), &[0x29]),
            (Cbor::Int(-100), &[0x38, 0x63]),
            (Cbor::Int(-1000), &[0x39, 0x03, 0xe7]),
            (Cbor::Bool(false), &[0xf4]),
            (Cbor::Bool(true), &[0xf5]),
            (Cbor::Null, &[0xf6]),
            (
                Cbor::Double(1.1),
                &[0xfb, 0x3f, 0xf1, 0x99, 0x99, 0x99, 0x99, 0x99, 0x9a],
            ),
            (
                Cbor::Double(-4.1),
                &[0xfb, 0xc0, 0x10, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66],
            ),
            (Cbor::Bytes(vec![]), &[0x40]),
            (
                Cbor::Bytes(vec![0x01, 0x02, 0x03, 0x04]),
                &[0x44, 0x01, 0x02, 0x03, 0x04],
            ),
            (Cbor::Text(String::new()), &[0x60]),
            (
                Cbor::Text(String::from("IETF")),
                &[0x64, 0x49, 0x45, 0x54, 0x46],
            ),
            (
                Cbor::Text(String::from("\u{20ac}")),
                &[0x63, 0xe2, 0x82, 0xac],
            ),
            (Cbor::Array(vec![]), &[0x80]),
            (
                Cbor::Array(vec![Cbor::Uint(1), Cbor::Uint(2), Cbor::Uint(3)]),
                &[0x83, 0x01, 0x02, 0x03],
            ),
            (Cbor::Map(vec![]), &[0xa0]),
            (
                Cbor::Map(vec![(1, Cbor::Uint(2)), (3, Cbor::Uint(4))]),
                &[0xa2, 0x01, 0x02, 0x03, 0x04],
            ),
        ];
        for (value, expected) in cases {
            assert_eq!(encode(&value), expected, "{value:?}");
        }
    }

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

    /// The profile's stated deviations from shortest-form: doubles
    /// hold binary64 regardless of value, and NaN is the one
    /// canonical quiet NaN.
    #[test]
    fn floats_hold_their_fixed_width_and_canonical_nan() {
        // 1.5 is exactly representable in binary16; the profile
        // still seals binary64.
        assert_eq!(
            encode(&Cbor::Double(1.5)),
            [0xfb, 0x3f, 0xf8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        let payload_nan = f64::from_bits(0x7ff8_0000_0000_1234);
        assert_eq!(
            encode(&Cbor::Double(payload_nan)),
            [0xfb, 0x7f, 0xf8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
            "every NaN seals as the canonical quiet NaN"
        );
        assert_eq!(
            encode(&Cbor::Double(-0.0)),
            [0xfb, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
            "-0.0 stays distinct from +0.0"
        );
    }
}
