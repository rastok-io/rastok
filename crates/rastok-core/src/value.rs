//! Materialized logical values owned by the Rastok core.
//!
//! Envelopes, expressions, state, codecs and ports share this representation;
//! lazy evaluation markers do not belong here and can never be serialized.
//! A value carries data, not its surrounding schema: widths, temporal units,
//! timezones and collection constraints remain in
//! [`DataType`](crate::datatype::DataType).

use alloc::boxed::Box;
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
    /// Canonical signed-integer carrier; the resolved type constrains width.
    Int(i64),
    /// Canonical unsigned-integer carrier; the resolved type constrains width.
    Uint(u64),
    /// Canonical binary-float carrier. Binary16/32 values promote here exactly;
    /// the resolved type constrains their original width.
    Double(f64),
    /// UTF-8 text.
    Text(String),
    /// Opaque bytes.
    Bytes(Vec<u8>),
    /// Duration in nanoseconds.
    Duration(i64),
    /// UTC instant in nanoseconds since the Unix epoch.
    Timestamp(i64),
    /// Calendar date as whole days since the Unix epoch.
    Date(i64),
    /// Time of day as nanoseconds since midnight.
    Time(i64),
    /// Timezone-naive timestamp as nanoseconds since the Unix epoch.
    TimestampNtz(i64),
    /// Calendar interval in the common month/day/nanosecond value domain.
    /// The surrounding interval type constrains which components may be set.
    Interval {
        /// Whole calendar months.
        months: i64,
        /// Whole calendar days.
        days: i64,
        /// Remaining nanoseconds.
        nanoseconds: i64,
    },
    /// UUID bytes in network order.
    Uuid([u8; 16]),
    /// The null value.
    Null,
    /// Ordered values. A resolved array type enforces homogeneity; a dynamic
    /// `variant` array may contain different value kinds.
    Array(Vec<Value>),
    /// Text-keyed values. This is the canonical representation of
    /// `map<text, V>`, including JSON and Avro objects. The resolved map type,
    /// when present, enforces one value type.
    Map(BTreeMap<String, Value>),
    /// Values keyed by a non-text canonical scalar or enum. The resolved map
    /// type enforces homogeneous key and value types. Entry order is
    /// presentation; equality and identity sort keys by canonical value bytes.
    KeyedMap(Vec<(Value, Value)>),
    /// Named fields in declaration order. Field order is presentation.
    Struct(Vec<(String, Value)>),
    /// An exact decimal: `unscaled × 10^(−scale)`.
    Decimal {
        /// The unscaled coefficient.
        unscaled: i128,
        /// Digits after the point (negative means trailing zeros).
        scale: i32,
    },
    /// One symbol of a closed enum. The surrounding [`DataType`](crate::datatype::DataType)
    /// owns the admitted symbol set.
    Enum(String),
    /// One selected branch of a structural union.
    ///
    /// The index addresses the union's canonical branch order. Conformance
    /// verifies that the index exists and that the nested value inhabits that
    /// exact branch.
    StructuralUnion {
        /// Selected canonical branch index.
        branch: u16,
        /// Value carried by the selected branch.
        value: Box<Value>,
    },
    /// One named case of a tagged union.
    TaggedUnion(Box<TaggedUnionValue>),
}

