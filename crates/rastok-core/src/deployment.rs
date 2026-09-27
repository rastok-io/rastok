//! Typed deployment inputs: boot values, secret references and privacy
//! attestations.
//!
//! This module deliberately contains no environment lookup, file parsing or
//! provider SDK. Hosted code turns physical configuration into these closed
//! values; the semantic core only validates and consumes the resulting facts.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use crate::accounting::{CostProfile, CostProfileName};
use crate::canonical_cbor::{Value as Cbor, encode};
use crate::value::Value;

/// One environment-backed boot binding named by source `env("NAME")`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EnvironmentName(String);

impl EnvironmentName {
    /// Builds one portable environment name.
    ///
    /// # Errors
    ///
    /// Names use the common process-environment subset
    /// `[A-Za-z_][A-Za-z0-9_]*` and are bounded to 128 bytes.
    pub fn new(name: impl Into<String>) -> Result<Self, DeploymentError> {
        let name = name.into();
        let mut bytes = name.bytes();
        let first = bytes.next();
        if name.len() > 128
            || !first.is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
            || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(DeploymentError::InvalidEnvironmentName(name));
        }
        Ok(Self(name))
    }

    /// Exact physical environment name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One source-visible logical secret reference.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SecretName(String);

impl SecretName {
    /// Builds a slash-separated logical secret reference.
    ///
    /// # Errors
    ///
    /// Empty, dot, control-bearing, unbounded or degenerate segments refuse.
    pub fn new(reference: impl Into<String>) -> Result<Self, DeploymentError> {
        let reference = reference.into();
        if !logical_path_holds(&reference) {
            return Err(DeploymentError::InvalidSecretRef(reference));
        }
        Ok(Self(reference))
    }

    /// Canonical logical spelling sealed by the graph.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One exact boot reference recognized in a Connector attribute expression.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BootReference {
    /// A typed value supplied by the deployment environment.
    Environment(EnvironmentName),
    /// Secret material supplied only to an authorized Driver setting.
    Secret(SecretName),
}

/// Stable provider identity selected by deployment, never by graph identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SecretProviderId(String);

impl SecretProviderId {
    /// Builds a stable provider identity such as `rastok/env@1`.
    ///
    /// # Errors
    ///
    /// Empty, whitespace/control-bearing or overlong identities refuse.
    pub fn new(id: impl Into<String>) -> Result<Self, DeploymentError> {
        let id = id.into();
        if !stable_ref_holds(&id) {
            return Err(DeploymentError::InvalidStableRef(id));
        }
        Ok(Self(id))
    }

    /// Stable provider spelling.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Provider-owned immutable version attested before a secret is used.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SecretVersion(String);

impl SecretVersion {
    /// Builds a non-empty provider version.
    ///
    /// # Errors
    ///
    /// Empty, whitespace/control-bearing or overlong versions refuse.
    pub fn new(version: impl Into<String>) -> Result<Self, DeploymentError> {
        let version = version.into();
        if !stable_ref_holds(&version) {
            return Err(DeploymentError::InvalidStableRef(version));
        }
        Ok(Self(version))
    }

    /// Exact provider version.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One deployment-time mapping from a logical secret to physical provider
/// coordinates. It contains no secret material.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SecretBinding {
    reference: SecretName,
    provider: SecretProviderId,
    provider_path: String,
    version: SecretVersion,
}

impl SecretBinding {
    /// Builds one complete, versioned secret binding.
    ///
    /// # Errors
    ///
    /// The provider path must be non-empty, bounded and contain no controls.
    pub fn new(
        reference: SecretName,
        provider: SecretProviderId,
        provider_path: impl Into<String>,
        version: SecretVersion,
    ) -> Result<Self, DeploymentError> {
        let provider_path = provider_path.into();
        if provider_path.is_empty()
            || provider_path.len() > 1024
            || provider_path.chars().any(char::is_control)
        {
            return Err(DeploymentError::InvalidProviderPath);
        }
        Ok(Self {
            reference,
            provider,
            provider_path,
            version,
        })
    }

