//! Static propagation and safe presentation of schema annotations.
//!
//! This module owns no logical typing rule. It consumes already resolved
//! [`DataType`]s and the sparse tables beside them, then derives the policy of
//! expression results. The plan validator runs the same functions as the
//! source compiler; execution never reconstructs policy from field names.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::datatype::DataType;
use crate::expr::{self, Expr, NarrowTarget, Signature};
use crate::schema_annotation::{
    AnnotationKind, AnnotationSet, AnnotationTable, DataAnnotation, SchemaAnnotationError,
    SchemaPath, SchemaPathSegment,
};
use crate::value::Value;

/// One resolved value visible while deriving an expression's policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Binding {
    ty: DataType,
    annotations: AnnotationTable,
}

impl Binding {
    #[must_use]
    pub(crate) const fn new(ty: DataType, annotations: AnnotationTable) -> Self {
        Self { ty, annotations }
    }

    #[must_use]
    pub(crate) const fn ty(&self) -> &DataType {
        &self.ty
    }

    #[must_use]
    pub(crate) const fn annotations(&self) -> &AnnotationTable {
        &self.annotations
    }
}

/// Closed immutable environment used by policy derivation.
#[derive(Debug, Clone, Default)]
pub(crate) struct Environment {
    bindings: BTreeMap<String, Binding>,
}

impl Environment {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub(crate) fn with(
        mut self,
        name: impl Into<String>,
        ty: DataType,
        annotations: AnnotationTable,
    ) -> Self {
        self.bindings
            .insert(name.into(), Binding::new(ty, annotations));
        self
    }

    pub(crate) fn insert(
        &mut self,
        name: impl Into<String>,
        ty: DataType,
        annotations: AnnotationTable,
    ) {
        self.bindings
            .insert(name.into(), Binding::new(ty, annotations));
    }

    pub(crate) fn bindings(&self) -> impl Iterator<Item = (&String, &Binding)> {
        self.bindings.iter()
    }

    fn signature(&self) -> Signature {
        self.bindings
            .iter()
            .fold(Signature::new(), |signature, (name, binding)| {
                if name == "meta" || name.ends_with(".meta") {
                    signature.with_metadata(name.clone())
                } else {
                    signature.with(name.clone(), binding.ty.clone())
                }
            })
    }

    fn resolve_path(
        &self,
        path: &[String],
        result_ty: &DataType,
    ) -> Result<Binding, PropagationError> {
        let Some((prefix, name, binding)) = (1..=path.len()).rev().find_map(|length| {
            let name = path[..length].join(".");
            self.bindings
                .get(&name)
                .map(|binding| (length, name, binding))
        }) else {
            return Err(PropagationError::UnknownBinding(path.join(".")));
        };
        if name == "meta" || name.ends_with(".meta") {
            return Ok(Binding::new(result_ty.clone(), AnnotationTable::empty()));
        }
        project_members(binding, &path[prefix..])
    }
}

/// A policy claim cannot be derived from the canonical operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropagationError {
    /// An expression path has no policy/type binding.
    UnknownBinding(String),
    /// A member path cannot be projected through its resolved type.
    InvalidProjection(String),
    /// Transported expression typing could not be independently repeated.
    UnresolvedExpression,
    /// Alternative values disagree about one exact accounting meaning.
    ConflictingMeaning {
        /// Canonical result path.
        path: SchemaPath,
        /// Measure or cost.
        kind: AnnotationKind,
    },
    /// A newly derived table violated the adjacent schema contract.
    InvalidDerivedTable(SchemaAnnotationError),
}

impl fmt::Display for PropagationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownBinding(name) => write!(f, "policy binding `{name}` is unavailable"),
            Self::InvalidProjection(path) => {
                write!(f, "policy projection `{path}` does not exist in its schema")
            }
            Self::UnresolvedExpression => {
                f.write_str("the expression result cannot be re-derived for policy")
            }
            Self::ConflictingMeaning { path, kind } => write!(
                f,
                "alternative values disagree about {kind:?} meaning at {path}"
            ),
            Self::InvalidDerivedTable(reason) => {
                write!(f, "the derived annotation table is invalid: {reason}")
            }
        }
    }
}

impl core::error::Error for PropagationError {}

#[derive(Debug, Clone)]
struct Derived {
    ty: DataType,
    annotations: AnnotationTable,
}

/// Derives the annotation table of one already typed expression.
///
/// Exact projections and constructors preserve their meanings. Operators and
/// registered functions are calculations: they retain only the strongest
/// security taint of anything they inspect. No calculation manufactures a
/// measure or cost identity.
pub(crate) fn expression(
    tree: &expr::Tree,
    environment: &Environment,
) -> Result<AnnotationTable, PropagationError> {
    let signature = environment.signature();
    let derived = derive(&tree.root, environment, &signature)?;
    Ok(derived.annotations)
}

