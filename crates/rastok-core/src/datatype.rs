//! Logical payload types owned by the Rastok core.
//!
//! Expression engines, ports, state, metadata and codecs consume this
//! vocabulary; none of them defines a parallel type system. Every
//! parameterized or composite case is valid by construction. Global tree
//! budgets are checked before a type enters a resolved plan.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use crate::value::{Value, ValueDefect, ValueKind, ValuePathSegment, map_key_label};

/// Significant-digit budget of the canonical decimal128 value domain.
pub const DECIMAL_MAX_DIGITS: u32 = 38;
/// Largest carried scale of a schema-less exact decimal value.
pub const DECIMAL_MAX_SCALE: i32 = 6144;
/// Smallest carried scale of a schema-less exact decimal value.
pub const DECIMAL_MIN_SCALE: i32 = -6143;
/// Largest decimal scale admitted by a fixed `decimal(p,s)` type.
pub const DECIMAL_TYPE_MAX_SCALE: i32 = 38;
/// Smallest decimal scale admitted by a fixed `decimal(p,s)` type.
pub const DECIMAL_TYPE_MIN_SCALE: i32 = -38;
/// Largest fixed byte or array length in the v1 type algebra.
pub const MAX_FIXED_LENGTH: u32 = 0x7fff_ffff;
/// Maximum resolved type-tree nesting.
pub const MAX_TYPE_NESTING: usize = 128;
/// Maximum nodes in one expanded resolved type tree.
pub const MAX_TYPE_NODES: usize = 4096;
/// Maximum fields, symbols, branches or cases in one composite.
pub const MAX_TYPE_MEMBERS: usize = 256;

/// Whether an exact coefficient and scale inhabit Rastok's schema-less
/// decimal128 value domain. Origin-specific rules such as JSON minimal scale
/// remain at the constructor that owns that origin.
#[must_use]
pub fn decimal_in_domain(unscaled: i128, scale: i32) -> bool {
    (DECIMAL_MIN_SCALE..=DECIMAL_MAX_SCALE).contains(&scale)
        && decimal_digits(unscaled) <= DECIMAL_MAX_DIGITS
}

fn decimal_digits(value: i128) -> u32 {
    let magnitude = value.unsigned_abs();
    if magnitude == 0 {
        1
    } else {
        magnitude.ilog10() + 1
    }
}

fn narrow_decimal(target: DecimalType, value: Value) -> Option<Value> {
    let (unscaled, source_scale) = match value {
        Value::Int(value) => (i128::from(value), 0),
        Value::Uint(value) => (i128::from(value), 0),
        Value::Decimal { unscaled, scale } if decimal_in_domain(unscaled, scale) => {
            (unscaled, scale)
        }
        _ => return None,
    };
    let target_scale = i32::from(target.scale());
    let unscaled = rescale_decimal(unscaled, source_scale, target_scale)?;
    if decimal_digits(unscaled) > u32::from(target.precision()) {
        return None;
    }
    Some(Value::Decimal {
        unscaled,
        scale: target_scale,
    })
}

fn rescale_decimal(unscaled: i128, source_scale: i32, target_scale: i32) -> Option<i128> {
    if unscaled == 0 || source_scale == target_scale {
        return Some(unscaled);
    }
    if source_scale < target_scale {
        let places = u32::try_from(target_scale - source_scale).ok()?;
        return unscaled.checked_mul(10_i128.checked_pow(places)?);
    }
    let places = u32::try_from(source_scale - target_scale).ok()?;
    let divisor = 10_i128.checked_pow(places)?;
    (unscaled % divisor == 0).then_some(unscaled / divisor)
}

/// Width of a signed or unsigned integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntegerWidth {
    /// 8 bits.
    Bits8,
    /// 16 bits.
    Bits16,
    /// 32 bits.
    Bits32,
    /// 64 bits.
    Bits64,
}

impl IntegerWidth {
    /// Numeric bit count.
    #[must_use]
    pub const fn bits(self) -> u8 {
        match self {
            Self::Bits8 => 8,
            Self::Bits16 => 16,
            Self::Bits32 => 32,
            Self::Bits64 => 64,
        }
    }

    fn signed_accepts(self, value: i64) -> bool {
        match self {
            Self::Bits8 => i8::try_from(value).is_ok(),
            Self::Bits16 => i16::try_from(value).is_ok(),
            Self::Bits32 => i32::try_from(value).is_ok(),
            Self::Bits64 => true,
        }
    }

    fn unsigned_accepts(self, value: u64) -> bool {
        match self {
            Self::Bits8 => u8::try_from(value).is_ok(),
            Self::Bits16 => u16::try_from(value).is_ok(),
            Self::Bits32 => u32::try_from(value).is_ok(),
            Self::Bits64 => true,
        }
    }
}

/// Signedness and width of an integer type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntegerType {
    /// Signed two's-complement integer.
    Signed(IntegerWidth),
    /// Unsigned integer.
    Unsigned(IntegerWidth),
}

/// IEEE-754 binary floating-point width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FloatType {
    /// binary16.
    Binary16,
    /// binary32.
    Binary32,
    /// binary64.
    Binary64,
}

impl FloatType {
    /// Numeric bit count.
    #[must_use]
    pub const fn bits(self) -> u8 {
        match self {
            Self::Binary16 => 16,
            Self::Binary32 => 32,
            Self::Binary64 => 64,
        }
    }
}

/// A fixed decimal128 logical type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DecimalType {
    precision: u8,
    scale: i8,
}

impl DecimalType {
    /// Builds `decimal(precision, scale)` within the v1 decimal128 profile.
    ///
    /// # Errors
    ///
    /// Returns a parameter-specific refusal when precision or scale is out of
    /// the canonical domain.
    pub fn new(precision: u32, scale: i32) -> Result<Self, TypeError> {
        if !(1..=DECIMAL_MAX_DIGITS).contains(&precision) {
            return Err(TypeError::DecimalPrecision { precision });
        }
        if !(DECIMAL_TYPE_MIN_SCALE..=DECIMAL_TYPE_MAX_SCALE).contains(&scale) {
            return Err(TypeError::DecimalScale { scale });
        }
        Ok(Self {
            precision: u8::try_from(precision)
                .map_err(|_| TypeError::DecimalPrecision { precision })?,
            scale: i8::try_from(scale).map_err(|_| TypeError::DecimalScale { scale })?,
        })
    }

    /// Declared precision.
    #[must_use]
    pub const fn precision(self) -> u8 {
        self.precision
    }

    /// Declared signed scale.
    #[must_use]
    pub const fn scale(self) -> i8 {
        self.scale
    }
}

/// Variable or fixed binary logical type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BinaryType {
    /// Variable-size bytes.
    Variable,
    /// Exactly this many bytes.
    Fixed(ByteLength),
}

/// Valid fixed-binary length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteLength(u32);

impl ByteLength {
    /// Builds a non-zero fixed-binary length under the v1 bound.
    ///
    /// # Errors
    ///
    /// Zero and values beyond [`MAX_FIXED_LENGTH`] refuse.
    pub fn new(length: u32) -> Result<Self, TypeError> {
        if length == 0 || length > MAX_FIXED_LENGTH {
            return Err(TypeError::ByteLength { length });
        }
        Ok(Self(length))
    }

    /// Number of bytes.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Unit used by time, timestamps and durations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TimeUnit {
    /// Seconds.
    Second,
    /// Milliseconds.
    Millisecond,
    /// Microseconds.
    Microsecond,
    /// Nanoseconds.
    Nanosecond,
}

/// Unit used by dates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DateUnit {
    /// Days since the Unix epoch.
    Day,
    /// Milliseconds since the Unix epoch.
    Millisecond,
}

/// Unit used by intervals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntervalUnit {
    /// Year/month pair.
    YearMonth,
    /// Day/time pair.
    DayTime,
    /// Month/day/nanosecond triple.
    MonthDayNano,
    /// Unsigned month/day/millisecond triple used by Avro `duration`.
    MonthDayMillisUnsigned,
}

/// Timestamp timezone semantics.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TimeZone {
    /// UTC, with all accepted UTC aliases normalized to this case.
    Utc,
    /// A non-empty named timezone.
    Named(NamedTimeZone),
}

impl TimeZone {
    /// Builds a timezone from source spelling, normalizing UTC aliases.
    ///
    /// # Errors
    ///
    /// An empty timezone name refuses.
    pub fn named(name: impl Into<String>) -> Result<Self, TypeError> {
        let name = name.into();
        if name.is_empty() {
            return Err(TypeError::EmptyTimeZone);
        }
        if matches!(name.as_str(), "UTC" | "Etc/UTC" | "+00:00" | "utc") {
            Ok(Self::Utc)
        } else {
            Ok(Self::Named(NamedTimeZone(name)))
        }
    }
}

/// Canonical non-UTC timezone name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NamedTimeZone(String);

impl NamedTimeZone {
    /// Canonical text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Canonical temporal parameters for the supported Arrow and Avro profiles.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TemporalType {
    /// Calendar date.
    Date(DateUnit),
    /// Time of day.
    Time(TimeUnit),
    /// Instant with UTC or a named timezone.
    Timestamp {
        /// Resolution.
        unit: TimeUnit,
        /// Timezone semantics.
        zone: TimeZone,
    },
    /// Timezone-naive timestamp.
    TimestampNtz(TimeUnit),
    /// Elapsed duration.
    Duration(TimeUnit),
    /// Calendar interval.
    Interval(IntervalUnit),
}

/// One field in a closed struct type.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeField {
    name: String,
    ty: DataType,
}

impl TypeField {
    /// Builds a field. Empty names are admitted because external schemas may
    /// name such properties even though source identifiers cannot.
    #[must_use]
    pub fn new(name: impl Into<String>, ty: DataType) -> Self {
        Self {
            name: name.into(),
            ty,
        }
    }

    /// Semantic field name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Resolved field type.
    #[must_use]
    pub const fn ty(&self) -> &DataType {
        &self.ty
    }
}

/// Closed, canonical struct type.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StructType(Vec<TypeField>);

impl StructType {
    /// Sorts fields by UTF-8 name bytes and rejects duplicate names or the
    /// per-struct member budget.
    ///
    /// # Errors
    ///
    /// A duplicate name or oversized field table refuses.
    pub fn new(mut fields: Vec<TypeField>) -> Result<Self, TypeError> {
        member_budget("struct fields", fields.len())?;
        fields.sort_by(|a, b| a.name.cmp(&b.name));
        if let Some(name) = duplicate_adjacent(fields.iter().map(|field| field.name.as_str())) {
            return Err(TypeError::DuplicateStructField(name.to_string()));
        }
        Ok(Self(fields))
    }

    /// Canonically sorted fields.
    #[must_use]
    pub fn fields(&self) -> &[TypeField] {
        &self.0
    }
}

/// Homogeneous keyed collection type.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MapType {
    key: Box<DataType>,
    value: Box<DataType>,
}

impl MapType {
    /// Builds a map whose key has a total canonical encoding.
    ///
    /// # Errors
    ///
    /// Key types outside the v1 canonical-key domain refuse.
    pub fn new(key: DataType, value: DataType) -> Result<Self, TypeError> {
        if !key.is_map_key() {
            return Err(TypeError::InvalidMapKey(Box::new(key)));
        }
        Ok(Self {
            key: Box::new(key),
            value: Box::new(value),
        })
    }

    /// Key type.
    #[must_use]
    pub const fn key(&self) -> &DataType {
        &self.key
    }

