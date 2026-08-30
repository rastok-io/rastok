//! Exact, replayable measure and cost folds over causal journal facts.
//!
//! The journal remains the authority: this module derives typed accounting
//! observations lazily from immutable envelopes and resolved schema policy.
//! Exporters neither feed nor amend this fold.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use crate::canonical_cbor::{Value as Cbor, encode};
use crate::deployment::CostProfileAttestation;
use crate::envelope::{Id, Value};
use crate::identity::InstanceRef;
use crate::journal::{
    Journal, MemJournal, Record, observations_for_records_from, recorded_envelope,
};
use crate::node;
use crate::observation::{ExchangeCorrelation, RecordCorrelation, RecordObservation};
use crate::plan::{OwnerKind, OwnerPath, Slot, ValidatedExecutionPlan};
use crate::schema_annotation::{
    AnnotationTable, CostAnnotation, CostBasis, CurrencyCode, DataAnnotation, MeasureCarrier,
    MeasureId, MeasureUnit, SchemaPath, SchemaPathSegment, is_iso_4217_currency, measure_spec,
};

/// Exact decimal quantity used by monetary reports and deployment prices.
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

    fn checked_add(self, other: Self) -> Result<Self, AccountingError> {
        let target_scale = self.scale.max(other.scale);
        let left = rescale(self.unscaled, self.scale, target_scale)?;
        let right = rescale(other.unscaled, other.scale, target_scale)?;
        Self::new(
            left.checked_add(right)
                .ok_or(AccountingError::DecimalOverflow)?,
            target_scale,
        )
    }

    fn price(self, quantity: u128, units: u64) -> Result<Self, AccountingError> {
        let quantity = i128::try_from(quantity).map_err(|_| AccountingError::DecimalOverflow)?;
        let unscaled = self
            .unscaled
            .checked_mul(quantity)
            .ok_or(AccountingError::DecimalOverflow)?;
        let divisor_scale = decimal_power(units).ok_or(AccountingError::InvalidPriceUnits)?;
        let scale = self
            .scale
            .checked_add(divisor_scale)
            .ok_or(AccountingError::DecimalOverflow)?;
        if scale > 18 {
            return Err(AccountingError::PriceRequiresRounding);
        }
        Self::new(unscaled, scale)
    }
}

/// One exact amount and its sole currency authority.
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

    /// ISO currency; reports never combine another code with it.
    #[must_use]
    pub const fn currency(&self) -> &CurrencyCode {
        &self.currency
    }
}

/// Stable deployment price-profile identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CostProfileRef(String);

impl CostProfileRef {
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
        if measure_spec(&measure).is_none() {
            return Err(AccountingError::UnknownMeasure(
                measure.as_str().to_string(),
            ));
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

    /// Connector whose observations this rule prices.
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
    reference: CostProfileRef,
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
        reference: CostProfileRef,
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
    pub const fn reference(&self) -> &CostProfileRef {
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

    fn rule(&self, connector: Option<&node::Path>, measure: &MeasureId) -> Option<&UnitPrice> {
        let connector = connector?;
        self.rules.iter().find(|rule| {
            rule.connector() == connector && rule.measure().as_str() == measure.as_str()
        })
    }

    fn dimensions(&self, connector: Option<&node::Path>) -> Option<(&str, &str)> {
        let connector = connector?;
        self.rules
            .iter()
            .find(|rule| rule.connector() == connector)
            .map(|rule| (rule.provider(), rule.model()))
    }
}

/// Exact additive quantity carried by one measure sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MeasureQuantity {
    /// Non-negative exact count, token count or byte count.
    Integer(u128),
    /// Non-negative exact elapsed nanoseconds.
    DurationNanoseconds(u128),
}

impl MeasureQuantity {
    /// Numeric magnitude in the unit fixed by the registry.
    #[must_use]
    pub const fn magnitude(self) -> u128 {
        match self {
            Self::Integer(value) | Self::DurationNanoseconds(value) => value,
        }
    }

    fn checked_add(self, other: Self) -> Result<Self, AccountingError> {
        match (self, other) {
            (Self::Integer(left), Self::Integer(right)) => left
                .checked_add(right)
                .map(Self::Integer)
                .ok_or(AccountingError::MeasureOverflow),
            (Self::DurationNanoseconds(left), Self::DurationNanoseconds(right)) => left
                .checked_add(right)
                .map(Self::DurationNanoseconds)
                .ok_or(AccountingError::MeasureOverflow),
            _ => Err(AccountingError::MeasureCarrierChanged),
        }
    }

    fn checked_sub(self, other: Self) -> Result<Self, AccountingError> {
        match (self, other) {
            (Self::Integer(left), Self::Integer(right)) => left
                .checked_sub(right)
                .map(Self::Integer)
                .ok_or(AccountingError::SubsetExceedsTotal),
            (Self::DurationNanoseconds(left), Self::DurationNanoseconds(right)) => left
                .checked_sub(right)
                .map(Self::DurationNanoseconds)
                .ok_or(AccountingError::SubsetExceedsTotal),
            _ => Err(AccountingError::MeasureCarrierChanged),
        }
    }
}

/// One exact semantic quantity extracted from an annotated envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasureSample {
    path: SchemaPath,
    id: MeasureId,
    quantity: MeasureQuantity,
    unit: MeasureUnit,
    subset_of: Option<MeasureId>,
}

impl MeasureSample {
    /// Canonical schema position which supplied the quantity.
    #[must_use]
    pub const fn path(&self) -> &SchemaPath {
        &self.path
    }

