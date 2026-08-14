//! Logical payload types owned by the Rastok core.
//!
//! Expression engines, ports, state, metadata and codecs consume this
//! vocabulary; none of them defines a parallel type system.  This is the
//! executable subset admitted before the M1.3 categorical type-registry
//! landing.  New logical families extend this authority rather than being
//! added to an expression-specific enum.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::value::{Value, ValueKind, ValuePathSegment};

/// Significant-digit budget of the canonical decimal128 value domain.
pub const DECIMAL_MAX_DIGITS: u32 = 38;
/// Largest carried decimal scale in the canonical decimal128 value domain.
pub const DECIMAL_MAX_SCALE: i32 = 6144;
/// Smallest carried decimal scale in the canonical decimal128 value domain.
pub const DECIMAL_MIN_SCALE: i32 = -6143;

/// Whether an exact coefficient and scale inhabit Rastok's decimal128 value
/// domain. Origin-specific rules such as JSON minimal scale remain at the
/// constructor that owns that origin.
#[must_use]
pub fn decimal_in_domain(unscaled: i128, scale: i32) -> bool {
    (DECIMAL_MIN_SCALE..=DECIMAL_MAX_SCALE).contains(&scale)
        && unscaled.unsigned_abs() < 10_u128.pow(DECIMAL_MAX_DIGITS)
}

/// The currently executable subset of Rastok's logical [`DataType`] tree.
///
/// `Variant` is the explicit dynamic value type; the type algebra deliberately
/// contains no inference-hole or recovery variant.  Some pre-M1.3 compiler
/// paths still collapse an unresolved constraint into `Variant`; the B2
/// migration removes those callers without adding such a case here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataType {
    /// Boolean.
    Bool,
    /// The currently admitted signed-integer domain.
    Int,
    /// The currently admitted unsigned-integer domain.
    Uint,
    /// IEEE-754 binary64.
    Double,
    /// UTF-8 text.
    Text,
    /// Opaque bytes.
    Bytes,
    /// Nanosecond-resolution duration.
    Duration,
    /// UTC instant at nanosecond resolution.
    Timestamp,
    /// The null type.
    Null,
    /// The explicitly dynamic, self-describing value type.
    Variant,
    /// Homogeneous ordered values.
    Array(Box<DataType>),
    /// Text-keyed homogeneous values in the current executable subset.
    Map(Box<DataType>),
}

impl DataType {
    /// Judges one materialized value against this logical type.
    ///
    /// `None` is conformance. A refusal carries the precise nested path and
    /// preserves the semantic reason for callers that render it differently.
    #[must_use]
    pub fn violation(&self, name: &str, value: &Value) -> Option<ConformanceViolation> {
        self.violation_at(name, value, &mut Vec::new())
    }

    /// Whether `value` conforms to this logical type.
    #[must_use]
    pub fn accepts(&self, value: &Value) -> bool {
        self.violation("value", value).is_none()
    }

    fn violation_at(
        &self,
        name: &str,
        value: &Value,
        path: &mut Vec<ValuePathSegment>,
    ) -> Option<ConformanceViolation> {
        match (self, value) {
            (Self::Variant, _) => None,
            // A dynamic decimal conforms to no precise type in the current
            // executable subset. B1's decimal type makes this an ordinary
            // parameterized judgement rather than weakening it to double.
            (_, Value::Decimal { .. }) => {
                refusal(name, path, self, value, ConformanceReason::TypeMismatch)
            }
            (Self::Int, Value::Int(candidate)) => {
                if i32::try_from(*candidate).is_err() {
                    refusal(
                        name,
                        path,
                        self,
                        value,
                        ConformanceReason::IntegerOutOfRange { value: *candidate },
                    )
                } else {
                    None
                }
            }
            (Self::Bool, Value::Bool(_))
            | (Self::Uint, Value::Uint(_))
            | (Self::Text, Value::Text(_))
            | (Self::Bytes, Value::Bytes(_))
            | (Self::Double, Value::Double(_))
            | (Self::Duration, Value::Duration(_))
            | (Self::Timestamp, Value::Timestamp(_))
            | (Self::Null, Value::Null) => None,
            (Self::Array(inner), Value::Array(items)) => {
                for (index, item) in items.iter().enumerate() {
                    path.push(ValuePathSegment::Index(index));
                    let violation = inner.violation_at(name, item, path);
                    path.pop();
                    if violation.is_some() {
                        return violation;
                    }
                }
                None
            }
            (Self::Map(inner), Value::Map(entries)) => {
                for (key, item) in entries {
                    path.push(ValuePathSegment::MapValue(key.clone()));
                    let violation = inner.violation_at(name, item, path);
                    path.pop();
                    if violation.is_some() {
                        return violation;
                    }
                }
                None
            }
            _ => refusal(name, path, self, value, ConformanceReason::TypeMismatch),
        }
    }
}