    /// Logical source reference.
    #[must_use]
    pub fn reference(&self) -> &SecretName {
        &self.reference
    }

    /// Selected provider.
    #[must_use]
    pub fn provider(&self) -> &SecretProviderId {
        &self.provider
    }

    /// Provider-owned lookup path. This is metadata, never secret material.
    #[must_use]
    pub fn provider_path(&self) -> &str {
        &self.provider_path
    }

    /// Immutable provider version used for replay/audit attestation.
    #[must_use]
    pub fn version(&self) -> &SecretVersion {
        &self.version
    }
}

/// Secret bytes returned only across the declared provider/Driver boundary.
///
/// `Debug` and `Display` never reveal contents. This type is intentionally not
/// convertible to [`Value`].
#[derive(Clone)]
pub struct SecretMaterial(Vec<u8>);

impl SecretMaterial {
    /// Wraps non-empty provider bytes.
    ///
    /// # Errors
    ///
    /// An empty provider result refuses rather than becoming a plausible
    /// credential.
    pub fn new(bytes: Vec<u8>) -> Result<Self, SecretProviderError> {
        if bytes.is_empty() {
            return Err(SecretProviderError::EmptyMaterial);
        }
        Ok(Self(bytes))
    }

    /// Exposes bytes only to a boundary implementation that already owns the
    /// corresponding secret-compatible setting.
    #[must_use]
    pub fn expose_to_driver(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for SecretMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretMaterial([REDACTED])")
    }
}

/// Hosted or embedded secret resolver. A registry may implement this one seam
/// and dispatch by [`SecretBinding::provider`].
pub trait SecretProvider {
    /// Resolves one exact versioned binding without logging or formatting the
    /// returned bytes.
    ///
    /// # Errors
    ///
    /// Missing providers, paths, versions and physical failures remain typed
    /// provider refusals.
    fn resolve(&self, binding: &SecretBinding) -> Result<SecretMaterial, SecretProviderError>;
}

/// One provider refusal. Details describe coordinates or failure classes,
/// never material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretProviderError {
    /// The selected provider is unavailable in this deployment image.
    ProviderUnavailable(SecretProviderId),
    /// The logical or physical reference was not found.
    NotFound(SecretName),
    /// The provider could not prove the requested immutable version.
    VersionMismatch(SecretName),
    /// The provider returned no credential bytes.
    EmptyMaterial,
    /// A bounded provider-specific failure class, without secret content.
    ProviderFailure(&'static str),
}

impl fmt::Display for SecretProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProviderUnavailable(provider) => {
                write!(
                    formatter,
                    "secret provider `{}` is unavailable",
                    provider.as_str()
                )
            }
            Self::NotFound(reference) => {
                write!(formatter, "secret `{}` was not found", reference.as_str())
            }
            Self::VersionMismatch(reference) => write!(
                formatter,
                "secret `{}` did not match its attested version",
                reference.as_str()
            ),
            Self::EmptyMaterial => formatter.write_str("the secret provider returned no material"),
            Self::ProviderFailure(class) => {
                write!(formatter, "the secret provider failed: {class}")
            }
        }
    }
}

impl core::error::Error for SecretProviderError {}

/// Stable reference to one controller-owned privacy profile.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrivacyProfileName(String);

impl PrivacyProfileName {
    /// Builds one stable profile reference.
    ///
    /// # Errors
    ///
    /// Empty, whitespace/control-bearing or overlong references refuse.
    pub fn new(reference: impl Into<String>) -> Result<Self, DeploymentError> {
        let reference = reference.into();
        if !stable_ref_holds(&reference) {
            return Err(DeploymentError::InvalidStableRef(reference));
        }
        Ok(Self(reference))
    }

