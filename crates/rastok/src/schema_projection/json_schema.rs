//! Strict JSON Schema 2020-12 projection.

use std::collections::{BTreeMap, BTreeSet};

use crate::datatype::{
    ArrayType, DataType, EnumType, IntegerType, IntegerWidth, MapType, StructType, TypeField,
};
use crate::value::Value;

use super::{
    LossReason, ProjectionError, as_array, as_object, as_text, as_u32, exact_integer, field_path,
    index_path, integer_value, object, parse_json_document, render_json_document, text, type_error,
};

/// Projects one complete JSON Schema document into a canonical Rastok type.
///
/// Local `#/$defs/...` references are resolved. A recursive reference, an
/// unsupported vocabulary or a projection that would hide a constraint is a
/// typed, path-specific refusal.
pub fn project(document: &str) -> Result<DataType, ProjectionError> {
    let root = parse_json_document(document)?;
    let defs = root
        .as_map()
        .and_then(|map| map.get("$defs"))
        .map(|value| as_object(value, "$.$defs"))
        .transpose()?
        .cloned()
        .unwrap_or_default();
    let mut active = BTreeSet::new();
    let ty = project_value(&root, "$", &defs, &mut active, true)?;
    ty.validate().map_err(|error| type_error("$", error))?;
    for (name, definition) in &defs {
        active.clear();
        let definition_path = field_path("$.$defs", name);
        let definition_ty = project_value(definition, &definition_path, &defs, &mut active, false)?;
        definition_ty
            .validate()
            .map_err(|error| type_error(&definition_path, error))?;
    }
    Ok(ty)
}

/// Reverses a canonical type into one canonical JSON Schema document.
///
/// Only distinctions represented exactly by the standard 2020-12
/// vocabularies are admitted. For example, bytes, fixed decimals, temporal
/// parameters and tagged discriminants refuse rather than becoming strings or
/// numbers with weaker meaning.
pub fn reverse(ty: &DataType) -> Result<String, ProjectionError> {
    let schema = reverse_type(ty, "$", true)?;
    let document = render_json_document(&schema)?;
    let round_trip = project(&document).map_err(|error| {
        ProjectionError::new(
            "$",
            LossReason::UnsupportedTarget,
            format!("generated JSON Schema is not in the exact profile: {error}"),
        )
    })?;
    if round_trip != *ty {
        return Err(ProjectionError::new(
            "$",
            LossReason::LossyProjection,
            "generated JSON Schema changes the logical type",
        ));
    }
    Ok(document)
}