    /// Value type.
    #[must_use]
    pub const fn value(&self) -> &DataType {
        &self.value
    }
}

/// Homogeneous ordered collection type.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArrayType {
    element: Box<DataType>,
    length: Option<ArrayLength>,
}

impl ArrayType {
    /// Variable-length array.
    #[must_use]
    pub fn variable(element: DataType) -> Self {
        Self {
            element: Box::new(element),
            length: None,
        }
    }

    /// Fixed-length array.
    ///
    /// # Errors
    ///
    /// Values beyond [`MAX_FIXED_LENGTH`] refuse; zero is valid.
    pub fn fixed(element: DataType, length: u32) -> Result<Self, TypeError> {
        Ok(Self {
            element: Box::new(element),
            length: Some(ArrayLength::new(length)?),
        })
    }

    /// Element type.
    #[must_use]
    pub const fn element(&self) -> &DataType {
        &self.element
    }

    /// Exact length, or `None` for a variable array.
    #[must_use]
    pub const fn length(&self) -> Option<ArrayLength> {
        self.length
    }
}

/// Valid fixed-array length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArrayLength(u32);

impl ArrayLength {
    /// Builds an exact array length; zero is valid.
    ///
    /// # Errors
    ///
    /// Values beyond [`MAX_FIXED_LENGTH`] refuse.
    pub fn new(length: u32) -> Result<Self, TypeError> {
        if length > MAX_FIXED_LENGTH {
            return Err(TypeError::ArrayLength { length });
        }
        Ok(Self(length))
    }

    /// Number of elements.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Closed set of enum symbols.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EnumType(Vec<String>);

impl EnumType {
    /// Sorts symbols canonically and rejects duplicates, emptiness, or the
    /// per-enum budget.
    ///
    /// # Errors
    ///
    /// The returned variant names the violated enum invariant.
    pub fn new(mut symbols: Vec<String>) -> Result<Self, TypeError> {
        member_budget("enum symbols", symbols.len())?;
        if symbols.is_empty() {
            return Err(TypeError::EmptyEnum);
        }
        symbols.sort();
        if symbols.first().is_some_and(String::is_empty) {
            return Err(TypeError::EmptyEnumSymbol);
        }
        if let Some(symbol) = duplicate_adjacent(symbols.iter().map(String::as_str)) {
            return Err(TypeError::DuplicateEnumSymbol(symbol.to_string()));
        }
        Ok(Self(symbols))
    }

    /// Canonically sorted symbols.
    #[must_use]
    pub fn symbols(&self) -> &[String] {
        &self.0
    }
}

/// Pairwise-disjoint, canonically sorted structural union.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StructuralUnionType(Vec<DataType>);

impl StructuralUnionType {
    /// Canonically sorted branches.
    #[must_use]
    pub fn branches(&self) -> &[DataType] {
        &self.0
    }
}

/// One named tagged-union case with a closed struct payload.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaggedCase {
    name: String,
    payload: StructType,
}

impl TaggedCase {
    /// Builds a named case.
    ///
    /// # Errors
    ///
    /// An empty discriminant refuses.
    pub fn new(name: impl Into<String>, payload: StructType) -> Result<Self, TypeError> {
        let name = name.into();
        if name.is_empty() {
            return Err(TypeError::EmptyTaggedCase);
        }
        Ok(Self { name, payload })
    }

    /// Semantic discriminant.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Closed payload shape.
    #[must_use]
    pub const fn payload(&self) -> &StructType {
        &self.payload
    }
}

/// Closed tagged union with name-stable cases.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaggedUnionType(Vec<TaggedCase>);

impl TaggedUnionType {
    /// Sorts cases by name and rejects duplicates, emptiness, or the member
    /// budget.
    ///
    /// # Errors
    ///
    /// The returned variant names the violated tagged-union invariant.
    pub fn new(mut cases: Vec<TaggedCase>) -> Result<Self, TypeError> {
        member_budget("tagged-union cases", cases.len())?;
        if cases.is_empty() {
            return Err(TypeError::EmptyTaggedUnion);
        }
        cases.sort_by(|a, b| a.name.cmp(&b.name));
        if let Some(name) = duplicate_adjacent(cases.iter().map(|case| case.name.as_str())) {
            return Err(TypeError::DuplicateTaggedCase(name.to_string()));
        }
        Ok(Self(cases))
    }

    /// Canonically sorted cases.
    #[must_use]
    pub fn cases(&self) -> &[TaggedCase] {
        &self.0
    }
}

/// Canonical Rastok logical type.
///
/// The enum contains no inference or recovery case. `Variant` is the one
/// explicit dynamic type. Source aliases such as `int` and `double` resolve
/// before entering this tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DataType {
    /// Null.
    Null,
    /// Boolean.
    Bool,
    /// Fixed-width signed or unsigned integer.
    Integer(IntegerType),
    /// Fixed-width IEEE-754 binary float.
    Float(FloatType),
    /// Fixed decimal128 type.
    Decimal(DecimalType),
    /// Exact integer-or-decimal dynamic numeric domain.
    Number,
    /// UTF-8 text.
    Text,
    /// Variable or fixed bytes.
    Bytes(BinaryType),
    /// Canonical temporal family.
    Temporal(TemporalType),
    /// UUID bytes with UUID semantics.
    Uuid,
    /// Closed named-field record.
    Struct(StructType),
    /// Homogeneous map.
    Map(MapType),
    /// Homogeneous ordered array.
    Array(ArrayType),
    /// Closed set of unit symbols.
    Enum(EnumType),
    /// Pairwise-disjoint structural union.
    StructuralUnion(StructuralUnionType),
    /// Name-discriminated union.
    TaggedUnion(TaggedUnionType),
    /// Explicitly dynamic self-describing tree.
    Variant,
}

/// A boundary-declared discriminator used when a self-describing struct is
/// narrowed to a tagged union.
///
/// The discriminator is projection metadata, not part of [`DataType`]: Avro
/// branch indexes and Arrow type codes can provide the same logical case by a
/// different physical mechanism. An expression tree carries this resolved
/// rule when it needs to narrow an untagged `variant` at runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discriminator {
    field: String,
    mapping: BTreeMap<String, String>,
}

impl Discriminator {
    /// Builds a field discriminator. An empty mapping means that the field's
    /// text is already the canonical tagged-case name.
    ///
    /// # Errors
    ///
    /// Empty field names, external spellings, or case names refuse.
    pub fn new(
        field: impl Into<String>,
        mapping: BTreeMap<String, String>,
    ) -> Result<Self, DiscriminatorError> {
        let field = field.into();
        if field.is_empty() {
            return Err(DiscriminatorError::EmptyField);
        }
        if let Some((external, case)) = mapping
            .iter()
            .find(|(external, case)| external.is_empty() || case.is_empty())
        {
            return Err(DiscriminatorError::EmptyMapping {
                external: external.clone(),
                case: case.clone(),
            });
        }
        Ok(Self { field, mapping })
    }

    /// Struct field carrying the external discriminant.
    #[must_use]
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Canonical case selected by one external text.
    #[must_use]
    pub fn case_for<'a>(&'a self, external: &'a str) -> Option<&'a str> {
        if self.mapping.is_empty() {
            Some(external)
        } else {
            self.mapping.get(external).map(String::as_str)
        }
    }

    /// Explicit external-to-case mapping. Empty means identity.
    #[must_use]
    pub fn mapping(&self) -> &BTreeMap<String, String> {
        &self.mapping
    }
}

/// Invalid discriminator declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscriminatorError {
    /// The field name is empty.
    EmptyField,
    /// One mapping endpoint is empty.
    EmptyMapping {
        /// External spelling.
        external: String,
        /// Canonical case name.
        case: String,
    },
}

impl fmt::Display for DiscriminatorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField => f.write_str("a discriminator field cannot be empty"),
            Self::EmptyMapping { external, case } => write!(
                f,
                "a discriminator mapping cannot contain an empty endpoint ({external:?} -> {case:?})"
            ),
        }
    }
}

impl core::error::Error for DiscriminatorError {}

/// Typed reason why an explicit narrowing could not produce its target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NarrowingFailure {
    /// The value conforms to no requested target or union branch.
    NoMatch {
        /// Requested logical target.
        target: Box<DataType>,
    },
    /// More than one branch accepted the same untagged value.
    AmbiguousUnion {
        /// Canonical indexes of the accepting branches.
        branches: Vec<u16>,
    },
    /// A tagged target needs a discriminator for an untagged value.
    DiscriminatorRequired,
    /// The configured discriminator field is absent.
    MissingDiscriminator {
        /// Required field.
        field: String,
    },
    /// The discriminator field is not text.
    NonTextDiscriminator {
        /// Offending field.
        field: String,
    },
    /// The external discriminant maps to no canonical case.
    UnknownDiscriminator {
        /// External value.
        value: String,
    },
    /// The discriminator selected a case absent from the target union.
    UnknownCase {
        /// Canonical case name.
        case: String,
    },
    /// A case-level conversion was applied to another tagged case.
    CaseMismatch {
        /// Requested canonical case.
        expected: String,
        /// Case actually carried by the value.
        found: String,
    },
    /// The selected case payload does not conform to its declared struct.
    CasePayload {
        /// Canonical case name.
        case: String,
        /// Precise payload refusal.
        violation: Box<ConformanceViolation>,
    },
    /// A type exceeded a representation bound that its constructor normally
    /// guarantees. This protects direct hostile construction.
    InvalidTarget,
}

impl NarrowingFailure {
    /// Whether `is` should report `false` rather than surface an execution
    /// fault. Ambiguity and invalid target/configuration remain errors.
    #[must_use]
    pub fn is_mismatch(&self) -> bool {
        matches!(
            self,
            Self::NoMatch { .. }
                | Self::MissingDiscriminator { .. }
                | Self::NonTextDiscriminator { .. }
                | Self::UnknownDiscriminator { .. }
                | Self::UnknownCase { .. }
                | Self::CaseMismatch { .. }
                | Self::CasePayload { .. }
        )
    }
}

impl fmt::Display for NarrowingFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoMatch { target } => write!(f, "the value does not conform to {target}"),
            Self::AmbiguousUnion { branches } => {
                write!(f, "the value matches multiple union branches {branches:?}")
            }
            Self::DiscriminatorRequired => {
                f.write_str("an untagged value needs a declared discriminator for this union")
            }
            Self::MissingDiscriminator { field } => {
                write!(f, "the value has no discriminator field `{field}`")
            }
            Self::NonTextDiscriminator { field } => {
                write!(f, "the discriminator field `{field}` is not text")
            }
            Self::UnknownDiscriminator { value } => {
                write!(f, "the discriminator value `{value}` maps to no case")
            }
            Self::UnknownCase { case } => {
                write!(f, "the discriminator selects undeclared case `{case}`")
            }
            Self::CaseMismatch { expected, found } => {
                write!(f, "case `{found}` cannot be narrowed to case `{expected}`")
            }
            Self::CasePayload { case, violation } => {
                write!(f, "case `{case}` has an invalid payload: {violation}")
            }
            Self::InvalidTarget => f.write_str("the narrowing target is malformed"),
        }
    }
}

impl core::error::Error for NarrowingFailure {}