    /// Registered semantic quantity.
    #[must_use]
    pub const fn id(&self) -> &MeasureId {
        &self.id
    }

    /// Exact non-negative sample.
    #[must_use]
    pub const fn quantity(&self) -> MeasureQuantity {
        self.quantity
    }

    /// Canonical unit fixed by the registry.
    #[must_use]
    pub const fn unit(&self) -> MeasureUnit {
        self.unit
    }

    /// Inclusive parent quantity, when this measure is a subset.
    #[must_use]
    pub const fn subset_of(&self) -> Option<&MeasureId> {
        self.subset_of.as_ref()
    }
}

/// Evidence attached to an exact monetary sample.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CostEvidence {
    /// The boundary/provider supplied the amount.
    ProviderReported,
    /// A complete deployment price profile calculated or attested it.
    PricebookCalculated {
        /// Stable profile identity.
        profile: CostProfileRef,
        /// Digest of every rule in force.
        digest: [u8; 32],
    },
}

impl CostEvidence {
    /// Accounting basis corresponding to this evidence.
    #[must_use]
    pub const fn basis(&self) -> CostBasis {
        match self {
            Self::ProviderReported => CostBasis::ProviderReported,
            Self::PricebookCalculated { .. } => CostBasis::PricebookCalculated,
        }
    }
}

/// One exact monetary fact extracted or calculated outside causal identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostSample {
    path: Option<SchemaPath>,
    money: Money,
    evidence: CostEvidence,
    calculated_from: Option<MeasureId>,
}

impl CostSample {
    /// Schema position for a reported field; absent for a profile calculation.
    #[must_use]
    pub const fn path(&self) -> Option<&SchemaPath> {
        self.path.as_ref()
    }

    /// Exact amount and currency.
    #[must_use]
    pub const fn money(&self) -> &Money {
        &self.money
    }

    /// Provider or versioned-pricebook evidence.
    #[must_use]
    pub const fn evidence(&self) -> &CostEvidence {
        &self.evidence
    }

    /// Measure priced by this row, absent for provider-reported amounts.
    #[must_use]
    pub const fn calculated_from(&self) -> Option<&MeasureId> {
        self.calculated_from.as_ref()
    }
}

/// Exact per-flow grouping dimensions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReportScope {
    origin_id: Option<Id>,
    instance: Option<InstanceRef>,
    owner: Option<OwnerPath>,
    node: Option<node::Path>,
    connector: Option<node::Path>,
    port: node::Port,
    exchange: Option<ExchangeCorrelation>,
    provider: Option<String>,
    model: Option<String>,
}

impl ReportScope {
    /// Root flow identity.
    #[must_use]
    pub const fn origin_id(&self) -> Option<Id> {
        self.origin_id
    }

    /// Exact logical instance generation.
    #[must_use]
    pub const fn instance(&self) -> Option<&InstanceRef> {
        self.instance.as_ref()
    }

    /// Transaction owner.
    #[must_use]
    pub const fn owner(&self) -> Option<&OwnerPath> {
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
    scope: ReportScope,
    measures: Vec<MeasureSample>,
    costs: Vec<CostSample>,
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
    pub const fn scope(&self) -> &ReportScope {
        &self.scope
    }

    /// Exact additive samples in canonical schema-path order.
    #[must_use]
    pub fn measures(&self) -> &[MeasureSample] {
        &self.measures
    }

    /// Exact monetary facts in canonical schema-path order.
    #[must_use]
    pub fn costs(&self) -> &[CostSample] {
        &self.costs
    }
}

/// One aggregated semantic quantity at one exact flow point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasureReportRow {
    scope: ReportScope,
    id: MeasureId,
    quantity: MeasureQuantity,
    unit: MeasureUnit,
    subset_of: Option<MeasureId>,
}

impl MeasureReportRow {
    /// Per-flow dimensions.
    #[must_use]
    pub const fn scope(&self) -> &ReportScope {
        &self.scope
    }

    /// Registered measure.
    #[must_use]
    pub const fn id(&self) -> &MeasureId {
        &self.id
    }

    /// Exact sum of observed samples.
    #[must_use]
    pub const fn quantity(&self) -> MeasureQuantity {
        self.quantity
    }

    /// Canonical unit.
    #[must_use]
    pub const fn unit(&self) -> MeasureUnit {
        self.unit
    }

    /// Inclusive parent, retained explicitly to prevent naive addition.
    #[must_use]
    pub const fn subset_of(&self) -> Option<&MeasureId> {
        self.subset_of.as_ref()
    }
}

/// One aggregated exact cost, never combined with another currency or basis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostReportRow {
    scope: ReportScope,
    money: Money,
    evidence: CostEvidence,
    calculated_from: Option<MeasureId>,
}

impl CostReportRow {
    /// Per-flow dimensions.
    #[must_use]
    pub const fn scope(&self) -> &ReportScope {
        &self.scope
    }

    /// Exact amount and currency.
    #[must_use]
    pub const fn money(&self) -> &Money {
        &self.money
    }

    /// Provider or pricebook basis and attestation.
    #[must_use]
    pub const fn evidence(&self) -> &CostEvidence {
        &self.evidence
    }

    /// Measure priced by this row.
    #[must_use]
    pub const fn calculated_from(&self) -> Option<&MeasureId> {
        self.calculated_from.as_ref()
    }
}

