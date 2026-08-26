//! Strict boundary projections between external schema vocabularies and the
//! one Rastok [`DataType`](crate::datatype::DataType) algebra.
//!
//! The adapters in this module deliberately own no runtime semantics. They
//! parse or receive an external schema, construct a canonical logical type,
//! and either reverse that type without loss or return a typed, located
//! refusal. External ordering and physical union selectors are returned as
//! adapter mappings and never enter [`DataType`](crate::datatype::DataType).

use std::collections::BTreeMap;

use crate::datatype::TypeError;
use crate::value::Value;

pub mod arrow;
pub mod avro;
pub mod json_schema;
pub mod openapi;

/// Stable reason why an external schema cannot cross a projection boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LossReason {
    /// The source construct is outside the admitted external profile.
    UnsupportedSource,
    /// The target ecosystem has no admitted exact representation.
    UnsupportedTarget,
    /// The external document violates its own structural rules.
    InvalidSchema,
    /// Resolving a named schema would revisit an active definition.
    RecursiveSchema,
    /// A mapping would erase a logical distinction.
    LossyProjection,
    /// Multiple union branches could accept the same logical value.
    AmbiguousUnion,
    /// A map key is outside Rastok's canonical-key domain.
    InvalidMapKey,
    /// The projected type exceeds a canonical structural budget.
    BudgetExceeded,
}

impl std::fmt::Display for LossReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::UnsupportedSource => "unsupported source",
            Self::UnsupportedTarget => "unsupported target",
            Self::InvalidSchema => "invalid schema",
            Self::RecursiveSchema => "recursive schema",
            Self::LossyProjection => "lossy projection",
            Self::AmbiguousUnion => "ambiguous union",
            Self::InvalidMapKey => "invalid map key",
            Self::BudgetExceeded => "schema budget exceeded",
        })
    }
}

/// A typed projection refusal located in the external or logical tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionError {
    path: String,
    reason: LossReason,
    detail: String,
}

impl ProjectionError {
    pub(crate) fn new(
        path: impl Into<String>,
        reason: LossReason,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            reason,
            detail: detail.into(),
        }
    }

    /// JSONPath-like location of the first lost or invalid construct.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Stable refusal category.
    #[must_use]
    pub const fn reason(&self) -> LossReason {
        self.reason
    }

    /// Human-readable explanation; callers key on [`Self::reason`].
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl std::fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {}: {}", self.reason, self.path, self.detail)
    }
}

impl std::error::Error for ProjectionError {}

/// Logical branch selected by one external union selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnionSelection {
    /// Canonical index in a structural union.
    Structural(u16),
    /// Canonical name in a tagged union.
    Tagged(String),
}

pub(crate) fn field_path(path: &str, field: &str) -> String {
    if field
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        format!("{path}.{field}")
    } else {
        format!("{path}[{field:?}]")
    }
}

pub(crate) fn index_path(path: &str, index: usize) -> String {
    format!("{path}[{index}]")
}

pub(crate) fn type_error(path: &str, error: TypeError) -> ProjectionError {
    let reason = match error {
        TypeError::InvalidMapKey(_) => LossReason::InvalidMapKey,
        TypeError::DuplicateUnionBranch(_) | TypeError::OverlappingUnionBranches { .. } => {
            LossReason::AmbiguousUnion
        }
        TypeError::DecimalPrecision { precision } if precision > 0 => LossReason::UnsupportedSource,
        TypeError::DecimalScale { .. } => LossReason::UnsupportedSource,
        TypeError::ByteLength { length } if length > 0 => LossReason::BudgetExceeded,
        TypeError::ArrayLength { .. }
        | TypeError::MemberBudget { .. }
        | TypeError::NodeBudget { .. }
        | TypeError::NestingBudget { .. } => LossReason::BudgetExceeded,
        _ => LossReason::InvalidSchema,
    };
    ProjectionError::new(path, reason, error.to_string())
}