fn derive(
    expression: &Expr,
    environment: &Environment,
    signature: &Signature,
) -> Result<Derived, PropagationError> {
    // The plan's ordinary expression validator remains the type authority.
    // Dynamic `variant` access may have no more precise recursive type, but
    // policy still has one safe answer: treat the access as a calculation and
    // retain only its security taint.
    let ty = expr::rederive_type(expression, signature).unwrap_or(DataType::Variant);
    let annotations = match expression {
        Expr::Literal(_) | Expr::Asset(_) => AnnotationTable::empty(),
        Expr::Path(path) => {
            let binding = environment.resolve_path(path, &ty)?;
            return Ok(Derived {
                ty: binding.ty,
                annotations: binding.annotations,
            });
        }
        Expr::Unary { expr, .. } | Expr::Is { expr, .. } => {
            calculation([derive(expr, environment, signature)?.annotations])?
        }
        Expr::Binary { left, right, .. } => calculation([
            derive(left, environment, signature)?.annotations,
            derive(right, environment, signature)?.annotations,
        ])?,
        Expr::Call { args, .. } => {
            let inputs = args
                .iter()
                .map(|argument| {
                    derive(argument, environment, signature).map(|item| item.annotations)
                })
                .collect::<Result<Vec<_>, _>>()?;
            calculation(inputs)?
        }
        Expr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            let condition = derive(cond, environment, signature)?;
            let branches = alternatives([
                derive(then, environment, signature)?.annotations,
                derive(otherwise, environment, signature)?.annotations,
            ])?;
            overlay(&branches, &calculation([condition.annotations])?)?
        }
        Expr::Index { base, index } => {
            let base = derive(base, environment, signature)?;
            let index = derive(index, environment, signature)?;
            let exact = match base.ty {
                DataType::Array(_) => Some(project_segment(
                    &base.annotations,
                    &SchemaPathSegment::ArrayElement,
                )?),
                DataType::Map(_) => Some(project_segment(
                    &base.annotations,
                    &SchemaPathSegment::MapValue,
                )?),
                _ => None,
            };
            match exact {
                Some(exact) => overlay(&exact, &calculation([index.annotations])?)?,
                None => calculation([base.annotations, index.annotations])?,
            }
        }
        Expr::List(items) => {
            let items = items
                .iter()
                .map(|item| derive(item, environment, signature).map(|item| item.annotations))
                .collect::<Result<Vec<_>, _>>()?;
            let element = alternatives(items)?;
            relocate(&element, &[SchemaPathSegment::ArrayElement])?
        }
        Expr::MapLit(entries) => {
            let values = entries
                .iter()
                .map(|(_, value)| {
                    derive(value, environment, signature).map(|item| item.annotations)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let value = alternatives(values)?;
            relocate(&value, &[SchemaPathSegment::MapValue])?
        }
        Expr::Struct(fields) => {
            let mut result = AnnotationTable::empty();
            for (name, value) in fields {
                let value = derive(value, environment, signature)?;
                let value = relocate(
                    &value.annotations,
                    &[SchemaPathSegment::Field(name.clone())],
                )?;
                result = overlay(&result, &value)?;
            }
            result
        }
        Expr::Tagged { case, fields, .. } => {
            let mut payload = AnnotationTable::empty();
            for (name, value) in fields {
                let value = derive(value, environment, signature)?;
                let value = relocate(
                    &value.annotations,
                    &[SchemaPathSegment::Field(name.clone())],
                )?;
                payload = overlay(&payload, &value)?;
            }
            relocate(
                &payload,
                &[SchemaPathSegment::TaggedUnionCase(case.clone())],
            )?
        }
        Expr::As { expr, target } => {
            let source = derive(expr, environment, signature)?;
            narrow(&source, target)?
        }
    };
    Ok(Derived { ty, annotations })
}

fn narrow(source: &Derived, target: &NarrowTarget) -> Result<AnnotationTable, PropagationError> {
    match target {
        NarrowTarget::Case { name, .. } => project_segment(
            &source.annotations,
            &SchemaPathSegment::TaggedUnionCase(name.clone()),
        ),
        NarrowTarget::Type(target) if source.ty == *target.ty() => Ok(source.annotations.clone()),
        NarrowTarget::Type(target) => {
            let DataType::StructuralUnion(union) = &source.ty else {
                return calculation([source.annotations.clone()]);
            };
            let Some(index) = union
                .branches()
                .iter()
                .position(|branch| branch == target.ty())
                .and_then(|index| u16::try_from(index).ok())
            else {
                return calculation([source.annotations.clone()]);
            };
            project_segment(
                &source.annotations,
                &SchemaPathSegment::StructuralUnionBranch(index),
            )
        }
    }
}

fn project_members(binding: &Binding, members: &[String]) -> Result<Binding, PropagationError> {
    if members.is_empty() {
        return Ok(binding.clone());
    }
    let mut ty = &binding.ty;
    let mut prefix = Vec::with_capacity(members.len());
    for member in members {
        match ty {
            DataType::Struct(structure) => {
                let Some(field) = structure
                    .fields()
                    .iter()
                    .find(|field| field.name() == member)
                else {
                    return Err(PropagationError::InvalidProjection(members.join(".")));
                };
                prefix.push(SchemaPathSegment::Field(member.clone()));
                ty = field.ty();
            }
            DataType::Map(map) if map.key() == &DataType::Text => {
                prefix.push(SchemaPathSegment::MapValue);
                ty = map.value();
            }
            _ => return Err(PropagationError::InvalidProjection(members.join("."))),
        }
    }
    Ok(Binding::new(
        ty.clone(),
        project_prefix(&binding.annotations, &prefix)?,
    ))
}

fn project_segment(
    table: &AnnotationTable,
    segment: &SchemaPathSegment,
) -> Result<AnnotationTable, PropagationError> {
    project_prefix(table, core::slice::from_ref(segment))
}

fn project_prefix(
    table: &AnnotationTable,
    prefix: &[SchemaPathSegment],
) -> Result<AnnotationTable, PropagationError> {
    if table.is_empty() {
        return Ok(AnnotationTable::empty());
    }
    let mut rows = BTreeMap::<SchemaPath, Vec<DataAnnotation>>::new();
    for (path, set) in table.entries() {
        let segments = path.segments();
        if segments.starts_with(prefix) {
            push_annotations(
                &mut rows,
                SchemaPath::new(segments[prefix.len()..].to_vec()),
                set.annotations().iter().cloned(),
            )?;
        } else if prefix.starts_with(segments) {
            push_annotations(
                &mut rows,
                SchemaPath::root(),
                set.annotations()
                    .iter()
                    .filter_map(derived_security_annotation),
            )?;
        }
    }
    table_from_rows(rows)
}

pub(crate) fn relocate(
    table: &AnnotationTable,
    prefix: &[SchemaPathSegment],
) -> Result<AnnotationTable, PropagationError> {
    if table.is_empty() {
        return Ok(AnnotationTable::empty());
    }
    let entries = table
        .entries()
        .iter()
        .map(|(path, set)| {
            let mut segments = prefix.to_vec();
            segments.extend_from_slice(path.segments());
            (SchemaPath::new(segments), set.clone())
        })
        .collect();
    AnnotationTable::new(entries).map_err(PropagationError::InvalidDerivedTable)
}

/// Replaces one exact structural subtree in an inherited table. Ancestor
/// classifications remain in force; annotations at or below the written path
/// are replaced by the value being written.
pub(crate) fn replace_data_path(
    inherited: &AnnotationTable,
    output_type: &DataType,
    members: &[String],
    replacement: &AnnotationTable,
) -> Result<AnnotationTable, PropagationError> {
    if members.is_empty() {
        return Ok(replacement.clone());
    }
    let prefix = member_segments(output_type, members)?;
    let retained = inherited
        .entries()
        .iter()
        .filter(|(path, _)| !path.segments().starts_with(&prefix))
        .map(|(path, set)| (path.clone(), set.clone()))
        .collect();
    let retained = AnnotationTable::new(retained).map_err(PropagationError::InvalidDerivedTable)?;
    let replacement = relocate(replacement, &prefix)?;
    overlay(&retained, &replacement)
}

/// Nests a table below a generated structural position, such as Loop's
/// `LoopExpired.last_state` carrier.
pub(crate) fn nest(
    table: &AnnotationTable,
    prefix: &[SchemaPathSegment],
) -> Result<AnnotationTable, PropagationError> {
    relocate(table, prefix)
}

fn member_segments(
    root: &DataType,
    members: &[String],
) -> Result<Vec<SchemaPathSegment>, PropagationError> {
    let mut ty = root;
    let mut segments = Vec::with_capacity(members.len());
    for member in members {
        match ty {
            DataType::Struct(structure) => {
                let Some(field) = structure
                    .fields()
                    .iter()
                    .find(|field| field.name() == member)
                else {
                    return Err(PropagationError::InvalidProjection(members.join(".")));
                };
                segments.push(SchemaPathSegment::Field(member.clone()));
                ty = field.ty();
            }
            DataType::Map(map) if map.key() == &DataType::Text => {
                segments.push(SchemaPathSegment::MapValue);
                ty = map.value();
            }
            _ => return Err(PropagationError::InvalidProjection(members.join("."))),
        }
    }
    Ok(segments)
}

/// Combines independent derived fragments or overlays an explicit
/// destination classification. Missing semantic annotations are not a
/// conflict here; an explicit result declaration is an auditable assertion.
pub(crate) fn overlay(
    left: &AnnotationTable,
    right: &AnnotationTable,
) -> Result<AnnotationTable, PropagationError> {
    let mut rows = table_rows(left);
    for (path, set) in right.entries() {
        for annotation in set.annotations() {
            insert_annotation(&mut rows, path.clone(), annotation.clone())?;
        }
    }
    table_from_rows(rows)
}

/// Merges values that may arrive at the same logical position. Security is a
/// conservative maximum. Measure and cost must be identical on every
/// alternative; otherwise the destination would sometimes mean something
/// its schema cannot state.
pub(crate) fn alternatives(
    tables: impl IntoIterator<Item = AnnotationTable>,
) -> Result<AnnotationTable, PropagationError> {
    let tables: Vec<_> = tables.into_iter().collect();
    let Some(first) = tables.first() else {
        return Ok(AnnotationTable::empty());
    };
    let first_semantics = semantic_rows(first);
    if tables
        .iter()
        .skip(1)
        .any(|table| semantic_rows(table) != first_semantics)
    {
        let candidate_paths = tables
            .iter()
            .flat_map(|table| table.entries().keys().cloned())
            .collect::<alloc::collections::BTreeSet<_>>();
        for path in candidate_paths {
            for kind in [AnnotationKind::Measure, AnnotationKind::Cost] {
                let values: Vec<_> = tables
                    .iter()
                    .map(|table| annotation_at(table, &path, kind))
                    .collect();
                if values.windows(2).any(|pair| pair[0] != pair[1]) {
                    return Err(PropagationError::ConflictingMeaning { path, kind });
                }
            }
        }
    }
    let mut rows = BTreeMap::new();
    for table in &tables {
        for (path, set) in table.entries() {
            for annotation in set.annotations().iter().filter(|annotation| {
                matches!(
                    annotation,
                    DataAnnotation::Sensitive(_) | DataAnnotation::Secret
                )
            }) {
                insert_annotation(&mut rows, path.clone(), annotation.clone())?;
            }
        }
    }
    let personal_paths = tables
        .iter()
        .flat_map(|table| {
            table
                .entries()
                .iter()
                .filter(|(_, set)| {
                    set.annotations()
                        .iter()
                        .any(|annotation| matches!(annotation, DataAnnotation::Personal(_)))
                })
                .map(|(path, _)| path.clone())
        })
        .collect::<alloc::collections::BTreeSet<_>>();
    for path in personal_paths {
        let mut combined: Option<crate::schema_annotation::PersonalDataClass> = None;
        let mut every_alternative_is_identifier = true;
        for table in &tables {
            let class =
                annotation_at(table, &path, AnnotationKind::Personal).and_then(|annotation| {
                    match annotation {
                        DataAnnotation::Personal(class) => Some(*class),
                        _ => None,
                    }
                });
            every_alternative_is_identifier &= class.is_some_and(|class| class.is_identifier());
            if let Some(class) = class {
                combined = Some(match combined {
                    None => class,
                    Some(current) => current.combine(class),
                });
            }
        }
        if let Some(class) = combined {
            let class = crate::schema_annotation::PersonalDataClass::new(
                class.is_special_category(),
                class.is_criminal_offence(),
                every_alternative_is_identifier,
            );
            insert_annotation(&mut rows, path, DataAnnotation::Personal(class))?;
        }
    }
    for (path, annotations) in first_semantics {
        for annotation in annotations {
            insert_annotation(&mut rows, path.clone(), annotation)?;
        }
    }
    table_from_rows(rows)
}

fn calculation(
    tables: impl IntoIterator<Item = AnnotationTable>,
) -> Result<AnnotationTable, PropagationError> {
    let mut rows = BTreeMap::new();
    for table in tables {
        for set in table.entries().values() {
            for annotation in set
                .annotations()
                .iter()
                .filter_map(derived_security_annotation)
            {
                insert_annotation(&mut rows, SchemaPath::root(), annotation)?;
            }
        }
    }
    table_from_rows(rows)
}

/// Collapses every security classification visible in the inputs onto the
/// result root and deliberately discards accounting meaning.
pub(crate) fn security_taint(
    tables: impl IntoIterator<Item = AnnotationTable>,
) -> Result<AnnotationTable, PropagationError> {
    calculation(tables)
}

fn table_rows(table: &AnnotationTable) -> BTreeMap<SchemaPath, Vec<DataAnnotation>> {
    table
        .entries()
        .iter()
        .map(|(path, set)| (path.clone(), set.annotations().to_vec()))
        .collect()
}

fn semantic_rows(table: &AnnotationTable) -> BTreeMap<SchemaPath, Vec<DataAnnotation>> {
    table
        .entries()
        .iter()
        .filter_map(|(path, set)| {
            let annotations: Vec<_> = set
                .annotations()
                .iter()
                .filter(|annotation| {
                    matches!(
                        annotation,
                        DataAnnotation::Measure(_) | DataAnnotation::Cost(_)
                    )
                })
                .cloned()
                .collect();
            (!annotations.is_empty()).then(|| (path.clone(), annotations))
        })
        .collect()
}

fn annotation_at<'a>(
    table: &'a AnnotationTable,
    path: &SchemaPath,
    kind: AnnotationKind,
) -> Option<&'a DataAnnotation> {
    table
        .entries()
        .get(path)?
        .annotations()
        .iter()
        .find(|annotation| annotation.kind() == kind)
}