/// One positive quantity for which no deployment rule exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnpricedMeasure {
    scope: ReportScope,
    id: MeasureId,
    quantity: MeasureQuantity,
}

impl UnpricedMeasure {
    /// Per-flow dimensions.
    #[must_use]
    pub const fn scope(&self) -> &ReportScope {
        &self.scope
    }

    /// Semantic quantity lacking a price.
    #[must_use]
    pub const fn id(&self) -> &MeasureId {
        &self.id
    }

    /// Positive exact remainder; unknown is never represented as zero.
    #[must_use]
    pub const fn quantity(&self) -> MeasureQuantity {
        self.quantity
    }
}

/// Deterministic report rebuilt from journal records and optional deployment
/// pricing. No exporter state participates.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CostReport {
    measures: Vec<MeasureReportRow>,
    costs: Vec<CostReportRow>,
    unpriced: Vec<UnpricedMeasure>,
}

impl CostReport {
    /// Rebuilds a report from the M1 journal and verifies that an optional
    /// price profile is the exact one attested in the run header.
    pub fn from_journal(
        plan: &ValidatedExecutionPlan,
        journal: &MemJournal,
        profile: Option<&CostProfile>,
    ) -> Result<Self, AccountingError> {
        verify_profile_attestation(
            journal
                .deployment_attestation()
                .and_then(|attestation| attestation.cost_profile.as_ref()),
            profile,
        )?;
        Self::from_records(plan, journal.records(), profile)
    }

    /// Derives typed observations from causal records, then folds them.
    ///
    /// # Errors
    ///
    /// A record whose annotated payload is unavailable or malformed, an
    /// inconsistent subset, or inexact decimal arithmetic refuses rather
    /// than dropping or coercing a fact.
    pub fn from_records(
        plan: &ValidatedExecutionPlan,
        records: &[Record],
        profile: Option<&CostProfile>,
    ) -> Result<Self, AccountingError> {
        let observations = accounting_observations(plan, records, profile)?;
        Self::from_observations(&observations, profile)
    }

    /// Folds an already typed accounting stream.
    pub fn from_observations(
        observations: &[AccountingObservation],
        profile: Option<&CostProfile>,
    ) -> Result<Self, AccountingError> {
        if let Some(profile) = profile {
            for observation in observations {
                let expected = profile
                    .dimensions(observation.scope.connector())
                    .map_or((None, None), |(provider, model)| {
                        (Some(provider), Some(model))
                    });
                let actual = (observation.scope.provider(), observation.scope.model());
                if expected != actual {
                    return Err(AccountingError::CostProfileScopeMismatch);
                }
            }
        }
        let mut measures = BTreeMap::<(ReportScope, MeasureId), MeasureAggregate>::new();
        let mut costs = BTreeMap::<CostKey, ExactDecimal>::new();
        for observation in observations {
            validate_observation_subsets(observation)?;
            for sample in observation.measures() {
                let key = (observation.scope.clone(), sample.id.clone());
                match measures.get_mut(&key) {
                    Some(total) => {
                        total.quantity = total.quantity.checked_add(sample.quantity)?;
                    }
                    None => {
                        measures.insert(
                            key,
                            MeasureAggregate {
                                quantity: sample.quantity,
                                unit: sample.unit,
                                subset_of: sample.subset_of.clone(),
                            },
                        );
                    }
                }
            }
            for sample in observation.costs() {
                add_cost(
                    &mut costs,
                    observation.scope.clone(),
                    sample.money.clone(),
                    sample.evidence.clone(),
                    sample.calculated_from.clone(),
                )?;
            }
        }

        let mut unpriced = BTreeMap::<(ReportScope, MeasureId), MeasureQuantity>::new();
        let scopes: BTreeSet<ReportScope> =
            measures.keys().map(|(scope, _)| scope.clone()).collect();
        for scope in scopes {
            price_scope(&scope, &measures, profile, &mut costs, &mut unpriced)?;
        }

        let measures = measures
            .into_iter()
            .map(|((scope, id), total)| MeasureReportRow {
                scope,
                id,
                quantity: total.quantity,
                unit: total.unit,
                subset_of: total.subset_of,
            })
            .collect();
        let costs = costs
            .into_iter()
            .map(|(key, amount)| CostReportRow {
                scope: key.scope,
                money: Money {
                    amount,
                    currency: key.currency,
                },
                evidence: key.evidence,
                calculated_from: key.calculated_from,
            })
            .collect();
        let unpriced = unpriced
            .into_iter()
            .filter(|(_, quantity)| quantity.magnitude() != 0)
            .map(|((scope, id), quantity)| UnpricedMeasure {
                scope,
                id,
                quantity,
            })
            .collect();
        Ok(Self {
            measures,
            costs,
            unpriced,
        })
    }

    /// Exact measure rows in canonical grouping order.
    #[must_use]
    pub fn measures(&self) -> &[MeasureReportRow] {
        &self.measures
    }

    /// Exact monetary rows, separated by basis, currency and priced measure.
    #[must_use]
    pub fn costs(&self) -> &[CostReportRow] {
        &self.costs
    }