// This key, rather than the source order of `DataType` variants, owns the
// deterministic order used to canonicalize structural unions. Format v7
// serializes branches in this already-explicit logical order. The derived
// `Ord` of every borrowed parameter type below is consequently identity
// material too; `canonical_parameter_order_is_pinned` freezes those orders.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum TypeOrderKey<'a> {
    Null,
    Bool,
    Integer(&'a IntegerType),
    Float(&'a FloatType),
    Decimal(&'a DecimalType),
    Number,
    Text,
    Bytes(&'a BinaryType),
    Temporal(&'a TemporalType),
    Uuid,
    Struct(&'a StructType),
    Map(&'a MapType),
    Array(&'a ArrayType),
    Enum(&'a EnumType),
    StructuralUnion(&'a StructuralUnionType),
    TaggedUnion(&'a TaggedUnionType),
    Variant,
}

impl PartialOrd for DataType {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DataType {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.order_key().cmp(&other.order_key())
    }
}

impl DataType {
    /// Friendly source alias `int = int32` used by the current executable
    /// subset.
    pub const INT: Self = Self::Integer(IntegerType::Signed(IntegerWidth::Bits32));
    /// Friendly source alias `long = int64`.
    pub const LONG: Self = Self::Integer(IntegerType::Signed(IntegerWidth::Bits64));
    /// Friendly source alias `uint = uint64`.
    pub const UINT: Self = Self::Integer(IntegerType::Unsigned(IntegerWidth::Bits64));
    /// Friendly source alias `float = float32`.
    pub const FLOAT: Self = Self::Float(FloatType::Binary32);
    /// Friendly source alias `double = float64`.
    pub const DOUBLE: Self = Self::Float(FloatType::Binary64);
    /// Variable opaque bytes.
    pub const BYTES: Self = Self::Bytes(BinaryType::Variable);
    /// Nanosecond duration, the friendly default.
    pub const DURATION: Self = Self::Temporal(TemporalType::Duration(TimeUnit::Nanosecond));
    /// Nanosecond UTC timestamp, the friendly default.
    pub const TIMESTAMP: Self = Self::Temporal(TemporalType::Timestamp {
        unit: TimeUnit::Nanosecond,
        zone: TimeZone::Utc,
    });

    fn order_key(&self) -> TypeOrderKey<'_> {
        match self {
            Self::Null => TypeOrderKey::Null,
            Self::Bool => TypeOrderKey::Bool,
            Self::Integer(integer) => TypeOrderKey::Integer(integer),
            Self::Float(float) => TypeOrderKey::Float(float),
            Self::Decimal(decimal) => TypeOrderKey::Decimal(decimal),
            Self::Number => TypeOrderKey::Number,
            Self::Text => TypeOrderKey::Text,
            Self::Bytes(binary) => TypeOrderKey::Bytes(binary),
            Self::Temporal(temporal) => TypeOrderKey::Temporal(temporal),
            Self::Uuid => TypeOrderKey::Uuid,
            Self::Struct(structure) => TypeOrderKey::Struct(structure),
            Self::Map(map) => TypeOrderKey::Map(map),
            Self::Array(array) => TypeOrderKey::Array(array),
            Self::Enum(enumeration) => TypeOrderKey::Enum(enumeration),
            Self::StructuralUnion(union) => TypeOrderKey::StructuralUnion(union),
            Self::TaggedUnion(union) => TypeOrderKey::TaggedUnion(union),
            Self::Variant => TypeOrderKey::Variant,
        }
    }

    /// Variable homogeneous array.
    #[must_use]
    pub fn array(element: DataType) -> Self {
        Self::Array(ArrayType::variable(element))
    }

    /// Text-keyed homogeneous map, the JSON/Avro-friendly common case.
    #[must_use]
    pub fn text_map(value: DataType) -> Self {
        Self::Map(MapType {
            key: Box::new(Self::Text),
            value: Box::new(value),
        })
    }

    /// Closed record with no fields.
    #[must_use]
    pub fn empty_struct() -> Self {
        Self::Struct(StructType(Vec::new()))
    }

    /// Whether every value admitted by `self` is also admitted by `target`.
    ///
    /// This is the one static compatibility relation used when values cross
    /// ports, enter state, or satisfy a declared metadata field. It is about
    /// logical values, not representation: a codec or an Arrow/Avro layout
    /// cannot make two otherwise distinct logical types assignable.
    #[must_use]
    pub fn is_assignable_to(&self, target: &Self) -> bool {
        if self == target || matches!(target, Self::Variant) {
            return true;
        }
        match (self, target) {
            // An explicitly dynamic value must be narrowed before entering a
            // precise type. It is deliberately not the top half of a
            // symmetric wildcard rule.
            (Self::Variant, _) => false,
            (Self::Integer(source), Self::Integer(target)) => integer_assignable(*source, *target),
            (Self::Integer(_) | Self::Decimal(_), Self::Number) => true,
            (Self::Float(source), Self::Float(target)) => source.bits() <= target.bits(),
            (Self::Bytes(BinaryType::Fixed(_)), Self::Bytes(BinaryType::Variable)) => true,
            (Self::Array(source), Self::Array(target)) => {
                array_length_assignable(source.length(), target.length())
                    && source.element().is_assignable_to(target.element())
            }
            (Self::Map(source), Self::Map(target)) => {
                source.key() == target.key() && source.value().is_assignable_to(target.value())
            }
            (Self::Struct(source), Self::Struct(target)) => {
                source.fields().len() == target.fields().len()
                    && source
                        .fields()
                        .iter()
                        .zip(target.fields())
                        .all(|(a, b)| a.name() == b.name() && a.ty().is_assignable_to(b.ty()))
            }
            (Self::Enum(source), Self::Enum(target)) => source
                .symbols()
                .iter()
                .all(|symbol| target.symbols().contains(symbol)),
            // A structural-union discriminant is an index into this exact
            // resolved union. Widening to another union cannot preserve that
            // index, so it requires an explicit narrowing that reselects the
            // target branch.
            (Self::StructuralUnion(_), Self::StructuralUnion(_)) => false,
            (Self::StructuralUnion(source), target) => source
                .branches()
                .iter()
                .all(|branch| branch.is_assignable_to(target)),
            (source, Self::StructuralUnion(target)) => target
                .branches()
                .iter()
                .any(|branch| source.is_assignable_to(branch)),
            (Self::TaggedUnion(source), Self::TaggedUnion(target)) => {
                source.cases().iter().all(|source_case| {
                    target.cases().iter().any(|target_case| {
                        source_case.name() == target_case.name()
                            && Self::Struct(source_case.payload().clone())
                                .is_assignable_to(&Self::Struct(target_case.payload().clone()))
                    })
                })
            }
            _ => false,
        }
    }

    /// Builds a structural union, flattening nested structural unions and
    /// sorting branches by the canonical type order.
    ///
    /// # Errors
    ///
    /// Fewer than two branches, duplicate/overlapping branches, member
    /// overflow, or a global type budget violation refuses.
    pub fn structural_union(branches: Vec<DataType>) -> Result<Self, TypeError> {
        let mut flat = Vec::new();
        for branch in branches {
            match branch {
                Self::StructuralUnion(union) => flat.extend(union.0),
                other => flat.push(other),
            }
        }
        member_budget("structural-union branches", flat.len())?;
        if flat.len() < 2 {
            return Err(TypeError::TooFewUnionBranches { count: flat.len() });
        }
        flat.sort();
        for pair in flat.windows(2) {
            if pair[0] == pair[1] {
                return Err(TypeError::DuplicateUnionBranch(Box::new(pair[0].clone())));
            }
        }
        for left in 0..flat.len() {
            for right in left + 1..flat.len() {
                if flat[left].overlaps(&flat[right]) {
                    return Err(TypeError::OverlappingUnionBranches {
                        left: Box::new(flat[left].clone()),
                        right: Box::new(flat[right].clone()),
                    });
                }
            }
        }
        let ty = Self::StructuralUnion(StructuralUnionType(flat));
        ty.validate()?;
        Ok(ty)
    }

    /// Builds `T?` as the same structural union as `T | null`.
    ///
    /// # Errors
    ///
    /// An already-nullable or otherwise overlapping union refuses.
    pub fn optional(self) -> Result<Self, TypeError> {
        Self::structural_union(alloc::vec![self, Self::Null])
    }

    /// Verifies global type budgets. Local parameter/order/uniqueness
    /// invariants are already held by the wrapper constructors.
    ///
    /// # Errors
    ///
    /// Returns the first exceeded global budget.
    pub fn validate(&self) -> Result<(), TypeError> {
        let mut stack = alloc::vec![(self, 1_usize)];
        let mut nodes = 0_usize;
        while let Some((ty, depth)) = stack.pop() {
            nodes = nodes.saturating_add(1);
            if nodes > MAX_TYPE_NODES {
                return Err(TypeError::NodeBudget {
                    nodes,
                    budget: MAX_TYPE_NODES,
                });
            }
            if depth > MAX_TYPE_NESTING {
                return Err(TypeError::NestingBudget {
                    depth,
                    budget: MAX_TYPE_NESTING,
                });
            }
            match ty {
                Self::Struct(structure) => {
                    stack.extend(
                        structure
                            .fields()
                            .iter()
                            .map(|field| (field.ty(), depth + 1)),
                    );
                }
                Self::Map(map) => {
                    stack.push((map.key(), depth + 1));
                    stack.push((map.value(), depth + 1));
                }
                Self::Array(array) => stack.push((array.element(), depth + 1)),
                Self::StructuralUnion(union) => {
                    stack.extend(union.branches().iter().map(|branch| (branch, depth + 1)));
                }
                Self::TaggedUnion(union) => {
                    for case in union.cases() {
                        let payload_depth = depth + 1;
                        nodes = nodes.saturating_add(1);
                        if nodes > MAX_TYPE_NODES {
                            return Err(TypeError::NodeBudget {
                                nodes,
                                budget: MAX_TYPE_NODES,
                            });
                        }
                        if payload_depth > MAX_TYPE_NESTING {
                            return Err(TypeError::NestingBudget {
                                depth: payload_depth,
                                budget: MAX_TYPE_NESTING,
                            });
                        }
                        stack.extend(
                            case.payload()
                                .fields()
                                .iter()
                                .map(|field| (field.ty(), payload_depth + 1)),
                        );
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Judges one materialized value against this logical type.
    ///
    /// `None` is conformance. A refusal carries the precise nested path and
    /// preserves the semantic reason for callers that render it differently.
    #[must_use]
    pub fn violation(&self, name: &str, value: &Value) -> Option<ConformanceViolation> {
        if let Err(error) = value.validate() {
            return Some(ConformanceViolation {
                name: String::from(name),
                path: error.path,
                expected: self.clone(),
                found: ValueKind::of(value),
                reason: ConformanceReason::MalformedValue(error.defect),
            });
        }
        self.violation_at(name, value, &mut Vec::new())
    }

    /// Whether `value` conforms to this logical type.
    #[must_use]
    pub fn accepts(&self, value: &Value) -> bool {
        self.violation("value", value).is_none()
    }

    /// Explicitly narrows one materialized value to this logical type.
    ///
    /// Precise targets preserve the value except that fixed decimals rescale
    /// exact integer/decimal carriers to their declared scale. Structural
    /// unions wrap the unique matching branch with its canonical index; an
    /// explicit conversion between different unions reselects that index.
    /// Tagged unions preserve an already tagged value or use the supplied
    /// boundary discriminator to tag an untagged struct. No first-match rule
    /// exists.
    ///
    /// # Errors
    ///
    /// Returns a typed mismatch, ambiguity, or discriminator refusal.
    pub fn narrow(
        &self,
        value: Value,
        discriminator: Option<&Discriminator>,
    ) -> Result<Value, NarrowingFailure> {
        match (self, value) {
            (Self::StructuralUnion(_), value @ Value::StructuralUnion { .. })
                if self.accepts(&value) =>
            {
                Ok(value)
            }
            (Self::StructuralUnion(_), Value::StructuralUnion { value, .. }) => {
                self.narrow(*value, discriminator)
            }
            (Self::StructuralUnion(union), value) => {
                let mut matching = Vec::new();
                for (index, branch) in union.branches().iter().enumerate() {
                    if branch.accepts(&value) {
                        let Ok(index) = u16::try_from(index) else {
                            return Err(NarrowingFailure::InvalidTarget);
                        };
                        matching.push(index);
                    }
                }
                match matching.as_slice() {
                    [] => Err(NarrowingFailure::NoMatch {
                        target: Box::new(self.clone()),
                    }),
                    [branch] => Ok(Value::StructuralUnion {
                        branch: *branch,
                        value: Box::new(value),
                    }),
                    _ => Err(NarrowingFailure::AmbiguousUnion { branches: matching }),
                }
            }
            (Self::Decimal(decimal), value) => {
                if let Some(value) = narrow_decimal(*decimal, value) {
                    Ok(value)
                } else {
                    Err(NarrowingFailure::NoMatch {
                        target: Box::new(self.clone()),
                    })
                }
            }
            (Self::TaggedUnion(_), value @ Value::TaggedUnion(_)) => {
                if self.accepts(&value) {
                    Ok(value)
                } else {
                    Err(NarrowingFailure::NoMatch {
                        target: Box::new(self.clone()),
                    })
                }
            }
            (Self::TaggedUnion(union), Value::Struct(payload)) => {
                let discriminator = discriminator.ok_or(NarrowingFailure::DiscriminatorRequired)?;
                let Some((_, discriminant)) = payload
                    .iter()
                    .find(|(name, _)| name == discriminator.field())
                else {
                    return Err(NarrowingFailure::MissingDiscriminator {
                        field: discriminator.field().to_string(),
                    });
                };
                let Value::Text(discriminant) = discriminant else {
                    return Err(NarrowingFailure::NonTextDiscriminator {
                        field: discriminator.field().to_string(),
                    });
                };
                let case = discriminator.case_for(discriminant).ok_or_else(|| {
                    NarrowingFailure::UnknownDiscriminator {
                        value: discriminant.clone(),
                    }
                })?;
                let selected = union
                    .cases()
                    .iter()
                    .find(|candidate| candidate.name() == case)
                    .ok_or_else(|| NarrowingFailure::UnknownCase {
                        case: case.to_string(),
                    })?;
                let payload_value = Value::Struct(payload.clone());
                if let Some(violation) =
                    Self::Struct(selected.payload().clone()).violation("payload", &payload_value)
                {
                    return Err(NarrowingFailure::CasePayload {
                        case: case.to_string(),
                        violation: Box::new(violation),
                    });
                }
                Ok(Value::TaggedUnion(Box::new(
                    crate::value::TaggedUnionValue {
                        case: case.to_string(),
                        payload,
                    },
                )))
            }
            (Self::TaggedUnion(_), _) => Err(NarrowingFailure::NoMatch {
                target: Box::new(self.clone()),
            }),
            (target, Value::StructuralUnion { value, .. }) if target.accepts(&value) => Ok(*value),
            (target, value) => {
                if target.accepts(&value) {
                    Ok(value)
                } else {
                    Err(NarrowingFailure::NoMatch {
                        target: Box::new(target.clone()),
                    })
                }
            }
        }
    }

    /// Tests the same narrowing rule used by [`Self::narrow`]. Ordinary
    /// mismatch is `false`; ambiguity and invalid configuration stay typed
    /// failures rather than pretending the predicate was meaningful.
    ///
    /// # Errors
    ///
    /// Returns a non-mismatch [`NarrowingFailure`].
    pub fn is_instance(
        &self,
        value: &Value,
        discriminator: Option<&Discriminator>,
    ) -> Result<bool, NarrowingFailure> {
        match self.narrow(value.clone(), discriminator) {
            Ok(_) => Ok(true),
            Err(error) if error.is_mismatch() => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn violation_at(
        &self,
        name: &str,
        value: &Value,
        path: &mut Vec<ValuePathSegment>,
    ) -> Option<ConformanceViolation> {
        match (self, value) {
            (Self::Variant, _) => None,
            (Self::Integer(IntegerType::Signed(width)), Value::Int(candidate)) => {
                if width.signed_accepts(*candidate) {
                    None
                } else {
                    refusal(
                        name,
                        path,
                        self,
                        value,
                        ConformanceReason::SignedIntegerOutOfRange {
                            value: *candidate,
                            bits: width.bits(),
                        },
                    )
                }
            }
            (Self::Integer(IntegerType::Unsigned(width)), Value::Uint(candidate)) => {
                if width.unsigned_accepts(*candidate) {
                    None
                } else {
                    refusal(
                        name,
                        path,
                        self,
                        value,
                        ConformanceReason::UnsignedIntegerOutOfRange {
                            value: *candidate,
                            bits: width.bits(),
                        },
                    )
                }
            }
            (Self::Float(width), Value::Double(candidate)) => {
                if float_accepts(*width, *candidate) {
                    None
                } else {
                    refusal(
                        name,
                        path,
                        self,
                        value,
                        ConformanceReason::FloatNotRepresentable { bits: width.bits() },
                    )
                }
            }
            (Self::Decimal(decimal), Value::Decimal { unscaled, scale }) => {
                if *scale != i32::from(decimal.scale()) {
                    refusal(
                        name,
                        path,
                        self,
                        value,
                        ConformanceReason::DecimalScale {
                            expected: decimal.scale(),
                            found: *scale,
                        },
                    )
                } else if decimal_digits(*unscaled) > u32::from(decimal.precision()) {
                    refusal(
                        name,
                        path,
                        self,
                        value,
                        ConformanceReason::DecimalPrecision {
                            precision: decimal.precision(),
                            digits: decimal_digits(*unscaled),
                        },
                    )
                } else {
                    None
                }
            }
            (Self::Number, Value::Int(_) | Value::Uint(_) | Value::Decimal { .. }) => None,
            (Self::Bool, Value::Bool(_))
            | (Self::Text, Value::Text(_))
            | (Self::Null, Value::Null) => None,
            (Self::Temporal(TemporalType::Date(unit)), Value::Date(days)) => {
                if date_accepts(*unit, *days) {
                    None
                } else {
                    refusal(
                        name,
                        path,
                        self,
                        value,
                        ConformanceReason::DateOutOfRange { unit: *unit },
                    )
                }
            }
            (Self::Temporal(TemporalType::Time(unit)), Value::Time(nanoseconds)) => {
                if time_accepts(*unit, *nanoseconds) {
                    None
                } else {
                    refusal(
                        name,
                        path,
                        self,
                        value,
                        ConformanceReason::TemporalPrecision { unit: *unit },
                    )
                }
            }
            (
                Self::Temporal(TemporalType::Timestamp { unit, .. }),
                Value::Timestamp(nanoseconds),
            )
            | (
                Self::Temporal(TemporalType::TimestampNtz(unit)),
                Value::TimestampNtz(nanoseconds),
            )
            | (Self::Temporal(TemporalType::Duration(unit)), Value::Duration(nanoseconds)) => {
                if time_unit_accepts(*unit, *nanoseconds) {
                    None
                } else {
                    refusal(
                        name,
                        path,
                        self,
                        value,
                        ConformanceReason::TemporalPrecision { unit: *unit },
                    )
                }
            }
            (
                Self::Temporal(TemporalType::Interval(unit)),
                Value::Interval {
                    months,
                    days,
                    nanoseconds,
                },
            ) => {
                if interval_accepts(*unit, *months, *days, *nanoseconds) {
                    None
                } else {
                    refusal(
                        name,
                        path,
                        self,
                        value,
                        ConformanceReason::IntervalShape { unit: *unit },
                    )
                }
            }
            (Self::Uuid, Value::Uuid(_)) => None,
            (Self::Bytes(binary), Value::Bytes(bytes)) => match binary {
                BinaryType::Variable => None,
                BinaryType::Fixed(length)
                    if usize::try_from(length.get()).ok() == Some(bytes.len()) =>
                {
                    None
                }
                BinaryType::Fixed(length) => refusal(
                    name,
                    path,
                    self,
                    value,
                    ConformanceReason::Length {
                        expected: length.get(),
                        found: bytes.len(),
                    },
                ),
            },
            (Self::Array(array), Value::Array(items)) => {
                if let Some(length) = array.length()
                    && usize::try_from(length.get()).ok() != Some(items.len())
                {
                    return refusal(
                        name,
                        path,
                        self,
                        value,
                        ConformanceReason::Length {
                            expected: length.get(),
                            found: items.len(),
                        },
                    );
                }
                for (index, item) in items.iter().enumerate() {
                    path.push(ValuePathSegment::Index(index));
                    let violation = array.element().violation_at(name, item, path);
                    path.pop();
                    if violation.is_some() {
                        return violation;
                    }
                }
                None
            }
            (Self::Map(map), Value::Map(entries)) if map.key() == &Self::Text => {
                for (key, item) in entries {
                    path.push(ValuePathSegment::MapValue(key.clone()));
                    let violation = map.value().violation_at(name, item, path);
                    path.pop();
                    if violation.is_some() {
                        return violation;
                    }
                }
                None
            }
            (Self::Map(map), Value::KeyedMap(entries)) if map.key() != &Self::Text => {
                for (key, item) in entries {
                    let label = map_key_label(key);
                    path.push(ValuePathSegment::MapKey(label.clone()));
                    let violation = map.key().violation_at(name, key, path);
                    path.pop();
                    if violation.is_some() {
                        return violation;
                    }
                    path.push(ValuePathSegment::MapValue(label));
                    let violation = map.value().violation_at(name, item, path);
                    path.pop();
                    if violation.is_some() {
                        return violation;
                    }
                }
                None
            }
            (Self::Struct(structure), Value::Struct(fields)) => {
                for expected in structure.fields() {
                    let Some((_, field_value)) =
                        fields.iter().find(|(name, _)| name == expected.name())
                    else {
                        return refusal(
                            name,
                            path,
                            self,
                            value,
                            ConformanceReason::MissingField(expected.name().to_string()),
                        );
                    };
                    path.push(ValuePathSegment::Field(expected.name().to_string()));
                    let violation = expected.ty().violation_at(name, field_value, path);
                    path.pop();
                    if violation.is_some() {
                        return violation;
                    }
                }
                if let Some((unexpected, _)) = fields
                    .iter()
                    .find(|(field, _)| !structure.fields().iter().any(|item| item.name() == field))
                {
                    return refusal(
                        name,
                        path,
                        self,
                        value,
                        ConformanceReason::UnexpectedField(unexpected.clone()),
                    );
                }
                None
            }
            (Self::Enum(enumeration), Value::Enum(symbol)) => {
                if enumeration.symbols().contains(symbol) {
                    None
                } else {
                    refusal(
                        name,
                        path,
                        self,
                        value,
                        ConformanceReason::UnknownEnumSymbol(symbol.clone()),
                    )
                }
            }
            (
                Self::StructuralUnion(union),
                Value::StructuralUnion {
                    branch,
                    value: selected,
                },
            ) => {
                let Some(branch_type) = union.branches().get(usize::from(*branch)) else {
                    return refusal(
                        name,
                        path,
                        self,
                        value,
                        ConformanceReason::UnionBranchOutOfRange {
                            branch: *branch,
                            branches: union.branches().len(),
                        },
                    );
                };
                path.push(ValuePathSegment::UnionBranch(*branch));
                let violation = branch_type.violation_at(name, selected, path);
                path.pop();
                violation
            }
            (Self::TaggedUnion(union), Value::TaggedUnion(tagged)) => {
                let Some(selected) = union
                    .cases()
                    .iter()
                    .find(|candidate| candidate.name() == tagged.case)
                else {
                    return refusal(
                        name,
                        path,
                        self,
                        value,
                        ConformanceReason::UnknownTaggedCase(tagged.case.clone()),
                    );
                };
                Self::Struct(selected.payload().clone()).violation_at(
                    name,
                    &Value::Struct(tagged.payload.clone()),
                    path,
                )
            }
            _ => refusal(name, path, self, value, ConformanceReason::TypeMismatch),
        }
    }

    fn is_map_key(&self) -> bool {
        matches!(
            self,
            Self::Bool
                | Self::Integer(_)
                | Self::Decimal(_)
                | Self::Text
                | Self::Bytes(_)
                | Self::Temporal(TemporalType::Date(_))
                | Self::Temporal(TemporalType::Time(_))
                | Self::Temporal(TemporalType::Timestamp { .. })
                | Self::Temporal(TemporalType::TimestampNtz(_))
                | Self::Temporal(TemporalType::Duration(_))
                | Self::Uuid
                | Self::Enum(_)
        )
    }

    fn overlaps(&self, other: &Self) -> bool {
        if self == other || matches!(self, Self::Variant) || matches!(other, Self::Variant) {
            return true;
        }
        match (self, other) {
            (Self::StructuralUnion(union), right) => {
                union.branches().iter().any(|left| left.overlaps(right))
            }
            (left, Self::StructuralUnion(union)) => {
                union.branches().iter().any(|right| left.overlaps(right))
            }
            (Self::Integer(_), Self::Integer(_))
            | (Self::Number, Self::Integer(_) | Self::Decimal(_))
            | (Self::Integer(_) | Self::Decimal(_), Self::Number) => true,
            (Self::Float(_), Self::Float(_)) => true,
            (Self::Decimal(left), Self::Decimal(right)) => left.scale() == right.scale(),
            (Self::Temporal(TemporalType::Date(_)), Self::Temporal(TemporalType::Date(_)))
            | (Self::Temporal(TemporalType::Time(_)), Self::Temporal(TemporalType::Time(_)))
            | (
                Self::Temporal(TemporalType::Timestamp { .. }),
                Self::Temporal(TemporalType::Timestamp { .. }),
            )
            | (
                Self::Temporal(TemporalType::TimestampNtz(_)),
                Self::Temporal(TemporalType::TimestampNtz(_)),
            )
            | (
                Self::Temporal(TemporalType::Duration(_)),
                Self::Temporal(TemporalType::Duration(_)),
            )
            | (
                Self::Temporal(TemporalType::Interval(_)),
                Self::Temporal(TemporalType::Interval(_)),
            ) => true,
            (Self::Bytes(BinaryType::Variable), Self::Bytes(_))
            | (Self::Bytes(_), Self::Bytes(BinaryType::Variable)) => true,
            (Self::Bytes(BinaryType::Fixed(left)), Self::Bytes(BinaryType::Fixed(right))) => {
                left == right
            }
            (Self::Array(left), Self::Array(right)) => arrays_overlap(left, right),
            // The empty map inhabits every map type.
            (Self::Map(_), Self::Map(_)) => true,
            (Self::Struct(left), Self::Struct(right)) => {
                left.fields().len() == right.fields().len()
                    && left
                        .fields()
                        .iter()
                        .zip(right.fields())
                        .all(|(a, b)| a.name() == b.name() && a.ty().overlaps(b.ty()))
            }
            (Self::Enum(left), Self::Enum(right)) => left
                .symbols()
                .iter()
                .any(|symbol| right.symbols().contains(symbol)),
            (Self::TaggedUnion(left), Self::TaggedUnion(right)) => left.cases().iter().any(|a| {
                right.cases().iter().any(|b| {
                    a.name() == b.name()
                        && Self::Struct(a.payload().clone())
                            .overlaps(&Self::Struct(b.payload().clone()))
                })
            }),
            _ => false,
        }
    }
}

const NANOS_PER_MILLISECOND: i64 = 1_000_000;
const NANOS_PER_MICROSECOND: i64 = 1_000;
const NANOS_PER_SECOND: i64 = 1_000_000_000;
const NANOS_PER_DAY: i64 = 86_400 * NANOS_PER_SECOND;
const MILLIS_PER_DAY: i64 = 86_400_000;

fn float_accepts(width: FloatType, value: f64) -> bool {
    if value.is_nan() {
        return true;
    }
    match width {
        FloatType::Binary64 => true,
        FloatType::Binary32 => f64::from(value as f32).to_bits() == value.to_bits(),
        FloatType::Binary16 => binary16_accepts(value),
    }
}

fn binary16_accepts(value: f64) -> bool {
    if value.is_infinite() || value == 0.0 {
        return true;
    }
    let bits = value.abs().to_bits();
    let encoded_exponent = ((bits >> 52) & 0x7ff) as i32;
    if encoded_exponent == 0 || encoded_exponent == 0x7ff {
        return false;
    }
    let exponent = encoded_exponent - 1023;
    if !(-24..=15).contains(&exponent) {
        return false;
    }
    let significand = (1_u64 << 52) | (bits & ((1_u64 << 52) - 1));
    let discarded = if exponent >= -14 {
        42
    } else {
        let Ok(discarded) = u32::try_from(52 - (exponent + 24)) else {
            return false;
        };
        discarded
    };
    significand & ((1_u64 << discarded) - 1) == 0
}

fn time_unit_nanoseconds(unit: TimeUnit) -> i64 {
    match unit {
        TimeUnit::Second => NANOS_PER_SECOND,
        TimeUnit::Millisecond => NANOS_PER_MILLISECOND,
        TimeUnit::Microsecond => NANOS_PER_MICROSECOND,
        TimeUnit::Nanosecond => 1,
    }
}

fn time_unit_name(unit: TimeUnit) -> &'static str {
    match unit {
        TimeUnit::Second => "second",
        TimeUnit::Millisecond => "millisecond",
        TimeUnit::Microsecond => "microsecond",
        TimeUnit::Nanosecond => "nanosecond",
    }
}

fn date_unit_name(unit: DateUnit) -> &'static str {
    match unit {
        DateUnit::Day => "day",
        DateUnit::Millisecond => "millisecond",
    }
}

fn interval_unit_name(unit: IntervalUnit) -> &'static str {
    match unit {
        IntervalUnit::YearMonth => "year_month",
        IntervalUnit::DayTime => "day_time",
        IntervalUnit::MonthDayNano => "month_day_nano",
        IntervalUnit::MonthDayMillisUnsigned => "month_day_millis_unsigned",
    }
}

fn time_unit_accepts(unit: TimeUnit, nanoseconds: i64) -> bool {
    nanoseconds % time_unit_nanoseconds(unit) == 0
}

fn date_accepts(unit: DateUnit, days: i64) -> bool {
    match unit {
        DateUnit::Day => i32::try_from(days).is_ok(),
        DateUnit::Millisecond => days.checked_mul(MILLIS_PER_DAY).is_some(),
    }
}

fn time_accepts(unit: TimeUnit, nanoseconds: i64) -> bool {
    (0..NANOS_PER_DAY).contains(&nanoseconds) && time_unit_accepts(unit, nanoseconds)
}

fn interval_accepts(unit: IntervalUnit, months: i64, days: i64, nanoseconds: i64) -> bool {
    match unit {
        IntervalUnit::YearMonth => i32::try_from(months).is_ok() && days == 0 && nanoseconds == 0,
        IntervalUnit::DayTime => {
            months == 0
                && i32::try_from(days).is_ok()
                && nanoseconds % NANOS_PER_MILLISECOND == 0
                && i32::try_from(nanoseconds / NANOS_PER_MILLISECOND).is_ok()
        }
        IntervalUnit::MonthDayNano => i32::try_from(months).is_ok() && i32::try_from(days).is_ok(),
        IntervalUnit::MonthDayMillisUnsigned => {
            u32::try_from(months).is_ok()
                && u32::try_from(days).is_ok()
                && nanoseconds % NANOS_PER_MILLISECOND == 0
                && u32::try_from(nanoseconds / NANOS_PER_MILLISECOND).is_ok()
        }
    }
}

fn arrays_overlap(left: &ArrayType, right: &ArrayType) -> bool {
    match (left.length(), right.length()) {
        (Some(a), Some(b)) if a != b => false,
        (Some(a), Some(_)) if a.get() == 0 => true,
        (None, None) => true,
        (None, Some(length)) | (Some(length), None) if length.get() == 0 => true,
        _ => left.element().overlaps(right.element()),
    }
}

fn integer_assignable(source: IntegerType, target: IntegerType) -> bool {
    match (source, target) {
        (IntegerType::Signed(source), IntegerType::Signed(target))
        | (IntegerType::Unsigned(source), IntegerType::Unsigned(target)) => {
            source.bits() <= target.bits()
        }
        // A u8 fits in i16, a u16 in i32, and a u32 in i64. No
        // supported signed width can contain all u64 values.
        (IntegerType::Unsigned(source), IntegerType::Signed(target)) => {
            source.bits() < target.bits()
        }
        (IntegerType::Signed(_), IntegerType::Unsigned(_)) => false,
    }
}

fn array_length_assignable(source: Option<ArrayLength>, target: Option<ArrayLength>) -> bool {
    match (source, target) {
        (_, None) => true,
        (Some(source), Some(target)) => source == target,
        (None, Some(_)) => false,
    }
}

fn duplicate_adjacent<'a>(mut names: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let mut previous = names.next()?;
    for name in names {
        if name == previous {
            return Some(name);
        }
        previous = name;
    }
    None
}

fn member_budget(kind: &'static str, count: usize) -> Result<(), TypeError> {
    if count > MAX_TYPE_MEMBERS {
        Err(TypeError::MemberBudget {
            kind,
            count,
            budget: MAX_TYPE_MEMBERS,
        })
    } else {
        Ok(())
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

/// A malformed or non-canonical logical type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeError {
    /// Decimal precision is outside `1..=38`.
    DecimalPrecision {
        /// Rejected precision.
        precision: u32,
    },
    /// Fixed decimal scale is outside `-38..=38`.
    DecimalScale {
        /// Rejected scale.
        scale: i32,
    },
    /// Fixed-binary length is zero or above the v1 bound.
    ByteLength {
        /// Rejected length.
        length: u32,
    },
    /// Fixed-array length is above the v1 bound.
    ArrayLength {
        /// Rejected length.
        length: u32,
    },
    /// A named timezone is empty.
    EmptyTimeZone,
    /// One struct repeats a field name.
    DuplicateStructField(String),
    /// An enum has no symbols.
    EmptyEnum,
    /// One enum symbol is empty.
    EmptyEnumSymbol,
    /// One enum repeats a symbol.
    DuplicateEnumSymbol(String),
    /// One tagged-union discriminant is empty.
    EmptyTaggedCase,
    /// A tagged union has no cases.
    EmptyTaggedUnion,
    /// One tagged union repeats a case name.
    DuplicateTaggedCase(String),
    /// A map key has no admitted total canonical ordering.
    InvalidMapKey(Box<DataType>),
    /// A structural union has fewer than two branches.
    TooFewUnionBranches {
        /// Supplied branch count after flattening.
        count: usize,
    },
    /// One structural-union branch is duplicated.
    DuplicateUnionBranch(Box<DataType>),
    /// Two structural-union branches accept at least one common value.
    OverlappingUnionBranches {
        /// First branch.
        left: Box<DataType>,
        /// Second branch.
        right: Box<DataType>,
    },
    /// One composite exceeds its member budget.
    MemberBudget {
        /// Composite member category.
        kind: &'static str,
        /// Observed count.
        count: usize,
        /// Admitted count.
        budget: usize,
    },
    /// The expanded tree exceeds its node budget.
    NodeBudget {
        /// Observed nodes at refusal.
        nodes: usize,
        /// Admitted nodes.
        budget: usize,
    },
    /// The expanded tree exceeds its nesting budget.
    NestingBudget {
        /// Observed depth.
        depth: usize,
        /// Admitted depth.
        budget: usize,
    },
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DecimalPrecision { precision } => {
                write!(f, "decimal precision {precision} is outside 1..=38")
            }
            Self::DecimalScale { scale } => {
                write!(f, "decimal scale {scale} is outside -38..=38")
            }
            Self::ByteLength { length } => write!(
                f,
                "fixed bytes length {length} is outside 1..={MAX_FIXED_LENGTH}"
            ),
            Self::ArrayLength { length } => {
                write!(f, "fixed array length {length} is above {MAX_FIXED_LENGTH}")
            }
            Self::EmptyTimeZone => f.write_str("a named timezone cannot be empty"),
            Self::DuplicateStructField(name) => {
                write!(f, "struct field `{name}` appears twice")
            }
            Self::EmptyEnum => f.write_str("an enum must declare at least one symbol"),
            Self::EmptyEnumSymbol => f.write_str("an enum symbol cannot be empty"),
            Self::DuplicateEnumSymbol(symbol) => {
                write!(f, "enum symbol `{symbol}` appears twice")
            }
            Self::EmptyTaggedCase => f.write_str("a tagged-union case name cannot be empty"),
            Self::EmptyTaggedUnion => f.write_str("a tagged union must declare at least one case"),
            Self::DuplicateTaggedCase(name) => {
                write!(f, "tagged-union case `{name}` appears twice")
            }
            Self::InvalidMapKey(ty) => write!(f, "{ty} is not a canonical map-key type"),
            Self::TooFewUnionBranches { count } => {
                write!(
                    f,
                    "a structural union needs at least two branches, found {count}"
                )
            }
            Self::DuplicateUnionBranch(ty) => {
                write!(f, "structural-union branch `{ty}` appears twice")
            }
            Self::OverlappingUnionBranches { left, right } => {
                write!(
                    f,
                    "structural-union branches `{left}` and `{right}` overlap"
                )
            }
            Self::MemberBudget {
                kind,
                count,
                budget,
            } => write!(f, "{kind} count {count} exceeds budget {budget}"),
            Self::NodeBudget { nodes, budget } => {
                write!(f, "type node count {nodes} exceeds budget {budget}")
            }
            Self::NestingBudget { depth, budget } => {
                write!(f, "type nesting {depth} exceeds budget {budget}")
            }
        }
    }
}

impl core::error::Error for TypeError {}

impl fmt::Display for DataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => f.write_str("null"),
            Self::Bool => f.write_str("bool"),
            Self::Integer(IntegerType::Signed(width)) => write!(f, "int{}", width.bits()),
            Self::Integer(IntegerType::Unsigned(width)) => write!(f, "uint{}", width.bits()),
            Self::Float(width) => write!(f, "float{}", width.bits()),
            Self::Decimal(decimal) => {
                write!(f, "decimal({},{})", decimal.precision(), decimal.scale())
            }
            Self::Number => f.write_str("number"),
            Self::Text => f.write_str("text"),
            Self::Bytes(BinaryType::Variable) => f.write_str("bytes"),
            Self::Bytes(BinaryType::Fixed(length)) => write!(f, "bytes({})", length.get()),
            Self::Temporal(temporal) => write!(f, "{temporal:?}"),
            Self::Uuid => f.write_str("uuid"),
            Self::Struct(_) => f.write_str("struct"),
            Self::Map(map) => write!(f, "map<{},{}>", map.key(), map.value()),
            Self::Array(array) => match array.length() {
                Some(length) => write!(f, "array<{},{}>", array.element(), length.get()),
                None => write!(f, "array<{}>", array.element()),
            },
            Self::Enum(_) => f.write_str("enum"),
            Self::StructuralUnion(_) => f.write_str("structural union"),
            Self::TaggedUnion(_) => f.write_str("tagged union"),
            Self::Variant => f.write_str("variant"),
        }
    }
}

/// Why a materialized value failed its logical type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConformanceReason {
    /// The value violates the canonical value algebra before type judgement.
    MalformedValue(ValueDefect),
    /// The value category cannot inhabit the declared type.
    TypeMismatch,
    /// A signed integer lies outside the declared width.
    SignedIntegerOutOfRange {
        /// Offending value.
        value: i64,
        /// Declared width.
        bits: u8,
    },
    /// An unsigned integer lies outside the declared width.
    UnsignedIntegerOutOfRange {
        /// Offending value.
        value: u64,
        /// Declared width.
        bits: u8,
    },
    /// A floating value cannot be represented exactly at the declared width.
    FloatNotRepresentable {
        /// Declared IEEE-754 width.
        bits: u8,
    },
    /// A canonical date lies outside the storage domain of its declared unit.
    DateOutOfRange {
        /// Declared date unit.
        unit: DateUnit,
    },
    /// A canonical temporal value is not aligned to its declared unit, or a
    /// time-of-day lies outside one day.
    TemporalPrecision {
        /// Declared temporal unit.
        unit: TimeUnit,
    },
    /// Interval components do not inhabit the declared interval family.
    IntervalShape {
        /// Declared interval family.
        unit: IntervalUnit,
    },
    /// A fixed binary or array has the wrong length.
    Length {
        /// Declared length.
        expected: u32,
        /// Observed length.
        found: usize,
    },
    /// Decimal value scale differs from the declared scale.
    DecimalScale {
        /// Declared scale.
        expected: i8,
        /// Carried scale.
        found: i32,
    },
    /// Decimal coefficient exceeds the declared precision.
    DecimalPrecision {
        /// Declared precision.
        precision: u8,
        /// Coefficient digits.
        digits: u32,
    },
    /// A closed struct lacks a declared field.
    MissingField(String),
    /// A closed struct contains an undeclared field.
    UnexpectedField(String),
    /// An enum value carries no declared symbol.
    UnknownEnumSymbol(String),
    /// A structural-union value points outside the declared branch table.
    UnionBranchOutOfRange {
        /// Carried branch index.
        branch: u16,
        /// Number of declared branches.
        branches: usize,
    },
    /// A tagged-union value carries no declared case.
    UnknownTaggedCase(String),
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
                ValuePathSegment::MapKey(key) => write!(f, "<key:{key}>")?,
                ValuePathSegment::MapValue(key) => write!(f, "[{key:?}]")?,
                ValuePathSegment::UnionBranch(branch) => write!(f, "<branch:{branch}>")?,
                ValuePathSegment::TaggedField { case, field } => write!(f, "<{case}>.{field}")?,
            }
        }
        match &self.reason {
            ConformanceReason::MalformedValue(defect) => {
                write!(f, "` is not a valid materialized value: {defect}")
            }
            ConformanceReason::TypeMismatch => write!(
                f,
                "` is declared {}; a {} cannot enter it",
                self.expected, self.found
            ),
            ConformanceReason::SignedIntegerOutOfRange { value, bits } => {
                write!(f, "` is an int{bits}; {value} leaves that range")
            }
            ConformanceReason::UnsignedIntegerOutOfRange { value, bits } => {
                write!(f, "` is a uint{bits}; {value} leaves that range")
            }
            ConformanceReason::FloatNotRepresentable { bits } => {
                write!(
                    f,
                    "` is a float{bits}; the value is not exactly representable"
                )
            }
            ConformanceReason::DateOutOfRange { unit } => write!(
                f,
                "` does not fit the {} date representation",
                date_unit_name(*unit)
            ),
            ConformanceReason::TemporalPrecision { unit } => write!(
                f,
                "` is not a valid value at {} resolution",
                time_unit_name(*unit)
            ),
            ConformanceReason::IntervalShape { unit } => write!(
                f,
                "` does not have the component shape required by {}",
                interval_unit_name(*unit)
            ),
            ConformanceReason::Length { expected, found } => {
                write!(f, "` has length {found}; the type requires {expected}")
            }
            ConformanceReason::DecimalScale { expected, found } => {
                write!(
                    f,
                    "` carries decimal scale {found}; the type requires {expected}"
                )
            }
            ConformanceReason::DecimalPrecision { precision, digits } => write!(
                f,
                "` carries {digits} decimal digits; precision is {precision}"
            ),
            ConformanceReason::MissingField(field) => {
                write!(f, "` is missing declared field `{field}`")
            }
            ConformanceReason::UnexpectedField(field) => {
                write!(f, "` contains undeclared field `{field}`")
            }
            ConformanceReason::UnknownEnumSymbol(symbol) => {
                write!(f, "` contains undeclared enum symbol `{symbol}`")
            }
            ConformanceReason::UnionBranchOutOfRange { branch, branches } => write!(
                f,
                "` selects union branch {branch}; the type declares {branches} branches"
            ),
            ConformanceReason::UnknownTaggedCase(case) => {
                write!(f, "` contains undeclared tagged case `{case}`")
            }
        }
    }
}

impl core::error::Error for ConformanceViolation {}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeMap;

    #[test]
    fn canonical_parameter_order_is_pinned() {
        use core::cmp::Ordering::Less;

        let ordered_pairs = [
            (
                DataType::Integer(IntegerType::Signed(IntegerWidth::Bits8)),
                DataType::Integer(IntegerType::Signed(IntegerWidth::Bits64)),
            ),
            (
                DataType::Integer(IntegerType::Signed(IntegerWidth::Bits64)),
                DataType::Integer(IntegerType::Unsigned(IntegerWidth::Bits8)),
            ),
            (
                DataType::Float(FloatType::Binary16),
                DataType::Float(FloatType::Binary64),
            ),
            (
                DataType::Bytes(BinaryType::Variable),
                DataType::Bytes(BinaryType::Fixed(
                    ByteLength::new(1).expect("one byte is valid"),
                )),
            ),
            (
                DataType::Temporal(TemporalType::Date(DateUnit::Millisecond)),
                DataType::Temporal(TemporalType::Time(TimeUnit::Second)),
            ),
            (
                DataType::Temporal(TemporalType::Timestamp {
                    unit: TimeUnit::Nanosecond,
                    zone: TimeZone::Utc,
                }),
                DataType::Temporal(TemporalType::TimestampNtz(TimeUnit::Second)),
            ),
            (
                DataType::Temporal(TemporalType::Duration(TimeUnit::Nanosecond)),
                DataType::Temporal(TemporalType::Interval(IntervalUnit::YearMonth)),
            ),
        ];
        for (left, right) in ordered_pairs {
            assert_eq!(left.cmp(&right), Less, "{left} must sort before {right}");
        }

        assert!(TimeUnit::Second < TimeUnit::Nanosecond);
        assert!(IntervalUnit::YearMonth < IntervalUnit::MonthDayMillisUnsigned);
        assert!(
            TimeZone::Utc < TimeZone::named("Europe/Paris").expect("a named zone is canonical")
        );
    }

    #[test]
    fn conformance_reports_the_exact_nested_path() {
        let value = Value::Map(BTreeMap::from([(
            String::from("items"),
            Value::Array(alloc::vec![Value::Int(7), Value::Text(String::from("no"))]),
        )]));
        let ty = DataType::text_map(DataType::array(DataType::INT));
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
        assert_eq!(violation.expected, DataType::INT);
        assert_eq!(violation.found, ValueKind::Text);
        assert_eq!(
            violation.to_string(),
            "`payload[\"items\"][1]` is declared int32; a text cannot enter it"
        );
    }

    #[test]
    fn decimal_value_and_type_budgets_are_distinct() {
        assert!(decimal_in_domain(10_i128.pow(DECIMAL_MAX_DIGITS - 1), 0));
        assert!(!decimal_in_domain(10_i128.pow(DECIMAL_MAX_DIGITS), 0));
        assert!(decimal_in_domain(1, DECIMAL_MIN_SCALE));
        assert!(decimal_in_domain(1, DECIMAL_MAX_SCALE));
        assert!(DecimalType::new(38, -38).is_ok());
        assert!(DecimalType::new(38, 38).is_ok());
        assert!(DecimalType::new(38, 39).is_err());
    }

    #[test]
    fn floating_carrier_preserves_every_arrow_width_exactly() {
        let half = DataType::Float(FloatType::Binary16);
        assert!(half.accepts(&Value::Double(65_504.0)));
        assert!(!half.accepts(&Value::Double(65_520.0)));
        assert!(half.accepts(&Value::Double(2_f64.powi(-24))));
        assert!(!half.accepts(&Value::Double(2_f64.powi(-25))));
        assert!(half.accepts(&Value::Double(-0.0)));
        assert!(half.accepts(&Value::Double(f64::NAN)));

        let single = DataType::Float(FloatType::Binary32);
        assert!(single.accepts(&Value::Double(f64::from(0.1_f32))));
        assert!(!single.accepts(&Value::Double(0.1_f64)));
        assert!(DataType::DOUBLE.accepts(&Value::Double(0.1_f64)));
    }

    #[test]
    fn temporal_carriers_are_canonical_and_types_hold_the_parameters() {
        let date32 = DataType::Temporal(TemporalType::Date(DateUnit::Day));
        assert!(date32.accepts(&Value::Date(i64::from(i32::MAX))));
        assert!(!date32.accepts(&Value::Date(i64::from(i32::MAX) + 1)));

        let date64 = DataType::Temporal(TemporalType::Date(DateUnit::Millisecond));
        let largest_date64 = i64::MAX / MILLIS_PER_DAY;
        assert!(date64.accepts(&Value::Date(largest_date64)));
        assert!(!date64.accepts(&Value::Date(largest_date64 + 1)));

        let time_millis = DataType::Temporal(TemporalType::Time(TimeUnit::Millisecond));
        assert!(time_millis.accepts(&Value::Time(1_000_000)));
        assert!(!time_millis.accepts(&Value::Time(1)));
        assert!(!time_millis.accepts(&Value::Time(NANOS_PER_DAY)));

        let timestamp_seconds = DataType::Temporal(TemporalType::Timestamp {
            unit: TimeUnit::Second,
            zone: TimeZone::named("Europe/Paris").expect("a named timezone"),
        });
        assert!(timestamp_seconds.accepts(&Value::Timestamp(NANOS_PER_SECOND)));
        assert!(!timestamp_seconds.accepts(&Value::Timestamp(1)));

        let local_micros = DataType::Temporal(TemporalType::TimestampNtz(TimeUnit::Microsecond));
        assert!(local_micros.accepts(&Value::TimestampNtz(1_000)));
        assert!(!local_micros.accepts(&Value::TimestampNtz(1)));

        let duration_millis = DataType::Temporal(TemporalType::Duration(TimeUnit::Millisecond));
        assert!(duration_millis.accepts(&Value::Duration(-1_000_000)));
        assert!(!duration_millis.accepts(&Value::Duration(-1)));

        let year_month = DataType::Temporal(TemporalType::Interval(IntervalUnit::YearMonth));
        assert!(year_month.accepts(&Value::Interval {
            months: 15,
            days: 0,
            nanoseconds: 0,
        }));
        assert!(!year_month.accepts(&Value::Interval {
            months: 15,
            days: 1,
            nanoseconds: 0,
        }));

        let day_time = DataType::Temporal(TemporalType::Interval(IntervalUnit::DayTime));
        assert!(day_time.accepts(&Value::Interval {
            months: 0,
            days: 2,
            nanoseconds: 3_000_000,
        }));
        assert!(!day_time.accepts(&Value::Interval {
            months: 1,
            days: 2,
            nanoseconds: 3_000_000,
        }));

        let month_day_nano = DataType::Temporal(TemporalType::Interval(IntervalUnit::MonthDayNano));
        assert!(month_day_nano.accepts(&Value::Interval {
            months: -1,
            days: 2,
            nanoseconds: -3,
        }));
        assert!(!month_day_nano.accepts(&Value::Interval {
            months: i64::from(i32::MAX) + 1,
            days: 0,
            nanoseconds: 0,
        }));

        let avro_duration =
            DataType::Temporal(TemporalType::Interval(IntervalUnit::MonthDayMillisUnsigned));
        assert!(avro_duration.accepts(&Value::Interval {
            months: i64::from(u32::MAX),
            days: i64::from(u32::MAX),
            nanoseconds: i64::from(u32::MAX) * NANOS_PER_MILLISECOND,
        }));
        assert!(!avro_duration.accepts(&Value::Interval {
            months: -1,
            days: 0,
            nanoseconds: 0,
        }));
        assert!(DataType::Uuid.accepts(&Value::Uuid([0xA5; 16])));
    }

    #[test]
    fn non_text_maps_have_one_canonical_materialized_form() {
        let ty = DataType::Map(
            MapType::new(DataType::INT, DataType::Text).expect("int is a canonical map key"),
        );
        let value = Value::KeyedMap(alloc::vec![
            (Value::Int(2), Value::Text(String::from("b"))),
            (Value::Int(1), Value::Text(String::from("a"))),
        ]);
        assert!(ty.accepts(&value));

        let wrong_key = Value::KeyedMap(alloc::vec![(
            Value::Uint(1),
            Value::Text(String::from("wrong signedness")),
        )]);
        let violation = ty
            .violation("payload", &wrong_key)
            .expect("the key type is checked recursively");
        assert!(matches!(
            violation.path.as_slice(),
            [ValuePathSegment::MapKey(_)]
        ));
        assert_eq!(violation.expected, DataType::INT);

        let text_in_the_non_text_form =
            Value::KeyedMap(alloc::vec![
                (Value::Text(String::from("k")), Value::Int(1),)
            ]);
        assert!(matches!(
            text_in_the_non_text_form.validate(),
            Err(crate::value::ValueError {
                defect: ValueDefect::NonCanonicalMapKey(ValueKind::Text),
                ..
            })
        ));

        let duplicate = Value::KeyedMap(alloc::vec![
            (Value::Int(1), Value::Bool(true)),
            (Value::Int(1), Value::Bool(false)),
        ]);
        assert!(matches!(
            duplicate.validate(),
            Err(crate::value::ValueError {
                defect: ValueDefect::DuplicateMapKey(_),
                ..
            })
        ));
    }

    #[test]
    fn strict_json_avro_and_arrow_profiles_have_materialized_representatives() {
        let fixed_four = DataType::Bytes(BinaryType::Fixed(
            ByteLength::new(4).expect("a valid fixed binary length"),
        ));
        let fixed_pair = DataType::Array(
            ArrayType::fixed(DataType::Text, 2).expect("a valid fixed array length"),
        );
        let status = DataType::Enum(
            EnumType::new(alloc::vec![String::from("done"), String::from("pending")])
                .expect("a closed enum"),
        );
        let record = DataType::Struct(
            StructType::new(alloc::vec![
                TypeField::new("id", DataType::Uuid),
                TypeField::new("status", status.clone()),
            ])
            .expect("a closed record"),
        );
        let tagged = DataType::TaggedUnion(
            TaggedUnionType::new(alloc::vec![
                TaggedCase::new(
                    "failed",
                    StructType::new(alloc::vec![TypeField::new("code", DataType::INT)])
                        .expect("a closed payload"),
                )
                .expect("a named case"),
                TaggedCase::new(
                    "ok",
                    StructType::new(alloc::vec![]).expect("a unit payload"),
                )
                .expect("a named case"),
            ])
            .expect("stable union cases"),
        );

        let representatives = alloc::vec![
            (
                "json exact number",
                DataType::Number,
                Value::Decimal {
                    unscaled: 125,
                    scale: 2
                }
            ),
            (
                "json text map",
                DataType::text_map(DataType::Bool),
                Value::Map(BTreeMap::from([(String::from("ready"), Value::Bool(true))])),
            ),
            (
                "avro float",
                DataType::FLOAT,
                Value::Double(f64::from(1.25_f32))
            ),
            (
                "avro fixed",
                fixed_four,
                Value::Bytes(alloc::vec![0, 1, 2, 3])
            ),
            (
                "avro enum",
                status.clone(),
                Value::Enum(String::from("done"))
            ),
            (
                "avro duration",
                DataType::Temporal(TemporalType::Interval(IntervalUnit::MonthDayMillisUnsigned,)),
                Value::Interval {
                    months: 3,
                    days: 2,
                    nanoseconds: 1_000_000,
                },
            ),
            (
                "avro record",
                record,
                Value::Struct(alloc::vec![
                    (String::from("status"), Value::Enum(String::from("pending"))),
                    (String::from("id"), Value::Uuid([7; 16])),
                ]),
            ),
            (
                "arrow half",
                DataType::Float(FloatType::Binary16),
                Value::Double(1.5),
            ),
            (
                "arrow fixed-size list",
                fixed_pair,
                Value::Array(alloc::vec![
                    Value::Text(String::from("a")),
                    Value::Text(String::from("b")),
                ]),
            ),
            (
                "arrow non-text map",
                DataType::Map(
                    MapType::new(DataType::Uuid, DataType::LONG).expect("uuid is a canonical key"),
                ),
                Value::KeyedMap(alloc::vec![(Value::Uuid([9; 16]), Value::Int(42))]),
            ),
            (
                "avro/arrow tagged union",
                tagged,
                Value::TaggedUnion(Box::new(crate::value::TaggedUnionValue {
                    case: String::from("failed"),
                    payload: alloc::vec![(String::from("code"), Value::Int(5))],
                })),
            ),
        ];

        for (profile_row, ty, value) in representatives {
            assert!(
                ty.accepts(&value),
                "{profile_row} must be represented exactly: {:?}",
                ty.violation("payload", &value)
            );
        }
    }

    #[test]
    fn composites_are_canonical_at_construction() {
        let structure = StructType::new(alloc::vec![
            TypeField::new("z", DataType::Text),
            TypeField::new("a", DataType::INT),
        ])
        .expect("unique fields");
        assert_eq!(structure.fields()[0].name(), "a");

        let enumeration = EnumType::new(alloc::vec![String::from("z"), String::from("a")])
            .expect("unique symbols");
        assert_eq!(
            enumeration.symbols(),
            &[String::from("a"), String::from("z")]
        );

        let first = TaggedCase::new(
            "z",
            StructType::new(alloc::vec![]).expect("the unit payload is a closed struct"),
        )
        .expect("a named case");
        let second = TaggedCase::new("a", structure).expect("a named case");
        let tagged = TaggedUnionType::new(alloc::vec![first, second]).expect("unique cases");
        assert_eq!(tagged.cases()[0].name(), "a");
        assert_eq!(tagged.cases()[0].payload().fields()[0].name(), "a");
    }

    #[test]
    fn parameterized_types_reject_invalid_states_at_construction() {
        assert!(matches!(
            DecimalType::new(0, 0),
            Err(TypeError::DecimalPrecision { precision: 0 })
        ));
        assert!(matches!(
            DecimalType::new(38, 39),
            Err(TypeError::DecimalScale { scale: 39 })
        ));
        assert!(matches!(
            ByteLength::new(0),
            Err(TypeError::ByteLength { length: 0 })
        ));
        assert!(matches!(
            MapType::new(DataType::FLOAT, DataType::Text),
            Err(TypeError::InvalidMapKey(_))
        ));
        assert!(matches!(
            StructType::new(alloc::vec![
                TypeField::new("same", DataType::Text),
                TypeField::new("same", DataType::INT),
            ]),
            Err(TypeError::DuplicateStructField(name)) if name == "same"
        ));
        assert!(matches!(
            EnumType::new(alloc::vec![]),
            Err(TypeError::EmptyEnum)
        ));
        assert!(matches!(
            EnumType::new(alloc::vec![String::new()]),
            Err(TypeError::EmptyEnumSymbol)
        ));
        assert_eq!(TimeZone::named("UTC").expect("a UTC alias"), TimeZone::Utc);
        assert_eq!(
            TimeZone::named("Etc/UTC").expect("a UTC alias"),
            TimeZone::Utc
        );
        assert_eq!(
            TimeZone::named("+00:00").expect("a UTC alias"),
            TimeZone::Utc
        );
        assert!(matches!(TimeZone::named(""), Err(TypeError::EmptyTimeZone)));

        let unit = StructType::new(alloc::vec![]).expect("a unit payload");
        assert!(matches!(
            TaggedUnionType::new(alloc::vec![]),
            Err(TypeError::EmptyTaggedUnion)
        ));
        assert!(matches!(
            TaggedUnionType::new(alloc::vec![
                TaggedCase::new("same", unit.clone()).expect("a named case"),
                TaggedCase::new("same", unit).expect("a named case"),
            ]),
            Err(TypeError::DuplicateTaggedCase(name)) if name == "same"
        ));
    }

    #[test]
    fn structural_unions_refuse_ambiguous_branches() {
        let overlap = DataType::structural_union(alloc::vec![DataType::INT, DataType::LONG]);
        assert!(matches!(
            overlap,
            Err(TypeError::OverlappingUnionBranches { .. })
        ));

        let float_overlap = DataType::structural_union(alloc::vec![
            DataType::Float(FloatType::Binary16),
            DataType::Float(FloatType::Binary32),
        ]);
        assert!(matches!(
            float_overlap,
            Err(TypeError::OverlappingUnionBranches { .. })
        ));

        let temporal_overlap = DataType::structural_union(alloc::vec![
            DataType::Temporal(TemporalType::Timestamp {
                unit: TimeUnit::Second,
                zone: TimeZone::Utc,
            }),
            DataType::Temporal(TemporalType::Timestamp {
                unit: TimeUnit::Nanosecond,
                zone: TimeZone::Utc,
            }),
        ]);
        assert!(matches!(
            temporal_overlap,
            Err(TypeError::OverlappingUnionBranches { .. })
        ));

        let optional = DataType::Text
            .optional()
            .expect("text and null are disjoint");
        let DataType::StructuralUnion(union) = optional else {
            panic!("optional is a structural union");
        };
        assert_eq!(union.branches(), &[DataType::Null, DataType::Text]);
    }

    #[test]
    fn narrowing_distinguishes_no_match_from_ambiguous_match() {
        let union = DataType::structural_union(alloc::vec![DataType::Text, DataType::INT])
            .expect("the branches are disjoint");
        assert!(matches!(
            union.narrow(Value::Bool(true), None),
            Err(NarrowingFailure::NoMatch { .. })
        ));

        // A public constructor refuses this overlapping union. Keeping the
        // runtime judge total over a hostile in-memory target proves that it
        // still never falls back to first-match-wins.
        let hostile = DataType::StructuralUnion(StructuralUnionType(alloc::vec![
            DataType::INT,
            DataType::LONG,
        ]));
        assert!(matches!(
            hostile.narrow(Value::Int(7), None),
            Err(NarrowingFailure::AmbiguousUnion { branches })
                if branches == alloc::vec![0, 1]
        ));
    }

    #[test]
    fn decimal_narrowing_rescales_exact_values_to_the_declared_type() {
        let integer_scale =
            DataType::Decimal(DecimalType::new(10, 0).expect("a fixed decimal type"));
        assert_eq!(
            integer_scale.narrow(Value::Int(5), None),
            Ok(Value::Decimal {
                unscaled: 5,
                scale: 0,
            })
        );

        let decimal = DataType::Decimal(DecimalType::new(10, 2).expect("a fixed decimal type"));

        assert_eq!(
            decimal.narrow(Value::Int(5), None),
            Ok(Value::Decimal {
                unscaled: 500,
                scale: 2,
            })
        );
        assert_eq!(
            decimal.narrow(
                Value::Decimal {
                    unscaled: 55,
                    scale: 1,
                },
                None,
            ),
            Ok(Value::Decimal {
                unscaled: 550,
                scale: 2,
            })
        );
        assert_eq!(
            decimal.narrow(
                Value::Decimal {
                    unscaled: 550,
                    scale: 2,
                },
                None,
            ),
            Ok(Value::Decimal {
                unscaled: 550,
                scale: 2,
            })
        );

        let one_decimal_place =
            DataType::Decimal(DecimalType::new(10, 1).expect("a fixed decimal type"));
        assert_eq!(
            one_decimal_place.narrow(
                Value::Decimal {
                    unscaled: 550,
                    scale: 2,
                },
                None,
            ),
            Ok(Value::Decimal {
                unscaled: 55,
                scale: 1,
            })
        );
        assert!(matches!(
            one_decimal_place.narrow(
                Value::Decimal {
                    unscaled: 551,
                    scale: 2,
                },
                None,
            ),
            Err(NarrowingFailure::NoMatch { .. })
        ));

        let hundreds = DataType::Decimal(DecimalType::new(10, -2).expect("a fixed decimal type"));
        assert_eq!(
            hundreds.narrow(Value::Int(500), None),
            Ok(Value::Decimal {
                unscaled: 5,
                scale: -2,
            })
        );
        assert!(matches!(
            hundreds.narrow(Value::Int(5), None),
            Err(NarrowingFailure::NoMatch { .. })
        ));

        let narrow_precision =
            DataType::Decimal(DecimalType::new(3, 2).expect("a fixed decimal type"));
        assert!(matches!(
            narrow_precision.narrow(Value::Int(10), None),
            Err(NarrowingFailure::NoMatch { .. })
        ));
        assert_eq!(
            narrow_precision.narrow(
                Value::Decimal {
                    unscaled: 0,
                    scale: DECIMAL_MAX_SCALE,
                },
                None,
            ),
            Ok(Value::Decimal {
                unscaled: 0,
                scale: 2,
            })
        );
    }

    #[test]
    fn structural_union_widening_requires_explicit_reselection() {
        let source = DataType::structural_union(alloc::vec![DataType::INT, DataType::Text])
            .expect("source branches are disjoint");
        let target =
            DataType::structural_union(alloc::vec![DataType::Bool, DataType::INT, DataType::Text,])
                .expect("target branches are disjoint");

        assert!(!source.is_assignable_to(&target));
        let source_value = source
            .narrow(Value::Int(7), None)
            .expect("int selects the first source branch");
        assert_eq!(
            source_value,
            Value::StructuralUnion {
                branch: 0,
                value: Box::new(Value::Int(7)),
            }
        );
        assert_eq!(
            target.narrow(source_value, None),
            Ok(Value::StructuralUnion {
                branch: 1,
                value: Box::new(Value::Int(7)),
            })
        );
    }

    #[test]
    fn assignability_is_directional_and_structural() {
        assert!(DataType::INT.is_assignable_to(&DataType::LONG));
        assert!(!DataType::LONG.is_assignable_to(&DataType::INT));
        assert!(DataType::INT.is_assignable_to(&DataType::Number));
        assert!(DataType::INT.is_assignable_to(&DataType::Variant));
        assert!(!DataType::Variant.is_assignable_to(&DataType::INT));

        let optional_text = DataType::Text
            .optional()
            .expect("text and null are disjoint");
        assert!(DataType::Text.is_assignable_to(&optional_text));
        assert!(DataType::Null.is_assignable_to(&optional_text));
        assert!(!optional_text.is_assignable_to(&DataType::Text));

        let narrow = DataType::Struct(
            StructType::new(alloc::vec![TypeField::new("n", DataType::INT)]).expect("one field"),
        );
        let wide = DataType::Struct(
            StructType::new(alloc::vec![TypeField::new("n", DataType::LONG)]).expect("one field"),
        );
        assert!(narrow.is_assignable_to(&wide));
        assert!(!wide.is_assignable_to(&narrow));
    }

    #[test]
    fn global_type_budgets_are_checked_iteratively() {
        let mut at_limit = DataType::Text;
        for _ in 1..MAX_TYPE_NESTING {
            at_limit = DataType::array(at_limit);
        }
        assert!(at_limit.validate().is_ok());

        let beyond = DataType::array(at_limit);
        assert!(matches!(
            beyond.validate(),
            Err(TypeError::NestingBudget {
                depth,
                budget: MAX_TYPE_NESTING,
            }) if depth == MAX_TYPE_NESTING + 1
        ));

        let payload = StructType::new(alloc::vec![]).expect("a unit payload");
        let case = TaggedCase::new("done", payload).expect("a named case");
        let mut tagged = DataType::TaggedUnion(
            TaggedUnionType::new(alloc::vec![case]).expect("one case is sufficient"),
        );
        for _ in 1..MAX_TYPE_NESTING {
            tagged = DataType::array(tagged);
        }
        assert!(matches!(
            tagged.validate(),
            Err(TypeError::NestingBudget {
                depth,
                budget: MAX_TYPE_NESTING,
            }) if depth == MAX_TYPE_NESTING + 1
        ));

        let child_fields = (0..16)
            .map(|index| TypeField::new(alloc::format!("f{index}"), DataType::Text))
            .collect();
        let child =
            DataType::Struct(StructType::new(child_fields).expect("sixteen unique child fields"));
        let root_fields = (0..MAX_TYPE_MEMBERS)
            .map(|index| TypeField::new(alloc::format!("f{index}"), child.clone()))
            .collect();
        let too_wide = DataType::Struct(
            StructType::new(root_fields).expect("the root stays within its member budget"),
        );
        assert!(matches!(
            too_wide.validate(),
            Err(TypeError::NodeBudget {
                nodes,
                budget: MAX_TYPE_NODES,
            }) if nodes == MAX_TYPE_NODES + 1
        ));
    }

    #[test]
    fn conformance_never_admits_a_malformed_value_even_through_variant() {
        let malformed = Value::Struct(alloc::vec![
            (String::from("same"), Value::Int(1)),
            (String::from("same"), Value::Int(2)),
        ]);
        let violation = DataType::Variant
            .violation("payload", &malformed)
            .expect("value validity precedes dynamic type admission");
        assert!(matches!(
            violation.reason,
            ConformanceReason::MalformedValue(ValueDefect::DuplicateStructField(name))
                if name == "same"
        ));
    }
}
