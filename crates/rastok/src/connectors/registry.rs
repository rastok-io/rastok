//! Physical Driver composition for one runtime build.
//!
//! The compiler's [`crate::contract::ContractCatalog`] owns semantic sheets.
//! This registry owns implementation factories.  Their stable Connector ids
//! meet only at binding; registration order is never selection policy.

use crate::boundary::{
    CallConnectorDriver, CallId, CallOpen, CallStart, Completion, DriverRouteError,
    InboundCallResponseStart, InboundCallTimeoutStart, InboundConnectorDriver, InboundExchangeId,
    IngressDecision, IngressProposal, OutboundConnectorBindings, OutboundConnectorDriver,
    RequestPreparation, Response, SchedulerCompletion, SessionConnectorBindings,
    SessionConnectorDriver, SessionId, SessionObservation, SessionOpen, SessionWrite,
};
use crate::connector::{ConnectorDirection, ConnectorExchange, ContractId};
use crate::envelope::{Data, Envelope};
use crate::node;
use crate::plan::ValidatedExecutionPlan;
use crate::runtime::vm::{InboundCallPollReport, Vm, VmFault};
use crate::{boundary::IngressAdmission, journal::Journal};
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
    fn connector_id(&self) -> &ContractId;

    /// Roles the implementation exposes.
    fn roles(&self) -> ConnectorDirection;

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

    /// Binds with deployment inputs already resolved and attested. The
    /// default keeps every existing family honest: it delegates only when no
    /// selected Node carries a boot reference. A family opens a boot setting
    /// by overriding this method and consuming it before returning.
    fn bind_with_boot<'plan>(
        &self,
        plan: &'plan ValidatedExecutionPlan,
        nodes: &[node::Path],
        _boot: &crate::deployment::BootBindings,
    ) -> Result<Box<dyn BoundConnectorDriver + 'plan>, DriverFactoryError> {
        for planned in &plan.plan().nodes {
            if !nodes.contains(&planned.path) {
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
        self.bind(plan, nodes)
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
    factories: BTreeMap<ContractId, Box<dyn ConnectorDriverFactory>>,
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
        id: &ContractId,
        required: ConnectorDirection,
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
    nodes: Vec<(node::Path, ConnectorDirection, ConnectorExchange)>,
    required: ConnectorDirection,
}

struct BoundFamily<'plan> {
    driver: Box<dyn BoundConnectorDriver + 'plan>,
    uses_inbound: bool,
    inbound_paths: BTreeSet<node::Path>,
    exchange: ConnectorExchange,
    pending_inbound: Option<IngressProposal>,
    outbound_path: Option<node::Path>,
    session_path: Option<node::Path>,
}

/// One retained proposal paired with the exact Driver family that owns it.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutedInboundProposal {
    connector: ContractId,
    proposal: IngressProposal,
}

/// Failure of one inbound Driver/VM boundary round.
#[derive(Debug, Clone, PartialEq)]
pub enum InboundPollError {
    /// The bound Driver registry contradicted its retained routing state.
    Driver(DriverRouteError),
    /// The VM or journal raised a fail-stop integrity fault.
    Machine(VmFault),
}

/// One bounded pass offering already-journaled graph responses to inbound
/// Call Drivers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InboundCallResponsePollReport {
    /// Responses whose protocol write is now Driver-owned.
    pub started: usize,
    /// Responses retained by the VM because the Driver's finite capacity is
    /// currently full.
    pub pending: usize,
}

/// One bounded pass transferring scheduler-owned inbound request expiries to
/// their exact Driver family.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InboundCallTimeoutPollReport {
    /// Timeout attempts whose protocol action is now Driver-owned.
    pub started: usize,
    /// Timeout attempts retained by the VM under finite Driver pressure.
    pub pending: usize,
}

