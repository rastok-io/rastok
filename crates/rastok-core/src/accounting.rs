//! Individual typed measurements read from causal journal facts.
//!
//! Values remain in the original journaled envelopes and definitions in the
//! validated plan. Inspection neither aggregates samples nor calculates prices.
//! Deployment price-profile data retains its existing attested representation.

use crate::instance;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use crate::canonical_cbor::{Value as Cbor, encode};
use crate::envelope::{Id, Value};

use crate::journal::{Record, observations_for_records_from, recorded_envelope};
use crate::node;
use crate::observation::{ExchangeCorrelation, RecordCorrelation, RecordObservation};
use crate::plan::{OwnerKind, Slot, ValidatedExecutionPlan};
use crate::schema_annotation::{
    AnnotationTable, CurrencyCode, DataAnnotation, MeasureId, SchemaPath, SchemaPathSegment,
    is_iso_4217_currency,
};

/// Exact decimal quantity retained by measurements and deployment metadata.
///
/// Equivalent spellings normalize to one representation. The value remains
/// inside Rastok's 38-digit decimal domain and never crosses through a float.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExactDecimal {
    unscaled: i128,
    scale: i32,
}

impl ExactDecimal {
    /// Builds and numerically canonicalizes one exact decimal.
    ///
    /// # Errors
    ///
    /// Values outside Rastok's strict decimal domain refuse.
    pub fn new(unscaled: i128, scale: i32) -> Result<Self, AccountingError> {
        if !crate::datatype::decimal_in_domain(unscaled, scale) {
            return Err(AccountingError::DecimalOutOfDomain);
        }
        if unscaled == 0 {
            return Ok(Self {
                unscaled: 0,
                scale: 0,
            });
        }
        let mut unscaled = unscaled;
        let mut scale = scale;
        while scale > crate::datatype::DECIMAL_MIN_SCALE && unscaled % 10 == 0 {
            unscaled /= 10;
            scale -= 1;
        }
        Ok(Self { unscaled, scale })
    }

    /// Canonical coefficient.
    #[must_use]
    pub const fn unscaled(self) -> i128 {
        self.unscaled
    }

    /// Canonical decimal scale.
    #[must_use]
    pub const fn scale(self) -> i32 {
        self.scale
    }
}

impl fmt::Display for ExactDecimal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let digits = self.unscaled.unsigned_abs().to_string();
        if self.unscaled < 0 {
            formatter.write_str("-")?;
        }
        if self.scale <= 0 {
            formatter.write_str(&digits)?;
            for _ in self.scale..0 {
                formatter.write_str("0")?;
            }
        } else {
            let scale = self.scale as usize;
            if scale >= digits.len() {
                formatter.write_str("0.")?;
                for _ in digits.len()..scale {
                    formatter.write_str("0")?;
                }
                formatter.write_str(&digits)?;
            } else {
                let (integer, fraction) = digits.split_at(digits.len() - scale);
                write!(formatter, "{integer}.{fraction}")?;
            }
        }
        Ok(())
    }
}

/// Exact monetary metadata retained in an attested deployment tariff.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Money {
    amount: ExactDecimal,
    currency: CurrencyCode,
}

impl Money {
    /// Builds one exact monetary value under the pinned ISO registry.
    ///
    /// # Errors
    ///
    /// A value outside canonical `decimal(38,18)` or an unknown currency
    /// refuses.
    pub fn new(amount: ExactDecimal, currency: CurrencyCode) -> Result<Self, AccountingError> {
        if amount.scale > 18
            || !rescale(amount.unscaled, amount.scale, 18)
                .is_ok_and(|unscaled| crate::datatype::decimal_in_domain(unscaled, 18))
        {
            return Err(AccountingError::MoneyOutOfDomain);
        }
        if !is_iso_4217_currency(&currency) {
            return Err(AccountingError::UnknownCurrency);
        }
        Ok(Self { amount, currency })
    }

    /// Exact normalized amount.
    #[must_use]
    pub const fn amount(&self) -> ExactDecimal {
        self.amount
    }

    /// ISO currency retained with this individual amount.
    #[must_use]
    pub const fn currency(&self) -> &CurrencyCode {
        &self.currency
    }
}

/// Stable deployment price-profile identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CostProfileName(String);

