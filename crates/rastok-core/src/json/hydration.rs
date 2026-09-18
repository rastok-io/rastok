//! The declared codec schema, not object insertion order, owns materialization.

use alloc::boxed::Box;
use alloc::string::ToString;
use alloc::vec::Vec;

use crate::datatype::{ConformanceReason, ConformanceViolation, DataType, IntegerType};
use crate::value::{Value, ValueKind, ValuePathSegment};

/// Struct-member policy chosen by a validated codec, never by response contents.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HydrationProfile {
    /// Every declared member must be present, and no extra member is admitted.
    #[default]
    Strict,
    /// Omit extra struct members and normalize absent nullable members to null.
    /// Declared maps and variant subtrees retain their contents.
    Projected,
}

/// Whether the exact JSON hydration profile defines this entire schema.
/// No byte, temporal, float or discriminator convention is guessed.
#[must_use]
pub fn supports_type(ty: &DataType) -> bool {
    fn supported(ty: &DataType) -> bool {
        match ty {
            DataType::Null
            | DataType::Bool
            | DataType::Integer(_)
            | DataType::Decimal(_)
            | DataType::Number
            | DataType::Text
            | DataType::Enum(_)
            | DataType::Variant => true,
            DataType::Struct(record) => record.fields().iter().all(|field| supported(field.ty())),
            DataType::Array(array) => supported(array.element()),
            DataType::Map(map) => map.key() == &DataType::Text && supported(map.value()),
            DataType::StructuralUnion(union) => {
                union.branches().len() == 2
                    && union.branches().contains(&DataType::Null)
                    && union.branches().iter().all(supported)
            }
            DataType::Float(_)
            | DataType::MediaType
            | DataType::Bytes(_)
            | DataType::Temporal(_)
            | DataType::Uuid
            | DataType::TaggedUnion(_) => false,
        }
    }
    // Bound recursion even when called on a manually assembled, unvalidated type.
    ty.validate().is_ok() && supported(ty)
}

/// Hydrates an already parsed JSON value using one explicit codec schema.
/// Objects become canonical structs by member name; all declared fields,
/// including nullable ones, must be present. Decimals rescale exactly.
///
/// # Errors
/// A path-bearing conformance refusal whose Display omits payload values.
/// Unsupported schema encodings refuse rather than selecting a convention.
pub fn hydrate(ty: &DataType, value: Value) -> Result<Value, Box<ConformanceViolation>> {
    hydrate_with_profile(ty, value, HydrationProfile::Strict)
}

/// Hydrates parsed JSON under an explicit member policy. Both profiles produce
/// ordinary closed structs and reject incompatible present values.
///
/// # Errors
/// The same path-bearing, payload-free conformance refusals as [`hydrate`].
pub fn hydrate_with_profile(
    ty: &DataType,
    value: Value,
    profile: HydrationProfile,
) -> Result<Value, Box<ConformanceViolation>> {
    let refusal = |reason| {
        Box::new(ConformanceViolation {
            name: "data".to_string(),
            path: Vec::new(),
            expected: ty.clone(),
            found: ValueKind::of(&value),
            reason,
        })
    };
    if !supports_type(ty) {
        return Err(refusal(ConformanceReason::TypeMismatch));
    }
    if let Err(error) = value.validate() {
        return Err(refusal(ConformanceReason::MalformedValue(error.defect)));
    }
    hydrate_at(ty, value, profile)
}

fn at(
    result: Result<Value, Box<ConformanceViolation>>,
    segment: ValuePathSegment,
) -> Result<Value, Box<ConformanceViolation>> {
    result.map_err(|mut error| {
        error.path.insert(0, segment);
        error
    })
}

