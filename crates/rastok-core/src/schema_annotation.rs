//! Sparse data-policy metadata beside Rastok's logical type tree.
//!
//! [`DataType`] remains the sole assignability
//! authority. This module describes classifications and accounting meaning at
//! canonical positions in that tree; it never wraps or extends the logical
//! type algebra.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::datatype::{DataType, StructType};

/// Maximum UTF-8 byte length of one registered semantic-measure name.
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

/// Evidence basis of one exact monetary observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CostBasis {
    /// The external provider reported the amount.
    ProviderReported,
    /// A versioned deployment price profile calculated the amount.
    PricebookCalculated,
}

/// Canonical three-letter currency spelling.
///
/// This type owns lexical canonicality only. The M1.8 registry performs the
/// separately versioned ISO-4217 membership check when resolving `@cost`.
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
/// OpenTelemetry names are stored verbatim when a matching convention exists;
/// otherwise the resolver admits a registered `rastok.*` name. This newtype
/// enforces only the common canonical spelling. Registry membership remains a
/// separate, versioned judgement.
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

/// Where a registered semantic measure originates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MeasureSource {
    /// A typed field supplied by a provider or codec. These entries may be
    /// selected by source `@measure` annotations.
    Field,
    /// A runtime instrument produced by Rastok itself. Source fields cannot
    /// impersonate instruments merely by reusing their names.
    Instrument,
}

/// Logical carrier required by one registered measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MeasureCarrier {
    /// Exact integral count or byte quantity.
    Integer,
    /// Exact elapsed duration.
    Duration,
}

/// One entry of the pinned M1.8 semantic-measure registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeasureSpec {
    source: MeasureSource,
    carrier: MeasureCarrier,
}

impl MeasureSpec {
    /// Whether the value comes from a typed field or a runtime instrument.
    #[must_use]
    pub const fn source(self) -> MeasureSource {
        self.source
    }

    /// Logical value family required by the convention.
    #[must_use]
    pub const fn carrier(self) -> MeasureCarrier {
        self.carrier
    }
}

/// Looks up the pinned OpenTelemetry-first M1.8 registry.
///
/// The names and meanings are the snapshot recorded by D-2026-08-30-1.
/// Unknown canonical dotted names remain invalid rather than becoming
/// user-defined metrics by accident.
#[must_use]
pub fn measure_spec(id: &MeasureId) -> Option<MeasureSpec> {
    let (source, carrier) = match id.as_str() {
        "gen_ai.usage.input_tokens"
        | "gen_ai.usage.output_tokens"
        | "gen_ai.usage.cache_read.input_tokens"
        | "gen_ai.usage.cache_creation.input_tokens"
        | "gen_ai.usage.reasoning.output_tokens" => (MeasureSource::Field, MeasureCarrier::Integer),
        "gen_ai.client.token.usage"
        | "http.client.request.body.size"
        | "http.client.response.body.size"
        | "http.server.request.body.size"
        | "http.server.response.body.size" => (MeasureSource::Instrument, MeasureCarrier::Integer),
        "gen_ai.client.operation.duration"
        | "gen_ai.client.operation.time_to_first_chunk"
        | "gen_ai.client.operation.time_per_output_chunk"
        | "http.client.request.duration"
        | "http.server.request.duration" => (MeasureSource::Instrument, MeasureCarrier::Duration),
        _ => return None,
    };
    Some(MeasureSpec { source, carrier })
}

/// Exact monetary-field interpretation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CostAnnotation {
    currency: Option<CurrencyCode>,
    basis: CostBasis,
}

impl CostAnnotation {
    /// Cost on a scalar decimal field; the contract fixes its currency.
    #[must_use]
    pub const fn scalar(currency: CurrencyCode, basis: CostBasis) -> Self {
        Self {
            currency: Some(currency),
            basis,
        }
    }

    /// Cost on the canonical Money struct; the value carries its currency.
    #[must_use]
    pub const fn money(basis: CostBasis) -> Self {
        Self {
            currency: None,
            basis,
        }
    }

    /// Contract-fixed currency, absent for a Money-shaped value.
    #[must_use]
    pub const fn currency(&self) -> Option<&CurrencyCode> {
        self.currency.as_ref()
    }

    /// Evidence basis.
    #[must_use]
    pub const fn basis(&self) -> CostBasis {
        self.basis
    }
}

/// One closed M1.8 data annotation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DataAnnotation {
    /// Redaction and propagation classification.
    Sensitive(SensitivityLevel),
    /// Secret-handling marker. Secret origin remains the expression-level
    /// `secret("path")` function.
    Secret,
    /// Registered exact semantic quantity.
    Measure(MeasureId),
    /// Exact monetary interpretation.
    Cost(CostAnnotation),
}

impl DataAnnotation {
    /// Identity of this annotation within the closed family.
    #[must_use]
    pub const fn kind(&self) -> AnnotationKind {
        match self {
            Self::Sensitive(_) => AnnotationKind::Sensitive,
            Self::Secret => AnnotationKind::Secret,
            Self::Measure(_) => AnnotationKind::Measure,
            Self::Cost(_) => AnnotationKind::Cost,
        }
    }
}