fn push_annotations(
    rows: &mut BTreeMap<SchemaPath, Vec<DataAnnotation>>,
    path: SchemaPath,
    annotations: impl IntoIterator<Item = DataAnnotation>,
) -> Result<(), PropagationError> {
    for annotation in annotations {
        insert_annotation(rows, path.clone(), annotation)?;
    }
    Ok(())
}

fn insert_annotation(
    rows: &mut BTreeMap<SchemaPath, Vec<DataAnnotation>>,
    path: SchemaPath,
    annotation: DataAnnotation,
) -> Result<(), PropagationError> {
    let row = rows.entry(path.clone()).or_default();
    let Some(index) = row
        .iter()
        .position(|existing| existing.kind() == annotation.kind())
    else {
        row.push(annotation);
        return Ok(());
    };
    let existing = &row[index];
    match (existing, &annotation) {
        (DataAnnotation::Sensitive(left), DataAnnotation::Sensitive(right)) => {
            row[index] = DataAnnotation::Sensitive((*left).max(*right));
            Ok(())
        }
        (DataAnnotation::Secret, DataAnnotation::Secret) => Ok(()),
        (DataAnnotation::Personal(left), DataAnnotation::Personal(right)) => {
            row[index] = DataAnnotation::Personal(left.combine(*right));
            Ok(())
        }
        _ if existing == &annotation => Ok(()),
        _ => Err(PropagationError::ConflictingMeaning {
            path,
            kind: annotation.kind(),
        }),
    }
}