impl CostProfileName {
    /// Builds one bounded, whitespace-free stable reference.
    ///
    /// # Errors
    ///
    /// Empty, overlong, control-bearing or whitespace-bearing references
    /// refuse.
    pub fn new(reference: impl Into<String>) -> Result<Self, AccountingError> {
        let reference = reference.into();
        if !stable_text(&reference, 512, false) {
            return Err(AccountingError::InvalidProfileRef(reference));
        }
        Ok(Self(reference))
    }

    /// Canonical spelling.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One exact deployment price for one Connector and semantic measure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitPrice {
    connector: node::Path,
    provider: String,
    model: String,
    measure: MeasureId,
    units: u64,
    price: Money,
}

impl UnitPrice {
    /// Builds one price such as USD 2.50 per one million input tokens.
    ///
    /// `units` is restricted to a positive power of ten. That keeps division
    /// exact by increasing decimal scale; a rule which would require more
    /// than the canonical Money scale refuses instead of rounding.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        connector: node::Path,
        provider: impl Into<String>,
        model: impl Into<String>,
        measure: MeasureId,
        units: u64,
        amount: ExactDecimal,
        currency: CurrencyCode,
    ) -> Result<Self, AccountingError> {
        let provider = provider.into();
        let model = model.into();
        if !stable_dimension(&provider, 255) {
            return Err(AccountingError::InvalidDimension("provider"));
        }
        if !stable_dimension(&model, 255) {
            return Err(AccountingError::InvalidDimension("model"));
        }
        let Some(power) = decimal_power(units) else {
            return Err(AccountingError::InvalidPriceUnits);
        };
        if amount.unscaled < 0 {
            return Err(AccountingError::NegativeUnitPrice);
        }
        if amount
            .scale
            .checked_add(power)
            .is_none_or(|scale| scale > 18)
        {
            return Err(AccountingError::PriceRequiresRounding);
        }
        Ok(Self {
            connector,
            provider,
            model,
            measure,
            units,
            price: Money::new(amount, currency)?,
        })
    }

    /// Connector named by this retained tariff metadata.
    #[must_use]
    pub const fn connector(&self) -> &node::Path {
        &self.connector
    }

    /// Provider dimension declared by deployment.
    #[must_use]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Model/SKU dimension declared by deployment.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Semantic quantity priced by this row.
    #[must_use]
    pub const fn measure(&self) -> &MeasureId {
        &self.measure
    }

    /// Quantity represented by [`Self::price_money`].
    #[must_use]
    pub const fn units(&self) -> u64 {
        self.units
    }

    /// Exact unit-bundle price.
    #[must_use]
    pub const fn price_money(&self) -> &Money {
        &self.price
    }
}

/// Canonical optional deployment price book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostProfile {
    reference: CostProfileName,
    rules: Vec<UnitPrice>,
    digest: [u8; 32],
}

impl CostProfile {
    /// Sorts and seals one closed profile.
    ///
    /// # Errors
    ///
    /// A Connector/measure pair has exactly one rule, and every rule for one
    /// Connector must agree on provider/model dimensions.
    pub fn new(
        reference: CostProfileName,
        mut rules: Vec<UnitPrice>,
    ) -> Result<Self, AccountingError> {
        rules.sort_by(|left, right| {
            (left.connector(), left.measure()).cmp(&(right.connector(), right.measure()))
        });
        if rules.windows(2).any(|pair| {
            pair[0].connector == pair[1].connector && pair[0].measure == pair[1].measure
        }) {
            return Err(AccountingError::DuplicatePriceRule);
        }
        let mut dimensions = BTreeMap::<node::Path, (&str, &str)>::new();
        for rule in &rules {
            match dimensions.get(rule.connector()) {
                Some((provider, model))
                    if *provider != rule.provider() || *model != rule.model() =>
                {
                    return Err(AccountingError::ConflictingConnectorDimensions);
                }
                Some(_) => {}
                None => {
                    dimensions.insert(rule.connector.clone(), (rule.provider(), rule.model()));
                }
            }
        }
        let tree = Cbor::Array(alloc::vec![
            Cbor::Text(reference.as_str().to_string()),
            Cbor::Array(rules.iter().map(unit_price_tree).collect()),
        ]);
        let digest = *blake3::hash(&encode(&tree)).as_bytes();
        Ok(Self {
            reference,
            rules,
            digest,
        })
    }