    /// Stable spelling.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Reference to a controller-owned processing, purpose or legal-policy fact.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PolicyName(String);

impl PolicyName {
    /// Builds one stable policy reference.
    ///
    /// # Errors
    ///
    /// Empty, whitespace/control-bearing or overlong references refuse.
    pub fn new(reference: impl Into<String>) -> Result<Self, DeploymentError> {
        let reference = reference.into();
        if !stable_ref_holds(&reference) {
            return Err(DeploymentError::InvalidStableRef(reference));
        }
        Ok(Self(reference))
    }

    /// Stable spelling.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Explicit recipient declaration for one processing activity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecipientPolicy {
    /// No external recipient is declared.
    None,
    /// Stable references to declared recipient classes.
    Classes(Vec<PolicyName>),
}

/// Explicit international-transfer declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferPolicy {
    /// The activity declares no international transfer.
    None,
    /// Stable references to controller-approved safeguards.
    Safeguards(Vec<PolicyName>),
}

/// Retention capability admitted by M1's in-memory runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyRetention {
    /// Values and their causal journal disappear with the run.
    RunScoped,
}

/// Personal-data regimes found in one validated execution plan.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrivacyRequirements {
    /// At least one personal-data path exists.
    pub personal: bool,
    /// At least one path carries Article 9 special-category data.
    pub special_category: bool,
    /// At least one path carries Article 10 criminal-offence data.
    pub criminal_offence: bool,
    /// Personal data reaches an input of an external Connector.
    pub external_disclosure: bool,
}

/// Controller-supplied privacy facts. Rastok verifies completeness and exact
/// digest only; it never certifies that a legal judgement is substantively
/// correct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivacyProfile {
    reference: PrivacyProfileName,
    processing_activity: PolicyName,
    purposes: Vec<PolicyName>,
    legal_bases: Vec<PolicyName>,
    special_category_condition: Option<PolicyName>,
    criminal_offence_authority: Option<PolicyName>,
    recipients: RecipientPolicy,
    transfers: TransferPolicy,
    retention: PrivacyRetention,
    digest: [u8; 32],
}

impl PrivacyProfile {
    /// Builds one canonical, run-scoped privacy profile.
    ///
    /// # Errors
    ///
    /// Purpose and legal-basis attestations are mandatory. Lists must be
    /// non-empty where selected and contain no duplicates.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        reference: PrivacyProfileName,
        processing_activity: PolicyName,
        purposes: Vec<PolicyName>,
        legal_bases: Vec<PolicyName>,
        special_category_condition: Option<PolicyName>,
        criminal_offence_authority: Option<PolicyName>,
        recipients: RecipientPolicy,
        transfers: TransferPolicy,
        retention: PrivacyRetention,
    ) -> Result<Self, DeploymentError> {
        let purposes = canonical_non_empty(purposes, "purposes")?;
        let legal_bases = canonical_non_empty(legal_bases, "legal bases")?;
        let recipients = match recipients {
            RecipientPolicy::None => RecipientPolicy::None,
            RecipientPolicy::Classes(classes) => {
                RecipientPolicy::Classes(canonical_non_empty(classes, "recipient classes")?)
            }
        };
        let transfers = match transfers {
            TransferPolicy::None => TransferPolicy::None,
            TransferPolicy::Safeguards(safeguards) => {
                TransferPolicy::Safeguards(canonical_non_empty(safeguards, "transfer safeguards")?)
            }
        };
        let mut profile = Self {
            reference,
            processing_activity,
            purposes,
            legal_bases,
            special_category_condition,
            criminal_offence_authority,
            recipients,
            transfers,
            retention,
            digest: [0; 32],
        };
        profile.digest = *blake3::hash(&encode(&profile.canonical_tree())).as_bytes();
        Ok(profile)
    }

    /// Stable profile reference.
    #[must_use]
    pub fn reference(&self) -> &PrivacyProfileName {
        &self.reference
    }

    /// Digest of every controller-supplied privacy fact.
    #[must_use]
    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    /// Whether this profile contains each additional attestation required by
    /// the validated personal-data regimes.
    #[must_use]
    pub fn covers(&self, requirements: PrivacyRequirements) -> bool {
        (!requirements.personal || (!self.purposes.is_empty() && !self.legal_bases.is_empty()))
            && (!requirements.special_category || self.special_category_condition.is_some())
            && (!requirements.criminal_offence || self.criminal_offence_authority.is_some())
            && (!requirements.external_disclosure
                || matches!(&self.recipients, RecipientPolicy::Classes(classes) if !classes.is_empty()))
            && self.retention == PrivacyRetention::RunScoped
    }

    fn canonical_tree(&self) -> Cbor {
        Cbor::Array(alloc::vec![
            Cbor::Text(self.reference.as_str().to_string()),
            Cbor::Text(self.processing_activity.as_str().to_string()),
            policy_refs_tree(&self.purposes),
            policy_refs_tree(&self.legal_bases),
            optional_policy_ref_tree(self.special_category_condition.as_ref()),
            optional_policy_ref_tree(self.criminal_offence_authority.as_ref()),
            match &self.recipients {
                RecipientPolicy::None => Cbor::Array(alloc::vec![Cbor::Uint(0)]),
                RecipientPolicy::Classes(classes) => {
                    Cbor::Array(alloc::vec![Cbor::Uint(1), policy_refs_tree(classes)])
                }
            },
            match &self.transfers {
                TransferPolicy::None => Cbor::Array(alloc::vec![Cbor::Uint(0)]),
                TransferPolicy::Safeguards(safeguards) => {
                    Cbor::Array(alloc::vec![Cbor::Uint(1), policy_refs_tree(safeguards)])
                }
            },
            Cbor::Uint(match self.retention {
                PrivacyRetention::RunScoped => 1,
            }),
        ])
    }
}

