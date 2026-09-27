//! Strict OpenAPI 3 schema projection used by the project resolver.
//!
//! OpenAPI operation discovery is project tooling; this module owns only the
//! lossless projection of `components.schemas` into Rastok's one logical type
//! algebra. Annotation keywords are ignored, constraints Rastok cannot carry
//! refuse, and local component references are expanded before source output.

use std::collections::{BTreeMap, BTreeSet};

use crate::datatype::{DataType, EnumType, FloatType, MapType, StructType, TypeField};
use crate::value::Value;

use super::{
    LossReason, ProjectionError, as_array, as_object, as_text, field_path, index_path,
    parse_json_document, type_error,
};

/// Projects every named schema in one OpenAPI 3 document.
///
/// The document is JSON text. YAML is a project-file concern and is converted
/// to the same data tree before reaching this boundary.
///
/// # Errors
///
/// Returns the first path-specific invalid, unsupported or lossy construct.
pub fn project_components(document: &str) -> Result<BTreeMap<String, DataType>, ProjectionError> {
    let root = parse_json_document(document)?;
    let root = as_object(&root, "$")?;
    let version = root.get("openapi").ok_or_else(|| {
        ProjectionError::new(
            "$.openapi",
            LossReason::InvalidSchema,
            "missing OpenAPI version",
        )
    })?;
    let version = as_text(version, "$.openapi")?;
    if !version.starts_with("3.0.") && !version.starts_with("3.1.") {
        return Err(ProjectionError::new(
            "$.openapi",
            LossReason::UnsupportedSource,
            format!("OpenAPI version `{version}` is outside the 3.0/3.1 profile"),
        ));
    }
    let components = root
        .get("components")
        .map(|value| as_object(value, "$.components"))
        .transpose()?
        .cloned()
        .unwrap_or_default();
    let schemas = components
        .get("schemas")
        .map(|value| as_object(value, "$.components.schemas"))
        .transpose()?
        .cloned()
        .unwrap_or_default();
    let mut projected = BTreeMap::new();
    for (name, schema) in &schemas {
        let mut active = BTreeSet::new();
        active.insert(name.clone());
        let path = field_path("$.components.schemas", name);
        let ty = project_schema(schema, &path, &schemas, &mut active)?;
        ty.check_bounds()
            .map_err(|error| type_error(&path, error))?;
        projected.insert(name.clone(), ty);
    }
    Ok(projected)
}

