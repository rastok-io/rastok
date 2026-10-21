//! Available physical Driver implementations, checked before execution.
//!
//! Registration describes capabilities; it does not choose a Driver by insertion order.

use super::{
    DriverManifest, DriverManifestEntry, DriverManifestError, DriverName, DriverNameError,
};
use crate::connector::driver::Driver;
use crate::connector::{ConnectorDirection, ContractId, ExchangeKind};
use crate::{node, plan::ValidatedExecutionPlan};
use std::collections::BTreeMap;

/// Creates one plan-bound Driver object for a stable Connector contract.
pub trait DriverFactory {
    /// Deployment implementation name, independent of the semantic Contract id.
    fn name(&self) -> &str;

    /// Exchange protocol implemented by objects returned by this factory.
    fn exchange(&self) -> ExchangeKind;

    /// Exact supported mode words. Empty means a Contract without a mode selector.
    fn modes(&self) -> &[&str];

    /// Contract attribute selecting a mode. Standard Contracts derive it from
    /// their shared definition; nonstandard Contracts override it when needed.
    fn mode_attribute(&self) -> Option<&str> {
        crate::connector::stdlib::contract(self.contract_id())
            .and_then(|definition| definition.selector.as_ref())
            .map(|selector| selector.attribute)
    }

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
    factories: BTreeMap<(ContractId, DriverName), Box<dyn DriverFactory>>,
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
    /// Registers an implementation by exact Contract and deployment name.
    /// Registration order is never a selection rule.
    ///
    /// # Errors
    /// Invalid implementation metadata and duplicate names refuse registration.
    pub fn try_with(
        mut self,
        factory: Box<dyn DriverFactory>,
    ) -> Result<Self, DriverRegistrationError> {
        let name = DriverName::new(factory.name()).map_err(DriverRegistrationError::InvalidName)?;
        let id = factory.contract_id().clone();
        let key = (id.clone(), name.clone());
        if self.factories.contains_key(&key) {
            return Err(DriverRegistrationError::Duplicate { id, name });
        }
        if let Some(definition) = crate::connector::stdlib::contract(&id)
            && definition
                .selector
                .as_ref()
                .map(|selector| selector.attribute)
                != factory.mode_attribute()
        {
            return Err(DriverRegistrationError::ModeAttribute { id });
        }
        if self.factories.iter().any(|((contract, _), registered)| {
            contract == &id && registered.mode_attribute() != factory.mode_attribute()
        }) {
            return Err(DriverRegistrationError::ModeAttribute { id });
        }
        DriverManifest::try_new([DriverManifestEntry::new(
            name,
            id,
            factory.roles(),
            factory.exchange(),
            factory.mode_attribute().map(String::from),
            factory.modes().iter().map(|mode| String::from(*mode)),
        )])
        .map_err(DriverRegistrationError::Manifest)?;
        self.factories.insert(key, factory);
        Ok(self)
    }

    /// Describes registered capabilities without constructing any Driver object.
    ///
    /// # Errors
    /// Refuses contradictory capability declarations.
    pub fn manifest(&self) -> Result<DriverManifest, DriverManifestError> {
        DriverManifest::try_new(self.factories.iter().map(|((id, name), factory)| {
            DriverManifestEntry::new(
                name.clone(),
                id.clone(),
                factory.roles(),
                factory.exchange(),
                factory.mode_attribute().map(String::from),
                factory.modes().iter().map(|mode| String::from(*mode)),
            )
        }))
    }

    // Registration proves that every implementation of one Contract agrees on
    // its selector. The lookup does not select an implementation by order.
    pub(crate) fn mode_attribute(&self, id: &ContractId) -> Option<&str> {
        self.factories
            .iter()
            .find(|((contract, _), _)| contract == id)
            .and_then(|(_, factory)| factory.mode_attribute())
    }