/// The semantic discriminant and closed payload of one tagged-union value.
///
/// Keeping the pair behind one pointer prevents this uncommon composite from
/// inflating every [`Value`] — and therefore every expression AST node — while
/// preserving a named, inspectable domain object.
#[derive(Debug, Clone)]
pub struct TaggedUnionValue {
    /// Semantic case discriminant.
    pub case: String,
    /// Closed struct payload carried by the case.
    pub payload: Vec<(String, Value)>,
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
            (Self::Date(a), Self::Date(b)) => a == b,
            (Self::Time(a), Self::Time(b)) => a == b,
            (Self::TimestampNtz(a), Self::TimestampNtz(b)) => a == b,
            (
                Self::Interval {
                    months: a_months,
                    days: a_days,
                    nanoseconds: a_nanoseconds,
                },
                Self::Interval {
                    months: b_months,
                    days: b_days,
                    nanoseconds: b_nanoseconds,
                },
            ) => a_months == b_months && a_days == b_days && a_nanoseconds == b_nanoseconds,
            (Self::Uuid(a), Self::Uuid(b)) => a == b,
            (Self::Null, Self::Null) => true,
            (Self::Array(a), Self::Array(b)) => a == b,
            (Self::Map(a), Self::Map(b)) => a == b,
            (Self::KeyedMap(a), Self::KeyedMap(b)) => map_entries_eq(a, b),
            (Self::Struct(a), Self::Struct(b)) => struct_fields_eq(a, b, Value::eq),
            (
                Self::Decimal { unscaled, scale },
                Self::Decimal {
                    unscaled: other_unscaled,
                    scale: other_scale,
                },
            ) => unscaled == other_unscaled && scale == other_scale,
            (Self::Enum(a), Self::Enum(b)) => a == b,
            (
                Self::StructuralUnion {
                    branch: a_branch,
                    value: a_value,
                },
                Self::StructuralUnion {
                    branch: b_branch,
                    value: b_value,
                },
            ) => a_branch == b_branch && a_value == b_value,
            (Self::TaggedUnion(a), Self::TaggedUnion(b)) => {
                a.case == b.case && struct_fields_eq(&a.payload, &b.payload, Value::eq)
            }
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
            Self::KeyedMap(entries) => {
                let mut keys = BTreeSet::new();
                for (key, value) in entries {
                    if !is_non_text_map_key(key) {
                        return Err(ValueError {
                            path: path.clone(),
                            defect: ValueDefect::NonCanonicalMapKey(ValueKind::of(key)),
                        });
                    }
                    // Only scalar/enum keys reach canonicalization. Rejecting
                    // composites first avoids recursively encoding hostile
                    // data merely to render an error path.
                    let label = map_key_label(key);
                    path.push(ValuePathSegment::MapKey(label.clone()));
                    let result = key.validate_at(path);
                    path.pop();
                    result?;
                    if !keys.insert(canonical_value_bytes(key)) {
                        return Err(ValueError {
                            path: path.clone(),
                            defect: ValueDefect::DuplicateMapKey(label),
                        });
                    }
                    path.push(ValuePathSegment::MapValue(label));
                    let result = value.validate_at(path);
                    path.pop();
                    result?;
                }
                Ok(())
            }
            Self::StructuralUnion { branch, value } => {
                path.push(ValuePathSegment::UnionBranch(*branch));
                let result = value.validate_at(path);
                path.pop();
                result
            }
            Self::TaggedUnion(tagged) => {
                if tagged.case.is_empty() {
                    return Err(ValueError {
                        path: path.clone(),
                        defect: ValueDefect::EmptyTaggedCase,
                    });
                }
                let mut names = BTreeSet::new();
                for (name, value) in &tagged.payload {
                    if !names.insert(name) {
                        return Err(ValueError {
                            path: path.clone(),
                            defect: ValueDefect::DuplicateStructField(name.clone()),
                        });
                    }
                    path.push(ValuePathSegment::TaggedField {
                        case: tagged.case.clone(),
                        field: name.clone(),
                    });
                    let result = value.validate_at(path);
                    path.pop();
                    result?;
                }
                Ok(())
            }
            Self::Enum(symbol) if symbol.is_empty() => Err(ValueError {
                path: path.clone(),
                defect: ValueDefect::EmptyEnumSymbol,
            }),
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
    /// A map key, named by its canonical bytes.
    MapKey(String),
    /// A value under a map key, named by the key's canonical bytes.
    MapValue(String),
    /// Value carried by one structural-union branch.
    UnionBranch(u16),
    /// Field of one tagged-union case payload.
    TaggedField {
        /// Case discriminant.
        case: String,
        /// Payload field.
        field: String,
    },
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
    /// Two map keys have the same canonical value.
    DuplicateMapKey(String),
    /// A non-text keyed map used a key outside the canonical scalar/enum
    /// domain, or used text instead of the unique text-map representation.
    NonCanonicalMapKey(ValueKind),
    /// An enum value cannot carry an empty symbol.
    EmptyEnumSymbol,
    /// A tagged value cannot carry an empty case name.
    EmptyTaggedCase,
}

