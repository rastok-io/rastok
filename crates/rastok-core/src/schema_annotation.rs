//! Sparse data-policy metadata beside Rastok's logical type tree.
//!
//! [`DataType`] remains the sole assignability
//! authority. This module describes classifications and accounting meaning at
//! canonical positions in that tree; it never wraps or extends the logical
//! type algebra.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::datatype::{DataType, StructType};

mod application;
pub use application::{
    AnnotationApplication, AnnotationBinding, MAX_ANNOTATION_DEFINITION_BYTES, flat_field,
    measure_argument_type,
};

/// Maximum UTF-8 byte length of one authored measure id or unit.
pub const MAX_MEASURE_ID_BYTES: usize = 255;

/// SHA-256 of the ISO 4217 Maintenance Agency's current List One snapshot
/// consumed for M1.8 A1 on 2026-08-30.
pub const ISO_4217_LIST_ONE_SHA256: &str =
    "838dfb991648cf36df939edd5fe3811737962b75a32252847d239cedd1e291c9";

// Sorted alphabetic codes from that snapshot. Registry updates are semantic
// migrations; they never happen merely because a host library changed.
const ISO_4217_CODES: &[&str] = &[
    "AED", "AFN", "ALL", "AMD", "AOA", "ARS", "AUD", "AWG", "AZN", "BAM", "BBD", "BDT", "BHD",
    "BIF", "BMD", "BND", "BOB", "BOV", "BRL", "BSD", "BTN", "BWP", "BYN", "BZD", "CAD", "CDF",
    "CHE", "CHF", "CHW", "CLF", "CLP", "CNY", "COP", "COU", "CRC", "CUP", "CVE", "CZK", "DJF",
    "DKK", "DOP", "DZD", "EGP", "ERN", "ETB", "EUR", "FJD", "FKP", "GBP", "GEL", "GHS", "GIP",
    "GMD", "GNF", "GTQ", "GYD", "HKD", "HNL", "HTG", "HUF", "IDR", "ILS", "INR", "IQD", "IRR",
    "ISK", "JMD", "JOD", "JPY", "KES", "KGS", "KHR", "KMF", "KPW", "KRW", "KWD", "KYD", "KZT",
    "LAK", "LBP", "LKR", "LRD", "LSL", "LYD", "MAD", "MDL", "MGA", "MKD", "MMK", "MNT", "MOP",
    "MRU", "MUR", "MVR", "MWK", "MXN", "MXV", "MYR", "MZN", "NAD", "NGN", "NIO", "NOK", "NPR",
    "NZD", "OMR", "PAB", "PEN", "PGK", "PHP", "PKR", "PLN", "PYG", "QAR", "RON", "RSD", "RUB",
    "RWF", "SAR", "SBD", "SCR", "SDG", "SEK", "SGD", "SHP", "SLE", "SOS", "SRD", "SSP", "STN",
    "SVC", "SYP", "SZL", "THB", "TJS", "TMT", "TND", "TOP", "TRY", "TTD", "TWD", "TZS", "UAH",
    "UGX", "USD", "USN", "UYI", "UYU", "UYW", "UZS", "VED", "VES", "VND", "VUV", "WST", "XAD",
    "XAF", "XAG", "XAU", "XBA", "XBB", "XBC", "XBD", "XCD", "XCG", "XDR", "XOF", "XPD", "XPF",
    "XPT", "XSU", "XTS", "XUA", "XXX", "YER", "ZAR", "ZMW", "ZWG",
];

/// One typed step through a resolved schema.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SchemaPathSegment {
    /// A closed struct field. Empty external-schema field names remain valid.
    Field(String),
    /// The homogeneous element of an array or table.
    ArrayElement,
    /// The key of a map.
    MapKey,
    /// The value of a map.
    MapValue,
    /// One branch in canonical structural-union order.
    StructuralUnionBranch(u16),
    /// One semantic tagged-union case.
    TaggedUnionCase(String),
}

/// Canonical position in a resolved [`DataType`] tree.
///
/// The empty segment sequence is the schema root. Typed segments, rather than
/// a dotted string, keep collection and union positions injective.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SchemaPath(Vec<SchemaPathSegment>);

