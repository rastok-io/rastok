//! Physical Driver composition for one runtime build.
//!
//! The compiler's [`crate::contract::ContractCatalog`] owns semantic sheets.
//! This registry owns implementation factories.  Their stable Connector ids
//! meet only at binding; registration order is never selection policy.

use crate::boundary::{InboundConnectorDriver, OutboundConnectorDriver};
use crate::connector::{ConnectorId, ConnectorRoles};
use crate::node;
use crate::plan::ValidatedExecutionPlan;
use std::collections::BTreeMap;

/// One bound implementation object, potentially exposing both roles over the
/// same state.
///
/// Returning role views from one object makes the duplex invariant explicit:
/// a both-role factory cannot accidentally register two unrelated states.
pub trait BoundConnectorDriver {
    /// The inbound role view, when implemented.
    fn inbound(&mut self) -> Option<&mut dyn InboundConnectorDriver>;

    /// The outbound role view, when implemented.
    fn outbound(&mut self) -> Option<&mut dyn OutboundConnectorDriver>;
}

/// Creates one plan-bound Driver object for a stable Connector contract.
pub trait ConnectorDriverFactory {
    /// Stable contract identity implemented by this factory.
    fn connector_id(&self) -> &ConnectorId;

    /// Roles the implementation exposes.
    fn roles(&self) -> ConnectorRoles;

    /// Binds all selected plan Nodes of this contract into one implementation
    /// object. C2's global binder is the sole caller.
    ///
    /// # Errors
    ///
    /// Returns a factory-specific refusal before a VM or journal exists.
    fn bind<'plan>(
        &self,
        plan: &'plan ValidatedExecutionPlan,
        nodes: &[node::Path],
    ) -> Result<Box<dyn BoundConnectorDriver + 'plan>, DriverFactoryError>;
}

/// One factory-specific refusal kept behind the generic registry boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverFactoryError {
    detail: String,
}

impl DriverFactoryError {
    /// Preserves one implementation-specific binding explanation.
    #[must_use]
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }

    /// Human-readable implementation-specific detail.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl core::fmt::Display for DriverFactoryError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for DriverFactoryError {}

/// Runtime factories explicitly linked or loaded into one build.
#[derive(Default)]
pub struct DriverRegistry {
    factories: BTreeMap<ConnectorId, Box<dyn ConnectorDriverFactory>>,
}

impl core::fmt::Debug for DriverRegistry {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut map = formatter.debug_map();
        for (id, factory) in &self.factories {
            map.entry(id, &factory.roles());
        }
        map.finish()
    }
}

impl DriverRegistry {
    /// Adds one factory. Repeated ids refuse; registration order never wins.
    ///
    /// # Errors
    ///
    /// Returns [`DriverRegistrationError::Duplicate`] when the exact stable
    /// id already has an implementation.
    pub fn try_with(
        mut self,
        factory: Box<dyn ConnectorDriverFactory>,
    ) -> Result<Self, DriverRegistrationError> {
        let id = factory.connector_id().clone();
        if self.factories.contains_key(&id) {
            return Err(DriverRegistrationError::Duplicate(id));
        }
        self.factories.insert(id, factory);
        Ok(self)
    }

    /// Resolves one exact id and proves that all required roles exist.
    ///
    /// # Errors
    ///
    /// Missing implementations and incomplete role sets remain distinct typed
    /// binding failures.
    pub fn require(
        &self,
        id: &ConnectorId,
        required: ConnectorRoles,
    ) -> Result<&dyn ConnectorDriverFactory, DriverBindingError> {
        let Some(factory) = self.factories.get(id) else {
            return Err(DriverBindingError::Missing { id: id.clone() });
        };
        let available = factory.roles();
        if !available.includes(required) {
            return Err(DriverBindingError::MissingRoles {
                id: id.clone(),
                required,
                available,
            });
        }
        Ok(factory.as_ref())
    }
}

/// Why one factory cannot enter a deterministic Driver registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverRegistrationError {
    /// The id already has a factory; first-registration-wins is forbidden.
    Duplicate(ConnectorId),
}

impl core::fmt::Display for DriverRegistrationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Duplicate(id) => write!(
                formatter,
                "Driver `{id}` is registered more than once; selection is ambiguous"
            ),
        }
    }
}

impl std::error::Error for DriverRegistrationError {}

/// Why a validated Connector requirement has no unique physical Driver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverBindingError {
    /// No implementation of the exact stable contract id is registered.
    Missing {
        /// Required contract.
        id: ConnectorId,
    },
    /// An implementation exists but omits at least one required role.
    MissingRoles {
        /// Required contract.
        id: ConnectorId,
        /// Roles transported by the plan.
        required: ConnectorRoles,
        /// Roles advertised by the factory.
        available: ConnectorRoles,
    },
}

impl core::fmt::Display for DriverBindingError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Missing { id } => write!(formatter, "no Driver is registered for `{id}`"),
            Self::MissingRoles {
                id,
                required,
                available,
            } => write!(
                formatter,
                "Driver `{id}` exposes {available}, but the plan requires {required}"
            ),
        }
    }
}

impl std::error::Error for DriverBindingError {}

#[cfg(test)]
mod tests {
    use super::{
        BoundConnectorDriver, ConnectorDriverFactory, DriverBindingError, DriverFactoryError,
        DriverRegistrationError, DriverRegistry,
    };
    use crate::connector::{ConnectorId, ConnectorRoles};
    use crate::node;
    use crate::plan::ValidatedExecutionPlan;

    struct FakeFactory {
        id: ConnectorId,
        roles: ConnectorRoles,
    }

    impl ConnectorDriverFactory for FakeFactory {
        fn connector_id(&self) -> &ConnectorId {
            &self.id
        }

        fn roles(&self) -> ConnectorRoles {
            self.roles
        }

        fn bind<'plan>(
            &self,
            _plan: &'plan ValidatedExecutionPlan,
            _nodes: &[node::Path],
        ) -> Result<Box<dyn BoundConnectorDriver + 'plan>, DriverFactoryError> {
            Err(DriverFactoryError::new("the registry test never binds"))
        }
    }

    fn fake(id: &str, roles: ConnectorRoles) -> Box<dyn ConnectorDriverFactory> {
        Box::new(FakeFactory {
            id: ConnectorId::parse(id).unwrap(),
            roles,
        })
    }

    #[test]
    fn registration_is_exact_role_aware_and_never_first_wins() {
        let id = ConnectorId::parse("test/duplex@1").unwrap();
        let registry = DriverRegistry::default()
            .try_with(fake(id.as_str(), ConnectorRoles::Inbound))
            .unwrap();

        assert_eq!(
            registry
                .require(&id, ConnectorRoles::Inbound)
                .unwrap()
                .connector_id(),
            &id
        );
        assert_eq!(
            registry.require(&id, ConnectorRoles::Outbound).err(),
            Some(DriverBindingError::MissingRoles {
                id: id.clone(),
                required: ConnectorRoles::Outbound,
                available: ConnectorRoles::Inbound,
            })
        );
        assert_eq!(
            registry
                .require(
                    &ConnectorId::parse("test/missing@1").unwrap(),
                    ConnectorRoles::Outbound,
                )
                .err(),
            Some(DriverBindingError::Missing {
                id: ConnectorId::parse("test/missing@1").unwrap(),
            })
        );
        assert_eq!(
            registry
                .try_with(fake(id.as_str(), ConnectorRoles::InboundAndOutbound))
                .unwrap_err(),
            DriverRegistrationError::Duplicate(id),
        );
    }
}
