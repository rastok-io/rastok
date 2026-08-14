//! Materialized logical values owned by the Rastok core.
//!
//! This is the executable subset of the M1.3 value algebra.  Envelopes,
//! expressions, state, codecs and ports share this representation; lazy
//! evaluation markers do not belong here and can never be serialized.

use alloc::collections::BTreeMap;
use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

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

impl Value {
    /// Verifies the invariants every materialized value must satisfy before it
    /// can enter an envelope, state cell, plan, or journal.
    ///
    /// # Errors
    ///
    /// Returns the first structural defect with its path from the value root.
    pub fn validate(&self) -> Result<(), ValueError> {
        self.validate_at(&mut Vec::new())
    }

    fn validate_at(&self, path: &mut Vec<ValuePathSegment>) -> Result<(), ValueError> {
        match self {
            Self::Decimal { unscaled, scale }
                if !crate::datatype::decimal_in_domain(*unscaled, *scale) =>
            {
                Err(ValueError {
                    path: path.clone(),
                    defect: ValueDefect::DecimalOutOfDomain {
                        unscaled: *unscaled,
                        scale: *scale,
                    },
                })
            }
            Self::Struct(fields) => {
                let mut names = BTreeSet::new();
                for (name, value) in fields {
                    if !names.insert(name) {
                        return Err(ValueError {
                            path: path.clone(),
                            defect: ValueDefect::DuplicateStructField(name.clone()),
                        });
                    }
                    path.push(ValuePathSegment::Field(name.clone()));
                    let result = value.validate_at(path);
                    path.pop();
                    result?;
                }
                Ok(())
            }
            Self::Array(items) => {
                for (index, value) in items.iter().enumerate() {
                    path.push(ValuePathSegment::Index(index));
                    let result = value.validate_at(path);
                    path.pop();
                    result?;
                }
                Ok(())
            }
            Self::Map(entries) => {
                for (key, value) in entries {
                    path.push(ValuePathSegment::MapValue(key.clone()));
                    let result = value.validate_at(path);
                    path.pop();
                    result?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// One step from a materialized value root to a nested value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValuePathSegment {
    /// A struct field.
    Field(String),
    /// An array index.
    Index(usize),
    /// A value under a text map key.
    MapValue(String),
}

/// A structural defect independent of the boundary that observed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueDefect {
    /// An exact decimal outside the canonical decimal128 budget.
    DecimalOutOfDomain {
        /// The exact coefficient.
        unscaled: i128,
        /// The carried scale.
        scale: i32,
    },
    /// Two fields of one struct carry the same semantic name.
    DuplicateStructField(String),
}

/// A malformed materialized value and the path to its first defect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueError {
    /// Path from the value root to the malformed value.
    pub path: Vec<ValuePathSegment>,
    /// The violated value invariant.
    pub defect: ValueDefect,
}

impl fmt::Display for ValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("value")?;
        for segment in &self.path {
            match segment {
                ValuePathSegment::Field(name) => write!(f, ".{name}")?,
                ValuePathSegment::Index(index) => write!(f, "[{index}]")?,
                ValuePathSegment::MapValue(key) => write!(f, "[{key:?}]")?,
            }
        }
        match &self.defect {
            ValueDefect::DecimalOutOfDomain { unscaled, scale } => write!(
                f,
                " contains decimal({unscaled}, scale {scale}) outside the decimal128 domain"
            ),
            ValueDefect::DuplicateStructField(name) => {
                write!(f, " contains a struct whose field `{name}` appears twice")
            }
        }
    }
}

impl core::error::Error for ValueError {}

/// The materialized value category reported by a conformance refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    /// A boolean.
    Bool,
    /// A signed or unsigned integer.
    Number,
    /// A binary64.
    Double,
    /// Text.
    Text,
    /// Bytes.
    Bytes,
    /// A duration.
    Duration,
    /// A timestamp.
    Timestamp,
    /// The null value.
    Null,
    /// An array.
    Array,
    /// A map.
    Map,
    /// A struct.
    Struct,
    /// An exact decimal.
    Decimal,
}

impl ValueKind {
    /// Returns the stable category of `value`.
    #[must_use]
    pub fn of(value: &Value) -> Self {
        match value {
            Value::Bool(_) => Self::Bool,
            Value::Int(_) | Value::Uint(_) => Self::Number,
            Value::Double(_) => Self::Double,
            Value::Text(_) => Self::Text,
            Value::Bytes(_) => Self::Bytes,
            Value::Duration(_) => Self::Duration,
            Value::Timestamp(_) => Self::Timestamp,
            Value::Null => Self::Null,
            Value::Array(_) => Self::Array,
            Value::Map(_) => Self::Map,
            Value::Struct(_) => Self::Struct,
            Value::Decimal { .. } => Self::Decimal,
        }
    }
}

impl fmt::Display for ValueKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Bool => "bool",
            Self::Number => "number",
            Self::Double => "double",
            Self::Text => "text",
            Self::Bytes => "bytes",
            Self::Duration => "duration",
            Self::Timestamp => "timestamp",
            Self::Null => "null",
            Self::Array => "array",
            Self::Map => "map",
            Self::Struct => "struct",
            Self::Decimal => "decimal",
        })
    }
}

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
    use alloc::string::ToString;

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

    #[test]
    fn validation_names_the_nested_value_defect() {
        let malformed = Value::Array(alloc::vec![Value::Struct(alloc::vec![
            (String::from("amount"), Value::Int(1)),
            (String::from("amount"), Value::Int(2)),
        ])]);
        let error = malformed.validate().expect_err("the duplicate refuses");
        assert_eq!(error.path, alloc::vec![ValuePathSegment::Index(0)]);
        assert_eq!(
            error.defect,
            ValueDefect::DuplicateStructField(String::from("amount"))
        );
        assert_eq!(
            error.to_string(),
            "value[0] contains a struct whose field `amount` appears twice"
        );
    }
}