impl SchemaPath {
    /// The root of a resolved schema.
    #[must_use]
    pub const fn root() -> Self {
        Self(Vec::new())
    }

    /// Builds a path from already-typed segments.
    #[must_use]
    pub const fn new(segments: Vec<SchemaPathSegment>) -> Self {
        Self(segments)
    }

    /// Typed segments from root to target.
    #[must_use]
    pub fn segments(&self) -> &[SchemaPathSegment] {
        &self.0
    }

    /// Returns one child target below this path.
    #[must_use]
    pub fn child(&self, segment: SchemaPathSegment) -> Self {
        let mut segments = self.0.clone();
        segments.push(segment);
        Self(segments)
    }
}

impl fmt::Display for SchemaPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return f.write_str("$");
        }
        f.write_str("$")?;
        for segment in &self.0 {
            match segment {
                SchemaPathSegment::Field(name) => write!(f, ".field({name:?})")?,
                SchemaPathSegment::ArrayElement => f.write_str(".array_element")?,
                SchemaPathSegment::MapKey => f.write_str(".map_key")?,
                SchemaPathSegment::MapValue => f.write_str(".map_value")?,
                SchemaPathSegment::StructuralUnionBranch(index) => {
                    write!(f, ".structural_union_branch({index})")?
                }
                SchemaPathSegment::TaggedUnionCase(name) => {
                    write!(f, ".tagged_union_case({name:?})")?
                }
            }
        }
        Ok(())
    }
}

/// Ordered sensitivity classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SensitivityLevel {
    /// Intended for internal audiences, but not public export.
    Internal,
    /// Confidential business or user data.
    Confidential,
    /// The strongest non-secret classification.
    Restricted,
}

/// Canonical three-letter currency spelling.
///
/// This type owns lexical canonicality only. The M1.8 registry performs the
/// separately versioned ISO-4217 membership check for retained tariff metadata.
/// Generic measurement units do not use this type or registry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CurrencyCode(String);

impl CurrencyCode {
    /// Accepts exactly three uppercase ASCII letters.
    ///
    /// # Errors
    ///
    /// Returns [`SchemaAnnotationError::InvalidCurrencyCode`] for every other
    /// spelling.
    pub fn new(code: impl Into<String>) -> Result<Self, SchemaAnnotationError> {
        let code = code.into();
        if code.len() != 3 || !code.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err(SchemaAnnotationError::InvalidCurrencyCode(code));
        }
        Ok(Self(code))
    }

    /// Canonical uppercase spelling.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Whether a canonical spelling belongs to the pinned current ISO 4217
/// currency-and-funds registry.
#[must_use]
pub fn is_iso_4217_currency(code: &CurrencyCode) -> bool {
    ISO_4217_CODES.binary_search(&code.as_str()).is_ok()
}

/// Stable semantic-measure identifier.
///
/// This newtype checks canonical spelling, not membership in a registry.
/// Units and interpretation come from the validated annotation arguments,
/// never from the name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MeasureId(String);

impl MeasureId {
    /// Builds one lowercase dotted semantic name.
    ///
    /// # Errors
    ///
    /// Empty, oversized, non-ASCII, uppercase, punctuated, or empty-segment
    /// names return [`SchemaAnnotationError::InvalidMeasureId`].
    pub fn new(id: impl Into<String>) -> Result<Self, SchemaAnnotationError> {
        let id = id.into();
        let valid = !id.is_empty()
            && id.len() <= MAX_MEASURE_ID_BYTES
            && id.split('.').all(|segment| {
                !segment.is_empty()
                    && segment.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                    })
            });
        if !valid {
            return Err(SchemaAnnotationError::InvalidMeasureId(id));
        }
        Ok(Self(id))
    }

    /// Canonical semantic name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// GDPR-facing classification of one personal-data position.