    /// Stable profile reference.
    #[must_use]
    pub const fn reference(&self) -> &CostProfileName {
        &self.reference
    }

    /// Digest of every price and dimension used by this profile.
    #[must_use]
    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    /// Canonically ordered rules.
    #[must_use]
    pub fn rules(&self) -> &[UnitPrice] {
        &self.rules
    }

    fn dimensions(&self, connector: Option<&node::Path>) -> Option<(&str, &str)> {
        let connector = connector?;
        self.rules
            .iter()
            .find(|rule| rule.connector() == connector)
            .map(|rule| (rule.provider(), rule.model()))
    }
}

/// One exact semantic quantity extracted from an annotated envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasureSample {
    path: SchemaPath,
    id: MeasureId,
    quantity: ExactDecimal,
    unit: String,
}

impl MeasureSample {
    /// Canonical schema position which supplied the quantity.
    #[must_use]
    pub const fn path(&self) -> &SchemaPath {
        &self.path
    }

    /// Author-chosen quantity id.
    #[must_use]
    pub const fn id(&self) -> &MeasureId {
        &self.id
    }

    /// Exact sample; business quantities retain their sign.
    #[must_use]
    pub const fn quantity(&self) -> ExactDecimal {
        self.quantity
    }

    /// Exact unit fixed by the validated measurement definition.
    #[must_use]
    pub fn unit(&self) -> &str {
        &self.unit
    }
}

/// Causal provenance of one retained measurement, without grouping.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObservationScope {
    origin_id: Option<Id>,
    instance: Option<instance::Generation>,
    owner: Option<node::OwnerPath>,
    node: Option<node::Path>,
    connector: Option<node::Path>,
    port: node::Port,
    exchange: Option<ExchangeCorrelation>,
    provider: Option<String>,
    model: Option<String>,
}

impl ObservationScope {
    /// Root flow identity.
    #[must_use]
    pub const fn origin_id(&self) -> Option<Id> {
        self.origin_id
    }

    /// Exact logical instance generation.
    #[must_use]
    pub const fn instance(&self) -> Option<&instance::Generation> {
        self.instance.as_ref()
    }

    /// Transaction owner.
    #[must_use]
    pub const fn owner(&self) -> Option<&node::OwnerPath> {
        self.owner.as_ref()
    }

    /// Executable Node at this point, if any.
    #[must_use]
    pub const fn node(&self) -> Option<&node::Path> {
        self.node.as_ref()
    }

    /// Nearest causal Connector which supplied this exact measured value.
    #[must_use]
    pub const fn connector(&self) -> Option<&node::Path> {
        self.connector.as_ref()
    }

    /// Exact flow port.
    #[must_use]
    pub const fn port(&self) -> &node::Port {
        &self.port
    }

    /// Typed Call/Session correlation.
    #[must_use]
    pub const fn exchange(&self) -> Option<&ExchangeCorrelation> {
        self.exchange.as_ref()
    }

    /// Deployment provider dimension, when a profile owns it.
    #[must_use]
    pub fn provider(&self) -> Option<&str> {
        self.provider.as_deref()
    }

    /// Deployment model/SKU dimension, when a profile owns it.
    #[must_use]
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }
}

/// Accounting facts observed at one exact envelope/port point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountingObservation {
    record: RecordObservation,
    envelope_id: Id,
    scope: ObservationScope,
    measures: Vec<MeasureSample>,
}

impl AccountingObservation {
    /// Structural transition and correlation from the A5 retained stream.
    #[must_use]
    pub const fn record(&self) -> &RecordObservation {
        &self.record
    }

    /// Causal envelope whose typed value supplied these facts.
    #[must_use]
    pub const fn envelope_id(&self) -> Id {
        self.envelope_id
    }

    /// Complete per-flow dimensions.
    #[must_use]
    pub const fn scope(&self) -> &ObservationScope {
        &self.scope
    }

    /// Individual samples in canonical schema-path order.
    #[must_use]
    pub fn measures(&self) -> &[MeasureSample] {
        &self.measures
    }
}

