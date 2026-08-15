//! Strict Apache Arrow logical-type projection.

use std::collections::BTreeSet;

use crate::datatype::{
    ArrayType, BinaryType, ByteLength, DECIMAL_MAX_DIGITS, DataType, DateUnit, DecimalType,
    FloatType, IntegerType, IntervalUnit, MapType, StructType, TaggedCase, TaggedUnionType,
    TemporalType, TimeUnit, TimeZone, TypeField,
};

use super::{LossReason, ProjectionError, field_path, index_path, type_error};

/// Arrow physical spelling for a variable binary value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrowBinaryKind {
    /// Standard-offset binary.
    Binary,
    /// Large-offset binary.
    LargeBinary,
    /// Binary view layout.
    BinaryView,
}

/// Arrow physical spelling for UTF-8 text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrowTextKind {
    /// Standard-offset UTF-8.
    Utf8,
    /// Large-offset UTF-8.
    LargeUtf8,
    /// UTF-8 view layout.
    Utf8View,
}

/// Arrow decimal storage width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrowDecimalWidth {
    /// 32-bit decimal storage, outside the V1 reverse profile.
    Bits32,
    /// 64-bit decimal storage, outside the V1 reverse profile.
    Bits64,
    /// 128-bit decimal storage admitted by V1.
    Bits128,
    /// 256-bit decimal storage, outside the V1 profile.
    Bits256,
}

/// Dense or sparse Arrow union layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrowUnionMode {
    /// Dense child offsets.
    Dense,
    /// Sparse children.
    Sparse,
}

/// One Arrow field, including physical nullability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrowField {
    name: String,
    nullable: bool,
    ty: ArrowType,
}

impl ArrowField {
    /// Builds a non-null Arrow field.
    #[must_use]
    pub fn required(name: impl Into<String>, ty: ArrowType) -> Self {
        Self {
            name: name.into(),
            nullable: false,
            ty,
        }
    }

    /// Builds a nullable Arrow field.
    #[must_use]
    pub fn nullable(name: impl Into<String>, ty: ArrowType) -> Self {
        Self {
            name: name.into(),
            nullable: true,
            ty,
        }
    }

    /// Field name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the Arrow validity bitmap admits null.
    #[must_use]
    pub const fn is_nullable(&self) -> bool {
        self.nullable
    }

    /// Arrow child type.
    #[must_use]
    pub const fn ty(&self) -> &ArrowType {
        &self.ty
    }
}

/// One named Arrow union child and its physical type code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrowUnionChild {
    name: String,
    type_code: i8,
    nullable: bool,
    ty: ArrowType,
}

impl ArrowUnionChild {
    /// Builds a union child.
    #[must_use]
    pub fn new(name: impl Into<String>, type_code: i8, ty: ArrowType) -> Self {
        Self {
            name: name.into(),
            type_code,
            nullable: false,
            ty,
        }
    }

    /// Builds a nullable union child.
    #[must_use]
    pub fn nullable(name: impl Into<String>, type_code: i8, ty: ArrowType) -> Self {
        Self {
            name: name.into(),
            type_code,
            nullable: true,
            ty,
        }
    }

    /// Stable logical case name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Physical Arrow type code.
    #[must_use]
    pub const fn type_code(&self) -> i8 {
        self.type_code
    }

    /// Whether the selected child may itself hold null.
    #[must_use]
    pub const fn is_nullable(&self) -> bool {
        self.nullable
    }

    /// Child type.
    #[must_use]
    pub const fn ty(&self) -> &ArrowType {
        &self.ty
    }
}

