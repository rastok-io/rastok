//! Available physical Driver implementations, checked before execution.
//!
//! Registration describes capabilities; it does not choose a Driver by insertion order.

use crate::connector::driver::Driver;
use crate::connector::{ConnectorDirection, ContractId, ExchangeKind};
use crate::{node, plan::ValidatedExecutionPlan};
use std::collections::BTreeMap;

/// Creates one plan-bound Driver object for a stable Connector contract.
pub trait DriverFactory {
    /// Stable contract identity implemented by this factory.
    fn contract_id(&self) -> &ContractId;

    /// Roles the implementation exposes.
    fn roles(&self) -> ConnectorDirection;

    /// Binds all selected plan Nodes of this contract into one implementation
    /// object. Runtime Driver preparation is the sole caller. Creation may reserve
    /// passive host resources whose availability must be proved atomically
    /// for the family (for example, all listener sockets of an inbound
    /// server). It must not acknowledge input, create causal facts, or perform
    /// an outbound effect; those actions remain behind the bound role views.
    ///
    /// # Errors
    ///
    /// Returns a factory-specific refusal before an Execution or journal exists.
    /// Binding may validate plan facts, allocate local state, and fail while
    /// reserving passive host resources. A reservation must remain bounded
    /// and externally unacknowledged until the runtime admits its input.
    fn create<'plan>(
        &self,
        plan: &'plan ValidatedExecutionPlan,
        nodes: &[node::Path],
    ) -> Result<Box<dyn Driver + 'plan>, DriverFactoryError>;

    /// Binds with deployment inputs already resolved and attested. The
    /// default keeps every existing family honest: it delegates only when no
    /// selected Node carries a boot reference. A family opens a boot setting
    /// by overriding this method and consuming it before returning.
    fn create_with_boot<'plan>(
        &self,
        plan: &'plan ValidatedExecutionPlan,
        nodes: &[node::Path],
        _boot: &crate::deployment::BootBindings,
    ) -> Result<Box<dyn Driver + 'plan>, DriverFactoryError> {
        for planned in &plan.plan().nodes {
            if !planned
                .executor_path()
                .is_some_and(|path| nodes.contains(path))
            {
                continue;
            }
            let Some(spec) = planned.connector() else {
                continue;
            };
            for (name, setting) in &spec.attributes {
                let crate::plan::AttrSetting::Expr { expr, .. } = setting else {
                    continue;
                };
                if crate::expr::boot_reference(&expr.root)
                    .map_err(|error| DriverFactoryError::new(error.to_string()))?
                    .is_some()
                {
                    return Err(DriverFactoryError::new(format!(
                        "the Driver for `{}` does not consume boot setting `{name}` at `{}`",
                        spec.contract, planned.path
                    )));
                }
            }
        }
        self.create(plan, nodes)
    }
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
    factories: BTreeMap<ContractId, Box<dyn DriverFactory>>,
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
        factory: Box<dyn DriverFactory>,
    ) -> Result<Self, DriverRegistrationError> {
        let id = factory.contract_id().clone();
        if self.factories.contains_key(&id) {
            return Err(DriverRegistrationError::Duplicate(id));
        }
        self.factories.insert(id, factory);
        Ok(self)
    }

    /// Projects the exact linked capabilities without loading or executing a
    /// second implementation path.
    ///
    /// # Errors
    ///
    /// Returns an error only if the registry's unique-id invariant is ever
    /// contradicted internally.
    pub fn manifest(&self) -> Result<super::DriverManifest, super::DriverManifestError> {
        super::DriverManifest::try_new(
            self.factories
                .iter()
                .map(|(id, factory)| super::DriverManifestEntry::new(id.clone(), factory.roles())),
        )
    }

    /// Resolves one exact id and proves that all required roles exist.
    ///
    /// # Errors
    ///
    /// Missing implementations and incomplete role sets remain distinct typed
    /// binding failures.
    pub fn require(
        &self,
        id: &ContractId,
        required: ConnectorDirection,
    ) -> Result<&dyn DriverFactory, DriverBindingError> {
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
    Duplicate(ContractId),
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
        id: ContractId,
    },
    /// An implementation exists but omits at least one required role.
    MissingRoles {
        /// Required contract.
        id: ContractId,
        /// Roles transported by the plan.
        required: ConnectorDirection,
        /// Roles advertised by the factory.
        available: ConnectorDirection,
    },
    /// The factory refused its selected plan Nodes before execution began.
    Factory {
        /// Stable contract id.
        id: ContractId,
        /// Factory-specific typed-boundary explanation.
        detail: String,
    },
    /// The object returned by a factory contradicts its advertised roles.
    BoundRolesMismatch {
        /// Stable contract id.
        id: ContractId,
        /// Factory claim used for deterministic selection.
        advertised: ConnectorDirection,
        /// Role views actually exposed; `None` means neither.
        actual: Option<ConnectorDirection>,
    },
    /// One stable contract id was assigned contradictory exchange facts by a
    /// forged plan.
    ConflictingExchange {
        /// Ambiguous semantic contract id.
        id: ContractId,
    },
    /// The bound outbound role exposes the wrong or an ambiguous exchange
    /// protocol view.
    BoundExchangeMismatch {
        /// Contract being bound.
        id: ContractId,
        /// Exchange carried by the validated plan.
        required: ExchangeKind,
        /// Sole exchange view actually exposed, if any.
        actual: Option<ExchangeKind>,
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
            Self::Factory { id, detail } => {
                write!(formatter, "Driver `{id}` refused binding: {detail}")
            }
            Self::BoundRolesMismatch {
                id,
                advertised,
                actual,
            } => match actual {
                Some(actual) => write!(
                    formatter,
                    "Driver `{id}` advertises {advertised}, but its bound object exposes {actual}"
                ),
                None => write!(
                    formatter,
                    "Driver `{id}` advertises {advertised}, but its bound object exposes no role"
                ),
            },
            Self::ConflictingExchange { id } => write!(
                formatter,
                "Connector contract `{id}` carries more than one exchange lifecycle"
            ),
            Self::BoundExchangeMismatch {
                id,
                required,
                actual,
            } => match actual {
                Some(actual) => write!(
                    formatter,
                    "Driver `{id}` exposes the {actual} exchange, but the plan requires {required}"
                ),
                None => write!(
                    formatter,
                    "Driver `{id}` does not expose exactly one {required} exchange view"
                ),
            },
        }
    }
}

impl std::error::Error for DriverBindingError {}