fn hydrate_at(
    ty: &DataType,
    value: Value,
    profile: HydrationProfile,
) -> Result<Value, Box<ConformanceViolation>> {
    let found = ValueKind::of(&value);
    let refusal = |reason| {
        Box::new(ConformanceViolation {
            name: "data".to_string(),
            path: Vec::new(),
            expected: ty.clone(),
            found,
            reason,
        })
    };
    let value = match (ty, value) {
        (DataType::Struct(record), Value::Map(mut members)) => {
            let mut fields = Vec::with_capacity(record.fields().len());
            for field in record.fields() {
                let value = match members.remove(field.name()) {
                    Some(value) => value,
                    None if profile == HydrationProfile::Projected
                        && DataType::Null.is_assignable_to(field.ty()) =>
                    {
                        Value::Null
                    }
                    None => {
                        return Err(refusal(ConformanceReason::MissingField(
                            field.name().to_string(),
                        )));
                    }
                };
                let value = at(
                    hydrate_at(field.ty(), value, profile),
                    ValuePathSegment::Field(field.name().to_string()),
                )?;
                fields.push((field.name().to_string(), value));
            }
            if profile == HydrationProfile::Strict
                && let Some(name) = members.keys().next()
            {
                return Err(refusal(ConformanceReason::UnexpectedField(name.clone())));
            }
            Value::Struct(fields)
        }
        (DataType::Array(array), Value::Array(elements)) => {
            let values = elements
                .into_iter()
                .enumerate()
                .map(|(index, value)| {
                    at(
                        hydrate_at(array.element(), value, profile),
                        ValuePathSegment::Index(index),
                    )
                })
                .collect::<Result<_, _>>()?;
            Value::Array(values)
        }
        (DataType::Map(map), Value::Map(members)) => {
            let values = members
                .into_iter()
                .map(|(name, value)| {
                    at(
                        hydrate_at(map.value(), value, profile),
                        ValuePathSegment::MapValue(name.clone()),
                    )
                    .map(|value| (name, value))
                })
                .collect::<Result<_, _>>()?;
            Value::Map(values)
        }
        (DataType::StructuralUnion(union), value) => {
            // This profile admits only T | null, so selection is unique before
            // conversion; there is no first-success coercion between branches.
            let is_null = matches!(value, Value::Null);
            let (index, branch) = union
                .branches()
                .iter()
                .enumerate()
                .find(|(_, branch)| (**branch == DataType::Null) == is_null)
                .ok_or_else(|| refusal(ConformanceReason::TypeMismatch))?;
            let value = hydrate_at(branch, value, profile)?;
            Value::StructuralUnion {
                branch: u16::try_from(index)
                    .map_err(|_| refusal(ConformanceReason::TypeMismatch))?,
                value: Box::new(value),
            }
        }
        (DataType::Enum(_), Value::Text(symbol)) => Value::Enum(symbol),
        (DataType::Integer(IntegerType::Unsigned(_)), Value::Int(number)) => Value::Uint(
            u64::try_from(number).map_err(|_| refusal(ConformanceReason::TypeMismatch))?,
        ),
        (DataType::Integer(IntegerType::Signed(_)), Value::Uint(number)) => {
            Value::Int(i64::try_from(number).map_err(|_| refusal(ConformanceReason::TypeMismatch))?)
        }
        (DataType::Integer(integer), Value::Decimal { unscaled, scale }) => {
            let exact = crate::datatype::rescale_decimal(unscaled, scale, 0)
                .ok_or_else(|| refusal(ConformanceReason::TypeMismatch))?;
            match integer {
                IntegerType::Signed(_) => Value::Int(
                    i64::try_from(exact).map_err(|_| refusal(ConformanceReason::TypeMismatch))?,
                ),
                IntegerType::Unsigned(_) => Value::Uint(
                    u64::try_from(exact).map_err(|_| refusal(ConformanceReason::TypeMismatch))?,
                ),
            }
        }
        (DataType::Decimal(_), value) => ty
            .narrow(value, None)
            .map_err(|_| refusal(ConformanceReason::TypeMismatch))?,
        (_, value) => value,
    };
    match ty.violation("data", &value) {
        Some(error) => Err(Box::new(error)),
        None => Ok(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datatype::{DecimalType, EnumType, StructType, TypeField};
    use alloc::string::String;
    use alloc::vec;

    fn record(fields: Vec<(&str, DataType)>) -> DataType {
        DataType::Struct(
            StructType::new(
                fields
                    .into_iter()
                    .map(|(name, ty)| TypeField::new(name, ty))
                    .collect(),
            )
            .unwrap(),
        )
    }

    fn response() -> DataType {
        record(vec![
            ("usage", record(vec![("tokens", DataType::UINT)])),
            ("cost", DataType::Decimal(DecimalType::new(38, 18).unwrap())),
        ])
    }

    fn read(ty: &DataType, text: &str) -> Result<Value, Box<ConformanceViolation>> {
        let (value, consumed) = crate::json::parse_value(text).unwrap();
        assert_eq!(consumed, text.len());
        hydrate(ty, value)
    }

    #[test]
    fn object_members_hydrate_by_name_with_exact_decimal_and_integer_values() {
        let a = read(&response(), r#"{"usage":{"tokens":120},"cost":0.0042}"#).unwrap();
        let b = read(&response(), r#"{"cost":42e-4,"usage":{"tokens":120}}"#).unwrap();
        assert_eq!(a, b);
        assert_eq!(
            a,
            Value::Struct(vec![
                (
                    String::from("cost"),
                    Value::Decimal {
                        unscaled: 4_200_000_000_000_000,
                        scale: 18
                    }
                ),
                (
                    String::from("usage"),
                    Value::Struct(vec![(String::from("tokens"), Value::Uint(120))])
                ),
            ])
        );
        assert!(response().accepts(&a));
    }

    #[test]
    fn arrays_maps_enums_and_explicit_null_use_the_same_recursive_rule() {
        let choice = DataType::Enum(EnumType::new(vec![String::from("ok")]).unwrap());
        let ty = record(vec![(
            "rows",
            DataType::array(DataType::text_map(choice.optional().unwrap())),
        )]);
        let value = read(&ty, r#"{"rows":[{"one":"ok","two":null}]}"#).unwrap();
        assert!(ty.accepts(&value));
        assert!(matches!(
            read(&ty, r#"{}"#).unwrap_err().reason,
            ConformanceReason::MissingField(_)
        ));
    }

    #[test]
    fn wrong_missing_and_extra_members_never_become_zero_or_partial_records() {
        for json in [
            r#"{"usage":{"tokens":-1},"cost":0}"#,
            r#"{"usage":{"tokens":1.5},"cost":0}"#,
            r#"{"usage":{},"cost":0}"#,
            r#"{"usage":{"tokens":1,"extra":2},"cost":0}"#,
        ] {
            let error = read(&response(), json).unwrap_err();
            assert_eq!(
                error.path.first(),
                Some(&ValuePathSegment::Field(String::from("usage")))
            );
        }
        let error = read(
            &response(),
            r#"{"usage":{"tokens":"PRIVATE-CONTENT"},"cost":0}"#,
        )
        .unwrap_err();
        assert_eq!(error.path.len(), 2);
        assert!(!error.to_string().contains("PRIVATE-CONTENT"));
    }

    #[test]
    fn projected_members_normalize_without_weakening_ordinary_structs() {
        let ty = record(vec![
            ("count", DataType::UINT.optional().unwrap()),
            ("required", DataType::Text),
        ]);
        let project = |json| {
            let (value, _) = crate::json::parse_value(json).unwrap();
            hydrate_with_profile(&ty, value, HydrationProfile::Projected)
        };
        let absent = project(r#"{"required":"ok","extra":"NOT-RETAINED"}"#).unwrap();
        assert_eq!(
            absent,
            project(r#"{"required":"ok","count":null}"#).unwrap()
        );
        assert!(ty.accepts(&absent));
        assert!(!alloc::format!("{absent:?}").contains("NOT-RETAINED"));
        assert_ne!(absent, project(r#"{"required":"ok","count":0}"#).unwrap());
        for json in [
            r#"{"count":1}"#,
            r#"{"required":null}"#,
            r#"{"required":"ok","count":"PRIVATE-CONTENT"}"#,
            r#"{"required":"ok","count":-1}"#,
            r#"{"required":"ok","count":1.5}"#,
        ] {
            let error = project(json).unwrap_err();
            assert!(!error.to_string().contains("PRIVATE-CONTENT"));
        }
        assert!(read(&ty, r#"{"required":"ok"}"#).is_err());
        assert!(read(&ty, r#"{"required":"ok","count":null,"extra":0}"#).is_err());
        assert!(
            ty.violation(
                "data",
                &Value::Struct(vec![("required".into(), Value::Text("ok".into()))])
            )
            .is_some()
        );
    }

    #[test]
    fn projection_recurses_into_structs_but_retains_map_keys_and_variant_subtrees() {
        let row = record(vec![("value", DataType::UINT.optional().unwrap())]);
        let ty = record(vec![
            ("rows", DataType::array(row.clone())),
            ("map", DataType::text_map(row)),
            ("raw", DataType::Variant),
        ]);
        let (value, _) = crate::json::parse_value(
            r#"{
          "rows":[{"extra":true}],"map":{"one":{},"two":{"value":5}},
          "raw":{"preserved":[1,null,{"more":true}]},"extra":0
        }"#,
        )
        .unwrap();
        let hydrated = hydrate_with_profile(&ty, value, HydrationProfile::Projected).unwrap();
        assert!(ty.accepts(&hydrated));
        let Value::Struct(fields) = hydrated else {
            panic!("struct")
        };
        let Value::Map(map) = &fields[0].1 else {
            panic!("map")
        };
        assert_eq!(map.len(), 2);
        assert!(map.contains_key("one") && map.contains_key("two"));
        assert!(alloc::format!("{:?}", fields[1].1).contains("preserved"));
        assert!(alloc::format!("{:?}", fields[1].1).contains("more"));
        assert!(!alloc::format!("{:?}", fields[2].1).contains("extra"));
    }

    #[test]
    fn decimals_never_round_and_unsigned_widths_remain_checked() {
        let cents = DataType::Decimal(DecimalType::new(4, 2).unwrap());
        assert!(read(&cents, "0.001").is_err());
        assert!(read(&cents, "100").is_err());
        assert_eq!(
            read(&cents, "12.30").unwrap(),
            Value::Decimal {
                unscaled: 1230,
                scale: 2
            }
        );
        assert_eq!(
            read(&DataType::UINT, "18446744073709551615").unwrap(),
            Value::Uint(u64::MAX)
        );
        assert!(read(&DataType::INT, "18446744073709551615").is_err());
    }

    #[test]
    fn unsupported_encodings_are_not_inferred_from_strings_or_numbers() {
        for ty in [
            DataType::BYTES,
            DataType::Uuid,
            DataType::FLOAT,
            DataType::MediaType,
        ] {
            assert!(!supports_type(&record(vec![("x", ty.clone())])));
            assert!(hydrate(&ty, Value::Text(String::from("guess"))).is_err());
        }
    }
}