///
/// Personal data is orthogonal to confidentiality: a public-facing account
/// name can still identify a data subject, while confidential business data
/// need not be personal. The two legal regimes below are independent flags
/// because one derived value can depend on both Article 9 special-category
/// data and Article 10 criminal-offence data. `identifier` marks a field that
/// directly or indirectly identifies the subject; calculations deliberately
/// clear that claim while retaining personal-data taint.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PersonalDataClass {
    special_category: bool,
    criminal_offence: bool,
    identifier: bool,
}

impl PersonalDataClass {
    /// Builds one closed personal-data classification.
    #[must_use]
    pub const fn new(special_category: bool, criminal_offence: bool, identifier: bool) -> Self {
        Self {
            special_category,
            criminal_offence,
            identifier,
        }
    }

    /// Whether Article 9 special-category handling applies.
    #[must_use]
    pub const fn is_special_category(self) -> bool {
        self.special_category
    }

    /// Whether Article 10 criminal-offence handling applies.
    #[must_use]
    pub const fn is_criminal_offence(self) -> bool {
        self.criminal_offence
    }

    /// Whether this exact value is declared as a data-subject identifier.
    #[must_use]
    pub const fn is_identifier(self) -> bool {
        self.identifier
    }

    /// Conservative result of combining dependencies at one derived path.
    ///
    /// Legal classes accumulate. Identifier meaning survives only when both
    /// operands are themselves identifiers; an arbitrary calculation may not
    /// invent a subject index.
    #[must_use]
    pub const fn combine(self, other: Self) -> Self {
        Self {
            special_category: self.special_category || other.special_category,
            criminal_offence: self.criminal_offence || other.criminal_offence,
            identifier: self.identifier && other.identifier,
        }
    }

    /// Personal-data taint after a calculation whose identifier semantics are
    /// not independently registered.
    #[must_use]
    pub const fn derived(self) -> Self {
        Self {
            special_category: self.special_category,
            criminal_offence: self.criminal_offence,
            identifier: false,
        }
    }
}

/// A compact standard interpretation or a typed, nominal metadata application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataAnnotation {
    /// Redaction and propagation classification.
    Sensitive(SensitivityLevel),
    /// Secret-handling marker. Secret origin remains the expression-level
    /// `secret("path")` function.
    Secret,
    /// GDPR personal-data classification and optional subject-id role.
    Personal(PersonalDataClass),
    /// A nominally bound application of an ordinary flat argument schema.
    Declared(Box<AnnotationApplication>),
}

impl DataAnnotation {
    /// Consumer class of this annotation; declared metadata also has a nominal identity.
    #[must_use]
    pub const fn kind(&self) -> AnnotationKind {
        match self {
            Self::Sensitive(_) => AnnotationKind::Sensitive,
            Self::Secret => AnnotationKind::Secret,
            Self::Personal(_) => AnnotationKind::Personal,
            Self::Declared(application) => match application.binding() {
                AnnotationBinding::Metadata => AnnotationKind::Metadata,
                AnnotationBinding::Measure => AnnotationKind::Measure,
            },
        }
    }

    /// Canonical identity at one path. Two descriptive declarations are not
    /// duplicates merely because their argument fields have the same shape.
    #[must_use]
    pub fn identity(&self) -> (AnnotationKind, &str) {
        (
            self.kind(),
            match self {
                Self::Declared(application)
                    if application.binding() == AnnotationBinding::Metadata =>
                {
                    application.definition()
                }
                _ => "",
            },
        )
    }
}

/// Canonical identity order of the closed annotation family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AnnotationKind {
    /// `rastok/sensitive@1`.
    Sensitive,
    /// `rastok/secret@1`.
    Secret,
    /// The protected numerical or monetary measure interpretation.
    Measure,
    /// `rastok/personal@1`.
    ///
    /// Appended after the original A1 registry so existing annotation-set
    /// order and semantic identity remain unchanged.
    Personal,
    /// User-declared descriptive metadata; ordered by declaring name within this kind.
    Metadata,
}

/// Canonically ordered annotations attached to one schema path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationSet(Vec<DataAnnotation>);

