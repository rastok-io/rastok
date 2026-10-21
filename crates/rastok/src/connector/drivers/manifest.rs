//! Hosted implementation names and declared Driver capabilities.
//!
//! Names select physical implementations in deployment configuration. They never
//! enter a plan, journal record or causal identity. A manifest describes what an
//! implementation claims; preparation verifies its actual Driver interfaces.

use crate::connector::{ConnectorDirection, ContractId, ExchangeKind};

/// A deployment name for one Driver implementation, independent of its Contract.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DriverName(String);

impl DriverName {
    /// Creates a nonempty name without whitespace or control characters.
    ///
    /// # Errors
    /// Returns a named refusal if the supplied name cannot be used in deployment configuration.
    pub fn new(name: impl Into<String>) -> Result<Self, DriverNameError> {
        let name = name.into();
        if name.is_empty() || name.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(DriverNameError);
        }
        Ok(Self(name))
    }

    /// Exact configured implementation name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for DriverName {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A Driver name was empty or contained whitespace or control characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriverNameError;

impl core::fmt::Display for DriverNameError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(
            "a Driver name must be nonempty and contain no whitespace or control characters",
        )
    }
}

impl std::error::Error for DriverNameError {}

/// Exact Contract and modes served by a named implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverManifestEntry {
    name: DriverName,
    connector: ContractId,
    roles: ConnectorDirection,
    exchange: ExchangeKind,
    mode_attribute: Option<String>,
    modes: Vec<String>,
}

impl DriverManifestEntry {
    /// Describes the implementation's protocol and supported mode words.
    /// An empty list means a Contract without a mode selector, never all modes.
    #[must_use]
    pub fn new(
        name: DriverName,
        connector: ContractId,
        roles: ConnectorDirection,
        exchange: ExchangeKind,
        mode_attribute: Option<String>,
        modes: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            name,
            connector,
            roles,
            exchange,
            mode_attribute,
            modes: modes.into_iter().collect(),
        }
    }

    /// Deployment implementation name.
    #[must_use]
    pub fn name(&self) -> &DriverName {
        &self.name
    }

    /// Exact semantic Contract identity.
    #[must_use]
    pub fn connector(&self) -> &ContractId {
        &self.connector
    }

    /// Inbound/outbound interfaces advertised by this implementation.
    #[must_use]
    pub const fn roles(&self) -> ConnectorDirection {
        self.roles
    }

    /// Protocol implemented by this factory's objects.
    #[must_use]
    pub const fn exchange(&self) -> ExchangeKind {
        self.exchange
    }

    /// Contract attribute which selects a mode, absent for a mode-less Contract.
    #[must_use]
    pub fn mode_attribute(&self) -> Option<&str> {
        self.mode_attribute.as_deref()
    }

    /// Supported mode words; empty only for an unselected Contract.
    #[must_use]
    pub fn modes(&self) -> &[String] {
        &self.modes
    }
}

/// Implementation capabilities in deterministic Contract/name order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverManifest {
    entries: Vec<DriverManifestEntry>,
}

impl DriverManifest {
    /// Orders implementation declarations and rejects duplicate names or modes.
    ///
    /// # Errors
    /// Repeated (Contract, implementation) pairs and invalid mode declarations refuse.
    pub fn try_new(
        entries: impl IntoIterator<Item = DriverManifestEntry>,
    ) -> Result<Self, DriverManifestError> {
        let mut entries: Vec<_> = entries.into_iter().collect();
        entries.sort_by(|a, b| (&a.connector, &a.name).cmp(&(&b.connector, &b.name)));
        for entry in &mut entries {
            entry.modes.sort();
            if entry
                .mode_attribute
                .as_ref()
                .is_some_and(|attribute| attribute.is_empty())
                || (entry.mode_attribute.is_some() == entry.modes.is_empty())
                || entry.modes.iter().any(|mode| mode.is_empty())
                || entry.modes.windows(2).any(|pair| pair[0] == pair[1])
            {
                return Err(DriverManifestError::InvalidModes {
                    connector: entry.connector.clone(),
                    name: entry.name.clone(),
                });
            }
        }
        for pair in entries.windows(2) {
            if pair[0].connector == pair[1].connector && pair[0].name == pair[1].name {
                return Err(DriverManifestError::Duplicate {
                    connector: pair[0].connector.clone(),
                    name: pair[0].name.clone(),
                });
            }
        }
        Ok(Self { entries })
    }

    /// Declared implementations; this never creates an object or performs I/O.
    #[must_use]
    pub fn entries(&self) -> &[DriverManifestEntry] {
        &self.entries
    }
}

/// Invalid or ambiguous implementation metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverManifestError {
    /// One implementation name was repeated for the same Contract.
    Duplicate {
        /// Exact Contract whose implementation is repeated.
        connector: ContractId,
        /// Repeated deployment implementation name.
        name: DriverName,
    },
    /// A declaration contains an empty or duplicate mode word.
    InvalidModes {
        /// Exact Contract whose modes are declared incorrectly.
        connector: ContractId,
        /// Implementation making the invalid claim.
        name: DriverName,
    },
}