/// Derives typed accounting observations from causal records and the
/// independently validated effective schemas.
pub fn accounting_observations(
    plan: &ValidatedExecutionPlan,
    records: &[Record],
    profile: Option<&CostProfile>,
) -> Result<Vec<AccountingObservation>, AccountingError> {
    let structural = observations_for_records_from(records, 0);
    let mut result = Vec::new();
    for (index, (record, observation)) in records.iter().zip(structural).enumerate() {
        // Attribution is a causal fold: a later record may never explain an
        // envelope which is being observed now, even in a hostile record set.
        let committed = &records[..=index];
        match record {
            Record::Ingress(inbound) => {
                let schema = inbound_schema(plan, inbound.connector_path.as_ref(), &inbound.port)?;
                let lineage_connector =
                    nearest_causal_connector(plan, committed, &inbound.envelope);
                observe_envelope(
                    &mut result,
                    observation.clone(),
                    &inbound.envelope,
                    schema.annotations,
                    scope_for(
                        observation.correlation(),
                        schema.node,
                        schema.connector,
                        lineage_connector.as_ref(),
                        inbound.port.clone(),
                        profile,
                    ),
                )?;
            }
            Record::Transaction(transaction) => {
                for emission in &transaction.emissions {
                    let schema = transaction_schema(plan, &transaction.owner_path, &emission.port)?;
                    let lineage_connector =
                        nearest_causal_connector(plan, committed, &emission.envelope);
                    observe_envelope(
                        &mut result,
                        observation.clone(),
                        &emission.envelope,
                        schema.annotations,
                        scope_for(
                            observation.correlation(),
                            schema.node,
                            schema.connector,
                            lineage_connector.as_ref(),
                            emission.port.clone(),
                            profile,
                        ),
                    )?;
                }
            }
            Record::Egress(_) => {}
        }
    }
    Ok(result)
}

#[derive(Clone, Copy)]
struct PortSchema<'a> {
    annotations: &'a AnnotationTable,
    node: Option<&'a node::Path>,
    connector: Option<&'a node::Path>,
}

fn inbound_schema<'a>(
    plan: &'a ValidatedExecutionPlan,
    connector: Option<&node::Path>,
    port: &node::Port,
) -> Result<PortSchema<'a>, AccountingError> {
    if let Some(connector) = connector {
        let (index, planned) = plan
            .plan()
            .nodes
            .iter()
            .enumerate()
            .find(|(_, candidate)| &candidate.path == connector)
            .ok_or(AccountingError::UnknownFlowSchema)?;
        let slot = Slot::new(u32::try_from(index).map_err(|_| AccountingError::UnknownFlowSchema)?);
        let annotations = plan
            .resolved_ports()
            .node(slot)
            .and_then(|ports| ports.iter().find(|candidate| candidate.port() == port))
            .map(crate::plan::ResolvedPort::annotations)
            .ok_or(AccountingError::UnknownFlowSchema)?;
        return Ok(PortSchema {
            annotations,
            node: Some(&planned.path),
            connector: planned.connector().map(|_| &planned.path),
        });
    }
    let annotations = plan
        .resolved_ports()
        .boundary()
        .iter()
        .find(|candidate| candidate.port() == port)
        .map(crate::plan::ResolvedPort::annotations)
        .ok_or(AccountingError::UnknownFlowSchema)?;
    Ok(PortSchema {
        annotations,
        node: None,
        connector: None,
    })
}

fn transaction_schema<'a>(
    plan: &'a ValidatedExecutionPlan,
    owner_path: &node::OwnerPath,
    port: &node::Port,
) -> Result<PortSchema<'a>, AccountingError> {
    let owner = plan
        .plan()
        .owners
        .iter()
        .find(|owner| &owner.path == owner_path)
        .ok_or(AccountingError::UnknownFlowSchema)?;
    match owner.kind {
        OwnerKind::Node(slot) => {
            let planned = plan
                .plan()
                .nodes
                .get(slot.index())
                .ok_or(AccountingError::UnknownFlowSchema)?;
            let annotations = plan
                .resolved_ports()
                .node(slot)
                .and_then(|ports| ports.iter().find(|candidate| candidate.port() == port))
                .map(crate::plan::ResolvedPort::annotations)
                .ok_or(AccountingError::UnknownFlowSchema)?;
            Ok(PortSchema {
                annotations,
                node: Some(&planned.path),
                connector: planned.connector().map(|_| &planned.path),
            })
        }
        OwnerKind::Graph if owner.path.is_root() => {
            let annotations = plan
                .resolved_ports()
                .boundary()
                .iter()
                .find(|candidate| candidate.port() == port)
                .map(crate::plan::ResolvedPort::annotations)
                .ok_or(AccountingError::UnknownFlowSchema)?;
            Ok(PortSchema {
                annotations,
                node: None,
                connector: None,
            })
        }
        OwnerKind::Graph => {
            let annotations = owner
                .ports
                .iter()
                .find(|candidate| &candidate.port == port)
                .map(|candidate| &candidate.annotations)
                .ok_or(AccountingError::UnknownFlowSchema)?;
            Ok(PortSchema {
                annotations,
                node: None,
                connector: None,
            })
        }
    }
}