impl core::fmt::Display for InboundPollError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Driver(error) => error.fmt(formatter),
            Self::Machine(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for InboundPollError {}

impl RoutedInboundProposal {
    /// Stable family that must receive the eventual decision callback.
    #[must_use]
    pub fn connector(&self) -> &ContractId {
        &self.connector
    }

    /// Retained proposal presented for atomic VM admission.
    #[must_use]
    pub fn proposal(&self) -> &IngressProposal {
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
    families: BTreeMap<ContractId, BoundFamily<'plan>>,
    /// Last inbound family polled. Cycling from this key prevents one
    /// backpressured transport from blocking an unrelated family.
    inbound_cursor: Option<ContractId>,
    outbound: BTreeMap<node::Path, ContractId>,
    calls: BTreeMap<CallId, ContractId>,
    inbound_calls: BTreeMap<crate::boundary::InboundCallId, ContractId>,
    session_outbound: BTreeMap<node::Path, ContractId>,
    sessions: BTreeMap<SessionId, ContractId>,
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
        Self::try_new_with_boot(plan, registry, &crate::deployment::BootBindings::empty())
    }

    /// Binds with one complete deployment input set before a VM or journal
    /// exists. Secret material can cross only a factory override that owns a
    /// contract-declared secret-compatible setting.
    pub fn try_new_with_boot(
        plan: &'plan ValidatedExecutionPlan,
        registry: &DriverRegistry,
        boot: &crate::deployment::BootBindings,
    ) -> Result<Self, DriverBindingError> {
        let mut requirements: BTreeMap<ContractId, FamilyRequirement> = BTreeMap::new();
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
            let mut driver = factory
                .bind_with_boot(plan, &paths, boot)
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
        decision: IngressDecision,
    ) -> Result<(), DriverRouteError> {
        let Some(family) = self.families.get_mut(&routed.connector) else {
            return Err(DriverRouteError::UnknownIngressProposal {
                connector: routed.connector.clone(),
                reference: routed.proposal.reference,
            });
        };
        let Some(pending) = &family.pending_inbound else {
            return Err(DriverRouteError::UnknownIngressProposal {
                connector: routed.connector.clone(),
                reference: routed.proposal.reference,
            });
        };
        if pending != &routed.proposal {
            return Err(DriverRouteError::UnknownIngressProposal {
                connector: routed.connector.clone(),
                reference: routed.proposal.reference,
            });
        }
        if let IngressDecision::Accepted(accepted) = decision {
            match accepted.exchange {
                Some(InboundExchangeId::Call(call_id)) => {
                    if let Some(expected) = self.inbound_calls.get(&call_id)
                        && expected != &routed.connector
                    {
                        return Err(DriverRouteError::CrossFamilyInboundCall {
                            call_id,
                            expected: expected.clone(),
                            actual: routed.connector.clone(),
                        });
                    }
                    self.inbound_calls.insert(call_id, routed.connector.clone());
                }
                Some(InboundExchangeId::Session(session)) => {
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
    pub fn poll_inbound(
        &mut self,
        vm: &mut Vm<'plan>,
        journal: &mut dyn Journal,
    ) -> Result<Option<IngressAdmission>, InboundPollError> {
        let Some(routed) = self.next_inbound().map_err(InboundPollError::Driver)? else {
            return Ok(None);
        };
        let admission = vm
            .admit_ingress(journal, routed.proposal())
            .map_err(InboundPollError::Machine)?;
        match admission {
            IngressAdmission::Accepted(accepted) => self
                .decide_inbound(&routed, IngressDecision::Accepted(accepted))
                .map_err(InboundPollError::Driver)?,
            IngressAdmission::Rejected(reason) => self
                .decide_inbound(&routed, IngressDecision::Rejected(reason))
                .map_err(InboundPollError::Driver)?,
            IngressAdmission::Pending => {}
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
    pub fn poll_inbound_call_responses(
        &mut self,
        vm: &mut Vm<'plan>,
    ) -> Result<InboundCallResponsePollReport, DriverRouteError> {
        let mut report = InboundCallResponsePollReport::default();
        for response in vm.ready_inbound_call_responses() {
            let Some(id) = self.inbound_calls.get(&response.call_id).cloned() else {
                return Err(DriverRouteError::UncorrelatedInboundCall(response.call_id));
            };
            let Some(family) = self.families.get_mut(&id) else {
                return Err(DriverRouteError::UncorrelatedInboundCall(response.call_id));
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
                        return Err(DriverRouteError::UncorrelatedInboundCall(response.call_id));
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
    pub fn poll_inbound_call_timeouts(
        &mut self,
        vm: &mut Vm<'plan>,
    ) -> Result<InboundCallTimeoutPollReport, DriverRouteError> {
        let mut report = InboundCallTimeoutPollReport::default();
        for timeout in vm.ready_inbound_call_timeouts() {
            let Some(id) = self.inbound_calls.get(&timeout.call_id).cloned() else {
                return Err(DriverRouteError::UncorrelatedInboundCall(timeout.call_id));
            };
            let Some(family) = self.families.get_mut(&id) else {
                return Err(DriverRouteError::UncorrelatedInboundCall(timeout.call_id));
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
                        return Err(DriverRouteError::UncorrelatedInboundCall(timeout.call_id));
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
    pub fn poll_inbound_call_events(
        &mut self,
        vm: &mut Vm<'plan>,
    ) -> Result<InboundCallPollReport, DriverRouteError> {
        let ids: Vec<_> = self
            .families
            .iter()
            .filter_map(|(id, family)| {
                (family.uses_inbound && family.exchange == ConnectorExchange::Call)
                    .then_some(id.clone())
            })
            .collect();
        let mut report = InboundCallPollReport::default();
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
                let Some(expected) = self.inbound_calls.get(&event.call_id).cloned() else {
                    report.uncorrelated.push(event);
                    continue;
                };
                if expected != id {
                    return Err(DriverRouteError::CrossFamilyInboundCall {
                        call_id: event.call_id,
                        expected,
                        actual: id,
                    });
                }
                let call_id = event.call_id;
                let mut observed = vm.queue_inbound_call_events(core::iter::once(event));
                if observed.queued == 1 || !observed.uncorrelated.is_empty() {
                    self.inbound_calls.remove(&call_id);
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
    ) -> Result<(ContractId, &mut dyn SessionConnectorDriver), DriverRouteError> {
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

    fn session_family(&self, connector: &node::Path) -> Result<ContractId, DriverRouteError> {
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

    fn cancel(&mut self, connector: &node::Path, call_id: &CallId) -> Result<(), DriverRouteError> {
        self.outbound_driver(connector)?.cancel(call_id);
        self.calls.remove(call_id);
        Ok(())
    }

    fn start(
        &mut self,
        intent: &CallOpen,
        request: &Envelope,
    ) -> Result<CallStart, DriverRouteError> {
        let connector = &intent.connector_path;
        let Some(id) = self.outbound.get(connector).cloned() else {
            return Err(DriverRouteError::UnboundOutbound(connector.clone()));
        };
        let start = self.outbound_driver(connector)?.start(intent, request);
        if start == CallStart::Started {
            self.calls.insert(intent.call_id, id);
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
                if let Some(expected) = self.calls.get(&response.call_id)
                    && expected != id
                {
                    return Err(DriverRouteError::CrossFamilyResponse {
                        call_id: response.call_id,
                        expected: expected.clone(),
                        actual: id.clone(),
                    });
                }
                self.calls.remove(&response.call_id);
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
        intent: &SessionOpen,
        request: &Envelope,
    ) -> Result<crate::boundary::SessionStart, DriverRouteError> {
        SessionConnectorBindings::open_session(self, intent, request)
    }

    fn prepare_session_write(
        &mut self,
        connector: &node::Path,
        session: SessionId,
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
        intent: &SessionWrite,
        message: &Envelope,
    ) -> Result<crate::boundary::SessionStart, DriverRouteError> {
        SessionConnectorBindings::write_session(self, intent, message)
    }

    fn close_session(
        &mut self,
        connector: &node::Path,
        session: SessionId,
    ) -> Result<crate::boundary::SessionStart, DriverRouteError> {
        SessionConnectorBindings::close_session(self, connector, session)
    }

    fn session_events(&mut self) -> Result<Vec<SessionObservation>, DriverRouteError> {
        SessionConnectorBindings::session_events(self)
    }

    fn abort_session(
        &mut self,
        connector: &node::Path,
        session: SessionId,
    ) -> Result<(), DriverRouteError> {
        SessionConnectorBindings::abort_session(self, connector, session)
    }

    fn forget_session(&mut self, session: SessionId) -> Result<(), DriverRouteError> {
        SessionConnectorBindings::forget_session(self, session)
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
        intent: &SessionOpen,
        request: &Envelope,
    ) -> Result<crate::boundary::SessionStart, DriverRouteError> {
        let (id, driver) = self.session_driver(&intent.connector_path)?;
        let start = driver.open_session(intent, request);
        // Ownership follows the journaled intent, even while local capacity
        // keeps creation Pending. Closing must fence that resource reference
        // without first starting the effect merely to establish correlation.
        self.sessions.insert(intent.session_id, id);
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
        session: SessionId,
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
        intent: &SessionWrite,
        message: &Envelope,
    ) -> Result<crate::boundary::SessionStart, DriverRouteError> {
        let actual = self.session_family(&intent.connector_path)?;
        let Some(expected) = self.sessions.get(&intent.session_id) else {
            return Err(DriverRouteError::UncorrelatedSession(intent.session_id));
        };
        if expected != &actual {
            return Err(DriverRouteError::CrossFamilySession {
                session: intent.session_id,
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
        session: SessionId,
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

    fn abort_session(
        &mut self,
        connector: &node::Path,
        session: SessionId,
    ) -> Result<(), DriverRouteError> {
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
        driver.abort_session(session);
        Ok(())
    }

    fn session_events(&mut self) -> Result<Vec<SessionObservation>, DriverRouteError> {
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
                    SessionObservation::Opened(session)
                    | SessionObservation::WriteSettled { session, .. }
                    | SessionObservation::Exited { session, .. } => *session,
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

    fn forget_session(&mut self, session: SessionId) -> Result<(), DriverRouteError> {
        let Some(id) = self.sessions.get(&session) else {
            return Ok(());
        };
        let family = self
            .families
            .get_mut(id)
            .ok_or(DriverRouteError::UncorrelatedSession(session))?;
        let path = family
            .session_path
            .as_ref()
            .ok_or(DriverRouteError::UncorrelatedSession(session))?;
        let driver = family
            .driver
            .outbound()
            .ok_or_else(|| DriverRouteError::UnavailableOutbound(path.clone()))?
            .session()
            .ok_or_else(|| DriverRouteError::UnavailableSession(path.clone()))?;
        driver.release_session(session);
        self.sessions.remove(&session);
        Ok(())
    }
}

fn union_roles(left: ConnectorDirection, right: ConnectorDirection) -> ConnectorDirection {
    if left == right {
        left
    } else {
        ConnectorDirection::InboundAndOutbound
    }
}

fn roles_of(driver: &mut dyn BoundConnectorDriver) -> Option<ConnectorDirection> {
    match (driver.inbound().is_some(), driver.outbound().is_some()) {
        (true, true) => Some(ConnectorDirection::InboundAndOutbound),
        (true, false) => Some(ConnectorDirection::Inbound),
        (false, true) => Some(ConnectorDirection::Outbound),
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
        CallConnectorDriver, CallOpen, CallStart, Completion, Draft, HttpServerExit,
        InboundCallConnectorDriver, InboundCallObservation, InboundCallResponse,
        InboundCallResponseStart, InboundExchangeId, IngressDecision, IngressEnvelopeProvenance,
        IngressProposal, IngressProposalHandle, IngressSubmission, OutboundConnectorBindings,
        OutboundConnectorDriver, ProcessExit, RequestPreparation, Response, SchedulerCompletion,
        SessionConnectorBindings, SessionConnectorDriver, SessionId, SessionObservation,
        SessionOpen, SessionWrite, SubmissionSequence,
    };
    use crate::compiler::compile;
    use crate::connector::{ConnectorDirection, ConnectorExchange, ContractId};
    use crate::contract::ContractCatalog;
    use crate::envelope::{Data, Envelope, Namespace, Value};
    use crate::journal::{Journal, MemJournal, ids};
    use crate::node;
    use crate::plan::{GraphVersion, ValidatedExecutionPlan};
    use crate::runtime::vm::Vm;
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    use std::sync::{Arc, Mutex};

    struct FakeFactory {
        id: ContractId,
        roles: ConnectorDirection,
    }

    impl ConnectorDriverFactory for FakeFactory {
        fn connector_id(&self) -> &ContractId {
            &self.id
        }

        fn roles(&self) -> ConnectorDirection {
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

    fn fake(id: &str, roles: ConnectorDirection) -> Box<dyn ConnectorDriverFactory> {
        Box::new(FakeFactory {
            id: ContractId::parse(id).unwrap(),
            roles,
        })
    }

    #[test]
    fn registration_is_exact_role_aware_and_never_first_wins() {
        let id = ContractId::parse("org.rastok.test/duplex@1").unwrap();
        let registry = DriverRegistry::default()
            .try_with(fake(id.as_str(), ConnectorDirection::Inbound))
            .unwrap();

        assert_eq!(
            registry
                .require(&id, ConnectorDirection::Inbound)
                .unwrap()
                .connector_id(),
            &id
        );
        assert_eq!(
            registry.require(&id, ConnectorDirection::Outbound).err(),
            Some(DriverBindingError::MissingRoles {
                id: id.clone(),
                required: ConnectorDirection::Outbound,
                available: ConnectorDirection::Inbound,
            })
        );
        assert_eq!(
            registry
                .require(
                    &ContractId::parse("org.rastok.test/missing@1").unwrap(),
                    ConnectorDirection::Outbound,
                )
                .err(),
            Some(DriverBindingError::Missing {
                id: ContractId::parse("org.rastok.test/missing@1").unwrap(),
            })
        );
        assert_eq!(
            registry
                .try_with(fake(id.as_str(), ConnectorDirection::InboundAndOutbound))
                .unwrap_err(),
            DriverRegistrationError::Duplicate(id),
        );
    }

    #[derive(Default)]
    struct EmptyInbound;

    impl crate::boundary::InboundConnectorDriver for EmptyInbound {
        fn proposal(&mut self) -> Option<IngressProposal> {
            None
        }

        fn decide(&mut self, _: IngressProposalHandle, _: IngressDecision) {}
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

        fn start(&mut self, intent: &CallOpen, _: &Envelope) -> CallStart {
            self.starts
                .lock()
                .unwrap()
                .push(format!("{}:{}", self.family, intent.connector_path));
            self.answers.push(Response {
                call_id: intent.call_id,
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
        id: ContractId,
        advertised: ConnectorDirection,
        actual: Option<ConnectorDirection>,
        starts: Arc<Mutex<Vec<String>>>,
    }

    impl ConnectorDriverFactory for RoutingFactory {
        fn connector_id(&self) -> &ContractId {
            &self.id
        }

        fn roles(&self) -> ConnectorDirection {
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
        advertised: ConnectorDirection,
        actual: Option<ConnectorDirection>,
        starts: &Arc<Mutex<Vec<String>>>,
    ) -> Box<dyn ConnectorDriverFactory> {
        Box::new(RoutingFactory {
            id: ContractId::parse(id).unwrap(),
            advertised,
            actual,
            starts: Arc::clone(starts),
        })
    }

    fn two_family_plan_with_first_roles(first_roles: ConnectorDirection) -> ValidatedExecutionPlan {
        let source = r#"
graph mixed : App {
  input in: text
  output out: text
  connector first : Process {
    mode = job ; command = "cat" ; codec = lines
    idempotent = true; behavior = deterministic
  }
  connector second : Process {
    mode = job ; command = "cat" ; codec = lines
    idempotent = true; behavior = deterministic
  }
  in -> first -> second -> out
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
            spec.contract = ContractId::parse(match planned.path.to_string().as_str() {
                "first" => "org.rastok.test/alpha@1",
                "second" => "org.rastok.test/beta@1",
                other => panic!("unexpected probe node `{other}`"),
            })
            .unwrap();
            spec.idempotent = None;
            spec.attributes.retain(|(name, _)| name != "idempotent");
            if planned.path.to_string() == "first" {
                spec.roles = first_roles;
            }
        }
        ValidatedExecutionPlan::try_new(raw).expect("foreign contracts remain structural")
    }

    fn two_family_plan() -> ValidatedExecutionPlan {
        two_family_plan_with_first_roles(ConnectorDirection::Outbound)
    }

    fn session_family_plan() -> ValidatedExecutionPlan {
        let base = two_family_plan_with_first_roles(ConnectorDirection::InboundAndOutbound);
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
    idempotent = true; behavior = deterministic
  }
  operator relay { input in: text ; on in { out = data } }
  in -> nothing
  source.out -> relay -> out ; source.err -> nothing
  source.exit -> nothing
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
        spec.contract = ContractId::parse("org.rastok.test/event-source@1").unwrap();
        spec.idempotent = None;
        spec.attributes.retain(|(name, _)| name != "idempotent");
        spec.roles = ConnectorDirection::Inbound;
        spec.exchange = ConnectorExchange::Event;
        source.ports.retain(|port| port.port.name() != "exit");
        raw.edges.retain(|edge| edge.from.port.name() != "exit");
        ValidatedExecutionPlan::try_new(raw).expect("the foreign Event contract is structural")
    }

    struct DuplexDriver {
        observations: Arc<Mutex<Vec<&'static str>>>,
    }

    impl crate::boundary::InboundConnectorDriver for DuplexDriver {
        fn proposal(&mut self) -> Option<IngressProposal> {
            self.observations.lock().unwrap().push("inbound");
            None
        }

        fn decide(&mut self, _: IngressProposalHandle, _: IngressDecision) {}

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

        fn events(&mut self) -> Vec<InboundCallObservation> {
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

        fn start(&mut self, _: &CallOpen, _: &Envelope) -> CallStart {
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
        id: ContractId,
        observations: Arc<Mutex<Vec<&'static str>>>,
    }

    impl ConnectorDriverFactory for DuplexFactory {
        fn connector_id(&self) -> &ContractId {
            &self.id
        }

        fn roles(&self) -> ConnectorDirection {
            ConnectorDirection::InboundAndOutbound
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
        let plan = two_family_plan_with_first_roles(ConnectorDirection::InboundAndOutbound);
        let observations = Arc::new(Mutex::new(Vec::new()));
        let starts = Arc::new(Mutex::new(Vec::new()));
        let registry = DriverRegistry::default()
            .try_with(Box::new(DuplexFactory {
                id: ContractId::parse("org.rastok.test/alpha@1").unwrap(),
                observations: Arc::clone(&observations),
            }))
            .unwrap()
            .try_with(routing_factory(
                "org.rastok.test/beta@1",
                ConnectorDirection::Outbound,
                Some(ConnectorDirection::Outbound),
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
        events: Vec<SessionObservation>,
        closed: BTreeSet<SessionId>,
    }

    impl crate::boundary::InboundConnectorDriver for SessionDuplexDriver {
        fn proposal(&mut self) -> Option<IngressProposal> {
            self.observations.lock().unwrap().push("inbound");
            None
        }

        fn decide(&mut self, _: IngressProposalHandle, _: IngressDecision) {
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
            intent: &SessionOpen,
            _: &Envelope,
        ) -> crate::boundary::SessionStart {
            self.observations.lock().unwrap().push("open");
            self.events
                .insert(0, SessionObservation::Opened(intent.session_id));
            crate::boundary::SessionStart::Started
        }

        fn prepare_session_write(
            &mut self,
            _: &node::Path,
            _: SessionId,
            data: &Data,
            _: &BTreeMap<String, crate::value::Value>,
        ) -> RequestPreparation {
            crate::boundary::prepare_process_request(crate::boundary::RequestCodec::Lines, data)
        }

        fn write_session(
            &mut self,
            intent: &SessionWrite,
            _: &Envelope,
        ) -> crate::boundary::SessionStart {
            self.observations.lock().unwrap().push("write");
            self.events.insert(
                0,
                SessionObservation::WriteSettled {
                    session: intent.session_id,
                    write: intent.write_ref,
                },
            );
            crate::boundary::SessionStart::Started
        }

        fn close_session(&mut self, session: SessionId) -> crate::boundary::SessionStart {
            if self.closed.insert(session) {
                self.observations.lock().unwrap().push("close");
                self.events.insert(
                    0,
                    SessionObservation::Exited {
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

        fn session_events(&mut self) -> Vec<SessionObservation> {
            core::mem::take(&mut self.events)
        }

        fn abort_session(&mut self, session: SessionId) {
            if self.closed.insert(session) {
                self.observations.lock().unwrap().push("abort");
            }
        }

        fn release_session(&mut self, session: SessionId) {
            self.observations.lock().unwrap().push("release");
            self.closed.remove(&session);
            self.events.retain(|event| match event {
                SessionObservation::Opened(reference)
                | SessionObservation::WriteSettled {
                    session: reference, ..
                }
                | SessionObservation::Exited {
                    session: reference, ..
                } => *reference != session,
            });
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
        id: ContractId,
        observations: Arc<Mutex<Vec<&'static str>>>,
    }

    impl ConnectorDriverFactory for SessionDuplexFactory {
        fn connector_id(&self) -> &ContractId {
            &self.id
        }

        fn roles(&self) -> ConnectorDirection {
            ConnectorDirection::InboundAndOutbound
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
                id: ContractId::parse("org.rastok.test/alpha@1").unwrap(),
                observations: Arc::clone(&observations),
            }))
            .unwrap()
            .try_with(routing_factory(
                "org.rastok.test/beta@1",
                ConnectorDirection::Outbound,
                Some(ConnectorDirection::Outbound),
                &starts,
            ))
            .unwrap();
        let mut bindings = RuntimeBindings::try_new(&plan, &registry).unwrap();
        assert!(bindings.next_inbound().unwrap().is_none());

        let mut journal = MemJournal::new(GraphVersion(1));
        let request = journal
            .append_ingress(IngressSubmission {
                provenance: IngressEnvelopeProvenance::single(node::Path::new(["world"]).unwrap()),
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
            SessionOpen {
                session_id: ids::outbound_session_id(&connector, emitting_id, prepared.digest()),
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
                SessionObservation::Opened(second.session_id),
                SessionObservation::Opened(first.session_id),
            ],
            "reverse arrival remains correlated by SessionId"
        );
        journal.confirm_session_open(&first.session_id).unwrap();
        journal.confirm_session_open(&second.session_id).unwrap();

        let write = |session: SessionId, byte| {
            let prepared = crate::boundary::PreparedRequest::seal(vec![byte]);
            let emitting_id = crate::envelope::Id::from_bytes([byte; 16]);
            SessionWrite {
                write_ref: ids::session_write_id(session, emitting_id, prepared.digest()),
                session_id: session,
                emitting_id,
                connector_path: connector.clone(),
                prepared,
            }
        };
        let first_write = write(first.session_id, 3);
        let second_write = write(second.session_id, 4);
        for intent in [&first_write, &second_write] {
            journal.open_session_write(intent.clone()).unwrap();
            SessionConnectorBindings::write_session(&mut bindings, intent, &request).unwrap();
        }
        assert_eq!(
            SessionConnectorBindings::session_events(&mut bindings).unwrap(),
            [
                SessionObservation::WriteSettled {
                    session: second.session_id,
                    write: second_write.write_ref,
                },
                SessionObservation::WriteSettled {
                    session: first.session_id,
                    write: first_write.write_ref,
                },
            ]
        );
        assert_eq!(
            *observations.lock().unwrap(),
            ["inbound", "open", "open", "write", "write"]
        );

        SessionConnectorBindings::abort_session(&mut bindings, &connector, first.session_id)
            .unwrap();
        SessionConnectorBindings::abort_session(&mut bindings, &connector, first.session_id)
            .unwrap();
        assert_eq!(
            observations
                .lock()
                .unwrap()
                .iter()
                .filter(|observation| **observation == "abort")
                .count(),
            1
        );
        assert!(bindings.sessions.contains_key(&first.session_id));
        assert!(
            SessionConnectorBindings::session_events(&mut bindings)
                .unwrap()
                .is_empty()
        );
        SessionConnectorBindings::forget_session(&mut bindings, first.session_id).unwrap();
        SessionConnectorBindings::forget_session(&mut bindings, first.session_id).unwrap();
        assert_eq!(
            observations
                .lock()
                .unwrap()
                .iter()
                .filter(|observation| **observation == "release")
                .count(),
            1,
            "the owning Driver releases a terminal resource once"
        );
        assert!(!bindings.sessions.contains_key(&first.session_id));
        assert!(bindings.sessions.contains_key(&second.session_id));
        assert!(matches!(
            SessionConnectorBindings::abort_session(&mut bindings, &connector, first.session_id),
            Err(crate::boundary::DriverRouteError::UncorrelatedSession(reference)) if reference == first.session_id
        ));
        SessionConnectorBindings::write_session(&mut bindings, &second_write, &request).unwrap();
    }

    #[test]
    fn a_session_driver_passes_the_exchange_specific_conformance_profile() {
        let plan = session_family_plan();
        let observations = Arc::new(Mutex::new(Vec::new()));
        let starts = Arc::new(Mutex::new(Vec::new()));
        let registry = DriverRegistry::default()
            .try_with(Box::new(SessionDuplexFactory {
                id: ContractId::parse("org.rastok.test/alpha@1").unwrap(),
                observations,
            }))
            .unwrap()
            .try_with(routing_factory(
                "org.rastok.test/beta@1",
                ConnectorDirection::Outbound,
                Some(ConnectorDirection::Outbound),
                &starts,
            ))
            .unwrap();
        let contract = Box::leak(Box::new(crate::contract::ConnectorContract {
            id: "org.rastok.test/alpha@1",
            name: "SessionProbe",
            roles: ConnectorDirection::InboundAndOutbound,
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
        proposals: VecDeque<IngressProposal>,
        decisions: Vec<(IngressProposalHandle, IngressDecision)>,
    }

    struct RetainedInboundDriver {
        state: Arc<Mutex<InboundQueueState>>,
    }

    impl crate::boundary::InboundConnectorDriver for RetainedInboundDriver {
        fn proposal(&mut self) -> Option<IngressProposal> {
            self.state.lock().unwrap().proposals.front().cloned()
        }

        fn decide(&mut self, reference: IngressProposalHandle, decision: IngressDecision) {
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
        id: ContractId,
        state: Arc<Mutex<InboundQueueState>>,
    }

    #[derive(Default)]
    struct RetainedCallState {
        proposals: VecDeque<IngressProposal>,
        decisions: Vec<(IngressProposalHandle, IngressDecision)>,
        events: Vec<InboundCallObservation>,
        responses: Vec<InboundCallResponse>,
        acknowledge_on_accept: bool,
        hold_first_response: bool,
    }

    struct RetainedCallDriver {
        state: Arc<Mutex<RetainedCallState>>,
    }

    impl crate::boundary::InboundConnectorDriver for RetainedCallDriver {
        fn proposal(&mut self) -> Option<IngressProposal> {
            self.state.lock().unwrap().proposals.front().cloned()
        }

        fn decide(&mut self, reference: IngressProposalHandle, decision: IngressDecision) {
            let mut state = self.state.lock().unwrap();
            let retained = state
                .proposals
                .pop_front()
                .expect("a decision follows a retained proposal");
            assert_eq!(retained.reference, reference);
            if state.acknowledge_on_accept
                && let IngressDecision::Accepted(accepted) = decision
            {
                let Some(InboundExchangeId::Call(call_id)) = accepted.exchange else {
                    panic!("an inbound Call acceptance carries its Call ref")
                };
                state.events.push(InboundCallObservation {
                    call_id,
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
            state.events.push(InboundCallObservation {
                call_id: response.call_id,
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

        fn events(&mut self) -> Vec<InboundCallObservation> {
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
        id: ContractId,
        state: Arc<Mutex<RetainedCallState>>,
    }

    impl ConnectorDriverFactory for RetainedCallFactory {
        fn connector_id(&self) -> &ContractId {
            &self.id
        }

        fn roles(&self) -> ConnectorDirection {
            ConnectorDirection::Inbound
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
        fn connector_id(&self) -> &ContractId {
            &self.id
        }

        fn roles(&self) -> ConnectorDirection {
            ConnectorDirection::Inbound
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

    fn inbound_proposal(reference: u64, port: &str) -> IngressProposal {
        IngressProposal {
            reference: IngressProposalHandle::new(reference),
            submission: IngressSubmission {
                provenance: IngressEnvelopeProvenance::single(node::Path::new(["source"]).unwrap()),
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
  server -> relay -> out
  server.exit -> nothing
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
        spec.contract = ContractId::parse("org.rastok.test/inbound-call@1").unwrap();

        let response = raw
            .edges
            .iter_mut()
            .find(|edge| {
                edge.from.node == Some(relay_slot)
                    && edge.to.as_ref().is_some_and(|to| to.node.is_none())
            })
            .expect("the relay boundary response exists");
        response.to = Some(crate::plan::EdgeEnd {
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
  server -> relay -> out
  server.exit -> nothing
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
                id: ContractId::parse(crate::connector::HTTP_SERVER_CONNECTOR_ID).unwrap(),
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
            proposals: VecDeque::from([IngressProposal {
                reference: IngressProposalHandle::new(1),
                submission: IngressSubmission {
                    provenance: IngressEnvelopeProvenance::single(
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
                id: ContractId::parse(crate::connector::HTTP_SERVER_CONNECTOR_ID).unwrap(),
                state: Arc::clone(&state),
            }))
            .unwrap();
        let mut bindings = RuntimeBindings::try_new(&plan, &registry).unwrap();
        let mut vm = Vm::boot(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));

        let Some(crate::boundary::IngressAdmission::Accepted(accepted)) = bindings
            .poll_inbound(&mut vm, &mut journal)
            .expect("the proposal reaches atomic admission")
        else {
            panic!("the request must be accepted")
        };
        let Some(InboundExchangeId::Call(call_id)) = accepted.exchange else {
            panic!("the accepting fact opens a Call")
        };
        assert_eq!(journal.open_inbound_calls()[0].reference, call_id);
        assert!(journal.find_envelope(accepted.envelope).is_some());
        assert_eq!(state.lock().unwrap().decisions.len(), 1);

        let report = bindings.poll_inbound_call_events(&mut vm).unwrap();
        assert_eq!(report.queued, 1);
        assert!(report.rejected.is_empty());
        vm.run_ready_with_bindings(&mut journal, &mut bindings)
            .expect("start then reply commit in causal order");
        assert!(journal.open_inbound_calls().is_empty());
        let terminals = journal
            .records()
            .iter()
            .filter_map(|record| match record {
                crate::journal::Record::Transaction(transaction)
                    if transaction.exchange_exit.as_deref()
                        == Some(&crate::journal::InboundExchangeExitRecord::Call(call_id)) =>
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
                    crate::journal::Record::Ingress(inbound)
                        if matches!(
                            inbound.exchange.as_deref(),
                            Some(crate::journal::InboundExchangeRecord::Call(call))
                                if call.reference == call_id
                        )
                )
            })
            .expect("the opening INGRESS fact is present");
        let terminal_index = journal
            .records()
            .iter()
            .position(|record| {
                matches!(
                    record,
                    crate::journal::Record::Transaction(transaction)
                        if transaction.exchange_exit.as_deref()
                            == Some(&crate::journal::InboundExchangeExitRecord::Call(call_id))
                )
            })
            .expect("the terminal TRANSACTION fact is present");
        for (index, key) in [(inbound_index, 16), (terminal_index, 38)] {
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

        state.lock().unwrap().events.push(InboundCallObservation {
            call_id,
            completion: Completion::http_server_exit(HttpServerExit::Completed { status: 202 }),
        });
        let duplicate = bindings.poll_inbound_call_events(&mut vm).unwrap();
        assert_eq!(duplicate.uncorrelated.len(), 1);
        assert_eq!(
            journal
                .records()
                .iter()
                .filter(|record| matches!(
                    record,
                    crate::journal::Record::Transaction(transaction)
                        if transaction.exchange_exit.as_deref()
                            == Some(&crate::journal::InboundExchangeExitRecord::Call(call_id))
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
            proposals: VecDeque::from([IngressProposal {
                reference: IngressProposalHandle::new(1),
                submission: IngressSubmission {
                    provenance: IngressEnvelopeProvenance::single(
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
                id: ContractId::parse("org.rastok.test/inbound-call@1").unwrap(),
                state: Arc::clone(&state),
            }))
            .unwrap();
        let mut bindings = RuntimeBindings::try_new(&plan, &registry).unwrap();
        let mut vm = Vm::boot(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));

        let Some(crate::boundary::IngressAdmission::Accepted(accepted)) = bindings
            .poll_inbound(&mut vm, &mut journal)
            .expect("the request reaches atomic admission")
        else {
            panic!("the request must be accepted")
        };
        let Some(InboundExchangeId::Call(call_id)) = accepted.exchange else {
            panic!("the accepted request opens one inbound Call")
        };
        let opening_instance = journal
            .records()
            .iter()
            .find_map(|record| match record {
                crate::journal::Record::Ingress(inbound) => {
                    inbound
                        .exchange
                        .as_deref()
                        .and_then(|exchange| match exchange {
                            crate::journal::InboundExchangeRecord::Call(call)
                                if call.reference == call_id =>
                            {
                                Some(call.instance.clone())
                            }
                            crate::journal::InboundExchangeRecord::Call(_)
                            | crate::journal::InboundExchangeRecord::Session(_) => None,
                        })
                }
                crate::journal::Record::Transaction(_) | crate::journal::Record::Egress(_) => None,
            })
            .expect("the accepted Call records its exact source instance");
        vm.run_ready_with_bindings(&mut journal, &mut bindings)
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
                crate::journal::Record::Ingress(_)
                | crate::journal::Record::Transaction(_)
                | crate::journal::Record::Egress(_) => None,
            })
            .expect("the response-producing transaction seals its route");
        assert_eq!(response_target, opening_instance);

        state.lock().unwrap().events.push(InboundCallObservation {
            call_id,
            completion: Completion::http_server_exit(HttpServerExit::Completed { status: 200 }),
        });
        let premature = bindings.poll_inbound_call_events(&mut vm).unwrap();
        assert_eq!(premature.queued, 0);
        assert_eq!(premature.rejected.len(), 1);

        let first = bindings.poll_inbound_call_responses(&mut vm).unwrap();
        assert_eq!((first.started, first.pending), (0, 1));
        assert!(state.lock().unwrap().responses.is_empty());
        let second = bindings.poll_inbound_call_responses(&mut vm).unwrap();
        assert_eq!((second.started, second.pending), (1, 0));
        {
            let state = state.lock().unwrap();
            assert_eq!(state.responses.len(), 1);
            assert_eq!(state.responses[0].call_id, call_id);
            assert_eq!(
                state.responses[0].response.data(),
                &Data::materialized(Value::Text(String::from("response")))
            );
        }
        let report = bindings.poll_inbound_call_events(&mut vm).unwrap();
        assert_eq!(report.queued, 1);
        assert!(report.rejected.is_empty());
        vm.run_ready_with_bindings(&mut journal, &mut bindings)
            .expect("the observed protocol completion closes the original Call");

        let terminals = journal
            .records()
            .iter()
            .filter_map(|record| match record {
                crate::journal::Record::Transaction(transaction)
                    if transaction.exchange_exit.as_deref()
                        == Some(&crate::journal::InboundExchangeExitRecord::Call(call_id)) =>
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
                .all(|record| !matches!(record, crate::journal::Record::Egress(_))),
            "respond is the return half of the inbound Call, never a new outbound Call"
        );
    }

    #[test]
    fn a_response_after_the_inbound_call_closed_is_discarded_and_journaled() {
        let plan = inbound_call_response_plan();
        let state = Arc::new(Mutex::new(RetainedCallState {
            proposals: VecDeque::from([IngressProposal {
                reference: IngressProposalHandle::new(1),
                submission: IngressSubmission {
                    provenance: IngressEnvelopeProvenance::single(
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
                id: ContractId::parse("org.rastok.test/inbound-call@1").unwrap(),
                state: Arc::clone(&state),
            }))
            .unwrap();
        let mut bindings = RuntimeBindings::try_new(&plan, &registry).unwrap();
        let mut vm = Vm::boot(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));

        let Some(crate::boundary::IngressAdmission::Accepted(accepted)) = bindings
            .poll_inbound(&mut vm, &mut journal)
            .expect("the request reaches atomic admission")
        else {
            panic!("the request must be accepted")
        };
        let Some(InboundExchangeId::Call(call_id)) = accepted.exchange else {
            panic!("the accepted request opens one inbound Call")
        };
        assert_eq!(
            bindings.poll_inbound_call_events(&mut vm).unwrap().queued,
            1
        );
        vm.run_ready_with_bindings(&mut journal, &mut bindings)
            .expect("the accepted terminal and late graph response settle deterministically");

        let response_report = bindings.poll_inbound_call_responses(&mut vm).unwrap();
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
                            == Some(&crate::journal::InboundExchangeExitRecord::Call(call_id))
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
                id: ContractId::parse("org.rastok.test/event-source@1").unwrap(),
                state: Arc::clone(&state),
            }))
            .unwrap();
        let mut bindings = RuntimeBindings::try_new(&plan, &registry).unwrap();
        let mut vm = Vm::boot(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));

        for expected in 0..1024 {
            let Some(crate::boundary::IngressAdmission::Accepted(accepted)) = bindings
                .poll_inbound(&mut vm, &mut journal)
                .expect("the bounded intake stays healthy")
            else {
                panic!("proposal {expected} should fit")
            };
            let state = state.lock().unwrap();
            assert_eq!(state.decisions.len(), expected as usize + 1);
            assert_eq!(state.decisions.last().unwrap().0.get(), expected);
            assert!(matches!(
                state.decisions.last().unwrap().1,
                IngressDecision::Accepted(observed) if observed == accepted
            ));
            assert!(journal.find_envelope(accepted.envelope).is_some());
        }
        let connector_roots = journal
            .records()
            .iter()
            .filter_map(|record| match record {
                crate::journal::Record::Ingress(inbound) => inbound.connector_path.as_ref(),
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
            bindings.poll_inbound(&mut vm, &mut journal).unwrap(),
            Some(crate::boundary::IngressAdmission::Pending)
        );
        assert_eq!(journal.records().len(), records_before);
        assert_eq!(state.lock().unwrap().decisions.len(), 1024);
        let retained = bindings.next_inbound().unwrap().unwrap();
        assert_eq!(retained.proposal().reference.get(), 1024);
        assert_eq!(
            bindings.poll_inbound(&mut vm, &mut journal).unwrap(),
            Some(crate::boundary::IngressAdmission::Pending),
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
                id: ContractId::parse("org.rastok.test/event-source@1").unwrap(),
                state: Arc::clone(&state),
            }))
            .unwrap();
        let mut bindings = RuntimeBindings::try_new(&plan, &registry).unwrap();
        let mut vm = Vm::boot(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        assert_eq!(
            bindings.poll_inbound(&mut vm, &mut journal).unwrap(),
            Some(crate::boundary::IngressAdmission::Rejected(
                crate::boundary::IngressRejection::UnknownPort
            ))
        );
        assert!(journal.records().is_empty());
        assert_eq!(
            state.lock().unwrap().decisions,
            [(
                IngressProposalHandle::new(7),
                IngressDecision::Rejected(crate::boundary::IngressRejection::UnknownPort)
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
                id: ContractId::parse("org.rastok.test/event-source@1").unwrap(),
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
            IngressEnvelopeProvenance::single(node::Path::new(["other"]).unwrap());
        let state = Arc::new(Mutex::new(InboundQueueState {
            proposals: VecDeque::from([proposal]),
            decisions: Vec::new(),
        }));
        let registry = DriverRegistry::default()
            .try_with(Box::new(RetainedInboundFactory {
                id: ContractId::parse("org.rastok.test/event-source@1").unwrap(),
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
                id: ContractId::parse("org.rastok.test/event-source@1").unwrap(),
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
            id: "org.rastok.test/event-source@1",
            name: "EventSource",
            roles: ConnectorDirection::Inbound,
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
                "org.rastok.test/alpha@1",
                ConnectorDirection::Outbound,
                Some(ConnectorDirection::Outbound),
                &starts,
            ))
            .unwrap()
            .try_with(routing_factory(
                "org.rastok.test/beta@1",
                ConnectorDirection::Outbound,
                Some(ConnectorDirection::Outbound),
                &starts,
            ))
            .unwrap();
        let mut bindings = RuntimeBindings::try_new(&plan, &registry).unwrap();
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&plan, GraphVersion(1));
        vm.inject(
            &mut journal,
            IngressSubmission {
                provenance: IngressEnvelopeProvenance::single(node::Path::new(["test"]).unwrap()),
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
            vm.run_ready_with_bindings(&mut journal, &mut bindings)
                .unwrap();
            vm.poll_drivers(&mut journal, &mut bindings).unwrap();
        }
        vm.run_ready_with_bindings(&mut journal, &mut bindings)
            .unwrap();

        assert_eq!(
            *starts.lock().unwrap(),
            [
                String::from("org.rastok.test/alpha@1:first"),
                String::from("org.rastok.test/beta@1:second"),
            ]
        );
        assert_eq!(
            vm.outputs()[0].envelope.data(),
            &Data::materialized(Value::Text(String::from("org.rastok.test/beta@1")))
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
                "org.rastok.test/alpha@1",
                ConnectorDirection::Inbound,
                Some(ConnectorDirection::Inbound),
                &starts,
            ))
            .unwrap();
        assert!(matches!(
            RuntimeBindings::try_new(&plan, &wrong),
            Err(DriverBindingError::MissingRoles { .. })
        ));

        let extra = DriverRegistry::default()
            .try_with(routing_factory(
                "org.rastok.test/alpha@1",
                ConnectorDirection::Outbound,
                Some(ConnectorDirection::InboundAndOutbound),
                &starts,
            ))
            .unwrap()
            .try_with(routing_factory(
                "org.rastok.test/beta@1",
                ConnectorDirection::Outbound,
                Some(ConnectorDirection::Outbound),
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
