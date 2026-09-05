//! Rastok expression compilation and evaluation, over core data types.
//!
//! The compiler hands this module expression text and a [`Signature`]; the
//! runtime adapts state and envelopes onto [`Bindings`].  Logical types are
//! owned by [`crate::datatype`], not by the expression engine. Diagnostics
//! and errors are native types; callers adapt them.
//!
//! The admitted subset covers literals, paths, member and index
//! access, arithmetic, comparisons, logic with short-circuit, the
//! ternary, list and map literals, and the `duration`, `timestamp`,
//! `size`, and context-bound `now` functions. Checked `is`/`as` narrowing is
//! a Rastok extension; macros and optional access remain refused explicitly.
//! `now()` is compiled as the reserved `now` binding: the host signature
//! decides where logical time exists, and evaluation never reads a clock.
//!
//! Map iteration and map value ordering are sorted-key — an evaluator
//! guarantee that golden-trace stability relies on.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::{format, vec};
use core::fmt;
use core::ops::Range;

use crate::datatype::{
    DataType, Discriminator, IntegerType, NarrowingFailure, StructType, TaggedUnionType, TypeField,
};
use crate::value::Value;

/// Names refused explicitly: admitted by full CEL, outside this build.
const UNSUPPORTED_FUNCTIONS: &[&str] = &["has", "exists", "exists_one", "all", "map", "filter"];

/// An evaluator-only value.
///
/// This mirrors [`Value`] only so lazy payload witnesses can occur recursively
/// while evaluating an expression. It is never serialized or stored; every
/// value crossing an evaluator boundary is a core [`Value`].
#[derive(Debug, Clone)]
pub enum EvalValue {
    /// Boolean.
    Bool(bool),
    /// 64-bit signed integer.
    Int(i64),
    /// 64-bit unsigned integer.
    Uint(u64),
    /// IEEE-754 double.
    Double(f64),
    /// UTF-8 text.
    Text(String),
    /// Opaque bytes.
    Bytes(Vec<u8>),
    /// Duration in nanoseconds.
    Duration(i64),
    /// UTC instant in nanoseconds since the Unix epoch.
    Timestamp(i64),
    /// Calendar date as days since the Unix epoch.
    Date(i64),
    /// Time of day as nanoseconds since midnight.
    Time(i64),
    /// Timezone-naive timestamp as nanoseconds since the Unix epoch.
    TimestampNtz(i64),
    /// Calendar interval in canonical components.
    Interval {
        /// Whole months.
        months: i64,
        /// Whole days.
        days: i64,
        /// Remaining nanoseconds.
        nanoseconds: i64,
    },
    /// UUID bytes in network order.
    Uuid([u8; 16]),
    /// The null value.
    Null,
    /// Ordered list; its resolved type, when static, enforces homogeneity.
    List(Vec<EvalValue>),
    /// Text-keyed map; `BTreeMap` keeps iteration sorted by key.
    Map(BTreeMap<String, EvalValue>),
    /// Map whose canonical key type is not text.
    KeyedMap(Vec<(EvalValue, EvalValue)>),
    /// Named fields in declaration order — the LOGICAL twin of
    /// [`Value::Struct`] (canonical tag 17, where a map is
    /// 18): the witness that lets a struct cross the expression evaluator —
    /// bindings, locals, ternaries — without silently becoming a
    /// map. The expression surface cannot construct one; it can only carry, access,
    /// and re-emit what the envelope domain handed it.
    Struct(Vec<(String, EvalValue)>),
    /// An exact decimal — `unscaled × 10^(−scale)` — the dynamic
    /// numeric beyond the integer domain (D-2026-08-10-7): at most
    /// 38 significant digits (Arrow decimal128). DOCUMENT-origin
    /// values carry MINIMAL scale (the reader strips trailing
    /// zeros); `decimal("…")` values carry their DECLARED scale
    /// (D-2026-08-13-1) — both fold an integer-representable
    /// written-scale-≤-0 value to the integer domain, which stays
    /// disjoint. No arithmetic —
    /// `variant` is narrowed before use.
    Decimal {
        /// The unscaled coefficient.
        unscaled: i128,
        /// Digits after the point (negative = trailing zeros).
        scale: i32,
    },
    /// One closed enum symbol.
    Enum(String),
    /// One selected structural-union branch.
    StructuralUnion {
        /// Canonical branch index.
        branch: u16,
        /// Selected value.
        value: Box<EvalValue>,
    },
    /// One named tagged-union case.
    TaggedUnion {
        /// Case discriminant.
        case: String,
        /// Closed struct payload.
        payload: Vec<(String, EvalValue)>,
    },
    /// One whole payload, still unmaterialized. The reference keeps join
    /// inputs distinct: demanding `left.data` must never decode `right.data`,
    /// and carrying either witness preserves that exact envelope's physical
    /// representation.
    Payload(PayloadRef),
}

/// Evaluation-local identity of a lazy payload source.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum PayloadRef {
    /// The ordinary single-input `data` binding (or a join's causal base).
    Input,
    /// A join input selected by its declared port name.
    Named(String),
}

impl PartialEq for EvalValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Uint(a), Self::Uint(b)) => a == b,
            // Canonicalized BITS, not IEEE: structural equality must
            // agree with the canonical bytes, or a plan carrying NaN
            // stops equalling itself across the trip while the two
            // zeros collapse despite different bytes.
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
            (Self::List(a), Self::List(b)) => a == b,
            (Self::Map(a), Self::Map(b)) => a == b,
            (Self::KeyedMap(a), Self::KeyedMap(b)) => keyed_map_eq(a, b),
            // Fields compare in SORTED order — by name, like the
            // canonical seal, so order is presentation. Sorting
            // (stable) instead of name-lookup keeps `Eq`'s contract
            // total: a malformed duplicate-name struct — refused at
            // every door, but representable — still equals itself.
            (Self::Struct(a), Self::Struct(b)) => struct_fields_eq(a, b, EvalValue::eq),
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
            (
                Self::TaggedUnion {
                    case: a_case,
                    payload: a_payload,
                },
                Self::TaggedUnion {
                    case: b_case,
                    payload: b_payload,
                },
            ) => a_case == b_case && struct_fields_eq(a_payload, b_payload, EvalValue::eq),
            (Self::Payload(a), Self::Payload(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for EvalValue {}

impl From<Value> for EvalValue {
    fn from(value: Value) -> Self {
        match value {
            Value::Bool(value) => Self::Bool(value),
            Value::Int(value) => Self::Int(value),
            Value::Uint(value) => Self::Uint(value),
            Value::Double(value) => Self::Double(value),
            Value::Text(value) => Self::Text(value),
            Value::Bytes(value) => Self::Bytes(value),
            Value::Duration(value) => Self::Duration(value),
            Value::Timestamp(value) => Self::Timestamp(value),
            Value::Date(value) => Self::Date(value),
            Value::Time(value) => Self::Time(value),
            Value::TimestampNtz(value) => Self::TimestampNtz(value),
            Value::Interval {
                months,
                days,
                nanoseconds,
            } => Self::Interval {
                months,
                days,
                nanoseconds,
            },
            Value::Uuid(value) => Self::Uuid(value),
            Value::Null => Self::Null,
            Value::Array(items) => Self::List(items.into_iter().map(Self::from).collect()),
            Value::Map(entries) => Self::Map(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, Self::from(value)))
                    .collect(),
            ),
            Value::KeyedMap(entries) => Self::KeyedMap(
                entries
                    .into_iter()
                    .map(|(key, value)| (Self::from(key), Self::from(value)))
                    .collect(),
            ),
            Value::Struct(fields) => Self::Struct(
                fields
                    .into_iter()
                    .map(|(name, value)| (name, Self::from(value)))
                    .collect(),
            ),
            Value::Decimal { unscaled, scale } => Self::Decimal { unscaled, scale },
            Value::Enum(symbol) => Self::Enum(symbol),
            Value::StructuralUnion { branch, value } => Self::StructuralUnion {
                branch,
                value: Box::new(Self::from(*value)),
            },
            Value::TaggedUnion(tagged) => Self::TaggedUnion {
                case: tagged.case,
                payload: tagged
                    .payload
                    .into_iter()
                    .map(|(name, value)| (name, Self::from(value)))
                    .collect(),
            },
        }
    }
}

impl EvalValue {
    fn kind_name(&self) -> &'static str {
        match self {
            Self::Bool(_) => "bool",
            Self::Int(_) | Self::Uint(_) => "integer",
            Self::Double(_) => "double",
            Self::Text(_) => "text",
            Self::Bytes(_) => "bytes",
            Self::Duration(_) => "duration",
            Self::Timestamp(_) => "timestamp",
            Self::Date(_) => "date",
            Self::Time(_) => "time",
            Self::TimestampNtz(_) => "timestamp_ntz",
            Self::Interval { .. } => "interval",
            Self::Uuid(_) => "uuid",
            Self::Null => "null",
            Self::List(_) => "list",
            Self::Map(_) | Self::KeyedMap(_) => "map",
            Self::Struct(_) => "struct",
            Self::Decimal { .. } => "decimal",
            Self::Enum(_) => "enum",
            Self::StructuralUnion { .. } => "structural union",
            Self::TaggedUnion { .. } => "tagged union",
            Self::Payload(_) => "unmaterialized payload",
        }
    }

    /// Converts an evaluator result into the materialized value algebra.
    ///
    /// # Errors
    /// An unresolved payload witness is evaluator state and cannot cross the
    /// materialization boundary.
    pub fn into_value(self) -> Result<Value, Error> {
        Ok(match self {
            Self::Bool(value) => Value::Bool(value),
            Self::Int(value) => Value::Int(value),
            Self::Uint(value) => Value::Uint(value),
            Self::Double(value) => Value::Double(value),
            Self::Text(value) => Value::Text(value),
            Self::Bytes(value) => Value::Bytes(value),
            Self::Duration(value) => Value::Duration(value),
            Self::Timestamp(value) => Value::Timestamp(value),
            Self::Date(value) => Value::Date(value),
            Self::Time(value) => Value::Time(value),
            Self::TimestampNtz(value) => Value::TimestampNtz(value),
            Self::Interval {
                months,
                days,
                nanoseconds,
            } => Value::Interval {
                months,
                days,
                nanoseconds,
            },
            Self::Uuid(value) => Value::Uuid(value),
            Self::Null => Value::Null,
            Self::List(items) => Value::Array(
                items
                    .into_iter()
                    .map(Self::into_value)
                    .collect::<Result<_, _>>()?,
            ),
            Self::Map(entries) => Value::Map(
                entries
                    .into_iter()
                    .map(|(key, value)| Ok((key, value.into_value()?)))
                    .collect::<Result<_, Error>>()?,
            ),
            Self::KeyedMap(entries) => Value::KeyedMap(
                entries
                    .into_iter()
                    .map(|(key, value)| Ok((key.into_value()?, value.into_value()?)))
                    .collect::<Result<_, Error>>()?,
            ),
            Self::Struct(fields) => Value::Struct(
                fields
                    .into_iter()
                    .map(|(name, value)| Ok((name, value.into_value()?)))
                    .collect::<Result<_, Error>>()?,
            ),
            Self::Decimal { unscaled, scale } => Value::Decimal { unscaled, scale },
            Self::Enum(symbol) => Value::Enum(symbol),
            Self::StructuralUnion { branch, value } => Value::StructuralUnion {
                branch,
                value: Box::new(value.into_value()?),
            },
            Self::TaggedUnion { case, payload } => {
                Value::TaggedUnion(Box::new(crate::value::TaggedUnionValue {
                    case,
                    payload: payload
                        .into_iter()
                        .map(|(name, value)| Ok((name, value.into_value()?)))
                        .collect::<Result<_, Error>>()?,
                }))
            }
            Self::Payload(_) => {
                return Err(fault(String::from(
                    "an unresolved payload cannot leave expression evaluation",
                )));
            }
        })
    }
}

/// Sorted-field struct comparison, shared by structural equality and
/// the expression `==` operator: stable-sort both sides by name, compare
/// positionally. For well-formed structs (unique names — the door
/// invariant) this IS by-name equality and agrees with the canonical
/// bytes; for malformed values it stays reflexive, symmetric, and
/// transitive, which `Eq` promises unconditionally.
fn struct_fields_eq(
    a: &[(String, EvalValue)],
    b: &[(String, EvalValue)],
    values: impl Fn(&EvalValue, &EvalValue) -> bool,
) -> bool {
    if a.len() != b.len() {
        return false;
    }
    fn sorted(fields: &[(String, EvalValue)]) -> alloc::vec::Vec<&(String, EvalValue)> {
        let mut refs: alloc::vec::Vec<&(String, EvalValue)> = fields.iter().collect();
        refs.sort_by(|x, y| x.0.cmp(&y.0));
        refs
    }
    sorted(a)
        .into_iter()
        .zip(sorted(b))
        .all(|(x, y)| x.0 == y.0 && values(&x.1, &y.1))
}

fn keyed_map_eq(a: &[(EvalValue, EvalValue)], b: &[(EvalValue, EvalValue)]) -> bool {
    keyed_map_eq_by(a, b, EvalValue::eq)
}

fn keyed_map_eq_by(
    a: &[(EvalValue, EvalValue)],
    b: &[(EvalValue, EvalValue)],
    values: impl Fn(&EvalValue, &EvalValue) -> bool + Copy,
) -> bool {
    if a.len() != b.len() {
        return false;
    }
    fn canonical(entries: &[(EvalValue, EvalValue)]) -> Option<BTreeMap<Vec<u8>, &EvalValue>> {
        let mut keyed = BTreeMap::new();
        for (key, value) in entries {
            let key = key.clone().into_value().ok()?;
            if keyed
                .insert(crate::value::canonical_value_bytes(&key), value)
                .is_some()
            {
                return None;
            }
        }
        Some(keyed)
    }
    if let (Some(a), Some(b)) = (canonical(a), canonical(b)) {
        return a
            .into_iter()
            .zip(b)
            .all(|((a_key, a_value), (b_key, b_value))| {
                a_key == b_key && values(a_value, b_value)
            });
    }

    // `Payload` is the only evaluator witness without a core value and cannot
    // inhabit a materialized map key. Keep `Eq` total even for that transient,
    // malformed state instead of assigning it a fake canonical byte string.
    let mut matched = alloc::vec![false; b.len()];
    a.iter().all(|(a_key, a_value)| {
        let found = b.iter().enumerate().position(|(index, (b_key, b_value))| {
            !matched[index] && a_key == b_key && values(a_value, b_value)
        });
        if let Some(index) = found {
            matched[index] = true;
            true
        } else {
            false
        }
    })
}

/// Names and types visible to an expression. Keys may be dotted
/// (`data.text`) for pre-resolved leaves, or bare roots. The
/// A handler signature may offer the reserved `now` binding. The expression
/// parser accepts it only through the function spelling `now()`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Signature {
    names: BTreeMap<String, DataType>,
    types: BTreeMap<String, NarrowType>,
    /// Exact roots whose descendants belong to Rastok's statically sealed
    /// metadata grid. `meta` itself is implicit; join aliases such as
    /// `left.meta` are declared here so `variant` cannot become a general
    /// member-access escape hatch.
    metadata_roots: BTreeSet<String>,
}

impl Signature {
    /// Starts an empty signature.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares a visible name, dotted or bare.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, ty: DataType) -> Self {
        self.names.insert(name.into(), ty);
        self
    }

    /// Declares one exact metadata projection root. Its leaves are verified
    /// by the host compiler against the sealed sys/app/ext schema after the
    /// full unit has been checked.
    #[must_use]
    pub fn with_metadata(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        self.names.insert(name.clone(), DataType::Variant);
        self.metadata_roots.insert(name);
        self
    }

    /// Whether an exact dotted binding is a metadata projection root.
    #[must_use]
    pub fn is_metadata_root(&self, name: &str) -> bool {
        name == "meta" || self.metadata_roots.contains(name)
    }

    /// The type of an exact name, if declared.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&DataType> {
        self.names.get(name)
    }

    /// Returns the signature that holds when `predicate` evaluated to true.
    ///
    /// The operation is deliberately syntactic and conservative: a positive
    /// `is` test narrows its path, and both sides of `&&` hold on the true
    /// branch. Negation, disjunction and arbitrary boolean equivalence do not
    /// manufacture facts.
    #[must_use]
    pub fn narrowed_by(&self, predicate: &Expr) -> Self {
        let mut narrowed = self.clone();
        narrowed.apply_true_facts(predicate);
        narrowed
    }

    fn apply_true_facts(&mut self, predicate: &Expr) {
        match predicate {
            Expr::Is { expr, target } => {
                if let Expr::Path(segments) = expr.as_ref() {
                    let ty = match target.as_ref() {
                        NarrowTarget::Type(target) => target.ty().clone(),
                        NarrowTarget::Case { payload, .. } => (**payload).clone(),
                    };
                    self.names.insert(segments.join("."), ty);
                }
            }
            Expr::Binary {
                op: BinaryOp::And,
                left,
                right,
            } => {
                self.apply_true_facts(left);
                self.apply_true_facts(right);
            }
            _ => {}
        }
    }

    /// Declares one named narrowing target. Alias spelling is compile-time
    /// only; expression trees carry the resolved logical type.
    #[must_use]
    pub fn with_type(mut self, name: impl Into<String>, ty: DataType) -> Self {
        self.types.insert(name.into(), NarrowType::new(ty));
        self
    }

    /// Declares a tagged-union target whose untagged variant projection uses
    /// a boundary-supplied discriminator.
    pub fn with_discriminated_type(
        mut self,
        name: impl Into<String>,
        ty: DataType,
        discriminator: Discriminator,
    ) -> Result<Self, NarrowTypeError> {
        self.types
            .insert(name.into(), NarrowType::discriminated(ty, discriminator)?);
        Ok(self)
    }

    fn type_named(&self, name: &str) -> Option<NarrowType> {
        self.types
            .get(name)
            .cloned()
            .or_else(|| builtin_type(name).map(NarrowType::new))
    }
}

