//! Strict Apache Avro 1.12 schema projection.

use std::collections::{BTreeMap, BTreeSet};

use crate::datatype::{
    BinaryType, ByteLength, DataType, DateUnit, DecimalType, EnumType, FloatType, IntegerType,
    IntegerWidth, IntervalUnit, MapType, StructType, TaggedCase, TaggedUnionType, TemporalType,
    TimeUnit, TimeZone, TypeError, TypeField,
};
use crate::value::Value;

use super::{
    LossReason, ProjectionError, UnionSelection, as_array, as_object, as_text, as_u32,
    exact_integer, field_path, index_path, object, parse_json_document, render_json_document, text,
    type_error,
};

/// One Avro union's physical branch-index mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvroUnionMapping {
    path: String,
    branches: Vec<UnionSelection>,
}

impl AvroUnionMapping {
    /// Location of the Avro union in the schema document.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Logical branch or case selected by each Avro branch index.
    #[must_use]
    pub fn branches(&self) -> &[UnionSelection] {
        &self.branches
    }
}

/// Result of projecting one Avro schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvroProjection {
    ty: DataType,
    unions: Vec<AvroUnionMapping>,
}

impl AvroProjection {
    /// Canonical logical type.
    #[must_use]
    pub const fn ty(&self) -> &DataType {
        &self.ty
    }

    /// Physical branch-index mappings, kept outside logical identity.
    #[must_use]
    pub fn unions(&self) -> &[AvroUnionMapping] {
        &self.unions
    }
}

/// Parses and projects one complete Avro schema JSON document.
pub fn project(document: &str) -> Result<AvroProjection, ProjectionError> {
    let root = parse_json_document(document)?;
    let mut projector = Projector::default();
    projector.collect_names(&root, "$", "")?;
    let ty = projector.schema(&root, "$", "")?;
    ty.validate().map_err(|error| type_error("$", error))?;
    Ok(AvroProjection {
        ty,
        unions: projector.unions,
    })
}

/// Reverses a canonical type into one canonical Avro schema document.
pub fn reverse(ty: &DataType) -> Result<String, ProjectionError> {
    let schema = reverse_type(ty, "$", "RastokRoot")?;
    let document = render_json_document(&schema)?;
    let round_trip = project(&document).map_err(|error| {
        ProjectionError::new(
            "$",
            LossReason::UnsupportedTarget,
            format!("generated Avro schema is not in the exact profile: {error}"),
        )
    })?;
    if round_trip.ty() != ty {
        return Err(ProjectionError::new(
            "$",
            LossReason::LossyProjection,
            "generated Avro schema changes the logical type",
        ));
    }
    Ok(document)
}

#[derive(Default)]
struct Projector {
    definitions: BTreeMap<String, Value>,
    active: BTreeSet<String>,
    cache: BTreeMap<String, DataType>,
    unions: Vec<AvroUnionMapping>,
}

