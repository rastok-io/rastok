//! The canonical byte discipline, independent of Rastok's tag registry.
//!
//! CBOR is a binary item model: each item starts with a head byte identifying
//! its kind and an argument (a value or length). Our profile uses shortest-form
//! arguments, definite lengths and sorted unsigned map keys. Binary64 has a
//! fixed width and one canonical NaN; `-0.0` remains distinct from `+0.0`.
//! These are the precise choices in [identity.md §3], based on RFC 8949 §4.2.
//!
//! Rastok uses this standard, schema-free substrate so every hashed artifact
//! can be audited without Rastok's code. The reader refuses non-canonical
//! forms: the item ↔ bytes bijection is the stability guarantee behind ids,
//! causal seals, plan hashes and goldens. The codec is handwritten because
//! a third-party library can change its legal encoding choices on an upgrade;
//! each such change would move a seal. [D-2026-08-03-1] records that decision.
//!
//! This module knows no Rastok tags. [`crate::canonical`] maps Rastok's
//! types and values onto these items; one byte discipline serves them all.
//!
//! [identity.md §3]: https://github.com/rastok-io/rastok/blob/main/docs/spec/identity.md#3-the-canonical-record-form
//! [D-2026-08-03-1]: https://github.com/rastok-io/rastok/blob/main/docs/spec/decisions.md#d-2026-08-03-1--canonical-bytes-are-hand-rolled-deterministic-cbor

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// The canonical CBOR subset the tables need. Map keys are the fixed
/// unsigned registry keys, emitted in table order — which is sorted
/// order, satisfying §4.2 at zero runtime cost. `Eq` is real: every
/// variant, floats included, has one state per canonical byte form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// An unsigned integer (major type 0).
    Uint(u64),
    /// A negative integer (major type 1), carried as the wire's own
    /// argument `n` for the value `−1 − n`. The two integer variants
    /// partition the integers exactly as the two major types
    /// partition the bytes: every argument is valid, no constructible
    /// state duplicates a `Uint` form — one state per canonical byte
    /// form, so `decode ∘ encode` is the identity and equality of
    /// values is equality of bytes.
    Negative(u64),
    /// A byte string (major type 2).
    Bytes(Vec<u8>),
    /// A UTF-8 text string (major type 3).
    Text(String),
    /// A definite-length array (major type 4).
    Array(Vec<Value>),
    /// A definite-length map with fixed unsigned keys (major type
    /// 5). The payload is a `BTreeMap`, so sorted unique keys are a
    /// property of the type — an unsorted or duplicated table is not
    /// constructible, and the encoder has no panic path.
    Map(BTreeMap<u64, Value>),
    /// A boolean (major type 7).
    Bool(bool),
    /// Null (major type 7).
    Null,
    /// Always IEEE-754 binary64 — width fixed by the type tag, the
    /// stated exception to shortest-form. The payload is
    /// [`CanonicalDouble`], so NaN is normalized at construction and
    /// equality is bit equality; `-0.0` stays distinct from `+0.0`.
    Double(CanonicalDouble),
}

/// A binary64 held in its canonical form: every NaN normalizes to
/// the one quiet NaN AT CONSTRUCTION, and equality compares the
/// BITS — so equality of values is equality of encoded bytes,
/// `-0.0` stays distinct from `+0.0`, and all NaNs are one value.
/// This is structural equality of the canonical representation; the
/// Rastok expression `==` operator is a different question and untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalDouble(u64);

impl CanonicalDouble {
    /// The one canonical quiet NaN.
    const CANONICAL_NAN: u64 = 0x7ff8_0000_0000_0000;

    /// Canonicalizes a float — any NaN becomes THE NaN.
    #[must_use]
    pub fn new(value: f64) -> Self {
        if value.is_nan() {
            Self(Self::CANONICAL_NAN)
        } else {
            Self(value.to_bits())
        }
    }

    /// The numeric value.
    #[must_use]
    pub fn value(self) -> f64 {
        f64::from_bits(self.0)
    }

    /// The canonical bit pattern — exactly what the encoder writes.
    #[must_use]
    pub fn to_bits(self) -> u64 {
        self.0
    }
}