/// One resolved type target carried by `is` or `as`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NarrowType {
    ty: DataType,
    discriminator: Option<Discriminator>,
}

impl NarrowType {
    /// A target requiring no external discriminator.
    #[must_use]
    pub fn new(ty: DataType) -> Self {
        Self {
            ty,
            discriminator: None,
        }
    }

    /// A target with its resolved boundary discriminator.
    pub fn discriminated(
        ty: DataType,
        discriminator: Discriminator,
    ) -> Result<Self, NarrowTypeError> {
        let DataType::TaggedUnion(union) = &ty else {
            return Err(NarrowTypeError::DiscriminatorOnNonTaggedType(ty));
        };
        if let Some(case) = discriminator.mapping().values().find(|case| {
            !union
                .cases()
                .iter()
                .any(|candidate| candidate.name() == case.as_str())
        }) {
            return Err(NarrowTypeError::UnknownMappedCase(case.clone()));
        }
        Ok(Self {
            ty,
            discriminator: Some(discriminator),
        })
    }

    /// Logical target.
    #[must_use]
    pub const fn ty(&self) -> &DataType {
        &self.ty
    }

    /// Optional external-to-tagged projection rule.
    #[must_use]
    pub const fn discriminator(&self) -> Option<&Discriminator> {
        self.discriminator.as_ref()
    }
}

/// Invalid declaration of a named expression narrowing target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NarrowTypeError {
    /// Only a tagged union can consume an external discriminator.
    DiscriminatorOnNonTaggedType(DataType),
    /// A static external mapping names no case in the target union.
    UnknownMappedCase(String),
}

impl fmt::Display for NarrowTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DiscriminatorOnNonTaggedType(ty) => {
                write!(f, "a discriminator cannot be attached to {ty}")
            }
            Self::UnknownMappedCase(case) => {
                write!(f, "a discriminator mapping names undeclared case `{case}`")
            }
        }
    }
}

impl core::error::Error for NarrowTypeError {}

/// Resolved target of a narrowing operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NarrowTarget {
    /// A logical type, possibly with a boundary discriminator.
    Type(Box<NarrowType>),
    /// One named case of the operand's tagged-union type.
    Case {
        /// Semantic discriminant.
        name: String,
        /// Resolved closed struct payload exposed after a successful test.
        payload: Box<DataType>,
    },
}

fn builtin_type(name: &str) -> Option<DataType> {
    Some(match name {
        "null" => DataType::Null,
        "bool" => DataType::Bool,
        "int" => DataType::INT,
        "long" => DataType::LONG,
        "uint" => DataType::UINT,
        "float" => DataType::FLOAT,
        "double" => DataType::DOUBLE,
        "number" => DataType::Number,
        "text" => DataType::Text,
        "media_type" => DataType::MediaType,
        "bytes" => DataType::BYTES,
        "duration" => DataType::DURATION,
        "timestamp" => DataType::TIMESTAMP,
        "variant" => DataType::Variant,
        _ => return None,
    })
}

/// Bridge exposing Rastok state to the expression evaluator. Lookup mirrors compilation:
/// the full dotted path is offered first, then the bare root.
pub trait Bindings {
    /// The value bound to `name`, if any.
    fn get(&self, name: &str) -> Option<EvalValue>;

    /// The whole payload behind a [`EvalValue::Payload`] witness,
    /// materialized. The default has no payload provenance: a witness
    /// reaching an environment that cannot honor it is a fault, not
    /// a silent value.
    ///
    /// # Errors
    /// When this environment carries no payload.
    fn payload(&self, reference: &PayloadRef) -> Result<EvalValue, Error> {
        Err(fault(format!(
            "payload {reference:?} is not available in this environment"
        )))
    }

    /// The bytes behind an [`Expr::Asset`] reference. The default
    /// has no asset table.
    ///
    /// # Errors
    /// When this environment carries no assets.
    fn asset(&self, index: u32) -> Result<EvalValue, Error> {
        Err(fault(format!("asset {index} is not available here")))
    }
}

impl Bindings for BTreeMap<String, EvalValue> {
    fn get(&self, name: &str) -> Option<EvalValue> {
        BTreeMap::get(self, name).cloned()
    }
}

/// What a compiler diagnostic is about, machine-readably. Messages
/// are never stable; kinds are — callers key on them, never on text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticKind {
    /// Any fault without a dedicated kind yet.
    General,
    /// A static map literal carrying the same key twice.
    DuplicateMapKey,
    /// A malformed build-time `asset(…)` call. The source compiler adapts
    /// this to the asset diagnostic family rather than a generic expression
    /// fault.
    AssetCall,
    /// An expression nesting beyond the format budget — refused at
    /// the gate, before any recursive pass rides the machine stack.
    /// Carries the measured depth: the STRUCTURED payload the
    /// registered diagnostic emits, never the human sentence.
    TooDeep {
        /// The nesting depth reached when the gate refused.
        depth: usize,
    },
}

/// Native expression compiler diagnostic; the caller adapts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// The stable machine-readable discriminant.
    pub kind: DiagnosticKind,
    /// What is wrong.
    pub message: String,
    /// Byte range inside the compiled expression text.
    pub range: Range<usize>,
}

/// Expression evaluation error — value errors only; static type errors are
/// rejected at compilation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// Stable failure family.
    pub kind: ErrorKind,
    /// What failed.
    pub message: String,
}

/// Stable expression failure family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// Ordinary evaluation failure.
    Evaluation,
    /// Checked `as`/`is` narrowing failure that is not a simple mismatch.
    Narrowing(NarrowingFailure),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl core::error::Error for Error {}

/// Canonical typed expression tree — one form for hashing, planning, and
/// evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct Tree {
    /// The expression.
    pub root: Expr,
    /// The expression's static type.
    pub ty: DataType,
}

/// One expression node.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A literal value.
    Literal(Value),
    /// A build-time asset by table index (D-2026-08-10-5): evaluates
    /// to the blob's bytes through [`Bindings::asset`]; the HASH
    /// seals the content digest, never the index or the path.
    Asset(u32),
    /// A dotted access path, resolved through bindings at evaluation.
    Path(Vec<String>),
    /// Unary operation.
    Unary {
        /// The operator.
        op: UnaryOp,
        /// The operand.
        expr: Box<Expr>,
    },
    /// Binary operation.
    Binary {
        /// The operator.
        op: BinaryOp,
        /// Left operand.
        left: Box<Expr>,
        /// Right operand.
        right: Box<Expr>,
    },
    /// `cond ? then : otherwise`
    Ternary {
        /// The condition.
        cond: Box<Expr>,
        /// Expression selected when true.
        then: Box<Expr>,
        /// Expression selected when false.
        otherwise: Box<Expr>,
    },
    /// Index access `base[index]`.
    Index {
        /// The indexed collection.
        base: Box<Expr>,
        /// The index or key.
        index: Box<Expr>,
    },
    /// Registered function call.
    Call {
        /// The function.
        function: Function,
        /// The arguments.
        args: Vec<Expr>,
    },
    /// List literal.
    List(Vec<Expr>),
    /// Map literal with text keys.
    MapLit(Vec<(String, Expr)>),
    /// A closed struct constructed by the host value grammar. Fields are
    /// canonicalized by UTF-8 name before this node enters HIR.
    Struct(Vec<(String, Expr)>),
    /// A named case of one resolved tagged union. Carrying the union here
    /// makes a forged tree independently checkable; evaluation uses only the
    /// case and payload after validation.
    Tagged {
        /// The exact resolved union whose case is constructed.
        union: Box<TaggedUnionType>,
        /// Semantic case name.
        case: String,
        /// Closed payload fields in canonical name order.
        fields: Vec<(String, Expr)>,
    },
    /// Checked type or tagged-case predicate.
    Is {
        /// Tested expression.
        expr: Box<Expr>,
        /// Resolved target.
        target: Box<NarrowTarget>,
    },
    /// Checked conversion to a type or tagged-case payload.
    As {
        /// Converted expression.
        expr: Box<Expr>,
        /// Resolved target.
        target: Box<NarrowTarget>,
    },
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// Logical negation.
    Not,
    /// Arithmetic negation.
    Neg,
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Rem,
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `&&`, short-circuit.
    And,
    /// `||`, short-circuit.
    Or,
}

/// The registered function vocabulary of the subset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Function {
    /// `asset("path", content_type?)` — compiler-bound build-time bytes.
    /// The expression parser recognizes the ordinary call; the source compiler
    /// must replace it with [`Expr::Asset`] before a plan can validate.
    Asset,
    /// `duration("2h30m")` — CEL/Go format; no `min`, `d`, or `w`.
    Duration,
    /// `timestamp("2026-01-05T00:00:00Z")` — RFC 3339, UTC only.
    Timestamp,
    /// `size(text | bytes | list | map)`
    Size,
    /// `decimal("1.50")` — Rastok's exact-number constructor: a
    /// standard JSON number at its WRITTEN scale (D-2026-08-13-1);
    /// the integer fold applies to the written scale.
    Decimal,
    /// `media_type("application/json")` — canonical open Internet media
    /// type; ranges such as `*/*` are deliberately a different future type.
    MediaType,
}

/// Compiles expression text against a signature into a typed tree.
///
/// # Errors
///
/// Returns every diagnostic found; an erroneous expression never
/// produces a tree.
pub fn compile(text: &str, signature: &Signature) -> Result<Tree, Vec<Diagnostic>> {
    let tokens = lex(text)?;
    let mut parser = ExprParser {
        tokens: &tokens,
        pos: 0,
        text,
        signature,
        depth: 0,
    };
    let root = parser.ternary().map_err(|diagnostic| vec![diagnostic])?;
    // The depth gate, FIRST — before the trailing-input check drops
    // the tree and before the type-checker, the hasher, or the
    // lowering recurse over it on the machine stack. The parser's own
    // recursion is guarded above, but a left-fold chain (`1 + 1 + …`)
    // builds depth with no parser recursion at all; it is caught
    // here, iteratively. Structural depth beyond the reader's budget
    // can never serialize anyway — every level costs at least one
    // CBOR level — so the refusal is the same named diagnostic, and
    // the refused tree is dismantled iteratively, never dropped
    // through a recursive `Drop`.
    let depth = structural_depth(&root);
    if depth > crate::canonical_cbor::MAX_NESTING {
        let range = 0..text.len();
        dismantle(root);
        return Err(vec![Diagnostic {
            kind: DiagnosticKind::TooDeep { depth },
            message: String::from("the expression nests deeper than the format budget"),
            range,
        }]);
    }
    if let Some(token) = parser.tokens.get(parser.pos) {
        return Err(vec![Diagnostic {
            kind: DiagnosticKind::General,
            message: String::from("unexpected trailing input"),
            range: token.range.clone(),
        }]);
    }
    let ty = check(&root, signature, text).map_err(|diagnostic| vec![diagnostic])?;
    Ok(Tree { root, ty })
}

/// Recognizes the two exact boot-reference forms through the ordinary
/// expression lexer. This is deliberately a root-only recognizer: boot facts
/// are references, not evaluator functions, and cannot be hidden in a
/// calculation.
///
/// # Errors
///
/// A spelling beginning with `env(` or `secret(` must contain exactly one
/// string argument and no surrounding expression. Lexical diagnostics retain
/// their ordinary ranges.
pub fn parse_boot_reference(
    text: &str,
) -> Result<Option<crate::deployment::BootReference>, Vec<Diagnostic>> {
    let tokens = lex(text)?;
    let Some(Spanned {
        tok: Tok::Ident(function @ ("env" | "secret")),
        range,
    }) = tokens.first()
    else {
        return Ok(None);
    };
    let [
        _,
        Spanned {
            tok: Tok::LParen, ..
        },
        Spanned {
            tok: Tok::Str(argument),
            ..
        },
        Spanned {
            tok: Tok::RParen, ..
        },
    ] = tokens.as_slice()
    else {
        return Err(vec![Diagnostic {
            kind: DiagnosticKind::General,
            message: format!(
                "`{function}()` is a boot reference and takes exactly one string literal"
            ),
            range: range.clone(),
        }]);
    };
    let reference = if *function == "env" {
        crate::deployment::EnvironmentName::new(argument.clone())
            .map(crate::deployment::BootReference::Environment)
    } else {
        crate::deployment::SecretRef::new(argument.clone())
            .map(crate::deployment::BootReference::Secret)
    }
    .map_err(|error| {
        vec![Diagnostic {
            kind: DiagnosticKind::General,
            message: error.to_string(),
            range: range.clone(),
        }]
    })?;
    Ok(Some(reference))
}

const BOOT_REFERENCE_ROOT: &str = "$boot";

/// Lowers one source boot reference into the existing categorical expression
/// path form. `$boot` is not a source identifier, so an author cannot forge
/// this reserved namespace through ordinary path syntax.
#[must_use]
pub fn boot_reference_tree(reference: &crate::deployment::BootReference, ty: DataType) -> Tree {
    let (kind, name) = match reference {
        crate::deployment::BootReference::Environment(name) => ("env", name.as_str()),
        crate::deployment::BootReference::Secret(reference) => ("secret", reference.as_str()),
    };
    Tree {
        root: Expr::Path(vec![
            String::from(BOOT_REFERENCE_ROOT),
            String::from(kind),
            String::from(name),
        ]),
        ty,
    }
}

/// Reconstructs a boot reference from its categorical expression spelling.
///
/// # Errors
///
/// A path entering the reserved `$boot` root must have one exact, valid
/// three-segment form.
pub fn boot_reference(
    expression: &Expr,
) -> Result<Option<crate::deployment::BootReference>, crate::deployment::DeploymentError> {
    let Expr::Path(segments) = expression else {
        return Ok(None);
    };
    if segments
        .first()
        .is_none_or(|root| root != BOOT_REFERENCE_ROOT)
    {
        return Ok(None);
    }
    match segments.as_slice() {
        [_, kind, name] if kind == "env" => crate::deployment::EnvironmentName::new(name.clone())
            .map(crate::deployment::BootReference::Environment)
            .map(Some),
        [_, kind, name] if kind == "secret" => crate::deployment::SecretRef::new(name.clone())
            .map(crate::deployment::BootReference::Secret)
            .map(Some),
        _ => Err(crate::deployment::DeploymentError::InvalidBootReference),
    }
}

/// Evaluates a compiled tree against runtime bindings.
///
/// # Errors
///
/// Value errors only: unknown bindings, arithmetic faults, dynamic
/// type mismatches, malformed time strings.
pub fn eval(tree: &Tree, bindings: &dyn Bindings) -> Result<EvalValue, Error> {
    eval_expr(&tree.root, bindings)
}

/// Evaluates a compiled tree against constants only — the boot-time
/// reduction used by initializer positions.
///
/// # Errors
///
/// Same as [`eval`]; a reference to a non-constant name is an error.
pub fn const_eval(tree: &Tree, consts: &BTreeMap<String, Value>) -> Result<Value, Error> {
    let bindings: BTreeMap<String, EvalValue> = consts
        .iter()
        .map(|(name, value)| (name.clone(), EvalValue::from(value.clone())))
        .collect();
    eval_expr(&tree.root, &bindings)?.into_value()
}

// ===== lexing =====

#[derive(Debug, Clone, PartialEq)]
enum Tok<'a> {
    Int(i64),
    Uint(u64),
    Double(f64),
    Str(String),
    Ident(&'a str),
    True,
    False,
    Null,
    LParen,
    RParen,
    LBrack,
    RBrack,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Dot,
    Question,
    Bang,
    Minus,
    Plus,
    Star,
    Slash,
    Percent,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
}

#[derive(Debug, Clone, PartialEq)]
struct Spanned<'a> {
    tok: Tok<'a>,
    range: Range<usize>,
}