    /// Positive quantities without a price rule.
    #[must_use]
    pub fn unpriced(&self) -> &[UnpricedMeasure] {
        &self.unpriced
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
            Record::Inbound(inbound) => {
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
                    profile,
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
                        profile,
                    )?;
                }
            }
            Record::Outbound(_) => {}
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
    owner_path: &OwnerPath,
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
) -> ReportScope {
    let connector = connector
        .or_else(|| correlation.connector())
        .or(lineage_connector);
    let (provider, model) = profile
        .and_then(|profile| profile.dimensions(connector))
        .map_or((None, None), |(provider, model)| {
            (Some(provider.to_string()), Some(model.to_string()))
        });
    ReportScope {
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
        Record::Inbound(inbound) if inbound.envelope.meta().sys().id() == envelope => {
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
        Record::Inbound(_) | Record::Transaction(_) | Record::Outbound(_) => None,
    })
}

fn observe_envelope(
    result: &mut Vec<AccountingObservation>,
    record: RecordObservation,
    envelope: &crate::envelope::Envelope,
    annotations: &AnnotationTable,
    scope: ReportScope,
    profile: Option<&CostProfile>,
) -> Result<(), AccountingError> {
    if annotations.is_empty() {
        return Ok(());
    }
    let value = envelope.data().materialized_value().ok_or(
        AccountingError::UnmaterializedAnnotatedEnvelope(envelope.meta().sys().id()),
    )?;
    let mut measures = Vec::new();
    let mut costs = Vec::new();
    for (path, set) in annotations.entries() {
        let values = values_at_path(value, path)?;
        for selected in values {
            for annotation in set.annotations() {
                match annotation {
                    DataAnnotation::Measure(id) => {
                        let spec = measure_spec(id).ok_or_else(|| {
                            AccountingError::UnknownMeasure(id.as_str().to_string())
                        })?;
                        let quantity = quantity_from(selected, spec.carrier())?;
                        let subset_of = spec
                            .subset_of()
                            .map(MeasureId::new)
                            .transpose()
                            .map_err(|_| AccountingError::CorruptMeasureRegistry)?;
                        measures.push(MeasureSample {
                            path: path.clone(),
                            id: id.clone(),
                            quantity,
                            unit: spec.unit(),
                            subset_of,
                        });
                    }
                    DataAnnotation::Cost(cost) => {
                        let money = money_from(selected, cost)?;
                        let evidence = cost_evidence(cost.basis(), profile, scope.connector())?;
                        costs.push(CostSample {
                            path: Some(path.clone()),
                            money,
                            evidence,
                            calculated_from: None,
                        });
                    }
                    DataAnnotation::Sensitive(_)
                    | DataAnnotation::Secret
                    | DataAnnotation::Personal(_) => {}
                }
            }
        }
    }
    if !measures.is_empty() || !costs.is_empty() {
        result.push(AccountingObservation {
            record,
            envelope_id: envelope.meta().sys().id(),
            scope,
            measures,
            costs,
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

fn quantity_from(
    value: &Value,
    carrier: MeasureCarrier,
) -> Result<MeasureQuantity, AccountingError> {
    match (carrier, value) {
        (MeasureCarrier::Integer, Value::Uint(value)) => {
            Ok(MeasureQuantity::Integer(u128::from(*value)))
        }
        (MeasureCarrier::Integer, Value::Int(value)) if *value >= 0 => {
            Ok(MeasureQuantity::Integer(*value as u128))
        }
        (MeasureCarrier::Duration, Value::Duration(value)) if *value >= 0 => Ok(
            MeasureQuantity::DurationNanoseconds(u128::from(*value as u64)),
        ),
        (MeasureCarrier::Integer | MeasureCarrier::Duration, _) => {
            Err(AccountingError::NegativeOrMalformedMeasure)
        }
    }
}

fn money_from(value: &Value, annotation: &CostAnnotation) -> Result<Money, AccountingError> {
    match (annotation.currency(), value) {
        (Some(currency), Value::Decimal { unscaled, scale }) => {
            Money::new(ExactDecimal::new(*unscaled, *scale)?, currency.clone())
        }
        (None, Value::Struct(fields)) => {
            let amount = fields
                .iter()
                .find(|(name, _)| name == "amount")
                .map(|(_, value)| value)
                .ok_or(AccountingError::MalformedMoney)?;
            let currency = fields
                .iter()
                .find(|(name, _)| name == "currency")
                .map(|(_, value)| value)
                .ok_or(AccountingError::MalformedMoney)?;
            let Value::Decimal { unscaled, scale } = amount else {
                return Err(AccountingError::MalformedMoney);
            };
            let Value::Text(currency) = currency else {
                return Err(AccountingError::MalformedMoney);
            };
            Money::new(
                ExactDecimal::new(*unscaled, *scale)?,
                CurrencyCode::new(currency.clone())
                    .map_err(|_| AccountingError::UnknownCurrency)?,
            )
        }
        _ => Err(AccountingError::MalformedMoney),
    }
}

fn cost_evidence(
    basis: CostBasis,
    profile: Option<&CostProfile>,
    connector: Option<&node::Path>,
) -> Result<CostEvidence, AccountingError> {
    match basis {
        CostBasis::ProviderReported => Ok(CostEvidence::ProviderReported),
        CostBasis::PricebookCalculated => {
            let profile = profile.ok_or(AccountingError::MissingCostProfileEvidence)?;
            if profile.dimensions(connector).is_none() {
                return Err(AccountingError::MissingCostProfileEvidence);
            }
            Ok(CostEvidence::PricebookCalculated {
                profile: profile.reference().clone(),
                digest: *profile.digest(),
            })
        }
    }
}

fn validate_observation_subsets(
    observation: &AccountingObservation,
) -> Result<(), AccountingError> {
    let mut totals = BTreeMap::<MeasureId, MeasureQuantity>::new();
    let mut children = BTreeMap::<MeasureId, MeasureQuantity>::new();
    for sample in observation.measures() {
        match totals.get_mut(&sample.id) {
            Some(total) => *total = total.checked_add(sample.quantity)?,
            None => {
                totals.insert(sample.id.clone(), sample.quantity);
            }
        }
        if let Some(parent) = &sample.subset_of {
            match children.get_mut(parent) {
                Some(total) => *total = total.checked_add(sample.quantity)?,
                None => {
                    children.insert(parent.clone(), sample.quantity);
                }
            }
        }
    }
    for (parent, children) in children {
        if let Some(parent) = totals.get(&parent) {
            parent.checked_sub(children)?;
        }
    }
    Ok(())
}

#[derive(Clone)]
struct MeasureAggregate {
    quantity: MeasureQuantity,
    unit: MeasureUnit,
    subset_of: Option<MeasureId>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CostKey {
    scope: ReportScope,
    currency: CurrencyCode,
    evidence: CostEvidence,
    calculated_from: Option<MeasureId>,
}

fn add_cost(
    costs: &mut BTreeMap<CostKey, ExactDecimal>,
    scope: ReportScope,
    money: Money,
    evidence: CostEvidence,
    calculated_from: Option<MeasureId>,
) -> Result<(), AccountingError> {
    let key = CostKey {
        scope,
        currency: money.currency.clone(),
        evidence,
        calculated_from,
    };
    match costs.get_mut(&key) {
        Some(total) => {
            let sum = total.checked_add(money.amount)?;
            Money::new(sum, money.currency.clone())?;
            *total = sum;
        }
        None => {
            costs.insert(key, money.amount);
        }
    }
    Ok(())
}

fn price_scope(
    scope: &ReportScope,
    measures: &BTreeMap<(ReportScope, MeasureId), MeasureAggregate>,
    profile: Option<&CostProfile>,
    costs: &mut BTreeMap<CostKey, ExactDecimal>,
    unpriced: &mut BTreeMap<(ReportScope, MeasureId), MeasureQuantity>,
) -> Result<(), AccountingError> {
    let scoped: BTreeMap<MeasureId, MeasureAggregate> = measures
        .iter()
        .filter(|((candidate, _), _)| candidate == scope)
        .map(|((_, id), value)| (id.clone(), value.clone()))
        .collect();
    let roots: Vec<MeasureId> = scoped
        .iter()
        .filter(|(_, value)| {
            value
                .subset_of
                .as_ref()
                .is_none_or(|parent| !scoped.contains_key(parent))
        })
        .map(|(id, _)| id.clone())
        .collect();
    for root in roots {
        price_measure_tree(scope, &root, &scoped, profile, costs, unpriced)?;
    }
    Ok(())
}

fn price_measure_tree(
    scope: &ReportScope,
    id: &MeasureId,
    measures: &BTreeMap<MeasureId, MeasureAggregate>,
    profile: Option<&CostProfile>,
    costs: &mut BTreeMap<CostKey, ExactDecimal>,
    unpriced: &mut BTreeMap<(ReportScope, MeasureId), MeasureQuantity>,
) -> Result<(), AccountingError> {
    let total = measures
        .get(id)
        .ok_or(AccountingError::CorruptMeasureRegistry)?;
    let children: Vec<MeasureId> = measures
        .iter()
        .filter(|(_, value)| value.subset_of.as_ref() == Some(id))
        .map(|(child, _)| child.clone())
        .collect();
    let mut remainder = total.quantity;
    for child in &children {
        if subtree_has_price(scope, child, measures, profile) {
            let child_total = measures
                .get(child)
                .ok_or(AccountingError::CorruptMeasureRegistry)?;
            remainder = remainder.checked_sub(child_total.quantity)?;
            price_measure_tree(scope, child, measures, profile, costs, unpriced)?;
        }
    }
    if remainder.magnitude() == 0 {
        return Ok(());
    }
    if let Some((profile, rule)) = profile.and_then(|profile| {
        profile
            .rule(scope.connector(), id)
            .map(|rule| (profile, rule))
    }) {
        let amount = rule
            .price_money()
            .amount()
            .price(remainder.magnitude(), rule.units())?;
        add_cost(
            costs,
            scope.clone(),
            Money::new(amount, rule.price_money().currency().clone())?,
            CostEvidence::PricebookCalculated {
                profile: profile.reference().clone(),
                digest: *profile.digest(),
            },
            Some(id.clone()),
        )?;
    } else {
        let key = (scope.clone(), id.clone());
        match unpriced.get_mut(&key) {
            Some(quantity) => *quantity = quantity.checked_add(remainder)?,
            None => {
                unpriced.insert(key, remainder);
            }
        }
    }
    Ok(())
}

fn subtree_has_price(
    scope: &ReportScope,
    id: &MeasureId,
    measures: &BTreeMap<MeasureId, MeasureAggregate>,
    profile: Option<&CostProfile>,
) -> bool {
    profile.is_some_and(|profile| {
        profile.rule(scope.connector(), id).is_some()
            || measures.iter().any(|(child, value)| {
                value.subset_of.as_ref() == Some(id)
                    && subtree_has_price(scope, child, measures, Some(profile))
            })
    })
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

fn verify_profile_attestation(
    attested: Option<&CostProfileAttestation>,
    profile: Option<&CostProfile>,
) -> Result<(), AccountingError> {
    match (attested, profile) {
        (Some(attested), Some(profile))
            if attested.reference == *profile.reference()
                && attested.digest == *profile.digest() =>
        {
            Ok(())
        }
        (None, None) | (Some(_), None) => Ok(()),
        (Some(_), Some(_)) => Err(AccountingError::CostProfileAttestationMismatch),
        (None, Some(_)) => Err(AccountingError::UnattestedCostProfile),
    }
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
        crate::identity::encoding::decimal_tagged(
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
    /// Unknown registered measure.
    UnknownMeasure(String),
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
    /// Exact additive quantity overflowed.
    MeasureOverflow,
    /// A measure changed carrier under one semantic id.
    MeasureCarrierChanged,
    /// A subset observation exceeded its inclusive total.
    SubsetExceedsTotal,
    /// A measure was negative or did not match its registered carrier.
    NegativeOrMalformedMeasure,
    /// Currency is absent from the pinned ISO registry.
    UnknownCurrency,
    /// A Money-shaped value did not contain exact amount/currency fields.
    MalformedMoney,
    /// A pricebook field lacked the profile attestation which gives it basis.
    MissingCostProfileEvidence,
    /// Annotated data was retained only as undecoded bytes.
    UnmaterializedAnnotatedEnvelope(Id),
    /// Journal flow could not be reconciled with a resolved port schema.
    UnknownFlowSchema,
    /// Accounting cannot target a tagged case without a value field.
    AnnotationTargetsUnionCase,
    /// The pinned measure registry contradicted itself.
    CorruptMeasureRegistry,
    /// A supplied price profile differs from the run-header attestation.
    CostProfileAttestationMismatch,
    /// A supplied price profile was never attested for this run.
    UnattestedCostProfile,
    /// Typed observations carry dimensions from another price profile.
    CostProfileScopeMismatch,
}

impl fmt::Display for AccountingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProfileRef(reference) => {
                write!(formatter, "invalid cost-profile reference `{reference}`")
            }
            Self::InvalidDimension(name) => write!(formatter, "invalid cost-profile {name}"),
            Self::UnknownMeasure(id) => write!(formatter, "unknown measure `{id}`"),
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
            Self::MeasureOverflow => formatter.write_str("exact measure addition overflowed"),
            Self::MeasureCarrierChanged => {
                formatter.write_str("one measure changed carrier while folding")
            }
            Self::SubsetExceedsTotal => {
                formatter.write_str("a measure subset exceeds its inclusive total")
            }
            Self::NegativeOrMalformedMeasure => formatter
                .write_str("a measure must be a non-negative value of its registered carrier"),
            Self::UnknownCurrency => formatter.write_str("unknown ISO currency"),
            Self::MalformedMoney => formatter.write_str("malformed canonical Money value"),
            Self::MissingCostProfileEvidence => formatter.write_str(
                "a pricebook-calculated field has no matching attested cost-profile evidence",
            ),
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
            Self::CorruptMeasureRegistry => {
                formatter.write_str("the pinned measure registry is internally inconsistent")
            }
            Self::CostProfileAttestationMismatch => {
                formatter.write_str("the cost profile differs from the run-header attestation")
            }
            Self::UnattestedCostProfile => {
                formatter.write_str("the cost profile was not attested for this run")
            }
            Self::CostProfileScopeMismatch => formatter
                .write_str("accounting observations do not match the cost-profile dimensions"),
        }
    }
}

impl core::error::Error for AccountingError {}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use alloc::vec;

    fn measure(id: &str, quantity: u128) -> MeasureSample {
        let id = MeasureId::new(id).unwrap_or_else(|error| panic!("measure id: {error}"));
        let spec = measure_spec(&id).unwrap_or_else(|| panic!("registered measure"));
        MeasureSample {
            path: SchemaPath::root(),
            subset_of: spec
                .subset_of()
                .map(MeasureId::new)
                .transpose()
                .unwrap_or_else(|error| panic!("parent id: {error}")),
            id,
            quantity: MeasureQuantity::Integer(quantity),
            unit: spec.unit(),
        }
    }

    fn connector() -> node::Path {
        node::Path::new(["model"]).unwrap_or_else(|error| panic!("path: {error}"))
    }

    fn scope() -> ReportScope {
        ReportScope {
            origin_id: Some(Id::from_bytes([7; 16])),
            instance: None,
            owner: None,
            node: Some(connector()),
            connector: Some(connector()),
            port: node::Port::new("out").unwrap_or_else(|error| panic!("port: {error}")),
            exchange: None,
            provider: Some(String::from("openai")),
            model: Some(String::from("gpt-5")),
        }
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
            CostProfileRef::new("pricing/test@1")
                .unwrap_or_else(|error| panic!("profile ref: {error}")),
            rules,
        )
        .unwrap_or_else(|error| panic!("profile: {error}"))
    }

    fn observation(measures: Vec<MeasureSample>, costs: Vec<CostSample>) -> AccountingObservation {
        AccountingObservation {
            record: crate::observation::RecordObservation::new(
                crate::observation::RecordObservationKind::TransactionCommitted,
                crate::observation::RecordCorrelation::new(Some(Id::from_bytes([7; 16]))),
            ),
            envelope_id: Id::from_bytes([9; 16]),
            scope: scope(),
            measures,
            costs,
        }
    }

    #[test]
    fn decimal_values_normalize_and_add_without_float() {
        let one = ExactDecimal::new(100, 2).unwrap_or_else(|error| panic!("decimal: {error}"));
        let half = ExactDecimal::new(5, 1).unwrap_or_else(|error| panic!("decimal: {error}"));
        assert_eq!(one, ExactDecimal::new(1, 0).unwrap());
        assert_eq!(
            one.checked_add(half).unwrap(),
            ExactDecimal::new(15, 1).unwrap()
        );
        assert_eq!(
            half.price(3, 100).unwrap(),
            ExactDecimal::new(15, 3).unwrap()
        );
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
            MeasureId::new("gen_ai.usage.input_tokens").unwrap(),
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
    fn report_pricing_accepts_only_the_profile_attested_for_the_run() {
        let profile = profile(&[("gen_ai.usage.input_tokens", 1, 1, 0, "USD")]);
        let exact = CostProfileAttestation {
            reference: profile.reference().clone(),
            digest: *profile.digest(),
        };
        assert_eq!(
            verify_profile_attestation(Some(&exact), Some(&profile)),
            Ok(())
        );
        assert_eq!(verify_profile_attestation(Some(&exact), None), Ok(()));
        assert_eq!(verify_profile_attestation(None, None), Ok(()));
        assert_eq!(
            verify_profile_attestation(None, Some(&profile)),
            Err(AccountingError::UnattestedCostProfile)
        );

        let mismatched = CostProfileAttestation {
            reference: exact.reference.clone(),
            digest: [0; 32],
        };
        assert_eq!(
            verify_profile_attestation(Some(&mismatched), Some(&profile)),
            Err(AccountingError::CostProfileAttestationMismatch)
        );
    }

    #[test]
    fn specialized_subset_prices_replace_only_their_inclusive_share() {
        let observation = observation(
            vec![
                measure("gen_ai.usage.input_tokens", 1_000),
                measure("gen_ai.usage.cache_read.input_tokens", 200),
                measure("gen_ai.usage.output_tokens", 100),
            ],
            Vec::new(),
        );
        let profile = profile(&[
            ("gen_ai.usage.input_tokens", 1_000, 10, 0, "USD"),
            ("gen_ai.usage.cache_read.input_tokens", 1_000, 2, 0, "USD"),
        ]);
        let report = CostReport::from_observations(&[observation], Some(&profile))
            .unwrap_or_else(|error| panic!("report: {error}"));

        assert_eq!(report.measures().len(), 3);
        assert_eq!(report.costs().len(), 2);
        let input = report
            .costs()
            .iter()
            .find(|row| {
                row.calculated_from()
                    .is_some_and(|id| id.as_str() == "gen_ai.usage.input_tokens")
            })
            .unwrap_or_else(|| panic!("input price"));
        let cache = report
            .costs()
            .iter()
            .find(|row| {
                row.calculated_from()
                    .is_some_and(|id| id.as_str() == "gen_ai.usage.cache_read.input_tokens")
            })
            .unwrap_or_else(|| panic!("cache price"));
        assert_eq!(input.money().amount(), ExactDecimal::new(8, 0).unwrap());
        assert_eq!(cache.money().amount(), ExactDecimal::new(4, 1).unwrap());
        assert_eq!(report.unpriced().len(), 1);
        assert_eq!(
            report.unpriced()[0].id().as_str(),
            "gen_ai.usage.output_tokens"
        );
        assert_eq!(report.unpriced()[0].quantity().magnitude(), 100);
    }

    #[test]
    fn typed_observations_cannot_be_repriced_under_other_dimensions() {
        let observation = observation(vec![measure("gen_ai.usage.input_tokens", 10)], Vec::new());
        let alien = CostProfile::new(
            CostProfileRef::new("pricing/alien@1").unwrap(),
            vec![
                UnitPrice::new(
                    connector(),
                    "anthropic",
                    "claude",
                    MeasureId::new("gen_ai.usage.input_tokens").unwrap(),
                    1,
                    ExactDecimal::new(1, 0).unwrap(),
                    CurrencyCode::new("USD").unwrap(),
                )
                .unwrap(),
            ],
        )
        .unwrap();

        assert_eq!(
            CostReport::from_observations(&[observation], Some(&alien)),
            Err(AccountingError::CostProfileScopeMismatch)
        );
    }

    #[test]
    fn an_absent_profile_keeps_only_inclusive_roots_unpriced() {
        let observation = observation(
            vec![
                measure("gen_ai.usage.input_tokens", 1_000),
                measure("gen_ai.usage.cache_read.input_tokens", 200),
                measure("gen_ai.usage.output_tokens", 100),
                measure("gen_ai.usage.reasoning.output_tokens", 40),
            ],
            Vec::new(),
        );
        let report = CostReport::from_observations(&[observation], None)
            .unwrap_or_else(|error| panic!("report: {error}"));
        assert_eq!(report.measures().len(), 4);
        assert_eq!(report.costs().len(), 0);
        assert_eq!(report.unpriced().len(), 2);
        assert!(
            report
                .unpriced()
                .iter()
                .any(|row| row.id().as_str() == "gen_ai.usage.input_tokens"
                    && row.quantity().magnitude() == 1_000)
        );
        assert!(
            report
                .unpriced()
                .iter()
                .any(|row| row.id().as_str() == "gen_ai.usage.output_tokens"
                    && row.quantity().magnitude() == 100)
        );
    }

    #[test]
    fn provider_costs_never_cross_currency_or_basis() {
        let usd = CurrencyCode::new("USD").unwrap();
        let eur = CurrencyCode::new("EUR").unwrap();
        let calculated = profile(&[("gen_ai.usage.input_tokens", 1, 0, 0, "USD")]);
        let observation = observation(
            Vec::new(),
            vec![
                CostSample {
                    path: Some(SchemaPath::root()),
                    money: Money::new(ExactDecimal::new(125, 2).unwrap(), usd).unwrap(),
                    evidence: CostEvidence::ProviderReported,
                    calculated_from: None,
                },
                CostSample {
                    path: Some(SchemaPath::root()),
                    money: Money::new(ExactDecimal::new(2, 0).unwrap(), eur).unwrap(),
                    evidence: CostEvidence::ProviderReported,
                    calculated_from: None,
                },
                CostSample {
                    path: Some(SchemaPath::root()),
                    money: Money::new(
                        ExactDecimal::new(5, 1).unwrap(),
                        CurrencyCode::new("USD").unwrap(),
                    )
                    .unwrap(),
                    evidence: CostEvidence::PricebookCalculated {
                        profile: calculated.reference().clone(),
                        digest: *calculated.digest(),
                    },
                    calculated_from: None,
                },
            ],
        );
        let report = CostReport::from_observations(&[observation], None)
            .unwrap_or_else(|error| panic!("report: {error}"));
        assert_eq!(report.costs().len(), 3);
        assert!(report.costs().iter().any(|row| {
            row.money().currency().as_str() == "USD"
                && row.evidence().basis() == CostBasis::ProviderReported
        }));
        assert!(report.costs().iter().any(|row| {
            row.money().currency().as_str() == "USD"
                && row.evidence().basis() == CostBasis::PricebookCalculated
        }));
    }

    #[test]
    fn an_explicit_zero_price_is_not_an_unknown_price() {
        let observation = observation(vec![measure("gen_ai.usage.input_tokens", 10)], Vec::new());
        let profile = profile(&[("gen_ai.usage.input_tokens", 1, 0, 0, "USD")]);
        let report = CostReport::from_observations(&[observation], Some(&profile))
            .unwrap_or_else(|error| panic!("report: {error}"));
        assert_eq!(report.costs().len(), 1);
        assert_eq!(
            report.costs()[0].money().amount(),
            ExactDecimal::new(0, 0).unwrap()
        );
        assert!(report.unpriced().is_empty());
    }

    #[test]
    fn contradictory_subset_observations_refuse_instead_of_underflowing() {
        let observation = observation(
            vec![
                measure("gen_ai.usage.input_tokens", 100),
                measure("gen_ai.usage.cache_read.input_tokens", 101),
            ],
            Vec::new(),
        );
        let profile = profile(&[
            ("gen_ai.usage.input_tokens", 1, 1, 0, "USD"),
            ("gen_ai.usage.cache_read.input_tokens", 1, 1, 0, "USD"),
        ]);
        assert_eq!(
            CostReport::from_observations(&[observation], Some(&profile)),
            Err(AccountingError::SubsetExceedsTotal)
        );
    }

    #[test]
    fn a_later_sample_cannot_mask_an_earlier_subset_contradiction() {
        let invalid = observation(
            vec![
                measure("gen_ai.usage.input_tokens", 100),
                measure("gen_ai.usage.cache_read.input_tokens", 101),
            ],
            Vec::new(),
        );
        let later = observation(
            vec![
                measure("gen_ai.usage.input_tokens", 1_000),
                measure("gen_ai.usage.cache_read.input_tokens", 0),
            ],
            Vec::new(),
        );
        assert_eq!(
            CostReport::from_observations(&[invalid, later], None),
            Err(AccountingError::SubsetExceedsTotal)
        );
    }

    #[test]
    fn aggregated_money_cannot_escape_decimal_38_18() {
        let money = Money::new(
            ExactDecimal::new(9, -19).unwrap(),
            CurrencyCode::new("USD").unwrap(),
        )
        .unwrap();
        let sample = CostSample {
            path: Some(SchemaPath::root()),
            money,
            evidence: CostEvidence::ProviderReported,
            calculated_from: None,
        };
        assert_eq!(
            CostReport::from_observations(
                &[observation(Vec::new(), vec![sample.clone(), sample])],
                None,
            ),
            Err(AccountingError::MoneyOutOfDomain)
        );
    }

    #[test]
    fn calculated_field_cost_needs_a_rule_for_its_causal_connector() {
        let matching = profile(&[("gen_ai.usage.input_tokens", 1, 1, 0, "USD")]);
        assert!(
            cost_evidence(
                CostBasis::PricebookCalculated,
                Some(&matching),
                Some(&connector()),
            )
            .is_ok()
        );
        let unrelated_path =
            node::Path::new(["other"]).unwrap_or_else(|error| panic!("path: {error}"));
        let unrelated = CostProfile::new(
            CostProfileRef::new("pricing/unrelated@1").unwrap(),
            vec![
                UnitPrice::new(
                    unrelated_path,
                    "openai",
                    "gpt-5",
                    MeasureId::new("gen_ai.usage.input_tokens").unwrap(),
                    1,
                    ExactDecimal::new(1, 0).unwrap(),
                    CurrencyCode::new("USD").unwrap(),
                )
                .unwrap(),
            ],
        )
        .unwrap();
        assert_eq!(
            cost_evidence(
                CostBasis::PricebookCalculated,
                Some(&unrelated),
                Some(&connector()),
            ),
            Err(AccountingError::MissingCostProfileEvidence)
        );
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