fn refusal(
    name: &str,
    path: &[ValuePathSegment],
    expected: &DataType,
    value: &Value,
    reason: ConformanceReason,
) -> Option<ConformanceViolation> {
    Some(ConformanceViolation {
        name: String::from(name),
        path: path.to_vec(),
        expected: expected.clone(),
        found: ValueKind::of(value),
        reason,
    })
}

/// Why a materialized value failed its logical type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConformanceReason {
    /// The value category cannot inhabit the declared type.
    TypeMismatch,
    /// A signed integer lies outside the current `int` range.
    IntegerOutOfRange {
        /// The offending value.
        value: i64,
    },
}

/// One type refusal, independent of the runtime boundary that renders it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConformanceViolation {
    /// Declared cell or port name.
    pub name: String,
    /// Path from that root to the rejected nested value.
    pub path: Vec<ValuePathSegment>,
    /// Logical type expected at the rejected path.
    pub expected: DataType,
    /// Materialized value category found there.
    pub found: ValueKind,
    /// Semantic reason for refusal.
    pub reason: ConformanceReason,
}

impl fmt::Display for ConformanceViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}", self.name)?;
        for segment in &self.path {
            match segment {
                ValuePathSegment::Field(name) => write!(f, ".{name}")?,
                ValuePathSegment::Index(index) => write!(f, "[{index}]")?,
                ValuePathSegment::MapValue(key) => write!(f, "[{key:?}]")?,
            }
        }
        match &self.reason {
            ConformanceReason::TypeMismatch => write!(
                f,
                "` is declared {:?}; a {} cannot enter it",
                self.expected, self.found
            ),
            ConformanceReason::IntegerOutOfRange { value } => {
                write!(f, "` is an int; {value} leaves the 32-bit range")
            }
        }
    }
}

impl core::error::Error for ConformanceViolation {}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeMap;
    use alloc::string::ToString;

    #[test]
    fn conformance_reports_the_exact_nested_path() {
        let value = Value::Map(BTreeMap::from([(
            String::from("items"),
            Value::Array(alloc::vec![Value::Int(7), Value::Text(String::from("no"))]),
        )]));
        let ty = DataType::Map(Box::new(DataType::Array(Box::new(DataType::Int))));
        let violation = ty
            .violation("payload", &value)
            .expect("the text does not conform");
        assert_eq!(
            violation.path,
            alloc::vec![
                ValuePathSegment::MapValue(String::from("items")),
                ValuePathSegment::Index(1),
            ]
        );
        assert_eq!(violation.expected, DataType::Int);
        assert_eq!(violation.found, ValueKind::Text);
        assert_eq!(
            violation.to_string(),
            "`payload[\"items\"][1]` is declared Int; a text cannot enter it"
        );
    }

    #[test]
    fn decimal_budget_is_not_owned_by_a_codec() {
        assert!(decimal_in_domain(10_i128.pow(DECIMAL_MAX_DIGITS - 1), 0));
        assert!(!decimal_in_domain(10_i128.pow(DECIMAL_MAX_DIGITS), 0));
        assert!(decimal_in_domain(1, DECIMAL_MIN_SCALE));
        assert!(decimal_in_domain(1, DECIMAL_MAX_SCALE));
        assert!(!decimal_in_domain(1, DECIMAL_MIN_SCALE - 1));
        assert!(!decimal_in_domain(1, DECIMAL_MAX_SCALE + 1));
    }

    #[test]
    fn uint_has_one_type_and_value_conformance_rule() {
        assert!(DataType::Uint.accepts(&Value::Uint(u64::MAX)));
        assert!(!DataType::Uint.accepts(&Value::Int(1)));
    }
}
