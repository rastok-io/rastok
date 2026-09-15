//! Declarative capabilities published before Driver implementation code loads.
//!
//! This is the semantic projection a future static-build description, native
//! plugin sidecar, or out-of-process plugin handshake must produce. It freezes
//! neither an artifact format nor a native ABI: only canonical Connector ids,
//! closed role claims, and duplicate refusal belong here.

use crate::connector::{ConnectorRoles, ContractId};

/// One Connector capability advertised by a physical Driver artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverManifestEntry {
    connector: ContractId,
    roles: ConnectorRoles,
}

impl DriverManifestEntry {
    /// Describes one exact contract implementation and its available roles.
    #[must_use]
    pub fn new(connector: ContractId, roles: ConnectorRoles) -> Self {
        Self { connector, roles }
    }

    /// Stable semantic contract identity, independent of the artifact path.
    #[must_use]
    pub fn connector(&self) -> &ContractId {
        &self.connector
    }

    /// Roles the artifact claims it can expose after loading and binding.
    #[must_use]
    pub const fn roles(&self) -> ConnectorRoles {
        self.roles
    }
}

/// Canonically ordered capability manifest for one physical Driver artifact.
///
/// The manifest contains no factory, function pointer, crate name, library
/// path, Host, Worker, trust decision, or loader policy. A host can therefore
/// inspect it without executing implementation code. Loading remains a later
/// physical step, and [`super::DriverRegistry`] independently verifies the
/// factory and its bound role views.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverManifest {
    entries: Vec<DriverManifestEntry>,
}

impl DriverManifest {
    /// Builds one canonical manifest and refuses ambiguous capability claims.
    ///
    /// # Errors
    ///
    /// Returns [`DriverManifestError::Duplicate`] when two entries advertise
    /// the same stable Connector id. Input order is never selection policy.
    pub fn try_new(
        entries: impl IntoIterator<Item = DriverManifestEntry>,
    ) -> Result<Self, DriverManifestError> {
        let mut entries: Vec<_> = entries.into_iter().collect();
        entries.sort_by(|left, right| left.connector.cmp(&right.connector));
        for pair in entries.windows(2) {
            if pair[0].connector == pair[1].connector {
                return Err(DriverManifestError::Duplicate(pair[0].connector.clone()));
            }
        }
        Ok(Self { entries })
    }

    /// Capabilities in canonical Connector-id order.
    #[must_use]
    pub fn entries(&self) -> &[DriverManifestEntry] {
        &self.entries
    }

    /// Looks up one advertised role set without loading a factory.
    #[must_use]
    pub fn roles_for(&self, connector: &ContractId) -> Option<ConnectorRoles> {
        self.entries
            .binary_search_by(|entry| entry.connector.cmp(connector))
            .ok()
            .map(|index| self.entries[index].roles)
    }
}

/// Why a declarative Driver capability manifest is ambiguous.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverManifestError {
    /// One physical artifact claimed the same semantic contract more than once.
    Duplicate(ContractId),
}

impl core::fmt::Display for DriverManifestError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Duplicate(id) => write!(formatter, "Driver manifest repeats `{id}`"),
        }
    }
}

impl std::error::Error for DriverManifestError {}

#[cfg(test)]
mod tests {
    use super::{DriverManifest, DriverManifestEntry, DriverManifestError};
    use crate::connector::{ConnectorRoles, ContractId};

    fn id(spelling: &str) -> ContractId {
        ContractId::parse(spelling).expect("the test Connector id is canonical")
    }

    #[test]
    fn a_manifest_describes_capabilities_without_loading_driver_code() {
        let process = id("rastok/process@4");
        let file = id("rastok/file@3");
        let manifest = DriverManifest::try_new([
            DriverManifestEntry::new(process.clone(), ConnectorRoles::Outbound),
            DriverManifestEntry::new(file.clone(), ConnectorRoles::Outbound),
        ])
        .expect("the capability claims are distinct");

        assert_eq!(manifest.entries()[0].connector(), &file);
        assert_eq!(manifest.entries()[1].connector(), &process);
        assert_eq!(manifest.roles_for(&file), Some(ConnectorRoles::Outbound));
        assert_eq!(manifest.roles_for(&process), Some(ConnectorRoles::Outbound));
        assert_eq!(manifest.roles_for(&id("org.example/http@1")), None);
    }

    #[test]
    fn duplicate_manifest_claims_never_gain_an_ordering_policy() {
        let process = id("rastok/process@4");
        assert_eq!(
            DriverManifest::try_new([
                DriverManifestEntry::new(process.clone(), ConnectorRoles::Inbound),
                DriverManifestEntry::new(process.clone(), ConnectorRoles::Outbound),
            ]),
            Err(DriverManifestError::Duplicate(process))
        );
    }
}