/// Resolved Arrow type descriptor used by the boundary adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArrowType {
    /// Null.
    Null,
    /// Boolean.
    Bool,
    /// Signed or unsigned fixed-width integer.
    Integer(IntegerType),
    /// IEEE-754 binary float.
    Float(FloatType),
    /// Decimal with an Arrow physical storage width.
    Decimal {
        /// Storage width.
        width: ArrowDecimalWidth,
        /// Declared precision.
        precision: u32,
        /// Declared signed scale.
        scale: i32,
    },
    /// Variable binary with one physical layout.
    Binary(ArrowBinaryKind),
    /// Exact-size binary.
    FixedBinary(u32),
    /// UTF-8 text with one physical layout.
    Text(ArrowTextKind),
    /// Calendar date.
    Date(DateUnit),
    /// Time of day.
    Time(TimeUnit),
    /// Timestamp; `None` is timezone-naive.
    Timestamp {
        /// Resolution.
        unit: TimeUnit,
        /// UTC or named timezone, absent for timestamp-ntz.
        timezone: Option<String>,
    },
    /// Elapsed duration.
    Duration(TimeUnit),
    /// Calendar interval.
    Interval(IntervalUnit),
    /// Closed struct.
    Struct(Vec<ArrowField>),
    /// Variable list. The child field carries element nullability.
    List(Box<ArrowField>),
    /// Exact-size list.
    FixedSizeList {
        /// Element field.
        field: Box<ArrowField>,
        /// Exact number of elements.
        length: u32,
    },
    /// Arrow map key and value fields.
    Map {
        /// Non-null key field.
        key: Box<ArrowField>,
        /// Value field.
        value: Box<ArrowField>,
    },
    /// Dictionary encoding, projected as its decoded value type.
    Dictionary(Box<ArrowType>),
    /// Run-end encoding, projected as its decoded value type.
    RunEndEncoded(Box<ArrowType>),
    /// Dense or sparse union.
    Union {
        /// Physical layout, outside logical identity.
        mode: ArrowUnionMode,
        /// Named children and physical type codes.
        children: Vec<ArrowUnionChild>,
    },
    /// Extension type requiring custom interpretation.
    Extension(String),
    /// Tensor or fixed-shape tensor metadata.
    Tensor,
}

/// One Arrow type-code mapping, outside logical type identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrowTypeCodeMapping {
    type_code: i8,
    case: String,
}

impl ArrowTypeCodeMapping {
    /// Physical type code.
    #[must_use]
    pub const fn type_code(&self) -> i8 {
        self.type_code
    }

    /// Canonical Rastok tagged case.
    #[must_use]
    pub fn case(&self) -> &str {
        &self.case
    }
}

/// One Arrow union's physical mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrowUnionMapping {
    path: String,
    mode: ArrowUnionMode,
    codes: Vec<ArrowTypeCodeMapping>,
}

impl ArrowUnionMapping {
    /// Logical location of the union.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Physical dense/sparse layout.
    #[must_use]
    pub const fn mode(&self) -> ArrowUnionMode {
        self.mode
    }

    /// Physical type-code-to-case mapping in declaration order.
    #[must_use]
    pub fn codes(&self) -> &[ArrowTypeCodeMapping] {
        &self.codes
    }
}

/// Result of projecting one Arrow type descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrowProjection {
    ty: DataType,
    unions: Vec<ArrowUnionMapping>,
}

impl ArrowProjection {
    /// Canonical logical type.
    #[must_use]
    pub const fn ty(&self) -> &DataType {
        &self.ty
    }

    /// Physical union mappings found in the descriptor.
    #[must_use]
    pub fn unions(&self) -> &[ArrowUnionMapping] {
        &self.unions
    }
}

/// Projects one Arrow descriptor into Rastok's canonical logical algebra.
pub fn project(source: &ArrowType) -> Result<ArrowProjection, ProjectionError> {
    let mut projector = Projector::default();
    let ty = projector.ty(source, "$")?;
    ty.validate().map_err(|error| type_error("$", error))?;
    Ok(ArrowProjection {
        ty,
        unions: projector.unions,
    })
}

/// Reverses one canonical logical type into a lossless Arrow descriptor.
pub fn reverse(ty: &DataType) -> Result<ArrowType, ProjectionError> {
    let descriptor = reverse_type(ty, "$", false)?;
    let round_trip = project(&descriptor).map_err(|error| {
        ProjectionError::new(
            "$",
            LossReason::UnsupportedTarget,
            format!("generated Arrow descriptor is not in the exact profile: {error}"),
        )
    })?;
    if round_trip.ty() != ty {
        return Err(ProjectionError::new(
            "$",
            LossReason::LossyProjection,
            "generated Arrow descriptor changes the logical type",
        ));
    }
    Ok(descriptor)
}