    /// Selects the requested implementation, or the sole compatible implementation.
    /// Mode, direction and exchange must all match before a factory is called.
    ///
    /// # Errors
    /// Absence, ambiguity and incompatible mode/protocol capabilities remain distinct.
    pub fn require(
        &self,
        id: &ContractId,
        required: ConnectorDirection,
        exchange: ExchangeKind,
        mode: Option<&str>,
        selected: Option<&DriverName>,
    ) -> Result<(&DriverName, &dyn DriverFactory), DriverBindingError> {
        let candidates: Vec<_> = self
            .factories
            .iter()
            .filter(|((contract, _), _)| contract == id)
            .collect();
        if candidates.is_empty() {
            return Err(DriverBindingError::Missing { id: id.clone() });
        }
        let candidates: Vec<_> = candidates
            .into_iter()
            .filter(|((_, name), _)| selected.is_none_or(|selected| selected == name))
            .collect();
        if candidates.is_empty() {
            return match selected {
                Some(name) => Err(DriverBindingError::UnknownImplementation {
                    id: id.clone(),
                    name: name.clone(),
                }),
                None => Err(DriverBindingError::Missing { id: id.clone() }),
            };
        }
        let mode_matches = |factory: &dyn DriverFactory| match mode {
            Some(word) => factory.modes().contains(&word),
            None => factory.modes().is_empty(),
        };
        let compatible: Vec<_> = candidates
            .iter()
            .copied()
            .filter(|(_, factory)| {
                factory.roles().includes(required)
                    && factory.exchange() == exchange
                    && mode_matches(factory.as_ref())
            })
            .collect();
        match compatible.as_slice() {
            [candidate] => {
                let ((_, name), factory) = *candidate;
                Ok((name, factory.as_ref()))
            }
            [] => match candidates.as_slice() {
                [(_, factory)] if !factory.roles().includes(required) => {
                    Err(DriverBindingError::MissingRoles {
                        id: id.clone(),
                        required,
                        available: factory.roles(),
                    })
                }
                [((_, name), factory)] if factory.exchange() != exchange => {
                    Err(DriverBindingError::IncompatibleExchange {
                        id: id.clone(),
                        name: name.clone(),
                        required: exchange,
                        available: factory.exchange(),
                    })
                }
                [((_, name), _)] => Err(DriverBindingError::UnsupportedMode {
                    id: id.clone(),
                    name: name.clone(),
                    mode: mode.map(String::from),
                }),
                _ => Err(DriverBindingError::NoCompatibleImplementation {
                    id: id.clone(),
                    required,
                    exchange,
                    mode: mode.map(String::from),
                }),
            },
            _ => Err(DriverBindingError::Ambiguous {
                id: id.clone(),
                implementations: compatible
                    .iter()
                    .map(|((_, name), _)| name.clone())
                    .collect(),
            }),
        }
    }
}

/// Why one implementation cannot enter the deterministic registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverRegistrationError {
    /// The same implementation name already serves this exact Contract.
    Duplicate {
        /// Semantic Contract served by the repeated implementation.
        id: ContractId,
        /// Repeated deployment implementation name.
        name: DriverName,
    },
    /// The implementation name cannot be used in deployment configuration.
    InvalidName(DriverNameError),
    /// Implementations of one Contract disagree on its declared mode selector.
    ModeAttribute {
        /// Contract whose selector metadata is contradictory.
        id: ContractId,
    },
    /// Advertised modes are contradictory or invalid.
    Manifest(DriverManifestError),
}