pub(crate) fn parse_json_document(text: &str) -> Result<Value, ProjectionError> {
    let (value, consumed) = rastok_core::json::parse_value(text).map_err(|error| {
        ProjectionError::new(
            format!("$@{}", error.offset),
            LossReason::InvalidSchema,
            error.message,
        )
    })?;
    if !text.as_bytes()[consumed..]
        .iter()
        .all(|byte| byte.is_ascii_whitespace())
    {
        return Err(ProjectionError::new(
            format!("$@{consumed}"),
            LossReason::InvalidSchema,
            "trailing content after the schema document",
        ));
    }
    Ok(value)
}

pub(crate) fn render_json_document(value: &Value) -> Result<String, ProjectionError> {
    rastok_core::json::write_value(value)
        .map_err(|error| ProjectionError::new("$", LossReason::UnsupportedTarget, error.message))
}

pub(crate) fn object(entries: impl IntoIterator<Item = (String, Value)>) -> Value {
    Value::Map(entries.into_iter().collect::<BTreeMap<_, _>>())
}

pub(crate) fn text(value: impl Into<String>) -> Value {
    Value::Text(value.into())
}

pub(crate) fn as_object<'a>(
    value: &'a Value,
    path: &str,
) -> Result<&'a BTreeMap<String, Value>, ProjectionError> {
    match value {
        Value::Map(map) => Ok(map),
        _ => Err(ProjectionError::new(
            path,
            LossReason::InvalidSchema,
            "expected an object",
        )),
    }
}

pub(crate) fn as_array<'a>(value: &'a Value, path: &str) -> Result<&'a [Value], ProjectionError> {
    match value {
        Value::Array(items) => Ok(items),
        _ => Err(ProjectionError::new(
            path,
            LossReason::InvalidSchema,
            "expected an array",
        )),
    }
}

pub(crate) fn as_text<'a>(value: &'a Value, path: &str) -> Result<&'a str, ProjectionError> {
    match value {
        Value::Text(text) => Ok(text),
        _ => Err(ProjectionError::new(
            path,
            LossReason::InvalidSchema,
            "expected text",
        )),
    }
}

pub(crate) fn as_u32(value: &Value, path: &str) -> Result<u32, ProjectionError> {
    match value {
        Value::Int(value) => u32::try_from(*value),
        Value::Uint(value) => u32::try_from(*value),
        _ => {
            return Err(ProjectionError::new(
                path,
                LossReason::InvalidSchema,
                "expected a non-negative integer",
            ));
        }
    }
    .map_err(|_| {
        ProjectionError::new(
            path,
            LossReason::InvalidSchema,
            "integer is outside the supported u32 range",
        )
    })
}

pub(crate) fn exact_integer(value: &Value, path: &str) -> Result<i128, ProjectionError> {
    match value {
        Value::Int(value) => Ok(i128::from(*value)),
        Value::Uint(value) => Ok(i128::from(*value)),
        Value::Decimal { unscaled, scale } if *scale <= 0 => {
            let factor = 10_i128.checked_pow(scale.unsigned_abs()).ok_or_else(|| {
                ProjectionError::new(path, LossReason::InvalidSchema, "integer bound overflows")
            })?;
            unscaled.checked_mul(factor).ok_or_else(|| {
                ProjectionError::new(path, LossReason::InvalidSchema, "integer bound overflows")
            })
        }
        _ => Err(ProjectionError::new(
            path,
            LossReason::InvalidSchema,
            "expected an exact integer",
        )),
    }
}

pub(crate) fn integer_value(value: i128) -> Result<Value, ProjectionError> {
    if let Ok(value) = i64::try_from(value) {
        Ok(Value::Int(value))
    } else if value >= 0 {
        Ok(Value::Decimal {
            unscaled: value,
            scale: 0,
        })
    } else {
        Err(ProjectionError::new(
            "$",
            LossReason::UnsupportedTarget,
            "integer bound is outside the JSON exact-number domain",
        ))
    }
}