fn table_from_rows(
    rows: BTreeMap<SchemaPath, Vec<DataAnnotation>>,
) -> Result<AnnotationTable, PropagationError> {
    let entries = rows
        .into_iter()
        .filter_map(|(path, annotations)| {
            (!annotations.is_empty()).then(|| {
                AnnotationSet::new(annotations)
                    .map(|set| (path, set))
                    .map_err(PropagationError::InvalidDerivedTable)
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    AnnotationTable::new(entries).map_err(PropagationError::InvalidDerivedTable)
}

/// Whether one table contains secret-classified data anywhere.
#[must_use]
pub fn contains_secret(table: &AnnotationTable) -> bool {
    table.entries().values().any(|set| {
        set.annotations()
            .iter()
            .any(|annotation| annotation == &DataAnnotation::Secret)
    })
}

/// Whether one table contains GDPR personal-data classification anywhere.
#[must_use]
pub fn contains_personal(table: &AnnotationTable) -> bool {
    table.entries().values().any(|set| {
        set.annotations()
            .iter()
            .any(|annotation| matches!(annotation, DataAnnotation::Personal(_)))
    })
}

/// Whether formatting this schema requires consulting the redactor.
#[must_use]
pub fn requires_redaction(table: &AnnotationTable) -> bool {
    table.entries().values().any(|set| {
        set.annotations().iter().any(|annotation| {
            matches!(
                annotation,
                DataAnnotation::Sensitive(_) | DataAnnotation::Secret | DataAnnotation::Personal(_)
            )
        })
    })
}

/// A presentation wrapper that consults policy before formatting any value.
///
/// Root-classified values become one marker. Composite values retain their
/// shape while classified descendants become markers. It is intentionally a
/// formatter rather than a replacement [`Value`]: a redaction marker is not a
/// value of the original logical type and must never re-enter execution.
pub struct RedactedValue<'a> {
    value: &'a Value,
    annotations: &'a AnnotationTable,
}

impl<'a> RedactedValue<'a> {
    /// Creates a safe presentation view. Constructing the view performs no
    /// schema walk; formatting an empty table delegates straight to `Value`.
    #[must_use]
    pub const fn new(value: &'a Value, annotations: &'a AnnotationTable) -> Self {
        Self { value, annotations }
    }
}

impl fmt::Debug for RedactedValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.annotations.is_empty() {
            return self.value.fmt(f);
        }
        format_value(f, self.value, self.annotations, &SchemaPath::root())
    }
}

fn path_is_hidden(table: &AnnotationTable, path: &SchemaPath) -> bool {
    table.entries().iter().any(|(classified, set)| {
        path.segments().starts_with(classified.segments())
            && set.annotations().iter().any(|annotation| {
                matches!(
                    annotation,
                    DataAnnotation::Sensitive(_)
                        | DataAnnotation::Secret
                        | DataAnnotation::Personal(_)
                )
            })
    })
}

fn derived_security_annotation(annotation: &DataAnnotation) -> Option<DataAnnotation> {
    match annotation {
        DataAnnotation::Sensitive(_) | DataAnnotation::Secret => Some(annotation.clone()),
        DataAnnotation::Personal(class) => Some(DataAnnotation::Personal(class.derived())),
        DataAnnotation::Measure(_) | DataAnnotation::Cost(_) => None,
    }
}

fn format_value(
    f: &mut fmt::Formatter<'_>,
    value: &Value,
    table: &AnnotationTable,
    path: &SchemaPath,
) -> fmt::Result {
    if path_is_hidden(table, path) {
        return f.write_str("[REDACTED]");
    }
    match value {
        Value::Array(items) => {
            f.write_str("[")?;
            let child = path.child(SchemaPathSegment::ArrayElement);
            for (index, item) in items.iter().enumerate() {
                if index != 0 {
                    f.write_str(", ")?;
                }
                format_value(f, item, table, &child)?;
            }
            f.write_str("]")
        }
        Value::Map(entries) => {
            f.write_str("{")?;
            let key_path = path.child(SchemaPathSegment::MapKey);
            let value_path = path.child(SchemaPathSegment::MapValue);
            for (index, (name, item)) in entries.iter().enumerate() {
                if index != 0 {
                    f.write_str(", ")?;
                }
                if path_is_hidden(table, &key_path) {
                    f.write_str("[REDACTED]")?;
                } else {
                    write!(f, "{name:?}")?;
                }
                f.write_str(": ")?;
                format_value(f, item, table, &value_path)?;
            }
            f.write_str("}")
        }
        Value::KeyedMap(entries) => {
            f.write_str("{")?;
            let key_path = path.child(SchemaPathSegment::MapKey);
            let value_path = path.child(SchemaPathSegment::MapValue);
            for (index, (key, item)) in entries.iter().enumerate() {
                if index != 0 {
                    f.write_str(", ")?;
                }
                format_value(f, key, table, &key_path)?;
                f.write_str(": ")?;
                format_value(f, item, table, &value_path)?;
            }
            f.write_str("}")
        }
        Value::Struct(fields) => {
            f.write_str("{")?;
            for (index, (name, item)) in fields.iter().enumerate() {
                if index != 0 {
                    f.write_str(", ")?;
                }
                write!(f, "{name}: ")?;
                format_value(
                    f,
                    item,
                    table,
                    &path.child(SchemaPathSegment::Field(name.clone())),
                )?;
            }
            f.write_str("}")
        }
        Value::StructuralUnion { branch, value } => format_value(
            f,
            value,
            table,
            &path.child(SchemaPathSegment::StructuralUnionBranch(*branch)),
        ),
        Value::TaggedUnion(tagged) => {
            let case_path = path.child(SchemaPathSegment::TaggedUnionCase(tagged.case.clone()));
            if path_is_hidden(table, &case_path) {
                return f.write_str("[REDACTED]");
            }
            write!(f, "{} {{", tagged.case)?;
            for (index, (name, item)) in tagged.payload.iter().enumerate() {
                if index != 0 {
                    f.write_str(", ")?;
                }
                write!(f, "{name}: ")?;
                format_value(
                    f,
                    item,
                    table,
                    &case_path.child(SchemaPathSegment::Field(name.clone())),
                )?;
            }
            f.write_str("}")
        }
        other => fmt::Debug::fmt(other, f),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datatype::{StructType, TypeField};
    use crate::schema_annotation::{
        CostAnnotation, CostBasis, CurrencyCode, MeasureId, PersonalDataClass, SensitivityLevel,
    };
    use alloc::boxed::Box;
    use alloc::format;
    use alloc::vec;

    fn set(annotation: DataAnnotation) -> AnnotationSet {
        AnnotationSet::new(vec![annotation])
            .unwrap_or_else(|error| panic!("one annotation is valid: {error}"))
    }

    fn table(path: SchemaPath, annotation: DataAnnotation) -> AnnotationTable {
        AnnotationTable::new(vec![(path, set(annotation))])
            .unwrap_or_else(|error| panic!("one row is valid: {error}"))
    }

    fn record_type() -> DataType {
        DataType::Struct(
            StructType::new(vec![
                TypeField::new("prompt", DataType::Text),
                TypeField::new("tokens", DataType::INT),
            ])
            .unwrap_or_else(|error| panic!("record type: {error}")),
        )
    }

    #[test]
    fn exact_projection_preserves_meaning_and_calculation_keeps_only_taint() {
        let prompt = SchemaPath::root().child(SchemaPathSegment::Field("prompt".into()));
        let tokens = SchemaPath::root().child(SchemaPathSegment::Field("tokens".into()));
        let source = overlay(
            &table(
                prompt,
                DataAnnotation::Sensitive(SensitivityLevel::Confidential),
            ),
            &table(
                tokens,
                DataAnnotation::Measure(
                    MeasureId::new("gen_ai.usage.input_tokens")
                        .unwrap_or_else(|error| panic!("measure: {error}")),
                ),
            ),
        )
        .unwrap_or_else(|error| panic!("disjoint rows: {error}"));
        let environment = Environment::new().with("data", record_type(), source);

        let projected = expression(
            &expr::Tree {
                root: Expr::Path(vec!["data".into(), "tokens".into()]),
                ty: DataType::INT,
            },
            &environment,
        )
        .unwrap_or_else(|error| panic!("projection: {error}"));
        assert!(matches!(
            projected
                .entries()
                .get(&SchemaPath::root())
                .map(AnnotationSet::annotations),
            Some([DataAnnotation::Measure(_)])
        ));

        let calculated = expression(
            &expr::Tree {
                root: Expr::Binary {
                    op: expr::BinaryOp::Add,
                    left: Box::new(Expr::Path(vec!["data".into(), "tokens".into()])),
                    right: Box::new(Expr::Literal(Value::Int(1))),
                },
                ty: DataType::INT,
            },
            &environment,
        )
        .unwrap_or_else(|error| panic!("calculation: {error}"));
        assert!(
            calculated.is_empty(),
            "a calculation cannot invent measure meaning"
        );
    }

    #[test]
    fn derivation_takes_the_maximum_security_class() {
        let source = overlay(
            &table(
                SchemaPath::root().child(SchemaPathSegment::Field("prompt".into())),
                DataAnnotation::Sensitive(SensitivityLevel::Internal),
            ),
            &table(
                SchemaPath::root().child(SchemaPathSegment::Field("tokens".into())),
                DataAnnotation::Secret,
            ),
        )
        .unwrap_or_else(|error| panic!("source table: {error}"));
        let environment = Environment::new().with("data", record_type(), source);
        let result = expression(
            &expr::Tree {
                root: Expr::Binary {
                    op: expr::BinaryOp::Eq,
                    left: Box::new(Expr::Path(vec!["data".into(), "prompt".into()])),
                    right: Box::new(Expr::Path(vec!["data".into(), "tokens".into()])),
                },
                ty: DataType::Bool,
            },
            &environment,
        )
        .unwrap_or_else(|error| panic!("derived comparison: {error}"));
        let annotations = result
            .entries()
            .get(&SchemaPath::root())
            .map(AnnotationSet::annotations)
            .unwrap_or(&[]);
        assert_eq!(
            annotations,
            [
                DataAnnotation::Sensitive(SensitivityLevel::Internal),
                DataAnnotation::Secret,
            ]
        );
    }

    #[test]
    fn personal_data_propagates_but_calculation_drops_subject_identifier_meaning() {
        let prompt = SchemaPath::root().child(SchemaPathSegment::Field("prompt".into()));
        let source = table(
            prompt,
            DataAnnotation::Personal(PersonalDataClass::new(true, false, true)),
        );
        let environment = Environment::new().with("data", record_type(), source);

        let projected = expression(
            &expr::Tree {
                root: Expr::Path(vec!["data".into(), "prompt".into()]),
                ty: DataType::Text,
            },
            &environment,
        )
        .unwrap_or_else(|error| panic!("personal projection: {error}"));
        assert!(matches!(
            annotation_at(&projected, &SchemaPath::root(), AnnotationKind::Personal),
            Some(DataAnnotation::Personal(class))
                if class.is_special_category() && class.is_identifier()
        ));

        let derived = expression(
            &expr::Tree {
                root: Expr::Binary {
                    op: expr::BinaryOp::Eq,
                    left: Box::new(Expr::Path(vec!["data".into(), "prompt".into()])),
                    right: Box::new(Expr::Literal(Value::Text("probe".into()))),
                },
                ty: DataType::Bool,
            },
            &environment,
        )
        .unwrap_or_else(|error| panic!("personal calculation: {error}"));
        assert!(matches!(
            annotation_at(&derived, &SchemaPath::root(), AnnotationKind::Personal),
            Some(DataAnnotation::Personal(class))
                if class.is_special_category() && !class.is_identifier()
        ));
    }

    #[test]
    fn personal_alternatives_keep_identifier_meaning_only_when_every_branch_has_it() {
        let path = SchemaPath::root();
        let identifier = table(
            path.clone(),
            DataAnnotation::Personal(PersonalDataClass::new(false, false, true)),
        );
        let special_non_identifier = table(
            path,
            DataAnnotation::Personal(PersonalDataClass::new(true, false, false)),
        );
        let merged = alternatives([identifier, special_non_identifier])
            .unwrap_or_else(|error| panic!("personal alternatives: {error}"));
        assert!(matches!(
            annotation_at(&merged, &SchemaPath::root(), AnnotationKind::Personal),
            Some(DataAnnotation::Personal(class))
                if class.is_special_category() && !class.is_identifier()
        ));
    }

    #[test]
    fn alternative_accounting_meanings_refuse_instead_of_becoming_a_lie() {
        let eur = CurrencyCode::new("EUR").unwrap_or_else(|error| panic!("currency: {error}"));
        let cost = table(
            SchemaPath::root(),
            DataAnnotation::Cost(CostAnnotation::scalar(eur, CostBasis::ProviderReported)),
        );
        assert!(matches!(
            alternatives([cost, AnnotationTable::empty()]),
            Err(PropagationError::ConflictingMeaning {
                kind: AnnotationKind::Cost,
                ..
            })
        ));
    }

    #[test]
    fn redaction_happens_before_a_classified_leaf_is_formatted() {
        let annotations = table(
            SchemaPath::root().child(SchemaPathSegment::Field("prompt".into())),
            DataAnnotation::Sensitive(SensitivityLevel::Restricted),
        );
        let value = Value::Struct(vec![
            ("prompt".into(), Value::Text("do not print".into())),
            ("tokens".into(), Value::Int(7)),
        ]);
        let rendered = format!("{:?}", RedactedValue::new(&value, &annotations));
        assert_eq!(rendered, "{prompt: [REDACTED], tokens: Int(7)}");
        assert!(!rendered.contains("do not print"));
    }

    #[test]
    fn personal_data_is_redacted_without_becoming_a_sensitivity_level() {
        let annotations = table(
            SchemaPath::root(),
            DataAnnotation::Personal(PersonalDataClass::new(false, false, true)),
        );
        assert!(requires_redaction(&annotations));
        assert!(contains_personal(&annotations));
        assert!(!contains_secret(&annotations));
        let rendered = format!(
            "{:?}",
            RedactedValue::new(&Value::Text("alice@example.test".into()), &annotations)
        );
        assert_eq!(rendered, "[REDACTED]");
        assert!(!rendered.contains("alice"));
    }

    #[test]
    fn redaction_covers_map_keys_and_tagged_discriminants() {
        let key_annotations = table(
            SchemaPath::root().child(SchemaPathSegment::MapKey),
            DataAnnotation::Sensitive(SensitivityLevel::Restricted),
        );
        let map = Value::Map(BTreeMap::from([(
            String::from("classified-key"),
            Value::Int(1),
        )]));
        let rendered = format!("{:?}", RedactedValue::new(&map, &key_annotations));
        assert_eq!(rendered, "{[REDACTED]: Int(1)}");
        assert!(!rendered.contains("classified-key"));

        let case_annotations = table(
            SchemaPath::root().child(SchemaPathSegment::TaggedUnionCase(String::from(
                "ClassifiedCase",
            ))),
            DataAnnotation::Sensitive(SensitivityLevel::Confidential),
        );
        let tagged = Value::TaggedUnion(Box::new(crate::value::TaggedUnionValue {
            case: String::from("ClassifiedCase"),
            payload: vec![(String::from("value"), Value::Int(1))],
        }));
        let rendered = format!("{:?}", RedactedValue::new(&tagged, &case_annotations));
        assert_eq!(rendered, "[REDACTED]");
        assert!(!rendered.contains("ClassifiedCase"));
    }
}