#[derive(Default)]
struct Projector {
    unions: Vec<ArrowUnionMapping>,
}

impl Projector {
    fn ty(&mut self, source: &ArrowType, path: &str) -> Result<DataType, ProjectionError> {
        let ty = match source {
            ArrowType::Null => DataType::Null,
            ArrowType::Bool => DataType::Bool,
            ArrowType::Integer(integer) => DataType::Integer(*integer),
            ArrowType::Float(float) => DataType::Float(*float),
            ArrowType::Decimal {
                width: ArrowDecimalWidth::Bits128,
                precision,
                scale,
            } => {
                if !(1..=DECIMAL_MAX_DIGITS).contains(precision) {
                    return Err(ProjectionError::new(
                        path,
                        LossReason::InvalidSchema,
                        "Arrow decimal128 precision must be in 1..=38",
                    ));
                }
                DecimalType::new(*precision, *scale)
                    .map(DataType::Decimal)
                    .map_err(|error| type_error(path, error))?
            }
            ArrowType::Decimal { width, .. } => {
                return Err(ProjectionError::new(
                    path,
                    LossReason::UnsupportedSource,
                    format!("Arrow {width:?} decimal storage is outside the V1 profile"),
                ));
            }
            ArrowType::Binary(_) => DataType::BYTES,
            ArrowType::FixedBinary(length) => ByteLength::new(*length)
                .map(|length| DataType::Bytes(BinaryType::Fixed(length)))
                .map_err(|error| type_error(path, error))?,
            ArrowType::Text(_) => DataType::Text,
            ArrowType::Date(unit) => DataType::Temporal(TemporalType::Date(*unit)),
            ArrowType::Time(unit) => DataType::Temporal(TemporalType::Time(*unit)),
            ArrowType::Timestamp {
                unit,
                timezone: Some(timezone),
            } => DataType::Temporal(TemporalType::Timestamp {
                unit: *unit,
                zone: TimeZone::named(timezone.clone()).map_err(|error| type_error(path, error))?,
            }),
            ArrowType::Timestamp {
                unit,
                timezone: None,
            } => DataType::Temporal(TemporalType::TimestampNtz(*unit)),
            ArrowType::Duration(unit) => DataType::Temporal(TemporalType::Duration(*unit)),
            ArrowType::Interval(
                unit @ (IntervalUnit::YearMonth
                | IntervalUnit::DayTime
                | IntervalUnit::MonthDayNano),
            ) => DataType::Temporal(TemporalType::Interval(*unit)),
            ArrowType::Interval(IntervalUnit::MonthDayMillisUnsigned) => {
                return Err(ProjectionError::new(
                    path,
                    LossReason::UnsupportedSource,
                    "Arrow has no unsigned Avro-duration interval",
                ));
            }
            ArrowType::Struct(fields) => {
                let fields = fields
                    .iter()
                    .map(|field| {
                        let field_path = field_path(path, field.name());
                        let ty = self.field(field, &field_path)?;
                        Ok(TypeField::new(field.name(), ty))
                    })
                    .collect::<Result<Vec<_>, ProjectionError>>()?;
                StructType::new(fields)
                    .map(DataType::Struct)
                    .map_err(|error| type_error(path, error))?
            }
            ArrowType::List(field) => {
                DataType::array(self.field(field, &field_path(path, "item"))?)
            }
            ArrowType::FixedSizeList { field, length } => {
                ArrayType::fixed(self.field(field, &field_path(path, "item"))?, *length)
                    .map(DataType::Array)
                    .map_err(|error| type_error(path, error))?
            }
            ArrowType::Map { key, value } => {
                if key.is_nullable() {
                    return Err(ProjectionError::new(
                        field_path(path, "key"),
                        LossReason::InvalidMapKey,
                        "an Arrow map key cannot be nullable",
                    ));
                }
                MapType::new(
                    self.ty(key.ty(), &field_path(path, "key"))?,
                    self.field(value, &field_path(path, "value"))?,
                )
                .map(DataType::Map)
                .map_err(|error| type_error(path, error))?
            }
            ArrowType::Dictionary(value) | ArrowType::RunEndEncoded(value) => {
                self.ty(value, path)?
            }
            ArrowType::Union { mode, children } => self.union(*mode, children, path)?,
            ArrowType::Extension(name) => {
                return Err(ProjectionError::new(
                    path,
                    LossReason::UnsupportedSource,
                    format!("Arrow extension `{name}` needs a registered projection"),
                ));
            }
            ArrowType::Tensor => {
                return Err(ProjectionError::new(
                    path,
                    LossReason::UnsupportedSource,
                    "Arrow tensor metadata needs custom interpretation",
                ));
            }
        };
        Ok(ty)
    }