fn lex(text: &str) -> Result<Vec<Spanned<'_>>, Vec<Diagnostic>> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        let start = pos;
        let byte = bytes[pos];
        let tok = match byte {
            b' ' | b'\t' | b'\r' | b'\n' => {
                pos += 1;
                continue;
            }
            // Host trivia is trivia HERE too (one authority,
            // D-2026-08-13-1 review round): `//` per expr-spec, and
            // the host's `/* … */` so a commented spelling shares
            // the compiled form (and so the hash) of the plain one.
            // The FACT of the comment comes from the one primitive.
            b'/' if let Some(comment) = crate::trivia::comment_at(text, pos) => {
                if !comment.terminated {
                    return Err(vec![Diagnostic {
                        kind: DiagnosticKind::General,
                        message: String::from("this block comment never closes"),
                        range: pos..text.len(),
                    }]);
                }
                pos = comment.end;
                continue;
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                while pos < bytes.len()
                    && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_')
                {
                    pos += 1;
                }
                match &text[start..pos] {
                    "true" => Tok::True,
                    "false" => Tok::False,
                    "null" => Tok::Null,
                    "in" => {
                        return Err(vec![Diagnostic {
                            kind: DiagnosticKind::General,
                            message: String::from("`in` is outside this build's subset"),
                            range: start..pos,
                        }]);
                    }
                    word => Tok::Ident(word),
                }
            }
            b'0'..=b'9' => {
                while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                    pos += 1;
                }
                if pos < bytes.len() && bytes[pos] == b'u' {
                    let value = text[start..pos].parse().map_err(|_| {
                        vec![Diagnostic {
                            kind: DiagnosticKind::General,
                            message: String::from("uint literal out of range"),
                            range: start..pos,
                        }]
                    })?;
                    pos += 1;
                    Tok::Uint(value)
                } else if pos + 1 < bytes.len()
                    && bytes[pos] == b'.'
                    && bytes[pos + 1].is_ascii_digit()
                {
                    pos += 1;
                    while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                        pos += 1;
                    }
                    let value = text[start..pos].parse().map_err(|_| {
                        vec![Diagnostic {
                            kind: DiagnosticKind::General,
                            message: String::from("malformed double literal"),
                            range: start..pos,
                        }]
                    })?;
                    Tok::Double(value)
                } else {
                    let value = text[start..pos].parse().map_err(|_| {
                        vec![Diagnostic {
                            kind: DiagnosticKind::General,
                            message: String::from("int literal out of range"),
                            range: start..pos,
                        }]
                    })?;
                    Tok::Int(value)
                }
            }
            quote @ (b'"' | b'\'') => {
                pos += 1;
                let mut value = String::new();
                loop {
                    if pos >= bytes.len() {
                        return Err(vec![Diagnostic {
                            kind: DiagnosticKind::General,
                            message: String::from("unterminated string"),
                            range: start..pos,
                        }]);
                    }
                    let current = bytes[pos];
                    if current == quote {
                        pos += 1;
                        break;
                    }
                    if matches!(current, b'\r' | b'\n') {
                        return Err(vec![Diagnostic {
                            kind: DiagnosticKind::General,
                            message: String::from(
                                "an ordinary Rastok expression string cannot cross a physical line",
                            ),
                            range: start..pos,
                        }]);
                    }
                    if current == b'\\' {
                        pos += 1;
                        let escaped = *bytes.get(pos).ok_or_else(|| {
                            vec![Diagnostic {
                                kind: DiagnosticKind::General,
                                message: String::from("unterminated escape"),
                                range: start..pos,
                            }]
                        })?;
                        if matches!(escaped, b'\r' | b'\n') {
                            return Err(vec![Diagnostic {
                                kind: DiagnosticKind::General,
                                message: String::from(
                                    "a backslash cannot continue an ordinary Rastok expression string",
                                ),
                                range: start..pos,
                            }]);
                        }
                        value.push(match escaped {
                            b'n' => '\n',
                            b't' => '\t',
                            b'r' => '\r',
                            b'\\' => '\\',
                            b'\'' => '\'',
                            b'"' => '"',
                            other => {
                                return Err(vec![Diagnostic {
                                    kind: DiagnosticKind::General,
                                    message: format!(
                                        "unsupported escape `\\{}`",
                                        char::from(other)
                                    ),
                                    range: pos - 1..pos + 1,
                                }]);
                            }
                        });
                        pos += 1;
                    } else {
                        let character = text[pos..].chars().next().unwrap_or('\u{fffd}');
                        value.push(character);
                        pos += character.len_utf8();
                    }
                }
                Tok::Str(value)
            }
            b'(' => one(&mut pos, Tok::LParen),
            b')' => one(&mut pos, Tok::RParen),
            b'[' => one(&mut pos, Tok::LBrack),
            b']' => one(&mut pos, Tok::RBrack),
            b'{' => one(&mut pos, Tok::LBrace),
            b'}' => one(&mut pos, Tok::RBrace),
            b',' => one(&mut pos, Tok::Comma),
            b':' => one(&mut pos, Tok::Colon),
            b'.' => {
                if bytes.get(pos + 1) == Some(&b'?') {
                    return Err(vec![Diagnostic {
                        kind: DiagnosticKind::General,
                        message: String::from(
                            "optional access `.?` is outside this build's subset",
                        ),
                        range: pos..pos + 2,
                    }]);
                }
                one(&mut pos, Tok::Dot)
            }
            b'?' => one(&mut pos, Tok::Question),
            b'!' => {
                if bytes.get(pos + 1) == Some(&b'=') {
                    pos += 2;
                    Tok::NotEq
                } else {
                    one(&mut pos, Tok::Bang)
                }
            }
            b'-' => one(&mut pos, Tok::Minus),
            b'+' => one(&mut pos, Tok::Plus),
            b'*' => one(&mut pos, Tok::Star),
            b'/' => one(&mut pos, Tok::Slash),
            b'%' => one(&mut pos, Tok::Percent),
            b'=' => {
                if bytes.get(pos + 1) == Some(&b'=') {
                    pos += 2;
                    Tok::EqEq
                } else {
                    return Err(vec![Diagnostic {
                        kind: DiagnosticKind::General,
                        message: String::from("assignment is not an expression"),
                        range: pos..pos + 1,
                    }]);
                }
            }
            b'<' => {
                if bytes.get(pos + 1) == Some(&b'=') {
                    pos += 2;
                    Tok::Le
                } else {
                    one(&mut pos, Tok::Lt)
                }
            }
            b'>' => {
                if bytes.get(pos + 1) == Some(&b'=') {
                    pos += 2;
                    Tok::Ge
                } else {
                    one(&mut pos, Tok::Gt)
                }
            }
            b'&' => {
                if bytes.get(pos + 1) == Some(&b'&') {
                    pos += 2;
                    Tok::AndAnd
                } else {
                    return Err(vec![Diagnostic {
                        kind: DiagnosticKind::General,
                        message: String::from("`&` is not a Rastok expression operator"),
                        range: pos..pos + 1,
                    }]);
                }
            }
            b'|' => {
                if bytes.get(pos + 1) == Some(&b'|') {
                    pos += 2;
                    Tok::OrOr
                } else {
                    return Err(vec![Diagnostic {
                        kind: DiagnosticKind::General,
                        message: String::from("`|` is not a Rastok expression operator"),
                        range: pos..pos + 1,
                    }]);
                }
            }
            _ => {
                let character = text[pos..].chars().next().unwrap_or('\u{fffd}');
                return Err(vec![Diagnostic {
                    kind: DiagnosticKind::General,
                    message: format!(
                        "the character `{character}` is not part of the Rastok expression grammar"
                    ),
                    range: pos..pos + character.len_utf8(),
                }]);
            }
        };
        tokens.push(Spanned {
            tok,
            range: start..pos,
        });
    }
    Ok(tokens)
}

fn one<'a>(pos: &mut usize, tok: Tok<'a>) -> Tok<'a> {
    *pos += 1;
    tok
}

// ===== parsing =====

struct ExprParser<'t, 'a> {
    tokens: &'t [Spanned<'a>],
    pos: usize,
    text: &'a str,
    signature: &'t Signature,
    /// Production-nesting depth — the recursion guard. Bounded at
    /// the format budget so a parenthesis tower or a unary chain
    /// refuses by name instead of overflowing the parser's stack;
    /// grouping does not create AST depth, so the bound is
    /// conservative for adversarial shapes, never for real programs.
    depth: usize,
}

