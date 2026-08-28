//! Physical Driver composition for one runtime build.
//!
//! The compiler's [`crate::contract::ContractCatalog`] owns semantic sheets.
//! This registry owns implementation factories.  Their stable Connector ids
//! meet only at binding; registration order is never selection policy.

use crate::boundary::{
    CallConnectorDriver, CallRef, CallStart, Completion, DriverRouteError,
    InboundCallResponseStart, InboundCallTimeoutStart, InboundConnectorDriver, InboundDecision,
    InboundExchangeRef, InboundProposal, Intent, OutboundConnectorBindings,
    OutboundConnectorDriver, RequestPreparation, Response, SchedulerCompletion,
    SessionConnectorBindings, SessionConnectorDriver, SessionDriverEvent, SessionOpenIntent,
    SessionRef, SessionWriteIntent,
};
use crate::connector::{ConnectorExchange, ConnectorId, ConnectorRoles};
use crate::envelope::{Data, Envelope};
use crate::node;
use crate::plan::ValidatedExecutionPlan;
use crate::runtime::vm::{InboundCallPumpReport, Vm, VmError};
use crate::{boundary::InboundAdmission, journal::Journal};
use std::collections::{BTreeMap, BTreeSet};

/// One bound implementation object, potentially exposing both roles over the
/// same state.
///
/// Returning role views from one object makes the duplex invariant explicit:
/// a both-role factory cannot accidentally register two unrelated states.
pub trait BoundConnectorDriver {
    /// The inbound role view, when implemented. Presence is immutable for the
    /// lifetime of this bound object; returning `None` after binding is an
    /// integrity fault.
    fn inbound(&mut self) -> Option<&mut dyn InboundConnectorDriver>;

    /// The outbound role view under the same stable-presence invariant.
    fn outbound(&mut self) -> Option<&mut dyn OutboundConnectorDriver>;
}

/// Creates one plan-bound Driver object for a stable Connector contract.
pub trait ConnectorDriverFactory {
    /// Stable contract identity implemented by this factory.
    fn connector_id(&self) -> &ConnectorId;

    /// Roles the implementation exposes.
    fn roles(&self) -> ConnectorRoles;

    /// Binds all selected plan Nodes of this contract into one implementation
    /// object. C2's global binder is the sole caller. Binding may reserve
    /// passive host resources whose availability must be proved atomically
    /// for the family (for example, all listener sockets of an inbound
    /// server). It must not acknowledge input, mint causal facts, or perform
    /// an outbound effect; those actions remain behind the bound role views.
    ///
    /// # Errors
    ///
    /// Returns a factory-specific refusal before a VM or journal exists.
    /// Binding may validate plan facts, allocate local state, and fail while
    /// reserving passive host resources. A reservation must remain bounded
    /// and externally unacknowledged until the runtime admits its input.
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

#[derive(Debug)]
struct FamilyRequirement {
    nodes: Vec<(node::Path, ConnectorRoles, ConnectorExchange)>,
    required: ConnectorRoles,
}

struct BoundFamily<'plan> {
    driver: Box<dyn BoundConnectorDriver + 'plan>,
    uses_inbound: bool,
    inbound_paths: BTreeSet<node::Path>,
    exchange: ConnectorExchange,
    pending_inbound: Option<InboundProposal>,
    outbound_path: Option<node::Path>,
    session_path: Option<node::Path>,
}

/// One retained proposal paired with the exact Driver family that owns it.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutedInboundProposal {
    connector: ConnectorId,
    proposal: InboundProposal,
}

/// Failure of one inbound Driver/VM boundary round.
#[derive(Debug, Clone, PartialEq)]
pub enum InboundPumpError {
    /// The bound Driver registry contradicted its retained routing state.
    Driver(DriverRouteError),
    /// The VM or journal raised a fail-stop integrity fault.
    Machine(VmError),
}

/// One bounded pass offering already-journaled graph responses to inbound
/// Call Drivers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InboundCallResponsePumpReport {
    /// Responses whose protocol write is now Driver-owned.
    pub started: usize,
    /// Responses retained by the VM because the Driver's finite capacity is
    /// currently full.
    pub pending: usize,
}

/// One bounded pass transferring scheduler-owned inbound request expiries to
/// their exact Driver family.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InboundCallTimeoutPumpReport {
    /// Timeout attempts whose protocol action is now Driver-owned.
    pub started: usize,
    /// Timeout attempts retained by the VM under finite Driver pressure.
    pub pending: usize,
}

impl core::fmt::Display for InboundPumpError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Driver(error) => error.fmt(formatter),
            Self::Machine(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for InboundPumpError {}

impl RoutedInboundProposal {
    /// Stable family that must receive the eventual decision callback.
    #[must_use]
    pub fn connector(&self) -> &ConnectorId {
        &self.connector
    }

    /// Retained proposal presented for atomic VM admission.
    #[must_use]
    pub fn proposal(&self) -> &InboundProposal {
        &self.proposal
    }
}

/// Complete runtime association between active plan Connectors and physical
/// Drivers.
///
/// Construction is the only registry lookup. Thereafter the VM routes by
/// exact Node path and role; no concrete contract name or registration order
/// participates in execution.
pub struct RuntimeBindings<'plan> {
    families: BTreeMap<ConnectorId, BoundFamily<'plan>>,
    /// Last inbound family polled. Cycling from this key prevents one
    /// backpressured transport from blocking an unrelated family.
    inbound_cursor: Option<ConnectorId>,
    outbound: BTreeMap<node::Path, ConnectorId>,
    calls: BTreeMap<CallRef, ConnectorId>,
    inbound_calls: BTreeMap<crate::boundary::InboundCallRef, ConnectorId>,
    session_outbound: BTreeMap<node::Path, ConnectorId>,
    sessions: BTreeMap<SessionRef, ConnectorId>,
}

impl core::fmt::Debug for RuntimeBindings<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RuntimeBindings")
            .field("families", &self.families.keys().collect::<Vec<_>>())
            .field("inbound_cursor", &self.inbound_cursor)
            .field("outbound", &self.outbound)
            .field("calls", &self.calls)
            .field("inbound_calls", &self.inbound_calls)
            .field("session_outbound", &self.session_outbound)
            .field("sessions", &self.sessions)
            .finish()
    }
}

impl<'plan> RuntimeBindings<'plan> {
    /// Whether this statically bound image contains at least one external
    /// source that must be polled even when no CLI submission is pending.
    pub fn has_runnable_inbound(&self) -> bool {
        self.families.values().any(|family| family.uses_inbound)
    }

    /// Binds every runnable inbound Connector and every callable outbound
    /// Connector before a VM or journal exists.
    ///
    /// An outbound-only Node with no incoming edge cannot open a call and is
    /// deliberately skipped. An inbound-capable Node is runnable by role and
    /// therefore binds even when no graph edge targets it.
    ///
    /// # Errors
    ///
    /// Missing, role-incomplete, lying, or factory-refused implementations
    /// remain distinct typed failures.
    pub fn try_new(
        plan: &'plan ValidatedExecutionPlan,
        registry: &DriverRegistry,
    ) -> Result<Self, DriverBindingError> {
        let mut requirements: BTreeMap<ConnectorId, FamilyRequirement> = BTreeMap::new();
        for (index, planned) in plan.plan().nodes.iter().enumerate() {
            let Some(spec) = planned.connector() else {
                continue;
            };
            let callable = plan.plan().edges.iter().any(|edge| {
                edge.to
                    .as_ref()
                    .and_then(|endpoint| endpoint.node)
                    .is_some_and(|slot| slot.index() == index)
            });
            if !spec.roles.has_inbound() && !(spec.roles.has_outbound() && callable) {
                continue;
            }
            requirements
                .entry(spec.contract.clone())
                .and_modify(|requirement| {
                    requirement.required = union_roles(requirement.required, spec.roles);
                    requirement
                        .nodes
                        .push((planned.path.clone(), spec.roles, spec.exchange));
                })
                .or_insert_with(|| FamilyRequirement {
                    nodes: vec![(planned.path.clone(), spec.roles, spec.exchange)],
                    required: spec.roles,
                });
        }

        let mut families = BTreeMap::new();
        let mut outbound = BTreeMap::new();
        let mut session_outbound = BTreeMap::new();
        for (id, requirement) in requirements {
            let factory = registry.require(&id, requirement.required)?;
            let paths: Vec<_> = requirement
                .nodes
                .iter()
                .map(|(path, _, _)| path.clone())
                .collect();
            let exchange = requirement.nodes[0].2;
            if requirement
                .nodes
                .iter()
                .any(|(_, _, candidate)| *candidate != exchange)
            {
                return Err(DriverBindingError::ConflictingExchange { id });
            }
            let mut driver =
                factory
                    .bind(plan, &paths)
                    .map_err(|error| DriverBindingError::Factory {
                        id: id.clone(),
                        detail: error.detail().to_string(),
                    })?;
            let actual = roles_of(driver.as_mut());
            let advertised = factory.roles();
            if actual != Some(advertised) {
                return Err(DriverBindingError::BoundRolesMismatch {
                    id,
                    advertised,
                    actual,
                });
            }
            if requirement.required.has_outbound() {
                let actual_exchange = outbound_exchange_of(driver.as_mut());
                if actual_exchange != Some(exchange) {
                    return Err(DriverBindingError::BoundExchangeMismatch {
                        id,
                        required: exchange,
                        actual: actual_exchange,
                    });
                }
            }
            if requirement.required.has_inbound() && exchange == ConnectorExchange::Call {
                let has_call_view = driver
                    .inbound()
                    .and_then(InboundConnectorDriver::call)
                    .is_some();
                if !has_call_view {
                    return Err(DriverBindingError::BoundExchangeMismatch {
                        id,
                        required: ConnectorExchange::Call,
                        actual: None,
                    });
                }
            }

            let uses_inbound = requirement.required.has_inbound();
            let inbound_paths = requirement
                .nodes
                .iter()
                .filter_map(|(path, roles, _)| roles.has_inbound().then_some(path.clone()))
                .collect();
            let mut outbound_path = None;
            let mut session_path = None;
            for (path, roles, _) in requirement.nodes {
                if roles.has_outbound() && exchange == ConnectorExchange::Call {
                    outbound_path.get_or_insert_with(|| path.clone());
                    outbound.insert(path, id.clone());
                } else if roles.has_outbound() && exchange == ConnectorExchange::Session {
                    session_path.get_or_insert_with(|| path.clone());
                    session_outbound.insert(path, id.clone());
                }
            }
            families.insert(
                id,
                BoundFamily {
                    driver,
                    uses_inbound,
                    inbound_paths,
                    exchange,
                    pending_inbound: None,
                    outbound_path,
                    session_path,
                },
            );
        }
        Ok(Self {
            families,
            inbound_cursor: None,
            outbound,
            calls: BTreeMap::new(),
            inbound_calls: BTreeMap::new(),
            session_outbound,
            sessions: BTreeMap::new(),
        })
    }

