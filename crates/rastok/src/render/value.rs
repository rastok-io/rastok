//! Shared policy-safe value projection for terminal and debugger inspection.

use crate::data_policy::is_redacted_path;
use crate::envelope::{Data, Value};
use crate::node::NodeIndex;
use crate::plan::ValidatedExecutionPlan;
use crate::schema_annotation::{AnnotationTable, SchemaPath, SchemaPathSegment};
use serde_json::{Map, Value as JsonValue, json};

/// Renders an App output with the Lab's policy-safe JSON projection, except
/// text, which remains bare. Unmaterialized raw payloads keep their existing
/// byte-to-text rendering; classified encoded data is never decoded for display.
#[must_use]
pub fn render_data(data: &Data, annotations: Option<&AnnotationTable>) -> String {
    let empty = AnnotationTable::default();
    let annotations = annotations.unwrap_or(&empty);
    let redacted = crate::data_policy::requires_redaction(annotations);
    match data {
        Data::Materialized(Value::Text(text)) if !redacted => {
            return text.trim_end_matches('\n').to_string();
        }
        Data::Encoded { .. } if redacted => return json!({ "redacted": true }).to_string(),
        Data::Encoded {
            encoding: crate::envelope::Encoding::Raw,
            bytes,
            ..
        } if data.materialized_value().is_none() => {
            return String::from_utf8_lossy(bytes)
                .trim_end_matches('\n')
                .to_string();
        }
        _ => {}
    }
    let projected = data_json(data, annotations);
    projected.get("content").unwrap_or(&projected).to_string()
}

pub(crate) fn classified(plan: &ValidatedExecutionPlan) -> bool {
    let sensitive = |annotations: &AnnotationTable| {
        annotations
            .entries()
            .iter()
            .any(|(path, _)| is_redacted_path(annotations, path))
    };
    plan.resolved_ports()
        .boundary()
        .iter()
        .any(|port| sensitive(port.annotations()))
        || plan.nodes.iter().enumerate().any(|(index, node)| {
            plan.resolved_ports()
                .node(NodeIndex::new(index as u32))
                .is_some_and(|ports| ports.iter().any(|port| sensitive(port.annotations())))
                || node.vars().iter().any(|var| sensitive(&var.annotations))
        })
        || plan.nodes.iter().any(|owner| {
            owner.vals.iter().any(|value| sensitive(&value.annotations))
                || owner.ports.iter().any(|port| sensitive(&port.annotations))
        })
}

pub(crate) fn data_json(data: &Data, annotations: &AnnotationTable) -> JsonValue {
    let Some(value) = data.materialized_value() else {
        return match data {
            Data::Encoded {
                encoding, bytes, ..
            } => json!({
                "kind": "encoded",
                "encoding": encoding.to_string(),
                "byte_size": bytes.len(),
                "materialized": false,
            }),
            Data::Materialized(_) => unreachable!("materialized data exposes its value"),
        };
    };
    materialized_value_json(value, annotations)
}

pub(crate) fn materialized_value_json(value: &Value, annotations: &AnnotationTable) -> JsonValue {
    json!({
        "kind": crate::value::ValueKind::of(value).to_string(),
        "materialized": true,
        "content": value_json(value, annotations, &SchemaPath::root(), !annotations.is_empty()),
    })
}