impl<'a> ExprParser<'_, 'a> {
    fn peek(&self) -> Option<&Tok<'a>> {
        self.tokens.get(self.pos).map(|spanned| &spanned.tok)
    }

    fn bump(&mut self) -> Option<&Spanned<'a>> {
        let token = self.tokens.get(self.pos);
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    fn eat(&mut self, tok: &Tok<'a>) -> bool {
        if self.peek() == Some(tok) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn fail(&self, message: impl Into<String>) -> Diagnostic {
        let range = self
            .tokens
            .get(self.pos)
            .map_or(self.text.len()..self.text.len(), |token| {
                token.range.clone()
            });
        Diagnostic {
            kind: DiagnosticKind::General,
            message: message.into(),
            range,
        }
    }

    fn expect(&mut self, tok: &Tok<'a>, what: &str) -> Result<(), Diagnostic> {
        if self.eat(tok) {
            Ok(())
        } else {
            Err(self.fail(format!("expected {what}")))
        }
    }

    /// One step deeper into the productions; refuses past the budget.
    /// Each SYNTACTIC nesting level counts exactly once — `ternary`
    /// counts (every nesting construct re-enters it through
    /// `primary`), and `unary` counts only when an operator actually
    /// nests, so the visible budget is the real one.
    fn descend(&mut self) -> Result<(), Diagnostic> {
        self.depth += 1;
        if self.depth > crate::canonical_cbor::MAX_NESTING {
            return Err(Diagnostic {
                kind: DiagnosticKind::TooDeep { depth: self.depth },
                message: String::from("the expression nests deeper than the format budget"),
                range: self
                    .tokens
                    .get(self.pos)
                    .map_or(0..self.text.len(), |token| token.range.clone()),
            });
        }
        Ok(())
    }

    fn ternary(&mut self) -> Result<Expr, Diagnostic> {
        self.descend()?;
        let result = self.ternary_inner();
        self.depth -= 1;
        result
    }

    fn ternary_inner(&mut self) -> Result<Expr, Diagnostic> {
        let cond = self.or()?;
        if self.eat(&Tok::Question) {
            let then = self.or()?;
            self.expect(&Tok::Colon, "`:`")?;
            let otherwise = self.ternary()?;
            return Ok(Expr::Ternary {
                cond: Box::new(cond),
                then: Box::new(then),
                otherwise: Box::new(otherwise),
            });
        }
        Ok(cond)
    }

    fn or(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.and()?;
        while self.eat(&Tok::OrOr) {
            let right = self.and()?;
            left = binary(BinaryOp::Or, left, right);
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.relation()?;
        while self.eat(&Tok::AndAnd) {
            let right = self.relation()?;
            left = binary(BinaryOp::And, left, right);
        }
        Ok(left)
    }

    fn relation(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.addition()?;
        loop {
            if matches!(self.peek(), Some(Tok::Ident("is"))) {
                left = self.is_suffix(left)?;
                continue;
            }
            let op = match self.peek() {
                Some(Tok::EqEq) => BinaryOp::Eq,
                Some(Tok::NotEq) => BinaryOp::Ne,
                Some(Tok::Lt) => BinaryOp::Lt,
                Some(Tok::Le) => BinaryOp::Le,
                Some(Tok::Gt) => BinaryOp::Gt,
                Some(Tok::Ge) => BinaryOp::Ge,
                _ => return Ok(left),
            };
            self.pos += 1;
            let right = self.addition()?;
            left = binary(op, left, right);
        }
    }

    fn addition(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.multiplication()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Plus) => BinaryOp::Add,
                Some(Tok::Minus) => BinaryOp::Sub,
                _ => return Ok(left),
            };
            self.pos += 1;
            let right = self.multiplication()?;
            left = binary(op, left, right);
        }
    }

    fn multiplication(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Star) => BinaryOp::Mul,
                Some(Tok::Slash) => BinaryOp::Div,
                Some(Tok::Percent) => BinaryOp::Rem,
                _ => return Ok(left),
            };
            self.pos += 1;
            let right = self.unary()?;
            left = binary(op, left, right);
        }
    }

    fn unary(&mut self) -> Result<Expr, Diagnostic> {
        // A unary chain recurses without ever re-entering `ternary`,
        // so an actual operator counts one level; the plain
        // fall-through to `postfix` is not a nesting and counts
        // nothing — one unit per syntactic level, everywhere.
        if matches!(self.peek(), Some(Tok::Bang | Tok::Minus)) {
            self.descend()?;
            let result = self.unary_inner();
            self.depth -= 1;
            return result;
        }
        self.postfix()
    }

    fn unary_inner(&mut self) -> Result<Expr, Diagnostic> {
        if self.eat(&Tok::Bang) {
            return Ok(Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(self.unary()?),
            });
        }
        if self.eat(&Tok::Minus) {
            return Ok(Expr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(self.unary()?),
            });
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Result<Expr, Diagnostic> {
        let mut expr = self.primary()?;
        loop {
            if self.eat(&Tok::Dot) {
                let Some(Tok::Ident(member)) = self.peek().cloned() else {
                    return Err(self.fail("expected a member name after `.`"));
                };
                self.pos += 1;
                if self.peek() == Some(&Tok::LParen) {
                    return Err(self.fail(format!(
                        "method call `.{member}()` is outside this build's subset"
                    )));
                }
                match &mut expr {
                    Expr::Path(segments) => segments.push(String::from(member)),
                    _ => {
                        return Err(self.fail(
                            "member access on a computed value is outside this build's subset",
                        ));
                    }
                }
            } else if self.eat(&Tok::LBrack) {
                let index = self.ternary()?;
                self.expect(&Tok::RBrack, "`]`")?;
                expr = Expr::Index {
                    base: Box::new(expr),
                    index: Box::new(index),
                };
            } else if matches!(self.peek(), Some(Tok::Ident("as"))) {
                expr = self.as_suffix(expr)?;
            } else {
                return Ok(expr);
            }
        }
    }

    // Keep narrowing's resolved `DataType` locals out of the recursive hot
    // frames (`relation`/`postfix`). The parser's exact 128-level guard is a
    // stack-safety contract, so an unused suffix must not enlarge every
    // ordinary parenthesized-expression frame.
    fn is_suffix(&mut self, expr: Expr) -> Result<Expr, Diagnostic> {
        self.pos += 1;
        let (name, range) = self.narrowing_name()?;
        let target = self.resolve_narrow_target(&expr, name, range)?;
        Ok(Expr::Is {
            expr: Box::new(expr),
            target: Box::new(target),
        })
    }

    fn as_suffix(&mut self, expr: Expr) -> Result<Expr, Diagnostic> {
        self.pos += 1;
        let (name, range) = self.narrowing_name()?;
        let target = self.resolve_narrow_target(&expr, name, range)?;
        Ok(Expr::As {
            expr: Box::new(expr),
            target: Box::new(target),
        })
    }

    fn narrowing_name(&mut self) -> Result<(&'a str, Range<usize>), Diagnostic> {
        match self.bump().cloned() {
            Some(Spanned {
                tok: Tok::Ident(name),
                range,
            }) => Ok((name, range)),
            _ => Err(self.fail("expected a named type or tagged-union case")),
        }
    }

    fn resolve_narrow_target(
        &self,
        expr: &Expr,
        name: &str,
        range: Range<usize>,
    ) -> Result<NarrowTarget, Diagnostic> {
        let source = check(expr, self.signature, self.text)?;
        if let DataType::TaggedUnion(union) = &source
            && let Some(case) = union.cases().iter().find(|case| case.name() == name)
        {
            return Ok(NarrowTarget::Case {
                name: String::from(name),
                payload: Box::new(DataType::Struct(case.payload().clone())),
            });
        }
        let Some(target) = self.signature.type_named(name) else {
            return Err(Diagnostic {
                kind: DiagnosticKind::General,
                message: format!("`{name}` names neither a logical type nor a case of {source}"),
                range,
            });
        };
        if let Err(error) = target.ty().validate() {
            return Err(Diagnostic {
                kind: DiagnosticKind::General,
                message: format!("`{name}` resolves to an invalid logical type: {error}"),
                range,
            });
        }
        Ok(NarrowTarget::Type(Box::new(target)))
    }

    fn primary(&mut self) -> Result<Expr, Diagnostic> {
        let Some(token) = self.bump().cloned() else {
            return Err(self.fail("expected an expression"));
        };
        match token.tok {
            Tok::Int(value) => Ok(Expr::Literal(Value::Int(value))),
            Tok::Uint(value) => Ok(Expr::Literal(Value::Uint(value))),
            Tok::Double(value) => Ok(Expr::Literal(Value::Double(value))),
            Tok::Str(value) => Ok(Expr::Literal(Value::Text(value))),
            Tok::True => Ok(Expr::Literal(Value::Bool(true))),
            Tok::False => Ok(Expr::Literal(Value::Bool(false))),
            Tok::Null => Ok(Expr::Literal(Value::Null)),
            Tok::LParen => self.grouped(),
            Tok::LBrack => self.list(),
            Tok::LBrace => self.map(),
            Tok::Ident(word) => self.name_or_call(word, token.range),
            _ => Err(Diagnostic {
                kind: DiagnosticKind::General,
                message: String::from("expected an expression"),
                range: token.range,
            }),
        }
    }

    // Keep the recursive grouping route small. In an unoptimized build Rust
    // reserves one frame large enough for every local in a function. Putting
    // map/call builders in `primary` therefore made a parenthesis tower pay
    // for temporaries belonging to branches it never entered, defeating the
    // explicit nesting guard before that guard could report the error.
    fn grouped(&mut self) -> Result<Expr, Diagnostic> {
        let expr = self.ternary()?;
        self.expect(&Tok::RParen, "`)`")?;
        Ok(expr)
    }

    fn list(&mut self) -> Result<Expr, Diagnostic> {
        let mut items = Vec::new();
        while self.peek() != Some(&Tok::RBrack) {
            items.push(self.ternary()?);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(&Tok::RBrack, "`]`")?;
        Ok(Expr::List(items))
    }

    fn map(&mut self) -> Result<Expr, Diagnostic> {
        let mut entries: Vec<(String, Expr)> = Vec::new();
        while self.peek() != Some(&Tok::RBrace) {
            let (key, range) = match self.bump().cloned() {
                Some(Spanned {
                    tok: Tok::Str(key),
                    range,
                }) => (key, range),
                _ => return Err(self.fail("map keys are string literals in this subset")),
            };
            if entries.iter().any(|(existing, _)| existing == &key) {
                // Stable message prefix: callers key on
                // "duplicate map key" to assign E-DUP-KEY.
                return Err(Diagnostic {
                    kind: DiagnosticKind::DuplicateMapKey,
                    message: format!("duplicate map key `{key}`"),
                    range,
                });
            }
            self.expect(&Tok::Colon, "`:`")?;
            entries.push((key, self.ternary()?));
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(&Tok::RBrace, "`}`")?;
        // Maps are semantically unordered: ONE canonical form serves
        // hashing, planning, and evaluation alike.
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(Expr::MapLit(entries))
    }

    fn name_or_call(&mut self, word: &'a str, range: Range<usize>) -> Result<Expr, Diagnostic> {
        if self.peek() != Some(&Tok::LParen) {
            if word == "now" {
                return Err(Diagnostic {
                    kind: DiagnosticKind::General,
                    message: String::from("`now` is a function; write `now()`"),
                    range,
                });
            }
            return Ok(Expr::Path(vec![String::from(word)]));
        }
        self.pos += 1;
        if word == "now" {
            let mut args = Vec::new();
            while self.peek() != Some(&Tok::RParen) {
                args.push(self.ternary()?);
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
            self.expect(&Tok::RParen, "`)`")?;
            if !args.is_empty() {
                return Err(Diagnostic {
                    kind: DiagnosticKind::General,
                    message: String::from("`now()` takes no arguments"),
                    range,
                });
            }
            // Context functions are bindings, not evaluator opcodes. This
            // keeps the plan form unchanged and makes the host's explicit
            // signature the sole authority for where logical time exists.
            return Ok(Expr::Path(vec![String::from("now")]));
        }
        let function = match word {
            "asset" => Function::Asset,
            "duration" => Function::Duration,
            "timestamp" => Function::Timestamp,
            "size" => Function::Size,
            "decimal" => Function::Decimal,
            "media_type" => Function::MediaType,
            "env" => {
                return Err(Diagnostic {
                    kind: DiagnosticKind::General,
                    message: String::from(
                        "`env()` is admitted only for a declared environment-compatible Connector setting (in M1: `HttpClient.url`)",
                    ),
                    range,
                });
            }
            "secret" => {
                return Err(Diagnostic {
                    kind: DiagnosticKind::General,
                    message: String::from(
                        "`secret()` is admitted only for a declared secret-compatible Connector setting (in M1: `HttpClient.auth_bearer`)",
                    ),
                    range,
                });
            }
            other if UNSUPPORTED_FUNCTIONS.contains(&other) => {
                return Err(Diagnostic {
                    kind: DiagnosticKind::General,
                    message: format!("the `{other}` macro is outside this build's subset"),
                    range,
                });
            }
            other => {
                return Err(Diagnostic {
                    kind: DiagnosticKind::General,
                    message: format!("unknown function `{other}`"),
                    range,
                });
            }
        };
        let mut args = Vec::new();
        while self.peek() != Some(&Tok::RParen) {
            args.push(self.ternary()?);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(&Tok::RParen, "`)`")?;
        Ok(Expr::Call { function, args })
    }
}

/// The structural nesting of a parsed expression, measured with an
/// explicit stack — the walk that bounds every later recursion over
/// the tree (type-check, hash, lowering, and the exact serialized
/// walker at validation). Parser literals are scalars, so only the
/// expression spine counts.
pub(crate) fn structural_depth(root: &Expr) -> usize {
    let mut max = 0;
    let mut stack = vec![(root, 1usize)];
    while let Some((expr, depth)) = stack.pop() {
        max = max.max(depth);
        match expr {
            Expr::Literal(_) | Expr::Path(_) | Expr::Asset(_) => {}
            Expr::Unary { expr, .. } | Expr::Is { expr, .. } | Expr::As { expr, .. } => {
                stack.push((expr, depth + 1));
            }
            Expr::Binary { left, right, .. } => {
                stack.push((left, depth + 1));
                stack.push((right, depth + 1));
            }
            Expr::Ternary {
                cond,
                then,
                otherwise,
            } => {
                stack.push((cond, depth + 1));
                stack.push((then, depth + 1));
                stack.push((otherwise, depth + 1));
            }
            Expr::Index { base, index } => {
                stack.push((base, depth + 1));
                stack.push((index, depth + 1));
            }
            Expr::Call { args, .. } => {
                for arg in args {
                    stack.push((arg, depth + 1));
                }
            }
            Expr::List(items) => {
                for item in items {
                    stack.push((item, depth + 1));
                }
            }
            Expr::MapLit(pairs) => {
                for (_, value) in pairs {
                    stack.push((value, depth + 1));
                }
            }
            Expr::Struct(fields) | Expr::Tagged { fields, .. } => {
                for (_, value) in fields {
                    stack.push((value, depth + 1));
                }
            }
        }
    }
    max
}

/// Consumes a refused tree with an explicit stack — a hostile-depth
/// chain must not recurse through its own destruction. Moving the
/// children out leaves each node a leaf, so its ordinary drop is
/// flat.
fn dismantle(root: Expr) {
    let mut stack = vec![root];
    while let Some(expr) = stack.pop() {
        match expr {
            Expr::Literal(_) | Expr::Path(_) | Expr::Asset(_) => {}
            Expr::Unary { expr, .. } | Expr::Is { expr, .. } | Expr::As { expr, .. } => {
                stack.push(*expr);
            }
            Expr::Binary { left, right, .. } => {
                stack.push(*left);
                stack.push(*right);
            }
            Expr::Ternary {
                cond,
                then,
                otherwise,
            } => {
                stack.push(*cond);
                stack.push(*then);
                stack.push(*otherwise);
            }
            Expr::Index { base, index } => {
                stack.push(*base);
                stack.push(*index);
            }
            Expr::Call { args, .. } => stack.extend(args),
            Expr::List(items) => stack.extend(items),
            Expr::MapLit(pairs) => stack.extend(pairs.into_iter().map(|(_, value)| value)),
            Expr::Struct(fields) | Expr::Tagged { fields, .. } => {
                stack.extend(fields.into_iter().map(|(_, value)| value));
            }
        }
    }
}

fn binary(op: BinaryOp, left: Expr, right: Expr) -> Expr {
    Expr::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
    }
}

// ===== type checking =====

fn check(expr: &Expr, signature: &Signature, text: &str) -> Result<DataType, Diagnostic> {
    let whole = 0..text.len();
    let fail = |message: String| Diagnostic {
        kind: DiagnosticKind::General,
        message,
        range: whole.clone(),
    };
    match expr {
        Expr::Literal(value) => Ok(literal_type(value)),
        // Compiler-constructed only — expression source never spells one.
        Expr::Asset(_) => Ok(DataType::BYTES),
        Expr::Path(segments) => {
            let dotted = segments.join(".");
            let mut resolved = None;
            for length in (1..=segments.len()).rev() {
                let prefix = segments[..length].join(".");
                if let Some(ty) = signature.get(&prefix) {
                    resolved = Some((ty.clone(), length));
                    break;
                }
            }
            let Some((mut current, consumed)) = resolved else {
                return Err(fail(format!("`{}` is not a visible name", segments[0])));
            };
            for segment in &segments[consumed..] {
                current = match current {
                    // Metadata paths are checked against their own sealed
                    // sys/app/ext grid after expression collection. The
                    // temporary `meta: variant` binding is not a payload
                    // escape hatch; ordinary dynamic data must narrow first.
                    DataType::Variant
                        if signature.is_metadata_root(&segments[..consumed].join(".")) =>
                    {
                        DataType::Variant
                    }
                    DataType::Variant => {
                        return Err(fail(format!(
                            "`{dotted}` reaches variant; narrow it with `is` or `as` before member access"
                        )));
                    }
                    DataType::Map(map) if map.key() == &DataType::Text => map.value().clone(),
                    DataType::Struct(structure) => structure
                        .fields()
                        .iter()
                        .find(|field| field.name() == segment)
                        .map(|field| field.ty().clone())
                        .ok_or_else(|| {
                            fail(format!("`{dotted}` has no declared member `{segment}`"))
                        })?,
                    _ => {
                        return Err(fail(format!("`{dotted}` does not resolve to a member")));
                    }
                };
            }
            Ok(current)
        }
        Expr::Unary { op, expr } => {
            let inner = check(expr, signature, text)?;
            if *op == UnaryOp::Neg
                && let Expr::Literal(Value::Int(value)) = expr.as_ref()
                && let Some(negated) = value.checked_neg()
            {
                return Ok(literal_type(&Value::Int(negated)));
            }
            match (op, &inner) {
                (_, DataType::Variant) => Ok(DataType::Variant),
                (UnaryOp::Not, DataType::Bool) => Ok(DataType::Bool),
                (UnaryOp::Neg, ty)
                    if matches!(ty, DataType::Integer(IntegerType::Signed(_)))
                        || matches!(ty, DataType::Float(_))
                        || ty == &DataType::DURATION =>
                {
                    Ok(inner)
                }
                _ => Err(fail(format!("operator does not accept a {inner:?}"))),
            }
        }
        Expr::Binary { op, left, right } => {
            let lhs = check(left, signature, text)?;
            let narrowed;
            let rhs_signature = if *op == BinaryOp::And {
                narrowed = signature.narrowed_by(left);
                &narrowed
            } else {
                signature
            };
            let rhs = check(right, rhs_signature, text)?;
            binary_type(*op, &lhs, &rhs).ok_or_else(|| {
                fail(format!(
                    "operator {op:?} does not accept {lhs:?} and {rhs:?}"
                ))
            })
        }
        Expr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            let cond_ty = check(cond, signature, text)?;
            if !matches!(cond_ty, DataType::Bool | DataType::Variant) {
                return Err(fail(String::from("the condition must be a bool")));
            }
            let narrowed = signature.narrowed_by(cond);
            let then_ty = check(then, &narrowed, text)?;
            let else_ty = check(otherwise, signature, text)?;
            Ok(join_types(&then_ty, &else_ty))
        }
        Expr::Index { base, index } => {
            let base_ty = check(base, signature, text)?;
            let index_ty = check(index, signature, text)?;
            match base_ty {
                DataType::Variant => Err(fail(String::from(
                    "variant must be narrowed with `is` or `as` before indexing",
                ))),
                DataType::Array(array) => {
                    if matches!(index_ty, DataType::Integer(_) | DataType::Variant) {
                        Ok(array.element().clone())
                    } else {
                        Err(fail(String::from("a list index must be an integer")))
                    }
                }
                DataType::Map(map) => {
                    if matches!(index_ty, DataType::Text | DataType::Variant) {
                        Ok(map.value().clone())
                    } else {
                        Err(fail(String::from("a map key must be text")))
                    }
                }
                other => Err(fail(format!("a {other:?} cannot be indexed"))),
            }
        }
        Expr::Call { function, args } => match function {
            Function::Asset => {
                if !(1..=2).contains(&args.len()) {
                    return Err(Diagnostic {
                        kind: DiagnosticKind::AssetCall,
                        message: String::from("`asset` takes a path and an optional content type"),
                        range: whole.clone(),
                    });
                }
                for arg in args {
                    if check(arg, signature, text)? != DataType::Text {
                        return Err(Diagnostic {
                            kind: DiagnosticKind::AssetCall,
                            message: String::from("`asset` arguments must be text"),
                            range: whole.clone(),
                        });
                    }
                }
                Ok(DataType::BYTES)
            }
            Function::Duration | Function::Timestamp | Function::Decimal | Function::MediaType => {
                if args.len() != 1 {
                    return Err(fail(String::from("this function takes one argument")));
                }
                let arg = check(&args[0], signature, text)?;
                if !matches!(arg, DataType::Text | DataType::Variant) {
                    return Err(fail(String::from("the argument must be text")));
                }
                Ok(match function {
                    Function::Duration => DataType::DURATION,
                    Function::Decimal => DataType::Number,
                    Function::MediaType => DataType::MediaType,
                    _ => DataType::TIMESTAMP,
                })
            }
            Function::Size => {
                if args.len() != 1 {
                    return Err(fail(String::from("`size` takes one argument")));
                }
                let arg = check(&args[0], signature, text)?;
                match arg {
                    DataType::Text
                    | DataType::BYTES
                    | DataType::Array(_)
                    | DataType::Map(_)
                    | DataType::Variant => Ok(DataType::INT),
                    other => Err(fail(format!("`size` does not accept a {other:?}"))),
                }
            }
        },
        Expr::List(items) => {
            let mut element = DataType::Variant;
            for (index, item) in items.iter().enumerate() {
                let ty = check(item, signature, text)?;
                if index == 0 {
                    element = ty;
                } else {
                    element = join_types(&element, &ty);
                }
            }
            Ok(DataType::array(element))
        }
        Expr::MapLit(entries) => {
            let mut values = DataType::Variant;
            for (index, (_, value)) in entries.iter().enumerate() {
                let ty = check(value, signature, text)?;
                if index == 0 {
                    values = ty;
                } else {
                    values = join_types(&values, &ty);
                }
            }
            Ok(DataType::text_map(values))
        }
        Expr::Struct(fields) => {
            let mut typed = Vec::with_capacity(fields.len());
            for (name, value) in fields {
                typed.push(TypeField::new(name.clone(), check(value, signature, text)?));
            }
            StructType::new(typed)
                .map(DataType::Struct)
                .map_err(|error| fail(error.to_string()))
        }
        Expr::Tagged {
            union,
            case,
            fields,
        } => {
            let Some(selected) = union
                .cases()
                .iter()
                .find(|candidate| candidate.name() == case)
            else {
                return Err(fail(format!(
                    "case `{case}` does not belong to this tagged union"
                )));
            };
            if fields.len() != selected.payload().fields().len() {
                return Err(fail(format!(
                    "case `{case}` requires {} fields, found {}",
                    selected.payload().fields().len(),
                    fields.len()
                )));
            }
            for ((name, value), expected) in fields.iter().zip(selected.payload().fields()) {
                if name != expected.name() {
                    return Err(fail(format!(
                        "case `{case}` requires field `{}`, found `{name}`",
                        expected.name()
                    )));
                }
                let found = check(value, signature, text)?;
                if !found.is_assignable_to(expected.ty()) {
                    return Err(fail(format!(
                        "case `{case}` field `{name}` requires {}, found {found}",
                        expected.ty()
                    )));
                }
            }
            Ok(DataType::TaggedUnion((**union).clone()))
        }
        Expr::Is { expr, target } => {
            let source = check(expr, signature, text)?;
            check_narrowing_compatibility(&source, target).map_err(fail)?;
            Ok(DataType::Bool)
        }
        Expr::As { expr, target } => {
            let source = check(expr, signature, text)?;
            check_narrowing_compatibility(&source, target).map_err(fail)?;
            Ok(match target.as_ref() {
                NarrowTarget::Type(target) => target.ty().clone(),
                NarrowTarget::Case { payload, .. } => (**payload).clone(),
            })
        }
    }
}

/// Re-derives the result type of an already parsed expression against an
/// explicit immutable environment. Transport validation uses this rather
/// than trusting [`Tree::ty`]; diagnostics remain the source compiler's job.
pub(crate) fn rederive_type(expr: &Expr, signature: &Signature) -> Option<DataType> {
    check(expr, signature, "transported expression").ok()
}

fn check_narrowing_compatibility(source: &DataType, target: &NarrowTarget) -> Result<(), String> {
    match target {
        NarrowTarget::Case { name, payload } => match source {
            DataType::TaggedUnion(union)
                if union.cases().iter().any(|case| {
                    case.name() == name
                        && matches!(payload.as_ref(), DataType::Struct(structure)
                            if structure == case.payload())
                }) =>
            {
                Ok(())
            }
            _ => Err(format!(
                "case `{name}` and its payload do not belong to the operand type {source}"
            )),
        },
        NarrowTarget::Type(target) => {
            let target = target.ty();
            if matches!(source, DataType::Variant)
                || matches!(target, DataType::Variant)
                || source.is_assignable_to(target)
                || target.is_assignable_to(source)
                || matches!(
                    source,
                    DataType::StructuralUnion(_) | DataType::TaggedUnion(_)
                )
                || matches!(
                    target,
                    DataType::StructuralUnion(_) | DataType::TaggedUnion(_)
                )
            {
                Ok(())
            } else {
                Err(format!("a {source} cannot be narrowed to {target}"))
            }
        }
    }
}

fn literal_type(value: &Value) -> DataType {
    match value {
        Value::Bool(_) => DataType::Bool,
        Value::Int(value) => {
            if i32::try_from(*value).is_ok() {
                DataType::INT
            } else {
                DataType::LONG
            }
        }
        Value::Uint(_) => DataType::UINT,
        Value::Double(_) => DataType::DOUBLE,
        Value::Text(_) => DataType::Text,
        Value::Bytes(_) => DataType::BYTES,
        Value::Duration(_) => DataType::DURATION,
        Value::Timestamp(_) => DataType::TIMESTAMP,
        Value::Null => DataType::Null,
        Value::Array(_) => DataType::array(DataType::Variant),
        Value::Map(_) => DataType::text_map(DataType::Variant),
        Value::KeyedMap(_) => DataType::Variant,
        // No struct type exists in the annotation subset; the value
        // carries its own witness and types as dynamic.
        Value::Struct(_) => DataType::Variant,
        Value::Decimal { .. } => DataType::Number,
        Value::Uuid(_) => DataType::Uuid,
        Value::Date(_)
        | Value::Time(_)
        | Value::TimestampNtz(_)
        | Value::Interval { .. }
        | Value::Enum(_)
        | Value::StructuralUnion { .. }
        | Value::TaggedUnion(_) => DataType::Variant,
    }
}

fn join_types(left: &DataType, right: &DataType) -> DataType {
    if left == right {
        left.clone()
    } else {
        DataType::Variant
    }
}

fn binary_type(op: BinaryOp, lhs: &DataType, rhs: &DataType) -> Option<DataType> {
    use BinaryOp::{Add, And, Div, Eq, Ge, Gt, Le, Lt, Mul, Ne, Or, Rem, Sub};
    if matches!(lhs, DataType::Variant) || matches!(rhs, DataType::Variant) {
        return Some(match op {
            Eq | Ne | Lt | Le | Gt | Ge | And | Or => DataType::Bool,
            _ => DataType::Variant,
        });
    }
    match op {
        And | Or => (lhs == &DataType::Bool && rhs == &DataType::Bool).then_some(DataType::Bool),
        Eq | Ne => Some(DataType::Bool),
        Lt | Le | Gt | Ge => {
            let comparable = numeric(lhs, rhs).is_some()
                || (lhs == rhs
                    && (lhs == &DataType::Text
                        || lhs == &DataType::DURATION
                        || lhs == &DataType::TIMESTAMP));
            comparable.then_some(DataType::Bool)
        }
        Add if lhs == &DataType::Text && rhs == &DataType::Text => Some(DataType::Text),
        Add if lhs == &DataType::DURATION && rhs == &DataType::DURATION => Some(DataType::DURATION),
        Add if (lhs == &DataType::TIMESTAMP && rhs == &DataType::DURATION)
            || (lhs == &DataType::DURATION && rhs == &DataType::TIMESTAMP) =>
        {
            Some(DataType::TIMESTAMP)
        }
        Sub if lhs == &DataType::DURATION && rhs == &DataType::DURATION => Some(DataType::DURATION),
        Sub if lhs == &DataType::TIMESTAMP && rhs == &DataType::DURATION => {
            Some(DataType::TIMESTAMP)
        }
        Sub if lhs == &DataType::TIMESTAMP && rhs == &DataType::TIMESTAMP => {
            Some(DataType::DURATION)
        }
        Add | Sub => numeric(lhs, rhs),
        Mul | Div | Rem => numeric(lhs, rhs),
    }
}

fn numeric(lhs: &DataType, rhs: &DataType) -> Option<DataType> {
    let same_carrier_family = matches!(
        (lhs, rhs),
        (
            DataType::Integer(IntegerType::Signed(_)),
            DataType::Integer(IntegerType::Signed(_))
        ) | (
            DataType::Integer(IntegerType::Unsigned(_)),
            DataType::Integer(IntegerType::Unsigned(_))
        ) | (DataType::Float(_), DataType::Float(_))
    );
    if !same_carrier_family {
        return None;
    }
    if lhs.is_assignable_to(rhs) {
        Some(rhs.clone())
    } else if rhs.is_assignable_to(lhs) {
        Some(lhs.clone())
    } else {
        None
    }
}

// ===== evaluation =====

/// Upgrades a payload witness at a point of STRUCTURAL consumption;
/// every other value passes through. Carrying positions (a `val`
/// binding, a ternary result, a list element) never call this — that
/// is what keeps `val copy = data` as lazy as `data` itself.
fn concrete(value: EvalValue, bindings: &dyn Bindings) -> Result<EvalValue, Error> {
    match value {
        EvalValue::Payload(reference) => bindings.payload(&reference),
        // A structural-union branch is transparent once an expression
        // consumes it. Its branch index remains intact in the materialized
        // `Value` and journal; the expression sees the selected logical
        // value after the checker has proved the narrowing.
        EvalValue::StructuralUnion { value, .. } => concrete(*value, bindings),
        other => Ok(other),
    }
}

/// The deep variant for comparisons: a witness inside a composite
/// materializes too, so `[data] == [x]` compares values, never
/// markers.
fn concrete_deep(value: EvalValue, bindings: &dyn Bindings) -> Result<EvalValue, Error> {
    Ok(match value {
        EvalValue::Payload(reference) => bindings.payload(&reference)?,
        EvalValue::List(items) => EvalValue::List(
            items
                .into_iter()
                .map(|item| concrete_deep(item, bindings))
                .collect::<Result<_, _>>()?,
        ),
        EvalValue::Map(map) => {
            let mut deep = BTreeMap::new();
            for (key, value) in map {
                deep.insert(key, concrete_deep(value, bindings)?);
            }
            EvalValue::Map(deep)
        }
        EvalValue::KeyedMap(entries) => EvalValue::KeyedMap(
            entries
                .into_iter()
                .map(|(key, value)| {
                    Ok((
                        concrete_deep(key, bindings)?,
                        concrete_deep(value, bindings)?,
                    ))
                })
                .collect::<Result<_, Error>>()?,
        ),
        EvalValue::Struct(fields) => EvalValue::Struct(
            fields
                .into_iter()
                .map(|(name, value)| Ok((name, concrete_deep(value, bindings)?)))
                .collect::<Result<_, _>>()?,
        ),
        // Structural branches are pairwise disjoint, so exposing the selected
        // value cannot guess between cases. Tagged unions stay tagged because
        // their case name is semantic and payload shapes may coincide.
        EvalValue::StructuralUnion { value, .. } => concrete_deep(*value, bindings)?,
        EvalValue::TaggedUnion { case, payload } => EvalValue::TaggedUnion {
            case,
            payload: payload
                .into_iter()
                .map(|(name, value)| Ok((name, concrete_deep(value, bindings)?)))
                .collect::<Result<_, Error>>()?,
        },
        other => other,
    })
}

fn eval_expr(expr: &Expr, bindings: &dyn Bindings) -> Result<EvalValue, Error> {
    match expr {
        Expr::Literal(value) => Ok(EvalValue::from(value.clone())),
        Expr::Asset(index) => bindings.asset(*index),
        Expr::Path(segments) => {
            if segments.len() == 1 {
                return bindings
                    .get(&segments[0])
                    .ok_or_else(|| fault(format!("`{}` is not bound", segments[0])));
            }
            let dotted = segments.join(".");
            if let Some(value) = bindings.get(&dotted) {
                return Ok(value);
            }
            let mut value = bindings
                .get(&segments[0])
                .ok_or_else(|| fault(format!("`{}` is not bound", segments[0])))?;
            for segment in &segments[1..] {
                // A member access IS the structural demand.
                value = concrete(value, bindings)?;
                match value {
                    EvalValue::Map(mut map) => {
                        value = map.remove(segment.as_str()).ok_or_else(|| {
                            fault(format!("`{dotted}` has no member `{segment}`"))
                        })?;
                    }
                    EvalValue::Struct(fields) => {
                        value = fields
                            .into_iter()
                            .find(|(name, _)| name == segment)
                            .map(|(_, value)| value)
                            .ok_or_else(|| {
                                fault(format!("`{dotted}` has no member `{segment}`"))
                            })?;
                    }
                    EvalValue::TaggedUnion { payload, .. } => {
                        value = payload
                            .into_iter()
                            .find(|(name, _)| name == segment)
                            .map(|(_, value)| value)
                            .ok_or_else(|| {
                                fault(format!("`{dotted}` has no member `{segment}`"))
                            })?;
                    }
                    _ => return Err(fault(format!("`{dotted}` is not a map"))),
                }
            }
            Ok(value)
        }
        Expr::Unary { op, expr } => {
            let value = concrete(eval_expr(expr, bindings)?, bindings)?;
            match (op, value) {
                (UnaryOp::Not, EvalValue::Bool(b)) => Ok(EvalValue::Bool(!b)),
                (UnaryOp::Neg, EvalValue::Int(i)) => i
                    .checked_neg()
                    .map(EvalValue::Int)
                    .ok_or_else(|| fault(String::from("integer overflow"))),
                (UnaryOp::Neg, EvalValue::Double(d)) => Ok(EvalValue::Double(-d)),
                (UnaryOp::Neg, EvalValue::Duration(d)) => d
                    .checked_neg()
                    .map(EvalValue::Duration)
                    .ok_or_else(|| fault(String::from("duration overflow"))),
                (_, other) => Err(fault(format!(
                    "operator does not accept {}",
                    other.kind_name()
                ))),
            }
        }
        Expr::Binary { op, left, right } => eval_binary(*op, left, right, bindings),
        Expr::Ternary {
            cond,
            then,
            otherwise,
        } => match concrete(eval_expr(cond, bindings)?, bindings)? {
            EvalValue::Bool(true) => eval_expr(then, bindings),
            EvalValue::Bool(false) => eval_expr(otherwise, bindings),
            other => Err(fault(format!(
                "the condition evaluated to {}",
                other.kind_name()
            ))),
        },
        Expr::Index { base, index } => {
            let base = concrete(eval_expr(base, bindings)?, bindings)?;
            let index = concrete(eval_expr(index, bindings)?, bindings)?;
            match (base, index) {
                (EvalValue::List(items), EvalValue::Int(i)) => usize::try_from(i)
                    .ok()
                    .and_then(|i| items.into_iter().nth(i))
                    .ok_or_else(|| fault(String::from("list index out of range"))),
                (EvalValue::List(items), EvalValue::Uint(i)) => usize::try_from(i)
                    .ok()
                    .and_then(|i| items.into_iter().nth(i))
                    .ok_or_else(|| fault(String::from("list index out of range"))),
                (EvalValue::Struct(fields), EvalValue::Text(key)) => fields
                    .into_iter()
                    .find(|(name, _)| name == &key)
                    .map(|(_, value)| value)
                    .ok_or_else(|| fault(String::from("no member with that name"))),
                (EvalValue::Map(mut map), EvalValue::Text(key)) => map
                    .remove(&key)
                    .ok_or_else(|| fault(String::from("no such map key"))),
                (EvalValue::KeyedMap(entries), key) => entries
                    .into_iter()
                    .find(|(candidate, _)| values_equal(candidate, &key))
                    .map(|(_, value)| value)
                    .ok_or_else(|| fault(String::from("no such canonical map key"))),
                (base, index) => Err(fault(format!(
                    "cannot index {} with {}",
                    base.kind_name(),
                    index.kind_name()
                ))),
            }
        }
        Expr::Call { function, args } => {
            let mut values = Vec::with_capacity(args.len());
            for arg in args {
                // A function consumes its argument's structure.
                values.push(concrete(eval_expr(arg, bindings)?, bindings)?);
            }
            match (function, values.as_slice()) {
                (Function::Asset, _) => Err(fault(String::from(
                    "an asset call reached evaluation before build-time binding",
                ))),
                (Function::Duration, [EvalValue::Text(text)]) => {
                    parse_duration(text).map(EvalValue::Duration)
                }
                (Function::Timestamp, [EvalValue::Text(text)]) => {
                    parse_timestamp(text).map(EvalValue::Timestamp)
                }
                (Function::Decimal, [EvalValue::Text(text)]) => {
                    match crate::json::parse_declared_number(text) {
                        Ok(crate::json::DeclaredNumber::Int(value)) => Ok(EvalValue::Int(value)),
                        Ok(crate::json::DeclaredNumber::Decimal { unscaled, scale }) => {
                            Ok(EvalValue::Decimal { unscaled, scale })
                        }
                        Err(reason) => Err(fault(alloc::format!(
                            "the text is not an exact decimal: {reason}"
                        ))),
                    }
                }
                (Function::MediaType, [EvalValue::Text(text)]) => {
                    crate::datatype::canonical_media_type(text)
                        .map(EvalValue::Text)
                        .map_err(|reason| {
                            fault(alloc::format!(
                                "the text is not a concrete media type: {reason}"
                            ))
                        })
                }
                (Function::Size, [EvalValue::Text(text)]) => {
                    Ok(EvalValue::Int(text.chars().count() as i64))
                }
                (Function::Size, [EvalValue::Bytes(bytes)]) => {
                    Ok(EvalValue::Int(bytes.len() as i64))
                }
                (Function::Size, [EvalValue::List(items)]) => {
                    Ok(EvalValue::Int(items.len() as i64))
                }
                (Function::Size, [EvalValue::Map(map)]) => Ok(EvalValue::Int(map.len() as i64)),
                (Function::Size, [EvalValue::KeyedMap(map)]) => {
                    Ok(EvalValue::Int(map.len() as i64))
                }
                (Function::Size, [EvalValue::Struct(fields)]) => {
                    Ok(EvalValue::Int(fields.len() as i64))
                }
                (function, values) => Err(fault(format!(
                    "{function:?} does not accept [{}]",
                    values
                        .iter()
                        .map(EvalValue::kind_name)
                        .collect::<Vec<_>>()
                        .join(", ")
                ))),
            }
        }
        Expr::List(items) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                values.push(eval_expr(item, bindings)?);
            }
            Ok(EvalValue::List(values))
        }
        Expr::MapLit(entries) => {
            let mut map = BTreeMap::new();
            for (key, value) in entries {
                map.insert(key.clone(), eval_expr(value, bindings)?);
            }
            Ok(EvalValue::Map(map))
        }
        Expr::Struct(fields) => {
            let mut values = Vec::with_capacity(fields.len());
            for (name, value) in fields {
                values.push((name.clone(), eval_expr(value, bindings)?));
            }
            Ok(EvalValue::Struct(values))
        }
        Expr::Tagged { case, fields, .. } => {
            let mut payload = Vec::with_capacity(fields.len());
            for (name, value) in fields {
                payload.push((name.clone(), eval_expr(value, bindings)?));
            }
            Ok(EvalValue::TaggedUnion {
                case: case.clone(),
                payload,
            })
        }
        Expr::Is { expr, target } => {
            let value = concrete_deep(eval_expr(expr, bindings)?, bindings)?.into_value()?;
            match target.as_ref() {
                NarrowTarget::Type(target) => target
                    .ty()
                    .is_instance(&value, target.discriminator())
                    .map(EvalValue::Bool)
                    .map_err(narrowing_fault),
                NarrowTarget::Case { name, payload } => match narrow_case(value, name, payload) {
                    Ok(_) => Ok(EvalValue::Bool(true)),
                    Err(error) if error.is_mismatch() => Ok(EvalValue::Bool(false)),
                    Err(error) => Err(narrowing_fault(error)),
                },
            }
        }
        Expr::As { expr, target } => {
            let value = concrete_deep(eval_expr(expr, bindings)?, bindings)?.into_value()?;
            match target.as_ref() {
                NarrowTarget::Type(target) => target
                    .ty()
                    .narrow(value, target.discriminator())
                    .map(EvalValue::from)
                    .map_err(narrowing_fault),
                NarrowTarget::Case { name, payload } => narrow_case(value, name, payload)
                    .map(EvalValue::from)
                    .map_err(narrowing_fault),
            }
        }
    }
}