fn project_schema(
    value: &Value,
    path: &str,
    schemas: &BTreeMap<String, Value>,
    active: &mut BTreeSet<String>,
) -> Result<DataType, ProjectionError> {
    let map = as_object(value, path)?;
    if let Some(Value::Array(types)) = map.get("type") {
        let mut names = types
            .iter()
            .enumerate()
            .map(|(index, value)| {
                as_text(value, &index_path(&field_path(path, "type"), index)).map(String::from)
            })
            .collect::<Result<Vec<_>, _>>()?;
        names.sort();
        if names.len() != 2 || names[0] != "null" || names[1] == "null" {
            return Err(ProjectionError::new(
                field_path(path, "type"),
                LossReason::UnsupportedSource,
                "the OpenAPI 3.1 type array profile admits exactly one concrete type plus null",
            ));
        }
        let concrete = names.remove(1);
        let mut normalized = map.clone();
        normalized.insert(String::from("type"), Value::Text(concrete));
        let projected = project_schema(&Value::Map(normalized), path, schemas, active)?;
        return projected
            .optional()
            .map_err(|error| type_error(path, error));
    }
    if let Some(reference) = map.get("$ref") {
        ensure_only_annotations(map, path, &["$ref"])?;
        let reference = as_text(reference, &field_path(path, "$ref"))?;
        let name = component_reference(reference).ok_or_else(|| {
            ProjectionError::new(
                field_path(path, "$ref"),
                LossReason::UnsupportedSource,
                "only local #/components/schemas/... references are admitted",
            )
        })?;
        let target = schemas.get(&name).ok_or_else(|| {
            ProjectionError::new(
                field_path("$.components.schemas", &name),
                LossReason::InvalidSchema,
                "the referenced component does not exist",
            )
        })?;
        if !active.insert(name.clone()) {
            return Err(ProjectionError::new(
                path,
                LossReason::RecursiveSchema,
                format!("component `{name}` reaches itself"),
            ));
        }
        let result = project_schema(
            target,
            &field_path("$.components.schemas", &name),
            schemas,
            active,
        );
        active.remove(&name);
        return result;
    }
    if let Some(one_of) = map.get("oneOf") {
        ensure_only_annotations(map, path, &["oneOf", "nullable", "discriminator"])?;
        if map.contains_key("discriminator") {
            return Err(ProjectionError::new(
                field_path(path, "discriminator"),
                LossReason::UnsupportedSource,
                "OpenAPI discriminators need a named tagged-union mapping, not structural oneOf",
            ));
        }
        let branches = as_array(one_of, &field_path(path, "oneOf"))?
            .iter()
            .enumerate()
            .map(|(index, branch)| {
                project_schema(
                    branch,
                    &index_path(&field_path(path, "oneOf"), index),
                    schemas,
                    active,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let ty = DataType::structural_union(branches).map_err(|error| type_error(path, error))?;
        return nullable(ty, map, path);
    }
    let kind = map
        .get("type")
        .ok_or_else(|| {
            ProjectionError::new(
                path,
                LossReason::UnsupportedSource,
                "a component schema needs `type`, `oneOf` or `$ref`",
            )
        })
        .and_then(|value| as_text(value, &field_path(path, "type")))?;
    let ty = match kind {
        "null" => {
            ensure_only_annotations(map, path, &["type"])?;
            DataType::Null
        }
        "boolean" => {
            ensure_only_annotations(map, path, &["type", "nullable"])?;
            DataType::Bool
        }
        "string" => project_string(map, path)?,
        "integer" => project_integer(map, path)?,
        "number" => project_number(map, path)?,
        "array" => {
            ensure_only_annotations(map, path, &["type", "items", "nullable"])?;
            let item_path = field_path(path, "items");
            let item = map.get("items").ok_or_else(|| {
                ProjectionError::new(
                    &item_path,
                    LossReason::InvalidSchema,
                    "an array needs one items schema",
                )
            })?;
            DataType::array(project_schema(item, &item_path, schemas, active)?)
        }
        "object" => project_object(map, path, schemas, active)?,
        other => {
            return Err(ProjectionError::new(
                field_path(path, "type"),
                LossReason::UnsupportedSource,
                format!("OpenAPI schema type `{other}` is outside the profile"),
            ));
        }
    };
    nullable(ty, map, path)
}

fn project_string(map: &BTreeMap<String, Value>, path: &str) -> Result<DataType, ProjectionError> {
    if let Some(symbols) = map.get("enum") {
        ensure_only_annotations(map, path, &["type", "enum", "nullable"])?;
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
    ensure_only_annotations(map, path, &["type", "format", "nullable"])?;
    let format = map
        .get("format")
        .map(|value| as_text(value, &field_path(path, "format")))
        .transpose()?;
    Ok(match format {
        None => DataType::Text,
        Some("uuid") => DataType::Uuid,
        Some("date") => DataType::Temporal(crate::datatype::TemporalType::Date(
            crate::datatype::DateUnit::Day,
        )),
        Some("date-time") => DataType::TIMESTAMP,
        Some("binary" | "byte") => DataType::BYTES,
        Some("media-type") => DataType::MediaType,
        Some(other) => {
            return Err(ProjectionError::new(
                field_path(path, "format"),
                LossReason::UnsupportedSource,
                format!("string format `{other}` has no exact Rastok projection"),
            ));
        }
    })
}

fn project_integer(map: &BTreeMap<String, Value>, path: &str) -> Result<DataType, ProjectionError> {
    ensure_only_annotations(map, path, &["type", "format", "nullable"])?;
    let format = map
        .get("format")
        .map(|value| as_text(value, &field_path(path, "format")))
        .transpose()?;
    Ok(match format {
        Some("int32") => DataType::INT,
        Some("int64") => DataType::LONG,
        Some(other) => {
            return Err(ProjectionError::new(
                field_path(path, "format"),
                LossReason::UnsupportedSource,
                format!("integer format `{other}` has no exact Rastok projection"),
            ));
        }
        None => {
            return Err(ProjectionError::new(
                path,
                LossReason::LossyProjection,
                "an unbounded OpenAPI integer needs format int32 or int64",
            ));
        }
    })
}

fn project_number(map: &BTreeMap<String, Value>, path: &str) -> Result<DataType, ProjectionError> {
    ensure_only_annotations(map, path, &["type", "format", "nullable"])?;
    let format = map
        .get("format")
        .map(|value| as_text(value, &field_path(path, "format")))
        .transpose()?;
    Ok(match format {
        None => DataType::Number,
        Some("float") => DataType::Float(FloatType::Binary32),
        Some("double") => DataType::Float(FloatType::Binary64),
        Some(other) => {
            return Err(ProjectionError::new(
                field_path(path, "format"),
                LossReason::UnsupportedSource,
                format!("number format `{other}` has no exact Rastok projection"),
            ));
        }
    })
}

fn project_object(
    map: &BTreeMap<String, Value>,
    path: &str,
    schemas: &BTreeMap<String, Value>,
    active: &mut BTreeSet<String>,
) -> Result<DataType, ProjectionError> {
    ensure_only_annotations(
        map,
        path,
        &[
            "type",
            "properties",
            "required",
            "additionalProperties",
            "nullable",
        ],
    )?;
    let properties = map
        .get("properties")
        .map(|value| as_object(value, &field_path(path, "properties")))
        .transpose()?
        .cloned()
        .unwrap_or_default();
    let required_path = field_path(path, "required");
    let required = map
        .get("required")
        .map(|value| as_array(value, &required_path))
        .transpose()?
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(index, value)| as_text(value, &index_path(&required_path, index)).map(String::from))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if properties.is_empty()
        && let Some(additional) = map.get("additionalProperties")
        && !matches!(additional, Value::Bool(false))
    {
        if !required.is_empty() {
            return Err(ProjectionError::new(
                required_path,
                LossReason::LossyProjection,
                "a pure Rastok map cannot preserve required property names",
            ));
        }
        let value = project_schema(
            additional,
            &field_path(path, "additionalProperties"),
            schemas,
            active,
        )?;
        return MapType::new(DataType::Text, value)
            .map(DataType::Map)
            .map_err(|error| type_error(path, error));
    }
    if !matches!(map.get("additionalProperties"), Some(Value::Bool(false))) {
        return Err(ProjectionError::new(
            field_path(path, "additionalProperties"),
            LossReason::LossyProjection,
            "a finite Rastok struct requires additionalProperties: false",
        ));
    }
    let property_names = properties.keys().cloned().collect::<BTreeSet<_>>();
    if required != property_names {
        return Err(ProjectionError::new(
            path,
            LossReason::LossyProjection,
            "optional object properties preserve absence, which Rastok V1 does not conflate with null",
        ));
    }
    let fields = properties
        .iter()
        .map(|(name, schema)| {
            let field_path = field_path(&field_path(path, "properties"), name);
            project_schema(schema, &field_path, schemas, active)
                .map(|ty| TypeField::new(name.clone(), ty))
        })
        .collect::<Result<Vec<_>, _>>()?;
    StructType::new(fields)
        .map(DataType::Struct)
        .map_err(|error| type_error(path, error))
}

fn nullable(
    ty: DataType,
    map: &BTreeMap<String, Value>,
    path: &str,
) -> Result<DataType, ProjectionError> {
    match map.get("nullable") {
        None | Some(Value::Bool(false)) => Ok(ty),
        Some(Value::Bool(true)) => ty.optional().map_err(|error| type_error(path, error)),
        Some(_) => Err(ProjectionError::new(
            field_path(path, "nullable"),
            LossReason::InvalidSchema,
            "nullable must be boolean",
        )),
    }
}

fn ensure_only_annotations(
    map: &BTreeMap<String, Value>,
    path: &str,
    assertions: &[&str],
) -> Result<(), ProjectionError> {
    const ANNOTATIONS: &[&str] = &[
        "default",
        "deprecated",
        "description",
        "example",
        "examples",
        "externalDocs",
        "title",
        "xml",
    ];
    if let Some(keyword) = map.keys().find(|keyword| {
        !assertions.contains(&keyword.as_str()) && !ANNOTATIONS.contains(&keyword.as_str())
    }) {
        return Err(ProjectionError::new(
            field_path(path, keyword),
            LossReason::UnsupportedSource,
            format!("OpenAPI schema keyword `{keyword}` has no exact Rastok projection"),
        ));
    }
    Ok(())
}

fn component_reference(reference: &str) -> Option<String> {
    let name = reference.strip_prefix("#/components/schemas/")?;
    if name.is_empty() || name.contains('/') {
        return None;
    }
    Some(name.replace("~1", "/").replace("~0", "~"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_components_project_without_weakening_absence_or_formats() {
        let document = r#"{
          "openapi":"3.1.0",
          "components":{"schemas":{
            "Payment":{"type":"object","additionalProperties":false,
              "required":["amount","content_type"],
              "properties":{
                "amount":{"type":"number"},
                "content_type":{"type":"string","format":"media-type"}
              }}
          }}
        }"#;
        let projected = project_components(document).expect("the exact profile projects");
        let DataType::Struct(payment) = &projected["Payment"] else {
            panic!("Payment is a struct")
        };
        assert_eq!(payment.fields()[0].name(), "amount");
        assert_eq!(payment.fields()[1].ty(), &DataType::MediaType);

        let optional = document.replace(
            "\"required\":[\"amount\",\"content_type\"]",
            "\"required\":[\"amount\"]",
        );
        assert_eq!(
            project_components(&optional)
                .expect_err("absence is not null")
                .reason(),
            LossReason::LossyProjection
        );

        let directional = document.replace(
            "\"amount\":{\"type\":\"number\"}",
            "\"amount\":{\"type\":\"number\",\"readOnly\":true}",
        );
        assert_eq!(
            project_components(&directional)
                .expect_err("one alias cannot erase request/response direction")
                .reason(),
            LossReason::UnsupportedSource
        );

        let required_map = r#"{
          "openapi":"3.1.0",
          "components":{"schemas":{"Labels":{"type":"object",
            "required":["trace"],
            "additionalProperties":{"type":"string"}}}}
        }"#;
        assert_eq!(
            project_components(required_map)
                .expect_err("a map cannot carry a required-name constraint")
                .reason(),
            LossReason::LossyProjection
        );

        let nullable = r#"{
          "openapi":"3.1.0",
          "components":{"schemas":{"MaybeId":{"type":["string","null"],"format":"uuid"}}}
        }"#;
        let projected = project_components(nullable).expect("3.1 nullable type arrays project");
        assert_eq!(
            projected["MaybeId"],
            DataType::Uuid
                .optional()
                .expect("uuid and null are disjoint")
        );
    }
}