impl AnnotationSet {
    /// Sorts by canonical annotation identity and rejects duplicate definitions or
    /// the empty set.
    ///
    /// # Errors
    ///
    /// Returns the exact duplicate kind or [`SchemaAnnotationError::EmptySet`].
    pub fn new(mut annotations: Vec<DataAnnotation>) -> Result<Self, SchemaAnnotationError> {
        if annotations.is_empty() {
            return Err(SchemaAnnotationError::EmptySet);
        }
        // Equal identities are rejected below, so stable tie order carries no
        // meaning. In-place sorting avoids a second annotation-sized buffer.
        annotations.sort_unstable_by(|left, right| left.identity().cmp(&right.identity()));
        if let Some(kind) = annotations
            .windows(2)
            .find_map(|pair| (pair[0].identity() == pair[1].identity()).then(|| pair[0].kind()))
        {
            return Err(SchemaAnnotationError::DuplicateAnnotation(kind));
        }
        Ok(Self(annotations))
    }

    /// Annotations in canonical identity order.
    #[must_use]
    pub fn annotations(&self) -> &[DataAnnotation] {
        &self.0
    }
}

/// Sparse canonical annotation side-table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnnotationTable(BTreeMap<SchemaPath, AnnotationSet>);

impl AnnotationTable {
    /// Empty table used by every unannotated schema.
    #[must_use]
    pub const fn empty() -> Self {
        Self(BTreeMap::new())
    }

    /// Canonicalizes rows by path and rejects duplicate paths.
    ///
    /// # Errors
    ///
    /// Returns [`SchemaAnnotationError::DuplicatePath`] for a second row at
    /// the same canonical target.
    pub fn new(entries: Vec<(SchemaPath, AnnotationSet)>) -> Result<Self, SchemaAnnotationError> {
        let mut table = BTreeMap::new();
        for (path, annotations) in entries {
            if table.insert(path.clone(), annotations).is_some() {
                return Err(SchemaAnnotationError::DuplicatePath(path));
            }
        }
        Ok(Self(table))
    }

    /// Rows in canonical path order.
    #[must_use]
    pub const fn entries(&self) -> &BTreeMap<SchemaPath, AnnotationSet> {
        &self.0
    }

    /// Whether the hot path can skip every annotation walk.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Checks every path and interpretation without copying the schema or its metadata.
    ///
    /// # Errors
    /// Returns the first canonical path, target or annotation contract violation.
    pub(crate) fn validate_for(&self, ty: &DataType) -> Result<(), SchemaAnnotationError> {
        for (path, set) in self.entries() {
            let target = validate_path(ty, path)?;
            for annotation in set.annotations() {
                validate_annotation(annotation, &target, path)?;
            }
        }
        Ok(())
    }
}

/// One logical schema and its orthogonal policy table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotatedSchema {
    ty: DataType,
    annotations: AnnotationTable,
}

impl AnnotatedSchema {
    /// Verifies every sparse path and annotation contract against the
    /// adjacent logical type and pinned registries.
    ///
    /// # Errors
    ///
    /// Returns the first canonical path, target, carrier, source or registry
    /// membership that does not hold. This is the Registry-free authority
    /// reused by package validation after decode.
    pub fn new(ty: DataType, annotations: AnnotationTable) -> Result<Self, SchemaAnnotationError> {
        annotations.validate_for(&ty)?;
        Ok(Self { ty, annotations })
    }

    /// Logical type and sole assignability authority.
    #[must_use]
    pub const fn ty(&self) -> &DataType {
        &self.ty
    }

    /// Sparse policy/accounting table.
    #[must_use]
    pub const fn annotations(&self) -> &AnnotationTable {
        &self.annotations
    }
}