fn narrow_case(
    value: Value,
    expected: &str,
    payload: &DataType,
) -> Result<Value, NarrowingFailure> {
    match value {
        Value::TaggedUnion(tagged) if tagged.case == expected => {
            let case = tagged.case;
            let fields = tagged.payload;
            let value = Value::Struct(fields);
            if let Some(violation) = payload.violation("case payload", &value) {
                Err(NarrowingFailure::CasePayload {
                    case,
                    violation: Box::new(violation),
                })
            } else {
                Ok(value)
            }
        }
        Value::TaggedUnion(tagged) => Err(NarrowingFailure::CaseMismatch {
            expected: expected.to_string(),
            found: tagged.case,
        }),
        _ => Err(NarrowingFailure::NoMatch {
            target: Box::new(payload.clone()),
        }),
    }
}

fn eval_binary(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    bindings: &dyn Bindings,
) -> Result<EvalValue, Error> {
    use BinaryOp::{Add, And, Div, Eq, Ge, Gt, Le, Lt, Mul, Ne, Or, Rem, Sub};
    if matches!(op, And | Or) {
        let EvalValue::Bool(lhs) = concrete(eval_expr(left, bindings)?, bindings)? else {
            return Err(fault(String::from("logic needs booleans")));
        };
        return match (op, lhs) {
            (And, false) => Ok(EvalValue::Bool(false)),
            (Or, true) => Ok(EvalValue::Bool(true)),
            _ => match concrete(eval_expr(right, bindings)?, bindings)? {
                EvalValue::Bool(rhs) => Ok(EvalValue::Bool(rhs)),
                _ => Err(fault(String::from("logic needs booleans"))),
            },
        };
    }
    let lhs = concrete_deep(eval_expr(left, bindings)?, bindings)?;
    let rhs = concrete_deep(eval_expr(right, bindings)?, bindings)?;
    match op {
        Eq => Ok(EvalValue::Bool(values_equal(&lhs, &rhs))),
        Ne => Ok(EvalValue::Bool(!values_equal(&lhs, &rhs))),
        Lt | Le | Gt | Ge => {
            let ordering = compare(&lhs, &rhs)?;
            Ok(EvalValue::Bool(match op {
                Lt => ordering.is_lt(),
                Le => ordering.is_le(),
                Gt => ordering.is_gt(),
                _ => ordering.is_ge(),
            }))
        }
        Add | Sub | Mul | Div | Rem => arithmetic(op, lhs, rhs),
        And | Or => unreachable!("handled above"),
    }
}