fn scope_for(
    correlation: &RecordCorrelation,
    node: Option<&node::Path>,
    connector: Option<&node::Path>,
    lineage_connector: Option<&node::Path>,
    port: node::Port,
    profile: Option<&CostProfile>,
) -> ObservationScope {
    let connector = connector
        .or_else(|| correlation.connector())
        .or(lineage_connector);
    let (provider, model) = profile
        .and_then(|profile| profile.dimensions(connector))
        .map_or((None, None), |(provider, model)| {
            (Some(provider.to_string()), Some(model.to_string()))
        });
    ObservationScope {
        origin_id: correlation.origin_id(),
        instance: correlation.instance().cloned(),
        owner: correlation.owner().cloned(),
        node: node.cloned(),
        connector: connector.cloned(),
        port,
        exchange: correlation.exchange().cloned(),
        provider,
        model,
    }
}

fn nearest_causal_connector(
    plan: &ValidatedExecutionPlan,
    records: &[Record],
    envelope: &crate::envelope::Envelope,
) -> Option<node::Path> {
    let mut cursor = Some(envelope.meta().sys().id());
    let mut seen = BTreeSet::new();
    while let Some(id) = cursor {
        if !seen.insert(id) {
            return None;
        }
        if let Some(connector) = connector_which_emitted(plan, records, id) {
            return Some(connector);
        }
        cursor =
            recorded_envelope(records, id).and_then(|recorded| recorded.meta().sys().parent_id());
    }
    None
}

fn connector_which_emitted(
    plan: &ValidatedExecutionPlan,
    records: &[Record],
    envelope: Id,
) -> Option<node::Path> {
    records.iter().rev().find_map(|record| match record {
        Record::Ingress(inbound) if inbound.envelope.meta().sys().id() == envelope => {
            inbound.connector_path.clone()
        }
        Record::Transaction(transaction)
            if transaction
                .emissions
                .iter()
                .any(|emission| emission.envelope.meta().sys().id() == envelope) =>
        {
            let owner = plan
                .plan()
                .owners
                .iter()
                .find(|owner| owner.path == transaction.owner_path)?;
            let OwnerKind::Node(slot) = owner.kind else {
                return None;
            };
            let planned = plan.plan().nodes.get(slot.index())?;
            planned.connector().map(|_| planned.path.clone())
        }
        Record::Ingress(_) | Record::Transaction(_) | Record::Egress(_) => None,
    })
}

fn observe_envelope(
    result: &mut Vec<AccountingObservation>,
    record: RecordObservation,
    envelope: &crate::envelope::Envelope,
    annotations: &AnnotationTable,
    scope: ObservationScope,
) -> Result<(), AccountingError> {
    if annotations.is_empty() {
        return Ok(());
    }
    let value = envelope.data().materialized_value().ok_or(
        AccountingError::UnmaterializedAnnotatedEnvelope(envelope.meta().sys().id()),
    )?;
    let mut measures = Vec::new();
    for (path, set) in annotations.entries() {
        let values = values_at_path(value, path)?;
        for selected in values {
            for annotation in set.annotations() {
                match annotation {
                    DataAnnotation::Declared(application) => match application.binding() {
                        crate::schema_annotation::AnnotationBinding::Metadata => {}
                        crate::schema_annotation::AnnotationBinding::Measure => {
                            let (id, unit) = application
                                .quantity_measure()
                                .ok_or(AccountingError::ConflictingMeasureDefinition)?;
                            let quantity = match selected {
                                Value::Int(value) => ExactDecimal::new(i128::from(*value), 0)?,
                                Value::Uint(value) => ExactDecimal::new(i128::from(*value), 0)?,
                                Value::Decimal { unscaled, scale } => {
                                    ExactDecimal::new(*unscaled, *scale)?
                                }
                                _ => return Err(AccountingError::MeasureCarrierChanged),
                            };
                            measures.push(MeasureSample {
                                path: path.clone(),
                                id: MeasureId::new(id)
                                    .map_err(|_| AccountingError::ConflictingMeasureDefinition)?,
                                quantity,
                                unit: unit.into(),
                            });
                        }
                    },
                    DataAnnotation::Sensitive(_)
                    | DataAnnotation::Secret
                    | DataAnnotation::Personal(_) => {}
                }
            }
        }
    }
    if !measures.is_empty() {
        result.push(AccountingObservation {
            record,
            envelope_id: envelope.meta().sys().id(),
            scope,
            measures,
        });
    }
    Ok(())
}

