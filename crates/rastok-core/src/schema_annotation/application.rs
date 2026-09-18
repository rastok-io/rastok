//! Annotation applications retain ordinary types and values beside explicit consumer authority.

use alloc::string::{String, ToString};
use alloc::vec;

use crate::datatype::{DataType, StructType, TypeError, TypeField};
use crate::value::Value;

use super::SchemaAnnotationError;

/// Maximum byte length of one canonical declaring identity in a package.
pub const MAX_ANNOTATION_DEFINITION_BYTES: usize = 255;

/// Consumer selected by a validated definition, never by its field shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotationBinding {
    /// Descriptive metadata for tools; no admission, redaction or accounting power.
    Metadata,
    /// Exact business quantities, retained as individual typed observations.
    Measure,
}

/// One constant application of an annotation-capable flat struct.
///
/// Definition names are nominal here, although the argument schema remains
/// structural for ordinary data. Aliases retain their declaring definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationApplication {
    binding: AnnotationBinding,
    definition: String,
    argument_type: DataType,
    arguments: Value,
}

impl AnnotationApplication {
    /// Validates definition authority, flatness and complete typed arguments.
    ///
    /// Omission must already be resolved by the source checker. A package
    /// cannot manufacture a default by leaving a field absent.
    ///
    /// # Errors
    /// Refuses invalid or protected names, non-flat argument schemas and
    /// values which do not inhabit the complete declared struct.
    pub fn new(
        binding: AnnotationBinding,
        definition: String,
        argument_type: DataType,
        arguments: Value,
    ) -> Result<Self, SchemaAnnotationError> {
        let application = Self {
            binding,
            definition,
            argument_type,
            arguments,
        };
        application.validate()?;
        Ok(application)
    }

    /// Interpretation explicitly authorized by this definition.
    #[must_use]
    pub const fn binding(&self) -> AnnotationBinding {
        self.binding
    }

    /// Declaring name, not the spelling of an alias at an application site.
    #[must_use]
    pub fn definition(&self) -> &str {
        &self.definition
    }

    /// Ordinary closed struct schema of the annotation's arguments.
    #[must_use]
    pub const fn argument_type(&self) -> &DataType {
        &self.argument_type
    }

    /// Complete ordinary struct value, including explicitly resolved nulls.
    #[must_use]
    pub const fn arguments(&self) -> &Value {
        &self.arguments
    }

    pub(super) fn validate(&self) -> Result<(), SchemaAnnotationError> {
        match self.binding {
            AnnotationBinding::Metadata => {
                let name = self.definition.as_str();
                if name.is_empty()
                    || name.len() > MAX_ANNOTATION_DEFINITION_BYTES
                    || !name.split('.').all(|segment| {
                        let mut bytes = segment.bytes();
                        bytes
                            .next()
                            .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
                            && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                    })
                {
                    return Err(self.invalid("invalid declaring name"));
                }
                if name.split('.').any(|part| {
                    matches!(
                        part,
                        "sensitive" | "secret" | "personal" | "measure" | "cost"
                    )
                }) {
                    return Err(self.invalid("a descriptive definition cannot claim a protected or retired annotation name"));
                }
            }
            AnnotationBinding::Measure => {
                if self.definition != "rastok/measure@3"
                    || !is_measure_argument_type(&self.argument_type)
                {
                    return Err(self.invalid(
                        "a measure requires its protected definition and exact argument schema",
                    ));
                }
            }
        }
        self.argument_type
            .validate()
            .map_err(|_| self.invalid("invalid argument schema"))?;
        let DataType::Struct(schema) = &self.argument_type else {
            return Err(self.invalid("annotation arguments require a flat closed struct"));
        };
        if !schema.fields().iter().all(|field| flat_field(field.ty())) {
            return Err(self.invalid("annotation fields must be scalar, enum, or nullable scalar"));
        }
        self.arguments
            .validate()
            .map_err(|_| self.invalid("invalid argument value"))?;
        if !self.argument_type.accepts(&self.arguments) {
            return Err(self.invalid("arguments do not conform to the complete declared struct"));
        }
        if self.binding == AnnotationBinding::Measure {
            let Some(id) = self.measure_id() else {
                return Err(self.invalid("a measure requires an id"));
            };
            super::MeasureId::new(id)?;
            let Some((_, unit)) = self.quantity_measure() else {
                return Err(self.invalid("a measure requires an explicit unit"));
            };
            if unit.is_empty()
                || unit.len() > super::MAX_MEASURE_ID_BYTES
                || unit.trim() != unit
                || unit.chars().any(char::is_control)
            {
                return Err(self.invalid("unit must be nonempty, bounded text without surrounding whitespace or control characters"));
            }
        }
        Ok(())
    }

    /// Id and explicit unit of a numerical quantity.
    #[must_use]
    pub fn quantity_measure(&self) -> Option<(&str, &str)> {
        let id = self.measure_id()?;
        let Some(Value::Text(unit)) = self.argument("unit") else {
            return None;
        };
        Some((id, unit))
    }

    /// Author-chosen identity, with no meaning inferred from its spelling.
    #[must_use]
    pub fn measure_id(&self) -> Option<&str> {
        if self.binding != AnnotationBinding::Measure {
            return None;
        }
        match self.argument("id")? {
            Value::Text(id) => Some(id),
            _ => None,
        }
    }

    fn argument(&self, name: &str) -> Option<&Value> {
        let Value::Struct(fields) = &self.arguments else {
            return None;
        };
        let (_, value) = fields.iter().find(|(field, _)| field == name)?;
        Some(match value {
            Value::StructuralUnion { value, .. } => value,
            value => value,
        })
    }

    fn invalid(&self, detail: &'static str) -> SchemaAnnotationError {
        SchemaAnnotationError::InvalidApplication {
            definition: self.definition.to_string(),
            detail,
        }
    }
}

fn is_measure_argument_type(ty: &DataType) -> bool {
    let DataType::Struct(schema) = ty else {
        return false;
    };
    if schema.fields().len() != 2 {
        return false;
    }
    schema
        .fields()
        .iter()
        .all(|field| matches!(field.name(), "id" | "unit") && field.ty() == &DataType::Text)
}

/// Standard flat signature constructed by the checker and checked independently in core.
///
/// # Errors
/// Reports an invalid logical schema instead of manufacturing an unchecked type.
pub fn measure_argument_type() -> Result<DataType, TypeError> {
    Ok(DataType::Struct(StructType::new(vec![
        TypeField::new("id", DataType::Text),
        TypeField::new("unit", DataType::Text),
    ])?))
}

/// The same logical field domain is used by source and independent validation.
#[must_use]
pub fn flat_field(ty: &DataType) -> bool {
    match ty {
        DataType::Null
        | DataType::Bool
        | DataType::Integer(_)
        | DataType::Float(_)
        | DataType::Decimal(_)
        | DataType::Number
        | DataType::Text
        | DataType::MediaType
        | DataType::Bytes(_)
        | DataType::Temporal(_)
        | DataType::Uuid
        | DataType::Enum(_) => true,
        DataType::StructuralUnion(union) => {
            union.branches().len() == 2
                && union.branches().contains(&DataType::Null)
                && union.branches().iter().all(flat_field)
        }
        DataType::Struct(_)
        | DataType::Map(_)
        | DataType::Array(_)
        | DataType::TaggedUnion(_)
        | DataType::Variant => false,
    }
}