    fn field(&mut self, field: &ArrowField, path: &str) -> Result<DataType, ProjectionError> {
        let ty = self.ty(field.ty(), path)?;
        if field.is_nullable() && ty != DataType::Null {
            ty.optional().map_err(|error| type_error(path, error))
        } else {
            Ok(ty)
        }
    }

    fn union(
        &mut self,
        mode: ArrowUnionMode,
        children: &[ArrowUnionChild],
        path: &str,
    ) -> Result<DataType, ProjectionError> {
        if children.is_empty() {
            return Err(ProjectionError::new(
                path,
                LossReason::InvalidSchema,
                "an Arrow union needs at least one child",
            ));
        }
        let mut names = BTreeSet::new();
        let mut codes = BTreeSet::new();
        let mut cases = Vec::with_capacity(children.len());
        let mut mapping = Vec::with_capacity(children.len());
        for (index, child) in children.iter().enumerate() {
            let child_path = index_path(path, index);
            if child.name().is_empty() || !names.insert(child.name()) {
                return Err(ProjectionError::new(
                    &child_path,
                    LossReason::InvalidSchema,
                    "Arrow union child names must be non-empty and unique",
                ));
            }
            if !codes.insert(child.type_code()) {
                return Err(ProjectionError::new(
                    &child_path,
                    LossReason::InvalidSchema,
                    "Arrow union type codes must be unique",
                ));
            }
            let projected = self.ty(child.ty(), &child_path)?;
            let projected = if child.is_nullable() && projected != DataType::Null {
                projected
                    .optional()
                    .map_err(|error| type_error(&child_path, error))?
            } else {
                projected
            };
            let payload = match projected {
                DataType::Struct(payload) => payload,
                other => StructType::new(vec![TypeField::new("value", other)])
                    .map_err(|error| type_error(&child_path, error))?,
            };
            cases.push(
                TaggedCase::new(child.name(), payload)
                    .map_err(|error| type_error(&child_path, error))?,
            );
            mapping.push(ArrowTypeCodeMapping {
                type_code: child.type_code(),
                case: child.name().to_string(),
            });
        }
        let union = TaggedUnionType::new(cases).map_err(|error| type_error(path, error))?;
        self.unions.push(ArrowUnionMapping {
            path: path.to_string(),
            mode,
            codes: mapping,
        });
        Ok(DataType::TaggedUnion(union))
    }
}