fn project_value(
    value: &Value,
    path: &str,
    defs: &BTreeMap<String, Value>,
    active: &mut BTreeSet<String>,
    root: bool,
) -> Result<DataType, ProjectionError> {
    let map = match value {
        Value::Map(map) => map,
        Value::Bool(_) => {
            return Err(ProjectionError::new(
                path,
                LossReason::UnsupportedSource,
                "boolean JSON Schemas are outside the V1 profile",
            ));
        }
        _ => {
            return Err(ProjectionError::new(
                path,
                LossReason::InvalidSchema,
                "expected a JSON Schema object",
            ));
        }
    };
    ensure_known_keywords(map, path, root)?;

    if let Some(reference) = map.get("$ref") {
        let reference_path = field_path(path, "$ref");
        let reference = as_text(reference, &reference_path)?;
        if map
            .keys()
            .any(|key| is_assertion_keyword(key) && key != "$ref")
        {
            return Err(ProjectionError::new(
                path,
                LossReason::UnsupportedSource,
                "a reference with sibling assertions is outside the V1 profile",
            ));
        }
        let name = local_definition_name(reference).ok_or_else(|| {
            ProjectionError::new(
                &reference_path,
                LossReason::UnsupportedSource,
                "only local #/$defs/... references are admitted",
            )
        })?;
        let definition = defs.get(&name).ok_or_else(|| {
            ProjectionError::new(
                field_path("$.$defs", &name),
                LossReason::InvalidSchema,
                "the referenced definition does not exist",
            )
        })?;
        if !active.insert(name.clone()) {
            return Err(ProjectionError::new(
                reference_path,
                LossReason::RecursiveSchema,
                format!("definition `{name}` reaches itself"),
            ));
        }
        let result = project_value(
            definition,
            &field_path("$.$defs", &name),
            defs,
            active,
            false,
        );
        active.remove(&name);
        return result;
    }

    if let Some(one_of) = map.get("oneOf") {
        if map
            .keys()
            .any(|key| is_assertion_keyword(key) && key != "oneOf")
        {
            return Err(ProjectionError::new(
                path,
                LossReason::UnsupportedSource,
                "oneOf cannot be combined with another assertion in the V1 profile",
            ));
        }
        let branches = as_array(one_of, &field_path(path, "oneOf"))?;
        let projected = branches
            .iter()
            .enumerate()
            .map(|(index, branch)| {
                project_value(
                    branch,
                    &index_path(&field_path(path, "oneOf"), index),
                    defs,
                    active,
                    false,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        return DataType::structural_union(projected).map_err(|error| type_error(path, error));
    }

    let type_value = map.get("type").ok_or_else(|| {
        ProjectionError::new(
            path,
            LossReason::UnsupportedSource,
            "the V1 profile needs type, oneOf or $ref",
        )
    })?;
    match type_value {
        Value::Text(kind) => project_typed(kind, map, path, defs, active),
        Value::Array(kinds) => {
            ensure_no_constraints(map, path, &["type"])?;
            if kinds.is_empty() {
                return Err(ProjectionError::new(
                    field_path(path, "type"),
                    LossReason::InvalidSchema,
                    "type must contain at least one type name",
                ));
            }
            let mut seen = BTreeSet::new();
            let mut branches = Vec::with_capacity(kinds.len());
            for (index, kind) in kinds.iter().enumerate() {
                let kind_path = index_path(&field_path(path, "type"), index);
                let kind = as_text(kind, &kind_path)?;
                if !seen.insert(kind) {
                    return Err(ProjectionError::new(
                        kind_path,
                        LossReason::InvalidSchema,
                        format!("type `{kind}` appears twice"),
                    ));
                }
                branches.push(project_typed(
                    kind,
                    &BTreeMap::new(),
                    &kind_path,
                    defs,
                    active,
                )?);
            }
            if branches.len() == 1 {
                Ok(branches.remove(0))
            } else {
                DataType::structural_union(branches).map_err(|error| type_error(path, error))
            }
        }
        _ => Err(ProjectionError::new(
            field_path(path, "type"),
            LossReason::InvalidSchema,
            "type must be text or an array of type names",
        )),
    }
}

fn project_typed(
    kind: &str,
    map: &BTreeMap<String, Value>,
    path: &str,
    defs: &BTreeMap<String, Value>,
    active: &mut BTreeSet<String>,
) -> Result<DataType, ProjectionError> {
    match kind {
        "null" => {
            ensure_no_constraints(map, path, &["type"])?;
            Ok(DataType::Null)
        }
        "boolean" => {
            ensure_no_constraints(map, path, &["type"])?;
            Ok(DataType::Bool)
        }
        "string" => {
            if let Some(symbols) = map.get("enum") {
                ensure_no_constraints(map, path, &["type", "enum"])?;
                let symbols_path = field_path(path, "enum");
                let symbols = as_array(symbols, &symbols_path)?
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        as_text(value, &index_path(&symbols_path, index)).map(String::from)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                return EnumType::new(symbols)
                    .map(DataType::Enum)
                    .map_err(|error| type_error(&symbols_path, error));
            }
            ensure_no_constraints(map, path, &["type"])?;
            Ok(DataType::Text)
        }
        "integer" => {
            ensure_no_constraints(map, path, &["type", "minimum", "maximum"])?;
            project_integer(map, path)
        }
        "number" => {
            ensure_no_constraints(map, path, &["type"])?;
            Ok(DataType::Number)
        }
        "array" => {
            ensure_no_constraints(map, path, &["type", "items", "minItems", "maxItems"])?;
            let items_path = field_path(path, "items");
            let items = map.get("items").ok_or_else(|| {
                ProjectionError::new(
                    &items_path,
                    LossReason::UnsupportedSource,
                    "a homogeneous array needs one items schema",
                )
            })?;
            let element = project_value(items, &items_path, defs, active, false)?;
            match (map.get("minItems"), map.get("maxItems")) {
                (None, None) => Ok(DataType::array(element)),
                (Some(min), Some(max)) => {
                    let min = as_u32(min, &field_path(path, "minItems"))?;
                    let max = as_u32(max, &field_path(path, "maxItems"))?;
                    if min != max {
                        return Err(ProjectionError::new(
                            path,
                            LossReason::LossyProjection,
                            "unequal minItems/maxItems cannot be represented exactly",
                        ));
                    }
                    ArrayType::fixed(element, min)
                        .map(DataType::Array)
                        .map_err(|error| type_error(path, error))
                }
                _ => Err(ProjectionError::new(
                    path,
                    LossReason::LossyProjection,
                    "a one-sided array length constraint cannot be represented exactly",
                )),
            }
        }
        "object" => {
            ensure_no_constraints(
                map,
                path,
                &["type", "properties", "required", "additionalProperties"],
            )?;
            project_object(map, path, defs, active)
        }
        other => Err(ProjectionError::new(
            field_path(path, "type"),
            LossReason::UnsupportedSource,
            format!("JSON Schema type `{other}` is outside the V1 profile"),
        )),
    }
}

fn project_integer(map: &BTreeMap<String, Value>, path: &str) -> Result<DataType, ProjectionError> {
    let minimum_path = field_path(path, "minimum");
    let maximum_path = field_path(path, "maximum");
    let minimum = map.get("minimum").ok_or_else(|| {
        ProjectionError::new(
            &minimum_path,
            LossReason::UnsupportedSource,
            "integer needs a proven inclusive lower bound",
        )
    })?;
    let maximum = map.get("maximum").ok_or_else(|| {
        ProjectionError::new(
            &maximum_path,
            LossReason::UnsupportedSource,
            "integer needs a proven inclusive upper bound",
        )
    })?;
    let minimum = exact_integer(minimum, &minimum_path)?;
    let maximum = exact_integer(maximum, &maximum_path)?;
    if minimum > maximum {
        return Err(ProjectionError::new(
            path,
            LossReason::InvalidSchema,
            "minimum is greater than maximum",
        ));
    }
    let width = [
        IntegerWidth::Bits8,
        IntegerWidth::Bits16,
        IntegerWidth::Bits32,
        IntegerWidth::Bits64,
    ];
    if minimum >= 0 {
        let maximum = u128::try_from(maximum).map_err(|_| {
            ProjectionError::new(
                path,
                LossReason::LossyProjection,
                "integer range is invalid",
            )
        })?;
        let selected = width.into_iter().find(|width| {
            let bits = width.bits();
            maximum
                <= if bits == 64 {
                    u64::MAX as u128
                } else {
                    (1_u128 << bits) - 1
                }
        });
        let Some(width) = selected else {
            return Err(ProjectionError::new(
                path,
                LossReason::LossyProjection,
                "integer range exceeds uint64",
            ));
        };
        let exact_maximum = if width.bits() == 64 {
            u128::from(u64::MAX)
        } else {
            (1_u128 << width.bits()) - 1
        };
        if minimum != 0 || maximum != exact_maximum {
            return Err(ProjectionError::new(
                path,
                LossReason::LossyProjection,
                "integer bounds do not exactly match one unsigned width",
            ));
        }
        return Ok(DataType::Integer(IntegerType::Unsigned(width)));
    }
    let selected = width.into_iter().find(|width| {
        let bits = width.bits();
        let lower = -(1_i128 << (bits - 1));
        let upper = (1_i128 << (bits - 1)) - 1;
        minimum >= lower && maximum <= upper
    });
    let Some(width) = selected else {
        return Err(ProjectionError::new(
            path,
            LossReason::LossyProjection,
            "integer range exceeds int64",
        ));
    };
    let bits = width.bits();
    let exact_minimum = -(1_i128 << (bits - 1));
    let exact_maximum = (1_i128 << (bits - 1)) - 1;
    if minimum != exact_minimum || maximum != exact_maximum {
        return Err(ProjectionError::new(
            path,
            LossReason::LossyProjection,
            "integer bounds do not exactly match one signed width",
        ));
    }
    Ok(DataType::Integer(IntegerType::Signed(width)))
}

fn project_object(
    map: &BTreeMap<String, Value>,
    path: &str,
    defs: &BTreeMap<String, Value>,
    active: &mut BTreeSet<String>,
) -> Result<DataType, ProjectionError> {
    let properties_path = field_path(path, "properties");
    let properties = map
        .get("properties")
        .map(|value| as_object(value, &properties_path))
        .transpose()?
        .cloned()
        .unwrap_or_default();
    let additional = map.get("additionalProperties");
    if properties.is_empty()
        && let Some(schema) = additional
        && !matches!(schema, Value::Bool(false))
    {
        let required_path = field_path(path, "required");
        if let Some(required) = map.get("required") {
            let required = as_array(required, &required_path)?;
            if !required.is_empty() {
                return Err(ProjectionError::new(
                    required_path,
                    LossReason::LossyProjection,
                    "required map keys cannot be represented by a homogeneous map type",
                ));
            }
        }
        if matches!(schema, Value::Bool(true)) {
            return Err(ProjectionError::new(
                field_path(path, "additionalProperties"),
                LossReason::UnsupportedSource,
                "an unconstrained open object has no precise map value type",
            ));
        }
        let value = project_value(
            schema,
            &field_path(path, "additionalProperties"),
            defs,
            active,
            false,
        )?;
        return MapType::new(DataType::Text, value)
            .map(DataType::Map)
            .map_err(|error| type_error(path, error));
    }
    if !matches!(additional, Some(Value::Bool(false))) {
        return Err(ProjectionError::new(
            field_path(path, "additionalProperties"),
            LossReason::UnsupportedSource,
            "a named-property object must close additionalProperties",
        ));
    }
    let required_path = field_path(path, "required");
    let mut required_names = BTreeSet::new();
    if let Some(required) = map.get("required") {
        for (index, name) in as_array(required, &required_path)?.iter().enumerate() {
            let item_path = index_path(&required_path, index);
            let name = as_text(name, &item_path)?;
            if !required_names.insert(name) {
                return Err(ProjectionError::new(
                    item_path,
                    LossReason::InvalidSchema,
                    format!("required property `{name}` appears twice"),
                ));
            }
            if !properties.contains_key(name) {
                return Err(ProjectionError::new(
                    item_path,
                    LossReason::InvalidSchema,
                    format!("required property `{name}` is not declared"),
                ));
            }
        }
    }
    if let Some(optional) = properties
        .keys()
        .find(|name| !required_names.contains(name.as_str()))
    {
        return Err(ProjectionError::new(
            field_path(&properties_path, optional),
            LossReason::LossyProjection,
            "an optional property has no exact closed-struct representation",
        ));
    }
    let fields = properties
        .iter()
        .map(|(name, schema)| {
            let field = field_path(&properties_path, name);
            project_value(schema, &field, defs, active, false)
                .map(|ty| TypeField::new(name.clone(), ty))
        })
        .collect::<Result<Vec<_>, _>>()?;
    StructType::new(fields)
        .map(DataType::Struct)
        .map_err(|error| type_error(path, error))
}

fn reverse_type(ty: &DataType, path: &str, root: bool) -> Result<Value, ProjectionError> {
    let mut schema = match ty {
        DataType::Null => typed("null"),
        DataType::Bool => typed("boolean"),
        DataType::Text => typed("string"),
        DataType::Number => typed("number"),
        DataType::Integer(integer) => reverse_integer(*integer)?,
        DataType::Enum(enumeration) => object([
            (String::from("type"), text("string")),
            (
                String::from("enum"),
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
        DataType::Array(array) => {
            let mut fields = BTreeMap::from([
                (String::from("type"), text("array")),
                (
                    String::from("items"),
                    reverse_type(array.element(), &field_path(path, "items"), false)?,
                ),
            ]);
            if let Some(length) = array.length() {
                fields.insert(
                    String::from("minItems"),
                    Value::Uint(u64::from(length.get())),
                );
                fields.insert(
                    String::from("maxItems"),
                    Value::Uint(u64::from(length.get())),
                );
            }
            Value::Map(fields)
        }
        DataType::Map(map) if map.key() == &DataType::Text => object([
            (String::from("type"), text("object")),
            (String::from("properties"), object([])),
            (
                String::from("additionalProperties"),
                reverse_type(
                    map.value(),
                    &field_path(path, "additionalProperties"),
                    false,
                )?,
            ),
        ]),
        DataType::Map(_) => return unsupported_target(path, "JSON object keys must be text"),
        DataType::Struct(structure) => {
            let properties = structure
                .fields()
                .iter()
                .map(|field| {
                    Ok((
                        field.name().to_string(),
                        reverse_type(
                            field.ty(),
                            &field_path(&field_path(path, "properties"), field.name()),
                            false,
                        )?,
                    ))
                })
                .collect::<Result<BTreeMap<_, _>, ProjectionError>>()?;
            object([
                (String::from("type"), text("object")),
                (String::from("properties"), Value::Map(properties)),
                (
                    String::from("required"),
                    Value::Array(
                        structure
                            .fields()
                            .iter()
                            .map(|field| text(field.name()))
                            .collect(),
                    ),
                ),
                (String::from("additionalProperties"), Value::Bool(false)),
            ])
        }
        DataType::StructuralUnion(union) => object([(
            String::from("oneOf"),
            Value::Array(
                union
                    .branches()
                    .iter()
                    .enumerate()
                    .map(|(index, branch)| reverse_type(branch, &index_path(path, index), false))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        )]),
        DataType::Float(_) => {
            return unsupported_target(path, "JSON Schema cannot preserve a binary float width");
        }
        DataType::Decimal(_) => {
            return unsupported_target(path, "standard JSON Schema cannot preserve decimal scale");
        }
        DataType::Bytes(_) => {
            return unsupported_target(path, "standard JSON Schema has no exact opaque-bytes type");
        }
        DataType::Temporal(_) => {
            return unsupported_target(
                path,
                "standard JSON Schema cannot preserve temporal parameters",
            );
        }
        DataType::Uuid => {
            return unsupported_target(
                path,
                "UUID semantics need a registered JSON Schema profile",
            );
        }
        DataType::TaggedUnion(_) => {
            return unsupported_target(
                path,
                "standard JSON Schema cannot preserve tagged discriminants",
            );
        }
        DataType::Variant => {
            return unsupported_target(
                path,
                "variant includes values outside the JSON data domain",
            );
        }
    };
    if root {
        let Value::Map(fields) = &mut schema else {
            return Ok(schema);
        };
        fields.insert(
            String::from("$schema"),
            text("https://json-schema.org/draft/2020-12/schema"),
        );
    }
    Ok(schema)
}

fn reverse_integer(integer: IntegerType) -> Result<Value, ProjectionError> {
    let (minimum, maximum) = match integer {
        IntegerType::Signed(width) => {
            let shift = width.bits() - 1;
            (-(1_i128 << shift), (1_i128 << shift) - 1)
        }
        IntegerType::Unsigned(width) => {
            let maximum = if width.bits() == 64 {
                i128::from(u64::MAX)
            } else {
                (1_i128 << width.bits()) - 1
            };
            (0, maximum)
        }
    };
    Ok(object([
        (String::from("type"), text("integer")),
        (String::from("minimum"), integer_value(minimum)?),
        (String::from("maximum"), integer_value(maximum)?),
    ]))
}

fn typed(kind: &str) -> Value {
    object([(String::from("type"), text(kind))])
}

fn unsupported_target<T>(path: &str, detail: &str) -> Result<T, ProjectionError> {
    Err(ProjectionError::new(
        path,
        LossReason::UnsupportedTarget,
        detail,
    ))
}

fn local_definition_name(reference: &str) -> Option<String> {
    let encoded = reference.strip_prefix("#/$defs/")?;
    if encoded.is_empty() || encoded.contains('/') {
        return None;
    }
    let mut decoded = String::with_capacity(encoded.len());
    let mut characters = encoded.chars();
    while let Some(character) = characters.next() {
        if character != '~' {
            decoded.push(character);
            continue;
        }
        decoded.push(match characters.next()? {
            '0' => '~',
            '1' => '/',
            _ => return None,
        });
    }
    Some(decoded)
}

fn ensure_known_keywords(
    map: &BTreeMap<String, Value>,
    path: &str,
    root: bool,
) -> Result<(), ProjectionError> {
    const KNOWN: &[&str] = &[
        "$schema",
        "$id",
        "$anchor",
        "$comment",
        "$ref",
        "$defs",
        "title",
        "description",
        "default",
        "examples",
        "deprecated",
        "readOnly",
        "writeOnly",
        "type",
        "enum",
        "minimum",
        "maximum",
        "items",
        "minItems",
        "maxItems",
        "properties",
        "required",
        "additionalProperties",
        "oneOf",
    ];
    for key in map.keys() {
        if !KNOWN.contains(&key.as_str()) || (key == "$defs" && !root) {
            return Err(ProjectionError::new(
                field_path(path, key),
                LossReason::UnsupportedSource,
                format!("keyword `{key}` is outside the V1 profile"),
            ));
        }
    }
    Ok(())
}

fn ensure_no_constraints(
    map: &BTreeMap<String, Value>,
    path: &str,
    allowed: &[&str],
) -> Result<(), ProjectionError> {
    for key in map.keys() {
        if !allowed.contains(&key.as_str())
            && !is_annotation_keyword(key)
            && !matches!(key.as_str(), "$schema" | "$id" | "$defs")
        {
            return Err(ProjectionError::new(
                field_path(path, key),
                LossReason::UnsupportedSource,
                format!("constraint `{key}` cannot be preserved by this projection"),
            ));
        }
    }
    Ok(())
}

fn is_assertion_keyword(key: &str) -> bool {
    !is_annotation_keyword(key) && key != "$defs" && key != "$schema" && key != "$id"
}

fn is_annotation_keyword(key: &str) -> bool {
    matches!(
        key,
        "$anchor"
            | "$comment"
            | "title"
            | "description"
            | "default"
            | "examples"
            | "deprecated"
            | "readOnly"
            | "writeOnly"
    )
}

trait ValueMap {
    fn as_map(&self) -> Option<&BTreeMap<String, Value>>;
}

impl ValueMap for Value {
    fn as_map(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Value::Map(map) => Some(map),
            _ => None,
        }
    }
}