/// Attested profile identity stored in a run header, never in graph identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivacyAttestation {
    /// Stable profile reference.
    pub reference: PrivacyProfileName,
    /// Digest of all profile facts used at boot.
    pub digest: [u8; 32],
}

/// Cost-profile identity stored in the run header, never in graph identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostProfileAttestation {
    /// Stable deployment profile reference.
    pub reference: CostProfileName,
    /// Digest of every price and dimension in force.
    pub digest: [u8; 32],
}

/// Digest of one non-secret boot value fixed before execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentAttestation {
    /// Portable source-visible binding name.
    pub name: EnvironmentName,
    /// Canonical tagged-value digest; the value itself need not enter the
    /// run header.
    pub digest: [u8; 32],
}

/// Deployment facts stored in the run header before effects begin.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeploymentAttestation {
    /// Non-secret boot values, sorted by source-visible name.
    pub environment: Vec<EnvironmentAttestation>,
    /// Versioned secret coordinates, sorted by logical reference.
    pub secrets: Vec<SecretBinding>,
    /// Privacy profile identity when personal data is admitted.
    pub privacy: Option<PrivacyAttestation>,
    /// Retained price-profile identity; M1 performs no automatic pricing.
    pub cost_profile: Option<CostProfileAttestation>,
}

/// Typed physical configuration before providers return any material.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeploymentProfile {
    environment: BTreeMap<EnvironmentName, Value>,
    secrets: BTreeMap<SecretName, SecretBinding>,
    privacy: Option<PrivacyProfile>,
    cost_profile: Option<CostProfile>,
}

impl DeploymentProfile {
    /// Builds one profile from already parsed typed inputs.
    ///
    /// # Errors
    ///
    /// A secret map key must equal its binding's logical reference.
    pub fn new(
        environment: BTreeMap<EnvironmentName, Value>,
        secrets: BTreeMap<SecretName, SecretBinding>,
        privacy: Option<PrivacyProfile>,
    ) -> Result<Self, DeploymentError> {
        if secrets
            .iter()
            .any(|(reference, binding)| reference != binding.reference())
        {
            return Err(DeploymentError::SecretBindingKeyMismatch);
        }
        Ok(Self {
            environment,
            secrets,
            privacy,
            cost_profile: None,
        })
    }