impl core::fmt::Display for DriverRegistrationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Duplicate { id, name } => {
                write!(f, "Driver `{name}` for `{id}` is registered more than once")
            }
            Self::InvalidName(error) => error.fmt(f),
            Self::ModeAttribute { id } => write!(
                f,
                "Drivers for `{id}` disagree on the Contract's mode selector"
            ),
            Self::Manifest(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for DriverRegistrationError {}

/// Why a validated Connector requirement has no unique compatible implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverBindingError {
    /// No implementation of the exact Contract is registered.
    Missing {
        /// Required semantic Contract.
        id: ContractId,
    },
    /// The only selected candidate omits a required direction.
    MissingRoles {
        /// Required semantic Contract.
        id: ContractId,
        /// Directions carried by the validated Node.
        required: ConnectorDirection,
        /// Directions advertised by the candidate.
        available: ConnectorDirection,
    },
    /// Configuration names no Connector in this plan.
    UnknownConnector(node::Path),
    /// The validated mode is not a static word.
    InvalidMode(node::Path),
    /// The selected name does not implement this Contract.
    UnknownImplementation {
        /// Required semantic Contract.
        id: ContractId,
        /// Deployment implementation selection.
        name: DriverName,
    },
    /// The selected object implements a different exchange protocol.
    IncompatibleExchange {
        /// Required semantic Contract.
        id: ContractId,
        /// Deployment implementation selection.
        name: DriverName,
        /// Protocol carried by the validated Node.
        required: ExchangeKind,
        /// Protocol advertised by the candidate.
        available: ExchangeKind,
    },
    /// The selected implementation does not serve the resolved mode.
    UnsupportedMode {
        /// Required semantic Contract.
        id: ContractId,
        /// Deployment implementation selection.
        name: DriverName,
        /// Effective mode word, absent for a mode-less Contract.
        mode: Option<String>,
    },
    /// Implementations exist, but none satisfies all required capabilities.
    NoCompatibleImplementation {
        /// Required semantic Contract.
        id: ContractId,
        /// Directions carried by the validated Node.
        required: ConnectorDirection,
        /// Protocol carried by the validated Node.
        exchange: ExchangeKind,
        /// Effective mode word, absent for a mode-less Contract.
        mode: Option<String>,
    },
    /// More than one compatible implementation requires explicit selection.
    Ambiguous {
        /// Required semantic Contract.
        id: ContractId,
        /// Compatible candidates in deterministic name order.
        implementations: Vec<DriverName>,
    },
    /// The selected factory refused its planned Nodes before execution.
    Factory {
        /// Required semantic Contract.
        id: ContractId,
        /// Implementation which refused construction.
        name: DriverName,
        /// Factory-specific boundary explanation.
        detail: String,
    },
    /// The created object contradicts its declared directions.
    BoundRolesMismatch {
        /// Required semantic Contract.
        id: ContractId,
        /// Implementation whose object contradicted its metadata.
        name: DriverName,
        /// Advertised directions used for selection.
        advertised: ConnectorDirection,
        /// Directions exposed by the object; absent means neither.
        actual: Option<ConnectorDirection>,
    },
    /// The created object exposes a missing, wrong or ambiguous exchange view.
    BoundExchangeMismatch {
        /// Required semantic Contract.
        id: ContractId,
        /// Implementation whose object contradicted its metadata.
        name: DriverName,
        /// Protocol carried by the validated Node.
        required: ExchangeKind,
        /// Sole protocol exposed by the object, if any.
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
                "Driver for `{id}` exposes {available}, but the plan requires {required}"
            ),
            Self::UnknownConnector(path) => write!(
                formatter,
                "Driver selection names no planned Connector `{path}`"
            ),
            Self::InvalidMode(path) => {
                write!(formatter, "Connector `{path}` has no static mode word")
            }
            Self::UnknownImplementation { id, name } => {
                write!(formatter, "Driver `{name}` is not registered for `{id}`")
            }
            Self::IncompatibleExchange {
                id,
                name,
                required,
                available,
            } => write!(
                formatter,
                "Driver `{name}` for `{id}` implements {available}, but the plan requires {required}"
            ),
            Self::UnsupportedMode { id, name, mode } => write!(
                formatter,
                "Driver `{name}` for `{id}` does not serve mode {}",
                mode.as_deref().unwrap_or("<none>")
            ),
            Self::NoCompatibleImplementation {
                id,
                required,
                exchange,
                mode,
            } => write!(
                formatter,
                "no Driver for `{id}` serves {required} {exchange}, mode {}",
                mode.as_deref().unwrap_or("<none>")
            ),
            Self::Ambiguous {
                id,
                implementations,
            } => write!(
                formatter,
                "multiple Drivers serve `{id}` ({}); select an implementation for this Connector",
                implementations
                    .iter()
                    .map(DriverName::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Factory { id, name, detail } => write!(
                formatter,
                "Driver `{name}` for `{id}` refused preparation: {detail}"
            ),
            Self::BoundRolesMismatch {
                id,
                name,
                advertised,
                actual,
            } => match actual {
                Some(actual) => write!(
                    formatter,
                    "Driver `{name}` for `{id}` advertises {advertised}, but its object exposes {actual}"
                ),
                None => write!(
                    formatter,
                    "Driver `{name}` for `{id}` advertises {advertised}, but its object exposes no direction"
                ),
            },
            Self::BoundExchangeMismatch {
                id,
                name,
                required,
                actual,
            } => match actual {
                Some(actual) => write!(
                    formatter,
                    "Driver `{name}` for `{id}` exposes {actual}, but the plan requires {required}"
                ),
                None => write!(
                    formatter,
                    "Driver `{name}` for `{id}` exposes no unique {required} exchange view"
                ),
            },
        }
    }
}

impl std::error::Error for DriverBindingError {}