enum ValueCursor<'a> {
    Value(&'a Value),
    Fields(&'a [(String, Value)]),
}

fn values_at_path<'a>(
    root: &'a Value,
    path: &SchemaPath,
) -> Result<Vec<&'a Value>, AccountingError> {
    let mut cursors = alloc::vec![ValueCursor::Value(root)];
    for segment in path.segments() {
        let mut next = Vec::new();
        for cursor in cursors {
            match (cursor, segment) {
                (ValueCursor::Value(Value::Struct(fields)), SchemaPathSegment::Field(name)) => {
                    if let Some((_, value)) = fields.iter().find(|(field, _)| field == name) {
                        next.push(ValueCursor::Value(value));
                    }
                }
                (ValueCursor::Fields(fields), SchemaPathSegment::Field(name)) => {
                    if let Some((_, value)) = fields.iter().find(|(field, _)| field == name) {
                        next.push(ValueCursor::Value(value));
                    }
                }
                (ValueCursor::Value(Value::Array(values)), SchemaPathSegment::ArrayElement) => {
                    next.extend(values.iter().map(ValueCursor::Value));
                }
                (ValueCursor::Value(Value::Map(values)), SchemaPathSegment::MapValue) => {
                    next.extend(values.values().map(ValueCursor::Value));
                }
                (ValueCursor::Value(Value::KeyedMap(values)), SchemaPathSegment::MapValue) => {
                    next.extend(values.iter().map(|(_, value)| ValueCursor::Value(value)));
                }
                (ValueCursor::Value(Value::KeyedMap(values)), SchemaPathSegment::MapKey) => {
                    next.extend(values.iter().map(|(key, _)| ValueCursor::Value(key)));
                }
                (
                    ValueCursor::Value(Value::StructuralUnion { branch, value }),
                    SchemaPathSegment::StructuralUnionBranch(expected),
                ) if branch == expected => next.push(ValueCursor::Value(value)),
                (
                    ValueCursor::Value(Value::TaggedUnion(tagged)),
                    SchemaPathSegment::TaggedUnionCase(expected),
                ) if &tagged.case == expected => next.push(ValueCursor::Fields(&tagged.payload)),
                _ => {}
            }
        }
        cursors = next;
    }
    cursors
        .into_iter()
        .map(|cursor| match cursor {
            ValueCursor::Value(value) => Ok(value),
            ValueCursor::Fields(_) => Err(AccountingError::AnnotationTargetsUnionCase),
        })
        .collect()
}

fn rescale(unscaled: i128, from: i32, to: i32) -> Result<i128, AccountingError> {
    let difference = to
        .checked_sub(from)
        .ok_or(AccountingError::DecimalOverflow)?;
    let power = u32::try_from(difference).map_err(|_| AccountingError::DecimalOverflow)?;
    let factor = 10_i128
        .checked_pow(power)
        .ok_or(AccountingError::DecimalOverflow)?;
    unscaled
        .checked_mul(factor)
        .ok_or(AccountingError::DecimalOverflow)
}

fn decimal_power(mut units: u64) -> Option<i32> {
    if units == 0 {
        return None;
    }
    let mut power = 0_i32;
    while units > 1 {
        if !units.is_multiple_of(10) {
            return None;
        }
        units /= 10;
        power = power.checked_add(1)?;
    }
    Some(power)
}

fn stable_text(value: &str, max: usize, allow_space: bool) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .chars()
            .all(|character| !character.is_control() && (allow_space || !character.is_whitespace()))
}

fn stable_dimension(value: &str, max: usize) -> bool {
    stable_text(value, max, true) && value.trim() == value
}