    /// Attaches one optional exact price profile without changing graph
    /// identity. Its reference and digest enter the run attestation at boot.
    #[must_use]
    pub fn with_cost_profile(mut self, profile: CostProfile) -> Self {
        self.cost_profile = Some(profile);
        self
    }

    /// Empty profile for graphs with no boot dependency.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }
}

/// Fully resolved boot inputs. Secret bytes remain out of [`Value`] and have
/// no `Debug` path.
pub struct BootBindings {
    environment: BTreeMap<EnvironmentName, Value>,
    secrets: BTreeMap<SecretName, SecretMaterial>,
    privacy: Option<PrivacyProfile>,
    cost_profile: Option<CostProfile>,
    attestation: DeploymentAttestation,
}

impl BootBindings {
    /// Resolves every profile secret atomically through one provider seam.
    ///
    /// # Errors
    ///
    /// Any provider refusal aborts construction; no partial binding is
    /// executable.
    pub fn resolve(
        profile: DeploymentProfile,
        provider: &dyn SecretProvider,
    ) -> Result<Self, SecretProviderError> {
        let mut secrets = BTreeMap::new();
        for (reference, binding) in &profile.secrets {
            secrets.insert(reference.clone(), provider.resolve(binding)?);
        }
        let attestation = DeploymentAttestation {
            environment: profile
                .environment
                .iter()
                .map(|(name, value)| EnvironmentAttestation {
                    name: name.clone(),
                    digest: *blake3::hash(&encode(&crate::identity::encoding::canonical_value(
                        value,
                    )))
                    .as_bytes(),
                })
                .collect(),
            secrets: profile.secrets.into_values().collect(),
            privacy: profile.privacy.as_ref().map(|privacy| PrivacyAttestation {
                reference: privacy.reference().clone(),
                digest: *privacy.digest(),
            }),
            cost_profile: profile
                .cost_profile
                .as_ref()
                .map(|cost| CostProfileAttestation {
                    reference: cost.reference().clone(),
                    digest: *cost.digest(),
                }),
        };
        Ok(Self {
            environment: profile.environment,
            secrets,
            privacy: profile.privacy,
            cost_profile: profile.cost_profile,
            attestation,
        })
    }

    /// Empty bindings for existing plans without boot policy.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            environment: BTreeMap::new(),
            secrets: BTreeMap::new(),
            privacy: None,
            cost_profile: None,
            attestation: DeploymentAttestation::default(),
        }
    }

    /// Typed non-secret boot value.
    #[must_use]
    pub fn environment(&self, name: &EnvironmentName) -> Option<&Value> {
        self.environment.get(name)
    }

    /// Secret material for one already-authorized Driver setting.
    #[must_use]
    pub fn secret(&self, reference: &SecretName) -> Option<&SecretMaterial> {
        self.secrets.get(reference)
    }

    /// Whether one exact categorical reference has a resolved binding.
    #[must_use]
    pub fn resolves(&self, reference: &BootReference) -> bool {
        match reference {
            BootReference::Environment(name) => self.environment.contains_key(name),
            BootReference::Secret(reference) => self.secrets.contains_key(reference),
        }
    }

    /// Full privacy profile used to validate a personal-data plan.
    #[must_use]
    pub fn privacy(&self) -> Option<&PrivacyProfile> {
        self.privacy.as_ref()
    }

    /// Attested deployment profile metadata; M1 does not calculate a price from it.
    #[must_use]
    pub fn cost_profile(&self) -> Option<&CostProfile> {
        self.cost_profile.as_ref()
    }

    /// Run-header facts corresponding exactly to these bindings.
    #[must_use]
    pub fn attestation(&self) -> &DeploymentAttestation {
        &self.attestation
    }
}

impl fmt::Debug for BootBindings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BootBindings")
            .field("environment", &self.environment.keys().collect::<Vec<_>>())
            .field("secrets", &self.secrets.keys().collect::<Vec<_>>())
            .field("privacy", &self.attestation.privacy)
            .field("cost_profile", &self.attestation.cost_profile)
            .finish()
    }
}