impl fmt::Display for ValueDefect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DecimalOutOfDomain { unscaled, scale } => write!(
                f,
                "decimal({unscaled}, scale {scale}) is outside the decimal128 domain"
            ),
            Self::DuplicateStructField(name) => {
                write!(f, "a struct whose field `{name}` appears twice")
            }
            Self::DuplicateMapKey(key) => {
                write!(f, "a map whose canonical key `{key}` appears twice")
            }
            Self::NonCanonicalMapKey(ValueKind::Text) => {
                f.write_str("a keyed map whose text key must use the text-map representation")
            }
            Self::NonCanonicalMapKey(kind) => {
                write!(f, "a keyed map whose {kind} key is not canonical")
            }
            Self::EmptyEnumSymbol => f.write_str("an enum whose symbol is empty"),
            Self::EmptyTaggedCase => f.write_str("a tagged union whose case name is empty"),
        }
    }
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
                ValuePathSegment::MapKey(key) => write!(f, "<key:{key}>")?,
                ValuePathSegment::MapValue(key) => write!(f, "[{key:?}]")?,
                ValuePathSegment::UnionBranch(branch) => write!(f, "<branch:{branch}>")?,
                ValuePathSegment::TaggedField { case, field } => write!(f, "<{case}>.{field}")?,
            }
        }
        write!(f, " contains {}", self.defect)
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
    /// A calendar date.
    Date,
    /// A time of day.
    Time,
    /// A timezone-naive timestamp.
    TimestampNtz,
    /// A calendar interval.
    Interval,
    /// A UUID.
    Uuid,
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
    /// A closed enum symbol.
    Enum,
    /// A selected structural-union branch.
    StructuralUnion,
    /// A named tagged-union case.
    TaggedUnion,
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
            Value::Date(_) => Self::Date,
            Value::Time(_) => Self::Time,
            Value::TimestampNtz(_) => Self::TimestampNtz,
            Value::Interval { .. } => Self::Interval,
            Value::Uuid(_) => Self::Uuid,
            Value::Null => Self::Null,
            Value::Array(_) => Self::Array,
            Value::Map(_) => Self::Map,
            Value::KeyedMap(_) => Self::Map,
            Value::Struct(_) => Self::Struct,
            Value::Decimal { .. } => Self::Decimal,
            Value::Enum(_) => Self::Enum,
            Value::StructuralUnion { .. } => Self::StructuralUnion,
            Value::TaggedUnion(_) => Self::TaggedUnion,
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
            Self::Date => "date",
            Self::Time => "time",
            Self::TimestampNtz => "timestamp_ntz",
            Self::Interval => "interval",
            Self::Uuid => "uuid",
            Self::Null => "null",
            Self::Array => "array",
            Self::Map => "map",
            Self::Struct => "struct",
            Self::Decimal => "decimal",
            Self::Enum => "enum",
            Self::StructuralUnion => "structural union",
            Self::TaggedUnion => "tagged union",
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

/// Canonical bytes used only to compare and name materialized map keys. The
/// v7 plan package transports this same complete value registry; map-key
/// ordering therefore has one byte authority in plans and journals.
pub(crate) fn canonical_value_bytes(value: &Value) -> Vec<u8> {
    crate::canonical_cbor::encode(&crate::identity::encoding::value_tagged(value))
}

pub(crate) fn map_key_label(value: &Value) -> String {
    use core::fmt::Write as _;

    let bytes = canonical_value_bytes(value);
    let mut label = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(label, "{byte:02x}");
    }
    label
}

fn map_entries_eq(a: &[(Value, Value)], b: &[(Value, Value)]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    fn sorted(entries: &[(Value, Value)]) -> Vec<(&Value, &Value)> {
        let mut refs: Vec<_> = entries.iter().map(|(key, value)| (key, value)).collect();
        refs.sort_by_cached_key(|(key, _)| canonical_value_bytes(key));
        refs
    }
    sorted(a)
        .into_iter()
        .zip(sorted(b))
        .all(|((a_key, a_value), (b_key, b_value))| a_key == b_key && a_value == b_value)
}

fn is_non_text_map_key(value: &Value) -> bool {
    matches!(
        value,
        Value::Bool(_)
            | Value::Int(_)
            | Value::Uint(_)
            | Value::Bytes(_)
            | Value::Duration(_)
            | Value::Timestamp(_)
            | Value::Date(_)
            | Value::Time(_)
            | Value::TimestampNtz(_)
            | Value::Uuid(_)
            | Value::Decimal { .. }
            | Value::Enum(_)
    )
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

    #[test]
    fn keyed_map_order_is_presentation_not_value_identity() {
        let ascending = Value::KeyedMap(alloc::vec![
            (Value::Int(1), Value::Text(String::from("a"))),
            (Value::Int(2), Value::Text(String::from("b"))),
        ]);
        let descending = Value::KeyedMap(alloc::vec![
            (Value::Int(2), Value::Text(String::from("b"))),
            (Value::Int(1), Value::Text(String::from("a"))),
        ]);
        assert_eq!(ascending, descending);
        assert_eq!(
            canonical_value_bytes(&ascending),
            canonical_value_bytes(&descending)
        );
    }

    #[test]
    fn keyed_maps_reject_non_key_values_before_type_judgement() {
        let malformed = Value::KeyedMap(alloc::vec![(
            Value::Array(alloc::vec![Value::Int(1)]),
            Value::Bool(true),
        )]);
        assert!(matches!(
            malformed.validate(),
            Err(ValueError {
                defect: ValueDefect::NonCanonicalMapKey(ValueKind::Array),
                ..
            })
        ));
    }
}