fn unit_price_tree(rule: &UnitPrice) -> Cbor {
    Cbor::Array(alloc::vec![
        Cbor::Array(
            rule.connector()
                .segments()
                .iter()
                .cloned()
                .map(Cbor::Text)
                .collect(),
        ),
        Cbor::Text(rule.provider().to_string()),
        Cbor::Text(rule.model().to_string()),
        Cbor::Text(rule.measure().as_str().to_string()),
        Cbor::Uint(rule.units()),
        crate::canonical::decimal(
            rule.price_money().amount().unscaled(),
            rule.price_money().amount().scale(),
        ),
        Cbor::Text(rule.price_money().currency().as_str().to_string()),
    ])
}

/// Exact accounting refusal. No variant contains payload or secret material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountingError {
    /// Invalid stable profile reference.
    InvalidProfileRef(String),
    /// Provider/model deployment dimension is empty or malformed.
    InvalidDimension(&'static str),
    /// Duplicate Connector/measure rule.
    DuplicatePriceRule,
    /// One Connector was assigned two provider/model identities.
    ConflictingConnectorDimensions,
    /// Price denominator is not a positive power of ten.
    InvalidPriceUnits,
    /// A unit price cannot be negative.
    NegativeUnitPrice,
    /// Exact calculation would require rounding beyond Money's scale.
    PriceRequiresRounding,
    /// Decimal value lies outside the strict domain.
    DecimalOutOfDomain,
    /// Exact amount cannot inhabit canonical `decimal(38,18)` Money.
    MoneyOutOfDomain,
    /// Exact decimal arithmetic overflowed.
    DecimalOverflow,
    /// A measure changed carrier under one semantic id.
    MeasureCarrierChanged,
    /// One id names incompatible units or interpretations.
    ConflictingMeasureDefinition,
    /// Currency is absent from the pinned ISO registry.
    UnknownCurrency,
    /// Annotated data was retained only as undecoded bytes.
    UnmaterializedAnnotatedEnvelope(Id),
    /// Journal flow could not be reconciled with a resolved port schema.
    UnknownFlowSchema,
    /// Accounting cannot target a tagged case without a value field.
    AnnotationTargetsUnionCase,
}

impl fmt::Display for AccountingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProfileRef(reference) => {
                write!(formatter, "invalid cost-profile reference `{reference}`")
            }
            Self::InvalidDimension(name) => write!(formatter, "invalid cost-profile {name}"),
            Self::DuplicatePriceRule => {
                formatter.write_str("a cost profile repeats one Connector/measure rule")
            }
            Self::ConflictingConnectorDimensions => formatter
                .write_str("cost-profile rules disagree on one Connector's provider or model"),
            Self::InvalidPriceUnits => {
                formatter.write_str("price units must be a positive power of ten")
            }
            Self::NegativeUnitPrice => formatter.write_str("a unit price cannot be negative"),
            Self::PriceRequiresRounding => {
                formatter.write_str("the exact price would require rounding beyond decimal(38,18)")
            }
            Self::DecimalOutOfDomain => formatter.write_str("decimal value is out of domain"),
            Self::MoneyOutOfDomain => {
                formatter.write_str("money amount is outside canonical decimal(38,18)")
            }
            Self::DecimalOverflow => formatter.write_str("exact decimal arithmetic overflowed"),
            Self::MeasureCarrierChanged => {
                formatter.write_str("one measure has an incompatible carrier")
            }
            Self::ConflictingMeasureDefinition => {
                formatter.write_str("one measure id has inconsistent units or interpretations")
            }
            Self::UnknownCurrency => formatter.write_str("unknown ISO currency"),
            Self::UnmaterializedAnnotatedEnvelope(id) => write!(
                formatter,
                "annotated envelope `{id}` has no materialized value for exact accounting"
            ),
            Self::UnknownFlowSchema => {
                formatter.write_str("a journal flow has no matching resolved port schema")
            }
            Self::AnnotationTargetsUnionCase => {
                formatter.write_str("an accounting annotation cannot target a bare union case")
            }
        }
    }
}