impl From<f64> for CanonicalDouble {
    fn from(value: f64) -> Self {
        Self::new(value)
    }
}

/// Encodes one canonical value to its deterministic bytes.
#[must_use]
pub fn encode(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write(value, &mut out);
    out
}

fn write(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Uint(n) => head(0, *n, out),
        Value::Negative(n) => head(1, *n, out),
        Value::Bytes(bytes) => {
            head(2, bytes.len() as u64, out);
            out.extend_from_slice(bytes);
        }
        Value::Text(text) => {
            head(3, text.len() as u64, out);
            out.extend_from_slice(text.as_bytes());
        }
        Value::Array(items) => {
            head(4, items.len() as u64, out);
            for item in items {
                write(item, out);
            }
        }
        Value::Map(pairs) => {
            // Sorted unique keys are the BTreeMap's own property —
            // iteration IS table order.
            head(5, pairs.len() as u64, out);
            for (key, item) in pairs {
                head(0, *key, out);
                write(item, out);
            }
        }
        Value::Bool(b) => out.push(if *b { 0xf5 } else { 0xf4 }),
        Value::Null => out.push(0xf6),
        Value::Double(d) => {
            // Canonical by construction — the bits ARE the payload.
            out.push(0xfb);
            out.extend_from_slice(&d.to_bits().to_be_bytes());
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

/// Why the canonical reader refused — the categories are real and
/// preserved by every caller: data that ends early is not the same
/// diagnosis as data that is syntactically outside the subset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// The bytes end before the announced structure does.
    UnexpectedEof,
    /// A declared length or offset does not fit the address space —
    /// refused arithmetically, never by panicking overflow.
    LengthOverflow,
    /// A text string whose payload is not UTF-8.
    InvalidUtf8,
    /// A CBOR head outside the canonical subset (indefinite lengths,
    /// tags, unsupported simple values), named by its leading byte.
    UnsupportedHead(u8),
    /// Bytes that parse structurally but break a table rule.
    Malformed(&'static str),
    /// Bytes that parse, but are not the canonical form — a
    /// non-minimal head, an unsorted or duplicated map key, a
    /// non-canonical NaN. `decode` succeeding MEANS the bytes were
    /// canonical.
    NonCanonical(&'static str),
    /// Nesting beyond the reader's documented budget
    /// ([`MAX_NESTING`]) — refused, never a stack overflow.
    NestingLimitExceeded,
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnexpectedEof => f.write_str("the bytes end before the structure does"),
            Self::LengthOverflow => f.write_str("a declared length exceeds the address space"),
            Self::InvalidUtf8 => f.write_str("a text payload is not valid UTF-8"),
            Self::UnsupportedHead(head) => {
                write!(f, "unsupported CBOR head byte 0x{head:02x}")
            }
            Self::Malformed(detail) => write!(f, "malformed canonical CBOR: {detail}"),
            Self::NonCanonical(detail) => write!(f, "not canonical CBOR: {detail}"),
            Self::NestingLimitExceeded => f.write_str("nesting beyond the reader's budget"),
        }
    }
}

impl core::error::Error for DecodeError {}

/// The canonical reader's nesting budget, independent of expression and type
/// depth. The Cortex-M33 release measurement is 240 bytes per `decode_at`
/// frame: 256 levels require 61,440 bytes for the reader frames alone. The
/// complete package path is measured separately in the embedded qualification.
/// The plan validator enforces the same budget
/// on the writing side (a deeper plan cannot earn its type, and the
/// compiler refuses the source as `E-PLAN-DEPTH`), so everything
/// `encode` accepts reads back; the check here stands for foreign
/// bytes, refusing as [`DecodeError::NestingLimitExceeded`], never
/// a stack overflow.
pub const MAX_NESTING: usize = 256;

/// Decodes one canonical value; returns the tree and the bytes
/// consumed. Only the subset the tables emit is accepted, and the
/// reading is STRICT: success means the bytes were the canonical
/// form (minimal heads, sorted unique map keys, the one canonical
/// NaN). Every refusal is categorized — total over arbitrary input,
/// panic-free by checked arithmetic and a bounded nesting budget.
pub fn decode(bytes: &[u8]) -> Result<(Value, usize), DecodeError> {
    decode_at(bytes, 0)
}

fn decode_at(bytes: &[u8], depth: usize) -> Result<(Value, usize), DecodeError> {
    if depth >= MAX_NESTING {
        return Err(DecodeError::NestingLimitExceeded);
    }
    let first = *bytes.first().ok_or(DecodeError::UnexpectedEof)?;
    let major = first >> 5;
    match (major, first) {
        (_, 0xf4) => Ok((Value::Bool(false), 1)),
        (_, 0xf5) => Ok((Value::Bool(true), 1)),
        (_, 0xf6) => Ok((Value::Null, 1)),
        (_, 0xfb) => {
            let raw: [u8; 8] = bytes
                .get(1..9)
                .ok_or(DecodeError::UnexpectedEof)?
                .try_into()
                .unwrap_or_else(|_| unreachable!("an 8-byte slice"));
            let bits = u64::from_be_bytes(raw);
            let value = f64::from_bits(bits);
            if value.is_nan() && bits != CanonicalDouble::CANONICAL_NAN {
                return Err(DecodeError::NonCanonical("a non-canonical NaN payload"));
            }
            // The bytes were canonical, so construction preserves
            // them exactly.
            Ok((Value::Double(CanonicalDouble::new(value)), 9))
        }
        (0, _) => {
            let (value, used) = argument(bytes)?;
            Ok((Value::Uint(value), used))
        }
        (1, _) => {
            // The argument IS the state — any range policy (the
            // tables bound their integers to i64) belongs to the
            // table layer, not the byte discipline.
            let (value, used) = argument(bytes)?;
            Ok((Value::Negative(value), used))
        }
        (2, _) => {
            let (payload, end) = sized_payload(bytes)?;
            Ok((Value::Bytes(payload.to_vec()), end))
        }
        (3, _) => {
            let (payload, end) = sized_payload(bytes)?;
            let text = String::from_utf8(payload.to_vec()).map_err(|_| DecodeError::InvalidUtf8)?;
            Ok((Value::Text(text), end))
        }
        (4, _) => {
            let (count, mut used) = argument(bytes)?;
            let mut items = Vec::new();
            for _ in 0..count {
                let rest = bytes.get(used..).ok_or(DecodeError::UnexpectedEof)?;
                let (item, item_used) = decode_at(rest, depth + 1)?;
                items.push(item);
                used = used
                    .checked_add(item_used)
                    .ok_or(DecodeError::LengthOverflow)?;
            }
            Ok((Value::Array(items), used))
        }
        (5, _) => {
            let (count, mut used) = argument(bytes)?;
            let mut pairs = BTreeMap::new();
            let mut previous: Option<u64> = None;
            for _ in 0..count {
                let rest = bytes.get(used..).ok_or(DecodeError::UnexpectedEof)?;
                let (key, key_used) = decode_at(rest, depth + 1)?;
                used = used
                    .checked_add(key_used)
                    .ok_or(DecodeError::LengthOverflow)?;
                let Value::Uint(key) = key else {
                    return Err(DecodeError::Malformed(
                        "a map key is not an unsigned integer",
                    ));
                };
                if previous.is_some_and(|last| last >= key) {
                    return Err(DecodeError::NonCanonical(
                        "map keys are not strictly increasing",
                    ));
                }
                previous = Some(key);
                let rest = bytes.get(used..).ok_or(DecodeError::UnexpectedEof)?;
                let (value, value_used) = decode_at(rest, depth + 1)?;
                used = used
                    .checked_add(value_used)
                    .ok_or(DecodeError::LengthOverflow)?;
                pairs.insert(key, value);
            }
            Ok((Value::Map(pairs), used))
        }
        _ => Err(DecodeError::UnsupportedHead(first)),
    }
}

/// A length-prefixed payload (byte and text strings): the declared
/// length is bounded against the address space BEFORE any offset
/// arithmetic — a length near `u64::MAX` refuses as
/// [`DecodeError::LengthOverflow`], never as a panicking add.
fn sized_payload(bytes: &[u8]) -> Result<(&[u8], usize), DecodeError> {
    let (length, used) = argument(bytes)?;
    let length = usize::try_from(length).map_err(|_| DecodeError::LengthOverflow)?;
    let end = used
        .checked_add(length)
        .ok_or(DecodeError::LengthOverflow)?;
    let payload = bytes.get(used..end).ok_or(DecodeError::UnexpectedEof)?;
    Ok((payload, end))
}

/// A head's argument and consumed length — SHORTEST FORM enforced:
/// a value that fits a smaller head refuses as non-canonical.
fn argument(bytes: &[u8]) -> Result<(u64, usize), DecodeError> {
    let first = *bytes.first().ok_or(DecodeError::UnexpectedEof)?;
    let info = first & 0x1f;
    let eof = DecodeError::UnexpectedEof;
    let non_minimal = DecodeError::NonCanonical("a non-minimal head");
    match info {
        0..=23 => Ok((u64::from(info), 1)),
        24 => {
            let value = u64::from(*bytes.get(1).ok_or(eof)?);
            if value < 24 {
                return Err(non_minimal);
            }
            Ok((value, 2))
        }
        25 => {
            let value = u64::from(u16::from_be_bytes(
                bytes
                    .get(1..3)
                    .ok_or(eof)?
                    .try_into()
                    .unwrap_or_else(|_| unreachable!("2 bytes")),
            ));
            if value <= u64::from(u8::MAX) {
                return Err(non_minimal);
            }
            Ok((value, 3))
        }
        26 => {
            let value = u64::from(u32::from_be_bytes(
                bytes
                    .get(1..5)
                    .ok_or(eof)?
                    .try_into()
                    .unwrap_or_else(|_| unreachable!("4 bytes")),
            ));
            if value <= u64::from(u16::MAX) {
                return Err(non_minimal);
            }
            Ok((value, 5))
        }
        27 => {
            let value = u64::from_be_bytes(
                bytes
                    .get(1..9)
                    .ok_or(eof)?
                    .try_into()
                    .unwrap_or_else(|_| unreachable!("8 bytes")),
            );
            if value <= u64::from(u32::MAX) {
                return Err(non_minimal);
            }
            Ok((value, 9))
        }
        _ => Err(DecodeError::UnsupportedHead(first)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    /// RFC 8949 Appendix A vectors, restricted to the subset the
    /// tables use — the encoder is pinned to the standard
    /// independently of any trace.
    #[test]
    fn the_encoder_matches_the_rfc_8949_vectors() {
        let cases: Vec<(Value, &[u8])> = vec![
            (Value::Uint(0), &[0x00]),
            (Value::Uint(23), &[0x17]),
            (Value::Uint(24), &[0x18, 0x18]),
            (Value::Uint(100), &[0x18, 0x64]),
            (Value::Uint(1000), &[0x19, 0x03, 0xe8]),
            (Value::Uint(1_000_000), &[0x1a, 0x00, 0x0f, 0x42, 0x40]),
            (
                Value::Uint(1_000_000_000_000),
                &[0x1b, 0x00, 0x00, 0x00, 0xe8, 0xd4, 0xa5, 0x10, 0x00],
            ),
            (Value::Negative(0), &[0x20]),               // −1
            (Value::Negative(9), &[0x29]),               // −10
            (Value::Negative(99), &[0x38, 0x63]),        // −100
            (Value::Negative(999), &[0x39, 0x03, 0xe7]), // −1000
            (Value::Bool(false), &[0xf4]),
            (Value::Bool(true), &[0xf5]),
            (Value::Null, &[0xf6]),
            (
                Value::Double(CanonicalDouble::new(1.1)),
                &[0xfb, 0x3f, 0xf1, 0x99, 0x99, 0x99, 0x99, 0x99, 0x9a],
            ),
            (
                Value::Double(CanonicalDouble::new(-4.1)),
                &[0xfb, 0xc0, 0x10, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66],
            ),
            (Value::Bytes(vec![]), &[0x40]),
            (
                Value::Bytes(vec![0x01, 0x02, 0x03, 0x04]),
                &[0x44, 0x01, 0x02, 0x03, 0x04],
            ),
            (Value::Text(String::new()), &[0x60]),
            (
                Value::Text(String::from("IETF")),
                &[0x64, 0x49, 0x45, 0x54, 0x46],
            ),
            (
                Value::Text(String::from("\u{20ac}")),
                &[0x63, 0xe2, 0x82, 0xac],
            ),
            (Value::Array(vec![]), &[0x80]),
            (
                Value::Array(vec![Value::Uint(1), Value::Uint(2), Value::Uint(3)]),
                &[0x83, 0x01, 0x02, 0x03],
            ),
            (Value::Map(BTreeMap::new()), &[0xa0]),
            (
                Value::Map(BTreeMap::from([(1, Value::Uint(2)), (3, Value::Uint(4))])),
                &[0xa2, 0x01, 0x02, 0x03, 0x04],
            ),
        ];
        for (value, expected) in cases {
            assert_eq!(encode(&value), expected, "{value:?}");
        }
    }

    /// `decode ∘ encode` is the identity: the type has exactly one
    /// state per canonical byte form, so two values are equal exactly
    /// when their bytes are — NaN included, because equality is bit
    /// equality on the canonicalized payload, not IEEE comparison.
    #[test]
    fn one_state_per_canonical_form() {
        let values = [
            Value::Uint(0),
            Value::Uint(5),
            Value::Uint(u64::MAX),
            Value::Negative(0),
            Value::Negative(4),
            Value::Negative(u64::MAX),
            Value::Bool(true),
            Value::Null,
            Value::Double(CanonicalDouble::new(1.5)),
            Value::Double(CanonicalDouble::new(-0.0)),
            Value::Double(CanonicalDouble::new(f64::NAN)),
            Value::Bytes(vec![1, 2]),
            Value::Text(String::from("x")),
            Value::Array(vec![Value::Negative(4), Value::Uint(5)]),
            Value::Map(BTreeMap::from([(1, Value::Negative(0))])),
        ];
        for value in &values {
            let bytes = encode(value);
            assert_eq!(
                decode(&bytes).unwrap(),
                (value.clone(), bytes.len()),
                "{value:?}"
            );
        }
        // Equality is byte equality, across the whole battery.
        for a in &values {
            for b in &values {
                assert_eq!(
                    a == b,
                    encode(a) == encode(b),
                    "{a:?} vs {b:?}: value equality must BE byte equality"
                );
            }
        }
    }

    /// The two float edge cases, held to the same law: the zeros are
    /// DIFFERENT values with different bytes, and all NaN payloads
    /// are ONE value with one byte form.
    #[test]
    fn zeros_split_and_nans_collapse() {
        let negative_zero = Value::Double(CanonicalDouble::new(-0.0));
        let positive_zero = Value::Double(CanonicalDouble::new(0.0));
        assert_ne!(negative_zero, positive_zero, "-0.0 is its own state");
        assert_ne!(encode(&negative_zero), encode(&positive_zero));

        let payload_nan = CanonicalDouble::new(f64::from_bits(0x7ff8_0000_0000_1234));
        let plain_nan = CanonicalDouble::new(f64::NAN);
        assert_eq!(payload_nan, plain_nan, "every NaN canonicalizes to THE NaN");
        assert_eq!(
            Value::Double(payload_nan),
            Value::Double(plain_nan),
            "equal as values, because equal as bytes"
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
            encode(&Value::Double(CanonicalDouble::new(1.5))),
            [0xfb, 0x3f, 0xf8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        let payload_nan = f64::from_bits(0x7ff8_0000_0000_1234);
        assert_eq!(
            encode(&Value::Double(CanonicalDouble::new(payload_nan))),
            [0xfb, 0x7f, 0xf8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
            "every NaN seals as the canonical quiet NaN"
        );
        assert_eq!(
            encode(&Value::Double(CanonicalDouble::new(-0.0))),
            [0xfb, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
            "-0.0 stays distinct from +0.0"
        );
    }
}