fn reverse_type(
    ty: &DataType,
    path: &str,
    field_context: bool,
) -> Result<ArrowType, ProjectionError> {
    Ok(match ty {
        DataType::Null => ArrowType::Null,
        DataType::Bool => ArrowType::Bool,
        DataType::Integer(integer) => ArrowType::Integer(*integer),
        DataType::Float(float) => ArrowType::Float(*float),
        DataType::Decimal(decimal) => ArrowType::Decimal {
            width: ArrowDecimalWidth::Bits128,
            precision: u32::from(decimal.precision()),
            scale: i32::from(decimal.scale()),
        },
        DataType::Text => ArrowType::Text(ArrowTextKind::Utf8),
        DataType::Bytes(BinaryType::Variable) => ArrowType::Binary(ArrowBinaryKind::Binary),
        DataType::Bytes(BinaryType::Fixed(length)) => ArrowType::FixedBinary(length.get()),
        DataType::Temporal(TemporalType::Date(unit)) => ArrowType::Date(*unit),
        DataType::Temporal(TemporalType::Time(unit)) => ArrowType::Time(*unit),
        DataType::Temporal(TemporalType::Timestamp { unit, zone }) => ArrowType::Timestamp {
            unit: *unit,
            timezone: Some(match zone {
                TimeZone::Utc => String::from("UTC"),
                TimeZone::Named(name) => name.as_str().to_string(),
            }),
        },
        DataType::Temporal(TemporalType::TimestampNtz(unit)) => ArrowType::Timestamp {
            unit: *unit,
            timezone: None,
        },
        DataType::Temporal(TemporalType::Duration(unit)) => ArrowType::Duration(*unit),
        DataType::Temporal(TemporalType::Interval(
            unit @ (IntervalUnit::YearMonth | IntervalUnit::DayTime | IntervalUnit::MonthDayNano),
        )) => ArrowType::Interval(*unit),
        DataType::Temporal(TemporalType::Interval(IntervalUnit::MonthDayMillisUnsigned)) => {
            return unsupported_target(path, "Arrow cannot preserve Avro's unsigned duration");
        }
        DataType::Struct(structure) => ArrowType::Struct(
            structure
                .fields()
                .iter()
                .map(|field| {
                    reverse_field(field.name(), field.ty(), &field_path(path, field.name()))
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        DataType::Map(map) => ArrowType::Map {
            key: Box::new(ArrowField::required(
                "key",
                reverse_type(map.key(), &field_path(path, "key"), true)?,
            )),
            value: Box::new(reverse_field(
                "value",
                map.value(),
                &field_path(path, "value"),
            )?),
        },
        DataType::Array(array) => {
            let field = Box::new(reverse_field(
                "item",
                array.element(),
                &field_path(path, "item"),
            )?);
            match array.length() {
                Some(length) => ArrowType::FixedSizeList {
                    field,
                    length: length.get(),
                },
                None => ArrowType::List(field),
            }
        }
        DataType::TaggedUnion(union) => {
            let children = union
                .cases()
                .iter()
                .enumerate()
                .map(|(index, case)| {
                    let type_code = i8::try_from(index).map_err(|_| {
                        ProjectionError::new(
                            path,
                            LossReason::UnsupportedTarget,
                            "a flat Arrow union admits at most 128 children",
                        )
                    })?;
                    Ok(ArrowUnionChild::new(
                        case.name(),
                        type_code,
                        reverse_type(
                            &DataType::Struct(case.payload().clone()),
                            &index_path(path, index),
                            false,
                        )?,
                    ))
                })
                .collect::<Result<Vec<_>, ProjectionError>>()?;
            ArrowType::Union {
                mode: ArrowUnionMode::Dense,
                children,
            }
        }
        DataType::StructuralUnion(union) if field_context => {
            if let Some(inner) = optional_inner(union.branches()) {
                return reverse_type(inner, path, true);
            }
            return unsupported_target(path, "Arrow field nullability only represents T | null");
        }
        DataType::StructuralUnion(_) => {
            return unsupported_target(path, "Arrow unions need stable child names");
        }
        DataType::Enum(_) => {
            return unsupported_target(path, "Arrow dictionary metadata is not a closed enum");
        }
        DataType::Number => {
            return unsupported_target(path, "Arrow has no integer-or-decimal number type");
        }
        DataType::Uuid => return unsupported_target(path, "Arrow V1 has no UUID extension"),
        DataType::Variant => return unsupported_target(path, "Arrow needs a resolved schema"),
    })
}

fn reverse_field(name: &str, ty: &DataType, path: &str) -> Result<ArrowField, ProjectionError> {
    if let DataType::StructuralUnion(union) = ty
        && let Some(inner) = optional_inner(union.branches())
    {
        return Ok(ArrowField::nullable(name, reverse_type(inner, path, true)?));
    }
    Ok(ArrowField::required(name, reverse_type(ty, path, true)?))
}

fn optional_inner(branches: &[DataType]) -> Option<&DataType> {
    if branches.len() != 2 {
        return None;
    }
    match (&branches[0], &branches[1]) {
        (DataType::Null, inner) | (inner, DataType::Null) => Some(inner),
        _ => None,
    }
}

fn unsupported_target<T>(path: &str, detail: &str) -> Result<T, ProjectionError> {
    Err(ProjectionError::new(
        path,
        LossReason::UnsupportedTarget,
        detail,
    ))
}