impl Projector {
    fn collect_names(
        &mut self,
        schema: &Value,
        path: &str,
        namespace: &str,
    ) -> Result<(), ProjectionError> {
        match schema {
            Value::Text(kind) => {
                if primitive(kind).is_none()
                    && resolve_name(kind, namespace, &self.definitions).is_none()
                {
                    return Err(ProjectionError::new(
                        path,
                        LossReason::InvalidSchema,
                        format!("Avro name `{kind}` is unknown or used before its definition"),
                    ));
                }
            }
            Value::Array(branches) => {
                for (index, branch) in branches.iter().enumerate() {
                    self.collect_names(branch, &index_path(path, index), namespace)?;
                }
            }
            Value::Map(map) => {
                let Some(kind) = map.get("type") else {
                    return Err(ProjectionError::new(
                        field_path(path, "type"),
                        LossReason::InvalidSchema,
                        "an Avro schema object needs type",
                    ));
                };
                if let Value::Text(kind) = kind {
                    match kind.as_str() {
                        "record" | "enum" | "fixed" => {
                            let full = definition_name(map, path, namespace)?;
                            if self
                                .definitions
                                .insert(full.clone(), schema.clone())
                                .is_some()
                            {
                                return Err(ProjectionError::new(
                                    field_path(path, "name"),
                                    LossReason::InvalidSchema,
                                    format!("Avro name `{full}` is declared twice"),
                                ));
                            }
                            if kind == "record" {
                                let child_namespace = namespace_of(&full);
                                let fields_path = field_path(path, "fields");
                                let fields = map.get("fields").ok_or_else(|| {
                                    ProjectionError::new(
                                        &fields_path,
                                        LossReason::InvalidSchema,
                                        "a record needs fields",
                                    )
                                })?;
                                for (index, field) in
                                    as_array(fields, &fields_path)?.iter().enumerate()
                                {
                                    let field_path = index_path(&fields_path, index);
                                    let field = as_object(field, &field_path)?;
                                    let ty = field.get("type").ok_or_else(|| {
                                        ProjectionError::new(
                                            super::field_path(&field_path, "type"),
                                            LossReason::InvalidSchema,
                                            "an Avro field needs type",
                                        )
                                    })?;
                                    self.collect_names(
                                        ty,
                                        &super::field_path(&field_path, "type"),
                                        &child_namespace,
                                    )?;
                                }
                            }
                        }
                        "array" => {
                            let items_path = field_path(path, "items");
                            let items = map.get("items").ok_or_else(|| {
                                ProjectionError::new(
                                    &items_path,
                                    LossReason::InvalidSchema,
                                    "an Avro array needs items",
                                )
                            })?;
                            self.collect_names(items, &items_path, namespace)?;
                        }
                        "map" => {
                            let values_path = field_path(path, "values");
                            let values = map.get("values").ok_or_else(|| {
                                ProjectionError::new(
                                    &values_path,
                                    LossReason::InvalidSchema,
                                    "an Avro map needs values",
                                )
                            })?;
                            self.collect_names(values, &values_path, namespace)?;
                        }
                        other if primitive(other).is_some() => {}
                        other => {
                            if resolve_name(other, namespace, &self.definitions).is_none() {
                                return Err(ProjectionError::new(
                                    field_path(path, "type"),
                                    LossReason::InvalidSchema,
                                    format!(
                                        "Avro name `{other}` is unknown or used before its definition"
                                    ),
                                ));
                            }
                        }
                    }
                } else {
                    self.collect_names(kind, &field_path(path, "type"), namespace)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn schema(
        &mut self,
        schema: &Value,
        path: &str,
        namespace: &str,
    ) -> Result<DataType, ProjectionError> {
        match schema {
            Value::Text(kind) => self.named_or_primitive(kind, path, namespace),
            Value::Array(branches) => self.union(branches, path, namespace),
            Value::Map(map) => self.object(map, path, namespace),
            _ => Err(ProjectionError::new(
                path,
                LossReason::InvalidSchema,
                "an Avro schema is a name, object or union array",
            )),
        }
    }

    fn named_or_primitive(
        &mut self,
        kind: &str,
        path: &str,
        namespace: &str,
    ) -> Result<DataType, ProjectionError> {
        if let Some(primitive) = primitive(kind) {
            return Ok(primitive);
        }
        let full = resolve_name(kind, namespace, &self.definitions).ok_or_else(|| {
            ProjectionError::new(
                path,
                LossReason::InvalidSchema,
                format!("unknown Avro name `{kind}`"),
            )
        })?;
        self.named(&full, path)
    }

    fn named(&mut self, full: &str, path: &str) -> Result<DataType, ProjectionError> {
        if let Some(cached) = self.cache.get(full) {
            return Ok(cached.clone());
        }
        if !self.active.insert(full.to_string()) {
            return Err(ProjectionError::new(
                path,
                LossReason::RecursiveSchema,
                format!("Avro name `{full}` reaches itself"),
            ));
        }
        let definition = self.definitions.get(full).cloned().ok_or_else(|| {
            ProjectionError::new(
                path,
                LossReason::InvalidSchema,
                format!("unknown Avro name `{full}`"),
            )
        })?;
        let map = as_object(&definition, path)?;
        let result = self.named_definition(map, path, full);
        self.active.remove(full);
        if let Ok(ty) = &result {
            self.cache.insert(full.to_string(), ty.clone());
        }
        result
    }

    fn object(
        &mut self,
        map: &BTreeMap<String, Value>,
        path: &str,
        namespace: &str,
    ) -> Result<DataType, ProjectionError> {
        let type_path = field_path(path, "type");
        let kind = map.get("type").ok_or_else(|| {
            ProjectionError::new(&type_path, LossReason::InvalidSchema, "missing Avro type")
        })?;
        let Value::Text(kind) = kind else {
            return self.schema(kind, &type_path, namespace);
        };
        if let Some(logical) = map.get("logicalType") {
            return self.logical(
                map,
                kind,
                as_text(logical, &field_path(path, "logicalType"))?,
                path,
            );
        }
        match kind.as_str() {
            "record" | "enum" | "fixed" => {
                let full = definition_name(map, path, namespace)?;
                self.named(&full, path)
            }
            "array" => {
                let items_path = field_path(path, "items");
                let items = map.get("items").ok_or_else(|| {
                    ProjectionError::new(
                        &items_path,
                        LossReason::InvalidSchema,
                        "an Avro array needs items",
                    )
                })?;
                Ok(DataType::array(self.schema(
                    items,
                    &items_path,
                    namespace,
                )?))
            }
            "map" => {
                let values_path = field_path(path, "values");
                let values = map.get("values").ok_or_else(|| {
                    ProjectionError::new(
                        &values_path,
                        LossReason::InvalidSchema,
                        "an Avro map needs values",
                    )
                })?;
                MapType::new(
                    DataType::Text,
                    self.schema(values, &values_path, namespace)?,
                )
                .map(DataType::Map)
                .map_err(|error| type_error(path, error))
            }
            other => self.named_or_primitive(other, path, namespace),
        }
    }

    fn named_definition(
        &mut self,
        map: &BTreeMap<String, Value>,
        path: &str,
        full: &str,
    ) -> Result<DataType, ProjectionError> {
        let kind = as_text(
            map.get("type").ok_or_else(|| {
                ProjectionError::new(path, LossReason::InvalidSchema, "missing Avro type")
            })?,
            &field_path(path, "type"),
        )?;
        match kind {
            "record" => {
                let fields_path = field_path(path, "fields");
                let fields = map.get("fields").ok_or_else(|| {
                    ProjectionError::new(
                        &fields_path,
                        LossReason::InvalidSchema,
                        "a record needs fields",
                    )
                })?;
                let namespace = namespace_of(full);
                let fields = as_array(fields, &fields_path)?
                    .iter()
                    .enumerate()
                    .map(|(index, field)| {
                        let item_path = index_path(&fields_path, index);
                        let field = as_object(field, &item_path)?;
                        let name_path = field_path(&item_path, "name");
                        let name = as_text(
                            field.get("name").ok_or_else(|| {
                                ProjectionError::new(
                                    &name_path,
                                    LossReason::InvalidSchema,
                                    "an Avro field needs a name",
                                )
                            })?,
                            &name_path,
                        )?;
                        valid_simple_name(name, &name_path)?;
                        let ty_path = field_path(&item_path, "type");
                        let ty = self.schema(
                            field.get("type").ok_or_else(|| {
                                ProjectionError::new(
                                    &ty_path,
                                    LossReason::InvalidSchema,
                                    "an Avro field needs type",
                                )
                            })?,
                            &ty_path,
                            &namespace,
                        )?;
                        Ok(TypeField::new(name, ty))
                    })
                    .collect::<Result<Vec<_>, ProjectionError>>()?;
                StructType::new(fields)
                    .map(DataType::Struct)
                    .map_err(|error| type_error(path, error))
            }
            "enum" => {
                let symbols_path = field_path(path, "symbols");
                let symbols = map.get("symbols").ok_or_else(|| {
                    ProjectionError::new(
                        &symbols_path,
                        LossReason::InvalidSchema,
                        "an Avro enum needs symbols",
                    )
                })?;
                let symbols = as_array(symbols, &symbols_path)?
                    .iter()
                    .enumerate()
                    .map(|(index, symbol)| {
                        let item_path = index_path(&symbols_path, index);
                        let symbol = as_text(symbol, &item_path)?;
                        valid_simple_name(symbol, &item_path)?;
                        Ok(String::from(symbol))
                    })
                    .collect::<Result<Vec<_>, ProjectionError>>()?;
                EnumType::new(symbols)
                    .map(DataType::Enum)
                    .map_err(|error| type_error(path, error))
            }
            "fixed" => {
                if let Some(logical) = map.get("logicalType") {
                    return self.logical(
                        map,
                        "fixed",
                        as_text(logical, &field_path(path, "logicalType"))?,
                        path,
                    );
                }
                let size = as_u32(
                    map.get("size").ok_or_else(|| {
                        ProjectionError::new(
                            field_path(path, "size"),
                            LossReason::InvalidSchema,
                            "fixed needs size",
                        )
                    })?,
                    &field_path(path, "size"),
                )?;
                ByteLength::new(size)
                    .map(|length| DataType::Bytes(BinaryType::Fixed(length)))
                    .map_err(|error| type_error(path, error))
            }
            _ => Err(ProjectionError::new(
                path,
                LossReason::InvalidSchema,
                "named definition has an invalid kind",
            )),
        }
    }

    fn logical(
        &self,
        map: &BTreeMap<String, Value>,
        storage: &str,
        logical: &str,
        path: &str,
    ) -> Result<DataType, ProjectionError> {
        let storage_is = |expected: &[&str]| {
            expected.contains(&storage).then_some(()).ok_or_else(|| {
                ProjectionError::new(
                    field_path(path, "type"),
                    LossReason::InvalidSchema,
                    format!("logical type `{logical}` cannot use `{storage}` storage"),
                )
            })
        };
        match logical {
            "decimal" => {
                storage_is(&["bytes", "fixed"])?;
                let precision = as_u32(
                    map.get("precision").ok_or_else(|| {
                        ProjectionError::new(
                            field_path(path, "precision"),
                            LossReason::InvalidSchema,
                            "Avro decimal needs precision",
                        )
                    })?,
                    &field_path(path, "precision"),
                )?;
                let scale = map
                    .get("scale")
                    .map(|value| exact_integer(value, &field_path(path, "scale")))
                    .transpose()?
                    .unwrap_or(0);
                let scale = i32::try_from(scale).map_err(|_| {
                    ProjectionError::new(
                        field_path(path, "scale"),
                        LossReason::InvalidSchema,
                        "Avro decimal scale is outside i32",
                    )
                })?;
                if scale < 0
                    || u32::try_from(scale)
                        .ok()
                        .is_none_or(|scale| scale > precision)
                {
                    return Err(ProjectionError::new(
                        field_path(path, "scale"),
                        LossReason::InvalidSchema,
                        "Avro decimal requires 0 <= scale <= precision",
                    ));
                }
                if storage == "fixed" {
                    let size_path = field_path(path, "size");
                    let size = as_u32(
                        map.get("size").ok_or_else(|| {
                            ProjectionError::new(
                                &size_path,
                                LossReason::InvalidSchema,
                                "a fixed Avro decimal needs size",
                            )
                        })?,
                        &size_path,
                    )?;
                    ByteLength::new(size).map_err(|error| type_error(&size_path, error))?;
                    if !fixed_decimal_holds(size, precision) {
                        return Err(ProjectionError::new(
                            field_path(path, "precision"),
                            LossReason::InvalidSchema,
                            format!(
                                "precision {precision} does not fit a fixed decimal of {size} bytes"
                            ),
                        ));
                    }
                }
                DecimalType::new(precision, scale)
                    .map(DataType::Decimal)
                    .map_err(|error| type_error(path, error))
            }
            "date" => {
                storage_is(&["int"])?;
                Ok(DataType::Temporal(TemporalType::Date(DateUnit::Day)))
            }
            "time-millis" => {
                storage_is(&["int"])?;
                Ok(DataType::Temporal(TemporalType::Time(
                    TimeUnit::Millisecond,
                )))
            }
            "time-micros" => {
                storage_is(&["long"])?;
                Ok(DataType::Temporal(TemporalType::Time(
                    TimeUnit::Microsecond,
                )))
            }
            "time-nanos" => {
                storage_is(&["long"])?;
                Ok(DataType::Temporal(TemporalType::Time(TimeUnit::Nanosecond)))
            }
            "timestamp-millis" | "timestamp-micros" | "timestamp-nanos" => {
                storage_is(&["long"])?;
                Ok(DataType::Temporal(TemporalType::Timestamp {
                    unit: logical_time_unit(logical),
                    zone: TimeZone::Utc,
                }))
            }
            "local-timestamp-millis" | "local-timestamp-micros" | "local-timestamp-nanos" => {
                storage_is(&["long"])?;
                Ok(DataType::Temporal(TemporalType::TimestampNtz(
                    logical_time_unit(logical),
                )))
            }
            "duration" => {
                storage_is(&["fixed"])?;
                let size = as_u32(
                    map.get("size").ok_or_else(|| {
                        ProjectionError::new(
                            field_path(path, "size"),
                            LossReason::InvalidSchema,
                            "Avro duration needs fixed size 12",
                        )
                    })?,
                    &field_path(path, "size"),
                )?;
                if size != 12 {
                    return Err(ProjectionError::new(
                        field_path(path, "size"),
                        LossReason::InvalidSchema,
                        "Avro duration storage is exactly 12 bytes",
                    ));
                }
                Ok(DataType::Temporal(TemporalType::Interval(
                    IntervalUnit::MonthDayMillisUnsigned,
                )))
            }
            "uuid" => {
                storage_is(&["string", "fixed"])?;
                if storage == "fixed"
                    && map
                        .get("size")
                        .map(|value| as_u32(value, &field_path(path, "size")))
                        .transpose()?
                        != Some(16)
                {
                    return Err(ProjectionError::new(
                        field_path(path, "size"),
                        LossReason::InvalidSchema,
                        "a fixed UUID is exactly 16 bytes",
                    ));
                }
                Ok(DataType::Uuid)
            }
            other => Err(ProjectionError::new(
                field_path(path, "logicalType"),
                LossReason::UnsupportedSource,
                format!("unknown Avro logical type `{other}`"),
            )),
        }
    }

    fn union(
        &mut self,
        branches: &[Value],
        path: &str,
        namespace: &str,
    ) -> Result<DataType, ProjectionError> {
        if branches.len() < 2 {
            return Err(ProjectionError::new(
                path,
                LossReason::InvalidSchema,
                "an Avro union needs at least two branches",
            ));
        }
        let mut projected = Vec::with_capacity(branches.len());
        let mut names = Vec::with_capacity(branches.len());
        let mut seen_names = BTreeSet::new();
        for (index, branch) in branches.iter().enumerate() {
            let branch_path = index_path(path, index);
            projected.push(self.schema(branch, &branch_path, namespace)?);
            let name = case_name(branch, &branch_path, namespace, &self.definitions)?;
            if !seen_names.insert(name.clone()) {
                return Err(ProjectionError::new(
                    branch_path,
                    LossReason::InvalidSchema,
                    format!("Avro union branch type or name `{name}` appears twice"),
                ));
            }
            names.push(name);
        }
        match DataType::structural_union(projected.clone()) {
            Ok(DataType::StructuralUnion(union)) => {
                let mapping = projected
                    .iter()
                    .map(|branch| {
                        let index = union
                            .branches()
                            .iter()
                            .position(|candidate| candidate == branch)
                            .and_then(|index| u16::try_from(index).ok())
                            .ok_or_else(|| {
                                ProjectionError::new(
                                    path,
                                    LossReason::InvalidSchema,
                                    "canonical union lost an Avro branch",
                                )
                            })?;
                        Ok(UnionSelection::Structural(index))
                    })
                    .collect::<Result<Vec<_>, ProjectionError>>()?;
                self.unions.push(AvroUnionMapping {
                    path: path.to_string(),
                    branches: mapping,
                });
                Ok(DataType::StructuralUnion(union))
            }
            Ok(_) => Err(ProjectionError::new(
                path,
                LossReason::InvalidSchema,
                "the structural-union constructor returned a non-union type",
            )),
            Err(
                TypeError::DuplicateUnionBranch(_) | TypeError::OverlappingUnionBranches { .. },
            ) => {
                let mut cases = Vec::with_capacity(projected.len());
                for (name, branch) in names.iter().cloned().zip(projected) {
                    let payload = match branch {
                        DataType::Struct(payload) => payload,
                        other => StructType::new(vec![TypeField::new("value", other)])
                            .map_err(|error| type_error(path, error))?,
                    };
                    cases.push(
                        TaggedCase::new(name, payload).map_err(|error| type_error(path, error))?,
                    );
                }
                let union = TaggedUnionType::new(cases).map_err(|error| type_error(path, error))?;
                self.unions.push(AvroUnionMapping {
                    path: path.to_string(),
                    branches: names.into_iter().map(UnionSelection::Tagged).collect(),
                });
                Ok(DataType::TaggedUnion(union))
            }
            Err(error) => Err(type_error(path, error)),
        }
    }
}

fn primitive(kind: &str) -> Option<DataType> {
    Some(match kind {
        "null" => DataType::Null,
        "boolean" => DataType::Bool,
        "int" => DataType::Integer(IntegerType::Signed(IntegerWidth::Bits32)),
        "long" => DataType::Integer(IntegerType::Signed(IntegerWidth::Bits64)),
        "float" => DataType::Float(FloatType::Binary32),
        "double" => DataType::Float(FloatType::Binary64),
        "bytes" => DataType::BYTES,
        "string" => DataType::Text,
        _ => return None,
    })
}

fn reverse_type(ty: &DataType, path: &str, name: &str) -> Result<Value, ProjectionError> {
    Ok(match ty {
        DataType::Null => text("null"),
        DataType::Bool => text("boolean"),
        DataType::Integer(IntegerType::Signed(IntegerWidth::Bits32)) => text("int"),
        DataType::Integer(IntegerType::Signed(IntegerWidth::Bits64)) => text("long"),
        DataType::Float(FloatType::Binary32) => text("float"),
        DataType::Float(FloatType::Binary64) => text("double"),
        DataType::Text => text("string"),
        DataType::MediaType => {
            return unsupported_target(path, "Avro V1 has no media-type logical type");
        }
        DataType::Bytes(BinaryType::Variable) => text("bytes"),
        DataType::Bytes(BinaryType::Fixed(length)) => object([
            (String::from("type"), text("fixed")),
            (String::from("name"), text(valid_generated_name(name))),
            (String::from("size"), Value::Uint(u64::from(length.get()))),
        ]),
        DataType::Decimal(decimal) if decimal.scale() >= 0 => object([
            (String::from("type"), text("bytes")),
            (String::from("logicalType"), text("decimal")),
            (
                String::from("precision"),
                Value::Uint(u64::from(decimal.precision())),
            ),
            (
                String::from("scale"),
                Value::Uint(u64::try_from(decimal.scale()).unwrap_or(0)),
            ),
        ]),
        DataType::Decimal(_) => {
            return unsupported_target(path, "Avro decimal scale cannot be negative");
        }
        DataType::Temporal(temporal) => reverse_temporal(temporal, path, name)?,
        DataType::Uuid => object([
            (String::from("type"), text("string")),
            (String::from("logicalType"), text("uuid")),
        ]),
        DataType::Struct(structure) => reverse_record(structure, path, name)?,
        DataType::Map(map) if map.key() == &DataType::Text => object([
            (String::from("type"), text("map")),
            (
                String::from("values"),
                reverse_type(
                    map.value(),
                    &field_path(path, "values"),
                    &format!("{name}Value"),
                )?,
            ),
        ]),
        DataType::Map(_) => return unsupported_target(path, "Avro map keys are text only"),
        DataType::Array(array) if array.length().is_none() => object([
            (String::from("type"), text("array")),
            (
                String::from("items"),
                reverse_type(
                    array.element(),
                    &field_path(path, "items"),
                    &format!("{name}Item"),
                )?,
            ),
        ]),
        DataType::Array(_) => {
            return unsupported_target(path, "Avro arrays cannot preserve an exact length");
        }
        DataType::Enum(enumeration) => object([
            (String::from("type"), text("enum")),
            (String::from("name"), text(valid_generated_name(name))),
            (
                String::from("symbols"),
                Value::Array(
                    enumeration
                        .symbols()
                        .iter()
                        .cloned()
                        .map(Value::Text)
                        .collect(),
                ),
            ),
        ]),
        DataType::StructuralUnion(union) => Value::Array(
            union
                .branches()
                .iter()
                .enumerate()
                .map(|(index, branch)| {
                    reverse_type(
                        branch,
                        &index_path(path, index),
                        &format!("{name}Branch{index}"),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        DataType::TaggedUnion(union) => reverse_tagged_union(union, path)?,
        DataType::Integer(_) => {
            return unsupported_target(path, "Avro cannot preserve this integer width or sign");
        }
        DataType::Float(_) => {
            return unsupported_target(path, "Avro cannot preserve binary16");
        }
        DataType::Number => {
            return unsupported_target(path, "Avro has no exact integer-or-decimal number type");
        }
        DataType::Variant => {
            return unsupported_target(path, "Avro needs a resolved schema, not variant");
        }
    })
}

fn reverse_record(
    structure: &StructType,
    path: &str,
    name: &str,
) -> Result<Value, ProjectionError> {
    let fields = structure
        .fields()
        .iter()
        .map(|field| {
            let field_path = field_path(path, field.name());
            valid_simple_name(field.name(), &field_path)?;
            Ok(object([
                (String::from("name"), text(field.name())),
                (
                    String::from("type"),
                    reverse_type(
                        field.ty(),
                        &field_path,
                        &format!("{}{}", name, title_word(field.name())),
                    )?,
                ),
            ]))
        })
        .collect::<Result<Vec<_>, ProjectionError>>()?;
    Ok(object([
        (String::from("type"), text("record")),
        (String::from("name"), text(valid_generated_name(name))),
        (String::from("fields"), Value::Array(fields)),
    ]))
}

fn reverse_tagged_union(union: &TaggedUnionType, path: &str) -> Result<Value, ProjectionError> {
    if union.cases().len() < 2 {
        return unsupported_target(path, "an Avro union needs at least two cases");
    }
    let payload_types = union
        .cases()
        .iter()
        .map(|case| DataType::Struct(case.payload().clone()))
        .collect::<Vec<_>>();
    if DataType::structural_union(payload_types).is_ok() {
        return unsupported_target(
            path,
            "Avro would erase these tagged case names because their payloads are structurally disjoint",
        );
    }
    let branches = union
        .cases()
        .iter()
        .enumerate()
        .map(|(index, case)| {
            valid_full_name(case.name(), &index_path(path, index))?;
            reverse_record(case.payload(), &index_path(path, index), case.name())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Array(branches))
}

fn reverse_temporal(
    temporal: &TemporalType,
    path: &str,
    name: &str,
) -> Result<Value, ProjectionError> {
    let logical = |storage: &str, logical: &str| {
        object([
            (String::from("type"), text(storage)),
            (String::from("logicalType"), text(logical)),
        ])
    };
    Ok(match temporal {
        TemporalType::Date(DateUnit::Day) => logical("int", "date"),
        TemporalType::Time(TimeUnit::Millisecond) => logical("int", "time-millis"),
        TemporalType::Time(TimeUnit::Microsecond) => logical("long", "time-micros"),
        TemporalType::Time(TimeUnit::Nanosecond) => logical("long", "time-nanos"),
        TemporalType::Timestamp {
            unit: TimeUnit::Millisecond,
            zone: TimeZone::Utc,
        } => logical("long", "timestamp-millis"),
        TemporalType::Timestamp {
            unit: TimeUnit::Microsecond,
            zone: TimeZone::Utc,
        } => logical("long", "timestamp-micros"),
        TemporalType::Timestamp {
            unit: TimeUnit::Nanosecond,
            zone: TimeZone::Utc,
        } => logical("long", "timestamp-nanos"),
        TemporalType::TimestampNtz(TimeUnit::Millisecond) => {
            logical("long", "local-timestamp-millis")
        }
        TemporalType::TimestampNtz(TimeUnit::Microsecond) => {
            logical("long", "local-timestamp-micros")
        }
        TemporalType::TimestampNtz(TimeUnit::Nanosecond) => {
            logical("long", "local-timestamp-nanos")
        }
        TemporalType::Interval(IntervalUnit::MonthDayMillisUnsigned) => object([
            (String::from("type"), text("fixed")),
            (String::from("name"), text(valid_generated_name(name))),
            (String::from("size"), Value::Uint(12)),
            (String::from("logicalType"), text("duration")),
        ]),
        _ => return unsupported_target(path, "Avro cannot preserve this temporal parameter set"),
    })
}

fn definition_name(
    map: &BTreeMap<String, Value>,
    path: &str,
    current_namespace: &str,
) -> Result<String, ProjectionError> {
    let name_path = field_path(path, "name");
    let name = as_text(
        map.get("name").ok_or_else(|| {
            ProjectionError::new(
                &name_path,
                LossReason::InvalidSchema,
                "named type needs name",
            )
        })?,
        &name_path,
    )?;
    let explicit_namespace = map
        .get("namespace")
        .map(|value| as_text(value, &field_path(path, "namespace")))
        .transpose()?;
    let full = if name.contains('.') {
        name.to_string()
    } else {
        let namespace = explicit_namespace.unwrap_or(current_namespace);
        if namespace.is_empty() {
            name.to_string()
        } else {
            format!("{namespace}.{name}")
        }
    };
    valid_full_name(&full, &name_path)?;
    Ok(full)
}

fn resolve_name(
    name: &str,
    namespace: &str,
    definitions: &BTreeMap<String, Value>,
) -> Option<String> {
    if name.contains('.') {
        return definitions.contains_key(name).then(|| name.to_string());
    }
    if !namespace.is_empty() {
        let qualified = format!("{namespace}.{name}");
        if definitions.contains_key(&qualified) {
            return Some(qualified);
        }
    }
    definitions.contains_key(name).then(|| name.to_string())
}

fn case_name(
    schema: &Value,
    path: &str,
    namespace: &str,
    definitions: &BTreeMap<String, Value>,
) -> Result<String, ProjectionError> {
    match schema {
        Value::Text(name) => {
            Ok(resolve_name(name, namespace, definitions).unwrap_or_else(|| name.clone()))
        }
        Value::Map(map) => {
            let kind = as_text(
                map.get("type").ok_or_else(|| {
                    ProjectionError::new(path, LossReason::InvalidSchema, "missing Avro type")
                })?,
                &field_path(path, "type"),
            )?;
            if matches!(kind, "record" | "enum" | "fixed") {
                definition_name(map, path, namespace)
            } else if map.get("logicalType").is_some() {
                // Avro union uniqueness is defined by the underlying schema
                // type, not by a logical-type annotation. Two differently
                // annotated `long` branches are still duplicate `long`s.
                Ok(kind.to_string())
            } else if let Some(full) = resolve_name(kind, namespace, definitions) {
                Ok(full)
            } else {
                Ok(kind.to_string())
            }
        }
        _ => Err(ProjectionError::new(
            path,
            LossReason::InvalidSchema,
            "nested union branches are not admitted",
        )),
    }
}

fn namespace_of(full: &str) -> String {
    full.rsplit_once('.')
        .map_or_else(String::new, |(namespace, _)| namespace.to_string())
}

fn valid_simple_name(name: &str, path: &str) -> Result<(), ProjectionError> {
    let mut bytes = name.bytes();
    if !bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(ProjectionError::new(
            path,
            LossReason::InvalidSchema,
            format!("`{name}` is not an Avro name"),
        ));
    }
    Ok(())
}

fn valid_full_name(name: &str, path: &str) -> Result<(), ProjectionError> {
    for part in name.split('.') {
        valid_simple_name(part, path)?;
    }
    Ok(())
}

fn valid_generated_name(name: &str) -> String {
    name.split('.')
        .map(|part| {
            let mut generated = String::new();
            for (index, character) in part.chars().enumerate() {
                if character.is_ascii_alphanumeric() || character == '_' {
                    if index == 0 && character.is_ascii_digit() {
                        generated.push('_');
                    }
                    generated.push(character);
                } else {
                    generated.push('_');
                }
            }
            if generated.is_empty() {
                generated.push('_');
            }
            generated
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn title_word(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::from("Field"),
    }
}

fn logical_time_unit(logical: &str) -> TimeUnit {
    if logical.ends_with("millis") {
        TimeUnit::Millisecond
    } else if logical.ends_with("micros") {
        TimeUnit::Microsecond
    } else {
        TimeUnit::Nanosecond
    }
}

fn fixed_decimal_holds(size: u32, precision: u32) -> bool {
    let value_bits = size.saturating_mul(8).saturating_sub(1);
    if value_bits >= u128::BITS {
        return true;
    }
    let Some(decimal_bound) = 10_u128.checked_pow(precision) else {
        return false;
    };
    decimal_bound <= 1_u128 << value_bits
}

fn unsupported_target<T>(path: &str, detail: &str) -> Result<T, ProjectionError> {
    Err(ProjectionError::new(
        path,
        LossReason::UnsupportedTarget,
        detail,
    ))
}