enum PathCursor<'a> {
    Type(&'a DataType),
    Struct(&'a StructType),
}

enum AnnotationTarget<'a> {
    Type(&'a DataType),
    TaggedUnionCase,
}

fn validate_path<'a>(
    ty: &'a DataType,
    path: &SchemaPath,
) -> Result<AnnotationTarget<'a>, SchemaAnnotationError> {
    let mut cursor = PathCursor::Type(ty);
    for (position, segment) in path.segments().iter().enumerate() {
        cursor = match (cursor, segment) {
            (PathCursor::Type(DataType::Struct(structure)), SchemaPathSegment::Field(name))
            | (PathCursor::Struct(structure), SchemaPathSegment::Field(name)) => {
                let Some(field) = structure.fields().iter().find(|field| field.name() == name)
                else {
                    return Err(SchemaAnnotationError::InvalidPath {
                        path: path.clone(),
                        position,
                    });
                };
                PathCursor::Type(field.ty())
            }
            (PathCursor::Type(DataType::Array(array)), SchemaPathSegment::ArrayElement) => {
                PathCursor::Type(array.element())
            }
            (PathCursor::Type(DataType::Map(map)), SchemaPathSegment::MapKey) => {
                PathCursor::Type(map.key())
            }
            (PathCursor::Type(DataType::Map(map)), SchemaPathSegment::MapValue) => {
                PathCursor::Type(map.value())
            }
            (
                PathCursor::Type(DataType::StructuralUnion(union)),
                SchemaPathSegment::StructuralUnionBranch(index),
            ) => {
                let Some(branch) = union.branches().get(usize::from(*index)) else {
                    return Err(SchemaAnnotationError::InvalidPath {
                        path: path.clone(),
                        position,
                    });
                };
                PathCursor::Type(branch)
            }
            (
                PathCursor::Type(DataType::TaggedUnion(union)),
                SchemaPathSegment::TaggedUnionCase(name),
            ) => {
                let Some(case) = union.cases().iter().find(|case| case.name() == name) else {
                    return Err(SchemaAnnotationError::InvalidPath {
                        path: path.clone(),
                        position,
                    });
                };
                PathCursor::Struct(case.payload())
            }
            _ => {
                return Err(SchemaAnnotationError::InvalidPath {
                    path: path.clone(),
                    position,
                });
            }
        };
    }
    Ok(match cursor {
        PathCursor::Type(ty) => AnnotationTarget::Type(ty),
        PathCursor::Struct(_) => AnnotationTarget::TaggedUnionCase,
    })
}

fn validate_annotation(
    annotation: &DataAnnotation,
    target: &AnnotationTarget<'_>,
    path: &SchemaPath,
) -> Result<(), SchemaAnnotationError> {
    match annotation {
        DataAnnotation::Declared(application) => {
            application.validate()?;
            if application.binding() == AnnotationBinding::Measure
                && !matches!(
                    target,
                    AnnotationTarget::Type(DataType::Integer(_) | DataType::Decimal(_))
                )
            {
                return Err(SchemaAnnotationError::IllegalAnnotationTarget {
                    path: path.clone(),
                    kind: AnnotationKind::Measure,
                });
            }
            Ok(())
        }
        DataAnnotation::Sensitive(_) | DataAnnotation::Secret | DataAnnotation::Personal(_) => {
            Ok(())
        }
    }
}

/// Invalid schema annotation metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaAnnotationError {
    /// A definition, interpretation or complete constant struct is invalid.
    InvalidApplication {
        /// Declaring name; argument values are deliberately not rendered.
        definition: String,
        /// The violated invariant, without user data.
        detail: &'static str,
    },
    /// No annotation belongs in a sparse table row.
    EmptySet,
    /// Two applications of the same annotation identity target one path.
    DuplicateAnnotation(AnnotationKind),
    /// Two rows target the same canonical path.
    DuplicatePath(SchemaPath),
    /// A path segment cannot traverse its adjacent logical type.
    InvalidPath {
        /// Complete offending path.
        path: SchemaPath,
        /// Zero-based segment position.
        position: usize,
    },
    /// Semantic-measure spelling is not canonical.
    InvalidMeasureId(String),
    /// Currency spelling is not three uppercase ASCII letters.
    InvalidCurrencyCode(String),
    /// An annotation kind cannot target this canonical schema position.
    IllegalAnnotationTarget {
        /// Exact offending path.
        path: SchemaPath,
        /// Closed annotation kind.
        kind: AnnotationKind,
    },
    /// Policy metadata was carried without any adjacent effective logical
    /// schema against which it could be checked.
    MissingLogicalType,
}