    /// Polls runnable inbound families in stable Connector-id order.
    ///
    /// # Errors
    ///
    /// A role view proven during binding disappeared.
    pub fn next_inbound(&mut self) -> Result<Option<RoutedInboundProposal>, DriverRouteError> {
        let ids: Vec<_> = self
            .families
            .iter()
            .filter_map(|(id, family)| family.uses_inbound.then_some(id.clone()))
            .collect();
        let start = self
            .inbound_cursor
            .as_ref()
            .and_then(|cursor| ids.iter().position(|id| id == cursor))
            .map_or(0, |position| position.saturating_add(1) % ids.len().max(1));
        for offset in 0..ids.len() {
            let id = &ids[(start + offset) % ids.len()];
            let Some(family) = self.families.get_mut(id) else {
                // `ids` was derived from this map without mutating it. Keep
                // the public boundary total even if that local invariant is
                // accidentally broken later.
                continue;
            };
            if let Some(proposal) = &family.pending_inbound {
                self.inbound_cursor = Some(id.clone());
                return Ok(Some(RoutedInboundProposal {
                    connector: id.clone(),
                    proposal: proposal.clone(),
                }));
            }
            let Some(driver) = family.driver.inbound() else {
                return Err(DriverRouteError::UnavailableInbound(id.clone()));
            };
            if let Some(proposal) = driver.proposal() {
                let source = proposal.submission.provenance.connector_path();
                if !family.inbound_paths.contains(source) {
                    return Err(DriverRouteError::InboundSourceMismatch {
                        connector: id.clone(),
                        source: source.clone(),
                    });
                }
                let framing_matches = match family.exchange {
                    ConnectorExchange::Event | ConnectorExchange::Call => {
                        proposal.session.is_none()
                    }
                    ConnectorExchange::Session => proposal.session.is_some(),
                };
                if !framing_matches {
                    return Err(DriverRouteError::InboundExchangeMismatch {
                        connector: id.clone(),
                        exchange: family.exchange,
                    });
                }
                family.pending_inbound = Some(proposal.clone());
                self.inbound_cursor = Some(id.clone());
                return Ok(Some(RoutedInboundProposal {
                    connector: id.clone(),
                    proposal,
                }));
            }
        }
        Ok(None)
    }

    /// Resolves one retained proposal after committed acceptance or a typed
    /// contract rejection. Pending admission makes no callback.
    ///
    /// # Errors
    ///
    /// Refuses a stale, crossed-family, or otherwise unknown proposal.
    pub fn decide_inbound(
        &mut self,
        routed: &RoutedInboundProposal,
        decision: InboundDecision,
    ) -> Result<(), DriverRouteError> {
        let Some(family) = self.families.get_mut(&routed.connector) else {
            return Err(DriverRouteError::UnknownInboundProposal {
                connector: routed.connector.clone(),
                reference: routed.proposal.reference,
            });
        };
        let Some(pending) = &family.pending_inbound else {
            return Err(DriverRouteError::UnknownInboundProposal {
                connector: routed.connector.clone(),
                reference: routed.proposal.reference,
            });
        };
        if pending != &routed.proposal {
            return Err(DriverRouteError::UnknownInboundProposal {
                connector: routed.connector.clone(),
                reference: routed.proposal.reference,
            });
        }
        if let InboundDecision::Accepted(accepted) = decision {
            match accepted.exchange {
                Some(InboundExchangeRef::Call(call_ref)) => {
                    if let Some(expected) = self.inbound_calls.get(&call_ref)
                        && expected != &routed.connector
                    {
                        return Err(DriverRouteError::CrossFamilyInboundCall {
                            call_ref,
                            expected: expected.clone(),
                            actual: routed.connector.clone(),
                        });
                    }
                    self.inbound_calls
                        .insert(call_ref, routed.connector.clone());
                }
                Some(InboundExchangeRef::Session(session)) => {
                    if let Some(expected) = self.sessions.get(&session)
                        && expected != &routed.connector
                    {
                        return Err(DriverRouteError::CrossFamilySession {
                            session,
                            expected: expected.clone(),
                            actual: routed.connector.clone(),
                        });
                    }
                    self.sessions.insert(session, routed.connector.clone());
                }
                None => {}
            }
        }
        let Some(driver) = family.driver.inbound() else {
            return Err(DriverRouteError::UnavailableInbound(
                routed.connector.clone(),
            ));
        };
        driver.decide(routed.proposal.reference, decision);
        family.pending_inbound = None;
        Ok(())
    }

    /// Runs one bounded inbound proposal through VM admission.
    ///
    /// Acceptance reaches the Driver only after the journal commit. Pending
    /// admission retains both the proposal and Driver-side transport unit;
    /// rejection releases it with a portable typed reason.
    ///
    /// # Errors
    ///
    /// Driver routing contradictions and fail-stop VM/journal faults remain
    /// distinct.
    pub fn pump_inbound(
        &mut self,
        vm: &mut Vm<'plan>,
        journal: &mut dyn Journal,
    ) -> Result<Option<InboundAdmission>, InboundPumpError> {
        let Some(routed) = self.next_inbound().map_err(InboundPumpError::Driver)? else {
            return Ok(None);
        };
        let admission = vm
            .admit_inbound(journal, routed.proposal())
            .map_err(InboundPumpError::Machine)?;
        match admission {
            InboundAdmission::Accepted(accepted) => self
                .decide_inbound(&routed, InboundDecision::Accepted(accepted))
                .map_err(InboundPumpError::Driver)?,
            InboundAdmission::Rejected(reason) => self
                .decide_inbound(&routed, InboundDecision::Rejected(reason))
                .map_err(InboundPumpError::Driver)?,
            InboundAdmission::Pending => {}
        }
        Ok(Some(admission))
    }

    /// Offers every ready graph response to the Driver family that admitted
    /// its inbound Call. The response stays retained and its Connector
    /// mailbox stays claimed while a bounded Driver reports `Pending`.
    ///
    /// # Errors
    ///
    /// A missing/crossed family or a Call view that disappeared after
    /// binding is an integrity fault.
    pub fn pump_inbound_call_responses(
        &mut self,
        vm: &mut Vm<'plan>,
    ) -> Result<InboundCallResponsePumpReport, DriverRouteError> {
        let mut report = InboundCallResponsePumpReport::default();
        for response in vm.ready_inbound_call_responses() {
            let Some(id) = self.inbound_calls.get(&response.call_ref).cloned() else {
                return Err(DriverRouteError::UncorrelatedInboundCall(response.call_ref));
            };
            let Some(family) = self.families.get_mut(&id) else {
                return Err(DriverRouteError::UncorrelatedInboundCall(response.call_ref));
            };
            if !family.inbound_paths.contains(&response.connector_path) {
                return Err(DriverRouteError::InboundSourceMismatch {
                    connector: id,
                    source: response.connector_path,
                });
            }
            let driver = family
                .driver
                .inbound()
                .ok_or_else(|| DriverRouteError::UnavailableInbound(id.clone()))?
                .call()
                .ok_or_else(|| DriverRouteError::UnavailableInboundCall(id.clone()))?;
            match driver.start_response(&response) {
                InboundCallResponseStart::Started => {
                    if !vm.mark_inbound_call_response_started(&response) {
                        return Err(DriverRouteError::UncorrelatedInboundCall(response.call_ref));
                    }
                    report.started = report.started.saturating_add(1);
                }
                InboundCallResponseStart::Pending => {
                    report.pending = report.pending.saturating_add(1);
                }
            }
        }
        Ok(report)
    }

    /// Offers every due inbound request deadline to the Driver family that
    /// admitted the Call. A `Pending` attempt remains scheduler-owned and is
    /// offered again without creating another queue or terminal fact.
    ///
    /// # Errors
    ///
    /// Missing or crossed family ownership is a binding-integrity fault.
    pub fn pump_inbound_call_timeouts(
        &mut self,
        vm: &mut Vm<'plan>,
    ) -> Result<InboundCallTimeoutPumpReport, DriverRouteError> {
        let mut report = InboundCallTimeoutPumpReport::default();
        for timeout in vm.ready_inbound_call_timeouts() {
            let Some(id) = self.inbound_calls.get(&timeout.call_ref).cloned() else {
                return Err(DriverRouteError::UncorrelatedInboundCall(timeout.call_ref));
            };
            let Some(family) = self.families.get_mut(&id) else {
                return Err(DriverRouteError::UncorrelatedInboundCall(timeout.call_ref));
            };
            if !family.inbound_paths.contains(&timeout.connector_path) {
                return Err(DriverRouteError::InboundSourceMismatch {
                    connector: id,
                    source: timeout.connector_path,
                });
            }
            let driver = family
                .driver
                .inbound()
                .ok_or_else(|| DriverRouteError::UnavailableInbound(id.clone()))?
                .call()
                .ok_or_else(|| DriverRouteError::UnavailableInboundCall(id.clone()))?;
            match driver.start_timeout(&timeout) {
                InboundCallTimeoutStart::Started => {
                    if !vm.mark_inbound_call_timeout_started(&timeout) {
                        return Err(DriverRouteError::UncorrelatedInboundCall(timeout.call_ref));
                    }
                    report.started = report.started.saturating_add(1);
                }
                InboundCallTimeoutStart::Pending => {
                    report.pending = report.pending.saturating_add(1);
                }
            }
        }
        Ok(report)
    }

    /// Drains terminal observations from every inbound Call family and
    /// offers them to the VM's typed, close-once queue. Family correlation is
    /// checked before the VM sees an event; invalid facts remain non-causal.
    ///
    /// # Errors
    ///
    /// A bound Call view disappeared or a family reported another family's
    /// Call reference.
    pub fn pump_inbound_call_events(
        &mut self,
        vm: &mut Vm<'plan>,
    ) -> Result<InboundCallPumpReport, DriverRouteError> {
        let ids: Vec<_> = self
            .families
            .iter()
            .filter_map(|(id, family)| {
                (family.uses_inbound && family.exchange == ConnectorExchange::Call)
                    .then_some(id.clone())
            })
            .collect();
        let mut report = InboundCallPumpReport::default();
        for id in ids {
            let events = {
                let Some(family) = self.families.get_mut(&id) else {
                    continue;
                };
                let driver = family
                    .driver
                    .inbound()
                    .ok_or_else(|| DriverRouteError::UnavailableInbound(id.clone()))?
                    .call()
                    .ok_or_else(|| DriverRouteError::UnavailableInboundCall(id.clone()))?;
                driver.events()
            };
            for event in events {
                let Some(expected) = self.inbound_calls.get(&event.call_ref).cloned() else {
                    report.uncorrelated.push(event);
                    continue;
                };
                if expected != id {
                    return Err(DriverRouteError::CrossFamilyInboundCall {
                        call_ref: event.call_ref,
                        expected,
                        actual: id,
                    });
                }
                let call_ref = event.call_ref;
                let mut observed = vm.queue_inbound_call_events(core::iter::once(event));
                if observed.queued == 1 || !observed.uncorrelated.is_empty() {
                    self.inbound_calls.remove(&call_ref);
                }
                report.queued = report.queued.saturating_add(observed.queued);
                report.discarded.append(&mut observed.discarded);
                report.uncorrelated.append(&mut observed.uncorrelated);
                report.rejected.append(&mut observed.rejected);
            }
        }
        Ok(report)
    }

