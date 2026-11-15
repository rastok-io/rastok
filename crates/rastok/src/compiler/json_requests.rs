//! JSON request constraints use resolved producer types, not the codec's
//! dynamic input pin. No source type is guessed from a runtime value.

use crate::datatype::{DataType, TemporalType};
use crate::diagnostics::{Bag, Diagnostic, Phase};
use crate::plan::{AttrSetting, ValidatedExecutionPlan};

pub(super) fn check(plan: &ValidatedExecutionPlan, bag: &mut Bag) {
    use crate::connector::stdlib::{
        HTTP_CLIENT_CONNECTOR_ID, HTTP_SERVER_CONNECTOR_ID, PROCESS_CONNECTOR_ID,
        WEBSOCKET_CLIENT_CONNECTOR_ID, WEBSOCKET_SERVER_CONNECTOR_ID,
    };

    // Composed edges preserve the actual producer through passive Graph
    // boundaries. Port inference has closed before this check runs.
    for edge in plan.edges() {
        let Some(target) = &edge.to else { continue };
        let node = &plan.nodes[target.node.index()];
        let Some(connector) = node.connector() else {
            continue;
        };
        if target.port.name() != "in"
            || !matches!(
                connector.contract.as_str(),
                HTTP_CLIENT_CONNECTOR_ID
                    | HTTP_SERVER_CONNECTOR_ID
                    | PROCESS_CONNECTOR_ID
                    | WEBSOCKET_CLIENT_CONNECTOR_ID
                    | WEBSOCKET_SERVER_CONNECTOR_ID
            )
            || !matches!(connector.attribute("codec"), Some(AttrSetting::Word(codec))
                if matches!(codec.as_str(), "json" | "jsonl" | "ndjson"))
        {
            continue;
        }
        let Some(source) = plan.resolved_ports().endpoint_type(&edge.from) else {
            continue;
        };
        let Some((path, ty)) = first_non_json_type(source, "") else {
            continue;
        };
        let hint = match ty {
            DataType::TaggedUnion(_) => {
                "a tagged union needs an explicit JSON discriminator projection"
            }
            DataType::Temporal(_) | DataType::Uuid => {
                "temporal and UUID values need an explicit conversion before JSON encoding"
            }
            DataType::Bytes(_) => "bytes need an explicit conversion before JSON encoding",
            DataType::Map(_) => "JSON objects require text keys",
            _ => continue,
        };
        let location = if path.is_empty() {
            "the payload".to_owned()
        } else {
            format!("field `{path}`")
        };
        let type_name = match ty {
            DataType::Map(_) => "map with non-text keys".to_owned(),
            DataType::Temporal(temporal) => match temporal {
                TemporalType::Date(_) => "date",
                TemporalType::Time(_) => "time",
                TemporalType::Timestamp { .. } => "timestamp",
                TemporalType::TimestampNtz(_) => "timestamp_ntz",
                TemporalType::Duration(_) => "duration",
                TemporalType::Interval(_) => "interval",
            }
            .to_owned(),
            _ => ty.to_string(),
        };
        let detail = format!(
            "`{}.{}` receives a value with no JSON form from `{}.{}`: {location} has type {type_name}; {hint}",
            node.path,
            target.port,
            plan.nodes[edge.from.node.index()].path,
            edge.from.port
        );
        bag.push(
            Diagnostic::new(super::check::E_TYPE_INVALID, Phase::Check, detail.clone())
                .with_field("detail", detail),
        );
    }
}

/// The writer emits finite floats, decimals, enum symbols and structural
/// unions. Its response-deserialization profile is narrower and cannot be
/// reused here. Variant leaves stay subject to the runtime encoder.
fn first_non_json_type<'a>(ty: &'a DataType, path: &str) -> Option<(String, &'a DataType)> {
    match ty {
        DataType::Bytes(_) | DataType::Temporal(_) | DataType::Uuid | DataType::TaggedUnion(_) => {
            Some((path.to_owned(), ty))
        }
        DataType::Map(map) if map.key() != &DataType::Text => Some((path.to_owned(), ty)),
        DataType::Map(map) => first_non_json_type(map.value(), &format!("{path}[]")),
        DataType::Array(array) => first_non_json_type(array.element(), &format!("{path}[]")),
        DataType::Struct(structure) => structure.fields().iter().find_map(|field| {
            let field_path = if path.is_empty() {
                field.name().to_owned()
            } else {
                format!("{path}.{}", field.name())
            };
            first_non_json_type(field.ty(), &field_path)
        }),
        DataType::StructuralUnion(union) => union
            .branches()
            .iter()
            .find_map(|branch| first_non_json_type(branch, path)),
        DataType::Null
        | DataType::Bool
        | DataType::Integer(_)
        | DataType::Float(_)
        | DataType::Decimal(_)
        | DataType::Number
        | DataType::Text
        | DataType::MediaType
        | DataType::Enum(_)
        | DataType::Variant => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datatype::{IntervalUnit, StructType, TypeField};

    #[test]
    fn intervals_refuse_as_fields_and_array_elements_without_source_syntax() {
        for unit in [
            IntervalUnit::YearMonth,
            IntervalUnit::DayTime,
            IntervalUnit::MonthDayNano,
            IntervalUnit::MonthDayMillisUnsigned,
        ] {
            let interval = DataType::Temporal(TemporalType::Interval(unit));
            for (field_type, path) in [
                (interval.clone(), "at"),
                (DataType::array(interval.clone()), "at[]"),
            ] {
                let record = DataType::Struct(
                    StructType::new(vec![TypeField::new("at", field_type)])
                        .expect("one field has a valid name"),
                );
                assert_eq!(
                    first_non_json_type(&record, ""),
                    Some((path.to_owned(), &interval)),
                    "{unit:?}",
                );
            }
        }
    }
}