/// Canonical identity order of the closed annotation family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AnnotationKind {
    /// `rastok/sensitive@1`.
    Sensitive,
    /// `rastok/secret@1`.
    Secret,
    /// `rastok/measure@1`.
    Measure,
    /// `rastok/cost@1`.
    Cost,
}

/// Canonically ordered annotations attached to one schema path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AnnotationSet(Vec<DataAnnotation>);

impl AnnotationSet {
    /// Sorts by canonical annotation identity and rejects duplicate kinds or
    /// the empty set.
    ///
    /// # Errors
    ///
    /// Returns the exact duplicate kind or [`SchemaAnnotationError::EmptySet`].
    pub fn new(mut annotations: Vec<DataAnnotation>) -> Result<Self, SchemaAnnotationError> {
        if annotations.is_empty() {
            return Err(SchemaAnnotationError::EmptySet);
        }
        annotations.sort_by_key(DataAnnotation::kind);
        if let Some(kind) = annotations
            .windows(2)
            .find_map(|pair| (pair[0].kind() == pair[1].kind()).then(|| pair[0].kind()))
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
}

/// One logical schema and its orthogonal policy table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotatedSchema {
    ty: DataType,
    annotations: AnnotationTable,
}

impl AnnotatedSchema {
    /// Verifies every sparse path against the adjacent logical type.
    ///
    /// # Errors
    ///
    /// Returns the first canonical path that cannot be traversed. Annotation
    /// target legality and registry membership are later checker/validator
    /// judgements; this constructor owns structural correspondence only.
    pub fn new(ty: DataType, annotations: AnnotationTable) -> Result<Self, SchemaAnnotationError> {
        for path in annotations.entries().keys() {
            validate_path(&ty, path)?;
        }
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

fn validate_path(ty: &DataType, path: &SchemaPath) -> Result<(), SchemaAnnotationError> {
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
    Ok(())
}

/// Invalid schema annotation metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaAnnotationError {
    /// No annotation belongs in a sparse table row.
    EmptySet,
    /// Two annotations of the same closed kind target one path.
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
}

impl fmt::Display for SchemaAnnotationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
        }
    }
}

impl core::error::Error for SchemaAnnotationError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datatype::{MapType, StructType, TaggedCase, TaggedUnionType, TypeField};
    use alloc::vec;

    fn sensitive(level: SensitivityLevel) -> AnnotationSet {
        AnnotationSet::new(vec![DataAnnotation::Sensitive(level)])
            .unwrap_or_else(|error| panic!("valid annotation set: {error}"))
    }

    #[test]
    fn annotations_are_canonical_but_do_not_wrap_the_logical_type() {
        let measure = DataAnnotation::Measure(
            MeasureId::new("gen_ai.usage.input_tokens")
                .unwrap_or_else(|error| panic!("valid measure: {error}")),
        );
        let annotations = AnnotationSet::new(vec![
            DataAnnotation::Secret,
            measure.clone(),
            DataAnnotation::Sensitive(SensitivityLevel::Restricted),
        ])
        .unwrap_or_else(|error| panic!("valid annotation set: {error}"));
        assert_eq!(
            annotations.annotations(),
            [
                DataAnnotation::Sensitive(SensitivityLevel::Restricted),
                DataAnnotation::Secret,
                measure,
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
    fn measure_registry_distinguishes_fields_from_runtime_instruments() {
        let field = MeasureId::new("gen_ai.usage.input_tokens")
            .unwrap_or_else(|error| panic!("valid field measure: {error}"));
        let instrument = MeasureId::new("http.client.request.duration")
            .unwrap_or_else(|error| panic!("valid instrument measure: {error}"));
        let unknown = MeasureId::new("rastok.unknown")
            .unwrap_or_else(|error| panic!("canonical unknown measure: {error}"));
        assert_eq!(
            measure_spec(&field),
            Some(MeasureSpec {
                source: MeasureSource::Field,
                carrier: MeasureCarrier::Integer,
            })
        );
        assert_eq!(
            measure_spec(&instrument),
            Some(MeasureSpec {
                source: MeasureSource::Instrument,
                carrier: MeasureCarrier::Duration,
            })
        );
        assert_eq!(measure_spec(&unknown), None);
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
    fn cost_annotation_separates_scalar_and_money_currency_authority() {
        let eur =
            CurrencyCode::new("EUR").unwrap_or_else(|error| panic!("valid currency: {error}"));
        let scalar = CostAnnotation::scalar(eur, CostBasis::ProviderReported);
        assert_eq!(scalar.currency().map(CurrencyCode::as_str), Some("EUR"));
        assert_eq!(scalar.basis(), CostBasis::ProviderReported);

        let money = CostAnnotation::money(CostBasis::PricebookCalculated);
        assert_eq!(money.currency(), None);
        assert_eq!(money.basis(), CostBasis::PricebookCalculated);
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