/// Invalid deployment metadata. No variant contains secret material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeploymentError {
    /// Invalid portable environment name.
    InvalidEnvironmentName(String),
    /// Invalid logical secret path.
    InvalidSecretRef(String),
    /// Invalid stable reference or version.
    InvalidStableRef(String),
    /// Invalid provider lookup path.
    InvalidProviderPath,
    /// A keyed secret binding names another logical reference.
    SecretBindingKeyMismatch,
    /// A categorical expression for a boot reference is malformed.
    InvalidBootReference,
    /// One required policy list is empty or duplicated.
    InvalidPolicyList(&'static str),
}

impl fmt::Display for DeploymentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEnvironmentName(name) => {
                write!(formatter, "`{name}` is not a portable environment name")
            }
            Self::InvalidSecretRef(reference) => {
                write!(
                    formatter,
                    "`{reference}` is not a canonical logical secret reference"
                )
            }
            Self::InvalidStableRef(reference) => {
                write!(formatter, "`{reference}` is not a bounded stable reference")
            }
            Self::InvalidProviderPath => formatter.write_str("the provider path is invalid"),
            Self::SecretBindingKeyMismatch => {
                formatter.write_str("a secret binding is stored under another logical reference")
            }
            Self::InvalidBootReference => {
                formatter.write_str("the categorical boot reference is malformed")
            }
            Self::InvalidPolicyList(name) => {
                write!(
                    formatter,
                    "privacy {name} must be non-empty and duplicate-free"
                )
            }
        }
    }
}

impl core::error::Error for DeploymentError {}

fn logical_path_holds(reference: &str) -> bool {
    !reference.is_empty()
        && reference.len() <= 512
        && reference.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        })
}

fn stable_ref_holds(reference: &str) -> bool {
    !reference.is_empty()
        && reference.len() <= 512
        && reference
            .chars()
            .all(|character| !character.is_control() && !character.is_whitespace())
}

fn canonical_non_empty(
    mut values: Vec<PolicyName>,
    name: &'static str,
) -> Result<Vec<PolicyName>, DeploymentError> {
    if values.is_empty() {
        return Err(DeploymentError::InvalidPolicyList(name));
    }
    values.sort();
    if values.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(DeploymentError::InvalidPolicyList(name));
    }
    Ok(values)
}

fn policy_refs_tree(values: &[PolicyName]) -> Cbor {
    Cbor::Array(
        values
            .iter()
            .map(|value| Cbor::Text(value.as_str().to_string()))
            .collect(),
    )
}

