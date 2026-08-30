// This hosted module is the explicit filesystem/environment configuration
// boundary. The no-std core receives only its already-typed result.
#![allow(clippy::disallowed_methods)]
//! Hosted JSON deployment profiles and the minimal environment-backed secret
//! provider.
//!
//! Human syntax stops here. The no-std core receives only typed
//! [`crate::deployment::DeploymentProfile`] and [`crate::deployment::BootBindings`]
//! values. Secret contents are never accepted by this format: a profile names
//! one process-environment coordinate and its separately attested version.

use crate::deployment::{
    BootBindings, DeploymentError, DeploymentProfile, EnvironmentName, PolicyRef, PrivacyProfile,
    PrivacyProfileRef, PrivacyRetention, RecipientPolicy, SecretBinding, SecretMaterial,
    SecretProvider, SecretProviderError, SecretProviderId, SecretRef, SecretVersion,
    TransferPolicy,
};
use crate::value::Value;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

const ENV_SECRET_PROVIDER: &str = "rastok/env@1";

/// Reads, validates and atomically resolves one hosted deployment profile.
///
/// Secret entries name an environment variable; the companion
/// `<NAME>_VERSION` variable must equal the declared immutable version. No
/// secret value appears in JSON or on the command line.
///
/// # Errors
///
/// I/O, JSON shape, duplicate logical bindings, policy construction and
/// secret-provider failures remain distinct safe messages.
pub fn load(path: &Path) -> Result<BootBindings, DeploymentConfigError> {
    let text = std::fs::read_to_string(path).map_err(DeploymentConfigError::Read)?;
    let profile = parse(&text)?;
    BootBindings::resolve(profile, &EnvironmentSecretProvider)
        .map_err(DeploymentConfigError::Secret)
}

fn parse(text: &str) -> Result<DeploymentProfile, DeploymentConfigError> {
    let raw: RawDeployment = serde_json::from_str(text).map_err(DeploymentConfigError::Json)?;
    let mut environment = BTreeMap::new();
    for entry in raw.environment {
        let name = EnvironmentName::new(entry.name).map_err(DeploymentConfigError::Profile)?;
        if environment
            .insert(name.clone(), Value::Text(entry.value))
            .is_some()
        {
            return Err(DeploymentConfigError::DuplicateEnvironment(name));
        }
    }

    let provider =
        SecretProviderId::new(ENV_SECRET_PROVIDER).map_err(DeploymentConfigError::Profile)?;
    let mut secrets = BTreeMap::new();
    for entry in raw.secrets {
        let reference = SecretRef::new(entry.reference).map_err(DeploymentConfigError::Profile)?;
        let environment =
            EnvironmentName::new(entry.environment).map_err(DeploymentConfigError::Profile)?;
        EnvironmentName::new(format!("{}_VERSION", environment.as_str()))
            .map_err(DeploymentConfigError::Profile)?;
        let binding = SecretBinding::new(
            reference.clone(),
            provider.clone(),
            environment.as_str(),
            SecretVersion::new(entry.version).map_err(DeploymentConfigError::Profile)?,
        )
        .map_err(DeploymentConfigError::Profile)?;
        if secrets.insert(reference.clone(), binding).is_some() {
            return Err(DeploymentConfigError::DuplicateSecret(reference));
        }
    }

    let privacy = raw.privacy.map(privacy_profile).transpose()?;
    DeploymentProfile::new(environment, secrets, privacy).map_err(DeploymentConfigError::Profile)
}

fn privacy_profile(raw: RawPrivacy) -> Result<PrivacyProfile, DeploymentConfigError> {
    if raw.retention != "run_scoped" {
        return Err(DeploymentConfigError::UnsupportedRetention(raw.retention));
    }
    let refs = |values: Vec<String>| {
        values
            .into_iter()
            .map(PolicyRef::new)
            .collect::<Result<Vec<_>, _>>()
            .map_err(DeploymentConfigError::Profile)
    };
    let optional = |value: Option<String>| {
        value
            .map(PolicyRef::new)
            .transpose()
            .map_err(DeploymentConfigError::Profile)
    };
    let recipients = refs(raw.recipients)?;
    let transfers = refs(raw.transfer_safeguards)?;
    PrivacyProfile::new(
        PrivacyProfileRef::new(raw.reference).map_err(DeploymentConfigError::Profile)?,
        PolicyRef::new(raw.processing_activity).map_err(DeploymentConfigError::Profile)?,
        refs(raw.purposes)?,
        refs(raw.legal_bases)?,
        optional(raw.special_category_condition)?,
        optional(raw.criminal_offence_authority)?,
        if recipients.is_empty() {
            RecipientPolicy::None
        } else {
            RecipientPolicy::Classes(recipients)
        },
        if transfers.is_empty() {
            TransferPolicy::None
        } else {
            TransferPolicy::Safeguards(transfers)
        },
        PrivacyRetention::RunScoped,
    )
    .map_err(DeploymentConfigError::Profile)
}