impl core::error::Error for AccountingError {}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use alloc::vec;

    #[test]
    fn signed_decimal_quantities_format_without_binary_rounding() {
        for (unscaled, scale, text) in [
            (0, 18, "0"),
            (1250, 3, "1.25"),
            (-1250, 3, "-1.25"),
            (-1, 4, "-0.0001"),
            (42, -3, "42000"),
        ] {
            assert_eq!(
                ExactDecimal::new(unscaled, scale).unwrap().to_string(),
                text
            );
        }
    }
    fn connector() -> node::Path {
        node::Path::new(["model"]).unwrap_or_else(|error| panic!("path: {error}"))
    }

    fn profile(prices: &[(&str, u64, i128, i32, &str)]) -> CostProfile {
        let rules = prices
            .iter()
            .map(|(id, units, unscaled, scale, currency)| {
                UnitPrice::new(
                    connector(),
                    "openai",
                    "gpt-5",
                    MeasureId::new(*id).unwrap_or_else(|error| panic!("measure id: {error}")),
                    *units,
                    ExactDecimal::new(*unscaled, *scale)
                        .unwrap_or_else(|error| panic!("decimal: {error}")),
                    CurrencyCode::new(*currency)
                        .unwrap_or_else(|error| panic!("currency: {error}")),
                )
                .unwrap_or_else(|error| panic!("unit price: {error}"))
            })
            .collect();
        CostProfile::new(
            CostProfileName::new("pricing.example/test@1")
                .unwrap_or_else(|error| panic!("profile ref: {error}")),
            rules,
        )
        .unwrap_or_else(|error| panic!("profile: {error}"))
    }

    #[test]
    fn decimal_values_normalize_without_float() {
        let one = ExactDecimal::new(100, 2).unwrap_or_else(|error| panic!("decimal: {error}"));
        assert_eq!(one, ExactDecimal::new(1, 0).unwrap());
        assert_eq!(
            ExactDecimal::new(10, -39).unwrap(),
            ExactDecimal::new(1, -40).unwrap()
        );
    }

    #[test]
    fn price_dimensions_are_stable_not_whitespace_dependent() {
        let price = UnitPrice::new(
            connector(),
            " openai",
            "gpt-5",
            MeasureId::new("input_tokens").unwrap(),
            1,
            ExactDecimal::new(1, 0).unwrap(),
            CurrencyCode::new("USD").unwrap(),
        );
        assert_eq!(price, Err(AccountingError::InvalidDimension("provider")));
    }

    #[test]
    fn money_accepts_only_values_representable_as_decimal_38_18() {
        let usd = CurrencyCode::new("USD").unwrap();
        assert!(Money::new(ExactDecimal::new(1, 18).unwrap(), usd.clone()).is_ok());
        assert_eq!(
            Money::new(ExactDecimal::new(1, 19).unwrap(), usd.clone()),
            Err(AccountingError::MoneyOutOfDomain)
        );
        assert_eq!(
            Money::new(ExactDecimal::new(1, -38).unwrap(), usd),
            Err(AccountingError::MoneyOutOfDomain)
        );
    }

    #[test]
    fn retained_profile_dimensions_belong_to_the_named_connector() {
        let matching = profile(&[("input_tokens", 1, 1, 0, "USD")]);
        assert_eq!(
            matching.dimensions(Some(&connector())),
            Some(("openai", "gpt-5"))
        );
        let unrelated = node::Path::new(["other"]).unwrap();
        assert_eq!(matching.dimensions(Some(&unrelated)), None);
        assert_eq!(matching.dimensions(None), None);
    }

    #[test]
    fn collection_and_tagged_case_paths_select_only_live_values() {
        let value = Value::TaggedUnion(Box::new(crate::value::TaggedUnionValue {
            case: String::from("Usage"),
            payload: vec![(
                String::from("tokens"),
                Value::Array(vec![Value::Uint(2), Value::Uint(3)]),
            )],
        }));
        let path = SchemaPath::new(vec![
            SchemaPathSegment::TaggedUnionCase(String::from("Usage")),
            SchemaPathSegment::Field(String::from("tokens")),
            SchemaPathSegment::ArrayElement,
        ]);
        assert_eq!(
            values_at_path(&value, &path).unwrap(),
            vec![&Value::Uint(2), &Value::Uint(3)]
        );
        let dead = SchemaPath::new(vec![SchemaPathSegment::TaggedUnionCase(String::from(
            "Other",
        ))]);
        assert!(values_at_path(&value, &dead).unwrap().is_empty());
    }
}