    fn outbound_driver(
        &mut self,
        connector: &node::Path,
    ) -> Result<&mut dyn CallConnectorDriver, DriverRouteError> {
        let Some(id) = self.outbound.get(connector) else {
            return Err(DriverRouteError::UnboundOutbound(connector.clone()));
        };
        let Some(family) = self.families.get_mut(id) else {
            return Err(DriverRouteError::UnboundOutbound(connector.clone()));
        };
        let outbound = family
            .driver
            .outbound()
            .ok_or_else(|| DriverRouteError::UnavailableOutbound(connector.clone()))?;
        outbound
            .call()
            .ok_or_else(|| DriverRouteError::UnavailableCall(connector.clone()))
    }

    fn session_driver(
        &mut self,
        connector: &node::Path,
    ) -> Result<(ConnectorId, &mut dyn SessionConnectorDriver), DriverRouteError> {
        let id = self.session_family(connector)?;
        let Some(family) = self.families.get_mut(&id) else {
            return Err(DriverRouteError::UnboundSession(connector.clone()));
        };
        let outbound = family
            .driver
            .outbound()
            .ok_or_else(|| DriverRouteError::UnavailableOutbound(connector.clone()))?;
        let driver = outbound
            .session()
            .ok_or_else(|| DriverRouteError::UnavailableSession(connector.clone()))?;
        Ok((id, driver))
    }

    fn session_family(&self, connector: &node::Path) -> Result<ConnectorId, DriverRouteError> {
        self.session_outbound
            .get(connector)
            .cloned()
            .ok_or_else(|| DriverRouteError::UnboundSession(connector.clone()))
    }
}

impl OutboundConnectorBindings for RuntimeBindings<'_> {
    fn prepare_request(
        &mut self,
        connector: &node::Path,
        data: &Data,
        resolved_settings: &BTreeMap<String, crate::value::Value>,
    ) -> Result<RequestPreparation, DriverRouteError> {
        Ok(self
            .outbound_driver(connector)?
            .prepare_request(connector, data, resolved_settings))
    }

    fn scheduler_completion(
        &mut self,
        connector: &node::Path,
        completion: SchedulerCompletion,
    ) -> Result<Completion, DriverRouteError> {
        Ok(self
            .outbound_driver(connector)?
            .scheduler_completion(connector, completion))
    }

    fn cancel(
        &mut self,
        connector: &node::Path,
        call_ref: &CallRef,
    ) -> Result<(), DriverRouteError> {
        self.outbound_driver(connector)?.cancel(call_ref);
        self.calls.remove(call_ref);
        Ok(())
    }

    fn start(
        &mut self,
        intent: &Intent,
        request: &Envelope,
    ) -> Result<CallStart, DriverRouteError> {
        let connector = &intent.connector_path;
        let Some(id) = self.outbound.get(connector).cloned() else {
            return Err(DriverRouteError::UnboundOutbound(connector.clone()));
        };
        let start = self.outbound_driver(connector)?.start(intent, request);
        if start == CallStart::Started {
            self.calls.insert(intent.call_ref, id);
        }
        Ok(start)
    }

    fn events(&mut self) -> Result<Vec<Response>, DriverRouteError> {
        let mut responses = Vec::new();
        for (id, family) in &mut self.families {
            let Some(path) = &family.outbound_path else {
                continue;
            };
            let Some(driver) = family.driver.outbound() else {
                return Err(DriverRouteError::UnavailableOutbound(path.clone()));
            };
            let Some(driver) = driver.call() else {
                return Err(DriverRouteError::UnavailableCall(path.clone()));
            };
            for response in driver.events() {
                if let Some(expected) = self.calls.get(&response.call_ref)
                    && expected != id
                {
                    return Err(DriverRouteError::CrossFamilyResponse {
                        call_ref: response.call_ref,
                        expected: expected.clone(),
                        actual: id.clone(),
                    });
                }
                self.calls.remove(&response.call_ref);
                responses.push(response);
            }
        }
        Ok(responses)
    }

    fn prepare_session_open(
        &mut self,
        connector: &node::Path,
        data: &Data,
        resolved_settings: &BTreeMap<String, crate::value::Value>,
    ) -> Result<RequestPreparation, DriverRouteError> {
        SessionConnectorBindings::prepare_session_open(self, connector, data, resolved_settings)
    }

    fn scheduler_session_completion(
        &mut self,
        connector: &node::Path,
        completion: SchedulerCompletion,
    ) -> Result<Completion, DriverRouteError> {
        SessionConnectorBindings::scheduler_session_completion(self, connector, completion)
    }

    fn open_session(
        &mut self,
        intent: &SessionOpenIntent,
        request: &Envelope,
    ) -> Result<crate::boundary::SessionStart, DriverRouteError> {
        SessionConnectorBindings::open_session(self, intent, request)
    }

    fn prepare_session_write(
        &mut self,
        connector: &node::Path,
        session: SessionRef,
        data: &Data,
        resolved_settings: &BTreeMap<String, crate::value::Value>,
    ) -> Result<RequestPreparation, DriverRouteError> {
        SessionConnectorBindings::prepare_session_write(
            self,
            connector,
            session,
            data,
            resolved_settings,
        )
    }

    fn write_session(
        &mut self,
        intent: &SessionWriteIntent,
        message: &Envelope,
    ) -> Result<crate::boundary::SessionStart, DriverRouteError> {
        SessionConnectorBindings::write_session(self, intent, message)
    }

    fn close_session(
        &mut self,
        connector: &node::Path,
        session: SessionRef,
    ) -> Result<crate::boundary::SessionStart, DriverRouteError> {
        SessionConnectorBindings::close_session(self, connector, session)
    }

    fn session_events(&mut self) -> Result<Vec<SessionDriverEvent>, DriverRouteError> {
        SessionConnectorBindings::session_events(self)
    }

    fn forget_session(&mut self, session: SessionRef) {
        SessionConnectorBindings::forget_session(self, session);
    }
}

impl SessionConnectorBindings for RuntimeBindings<'_> {
    fn prepare_session_open(
        &mut self,
        connector: &node::Path,
        data: &Data,
        resolved_settings: &BTreeMap<String, crate::value::Value>,
    ) -> Result<RequestPreparation, DriverRouteError> {
        Ok(self.session_driver(connector)?.1.prepare_session_open(
            connector,
            data,
            resolved_settings,
        ))
    }

    fn open_session(
        &mut self,
        intent: &SessionOpenIntent,
        request: &Envelope,
    ) -> Result<crate::boundary::SessionStart, DriverRouteError> {
        let (id, driver) = self.session_driver(&intent.connector_path)?;
        let start = driver.open_session(intent, request);
        if start == crate::boundary::SessionStart::Started {
            self.sessions.insert(intent.session_ref, id);
        }
        Ok(start)
    }

    fn scheduler_session_completion(
        &mut self,
        connector: &node::Path,
        completion: crate::boundary::SchedulerCompletion,
    ) -> Result<crate::boundary::Completion, DriverRouteError> {
        Ok(self
            .session_driver(connector)?
            .1
            .scheduler_session_completion(connector, completion))
    }

    fn prepare_session_write(
        &mut self,
        connector: &node::Path,
        session: SessionRef,
        data: &Data,
        resolved_settings: &BTreeMap<String, crate::value::Value>,
    ) -> Result<RequestPreparation, DriverRouteError> {
        let actual = self.session_family(connector)?;
        if let Some(expected) = self.sessions.get(&session)
            && expected != &actual
        {
            return Err(DriverRouteError::CrossFamilySession {
                session,
                expected: expected.clone(),
                actual,
            });
        }
        let (_, driver) = self.session_driver(connector)?;
        Ok(driver.prepare_session_write(connector, session, data, resolved_settings))
    }

    fn write_session(
        &mut self,
        intent: &SessionWriteIntent,
        message: &Envelope,
    ) -> Result<crate::boundary::SessionStart, DriverRouteError> {
        let actual = self.session_family(&intent.connector_path)?;
        let Some(expected) = self.sessions.get(&intent.session_ref) else {
            return Err(DriverRouteError::UncorrelatedSession(intent.session_ref));
        };
        if expected != &actual {
            return Err(DriverRouteError::CrossFamilySession {
                session: intent.session_ref,
                expected: expected.clone(),
                actual,
            });
        }
        let (_, driver) = self.session_driver(&intent.connector_path)?;
        Ok(driver.write_session(intent, message))
    }

    fn close_session(
        &mut self,
        connector: &node::Path,
        session: SessionRef,
    ) -> Result<crate::boundary::SessionStart, DriverRouteError> {
        let actual = self.session_family(connector)?;
        let Some(expected) = self.sessions.get(&session) else {
            return Err(DriverRouteError::UncorrelatedSession(session));
        };
        if expected != &actual {
            return Err(DriverRouteError::CrossFamilySession {
                session,
                expected: expected.clone(),
                actual,
            });
        }
        let (_, driver) = self.session_driver(connector)?;
        Ok(driver.close_session(session))
    }

    fn session_events(&mut self) -> Result<Vec<SessionDriverEvent>, DriverRouteError> {
        let mut events = Vec::new();
        for (id, family) in &mut self.families {
            let Some(path) = &family.session_path else {
                continue;
            };
            let Some(outbound) = family.driver.outbound() else {
                return Err(DriverRouteError::UnavailableOutbound(path.clone()));
            };
            let Some(driver) = outbound.session() else {
                return Err(DriverRouteError::UnavailableSession(path.clone()));
            };
            for event in driver.session_events() {
                let session = match &event {
                    SessionDriverEvent::Opened(session)
                    | SessionDriverEvent::WriteSettled { session, .. }
                    | SessionDriverEvent::Exited { session, .. } => *session,
                };
                let Some(expected) = self.sessions.get(&session) else {
                    return Err(DriverRouteError::UncorrelatedSession(session));
                };
                if expected != id {
                    return Err(DriverRouteError::CrossFamilySession {
                        session,
                        expected: expected.clone(),
                        actual: id.clone(),
                    });
                }
                events.push(event);
            }
        }
        Ok(events)
    }

    fn forget_session(&mut self, session: SessionRef) {
        self.sessions.remove(&session);
    }
}

fn union_roles(left: ConnectorRoles, right: ConnectorRoles) -> ConnectorRoles {
    if left == right {
        left
    } else {
        ConnectorRoles::InboundAndOutbound
    }
}

fn roles_of(driver: &mut dyn BoundConnectorDriver) -> Option<ConnectorRoles> {
    match (driver.inbound().is_some(), driver.outbound().is_some()) {
        (true, true) => Some(ConnectorRoles::InboundAndOutbound),
        (true, false) => Some(ConnectorRoles::Inbound),
        (false, true) => Some(ConnectorRoles::Outbound),
        (false, false) => None,
    }
}