fn optional_policy_ref_tree(value: Option<&PolicyName>) -> Cbor {
    value.map_or(Cbor::Null, |value| Cbor::Text(value.as_str().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    struct Provider;

    impl SecretProvider for Provider {
        fn resolve(&self, binding: &SecretBinding) -> Result<SecretMaterial, SecretProviderError> {
            assert_eq!(binding.version().as_str(), "v7");
            SecretMaterial::new(b"private".to_vec())
        }
    }

    fn policy(value: &str) -> PolicyName {
        PolicyName::new(value).unwrap_or_else(|error| panic!("policy ref: {error}"))
    }

    #[test]
    fn a_secret_reference_never_becomes_a_value_or_debug_output() {
        let reference = SecretName::new("providers/openai/token")
            .unwrap_or_else(|error| panic!("secret ref: {error}"));
        let binding = SecretBinding::new(
            reference.clone(),
            SecretProviderId::new("rastok/test@1")
                .unwrap_or_else(|error| panic!("provider: {error}")),
            "OPENAI_API_KEY",
            SecretVersion::new("v7").unwrap_or_else(|error| panic!("version: {error}")),
        )
        .unwrap_or_else(|error| panic!("binding: {error}"));
        let profile = DeploymentProfile::new(
            BTreeMap::new(),
            BTreeMap::from([(reference.clone(), binding.clone())]),
            None,
        )
        .unwrap_or_else(|error| panic!("profile: {error}"));
        let bindings = BootBindings::resolve(profile, &Provider)
            .unwrap_or_else(|error| panic!("bindings: {error}"));

        assert_eq!(bindings.attestation().secrets, vec![binding]);
        assert_eq!(
            bindings
                .secret(&reference)
                .map(SecretMaterial::expose_to_driver),
            Some(b"private".as_slice())
        );
        let rendered = alloc::format!("{bindings:?}");
        assert!(!rendered.contains("private"));
        assert!(!rendered.contains("OPENAI_API_KEY"));
    }

    #[test]
    fn privacy_digest_is_canonical_and_regime_coverage_is_explicit() {
        let build = |purposes| {
            PrivacyProfile::new(
                PrivacyProfileName::new("privacy/support@1")
                    .unwrap_or_else(|error| panic!("profile ref: {error}")),
                policy("ropa/support"),
                purposes,
                vec![policy("basis/contract")],
                Some(policy("article9/explicit-consent")),
                None,
                RecipientPolicy::None,
                TransferPolicy::None,
                PrivacyRetention::RunScoped,
            )
            .unwrap_or_else(|error| panic!("privacy profile: {error}"))
        };
        let left = build(vec![policy("purpose/quality"), policy("purpose/support")]);
        let right = build(vec![policy("purpose/support"), policy("purpose/quality")]);
        assert_eq!(left.digest(), right.digest());
        assert!(left.covers(PrivacyRequirements {
            personal: true,
            special_category: true,
            criminal_offence: false,
            external_disclosure: false,
        }));
        assert!(!left.covers(PrivacyRequirements {
            personal: true,
            special_category: false,
            criminal_offence: true,
            external_disclosure: false,
        }));
        assert!(!left.covers(PrivacyRequirements {
            personal: true,
            special_category: false,
            criminal_offence: false,
            external_disclosure: true,
        }));
        let disclosed = PrivacyProfile::new(
            PrivacyProfileName::new("privacy/support-disclosed@1")
                .unwrap_or_else(|error| panic!("profile ref: {error}")),
            policy("ropa/support"),
            vec![policy("purpose/support")],
            vec![policy("basis/contract")],
            None,
            None,
            RecipientPolicy::Classes(vec![policy("recipient/model-provider")]),
            TransferPolicy::None,
            PrivacyRetention::RunScoped,
        )
        .unwrap_or_else(|error| panic!("privacy profile: {error}"));
        assert!(disclosed.covers(PrivacyRequirements {
            personal: true,
            special_category: false,
            criminal_offence: false,
            external_disclosure: true,
        }));
    }

    #[test]
    fn invalid_or_ambiguous_physical_names_refuse_at_construction() {
        assert!(EnvironmentName::new("9TOKEN").is_err());
        assert!(SecretName::new("providers/../token").is_err());
        assert!(PrivacyProfileName::new("contains whitespace").is_err());
    }

    #[test]
    fn non_secret_boot_values_are_attested_by_canonical_value_digest() {
        let build = |value| {
            let profile = DeploymentProfile::new(
                BTreeMap::from([(
                    EnvironmentName::new("SERVICE_URL")
                        .unwrap_or_else(|error| panic!("environment name: {error}")),
                    Value::Text(String::from(value)),
                )]),
                BTreeMap::new(),
                None,
            )
            .unwrap_or_else(|error| panic!("profile: {error}"));
            BootBindings::resolve(profile, &Provider)
                .unwrap_or_else(|error| panic!("bindings: {error}"))
        };
        let left = build("https://a.example");
        let equivalent = build("https://a.example");
        let changed = build("https://b.example");
        assert_eq!(left.attestation(), equivalent.attestation());
        assert_ne!(left.attestation(), changed.attestation());
        assert_eq!(left.attestation().environment.len(), 1);
    }
}