fn value_json(
    value: &Value,
    annotations: &AnnotationTable,
    path: &SchemaPath,
    classified: bool,
) -> JsonValue {
    if classified && is_redacted_path(annotations, path) {
        return json!({ "redacted": true });
    }
    match value {
        Value::Bool(value) => JsonValue::Bool(*value),
        Value::Int(value) => json_integer(i128::from(*value), "int"),
        Value::Uint(value) => json_unsigned(u128::from(*value), "uint"),
        Value::Double(value) if value.is_finite() => JsonValue::from(*value),
        Value::Double(value) => json!({ "kind": "double", "exact": value.to_string() }),
        Value::Text(value) | Value::Enum(value) => JsonValue::String(value.clone()),
        Value::Bytes(value) => json!({ "kind": "bytes", "hex": hex(value) }),
        Value::Duration(value) => json!({ "kind": "duration", "nanoseconds": value.to_string() }),
        Value::Timestamp(value) => {
            json!({ "kind": "timestamp", "seconds_since_epoch": value.seconds().to_string(), "nanoseconds": value.nanoseconds() })
        }
        Value::Date(value) => json!({ "kind": "date", "days_since_epoch": value.to_string() }),
        Value::Time(value) => {
            json!({ "kind": "time", "nanoseconds_since_midnight": value.to_string() })
        }
        Value::TimestampNtz(value) => {
            json!({ "kind": "timestamp_ntz", "seconds_since_epoch": value.seconds().to_string(), "nanoseconds": value.nanoseconds() })
        }
        Value::Interval {
            months,
            days,
            nanoseconds,
        } => json!({
            "kind": "interval",
            "months": months,
            "days": days,
            "nanoseconds": nanoseconds.to_string(),
        }),
        Value::Uuid(value) => json!({ "kind": "uuid", "hex": hex(value) }),
        Value::Null => JsonValue::Null,
        Value::Array(items) => {
            let child = path.child(SchemaPathSegment::ArrayElement);
            JsonValue::Array(
                items
                    .iter()
                    .map(|item| value_json(item, annotations, &child, classified))
                    .collect(),
            )
        }
        Value::Map(entries) => {
            let key_path = path.child(SchemaPathSegment::MapKey);
            let value_path = path.child(SchemaPathSegment::MapValue);
            JsonValue::Array(
                entries
                    .iter()
                    .map(|(key, value)| {
                        json!({
                            "key": if classified && is_redacted_path(annotations, &key_path) {
                                JsonValue::String(String::from("[REDACTED]"))
                            } else {
                                JsonValue::String(key.clone())
                            },
                            "value": value_json(value, annotations, &value_path, classified),
                        })
                    })
                    .collect(),
            )
        }
        Value::KeyedMap(entries) => {
            let key_path = path.child(SchemaPathSegment::MapKey);
            let value_path = path.child(SchemaPathSegment::MapValue);
            JsonValue::Array(
                entries
                    .iter()
                    .map(|(key, value)| {
                        json!({
                            "key": value_json(key, annotations, &key_path, classified),
                            "value": value_json(value, annotations, &value_path, classified),
                        })
                    })
                    .collect(),
            )
        }
        Value::Struct(fields) => JsonValue::Object(
            fields
                .iter()
                .map(|(name, value)| {
                    let child = path.child(SchemaPathSegment::Field(name.clone()));
                    (
                        name.clone(),
                        value_json(value, annotations, &child, classified),
                    )
                })
                .collect(),
        ),
        Value::Decimal { unscaled, scale } => json!({
            "kind": "decimal",
            "unscaled": unscaled.to_string(),
            "scale": scale,
        }),
        Value::StructuralUnion { branch, value } => {
            let child = path.child(SchemaPathSegment::StructuralUnionBranch(*branch));
            json!({
                "kind": "structural-union",
                "branch": branch,
                "value": value_json(value, annotations, &child, classified),
            })
        }
        Value::TaggedUnion(tagged) => {
            let case = path.child(SchemaPathSegment::TaggedUnionCase(tagged.case.clone()));
            let payload = tagged
                .payload
                .iter()
                .map(|(name, value)| {
                    let child = case.child(SchemaPathSegment::Field(name.clone()));
                    (
                        name.clone(),
                        value_json(value, annotations, &child, classified),
                    )
                })
                .collect::<Map<_, _>>();
            json!({
                "kind": "tagged-union",
                "case": tagged.case,
                "payload": payload,
            })
        }
    }
}

pub(crate) fn json_integer(value: i128, kind: &'static str) -> JsonValue {
    const JS_EXACT_MAX: i128 = 9_007_199_254_740_991;
    if (-JS_EXACT_MAX..=JS_EXACT_MAX).contains(&value) {
        JsonValue::from(i64::try_from(value).unwrap_or_default())
    } else {
        json!({ "kind": kind, "exact": value.to_string() })
    }
}

fn json_unsigned(value: u128, kind: &'static str) -> JsonValue {
    const JS_EXACT_MAX: u128 = 9_007_199_254_740_991;
    if value <= JS_EXACT_MAX {
        JsonValue::from(u64::try_from(value).unwrap_or_default())
    } else {
        json!({ "kind": kind, "exact": value.to_string() })
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut value = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        let _ = write!(value, "{byte:02x}");
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::Encoding;
    use crate::schema_annotation::{AnnotationSet, DataAnnotation};

    #[test]
    fn terminal_values_share_the_lab_projection_without_debug_shapes() {
        let annotations = AnnotationTable::default();
        for value in [
            Value::Int(5),
            Value::Int(i64::MAX),
            Value::Uint(u64::MAX),
            Value::Bool(true),
            Value::Null,
            Value::Enum("Ready".into()),
            Value::Decimal {
                unscaled: 125,
                scale: 2,
            },
            Value::Bytes(vec![0, 255]),
            Value::Array(vec![Value::Text("hello".into())]),
            Value::Map(std::collections::BTreeMap::from([(
                "key".into(),
                Value::Int(1),
            )])),
            Value::Struct(vec![("size".into(), Value::Int(5))]),
        ] {
            let data = Data::materialized(value);
            let projected = data_json(&data, &annotations);
            assert_eq!(
                serde_json::from_str::<JsonValue>(&render_data(&data, None)).unwrap(),
                projected["content"],
            );
        }
    }

    #[test]
    fn terminal_text_and_raw_payloads_keep_their_line_rendering() {
        assert_eq!(
            render_data(&Data::materialized(Value::Text("hello\n".into())), None),
            "hello"
        );
        let raw = Data::encoded(Encoding::Raw, b"hello\n".to_vec());
        assert_eq!(render_data(&raw, None), "hello");
        assert!(raw.materialized_value().is_none());
    }

    #[test]
    fn terminal_redaction_precedes_text_or_encoded_rendering() {
        let annotations = AnnotationTable::new(vec![(
            SchemaPath::root(),
            AnnotationSet::new(vec![DataAnnotation::Secret]).unwrap(),
        )])
        .unwrap();
        for data in [
            Data::materialized(Value::Text("private fixture".into())),
            Data::encoded(Encoding::Raw, b"private fixture".to_vec()),
        ] {
            assert_eq!(
                render_data(&data, Some(&annotations)),
                "{\"redacted\":true}"
            );
        }
    }
}
