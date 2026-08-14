//! Materialized logical values owned by the Rastok core.
//!
//! This is the executable subset of the M1.3 value algebra.  Envelopes,
//! expressions, state, codecs and ports share this representation; lazy
//! evaluation markers do not belong here and can never be serialized.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// A materialized Rastok logical value.
#[derive(Debug, Clone)]
pub enum Value {
    /// Boolean.
    Bool(bool),
    /// 64-bit signed integer in the current executable subset.
    Int(i64),
    /// 64-bit unsigned integer in the current executable subset.
    Uint(u64),
    /// IEEE-754 binary64.
    Double(f64),
    /// UTF-8 text.
    Text(String),
    /// Opaque bytes.
    Bytes(Vec<u8>),
    /// Duration in nanoseconds.
    Duration(i64),
    /// UTC instant in nanoseconds since the Unix epoch.
    Timestamp(i64),
    /// The null value.
    Null,
    /// Homogeneous ordered values.
    Array(Vec<Value>),
    /// Text-keyed values in the current executable subset.
    Map(BTreeMap<String, Value>),
    /// Named fields in declaration order. Field order is presentation.
    Struct(Vec<(String, Value)>),
    /// An exact decimal: `unscaled × 10^(−scale)`.
    Decimal {
        /// The unscaled coefficient.
        unscaled: i128,
        /// Digits after the point (negative means trailing zeros).
        scale: i32,
    },
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Uint(a), Self::Uint(b)) => a == b,
            // Structural equality follows canonical bytes: all NaNs are one
            // value, while negative and positive zero stay distinct.
            (Self::Double(a), Self::Double(b)) => {
                crate::canonical_cbor::CanonicalDouble::new(*a)
                    == crate::canonical_cbor::CanonicalDouble::new(*b)
            }
            (Self::Text(a), Self::Text(b)) => a == b,
            (Self::Bytes(a), Self::Bytes(b)) => a == b,
            (Self::Duration(a), Self::Duration(b)) => a == b,
            (Self::Timestamp(a), Self::Timestamp(b)) => a == b,
            (Self::Null, Self::Null) => true,
            (Self::Array(a), Self::Array(b)) => a == b,
            (Self::Map(a), Self::Map(b)) => a == b,
            (Self::Struct(a), Self::Struct(b)) => struct_fields_eq(a, b, Value::eq),
            (
                Self::Decimal { unscaled, scale },
                Self::Decimal {
                    unscaled: other_unscaled,
                    scale: other_scale,
                },
            ) => unscaled == other_unscaled && scale == other_scale,
            _ => false,
        }
    }
}

impl Eq for Value {}

/// Stable-sort struct fields by name before comparing. Well-formed structs
/// have unique names; keeping the operation total also preserves `Eq` for a
/// malformed value while a boundary prepares its typed refusal.
fn struct_fields_eq(
    a: &[(String, Value)],
    b: &[(String, Value)],
    values: impl Fn(&Value, &Value) -> bool,
) -> bool {
    if a.len() != b.len() {
        return false;
    }
    fn sorted(fields: &[(String, Value)]) -> Vec<&(String, Value)> {
        let mut refs: Vec<&(String, Value)> = fields.iter().collect();
        refs.sort_by(|x, y| x.0.cmp(&y.0));
        refs
    }
    sorted(a)
        .into_iter()
        .zip(sorted(b))
        .all(|(x, y)| x.0 == y.0 && values(&x.1, &y.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structural_equality_agrees_with_canonical_identity() {
        assert_eq!(Value::Double(f64::NAN), Value::Double(-f64::NAN));
        assert_ne!(Value::Double(-0.0), Value::Double(0.0));

        let ab = Value::Struct(alloc::vec![
            (String::from("a"), Value::Int(1)),
            (String::from("b"), Value::Int(2)),
        ]);
        let ba = Value::Struct(alloc::vec![
            (String::from("b"), Value::Int(2)),
            (String::from("a"), Value::Int(1)),
        ]);
        assert_eq!(ab, ba);
    }
}