impl fmt::Display for SchemaAnnotationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidApplication { definition, detail } => {
                write!(f, "annotation definition {definition:?}: {detail}")
            }
            Self::EmptySet => f.write_str("an annotation-table row cannot be empty"),
            Self::DuplicateAnnotation(kind) => {
                write!(f, "annotation kind {kind:?} appears twice on one path")
            }
            Self::DuplicatePath(path) => write!(f, "schema path {path} appears twice"),
            Self::InvalidPath { path, position } => {
                write!(f, "schema path {path} is invalid at segment {position}")
            }
            Self::InvalidMeasureId(id) => {
                write!(f, "measure id {id:?} is not a canonical dotted name")
            }
            Self::InvalidCurrencyCode(code) => {
                write!(
                    f,
                    "currency code {code:?} is not three uppercase ASCII letters"
                )
            }
            Self::IllegalAnnotationTarget { path, kind } => {
                write!(f, "annotation kind {kind:?} cannot target {path}")
            }
            Self::MissingLogicalType => {
                f.write_str("an annotation table has no adjacent logical type")
            }
        }
    }
}

impl core::error::Error for SchemaAnnotationError {}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::datatype::{MapType, StructType, TaggedCase, TaggedUnionType, TypeField};
    use alloc::vec;

    pub(crate) fn measure(id: &str, unit: &str) -> DataAnnotation {
        use crate::value::Value;
        let ty = measure_argument_type().unwrap_or_else(|error| panic!("measure schema: {error}"));
        let DataType::Struct(schema) = &ty else {
            panic!("measure arguments are a struct")
        };
        let values = schema
            .fields()
            .iter()
            .map(|field| {
                let value = match field.name() {
                    "id" => Value::Text(id.into()),
                    "unit" => Value::Text(unit.into()),
                    _ => Value::Null,
                };
                (
                    field.name().into(),
                    field
                        .ty()
                        .narrow(value, None)
                        .unwrap_or_else(|error| panic!("measure constant: {error}")),
                )
            })
            .collect();
        DataAnnotation::Declared(Box::new(
            AnnotationApplication::new(
                AnnotationBinding::Measure,
                "rastok/measure@3".into(),
                ty,
                Value::Struct(values),
            )
            .unwrap_or_else(|error| panic!("measure application: {error}")),
        ))
    }

    fn sensitive(level: SensitivityLevel) -> AnnotationSet {
        AnnotationSet::new(vec![DataAnnotation::Sensitive(level)])
            .unwrap_or_else(|error| panic!("valid annotation set: {error}"))
    }

    #[test]
    fn annotations_are_canonical_but_do_not_wrap_the_logical_type() {
        let measure = measure("tokens", "token");
        let annotations = AnnotationSet::new(vec![
            DataAnnotation::Secret,
            measure.clone(),
            DataAnnotation::Sensitive(SensitivityLevel::Restricted),
            DataAnnotation::Personal(PersonalDataClass::new(true, false, true)),
        ])
        .unwrap_or_else(|error| panic!("valid annotation set: {error}"));
        assert_eq!(
            annotations.annotations(),
            [
                DataAnnotation::Sensitive(SensitivityLevel::Restricted),
                DataAnnotation::Secret,
                measure,
                DataAnnotation::Personal(PersonalDataClass::new(true, false, true)),
            ]
        );

        let schema = AnnotatedSchema::new(
            DataType::UINT,
            AnnotationTable::new(vec![(SchemaPath::root(), annotations)])
                .unwrap_or_else(|error| panic!("valid table: {error}")),
        )
        .unwrap_or_else(|error| panic!("valid schema: {error}"));
        assert_eq!(schema.ty(), &DataType::UINT);
    }

    #[test]
    fn personal_data_class_keeps_legal_regimes_orthogonal_to_identifier_role() {
        let general_identifier = PersonalDataClass::new(false, false, true);
        let special = PersonalDataClass::new(true, false, false);
        let combined = general_identifier.combine(special);
        assert!(combined.is_special_category());
        assert!(!combined.is_criminal_offence());
        assert!(!combined.is_identifier());
        assert_eq!(
            PersonalDataClass::new(true, true, true).derived(),
            PersonalDataClass::new(true, true, false),
            "a calculation preserves personal-data regimes but not subject-id meaning"
        );
    }

    #[test]
    fn duplicate_annotation_kinds_and_paths_refuse() {
        assert_eq!(
            AnnotationSet::new(vec![
                DataAnnotation::Sensitive(SensitivityLevel::Internal),
                DataAnnotation::Sensitive(SensitivityLevel::Restricted),
            ]),
            Err(SchemaAnnotationError::DuplicateAnnotation(
                AnnotationKind::Sensitive
            ))
        );

        let path = SchemaPath::root();
        let set = sensitive(SensitivityLevel::Internal);
        assert_eq!(
            AnnotationTable::new(vec![(path.clone(), set.clone()), (path.clone(), set)]),
            Err(SchemaAnnotationError::DuplicatePath(path))
        );
    }

    #[test]
    fn paths_distinguish_fields_collections_and_union_cases() {
        let payload = StructType::new(vec![TypeField::new(
            "items",
            DataType::array(DataType::text_map(DataType::Text)),
        )])
        .unwrap_or_else(|error| panic!("valid payload: {error}"));
        let union = TaggedUnionType::new(vec![
            TaggedCase::new("Usage", payload).unwrap_or_else(|error| panic!("valid case: {error}")),
        ])
        .unwrap_or_else(|error| panic!("valid union: {error}"));
        let path = SchemaPath::new(vec![
            SchemaPathSegment::TaggedUnionCase(String::from("Usage")),
            SchemaPathSegment::Field(String::from("items")),
            SchemaPathSegment::ArrayElement,
            SchemaPathSegment::MapValue,
        ]);
        let schema = AnnotatedSchema::new(
            DataType::TaggedUnion(union),
            AnnotationTable::new(vec![(path, sensitive(SensitivityLevel::Confidential))])
                .unwrap_or_else(|error| panic!("valid table: {error}")),
        );
        assert!(schema.is_ok());
    }

    #[test]
    fn a_path_cannot_claim_structure_the_type_does_not_have() {
        let path = SchemaPath::new(vec![SchemaPathSegment::ArrayElement]);
        let error = AnnotatedSchema::new(
            DataType::Text,
            AnnotationTable::new(vec![(path.clone(), sensitive(SensitivityLevel::Internal))])
                .unwrap_or_else(|failure| panic!("valid table: {failure}")),
        );
        assert_eq!(
            error,
            Err(SchemaAnnotationError::InvalidPath { path, position: 0 })
        );
    }

    #[test]
    fn semantic_measure_and_currency_spellings_are_canonical() {
        assert!(MeasureId::new("gen_ai.usage.input_tokens").is_ok());
        assert!(MeasureId::new("rastok.flow.envelope.count").is_ok());
        assert!(MeasureId::new("GenAI.InputTokens").is_err());
        assert!(MeasureId::new("gen_ai..input_tokens").is_err());
        assert!(MeasureId::new("gen-ai.input_tokens").is_err());
        assert_eq!(
            CurrencyCode::new("EUR")
                .unwrap_or_else(|error| panic!("valid currency: {error}"))
                .as_str(),
            "EUR"
        );
        assert!(CurrencyCode::new("eur").is_err());
    }

    #[test]
    fn currency_membership_is_pinned_separately_from_lexical_spelling() {
        let usd = CurrencyCode::new("USD")
            .unwrap_or_else(|error| panic!("canonical currency spelling: {error}"));
        let unknown = CurrencyCode::new("ZZZ")
            .unwrap_or_else(|error| panic!("canonical but unknown spelling: {error}"));
        let withdrawn = CurrencyCode::new("BGN")
            .unwrap_or_else(|error| panic!("canonical but withdrawn spelling: {error}"));
        assert!(is_iso_4217_currency(&usd));
        assert!(!is_iso_4217_currency(&unknown));
        assert!(!is_iso_4217_currency(&withdrawn));
    }

    #[test]
    fn annotated_schema_rechecks_the_numeric_target_without_an_id_registry() {
        let schema = |ty: DataType, id: &str| {
            let set = AnnotationSet::new(vec![measure(id, "item")])
                .unwrap_or_else(|error| panic!("one annotation: {error}"));
            AnnotatedSchema::new(
                ty,
                AnnotationTable::new(vec![(SchemaPath::root(), set)])
                    .unwrap_or_else(|error| panic!("one row: {error}")),
            )
        };
        for id in [
            "carrots",
            "gen_ai.usage.input_tokens",
            "http.client.request.duration",
        ] {
            assert!(schema(DataType::INT, id).is_ok());
            for ty in [DataType::DURATION, DataType::Text, DataType::Variant] {
                assert_eq!(
                    schema(ty, id),
                    Err(SchemaAnnotationError::IllegalAnnotationTarget {
                        path: SchemaPath::root(),
                        kind: AnnotationKind::Measure,
                    })
                );
            }
        }
    }

    #[test]
    fn map_key_and_structural_union_paths_are_structural() {
        let map = DataType::Map(
            MapType::new(DataType::Text, DataType::UINT)
                .unwrap_or_else(|error| panic!("valid map: {error}")),
        );
        let union = DataType::structural_union(vec![DataType::Null, map])
            .unwrap_or_else(|error| panic!("valid union: {error}"));
        let DataType::StructuralUnion(union_type) = &union else {
            panic!("expected structural union");
        };
        let map_index = union_type
            .branches()
            .iter()
            .position(|branch| matches!(branch, DataType::Map(_)))
            .and_then(|index| u16::try_from(index).ok())
            .unwrap_or_else(|| panic!("map branch"));
        let path = SchemaPath::new(vec![
            SchemaPathSegment::StructuralUnionBranch(map_index),
            SchemaPathSegment::MapKey,
        ]);
        assert!(
            AnnotatedSchema::new(
                union,
                AnnotationTable::new(vec![(path, sensitive(SensitivityLevel::Internal),)])
                    .unwrap_or_else(|error| panic!("valid table: {error}")),
            )
            .is_ok()
        );
    }

    #[test]
    fn empty_external_field_names_remain_addressable() {
        let structure = StructType::new(vec![TypeField::new("", DataType::Text)])
            .unwrap_or_else(|error| panic!("valid external field: {error}"));
        let path = SchemaPath::new(vec![SchemaPathSegment::Field(String::new())]);
        assert!(
            AnnotatedSchema::new(
                DataType::Struct(structure),
                AnnotationTable::new(vec![(path, sensitive(SensitivityLevel::Restricted),)])
                    .unwrap_or_else(|error| panic!("valid table: {error}")),
            )
            .is_ok()
        );
    }

    #[test]
    fn tagged_case_is_a_terminal_annotation_target_or_a_payload_parent() {
        let case = TaggedCase::new(
            "Done",
            StructType::new(Vec::new()).unwrap_or_else(|error| panic!("valid payload: {error}")),
        )
        .unwrap_or_else(|error| panic!("valid case: {error}"));
        let ty = DataType::TaggedUnion(
            TaggedUnionType::new(vec![case]).unwrap_or_else(|error| panic!("valid union: {error}")),
        );
        let path = SchemaPath::new(vec![SchemaPathSegment::TaggedUnionCase(String::from(
            "Done",
        ))]);
        assert!(
            AnnotatedSchema::new(
                ty,
                AnnotationTable::new(vec![(path, sensitive(SensitivityLevel::Internal),)])
                    .unwrap_or_else(|error| panic!("valid table: {error}")),
            )
            .is_ok()
        );
    }

    #[test]
    fn table_rows_are_sorted_by_structural_path() {
        let field_a = SchemaPath::new(vec![SchemaPathSegment::Field(String::from("a"))]);
        let field_b = SchemaPath::new(vec![SchemaPathSegment::Field(String::from("b"))]);
        let table = AnnotationTable::new(vec![
            (field_b.clone(), sensitive(SensitivityLevel::Internal)),
            (field_a.clone(), sensitive(SensitivityLevel::Internal)),
        ])
        .unwrap_or_else(|error| panic!("valid table: {error}"));
        assert_eq!(
            table.entries().keys().cloned().collect::<Vec<_>>(),
            [field_a, field_b]
        );
    }
}