fn values_equal(lhs: &EvalValue, rhs: &EvalValue) -> bool {
    match (lhs, rhs) {
        (EvalValue::Bool(a), EvalValue::Bool(b)) => a == b,
        // Structural on the carried pair — scales are DECLARED and
        // equality distinguishes them ({150,2} ≠ {15,1},
        // D-2026-08-13-1); an integer never structurally equals a
        // decimal, and the written-scale fold keeps every foldable
        // value in the integer domain.
        (
            EvalValue::Decimal { unscaled, scale },
            EvalValue::Decimal {
                unscaled: other_unscaled,
                scale: other_scale,
            },
        ) => unscaled == other_unscaled && scale == other_scale,
        (EvalValue::Int(a), EvalValue::Int(b)) => a == b,
        (EvalValue::Uint(a), EvalValue::Uint(b)) => a == b,
        (EvalValue::Double(a), EvalValue::Double(b)) => a == b,
        (EvalValue::Text(a), EvalValue::Text(b)) => a == b,
        (EvalValue::Bytes(a), EvalValue::Bytes(b)) => a == b,
        (EvalValue::Duration(a), EvalValue::Duration(b)) => a == b,
        (EvalValue::Timestamp(a), EvalValue::Timestamp(b)) => a == b,
        (EvalValue::Date(a), EvalValue::Date(b)) => a == b,
        (EvalValue::Time(a), EvalValue::Time(b)) => a == b,
        (EvalValue::TimestampNtz(a), EvalValue::TimestampNtz(b)) => a == b,
        (
            EvalValue::Interval {
                months: a_months,
                days: a_days,
                nanoseconds: a_nanoseconds,
            },
            EvalValue::Interval {
                months: b_months,
                days: b_days,
                nanoseconds: b_nanoseconds,
            },
        ) => a_months == b_months && a_days == b_days && a_nanoseconds == b_nanoseconds,
        (EvalValue::Uuid(a), EvalValue::Uuid(b)) => a == b,
        (EvalValue::Null, EvalValue::Null) => true,
        (EvalValue::List(a), EvalValue::List(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| values_equal(x, y))
        }
        (EvalValue::Struct(a), EvalValue::Struct(b)) => {
            // Sorted like the canonical seal — expression `==` must not
            // split two values one identity owns, and stays total on
            // malformed duplicates (see `struct_fields_eq`).
            struct_fields_eq(a, b, values_equal)
        }
        (EvalValue::Map(a), EvalValue::Map(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|((ka, va), (kb, vb))| ka == kb && values_equal(va, vb))
        }
        (EvalValue::KeyedMap(a), EvalValue::KeyedMap(b)) => keyed_map_eq_by(a, b, values_equal),
        (EvalValue::Enum(a), EvalValue::Enum(b)) => a == b,
        (
            EvalValue::StructuralUnion {
                branch: a_branch,
                value: a_value,
            },
            EvalValue::StructuralUnion {
                branch: b_branch,
                value: b_value,
            },
        ) => a_branch == b_branch && values_equal(a_value, b_value),
        (
            EvalValue::TaggedUnion {
                case: a_case,
                payload: a_payload,
            },
            EvalValue::TaggedUnion {
                case: b_case,
                payload: b_payload,
            },
        ) => a_case == b_case && struct_fields_eq(a_payload, b_payload, values_equal),
        _ => false,
    }
}

fn compare(lhs: &EvalValue, rhs: &EvalValue) -> Result<core::cmp::Ordering, Error> {
    match (lhs, rhs) {
        (EvalValue::Int(a), EvalValue::Int(b)) => Ok(a.cmp(b)),
        (EvalValue::Uint(a), EvalValue::Uint(b)) => Ok(a.cmp(b)),
        (EvalValue::Double(a), EvalValue::Double(b)) => a
            .partial_cmp(b)
            .ok_or_else(|| fault(String::from("NaN is not ordered"))),
        (EvalValue::Text(a), EvalValue::Text(b)) => Ok(a.cmp(b)),
        (EvalValue::Duration(a), EvalValue::Duration(b)) => Ok(a.cmp(b)),
        (EvalValue::Timestamp(a), EvalValue::Timestamp(b)) => Ok(a.cmp(b)),
        (lhs, rhs) => Err(fault(format!(
            "cannot order {} against {}",
            lhs.kind_name(),
            rhs.kind_name()
        ))),
    }
}

fn arithmetic(op: BinaryOp, lhs: EvalValue, rhs: EvalValue) -> Result<EvalValue, Error> {
    use BinaryOp::{Add, Div, Mul, Rem, Sub};
    let overflow = || fault(String::from("integer overflow"));
    match (op, lhs, rhs) {
        (Add, EvalValue::Int(a), EvalValue::Int(b)) => {
            a.checked_add(b).map(EvalValue::Int).ok_or_else(overflow)
        }
        (Sub, EvalValue::Int(a), EvalValue::Int(b)) => {
            a.checked_sub(b).map(EvalValue::Int).ok_or_else(overflow)
        }
        (Mul, EvalValue::Int(a), EvalValue::Int(b)) => {
            a.checked_mul(b).map(EvalValue::Int).ok_or_else(overflow)
        }
        (Div, EvalValue::Int(a), EvalValue::Int(b)) => a
            .checked_div(b)
            .map(EvalValue::Int)
            .ok_or_else(|| fault(String::from("division by zero"))),
        (Rem, EvalValue::Int(a), EvalValue::Int(b)) => a
            .checked_rem(b)
            .map(EvalValue::Int)
            .ok_or_else(|| fault(String::from("division by zero"))),
        (Add, EvalValue::Uint(a), EvalValue::Uint(b)) => {
            a.checked_add(b).map(EvalValue::Uint).ok_or_else(overflow)
        }
        (Sub, EvalValue::Uint(a), EvalValue::Uint(b)) => {
            a.checked_sub(b).map(EvalValue::Uint).ok_or_else(overflow)
        }
        (Mul, EvalValue::Uint(a), EvalValue::Uint(b)) => {
            a.checked_mul(b).map(EvalValue::Uint).ok_or_else(overflow)
        }
        (Div, EvalValue::Uint(a), EvalValue::Uint(b)) => a
            .checked_div(b)
            .map(EvalValue::Uint)
            .ok_or_else(|| fault(String::from("division by zero"))),
        (Rem, EvalValue::Uint(a), EvalValue::Uint(b)) => a
            .checked_rem(b)
            .map(EvalValue::Uint)
            .ok_or_else(|| fault(String::from("division by zero"))),
        (Add, EvalValue::Double(a), EvalValue::Double(b)) => Ok(EvalValue::Double(a + b)),
        (Sub, EvalValue::Double(a), EvalValue::Double(b)) => Ok(EvalValue::Double(a - b)),
        (Mul, EvalValue::Double(a), EvalValue::Double(b)) => Ok(EvalValue::Double(a * b)),
        (Div, EvalValue::Double(a), EvalValue::Double(b)) => Ok(EvalValue::Double(a / b)),
        (Add, EvalValue::Text(a), EvalValue::Text(b)) => Ok(EvalValue::Text(a + &b)),
        (Add, EvalValue::Duration(a), EvalValue::Duration(b)) => a
            .checked_add(b)
            .map(EvalValue::Duration)
            .ok_or_else(overflow),
        (Sub, EvalValue::Duration(a), EvalValue::Duration(b)) => a
            .checked_sub(b)
            .map(EvalValue::Duration)
            .ok_or_else(overflow),
        (Add, EvalValue::Timestamp(a), EvalValue::Duration(b))
        | (Add, EvalValue::Duration(b), EvalValue::Timestamp(a)) => a
            .checked_add(b)
            .map(EvalValue::Timestamp)
            .ok_or_else(overflow),
        (Sub, EvalValue::Timestamp(a), EvalValue::Duration(b)) => a
            .checked_sub(b)
            .map(EvalValue::Timestamp)
            .ok_or_else(overflow),
        (Sub, EvalValue::Timestamp(a), EvalValue::Timestamp(b)) => a
            .checked_sub(b)
            .map(EvalValue::Duration)
            .ok_or_else(overflow),
        (op, lhs, rhs) => Err(fault(format!(
            "{op:?} does not accept {} and {}",
            lhs.kind_name(),
            rhs.kind_name()
        ))),
    }
}

fn fault(message: String) -> Error {
    Error {
        kind: ErrorKind::Evaluation,
        message,
    }
}

fn narrowing_fault(reason: NarrowingFailure) -> Error {
    Error {
        message: reason.to_string(),
        kind: ErrorKind::Narrowing(reason),
    }
}

// ===== time parsing =====

/// Parses the CEL/Go duration format: `ns`/`us`/`ms`/`s`/`m`/`h`,
/// compound and fractional segments, optional leading sign. `min`,
/// `d`, and `w` are not accepted.
fn parse_duration(text: &str) -> Result<i64, Error> {
    let bad = || fault(String::from("the text is not a valid duration"));
    let (negative, rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    if rest.is_empty() {
        return Err(bad());
    }
    let bytes = rest.as_bytes();
    let mut pos = 0;
    let mut total: i64 = 0;
    while pos < bytes.len() {
        let start = pos;
        while pos < bytes.len() && (bytes[pos].is_ascii_digit() || bytes[pos] == b'.') {
            pos += 1;
        }
        let number: f64 = rest[start..pos].parse().map_err(|_| bad())?;
        let unit_start = pos;
        while pos < bytes.len() && bytes[pos].is_ascii_alphabetic() {
            pos += 1;
        }
        let scale: i64 = match &rest[unit_start..pos] {
            "ns" => 1,
            "us" => 1_000,
            "ms" => 1_000_000,
            "s" => 1_000_000_000,
            "m" => 60_000_000_000,
            "h" => 3_600_000_000_000,
            _ => return Err(bad()),
        };
        let nanos = number * scale as f64;
        if !nanos.is_finite() || nanos > i64::MAX as f64 {
            return Err(bad());
        }
        total = total.checked_add(nanos as i64).ok_or_else(bad)?;
    }
    Ok(if negative { -total } else { total })
}

/// Parses an RFC 3339 UTC timestamp (`Z` suffix only) to nanoseconds
/// since the Unix epoch.
fn parse_timestamp(text: &str) -> Result<i64, Error> {
    let bad = || fault(String::from("the text is not a valid UTC timestamp"));
    // RFC 3339 has fixed widths; checking them first also bounds every
    // component before any arithmetic, so nothing below can overflow.
    let field = |part: Option<&str>, width: usize| -> Result<i64, Error> {
        let part = part.ok_or_else(bad)?;
        if part.len() != width || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(bad());
        }
        part.parse().map_err(|_| bad())
    };
    let rest = text.strip_suffix('Z').ok_or_else(bad)?;
    let (date, time) = rest.split_once('T').ok_or_else(bad)?;
    let mut date_parts = date.splitn(3, '-');
    let year = field(date_parts.next(), 4)?;
    let month = field(date_parts.next(), 2)?;
    let day = field(date_parts.next(), 2)?;
    let (clock, fraction) = match time.split_once('.') {
        Some((clock, fraction)) => (clock, Some(fraction)),
        None => (time, None),
    };
    let mut clock_parts = clock.splitn(3, ':');
    let hour = field(clock_parts.next(), 2)?;
    let minute = field(clock_parts.next(), 2)?;
    let second = field(clock_parts.next(), 2)?;
    if !(1..=12).contains(&month)
        || !(0..24).contains(&hour)
        || !(0..60).contains(&minute)
        || !(0..60).contains(&second)
    {
        return Err(bad());
    }
    // A timestamp-shaped string is not yet a timestamp: the calendar
    // must agree (no February 31st).
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days_in_month = match month {
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        _ => 31,
    };
    if !(1..=days_in_month).contains(&day) {
        return Err(bad());
    }
    let nanos_fraction: i64 = match fraction {
        None => 0,
        Some(digits) => {
            if digits.is_empty() || digits.len() > 9 || !digits.bytes().all(|b| b.is_ascii_digit())
            {
                return Err(bad());
            }
            let value: i64 = digits.parse().map_err(|_| bad())?;
            value * 10_i64.pow(9 - digits.len() as u32)
        }
    };
    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second;
    seconds
        .checked_mul(1_000_000_000)
        .and_then(|nanos| nanos.checked_add(nanos_fraction))
        .ok_or_else(bad)
}