struct EnvironmentSecretProvider;

impl SecretProvider for EnvironmentSecretProvider {
    fn resolve(&self, binding: &SecretBinding) -> Result<SecretMaterial, SecretProviderError> {
        if binding.provider().as_str() != ENV_SECRET_PROVIDER {
            return Err(SecretProviderError::ProviderUnavailable(
                binding.provider().clone(),
            ));
        }
        let version_name = format!("{}_VERSION", binding.provider_path());
        let actual_version = std::env::var(&version_name)
            .map_err(|_| SecretProviderError::VersionMismatch(binding.reference().clone()))?;
        if actual_version != binding.version().as_str() {
            return Err(SecretProviderError::VersionMismatch(
                binding.reference().clone(),
            ));
        }
        let material = std::env::var(binding.provider_path())
            .map_err(|_| SecretProviderError::NotFound(binding.reference().clone()))?;
        SecretMaterial::new(material.into_bytes())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDeployment {
    #[serde(default)]
    environment: Vec<RawEnvironment>,
    #[serde(default)]
    secrets: Vec<RawSecret>,
    privacy: Option<RawPrivacy>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEnvironment {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSecret {
    reference: String,
    environment: String,
    version: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPrivacy {
    reference: String,
    processing_activity: String,
    purposes: Vec<String>,
    legal_bases: Vec<String>,
    special_category_condition: Option<String>,
    criminal_offence_authority: Option<String>,
    #[serde(default)]
    recipients: Vec<String>,
    #[serde(default)]
    transfer_safeguards: Vec<String>,
    retention: String,
}

/// Safe hosted-profile refusal. No variant contains secret material.
#[derive(Debug)]
pub enum DeploymentConfigError {
    /// The profile file could not be read.
    Read(std::io::Error),
    /// JSON syntax or its closed shape is invalid.
    Json(serde_json::Error),
    /// Typed core metadata is invalid.
    Profile(DeploymentError),
    /// One environment binding appears more than once.
    DuplicateEnvironment(EnvironmentName),
    /// One logical secret reference appears more than once.
    DuplicateSecret(SecretRef),
    /// M1 admits only run-scoped in-memory personal-data retention.
    UnsupportedRetention(String),
    /// Physical secret resolution failed without exposing material.
    Secret(SecretProviderError),
}

impl fmt::Display for DeploymentConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(error) => write!(formatter, "cannot read the deployment profile: {error}"),
            Self::Json(error) => write!(formatter, "invalid deployment JSON: {error}"),
            Self::Profile(error) => error.fmt(formatter),
            Self::DuplicateEnvironment(name) => write!(
                formatter,
                "deployment value `{}` is declared more than once",
                name.as_str()
            ),
            Self::DuplicateSecret(reference) => write!(
                formatter,
                "secret `{}` is declared more than once",
                reference.as_str()
            ),
            Self::UnsupportedRetention(retention) => write!(
                formatter,
                "privacy retention `{retention}` is unsupported; M1 requires `run_scoped`"
            ),
            Self::Secret(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for DeploymentConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedProvider;

    impl SecretProvider for FixedProvider {
        fn resolve(&self, binding: &SecretBinding) -> Result<SecretMaterial, SecretProviderError> {
            assert_eq!(binding.provider_path(), "API_TOKEN");
            SecretMaterial::new(b"private".to_vec())
        }
    }

    #[test]
    fn hosted_json_contains_coordinates_and_policy_but_never_secret_material() {
        let profile = parse(
            r#"{
  "environment": [{"name":"SERVICE_URL","value":"https://example.test"}],
  "secrets": [{
    "reference":"providers/http/token",
    "environment":"API_TOKEN",
    "version":"release-7"
  }],
  "privacy": {
    "reference":"privacy/support@1",
    "processing_activity":"ropa/support",
    "purposes":["purpose/support"],
    "legal_bases":["basis/contract"],
    "special_category_condition":null,
    "criminal_offence_authority":null,
    "recipients":[],
    "transfer_safeguards":[],
    "retention":"run_scoped"
  }
}"#,
        )
        .expect("the closed hosted profile parses");
        let boot = BootBindings::resolve(profile, &FixedProvider)
            .expect("the typed profile resolves atomically");
        let rendered = format!("{boot:?}");
        assert!(rendered.contains("providers/http/token"));
        assert!(!rendered.contains("private"));
        assert!(boot.attestation().privacy.is_some());
    }

    #[test]
    fn duplicate_or_open_profile_fields_refuse() {
        let duplicate = r#"{
          "environment":[
            {"name":"X","value":"a"},
            {"name":"X","value":"b"}
          ],
          "secrets":[]
        }"#;
        assert!(matches!(
            parse(duplicate),
            Err(DeploymentConfigError::DuplicateEnvironment(_))
        ));
        assert!(matches!(
            parse(r#"{"environment":[],"secrets":[],"surprise":1}"#),
            Err(DeploymentConfigError::Json(_))
        ));
    }
}