fn outbound_exchange_of(driver: &mut dyn BoundConnectorDriver) -> Option<ConnectorExchange> {
    let outbound = driver.outbound()?;
    match (outbound.call().is_some(), outbound.session().is_some()) {
        (true, false) => Some(ConnectorExchange::Call),
        (false, true) => Some(ConnectorExchange::Session),
        (false, false) | (true, true) => None,
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
    /// The factory refused its selected plan Nodes before execution began.
    Factory {
        /// Stable contract id.
        id: ConnectorId,
        /// Factory-specific typed-boundary explanation.
        detail: String,
    },
    /// The object returned by a factory contradicts its advertised roles.
    BoundRolesMismatch {
        /// Stable contract id.
        id: ConnectorId,
        /// Factory claim used for deterministic selection.
        advertised: ConnectorRoles,
        /// Role views actually exposed; `None` means neither.
        actual: Option<ConnectorRoles>,
    },
    /// One stable contract id was assigned contradictory exchange facts by a
    /// forged plan.
    ConflictingExchange {
        /// Ambiguous semantic contract id.
        id: ConnectorId,
    },
    /// The bound outbound role exposes the wrong or an ambiguous exchange
    /// protocol view.
    BoundExchangeMismatch {
        /// Contract being bound.
        id: ConnectorId,
        /// Exchange carried by the validated plan.
        required: ConnectorExchange,
        /// Sole exchange view actually exposed, if any.
        actual: Option<ConnectorExchange>,
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

#[cfg(test)]
mod tests {
    use super::{
        BoundConnectorDriver, ConnectorDriverFactory, DriverBindingError, DriverFactoryError,
        DriverRegistrationError, DriverRegistry, RuntimeBindings,
    };
    use crate::boundary::{
        CallConnectorDriver, CallStart, Completion, Draft, HttpServerExit,
        InboundCallConnectorDriver, InboundCallEvent, InboundCallResponse,
        InboundCallResponseStart, InboundDecision, InboundEnvelopeProvenance, InboundExchangeRef,
        InboundProposal, InboundProposalRef, InboundSubmission, Intent, OutboundConnectorBindings,
        OutboundConnectorDriver, ProcessExit, RequestPreparation, Response, SchedulerCompletion,
        SessionConnectorBindings, SessionConnectorDriver, SessionDriverEvent, SessionOpenIntent,
        SessionRef, SessionWriteIntent, SubmissionSequence,
    };
    use crate::compiler::compile;
    use crate::connector::{ConnectorExchange, ConnectorId, ConnectorRoles};
    use crate::contract::ContractCatalog;
    use crate::envelope::{Data, Envelope, Namespace, Value};
    use crate::journal::{Journal, MemJournal, ids};
    use crate::node;
    use crate::plan::{GraphVersion, ValidatedExecutionPlan};
    use crate::runtime::vm::Vm;
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    use std::sync::{Arc, Mutex};

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

    #[derive(Default)]
    struct EmptyInbound;

    impl crate::boundary::InboundConnectorDriver for EmptyInbound {
        fn proposal(&mut self) -> Option<InboundProposal> {
            None
        }

        fn decide(&mut self, _: InboundProposalRef, _: InboundDecision) {}
    }

    struct RoutingDriver {
        family: String,
        starts: Arc<Mutex<Vec<String>>>,
        answers: Vec<Response>,
    }

    impl CallConnectorDriver for RoutingDriver {
        fn prepare_request(
            &mut self,
            _: &node::Path,
            data: &Data,
            _: &BTreeMap<String, crate::value::Value>,
        ) -> RequestPreparation {
            crate::boundary::prepare_process_request(crate::boundary::RequestCodec::Lines, data)
        }

        fn scheduler_completion(
            &mut self,
            _: &node::Path,
            completion: SchedulerCompletion,
        ) -> Completion {
            crate::boundary::process_scheduler_completion(completion)
        }

        fn start(&mut self, intent: &Intent, _: &Envelope) -> CallStart {
            self.starts
                .lock()
                .unwrap()
                .push(format!("{}:{}", self.family, intent.connector_path));
            self.answers.push(Response {
                call_ref: intent.call_ref,
                completion: Completion::process(
                    vec![Data::materialized(Value::Text(self.family.clone()))],
                    Vec::new(),
                    ProcessExit::Exited {
                        exit_code: 0,
                        signal: None,
                    },
                ),
            });
            CallStart::Started
        }

        fn events(&mut self) -> Vec<Response> {
            core::mem::take(&mut self.answers)
        }
    }

    struct RoutingBound {
        inbound: Option<EmptyInbound>,
        outbound: Option<RoutingDriver>,
    }

    impl BoundConnectorDriver for RoutingBound {
        fn inbound(&mut self) -> Option<&mut dyn crate::boundary::InboundConnectorDriver> {
            self.inbound
                .as_mut()
                .map(|driver| driver as &mut dyn crate::boundary::InboundConnectorDriver)
        }

        fn outbound(&mut self) -> Option<&mut dyn OutboundConnectorDriver> {
            self.outbound
                .as_mut()
                .map(|driver| driver as &mut dyn OutboundConnectorDriver)
        }
    }

    struct RoutingFactory {
        id: ConnectorId,
        advertised: ConnectorRoles,
        actual: Option<ConnectorRoles>,
        starts: Arc<Mutex<Vec<String>>>,
    }

    impl ConnectorDriverFactory for RoutingFactory {
        fn connector_id(&self) -> &ConnectorId {
            &self.id
        }

        fn roles(&self) -> ConnectorRoles {
            self.advertised
        }

        fn bind<'plan>(
            &self,
            _: &'plan ValidatedExecutionPlan,
            _: &[node::Path],
        ) -> Result<Box<dyn BoundConnectorDriver + 'plan>, DriverFactoryError> {
            let inbound = self
                .actual
                .filter(|roles| roles.has_inbound())
                .map(|_| EmptyInbound);
            let outbound =
                self.actual
                    .filter(|roles| roles.has_outbound())
                    .map(|_| RoutingDriver {
                        family: self.id.as_str().to_owned(),
                        starts: Arc::clone(&self.starts),
                        answers: Vec::new(),
                    });
            Ok(Box::new(RoutingBound { inbound, outbound }))
        }
    }

    fn routing_factory(
        id: &str,
        advertised: ConnectorRoles,
        actual: Option<ConnectorRoles>,
        starts: &Arc<Mutex<Vec<String>>>,
    ) -> Box<dyn ConnectorDriverFactory> {
        Box::new(RoutingFactory {
            id: ConnectorId::parse(id).unwrap(),
            advertised,
            actual,
            starts: Arc::clone(starts),
        })
    }

    fn two_family_plan_with_first_roles(first_roles: ConnectorRoles) -> ValidatedExecutionPlan {
        let source = r#"
graph mixed : App {
  input in: text
  output out: text
  connector first : Process {
    mode = job ; command = "cat" ; codec = lines
    effect = idempotent ; behavior = deterministic
  }
  connector second : Process {
    mode = job ; command = "cat" ; codec = lines
    effect = idempotent ; behavior = deterministic
  }
  . -> first -> second -> .
  first.err -> nothing ; first.exit -> nothing
  second.err -> nothing ; second.exit -> nothing
}
"#;
        let compilation = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("the routing probe compiles");
        let mut raw = compilation.plan.plan().clone();
        for planned in &mut raw.nodes {
            let Some(spec) = (match &mut planned.execution {
                crate::plan::NodeExecution::Connector(spec) => Some(spec),
                crate::plan::NodeExecution::Operator => None,
            }) else {
                continue;
            };
            spec.contract = ConnectorId::parse(match planned.path.to_string().as_str() {
                "first" => "test/alpha@1",
                "second" => "test/beta@1",
                other => panic!("unexpected probe node `{other}`"),
            })
            .unwrap();
            if planned.path.to_string() == "first" {
                spec.roles = first_roles;
            }
        }
        ValidatedExecutionPlan::try_new(raw).expect("foreign contracts remain structural")
    }

    fn two_family_plan() -> ValidatedExecutionPlan {
        two_family_plan_with_first_roles(ConnectorRoles::Outbound)
    }

    fn session_family_plan() -> ValidatedExecutionPlan {
        let base = two_family_plan_with_first_roles(ConnectorRoles::InboundAndOutbound);
        let mut raw = base.plan().clone();
        let first = raw
            .nodes
            .iter_mut()
            .find(|planned| planned.path.to_string() == "first")
            .expect("the first connector exists");
        let crate::plan::NodeExecution::Connector(spec) = &mut first.execution else {
            panic!("the first node is a connector")
        };
        spec.exchange = ConnectorExchange::Session;
        ValidatedExecutionPlan::try_new(raw).expect("the foreign Session contract is structural")
    }

    fn inbound_event_plan() -> ValidatedExecutionPlan {
        let source = r#"
graph inbound : App {
  input in: text
  output out: text
  connector source : Process {
    mode = job ; command = "cat" ; codec = lines
    effect = idempotent ; behavior = deterministic
  }
  operator relay { input in: text ; on in { out = data } }
  . -> nothing
  source.out -> relay -> . ; source.err -> nothing
}
"#;
        let compilation = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("the inbound probe compiles");
        let mut raw = compilation.plan.plan().clone();
        let source = raw
            .nodes
            .iter_mut()
            .find(|planned| planned.path.to_string() == "source")
            .expect("the source connector exists");
        let crate::plan::NodeExecution::Connector(spec) = &mut source.execution else {
            panic!("the source node is a connector")
        };
        spec.contract = ConnectorId::parse("test/event-source@1").unwrap();
        spec.roles = ConnectorRoles::Inbound;
        spec.exchange = ConnectorExchange::Event;
        source.ports.retain(|port| port.port.name() != "exit");
        ValidatedExecutionPlan::try_new(raw).expect("the foreign Event contract is structural")
    }

    struct DuplexDriver {
        observations: Arc<Mutex<Vec<&'static str>>>,
    }

    impl crate::boundary::InboundConnectorDriver for DuplexDriver {
        fn proposal(&mut self) -> Option<InboundProposal> {
            self.observations.lock().unwrap().push("inbound");
            None
        }

        fn decide(&mut self, _: InboundProposalRef, _: InboundDecision) {}

        fn call(&mut self) -> Option<&mut dyn InboundCallConnectorDriver> {
            Some(self)
        }
    }

    impl InboundCallConnectorDriver for DuplexDriver {
        fn start_response(&mut self, _: &InboundCallResponse) -> InboundCallResponseStart {
            InboundCallResponseStart::Pending
        }

        fn start_timeout(
            &mut self,
            _: &crate::boundary::InboundCallTimeout,
        ) -> crate::boundary::InboundCallTimeoutStart {
            crate::boundary::InboundCallTimeoutStart::Pending
        }

        fn events(&mut self) -> Vec<InboundCallEvent> {
            Vec::new()
        }
    }

    impl CallConnectorDriver for DuplexDriver {
        fn prepare_request(
            &mut self,
            _: &node::Path,
            data: &Data,
            _: &BTreeMap<String, crate::value::Value>,
        ) -> RequestPreparation {
            self.observations.lock().unwrap().push("outbound");
            crate::boundary::prepare_process_request(crate::boundary::RequestCodec::Lines, data)
        }

        fn scheduler_completion(
            &mut self,
            _: &node::Path,
            completion: SchedulerCompletion,
        ) -> Completion {
            crate::boundary::process_scheduler_completion(completion)
        }

        fn start(&mut self, _: &Intent, _: &Envelope) -> CallStart {
            CallStart::Started
        }

        fn events(&mut self) -> Vec<Response> {
            Vec::new()
        }
    }

    impl BoundConnectorDriver for DuplexDriver {
        fn inbound(&mut self) -> Option<&mut dyn crate::boundary::InboundConnectorDriver> {
            Some(self)
        }

        fn outbound(&mut self) -> Option<&mut dyn OutboundConnectorDriver> {
            Some(self)
        }
    }

    struct DuplexFactory {
        id: ConnectorId,
        observations: Arc<Mutex<Vec<&'static str>>>,
    }

    impl ConnectorDriverFactory for DuplexFactory {
        fn connector_id(&self) -> &ConnectorId {
            &self.id
        }

        fn roles(&self) -> ConnectorRoles {
            ConnectorRoles::InboundAndOutbound
        }

        fn bind<'plan>(
            &self,
            _: &'plan ValidatedExecutionPlan,
            _: &[node::Path],
        ) -> Result<Box<dyn BoundConnectorDriver + 'plan>, DriverFactoryError> {
            Ok(Box::new(DuplexDriver {
                observations: Arc::clone(&self.observations),
            }))
        }
    }

    #[test]
    fn a_both_role_family_exposes_one_shared_bound_object() {
        let plan = two_family_plan_with_first_roles(ConnectorRoles::InboundAndOutbound);
        let observations = Arc::new(Mutex::new(Vec::new()));
        let starts = Arc::new(Mutex::new(Vec::new()));
        let registry = DriverRegistry::default()
            .try_with(Box::new(DuplexFactory {
                id: ConnectorId::parse("test/alpha@1").unwrap(),
                observations: Arc::clone(&observations),
            }))
            .unwrap()
            .try_with(routing_factory(
                "test/beta@1",
                ConnectorRoles::Outbound,
                Some(ConnectorRoles::Outbound),
                &starts,
            ))
            .unwrap();
        let mut bindings = RuntimeBindings::try_new(&plan, &registry).unwrap();

        assert!(bindings.next_inbound().unwrap().is_none());
        let preparation = OutboundConnectorBindings::prepare_request(
            &mut bindings,
            &node::Path::new(["first"]).unwrap(),
            &Data::materialized(Value::Text(String::from("request"))),
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(matches!(preparation, RequestPreparation::Prepared(_)));
        assert_eq!(*observations.lock().unwrap(), ["inbound", "outbound"]);
    }

    struct SessionDuplexDriver {
        observations: Arc<Mutex<Vec<&'static str>>>,
        events: Vec<SessionDriverEvent>,
        closed: BTreeSet<SessionRef>,
    }

    impl crate::boundary::InboundConnectorDriver for SessionDuplexDriver {
        fn proposal(&mut self) -> Option<InboundProposal> {
            self.observations.lock().unwrap().push("inbound");
            None
        }

        fn decide(&mut self, _: InboundProposalRef, _: InboundDecision) {
            self.observations.lock().unwrap().push("decision");
        }
    }

    impl SessionConnectorDriver for SessionDuplexDriver {
        fn prepare_session_open(
            &mut self,
            _: &node::Path,
            data: &Data,
            _: &BTreeMap<String, crate::value::Value>,
        ) -> RequestPreparation {
            crate::boundary::prepare_process_request(crate::boundary::RequestCodec::Lines, data)
        }

        fn scheduler_session_completion(
            &mut self,
            _: &node::Path,
            completion: SchedulerCompletion,
        ) -> Completion {
            Completion::process_exit(match completion {
                SchedulerCompletion::TimeoutExpired => ProcessExit::TimeoutExpired,
                SchedulerCompletion::OutcomeUnknown => ProcessExit::OutcomeUnknown,
            })
        }

        fn open_session(
            &mut self,
            intent: &SessionOpenIntent,
            _: &Envelope,
        ) -> crate::boundary::SessionStart {
            self.observations.lock().unwrap().push("open");
            self.events
                .insert(0, SessionDriverEvent::Opened(intent.session_ref));
            crate::boundary::SessionStart::Started
        }

        fn prepare_session_write(
            &mut self,
            _: &node::Path,
            _: SessionRef,
            data: &Data,
            _: &BTreeMap<String, crate::value::Value>,
        ) -> RequestPreparation {
            crate::boundary::prepare_process_request(crate::boundary::RequestCodec::Lines, data)
        }

        fn write_session(
            &mut self,
            intent: &SessionWriteIntent,
            _: &Envelope,
        ) -> crate::boundary::SessionStart {
            self.observations.lock().unwrap().push("write");
            self.events.insert(
                0,
                SessionDriverEvent::WriteSettled {
                    session: intent.session_ref,
                    write: intent.write_ref,
                },
            );
            crate::boundary::SessionStart::Started
        }

        fn close_session(&mut self, session: SessionRef) -> crate::boundary::SessionStart {
            if self.closed.insert(session) {
                self.observations.lock().unwrap().push("close");
                self.events.insert(
                    0,
                    SessionDriverEvent::Exited {
                        session,
                        exit: crate::boundary::PortOutput {
                            port: node::Port::conventional_exit(),
                            data: Data::materialized(
                                ProcessExit::Exited {
                                    exit_code: 0,
                                    signal: None,
                                }
                                .into_value(),
                            ),
                            ext: crate::envelope::Namespace::new(),
                        },
                    },
                );
            }
            crate::boundary::SessionStart::Started
        }

        fn session_events(&mut self) -> Vec<SessionDriverEvent> {
            core::mem::take(&mut self.events)
        }
    }

    impl OutboundConnectorDriver for SessionDuplexDriver {
        fn call(&mut self) -> Option<&mut dyn CallConnectorDriver> {
            None
        }

        fn session(&mut self) -> Option<&mut dyn SessionConnectorDriver> {
            Some(self)
        }
    }

    impl BoundConnectorDriver for SessionDuplexDriver {
        fn inbound(&mut self) -> Option<&mut dyn crate::boundary::InboundConnectorDriver> {
            Some(self)
        }

        fn outbound(&mut self) -> Option<&mut dyn OutboundConnectorDriver> {
            Some(self)
        }
    }

    struct SessionDuplexFactory {
        id: ConnectorId,
        observations: Arc<Mutex<Vec<&'static str>>>,
    }

    impl ConnectorDriverFactory for SessionDuplexFactory {
        fn connector_id(&self) -> &ConnectorId {
            &self.id
        }

        fn roles(&self) -> ConnectorRoles {
            ConnectorRoles::InboundAndOutbound
        }

        fn bind<'plan>(
            &self,
            _: &'plan ValidatedExecutionPlan,
            _: &[node::Path],
        ) -> Result<Box<dyn BoundConnectorDriver + 'plan>, DriverFactoryError> {
            Ok(Box::new(SessionDuplexDriver {
                observations: Arc::clone(&self.observations),
                events: Vec::new(),
                closed: BTreeSet::new(),
            }))
        }
    }

    #[test]
    fn one_both_role_session_driver_correlates_concurrent_openings_and_writes() {
        let plan = session_family_plan();
        let observations = Arc::new(Mutex::new(Vec::new()));
        let starts = Arc::new(Mutex::new(Vec::new()));
        let registry = DriverRegistry::default()
            .try_with(Box::new(SessionDuplexFactory {
                id: ConnectorId::parse("test/alpha@1").unwrap(),
                observations: Arc::clone(&observations),
            }))
            .unwrap()
            .try_with(routing_factory(
                "test/beta@1",
                ConnectorRoles::Outbound,
                Some(ConnectorRoles::Outbound),
                &starts,
            ))
            .unwrap();
        let mut bindings = RuntimeBindings::try_new(&plan, &registry).unwrap();
        assert!(bindings.next_inbound().unwrap().is_none());

        let mut journal = MemJournal::new(GraphVersion(1));
        let request = journal
            .append_inbound(InboundSubmission {
                provenance: InboundEnvelopeProvenance::single(node::Path::new(["world"]).unwrap()),
                port: node::Port::new("in").unwrap(),
                draft: Draft {
                    data: Data::materialized(Value::Text(String::from("hello"))),
                    ext: Namespace::new(),
                },
                sequence: SubmissionSequence::Next,
            })
            .unwrap()
            .envelope;
        let connector = node::Path::new(["first"]).unwrap();
        let opening = |byte| {
            let prepared = crate::boundary::PreparedRequest::seal(vec![byte]);
            let emitting_id = crate::envelope::Id::from_bytes([byte; 16]);
            SessionOpenIntent {
                session_ref: ids::outbound_session_ref(&connector, emitting_id, prepared.digest()),
                emitting_id,
                connector_path: connector.clone(),
                prepared,
            }
        };
        let first = opening(1);
        let second = opening(2);
        for intent in [&first, &second] {
            journal.open_outbound_session(intent.clone()).unwrap();
            SessionConnectorBindings::open_session(&mut bindings, intent, &request).unwrap();
        }
        assert_eq!(
            SessionConnectorBindings::session_events(&mut bindings).unwrap(),
            [
                SessionDriverEvent::Opened(second.session_ref),
                SessionDriverEvent::Opened(first.session_ref),
            ],
            "reverse arrival remains correlated by SessionRef"
        );
        journal.confirm_session_open(&first.session_ref).unwrap();
        journal.confirm_session_open(&second.session_ref).unwrap();

        let write = |session: SessionRef, byte| {
            let prepared = crate::boundary::PreparedRequest::seal(vec![byte]);
            let emitting_id = crate::envelope::Id::from_bytes([byte; 16]);
            SessionWriteIntent {
                write_ref: ids::session_write_ref(session, emitting_id, prepared.digest()),
                session_ref: session,
                emitting_id,
                connector_path: connector.clone(),
                prepared,
            }
        };
        let first_write = write(first.session_ref, 3);
        let second_write = write(second.session_ref, 4);
        for intent in [&first_write, &second_write] {
            journal.open_session_write(intent.clone()).unwrap();
            SessionConnectorBindings::write_session(&mut bindings, intent, &request).unwrap();
        }
        assert_eq!(
            SessionConnectorBindings::session_events(&mut bindings).unwrap(),
            [
                SessionDriverEvent::WriteSettled {
                    session: second.session_ref,
                    write: second_write.write_ref,
                },
                SessionDriverEvent::WriteSettled {
                    session: first.session_ref,
                    write: first_write.write_ref,
                },
            ]
        );
        assert_eq!(
            *observations.lock().unwrap(),
            ["inbound", "open", "open", "write", "write"]
        );
    }

    #[test]
    fn a_session_driver_passes_the_exchange_specific_conformance_profile() {
        let plan = session_family_plan();
        let observations = Arc::new(Mutex::new(Vec::new()));
        let starts = Arc::new(Mutex::new(Vec::new()));
        let registry = DriverRegistry::default()
            .try_with(Box::new(SessionDuplexFactory {
                id: ConnectorId::parse("test/alpha@1").unwrap(),
                observations,
            }))
            .unwrap()
            .try_with(routing_factory(
                "test/beta@1",
                ConnectorRoles::Outbound,
                Some(ConnectorRoles::Outbound),
                &starts,
            ))
            .unwrap();
        let contract = Box::leak(Box::new(crate::contract::ConnectorContract {
            id: "test/alpha@1",
            name: "SessionProbe",
            roles: ConnectorRoles::InboundAndOutbound,
            exchange: ConnectorExchange::Session,
            ..crate::stdlib::PROCESS.clone()
        }));
        let terminal = Data::materialized(
            ProcessExit::Exited {
                exit_code: 0,
                signal: None,
            }
            .into_value(),
        );
        let profile = crate::conformance::SessionProfile {
            contract,
            plan: &plan,
            connector: node::Path::new(["first"]).unwrap(),
            openings: [
                Data::materialized(Value::Text(String::from("open-a"))),
                Data::materialized(Value::Text(String::from("open-b"))),
            ],
            writes: [
                Data::materialized(Value::Text(String::from("write-a"))),
                Data::materialized(Value::Text(String::from("write-b"))),
            ],
            terminal,
            resolved_settings: BTreeMap::new(),
        };

        let report = crate::conformance::verify_session(&profile, &registry);
        crate::conformance::assert_conformant(&report, &[]);
        assert_eq!(
            report.proven_cases,
            [
                "session-open",
                "session-write",
                "session-ambiguity",
                "session-exit",
            ]
        );
    }

    #[derive(Default)]
    struct InboundQueueState {
        proposals: VecDeque<InboundProposal>,
        decisions: Vec<(InboundProposalRef, InboundDecision)>,
    }

    struct RetainedInboundDriver {
        state: Arc<Mutex<InboundQueueState>>,
    }

    impl crate::boundary::InboundConnectorDriver for RetainedInboundDriver {
        fn proposal(&mut self) -> Option<InboundProposal> {
            self.state.lock().unwrap().proposals.front().cloned()
        }

        fn decide(&mut self, reference: InboundProposalRef, decision: InboundDecision) {
            let mut state = self.state.lock().unwrap();
            let retained = state
                .proposals
                .pop_front()
                .expect("a decision follows a retained proposal");
            assert_eq!(retained.reference, reference);
            state.decisions.push((reference, decision));
        }
    }

    impl BoundConnectorDriver for RetainedInboundDriver {
        fn inbound(&mut self) -> Option<&mut dyn crate::boundary::InboundConnectorDriver> {
            Some(self)
        }

        fn outbound(&mut self) -> Option<&mut dyn OutboundConnectorDriver> {
            None
        }
    }

    struct RetainedInboundFactory {
        id: ConnectorId,
        state: Arc<Mutex<InboundQueueState>>,
    }

    #[derive(Default)]
    struct RetainedCallState {
        proposals: VecDeque<InboundProposal>,
        decisions: Vec<(InboundProposalRef, InboundDecision)>,
        events: Vec<InboundCallEvent>,
        responses: Vec<InboundCallResponse>,
        acknowledge_on_accept: bool,
        hold_first_response: bool,
    }

    struct RetainedCallDriver {
        state: Arc<Mutex<RetainedCallState>>,
    }

    impl crate::boundary::InboundConnectorDriver for RetainedCallDriver {
        fn proposal(&mut self) -> Option<InboundProposal> {
            self.state.lock().unwrap().proposals.front().cloned()
        }

        fn decide(&mut self, reference: InboundProposalRef, decision: InboundDecision) {
            let mut state = self.state.lock().unwrap();
            let retained = state
                .proposals
                .pop_front()
                .expect("a decision follows a retained proposal");
            assert_eq!(retained.reference, reference);
            if state.acknowledge_on_accept
                && let InboundDecision::Accepted(accepted) = decision
            {
                let Some(InboundExchangeRef::Call(call_ref)) = accepted.exchange else {
                    panic!("an inbound Call acceptance carries its Call ref")
                };
                state.events.push(InboundCallEvent {
                    call_ref,
                    completion: Completion::http_server_exit(HttpServerExit::Completed {
                        status: 202,
                    }),
                });
            }
            state.decisions.push((reference, decision));
        }

        fn call(&mut self) -> Option<&mut dyn InboundCallConnectorDriver> {
            Some(self)
        }
    }

    impl InboundCallConnectorDriver for RetainedCallDriver {
        fn start_response(&mut self, response: &InboundCallResponse) -> InboundCallResponseStart {
            let mut state = self.state.lock().unwrap();
            if state.hold_first_response {
                state.hold_first_response = false;
                return InboundCallResponseStart::Pending;
            }
            state.responses.push(response.clone());
            state.events.push(InboundCallEvent {
                call_ref: response.call_ref,
                completion: Completion::http_server_exit(HttpServerExit::Completed { status: 200 }),
            });
            InboundCallResponseStart::Started
        }

        fn start_timeout(
            &mut self,
            _: &crate::boundary::InboundCallTimeout,
        ) -> crate::boundary::InboundCallTimeoutStart {
            crate::boundary::InboundCallTimeoutStart::Pending
        }

        fn events(&mut self) -> Vec<InboundCallEvent> {
            core::mem::take(&mut self.state.lock().unwrap().events)
        }
    }

    impl BoundConnectorDriver for RetainedCallDriver {
        fn inbound(&mut self) -> Option<&mut dyn crate::boundary::InboundConnectorDriver> {
            Some(self)
        }

        fn outbound(&mut self) -> Option<&mut dyn OutboundConnectorDriver> {
            None
        }
    }

    struct RetainedCallFactory {
        id: ConnectorId,
        state: Arc<Mutex<RetainedCallState>>,
    }

    impl ConnectorDriverFactory for RetainedCallFactory {
        fn connector_id(&self) -> &ConnectorId {
            &self.id
        }

        fn roles(&self) -> ConnectorRoles {
            ConnectorRoles::Inbound
        }

        fn bind<'plan>(
            &self,
            _: &'plan ValidatedExecutionPlan,
            _: &[node::Path],
        ) -> Result<Box<dyn BoundConnectorDriver + 'plan>, DriverFactoryError> {
            Ok(Box::new(RetainedCallDriver {
                state: Arc::clone(&self.state),
            }))
        }
    }

    impl ConnectorDriverFactory for RetainedInboundFactory {
        fn connector_id(&self) -> &ConnectorId {
            &self.id
        }

        fn roles(&self) -> ConnectorRoles {
            ConnectorRoles::Inbound
        }

        fn bind<'plan>(
            &self,
            _: &'plan ValidatedExecutionPlan,
            _: &[node::Path],
        ) -> Result<Box<dyn BoundConnectorDriver + 'plan>, DriverFactoryError> {
            Ok(Box::new(RetainedInboundDriver {
                state: Arc::clone(&self.state),
            }))
        }
    }

    fn inbound_proposal(reference: u64, port: &str) -> InboundProposal {
        InboundProposal {
            reference: InboundProposalRef::new(reference),
            submission: InboundSubmission {
                provenance: InboundEnvelopeProvenance::single(node::Path::new(["source"]).unwrap()),
                port: node::Port::new(port).unwrap(),
                draft: Draft {
                    data: Data::materialized(Value::Text(format!("event-{reference}"))),
                    ext: Namespace::new(),
                },
                sequence: SubmissionSequence::Next,
            },
            session: None,
        }
    }

    fn inbound_call_response_plan() -> ValidatedExecutionPlan {
        let source = r#"
graph api : App {
  connector server : HttpServer {
    listen = "127.0.0.1:8080"
    codec = text
    instance = { key = data }
  }
  operator relay { input in: text ; on in { out = "response" } }
  server -> relay -> .
}
"#;
        let compilation = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("the ack-mode source checks");
        let mut raw = compilation.plan.plan().clone();
        let server_slot = raw
            .nodes
            .iter()
            .position(|planned| planned.path.to_string() == "server")
            .and_then(|index| u32::try_from(index).ok())
            .map(crate::plan::Slot::new)
            .expect("the server slot is representable");
        let relay_slot = raw
            .nodes
            .iter()
            .position(|planned| planned.path.to_string() == "relay")
            .and_then(|index| u32::try_from(index).ok())
            .map(crate::plan::Slot::new)
            .expect("the relay slot is representable");
        let server = raw
            .nodes
            .get_mut(server_slot.index())
            .expect("the server node exists");
        let crate::plan::NodeExecution::Connector(spec) = &mut server.execution else {
            panic!("the server node is a connector")
        };
        spec.contract = ConnectorId::parse("test/inbound-call@1").unwrap();

        let response = raw
            .edges
            .iter_mut()
            .find(|edge| {
                edge.from.node == Some(relay_slot)
                    && edge.to.as_ref().is_some_and(|to| to.node.is_none())
            })
            .expect("the relay boundary response exists");
        response.to = Some(crate::plan::EndpointRef {
            node: Some(server_slot),
            port: node::Port::new("in").expect("the conventional input name is valid"),
        });
        raw.boundary.retain(|decl| decl.port.name() != "out");
        let mut edges = core::mem::take(&mut raw.edges);
        edges.sort_by_key(|edge| raw.edge_text(edge));
        raw.edges = edges;
        ValidatedExecutionPlan::try_new(raw).expect("the foreign inbound Call is structural")
    }

    #[test]
    fn an_inbound_call_is_accepted_before_its_single_typed_exit_commits() {
        let source = r#"
graph api : App {
  connector server : HttpServer {
    listen = "127.0.0.1:8080"
    codec = text
  }
  operator relay { input in: text ; on in { out = data } }
  server -> relay -> .
}
"#;
        let plan = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("the ack-mode source checks")
        .plan;
        let event_only_registry = DriverRegistry::default()
            .try_with(Box::new(RetainedInboundFactory {
                id: ConnectorId::parse(crate::connector::HTTP_SERVER_CONNECTOR_ID).unwrap(),
                state: Arc::new(Mutex::new(InboundQueueState::default())),
            }))
            .unwrap();
        assert!(matches!(
            RuntimeBindings::try_new(&plan, &event_only_registry),
            Err(DriverBindingError::BoundExchangeMismatch {
                required: ConnectorExchange::Call,
                actual: None,
                ..
            })
        ));
        let state = Arc::new(Mutex::new(RetainedCallState {
            proposals: VecDeque::from([InboundProposal {
                reference: InboundProposalRef::new(1),
                submission: InboundSubmission {
                    provenance: InboundEnvelopeProvenance::single(
                        node::Path::new(["server"]).unwrap(),
                    ),
                    port: node::Port::conventional_out(),
                    draft: Draft {
                        data: Data::materialized(Value::Text(String::from("request"))),
                        ext: Namespace::new(),
                    },
                    sequence: SubmissionSequence::Next,
                },
                session: None,
            }]),
            acknowledge_on_accept: true,
            ..RetainedCallState::default()
        }));
        let registry = DriverRegistry::default()
            .try_with(Box::new(RetainedCallFactory {
                id: ConnectorId::parse(crate::connector::HTTP_SERVER_CONNECTOR_ID).unwrap(),
                state: Arc::clone(&state),
            }))
            .unwrap();
        let mut bindings = RuntimeBindings::try_new(&plan, &registry).unwrap();
        let mut vm = Vm::boot(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));

        let Some(crate::boundary::InboundAdmission::Accepted(accepted)) = bindings
            .pump_inbound(&mut vm, &mut journal)
            .expect("the proposal reaches atomic admission")
        else {
            panic!("the request must be accepted")
        };
        let Some(InboundExchangeRef::Call(call_ref)) = accepted.exchange else {
            panic!("the accepting fact opens a Call")
        };
        assert_eq!(journal.open_inbound_calls()[0].reference, call_ref);
        assert!(journal.find_envelope(accepted.envelope).is_some());
        assert_eq!(state.lock().unwrap().decisions.len(), 1);

        let report = bindings.pump_inbound_call_events(&mut vm).unwrap();
        assert_eq!(report.queued, 1);
        assert!(report.rejected.is_empty());
        vm.drain_with_bindings(&mut journal, &mut bindings)
            .expect("start then reply commit in causal order");
        assert!(journal.open_inbound_calls().is_empty());
        let terminals = journal
            .records()
            .iter()
            .filter_map(|record| match record {
                crate::journal::Record::Transaction(transaction)
                    if transaction.exchange_exit.as_deref()
                        == Some(&crate::journal::InboundExchangeExitRecord::Call(call_ref)) =>
                {
                    Some(transaction)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(terminals.len(), 1);
        assert_eq!(
            terminals[0].trigger,
            node::Trigger::Lifecycle(node::Lifecycle::Reply)
        );
        assert!(terminals[0].consumed.is_empty());
        assert_eq!(terminals[0].emissions.len(), 1);
        assert_eq!(terminals[0].emissions[0].port.name(), "exit");
        let trace = crate::golden::project(journal.records());
        let inbound_index = journal
            .records()
            .iter()
            .position(|record| {
                matches!(
                    record,
                    crate::journal::Record::Inbound(inbound)
                        if matches!(
                            inbound.exchange.as_deref(),
                            Some(crate::journal::InboundExchangeRecord::Call(call))
                                if call.reference == call_ref
                        )
                )
            })
            .expect("the opening INBOUND fact is present");
        let terminal_index = journal
            .records()
            .iter()
            .position(|record| {
                matches!(
                    record,
                    crate::journal::Record::Transaction(transaction)
                        if transaction.exchange_exit.as_deref()
                            == Some(&crate::journal::InboundExchangeExitRecord::Call(call_ref))
                )
            })
            .expect("the terminal TRANSACTION fact is present");
        for (index, key) in [(inbound_index, 28), (terminal_index, 52)] {
            let bytes = trace
                .record_bytes(index)
                .expect("the canonical trace retains every causal record");
            let (crate::canonical_cbor::Value::Map(tree), used) =
                crate::canonical_cbor::decode(bytes).expect("canonical trace bytes decode")
            else {
                panic!("a causal record projects as a map")
            };
            assert_eq!(used, bytes.len());
            assert!(tree.contains_key(&key), "canonical key {key} is present");
        }

        state.lock().unwrap().events.push(InboundCallEvent {
            call_ref,
            completion: Completion::http_server_exit(HttpServerExit::Completed { status: 202 }),
        });
        let duplicate = bindings.pump_inbound_call_events(&mut vm).unwrap();
        assert_eq!(duplicate.uncorrelated.len(), 1);
        assert_eq!(
            journal
                .records()
                .iter()
                .filter(|record| matches!(
                    record,
                    crate::journal::Record::Transaction(transaction)
                        if transaction.exchange_exit.as_deref()
                            == Some(&crate::journal::InboundExchangeExitRecord::Call(call_ref))
                ))
                .count(),
            1,
            "a late Driver observation cannot create a second exit"
        );
    }

    #[test]
    fn a_graph_response_closes_its_original_inbound_call_without_an_outbound_call() {
        let plan = inbound_call_response_plan();
        let state = Arc::new(Mutex::new(RetainedCallState {
            proposals: VecDeque::from([InboundProposal {
                reference: InboundProposalRef::new(1),
                submission: InboundSubmission {
                    provenance: InboundEnvelopeProvenance::single(
                        node::Path::new(["server"]).unwrap(),
                    ),
                    port: node::Port::conventional_out(),
                    draft: Draft {
                        data: Data::materialized(Value::Text(String::from("request"))),
                        ext: Namespace::new(),
                    },
                    sequence: SubmissionSequence::Next,
                },
                session: None,
            }]),
            hold_first_response: true,
            ..RetainedCallState::default()
        }));
        let registry = DriverRegistry::default()
            .try_with(Box::new(RetainedCallFactory {
                id: ConnectorId::parse("test/inbound-call@1").unwrap(),
                state: Arc::clone(&state),
            }))
            .unwrap();
        let mut bindings = RuntimeBindings::try_new(&plan, &registry).unwrap();
        let mut vm = Vm::boot(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));

        let Some(crate::boundary::InboundAdmission::Accepted(accepted)) = bindings
            .pump_inbound(&mut vm, &mut journal)
            .expect("the request reaches atomic admission")
        else {
            panic!("the request must be accepted")
        };
        let Some(InboundExchangeRef::Call(call_ref)) = accepted.exchange else {
            panic!("the accepted request opens one inbound Call")
        };
        let opening_instance =
            journal
                .records()
                .iter()
                .find_map(|record| match record {
                    crate::journal::Record::Inbound(inbound) => inbound
                        .exchange
                        .as_deref()
                        .and_then(|exchange| match exchange {
                            crate::journal::InboundExchangeRecord::Call(call)
                                if call.reference == call_ref =>
                            {
                                Some(call.instance.clone())
                            }
                            crate::journal::InboundExchangeRecord::Call(_)
                            | crate::journal::InboundExchangeRecord::Session(_) => None,
                        }),
                    crate::journal::Record::Transaction(_)
                    | crate::journal::Record::Outbound(_) => None,
                })
                .expect("the accepted Call records its exact source instance");
        vm.drain_with_bindings(&mut journal, &mut bindings)
            .expect("the graph response reaches the retained connector cursor");
        let response_target = journal
            .records()
            .iter()
            .find_map(|record| match record {
                crate::journal::Record::Transaction(transaction)
                    if transaction.owner_path.to_string() == "relay" =>
                {
                    transaction.routes.first().map(|route| route.target.clone())
                }
                crate::journal::Record::Inbound(_)
                | crate::journal::Record::Transaction(_)
                | crate::journal::Record::Outbound(_) => None,
            })
            .expect("the response-producing transaction seals its route");
        assert_eq!(response_target, opening_instance);

        state.lock().unwrap().events.push(InboundCallEvent {
            call_ref,
            completion: Completion::http_server_exit(HttpServerExit::Completed { status: 200 }),
        });
        let premature = bindings.pump_inbound_call_events(&mut vm).unwrap();
        assert_eq!(premature.queued, 0);
        assert_eq!(premature.rejected.len(), 1);

        let first = bindings.pump_inbound_call_responses(&mut vm).unwrap();
        assert_eq!((first.started, first.pending), (0, 1));
        assert!(state.lock().unwrap().responses.is_empty());
        let second = bindings.pump_inbound_call_responses(&mut vm).unwrap();
        assert_eq!((second.started, second.pending), (1, 0));
        {
            let state = state.lock().unwrap();
            assert_eq!(state.responses.len(), 1);
            assert_eq!(state.responses[0].call_ref, call_ref);
            assert_eq!(
                state.responses[0].response.data(),
                &Data::materialized(Value::Text(String::from("response")))
            );
        }
        let report = bindings.pump_inbound_call_events(&mut vm).unwrap();
        assert_eq!(report.queued, 1);
        assert!(report.rejected.is_empty());
        vm.drain_with_bindings(&mut journal, &mut bindings)
            .expect("the observed protocol completion closes the original Call");

        let terminals = journal
            .records()
            .iter()
            .filter_map(|record| match record {
                crate::journal::Record::Transaction(transaction)
                    if transaction.exchange_exit.as_deref()
                        == Some(&crate::journal::InboundExchangeExitRecord::Call(call_ref)) =>
                {
                    Some(transaction)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(terminals.len(), 1);
        assert_eq!(terminals[0].consumed.len(), 1);
        assert_eq!(
            terminals[0].trigger,
            node::Trigger::Lifecycle(node::Lifecycle::Reply)
        );
        assert!(journal.open_inbound_calls().is_empty());
        assert!(
            journal
                .records()
                .iter()
                .all(|record| !matches!(record, crate::journal::Record::Outbound(_))),
            "respond is the return half of the inbound Call, never a new outbound Call"
        );
    }

    #[test]
    fn a_response_after_the_inbound_call_closed_is_discarded_and_journaled() {
        let plan = inbound_call_response_plan();
        let state = Arc::new(Mutex::new(RetainedCallState {
            proposals: VecDeque::from([InboundProposal {
                reference: InboundProposalRef::new(1),
                submission: InboundSubmission {
                    provenance: InboundEnvelopeProvenance::single(
                        node::Path::new(["server"]).unwrap(),
                    ),
                    port: node::Port::conventional_out(),
                    draft: Draft {
                        data: Data::materialized(Value::Text(String::from("late"))),
                        ext: Namespace::new(),
                    },
                    sequence: SubmissionSequence::Next,
                },
                session: None,
            }]),
            acknowledge_on_accept: true,
            ..RetainedCallState::default()
        }));
        let registry = DriverRegistry::default()
            .try_with(Box::new(RetainedCallFactory {
                id: ConnectorId::parse("test/inbound-call@1").unwrap(),
                state: Arc::clone(&state),
            }))
            .unwrap();
        let mut bindings = RuntimeBindings::try_new(&plan, &registry).unwrap();
        let mut vm = Vm::boot(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));

        let Some(crate::boundary::InboundAdmission::Accepted(accepted)) = bindings
            .pump_inbound(&mut vm, &mut journal)
            .expect("the request reaches atomic admission")
        else {
            panic!("the request must be accepted")
        };
        let Some(InboundExchangeRef::Call(call_ref)) = accepted.exchange else {
            panic!("the accepted request opens one inbound Call")
        };
        assert_eq!(
            bindings.pump_inbound_call_events(&mut vm).unwrap().queued,
            1
        );
        vm.drain_with_bindings(&mut journal, &mut bindings)
            .expect("the accepted terminal and late graph response settle deterministically");

        let response_report = bindings.pump_inbound_call_responses(&mut vm).unwrap();
        assert_eq!((response_report.started, response_report.pending), (0, 0));
        assert!(state.lock().unwrap().responses.is_empty());
        let terminals = journal
            .records()
            .iter()
            .filter(|record| {
                matches!(
                    record,
                    crate::journal::Record::Transaction(transaction)
                        if transaction.exchange_exit.as_deref()
                            == Some(&crate::journal::InboundExchangeExitRecord::Call(call_ref))
                )
            })
            .count();
        assert_eq!(terminals, 1);
        let orphan = journal.records().iter().find_map(|record| match record {
            crate::journal::Record::Transaction(transaction)
                if transaction.owner_path.to_string() == "server"
                    && transaction.trigger
                        == node::Trigger::Port(node::Port::new("in").unwrap())
                    && transaction.exchange_exit.is_none() =>
            {
                Some(transaction)
            }
            _ => None,
        });
        let orphan = orphan.expect("the late response has one observable consume-only record");
        assert_eq!(orphan.consumed.len(), 1);
        assert!(orphan.emissions.is_empty());
        assert!(journal.open_inbound_calls().is_empty());
    }

    #[test]
    fn inbound_proposals_ack_after_commit_and_remain_retained_under_pressure() {
        let plan = inbound_event_plan();
        let state = Arc::new(Mutex::new(InboundQueueState {
            proposals: (0..=1024)
                .map(|reference| inbound_proposal(reference, "out"))
                .collect(),
            decisions: Vec::new(),
        }));
        let registry = DriverRegistry::default()
            .try_with(Box::new(RetainedInboundFactory {
                id: ConnectorId::parse("test/event-source@1").unwrap(),
                state: Arc::clone(&state),
            }))
            .unwrap();
        let mut bindings = RuntimeBindings::try_new(&plan, &registry).unwrap();
        let mut vm = Vm::boot(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));

        for expected in 0..1024 {
            let Some(crate::boundary::InboundAdmission::Accepted(accepted)) = bindings
                .pump_inbound(&mut vm, &mut journal)
                .expect("the bounded intake stays healthy")
            else {
                panic!("proposal {expected} should fit")
            };
            let state = state.lock().unwrap();
            assert_eq!(state.decisions.len(), expected as usize + 1);
            assert_eq!(state.decisions.last().unwrap().0.get(), expected);
            assert!(matches!(
                state.decisions.last().unwrap().1,
                InboundDecision::Accepted(observed) if observed == accepted
            ));
            assert!(journal.find_envelope(accepted.envelope).is_some());
        }
        let connector_roots = journal
            .records()
            .iter()
            .filter_map(|record| match record {
                crate::journal::Record::Inbound(inbound) => inbound.connector_path.as_ref(),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(connector_roots.len(), 1024);
        assert!(
            connector_roots
                .iter()
                .all(|path| path.to_string() == "source")
        );

        let records_before = journal.records().len();
        assert_eq!(
            bindings.pump_inbound(&mut vm, &mut journal).unwrap(),
            Some(crate::boundary::InboundAdmission::Pending)
        );
        assert_eq!(journal.records().len(), records_before);
        assert_eq!(state.lock().unwrap().decisions.len(), 1024);
        let retained = bindings.next_inbound().unwrap().unwrap();
        assert_eq!(retained.proposal().reference.get(), 1024);
        assert_eq!(
            bindings.pump_inbound(&mut vm, &mut journal).unwrap(),
            Some(crate::boundary::InboundAdmission::Pending),
            "re-polling cannot replace or acknowledge the retained proposal"
        );
        assert_eq!(journal.records().len(), records_before);
        assert_eq!(state.lock().unwrap().decisions.len(), 1024);
    }

    #[test]
    fn an_inbound_contract_rejection_releases_the_exact_proposal_without_a_fact() {
        let plan = inbound_event_plan();
        let state = Arc::new(Mutex::new(InboundQueueState {
            proposals: VecDeque::from([inbound_proposal(7, "missing")]),
            decisions: Vec::new(),
        }));
        let registry = DriverRegistry::default()
            .try_with(Box::new(RetainedInboundFactory {
                id: ConnectorId::parse("test/event-source@1").unwrap(),
                state: Arc::clone(&state),
            }))
            .unwrap();
        let mut bindings = RuntimeBindings::try_new(&plan, &registry).unwrap();
        let mut vm = Vm::boot(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        assert_eq!(
            bindings.pump_inbound(&mut vm, &mut journal).unwrap(),
            Some(crate::boundary::InboundAdmission::Rejected(
                crate::boundary::InboundRejection::UnknownPort
            ))
        );
        assert!(journal.records().is_empty());
        assert_eq!(
            state.lock().unwrap().decisions,
            [(
                InboundProposalRef::new(7),
                InboundDecision::Rejected(crate::boundary::InboundRejection::UnknownPort)
            )]
        );
    }

    #[test]
    fn inbound_session_framing_cannot_contradict_the_exchange_fact() {
        let plan = inbound_event_plan();
        let mut proposal = inbound_proposal(9, "out");
        proposal.session = Some(crate::boundary::InboundSession::Open);
        let state = Arc::new(Mutex::new(InboundQueueState {
            proposals: VecDeque::from([proposal]),
            decisions: Vec::new(),
        }));
        let registry = DriverRegistry::default()
            .try_with(Box::new(RetainedInboundFactory {
                id: ConnectorId::parse("test/event-source@1").unwrap(),
                state,
            }))
            .unwrap();
        let mut bindings = RuntimeBindings::try_new(&plan, &registry).unwrap();

        assert!(matches!(
            bindings.next_inbound(),
            Err(crate::boundary::DriverRouteError::InboundExchangeMismatch {
                exchange: ConnectorExchange::Event,
                ..
            })
        ));
    }

    #[test]
    fn an_inbound_family_cannot_forge_another_connector_as_its_source() {
        let plan = inbound_event_plan();
        let mut proposal = inbound_proposal(11, "out");
        proposal.submission.provenance =
            InboundEnvelopeProvenance::single(node::Path::new(["other"]).unwrap());
        let state = Arc::new(Mutex::new(InboundQueueState {
            proposals: VecDeque::from([proposal]),
            decisions: Vec::new(),
        }));
        let registry = DriverRegistry::default()
            .try_with(Box::new(RetainedInboundFactory {
                id: ConnectorId::parse("test/event-source@1").unwrap(),
                state,
            }))
            .unwrap();
        let mut bindings = RuntimeBindings::try_new(&plan, &registry).unwrap();

        assert!(matches!(
            bindings.next_inbound(),
            Err(crate::boundary::DriverRouteError::InboundSourceMismatch {
                source,
                ..
            }) if source.to_string() == "other"
        ));
    }

    #[test]
    fn an_event_driver_passes_the_exchange_specific_conformance_profile() {
        let plan = inbound_event_plan();
        let state = Arc::new(Mutex::new(InboundQueueState {
            proposals: VecDeque::from([inbound_proposal(4, "out")]),
            decisions: Vec::new(),
        }));
        let registry = DriverRegistry::default()
            .try_with(Box::new(RetainedInboundFactory {
                id: ConnectorId::parse("test/event-source@1").unwrap(),
                state,
            }))
            .unwrap();
        let ports = crate::stdlib::PROCESS
            .ports
            .iter()
            .filter(|port| port.name != "exit")
            .cloned()
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let contract = Box::leak(Box::new(crate::contract::ConnectorContract {
            id: "test/event-source@1",
            name: "EventSource",
            roles: ConnectorRoles::Inbound,
            exchange: ConnectorExchange::Event,
            ports: Box::leak(ports),
            common: crate::stdlib::PROCESS.common,
            selector: crate::stdlib::PROCESS.selector.clone(),
        }));
        let profile = crate::conformance::EventProfile {
            contract,
            plan: &plan,
            expected_outputs: vec![Data::materialized(Value::Text(String::from("event-4")))],
        };

        let report = crate::conformance::verify_event(&profile, &registry);
        crate::conformance::assert_conformant(&report, &[]);
        assert_eq!(report.proven_cases, ["event-admission"]);
    }

    #[test]
    fn two_connector_families_bind_and_route_without_vm_name_dispatch() {
        let plan = two_family_plan();
        let starts = Arc::new(Mutex::new(Vec::new()));
        let registry = DriverRegistry::default()
            .try_with(routing_factory(
                "test/alpha@1",
                ConnectorRoles::Outbound,
                Some(ConnectorRoles::Outbound),
                &starts,
            ))
            .unwrap()
            .try_with(routing_factory(
                "test/beta@1",
                ConnectorRoles::Outbound,
                Some(ConnectorRoles::Outbound),
                &starts,
            ))
            .unwrap();
        let mut bindings = RuntimeBindings::try_new(&plan, &registry).unwrap();
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&plan, GraphVersion(1));
        vm.inject(
            &mut journal,
            InboundSubmission {
                provenance: InboundEnvelopeProvenance::single(node::Path::new(["test"]).unwrap()),
                port: node::Port::new("in").unwrap(),
                draft: Draft {
                    data: Data::materialized(Value::Text(String::from("request"))),
                    ext: Namespace::new(),
                },
                sequence: SubmissionSequence::Next,
            },
        )
        .unwrap();
        for _ in 0..2 {
            vm.drain_with_bindings(&mut journal, &mut bindings).unwrap();
            vm.pump(&mut journal, &mut bindings).unwrap();
        }
        vm.drain_with_bindings(&mut journal, &mut bindings).unwrap();

        assert_eq!(
            *starts.lock().unwrap(),
            [
                String::from("test/alpha@1:first"),
                String::from("test/beta@1:second"),
            ]
        );
        assert_eq!(
            vm.outputs()[0].envelope.data(),
            &Data::materialized(Value::Text(String::from("test/beta@1")))
        );
        assert!(
            bindings.calls.is_empty(),
            "settled calls do not accumulate in the routing table"
        );
    }

    #[test]
    fn binding_refuses_missing_wrong_and_extra_roles_before_execution() {
        let plan = two_family_plan();
        let starts = Arc::new(Mutex::new(Vec::new()));
        assert!(matches!(
            RuntimeBindings::try_new(&plan, &DriverRegistry::default()),
            Err(DriverBindingError::Missing { .. })
        ));

        let wrong = DriverRegistry::default()
            .try_with(routing_factory(
                "test/alpha@1",
                ConnectorRoles::Inbound,
                Some(ConnectorRoles::Inbound),
                &starts,
            ))
            .unwrap();
        assert!(matches!(
            RuntimeBindings::try_new(&plan, &wrong),
            Err(DriverBindingError::MissingRoles { .. })
        ));

        let extra = DriverRegistry::default()
            .try_with(routing_factory(
                "test/alpha@1",
                ConnectorRoles::Outbound,
                Some(ConnectorRoles::InboundAndOutbound),
                &starts,
            ))
            .unwrap()
            .try_with(routing_factory(
                "test/beta@1",
                ConnectorRoles::Outbound,
                Some(ConnectorRoles::Outbound),
                &starts,
            ))
            .unwrap();
        assert!(matches!(
            RuntimeBindings::try_new(&plan, &extra),
            Err(DriverBindingError::BoundRolesMismatch { .. })
        ));
        assert!(
            starts.lock().unwrap().is_empty(),
            "no Driver effect started"
        );
    }
}