impl core::fmt::Display for DriverManifestError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Duplicate { connector, name } => write!(
                formatter,
                "Driver manifest repeats `{name}` for `{connector}`"
            ),
            Self::InvalidModes { connector, name } => write!(
                formatter,
                "Driver `{name}` for `{connector}` has an empty or repeated mode word"
            ),
        }
    }
}

impl std::error::Error for DriverManifestError {}

#[cfg(test)]
mod tests {
    use super::{DriverManifest, DriverManifestEntry, DriverManifestError, DriverName};
    use crate::connector::{ConnectorDirection, ContractId, ExchangeKind};

    fn entry(name: &str, contract: &str, modes: &[&str]) -> DriverManifestEntry {
        DriverManifestEntry::new(
            DriverName::new(name).expect("a test implementation name"),
            ContractId::parse(contract).expect("an exact test Contract"),
            ConnectorDirection::Outbound,
            ExchangeKind::Call,
            (!modes.is_empty()).then(|| String::from("mode")),
            modes.iter().map(|mode| String::from(*mode)),
        )
    }

    #[test]
    fn a_manifest_describes_capabilities_without_loading_driver_code() {
        let process = entry("process-job", "std/process@4", &["job"]);
        let file = entry("file", "std/file@3", &["replace", "read"]);
        let manifest = DriverManifest::try_new([process, file]).expect("distinct capabilities");
        assert_eq!(manifest.entries()[0].connector().as_str(), "std/file@3");
        assert_eq!(manifest.entries()[0].name().as_str(), "file");
        assert_eq!(manifest.entries()[0].roles(), ConnectorDirection::Outbound);
        assert_eq!(manifest.entries()[0].exchange(), ExchangeKind::Call);
        assert_eq!(manifest.entries()[0].mode_attribute(), Some("mode"));
        assert_eq!(manifest.entries()[0].modes(), ["read", "replace"]);
        assert_eq!(manifest.entries()[1].connector().as_str(), "std/process@4");
        assert_eq!(manifest.entries()[1].modes(), ["job"]);
        assert!(
            !manifest
                .entries()
                .iter()
                .any(|item| item.connector().as_str() == "org.example/http@1")
        );
    }

    #[test]
    fn duplicate_manifest_claims_never_gain_an_ordering_policy() {
        let first = entry("process-job", "std/process@4", &["job"]);
        let mut second = first.clone();
        second.roles = ConnectorDirection::Inbound;
        let expected = DriverManifestError::Duplicate {
            connector: first.connector.clone(),
            name: first.name.clone(),
        };
        assert_eq!(
            DriverManifest::try_new([first.clone(), second.clone()]),
            Err(expected.clone())
        );
        assert_eq!(DriverManifest::try_new([second, first]), Err(expected));
    }

    #[test]
    fn two_implementations_of_the_same_contract_have_stable_manifest_order() {
        let first = entry("a", "std/file@3", &["read"]);
        let second = entry("b", "std/file@3", &["replace", "read"]);
        let forward =
            DriverManifest::try_new([first.clone(), second.clone()]).expect("two implementations");
        let reverse = DriverManifest::try_new([second, first]).expect("reverse registration");
        assert_eq!(forward, reverse);
        assert_eq!(
            forward
                .entries()
                .iter()
                .map(|item| item.name().as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
    }

    #[test]
    fn invalid_names_and_repeated_or_empty_modes_refuse() {
        for name in ["", " ", "two names", "line\nbreak", "control\u{007f}"] {
            assert!(DriverName::new(name).is_err(), "{name:?}");
        }
        assert!(DriverName::new("vendor.file-driver").is_ok());
        for modes in [&[""][..], &["read", "read"][..]] {
            assert!(matches!(
                DriverManifest::try_new([entry("file", "std/file@3", modes)]),
                Err(DriverManifestError::InvalidModes { .. })
            ));
        }
        let no_selector = DriverManifest::try_new([entry("http-client", "std/http-client@3", &[])])
            .expect("a Contract without a mode selector");
        assert!(no_selector.entries()[0].modes().is_empty());
        assert_eq!(no_selector.entries()[0].mode_attribute(), None);
        let mut missing_selector = entry("file", "std/file@3", &["read"]);
        missing_selector.mode_attribute = None;
        assert!(matches!(
            DriverManifest::try_new([missing_selector]),
            Err(DriverManifestError::InvalidModes { .. })
        ));
        let mut missing_modes = entry("http-client", "std/http-client@3", &[]);
        missing_modes.mode_attribute = Some(String::from("mode"));
        assert!(matches!(
            DriverManifest::try_new([missing_modes]),
            Err(DriverManifestError::InvalidModes { .. })
        ));
    }
}