/// Days since 1970-01-01 for a proleptic-Gregorian civil date
/// (Howard Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let adjusted_year = if month <= 2 { year - 1 } else { year };
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let month_shifted = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_shifted + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transported_case_narrowing_rederives_the_payload_from_the_source_union() {
        let payload = StructType::new(vec![TypeField::new("code", DataType::INT)]).unwrap();
        let union = TaggedUnionType::new(vec![
            crate::datatype::TaggedCase::new("Exited", payload.clone()).unwrap(),
        ])
        .unwrap();
        let signature = Signature::new().with("data", DataType::TaggedUnion(union));
        for operator in ["is", "as"] {
            let mut tree = compile(&format!("data {operator} Exited"), &signature).unwrap();
            assert_eq!(rederive_type(&tree.root, &signature), Some(tree.ty.clone()));
            let (Expr::Is { target, .. } | Expr::As { target, .. }) = &mut tree.root else {
                unreachable!();
            };
            let NarrowTarget::Case { payload, .. } = target.as_mut() else {
                unreachable!();
            };
            **payload = DataType::Struct(
                StructType::new(vec![TypeField::new("code", DataType::Text)]).unwrap(),
            );
            assert_eq!(rederive_type(&tree.root, &signature), None, "{operator}");
        }
    }

    #[test]
    fn evaluator_carriage_preserves_value_equality_for_every_family() {
        let struct_a = Value::Struct(alloc::vec![
            (String::from("a"), Value::Int(1)),
            (String::from("b"), Value::Bool(true)),
        ]);
        let struct_b = Value::Struct(alloc::vec![
            (String::from("b"), Value::Bool(true)),
            (String::from("a"), Value::Int(1)),
        ]);
        let keyed_a = Value::KeyedMap(alloc::vec![
            (Value::Int(2), Value::Text(String::from("b"))),
            (Value::Int(1), Value::Text(String::from("a"))),
        ]);
        let keyed_b = Value::KeyedMap(alloc::vec![
            (Value::Int(1), Value::Text(String::from("a"))),
            (Value::Int(2), Value::Text(String::from("b"))),
        ]);
        let tagged_a = Value::TaggedUnion(Box::new(crate::value::TaggedUnionValue {
            case: String::from("done"),
            payload: alloc::vec![
                (String::from("code"), Value::Int(0)),
                (String::from("signal"), Value::Null),
            ],
        }));
        let tagged_b = Value::TaggedUnion(Box::new(crate::value::TaggedUnionValue {
            case: String::from("done"),
            payload: alloc::vec![
                (String::from("signal"), Value::Null),
                (String::from("code"), Value::Int(0)),
            ],
        }));
        let values = alloc::vec![
            Value::Bool(true),
            Value::Int(-7),
            Value::Uint(7),
            Value::Double(f64::NAN),
            Value::Double(f64::NAN),
            Value::Double(-0.0),
            Value::Double(0.0),
            Value::Text(String::from("text")),
            Value::Bytes(alloc::vec![0, 1]),
            Value::Duration(1),
            Value::Timestamp(2),
            Value::Date(3),
            Value::Time(4),
            Value::TimestampNtz(5),
            Value::Interval {
                months: 1,
                days: 2,
                nanoseconds: 3,
            },
            Value::Uuid([9; 16]),
            Value::Null,
            Value::Array(alloc::vec![Value::Int(1), Value::Int(2)]),
            Value::Map(alloc::collections::BTreeMap::from([(
                String::from("key"),
                Value::Bool(true),
            )])),
            keyed_a,
            keyed_b,
            struct_a,
            struct_b,
            Value::Decimal {
                unscaled: 150,
                scale: 2,
            },
            Value::Enum(String::from("ready")),
            Value::StructuralUnion {
                branch: 1,
                value: Box::new(Value::Text(String::from("selected"))),
            },
            tagged_a,
            tagged_b,
        ];

        for value in &values {
            assert_eq!(
                EvalValue::from(value.clone())
                    .into_value()
                    .expect("a materialized value has no payload witness"),
                value.clone()
            );
        }
        for left in &values {
            for right in &values {
                assert_eq!(
                    left == right,
                    EvalValue::from(left.clone()) == EvalValue::from(right.clone()),
                    "equality drifted for {left:?} and {right:?}"
                );
            }
        }
    }

    #[test]
    fn a_payload_witness_never_becomes_a_materialized_value() {
        let nested = EvalValue::List(alloc::vec![EvalValue::Payload(PayloadRef::Input)]);
        assert!(
            nested.into_value().is_err(),
            "evaluator state must not enter plans, envelopes, or journals"
        );

        let materialized = Value::Array(alloc::vec![Value::Int(1)]);
        assert_eq!(
            EvalValue::from(materialized.clone())
                .into_value()
                .expect("a core value has no lazy witness"),
            materialized
        );
    }

    #[test]
    fn struct_equality_is_reflexive_even_on_malformed_values() {
        // `Eq`'s contract is unconditional: a duplicate-name struct
        // is refused at every door, but it is REPRESENTABLE — and a
        // representable value must equal itself. Sorted comparison
        // keeps the door refusals AND the contract.
        let duplicated = EvalValue::Struct(alloc::vec![
            (String::from("x"), EvalValue::Int(1)),
            (String::from("x"), EvalValue::Int(2)),
        ]);
        assert_eq!(duplicated, duplicated.clone(), "reflexive");
        assert!(
            values_equal(&duplicated, &duplicated),
            "the expression `==` agrees"
        );
        // Symmetry, on two DIFFERENT malformed spellings.
        let reversed = EvalValue::Struct(alloc::vec![
            (String::from("x"), EvalValue::Int(2)),
            (String::from("x"), EvalValue::Int(1)),
        ]);
        assert_eq!(duplicated == reversed, reversed == duplicated, "symmetric");
        // And the well-formed case still compares by name.
        let ab = EvalValue::Struct(alloc::vec![
            (String::from("a"), EvalValue::Int(1)),
            (String::from("b"), EvalValue::Int(2)),
        ]);
        let ba = EvalValue::Struct(alloc::vec![
            (String::from("b"), EvalValue::Int(2)),
            (String::from("a"), EvalValue::Int(1)),
        ]);
        assert_eq!(ab, ba, "order is presentation");
        assert!(values_equal(&ab, &ba));
    }

    #[test]
    fn keyed_map_equality_stays_total_before_payload_resolution() {
        let unresolved = EvalValue::KeyedMap(alloc::vec![
            (
                EvalValue::Payload(PayloadRef::Input),
                EvalValue::Text(String::from("payload")),
            ),
            (EvalValue::Int(1), EvalValue::Text(String::from("one"))),
        ]);
        let reordered = EvalValue::KeyedMap(alloc::vec![
            (EvalValue::Int(1), EvalValue::Text(String::from("one"))),
            (
                EvalValue::Payload(PayloadRef::Input),
                EvalValue::Text(String::from("payload")),
            ),
        ]);
        assert_eq!(unresolved, reordered);
        assert!(values_equal(&unresolved, &reordered));
    }

    fn compile_ok(text: &str, signature: &Signature) -> Tree {
        compile(text, signature).unwrap_or_else(|diagnostics| {
            panic!("expected `{text}` to compile, got {diagnostics:?}")
        })
    }

    fn eval_const(text: &str) -> EvalValue {
        let tree = compile_ok(text, &Signature::new());
        eval(&tree, &BTreeMap::new()).unwrap()
    }

    fn oracle_value(value: cel_interpreter::Value) -> EvalValue {
        use cel_interpreter::objects::Key;
        match value {
            cel_interpreter::Value::Int(value) => EvalValue::Int(value),
            cel_interpreter::Value::UInt(value) => EvalValue::Uint(value),
            cel_interpreter::Value::Float(value) => EvalValue::Double(value),
            cel_interpreter::Value::String(value) => EvalValue::Text((*value).clone()),
            cel_interpreter::Value::Bytes(value) => EvalValue::Bytes((*value).clone()),
            cel_interpreter::Value::Bool(value) => EvalValue::Bool(value),
            cel_interpreter::Value::Null => EvalValue::Null,
            cel_interpreter::Value::List(items) => {
                EvalValue::List(items.iter().cloned().map(oracle_value).collect())
            }
            cel_interpreter::Value::Map(map) => EvalValue::Map(
                map.map
                    .iter()
                    .map(|(key, value)| {
                        let Key::String(key) = key else {
                            panic!("the compatible corpus uses text map keys only");
                        };
                        ((*key).as_ref().clone(), oracle_value(value.clone()))
                    })
                    .collect(),
            ),
            cel_interpreter::Value::Function(_, _) => {
                panic!("a function is not a materialized compatible result")
            }
        }
    }

    #[test]
    fn the_cel_compatible_core_matches_the_pinned_oracle() {
        // This table is intentionally source-level and versioned by the exact
        // dev dependency. Rastok-owned syntax and semantics live in the
        // divergence table in docs/spec/expressions.md, not in this oracle.
        let compatible = [
            "null",
            "true",
            "1 + 2 * 3",
            "7 / 2",
            "7 % 3",
            "3u + 4u",
            "1.5 + 0.25",
            "\"a\" + \"b\"",
            "1 < 2 && 2 <= 2",
            "true ? 1 : 2",
            "[1, 2, 3][1]",
            "{\"a\": 1, \"b\": 2}[\"b\"]",
            "size([1, 2, 3])",
        ];
        let context = cel_interpreter::Context::default();
        for source in compatible {
            let oracle = cel_interpreter::Program::compile(source)
                .unwrap_or_else(|error| panic!("oracle rejected {source:?}: {error}"))
                .execute(&context)
                .unwrap_or_else(|error| panic!("oracle failed {source:?}: {error}"));
            assert_eq!(
                eval_const(source),
                oracle_value(oracle),
                "compatible CEL drift for {source:?}"
            );
        }
    }

    #[test]
    fn literals_and_arithmetic_evaluate() {
        assert_eq!(eval_const("1 + 2 * 3"), EvalValue::Int(7));
        assert_eq!(eval_const("(1 + 2) * 3"), EvalValue::Int(9));
        assert_eq!(eval_const("7 % 3"), EvalValue::Int(1));
        assert_eq!(eval_const("1.5 + 0.25"), EvalValue::Double(1.75));
        assert_eq!(
            eval_const("\"a\" + 'b'"),
            EvalValue::Text(String::from("ab"))
        );
        assert_eq!(eval_const("-3"), EvalValue::Int(-3));
        assert_eq!(eval_const("3u + 4u"), EvalValue::Uint(7));
    }

    #[test]
    fn ordinary_strings_never_cross_a_physical_line() {
        for source in ["\"left\nright\"", "'left\nright'", "\"left\\\nright\""] {
            let diagnostics = compile(source, &Signature::new()).expect_err(
                "a physical line ending cannot live in an ordinary Rastok expression string",
            );
            assert_eq!(diagnostics.len(), 1, "{source:?}: {diagnostics:?}");
            assert!(
                diagnostics[0].message.contains("physical line")
                    || diagnostics[0].message.contains("cannot continue"),
                "{source:?}: {diagnostics:?}"
            );
        }
        assert_eq!(
            eval_const("\"left\\nright\""),
            EvalValue::Text(String::from("left\nright")),
            "the escaped newline remains the ordinary spelling"
        );
    }

    #[test]
    fn comparisons_ternary_and_logic_evaluate() {
        assert_eq!(eval_const("1 < 2 && 2 <= 2"), EvalValue::Bool(true));
        assert_eq!(eval_const("\"a\" != \"b\""), EvalValue::Bool(true));
        assert_eq!(eval_const("true ? 1 : 2"), EvalValue::Int(1));
        assert_eq!(eval_const("null == null"), EvalValue::Bool(true));
        assert_eq!(eval_const("1 == \"1\""), EvalValue::Bool(false));
        assert_eq!(eval_const("!false || false"), EvalValue::Bool(true));
    }

    /// The expression `==` OPERATOR keeps IEEE semantics — `values_equal`,
    /// not the structural `PartialEq` (which compares canonical bits
    /// so plans stay equal to themselves across the byte trip). The
    /// two equalities answer different questions and must not bleed
    /// into each other.
    #[test]
    fn the_expression_operator_keeps_ieee_while_structure_keeps_bits() {
        // IEEE, through the operator: NaN never equals itself, and
        // the two zeros are numerically one.
        assert_eq!(eval_const("0.0 / 0.0 == 0.0 / 0.0"), EvalValue::Bool(false));
        assert_eq!(eval_const("-0.0 == 0.0"), EvalValue::Bool(true));
        // Bits, through the structure: NaN is ONE value, the zeros
        // are TWO.
        assert_eq!(eval_const("0.0 / 0.0"), eval_const("0.0 / 0.0"));
        assert_ne!(eval_const("-0.0"), eval_const("0.0"));
    }

    #[test]
    fn logic_short_circuits() {
        // The right side would divide by zero; short-circuit skips it.
        assert_eq!(eval_const("false && 1 / 0 == 0"), EvalValue::Bool(false));
        assert_eq!(eval_const("true || 1 / 0 == 0"), EvalValue::Bool(true));
    }

    #[test]
    fn value_errors_are_values_not_panics() {
        let tree = compile_ok("1 / 0", &Signature::new());
        assert_eq!(
            eval(&tree, &BTreeMap::new()).unwrap_err().message,
            "division by zero"
        );
        let overflow = compile_ok("9223372036854775807 + 1", &Signature::new());
        assert_eq!(
            eval(&overflow, &BTreeMap::new()).unwrap_err().message,
            "integer overflow"
        );
    }

    #[test]
    fn runtime_faults_describe_kinds_without_formatting_values() {
        let sentinel = "classified-value-must-not-escape";
        let tree = Tree {
            root: Expr::Index {
                base: Box::new(Expr::Literal(Value::Map(BTreeMap::new()))),
                index: Box::new(Expr::Literal(Value::Text(String::from(sentinel)))),
            },
            ty: DataType::Variant,
        };
        let message = eval(&tree, &BTreeMap::new())
            .expect_err("the missing map key faults")
            .message;
        assert_eq!(message, "no such map key");
        assert!(!message.contains(sentinel));
    }

    #[test]
    fn paths_resolve_through_bindings() {
        let signature = Signature::new()
            .with("data", customer_type())
            .with("session.key", DataType::Text);
        let tree = compile_ok("data.name != \"\" && session.key == \"s-1\"", &signature);
        assert_eq!(tree.ty, DataType::Bool);
        let mut data = BTreeMap::new();
        data.insert(String::from("id"), EvalValue::Int(7));
        data.insert(String::from("name"), EvalValue::Text(String::from("hello")));
        let mut bindings = BTreeMap::new();
        bindings.insert(String::from("data"), EvalValue::Map(data));
        bindings.insert(
            String::from("session.key"),
            EvalValue::Text(String::from("s-1")),
        );
        assert_eq!(eval(&tree, &bindings).unwrap(), EvalValue::Bool(true));
    }

    fn customer_type() -> DataType {
        DataType::Struct(
            crate::datatype::StructType::new(alloc::vec![
                crate::datatype::TypeField::new("id", DataType::INT),
                crate::datatype::TypeField::new("name", DataType::Text),
            ])
            .expect("two unique fields"),
        )
    }

    #[test]
    fn variant_narrowing_is_checked_and_flow_sensitive() {
        let customer = customer_type();
        let signature = Signature::new()
            .with("data", DataType::Variant)
            .with_type("Customer", customer.clone());
        let predicate = compile_ok("data is Customer && data.id > 0", &signature);
        assert_eq!(predicate.ty, DataType::Bool);

        let value = EvalValue::Struct(alloc::vec![
            (String::from("name"), EvalValue::Text(String::from("Ada"))),
            (String::from("id"), EvalValue::Int(37)),
        ]);
        let bindings = BTreeMap::from([(String::from("data"), value.clone())]);
        assert_eq!(
            eval(&predicate, &bindings).expect("the struct matches"),
            EvalValue::Bool(true)
        );

        let converted = compile_ok("data as Customer", &signature);
        assert_eq!(converted.ty, customer);
        assert_eq!(
            eval(&converted, &bindings).expect("the checked conversion succeeds"),
            value
        );

        let wrong = BTreeMap::from([(
            String::from("data"),
            EvalValue::Struct(alloc::vec![(
                String::from("name"),
                EvalValue::Text(String::from("Ada")),
            )]),
        )]);
        assert_eq!(
            eval(&predicate, &wrong).expect("an ordinary mismatch is false"),
            EvalValue::Bool(false)
        );
        assert!(matches!(
            eval(&converted, &wrong)
                .expect_err("`as` must refuse the mismatch")
                .kind,
            ErrorKind::Narrowing(NarrowingFailure::NoMatch { .. })
        ));

        let nested_data = DataType::Struct(
            crate::datatype::StructType::new(alloc::vec![crate::datatype::TypeField::new(
                "customer",
                DataType::Variant,
            )])
            .expect("one field"),
        );
        let nested_signature = Signature::new()
            .with("data", nested_data)
            .with_type("Customer", customer.clone());
        let nested = compile_ok(
            "data.customer is Customer && data.customer.id > 0",
            &nested_signature,
        );
        let nested_value = EvalValue::Struct(alloc::vec![(String::from("customer"), value,)]);
        assert_eq!(
            eval(
                &nested,
                &BTreeMap::from([(String::from("data"), nested_value)])
            )
            .expect("a fact about a dotted path types its descendants"),
            EvalValue::Bool(true)
        );
    }

    #[test]
    fn variant_structure_requires_explicit_narrowing() {
        let signature = Signature::new()
            .with("data", DataType::Variant)
            .with_type("Customer", customer_type());
        for source in ["data.name", "data['name']"] {
            let diagnostics = compile(source, &signature)
                .expect_err("dynamic structure cannot be inspected without a type");
            assert_eq!(diagnostics.len(), 1, "{source}: {diagnostics:?}");
            assert!(
                diagnostics[0].message.contains("narrow"),
                "{source}: {diagnostics:?}"
            );
        }
        assert!(
            compile("data is Customer && data.name != ''", &signature).is_ok(),
            "a positive type fact makes the member statically visible"
        );
    }

    #[test]
    fn decimal_narrowing_uses_the_declared_scale_in_expressions() {
        let money = DataType::Decimal(
            crate::datatype::DecimalType::new(10, 2).expect("a fixed decimal type"),
        );
        let signature = Signature::new()
            .with("data", DataType::Variant)
            .with_type("Money", money.clone());
        let converted = compile_ok("data as Money", &signature);
        let (document_decimal, consumed) =
            crate::json::parse_value("5.50").expect("an exact JSON document number");
        assert_eq!(consumed, 4);

        for (source, expected) in [
            (
                EvalValue::Int(5),
                EvalValue::Decimal {
                    unscaled: 500,
                    scale: 2,
                },
            ),
            (
                EvalValue::from(document_decimal),
                EvalValue::Decimal {
                    unscaled: 550,
                    scale: 2,
                },
            ),
            (
                EvalValue::Decimal {
                    unscaled: 550,
                    scale: 2,
                },
                EvalValue::Decimal {
                    unscaled: 550,
                    scale: 2,
                },
            ),
        ] {
            assert_eq!(
                eval(
                    &converted,
                    &BTreeMap::from([(String::from("data"), source)])
                )
                .expect("the exact value rescales without rounding"),
                expected
            );
        }

        let constructor = compile_ok("decimal(\"5.50\") as Money", &signature);
        assert_eq!(
            eval(&constructor, &BTreeMap::new())
                .expect("the declared-scale constructor already matches Money"),
            EvalValue::Decimal {
                unscaled: 550,
                scale: 2,
            }
        );

        let inexact = BTreeMap::from([(
            String::from("data"),
            EvalValue::Decimal {
                unscaled: 551,
                scale: 3,
            },
        )]);
        assert!(matches!(
            eval(&converted, &inexact)
                .expect_err("a non-zero discarded digit must refuse")
                .kind,
            ErrorKind::Narrowing(NarrowingFailure::NoMatch { .. })
        ));
    }

    #[test]
    fn structural_and_tagged_unions_use_the_same_narrowing_path() {
        let structural = DataType::structural_union(alloc::vec![DataType::Text, DataType::INT])
            .expect("text and int are disjoint");
        let structural_signature = Signature::new()
            .with("data", DataType::Variant)
            .with_type("Payload", structural.clone());
        let structural_tree = compile_ok("data as Payload", &structural_signature);
        let structural_value = eval(
            &structural_tree,
            &BTreeMap::from([(String::from("data"), EvalValue::Text(String::from("hello")))]),
        )
        .expect("exactly one structural branch matches");
        let predicate = compile_ok(
            "data is text && size(data) == 5",
            &Signature::new().with("data", structural.clone()),
        );
        let wrapped = structural
            .narrow(Value::Text(String::from("hello")), None)
            .expect("one branch matches");
        assert_eq!(
            eval(
                &predicate,
                &BTreeMap::from([(String::from("data"), EvalValue::from(wrapped))])
            )
            .expect("the selected structural value is visible after `is`"),
            EvalValue::Bool(true)
        );
        let EvalValue::StructuralUnion { branch, value } = structural_value else {
            panic!("narrowing records the selected structural branch");
        };
        let DataType::StructuralUnion(branches) = structural else {
            panic!("the target is a structural union");
        };
        assert_eq!(
            branches.branches()[usize::from(branch)],
            DataType::Text,
            "the index addresses canonical branch order"
        );
        assert_eq!(*value, EvalValue::Text(String::from("hello")));

        let exited_payload =
            crate::datatype::StructType::new(alloc::vec![crate::datatype::TypeField::new(
                "exit_code",
                DataType::INT
            ),])
            .expect("one field");
        let failed_payload =
            crate::datatype::StructType::new(alloc::vec![crate::datatype::TypeField::new(
                "reason",
                DataType::Text
            ),])
            .expect("one field");
        let exit_type = DataType::TaggedUnion(
            crate::datatype::TaggedUnionType::new(alloc::vec![
                crate::datatype::TaggedCase::new("Exited", exited_payload).expect("a named case"),
                crate::datatype::TaggedCase::new("Failed", failed_payload).expect("a named case"),
            ])
            .expect("two tagged cases"),
        );
        let exit_signature = Signature::new().with("data", exit_type);
        let exit_guard = compile_ok("data is Exited && data.exit_code == 0", &exit_signature);
        let exit_value = EvalValue::TaggedUnion {
            case: String::from("Exited"),
            payload: alloc::vec![(String::from("exit_code"), EvalValue::Int(0))],
        };
        assert_eq!(
            eval(
                &exit_guard,
                &BTreeMap::from([(String::from("data"), exit_value)])
            )
            .expect("the selected case exposes its payload on the true branch"),
            EvalValue::Bool(true)
        );
        assert_eq!(
            eval(
                &exit_guard,
                &BTreeMap::from([(
                    String::from("data"),
                    EvalValue::TaggedUnion {
                        case: String::from("Exited"),
                        payload: alloc::vec![],
                    },
                )])
            )
            .expect("a malformed case payload is an ordinary failed predicate"),
            EvalValue::Bool(false)
        );

        let exited = compile_ok("data as Exited", &exit_signature);
        let failed_value = EvalValue::TaggedUnion {
            case: String::from("Failed"),
            payload: alloc::vec![(
                String::from("reason"),
                EvalValue::Text(String::from("boom")),
            )],
        };
        assert!(matches!(
            eval(
                &exited,
                &BTreeMap::from([(String::from("data"), failed_value)])
            )
            .expect_err("a different tagged case cannot pass `as Exited`")
            .kind,
            ErrorKind::Narrowing(NarrowingFailure::CaseMismatch { expected, found })
                if expected == "Exited" && found == "Failed"
        ));
    }

    #[test]
    fn an_external_discriminator_tags_a_variant_before_use() {
        let payload = crate::datatype::StructType::new(alloc::vec![
            crate::datatype::TypeField::new("kind", DataType::Text),
            crate::datatype::TypeField::new("id", DataType::INT),
        ])
        .expect("two fields");
        let event = DataType::TaggedUnion(
            crate::datatype::TaggedUnionType::new(alloc::vec![
                crate::datatype::TaggedCase::new("Customer", payload).expect("a named case"),
            ])
            .expect("one tagged case"),
        );
        let discriminator =
            Discriminator::new("kind", BTreeMap::new()).expect("the field name is non-empty");
        let signature = Signature::new()
            .with("data", DataType::Variant)
            .with_discriminated_type("Event", event, discriminator)
            .expect("the mapping only names declared cases");
        let tree = compile_ok("data as Event", &signature);
        let result = eval(
            &tree,
            &BTreeMap::from([(
                String::from("data"),
                EvalValue::Struct(alloc::vec![
                    (
                        String::from("kind"),
                        EvalValue::Text(String::from("Customer")),
                    ),
                    (String::from("id"), EvalValue::Int(7)),
                ]),
            )]),
        )
        .expect("the declared discriminator selects the case");
        assert!(matches!(
            result,
            EvalValue::TaggedUnion { case, .. } if case == "Customer"
        ));

        let predicate = compile_ok("data is Event", &signature);
        assert_eq!(
            eval(
                &predicate,
                &BTreeMap::from([(
                    String::from("data"),
                    EvalValue::Struct(alloc::vec![
                        (
                            String::from("kind"),
                            EvalValue::Text(String::from("Unknown")),
                        ),
                        (String::from("id"), EvalValue::Int(7)),
                    ]),
                )])
            )
            .expect("an unknown external discriminant is an ordinary failed predicate"),
            EvalValue::Bool(false)
        );

        let invalid = Discriminator::new(
            "kind",
            BTreeMap::from([(String::from("external"), String::from("Missing"))]),
        )
        .expect("the mapping is syntactically valid");
        assert!(matches!(
            Signature::new().with_discriminated_type(
                "Event",
                tree.ty.clone(),
                invalid,
            ),
            Err(NarrowTypeError::UnknownMappedCase(case)) if case == "Missing"
        ));
    }

    #[test]
    fn unknown_names_fail_at_compile_time() {
        let diagnostics = compile("mystery + 1", &Signature::new()).unwrap_err();
        assert!(diagnostics[0].message.contains("not a visible name"));
        let typed = compile(
            "session.key + 1",
            &Signature::new().with("session.key", DataType::Text),
        );
        assert!(typed.is_err(), "text + int must not type-check");
    }

    #[test]
    fn collections_index_and_size() {
        assert_eq!(eval_const("[1, 2, 3][1]"), EvalValue::Int(2));
        assert_eq!(eval_const("{'a': 1, 'b': 2}['b']"), EvalValue::Int(2));
        assert_eq!(eval_const("size([1, 2, 3])"), EvalValue::Int(3));
        assert_eq!(eval_const("size(\"héllo\")"), EvalValue::Int(5));
        assert_eq!(eval_const("size({'a': 1})"), EvalValue::Int(1));
    }

    #[test]
    fn durations_follow_the_go_format() {
        assert_eq!(
            eval_const("duration(\"2h\")"),
            EvalValue::Duration(7_200_000_000_000)
        );
        assert_eq!(
            eval_const("duration(\"2h30m\")"),
            EvalValue::Duration(9_000_000_000_000)
        );
        assert_eq!(
            eval_const("duration(\"1.5s\")"),
            EvalValue::Duration(1_500_000_000)
        );
        assert_eq!(
            eval_const("duration(\"-10ms\")"),
            EvalValue::Duration(-10_000_000)
        );
        assert_eq!(
            eval_const("duration(\"1h\") + duration(\"30m\")"),
            EvalValue::Duration(5_400_000_000_000)
        );
        let tree = compile_ok("duration(\"2d\")", &Signature::new());
        assert!(eval(&tree, &BTreeMap::new()).is_err(), "`d` is not a unit");
        let minutes = compile_ok("duration(\"5min\")", &Signature::new());
        assert!(
            eval(&minutes, &BTreeMap::new()).is_err(),
            "`min` is not a unit"
        );
    }

    #[test]
    fn timestamps_are_utc_rfc3339() {
        assert_eq!(
            eval_const("timestamp(\"1970-01-01T00:00:00Z\")"),
            EvalValue::Timestamp(0)
        );
        assert_eq!(
            eval_const("timestamp(\"2026-01-05T00:00:00Z\")"),
            EvalValue::Timestamp(1_767_571_200_000_000_000)
        );
        assert_eq!(
            eval_const("timestamp(\"2026-01-05T01:00:00Z\") - timestamp(\"2026-01-05T00:00:00Z\")"),
            EvalValue::Duration(3_600_000_000_000)
        );
        let offset = compile_ok(
            "timestamp(\"2026-01-05T00:00:00+01:00\")",
            &Signature::new(),
        );
        assert!(eval(&offset, &BTreeMap::new()).is_err(), "UTC only");
    }

    #[test]
    fn the_guard_rails_refuse_explicitly() {
        let cases: &[(&str, &str)] = &[
            ("data.?text", "outside this build's subset"),
            ("items.map(i, i * 2)", "outside this build's subset"),
            ("has(data.text)", "macro is outside"),
            ("1 in [1]", "outside this build's subset"),
        ];
        let signature = Signature::new()
            .with("data", customer_type())
            .with("items", DataType::array(DataType::INT));
        for (text, needle) in cases {
            let diagnostics = compile(text, &signature).unwrap_err();
            assert!(
                diagnostics[0].message.contains(needle),
                "`{text}` → {diagnostics:?}"
            );
        }
    }

    #[test]
    fn now_is_an_explicit_context_function() {
        let signature = Signature::new().with("now", DataType::TIMESTAMP);
        let tree = compile("now()", &signature).expect("the handler context supplies time");
        assert_eq!(tree.root, Expr::Path(vec![String::from("now")]));
        let bindings = BTreeMap::from([(String::from("now"), EvalValue::Timestamp(1_234_567_890))]);
        assert_eq!(
            eval(&tree, &bindings).expect("the injected instant evaluates"),
            EvalValue::Timestamp(1_234_567_890)
        );

        for (text, needle) in [("now", "write `now()`"), ("now(1)", "takes no arguments")] {
            let diagnostics = compile(text, &signature).expect_err("the spelling is invalid");
            assert!(diagnostics[0].message.contains(needle), "{diagnostics:?}");
        }
        let diagnostics = compile("now()", &Signature::new())
            .expect_err("a plan-time expression has no logical instant");
        assert!(
            diagnostics[0]
                .message
                .contains("`now` is not a visible name")
        );
    }

    #[test]
    fn boot_references_reuse_expression_lexing_but_never_become_calls() {
        let environment = parse_boot_reference("env( /* trivia */ 'SERVICE_URL' )")
            .unwrap_or_else(|diagnostics| panic!("env boot reference: {diagnostics:?}"))
            .expect("one boot reference");
        let secret = parse_boot_reference("secret(\"providers/openai/token\")")
            .unwrap_or_else(|diagnostics| panic!("secret boot reference: {diagnostics:?}"))
            .expect("one boot reference");
        assert!(matches!(
            environment,
            crate::deployment::BootReference::Environment(ref name)
                if name.as_str() == "SERVICE_URL"
        ));
        assert!(matches!(
            secret,
            crate::deployment::BootReference::Secret(ref reference)
                if reference.as_str() == "providers/openai/token"
        ));
        let tree = boot_reference_tree(&secret, DataType::Text);
        assert_eq!(boot_reference(&tree.root), Ok(Some(secret)));
        assert!(parse_boot_reference("secret(\"token\") + \"leak\"").is_err());
        assert_eq!(parse_boot_reference("size(data)"), Ok(None));
    }

    #[test]
    fn boot_reference_functions_report_their_declared_scope() {
        for (source, setting) in [
            ("secret(\"logical/path\")", "HttpClient.auth_bearer"),
            ("env(\"SERVICE_URL\")", "HttpClient.url"),
        ] {
            let diagnostics = compile(source, &Signature::new())
                .expect_err("a boot reference is not an ordinary expression call");
            assert!(
                diagnostics[0].message.contains("admitted only")
                    && diagnostics[0].message.contains(setting),
                "{diagnostics:?}"
            );
            assert!(!diagnostics[0].message.contains("unknown function"));
        }
    }

    #[test]
    fn duplicate_map_keys_are_refused_statically() {
        let diagnostics = compile("{'a': 1, 'a': 2}", &Signature::new()).unwrap_err();
        assert!(diagnostics[0].message.starts_with("duplicate map key"));
    }

    #[test]
    fn timestamps_have_fixed_widths_and_never_panic() {
        // RFC 3339 widths are fixed; and no user string may panic the
        // evaluator — the extreme year is a value error, not an abort.
        for bad in [
            "9223372036854775807-01-01T00:00:00Z",
            "2026-2-3T4:5:6Z",
            "26-01-01T00:00:00Z",
            "2026-01-01T00:00:00",
        ] {
            let tree = compile_ok(&format!("timestamp(\"{bad}\")"), &Signature::new());
            assert!(eval(&tree, &BTreeMap::new()).is_err(), "{bad} must refuse");
        }
    }

    #[test]
    fn timestamps_respect_the_calendar() {
        for bad in [
            "2026-02-31T00:00:00Z",
            "2026-04-31T00:00:00Z",
            "2025-02-29T00:00:00Z",
        ] {
            let tree = compile_ok(&format!("timestamp(\"{bad}\")"), &Signature::new());
            assert!(eval(&tree, &BTreeMap::new()).is_err(), "{bad} must refuse");
        }
        assert_eq!(
            eval_const("timestamp(\"2024-02-29T00:00:00Z\")"),
            eval_const("timestamp(\"2024-02-28T00:00:00Z\") + duration(\"24h\")")
        );
    }

    #[test]
    fn const_eval_reduces_initializers() {
        let tree = compile_ok(
            "retries * 2",
            &Signature::new().with("retries", DataType::INT),
        );
        let mut consts = BTreeMap::new();
        consts.insert(String::from("retries"), Value::Int(3));
        assert_eq!(const_eval(&tree, &consts).unwrap(), Value::Int(6));
        assert!(const_eval(&tree, &BTreeMap::new()).is_err());
    }

    #[test]
    fn host_constructed_tagged_values_are_checked_and_evaluated() {
        let payload = StructType::new(alloc::vec![TypeField::new("code", DataType::INT)])
            .expect("a closed payload");
        let union = TaggedUnionType::new(alloc::vec![
            crate::datatype::TaggedCase::new("Exited", payload).expect("a named case"),
        ])
        .expect("a tagged union");
        let root = Expr::Tagged {
            union: Box::new(union.clone()),
            case: String::from("Exited"),
            fields: alloc::vec![(String::from("code"), Expr::Literal(Value::Int(0)))],
        };
        assert_eq!(
            check(&root, &Signature::new(), "host constructor").expect("the tree is valid"),
            DataType::TaggedUnion(union.clone())
        );
        let tree = Tree {
            root: root.clone(),
            ty: DataType::TaggedUnion(union),
        };
        assert_eq!(
            const_eval(&tree, &BTreeMap::new()).expect("the constructor is constant"),
            Value::TaggedUnion(Box::new(crate::value::TaggedUnionValue {
                case: String::from("Exited"),
                payload: alloc::vec![(String::from("code"), Value::Int(0))],
            }))
        );

        let Expr::Tagged { union, case, .. } = root else {
            unreachable!("constructed above")
        };
        let forged = Expr::Tagged {
            union,
            case,
            fields: alloc::vec![(String::from("message"), Expr::Literal(Value::Int(0)))],
        };
        assert!(check(&forged, &Signature::new(), "forged").is_err());
    }
}
