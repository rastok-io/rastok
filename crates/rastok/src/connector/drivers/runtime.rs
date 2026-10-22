//! Plan-to-Driver associations and ownership of Calls, ingress proposals and Sessions.
//!
//! Every exchange continues through the exact Driver object selected during preparation.
//! The registry supplies implementations; this module owns their execution state.

use crate::connector::ConnectorDirection;
use crate::connector::ContractId;
use crate::connector::ExchangeKind;
use crate::connector::driver::Driver;
use crate::connector::driver::DriverRouteError;
use crate::connector::driver::Ingress;
use crate::connector::driver::OutboundCall;
use crate::connector::driver::OutboundDrivers;
use crate::connector::driver::Session;
use crate::connector::exchange::AdmittedIngress;
use crate::connector::exchange::CallId;
use crate::connector::exchange::CallOpen;
use crate::connector::exchange::CallResponse;
use crate::connector::exchange::CallStart;
use crate::connector::exchange::Completion;
use crate::connector::exchange::InboundCallResponseStart;
use crate::connector::exchange::InboundCallTimeoutStart;
use crate::connector::exchange::InboundExchangeId;
use crate::connector::exchange::IngressRejection;
use crate::connector::exchange::PendingIngress;
use crate::connector::exchange::RequestPreparation;
use crate::connector::exchange::SchedulerCompletion;
use crate::connector::exchange::SessionId;
use crate::connector::exchange::SessionObservation;
use crate::connector::exchange::SessionOpen;
use crate::connector::exchange::SessionWrite;
use crate::envelope::{Data, Envelope};
use crate::node;
use crate::plan::ValidatedExecutionPlan;
use crate::runtime::execution::{Execution, ExecutionFault, InboundCallPollReport};
use crate::{connector::exchange::IngressAdmission, journal::Journal};
use std::collections::{BTreeMap, BTreeSet};

use super::DriverName;
use super::registry::{DriverBindingError, DriverFactory, DriverRegistry};

struct DriverRequirement<'registry> {
    factory: &'registry dyn DriverFactory,
    nodes: Vec<(node::Path, ConnectorDirection)>,
    required: ConnectorDirection,
}

struct BoundDriver<'plan> {
    contract: ContractId,
    name: DriverName,
    driver: Box<dyn Driver + 'plan>,
    uses_inbound: bool,
    inbound_paths: BTreeSet<node::Path>,
    exchange: ExchangeKind,
    pending_inbound: Option<PendingIngress>,
    outbound_path: Option<node::Path>,
    session_path: Option<node::Path>,
}

/// One retained proposal paired with the exact Driver object that owns it.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutedInboundProposal {
    owner: usize,
    connector: ContractId,
    proposal: PendingIngress,
}

/// Failure of one inbound Driver polling pass through Execution admission.
#[derive(Debug, Clone, PartialEq)]
pub enum InboundPollError {
    /// The bound Driver registry contradicted its retained routing state.
    Driver(DriverRouteError),
    /// The Execution or journal raised a fail-stop integrity fault.
    Machine(ExecutionFault),
}

/// One bounded pass offering already-journaled graph responses to inbound
/// Call Drivers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InboundCallResponsePollReport {
    /// Responses whose protocol write is now Driver-owned.
    pub started: usize,
    /// Responses retained by the Execution because the Driver's finite capacity is
    /// currently full.
    pub pending: usize,
}

/// One bounded pass transferring scheduler-owned inbound request expiries to
/// their exact Driver object.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InboundCallTimeoutPollReport {
    /// Timeout attempts whose protocol action is now Driver-owned.
    pub started: usize,
    /// Timeout attempts retained by the Execution under finite Driver pressure.
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
    /// Contract of the proposing implementation; callback ownership is held separately.
    #[must_use]
    pub fn connector(&self) -> &ContractId {
        &self.connector
    }

    /// Retained proposal presented for atomic Execution admission.
    #[must_use]
    pub fn proposal(&self) -> &PendingIngress {
        &self.proposal
    }
}

/// Complete runtime association between active plan Connectors and physical
/// Drivers.
///
/// Construction is the only registry lookup. Thereafter the Execution routes by
/// exact Node path and role; no concrete contract name or registration order
/// participates in execution.
pub struct RuntimeDrivers<'plan> {
    drivers: Vec<BoundDriver<'plan>>,
    // Last object polled; rotating from it preserves fairness under backpressure.
    inbound_cursor: Option<usize>,
    outbound: BTreeMap<node::Path, usize>,
    calls: BTreeMap<CallId, usize>,
    inbound_calls: BTreeMap<crate::connector::exchange::InboundCallId, usize>,
    session_outbound: BTreeMap<node::Path, usize>,
    sessions: BTreeMap<SessionId, usize>,
}

impl core::fmt::Debug for RuntimeDrivers<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RuntimeDrivers")
            .field(
                "drivers",
                &self
                    .drivers
                    .iter()
                    .map(|bound| (&bound.contract, &bound.name))
                    .collect::<Vec<_>>(),
            )
            .field("inbound_cursor", &self.inbound_cursor)
            .field("outbound", &self.outbound)
            .field("calls", &self.calls)
            .field("inbound_calls", &self.inbound_calls)
            .field("session_outbound", &self.session_outbound)
            .field("sessions", &self.sessions)
            .finish()
    }
}

impl<'plan> RuntimeDrivers<'plan> {
    /// Whether this statically bound image contains at least one external
    /// source that must be polled even when no CLI submission is pending.
    pub fn has_runnable_inbound(&self) -> bool {
        self.drivers.iter().any(|bound| bound.uses_inbound)
    }

    /// Binds every runnable inbound Connector and every callable outbound
    /// Connector before an Execution or journal exists.
    ///
    /// An outbound-only Node with no incoming edge cannot open a call and is
    /// deliberately skipped. An inbound-capable Node is runnable by role and
    /// therefore binds even when no graph edge targets it.
    ///
    /// # Errors
    ///
    /// Missing, role-incomplete, lying, or factory-refused implementations
    /// remain distinct typed failures.
    pub fn prepare(
        plan: &'plan ValidatedExecutionPlan,
        registry: &DriverRegistry,
    ) -> Result<Self, DriverBindingError> {
        Self::prepare_with_boot(plan, registry, &crate::deployment::BootBindings::empty())
    }

    /// Binds with one complete deployment input set before an Execution or journal
    /// exists. Secret material can cross only a factory override that owns a
    /// contract-declared secret-compatible setting.
    pub fn prepare_with_boot(
        plan: &'plan ValidatedExecutionPlan,
        registry: &DriverRegistry,
        boot: &crate::deployment::BootBindings,
    ) -> Result<Self, DriverBindingError> {
        Self::prepare_with_selection(plan, registry, boot, &BTreeMap::new())
    }

    /// Resolves every Node's implementation before constructing any Driver.
    /// Selection affects deployment only; no implementation name enters the plan.
    pub fn prepare_with_selection(
        plan: &'plan ValidatedExecutionPlan,
        registry: &DriverRegistry,
        boot: &crate::deployment::BootBindings,
        selection: &BTreeMap<node::Path, DriverName>,
    ) -> Result<Self, DriverBindingError> {
        for path in selection.keys() {
            if !plan.plan().nodes.iter().any(|planned| {
                planned.executor_path() == Some(path) && planned.connector().is_some()
            }) {
                return Err(DriverBindingError::UnknownConnector(path.clone()));
            }
        }

        let mut requirements: BTreeMap<(ContractId, DriverName), DriverRequirement<'_>> =
            BTreeMap::new();
        for (index, planned) in plan.plan().nodes.iter().enumerate() {
            let Some(spec) = planned.connector() else {
                continue;
            };
            let Some(path) = planned.executor_path() else {
                continue;
            };
            let callable = plan.plan().edges().iter().any(|edge| {
                edge.to
                    .as_ref()
                    .is_some_and(|endpoint| endpoint.node.index() == index)
            });
            let runnable = spec.roles.has_inbound() || (spec.roles.has_outbound() && callable);
            if !runnable && !selection.contains_key(path) {
                continue;
            }

            let mode = match registry.mode_attribute(&spec.contract) {
                Some(attribute) => match spec.attribute(attribute) {
                    Some(crate::plan::AttrSetting::Word(word)) => Some(word.as_str()),
                    _ => return Err(DriverBindingError::InvalidMode(path.clone())),
                },
                None => None,
            };
            let (name, factory) = registry.require(
                &spec.contract,
                spec.roles,
                spec.exchange,
                mode,
                selection.get(path),
            )?;
            if !runnable {
                continue;
            }
            requirements
                .entry((spec.contract.clone(), name.clone()))
                .and_modify(|requirement| {
                    requirement.required = union_roles(requirement.required, spec.roles);
                    requirement.nodes.push((path.clone(), spec.roles));
                })
                .or_insert_with(|| DriverRequirement {
                    factory,
                    nodes: vec![(path.clone(), spec.roles)],
                    required: spec.roles,
                });
        }

        let mut drivers = Vec::new();
        let mut outbound = BTreeMap::new();
        let mut session_outbound = BTreeMap::new();
        for ((id, name), requirement) in requirements {
            let paths: Vec<_> = requirement
                .nodes
                .iter()
                .map(|(path, _)| path.clone())
                .collect();
            let factory = requirement.factory;
            let exchange = factory.exchange();
            let mut driver = factory
                .create_with_boot(plan, &paths, boot)
                .map_err(|error| DriverBindingError::Factory {
                    id: id.clone(),
                    name: name.clone(),
                    detail: error.detail().to_string(),
                })?;
            let actual = roles_of(driver.as_mut());
            let advertised = factory.roles();
            if actual != Some(advertised) {
                return Err(DriverBindingError::BoundRolesMismatch {
                    id,
                    name,
                    advertised,
                    actual,
                });
            }
            if requirement.required.has_outbound() {
                let actual = outbound_exchange_of(driver.as_mut());
                if actual != Some(exchange) {
                    return Err(DriverBindingError::BoundExchangeMismatch {
                        id,
                        name,
                        required: exchange,
                        actual,
                    });
                }
            }
            if requirement.required.has_inbound()
                && exchange == ExchangeKind::Call
                && driver.inbound().and_then(Ingress::call).is_none()
            {
                return Err(DriverBindingError::BoundExchangeMismatch {
                    id,
                    name,
                    required: exchange,
                    actual: None,
                });
            }

            let owner = drivers.len();
            let uses_inbound = requirement.required.has_inbound();
            let inbound_paths = requirement
                .nodes
                .iter()
                .filter_map(|(path, roles)| roles.has_inbound().then_some(path.clone()))
                .collect();
            let mut outbound_path = None;
            let mut session_path = None;
            for (path, roles) in requirement.nodes {
                if roles.has_outbound() && exchange == ExchangeKind::Call {
                    outbound_path.get_or_insert_with(|| path.clone());
                    outbound.insert(path, owner);
                } else if roles.has_outbound() && exchange == ExchangeKind::Session {
                    session_path.get_or_insert_with(|| path.clone());
                    session_outbound.insert(path, owner);
                }
            }
            drivers.push(BoundDriver {
                contract: id,
                name,
                driver,
                uses_inbound,
                inbound_paths,
                exchange,
                pending_inbound: None,
                outbound_path,
                session_path,
            });
        }
        Ok(Self {
            drivers,
            inbound_cursor: None,
            outbound,
            session_outbound,
            calls: BTreeMap::new(),
            inbound_calls: BTreeMap::new(),
            sessions: BTreeMap::new(),
        })
    }

    /// Polls runnable inbound Driver objects in deterministic Contract/name order.
    ///
    /// # Errors
    ///
    /// A role view proven during binding disappeared.
    pub fn next_inbound(&mut self) -> Result<Option<RoutedInboundProposal>, DriverRouteError> {
        let count = self.drivers.len();
        let start = self
            .inbound_cursor
            .map_or(0, |owner| (owner + 1) % count.max(1));
        for offset in 0..count {
            let owner = (start + offset) % count;
            let Some(bound) = self.drivers.get_mut(owner) else {
                continue;
            };
            if !bound.uses_inbound {
                continue;
            }
            if let Some(proposal) = &bound.pending_inbound {
                self.inbound_cursor = Some(owner);
                return Ok(Some(RoutedInboundProposal {
                    owner,
                    connector: bound.contract.clone(),
                    proposal: proposal.clone(),
                }));
            }
            let driver = bound
                .driver
                .inbound()
                .ok_or_else(|| DriverRouteError::UnavailableInbound(bound.contract.clone()))?;
            if let Some(proposal) = driver.poll_proposal() {
                let source = proposal.proposal.provenance.connector_path();
                if !bound.inbound_paths.contains(source) {
                    return Err(DriverRouteError::InboundSourceMismatch {
                        connector: bound.contract.clone(),
                        source: source.clone(),
                    });
                }
                let framing_matches = match bound.exchange {
                    ExchangeKind::Event | ExchangeKind::Call => proposal.proposal.session.is_none(),
                    ExchangeKind::Session => proposal.proposal.session.is_some(),
                };
                if !framing_matches {
                    return Err(DriverRouteError::InboundExchangeMismatch {
                        connector: bound.contract.clone(),
                        exchange: bound.exchange,
                    });
                }
                bound.pending_inbound = Some(proposal.clone());
                self.inbound_cursor = Some(owner);
                return Ok(Some(RoutedInboundProposal {
                    owner,
                    connector: bound.contract.clone(),
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
    /// Refuses a stale, crossed-object, or otherwise unknown proposal.
    pub fn notify_inbound_admission(
        &mut self,
        routed: &RoutedInboundProposal,
        decision: Result<AdmittedIngress, IngressRejection>,
    ) -> Result<(), DriverRouteError> {
        let unknown = || DriverRouteError::UnknownIngressProposal {
            connector: routed.connector.clone(),
            reference: routed.proposal.handle,
        };
        let bound = self.drivers.get(routed.owner).ok_or_else(unknown)?;
        if bound.contract != routed.connector
            || bound.pending_inbound.as_ref() != Some(&routed.proposal)
        {
            return Err(unknown());
        }
        if let Ok(accepted) = decision {
            match accepted.exchange {
                Some(InboundExchangeId::Call(call_id)) => {
                    if let Some(&expected) = self.inbound_calls.get(&call_id)
                        && expected != routed.owner
                    {
                        return Err(DriverRouteError::InboundCallOwnerMismatch {
                            call_id,
                            expected: self.owner_description(expected),
                            actual: self.owner_description(routed.owner),
                        });
                    }
                }
                Some(InboundExchangeId::Session(session)) => {
                    if let Some(&expected) = self.sessions.get(&session)
                        && expected != routed.owner
                    {
                        return Err(DriverRouteError::SessionOwnerMismatch {
                            session,
                            expected: self.owner_description(expected),
                            actual: self.owner_description(routed.owner),
                        });
                    }
                }
                None => {}
            }
        }
        let bound = self.drivers.get_mut(routed.owner).ok_or_else(unknown)?;
        let driver = bound
            .driver
            .inbound()
            .ok_or_else(|| DriverRouteError::UnavailableInbound(routed.connector.clone()))?;
        if let Ok(accepted) = decision {
            match accepted.exchange {
                Some(InboundExchangeId::Call(call)) => {
                    self.inbound_calls.insert(call, routed.owner);
                }
                Some(InboundExchangeId::Session(session)) => {
                    self.sessions.insert(session, routed.owner);
                }
                None => {}
            }
        }
        driver.notify_admission(routed.proposal.handle, decision);
        bound.pending_inbound = None;
        Ok(())
    }

    /// Runs one bounded inbound proposal through Execution admission.
    ///
    /// Acceptance reaches the Driver only after the journal commit. Pending
    /// admission retains both the proposal and Driver-side transport unit;
    /// rejection releases it with a portable typed reason.
    ///
    /// # Errors
    ///
    /// Driver routing contradictions and fail-stop Execution/journal faults remain
    /// distinct.
    pub fn poll_inbound(
        &mut self,
        execution: &mut Execution<'plan>,
        journal: &mut dyn Journal,
    ) -> Result<Option<IngressAdmission>, InboundPollError> {
        let Some(routed) = self.next_inbound().map_err(InboundPollError::Driver)? else {
            return Ok(None);
        };
        let admission = execution
            .accept_connector_output(journal, routed.proposal())
            .map_err(InboundPollError::Machine)?;
        match admission {
            IngressAdmission::Accepted(accepted) => self
                .notify_inbound_admission(&routed, Ok(accepted))
                .map_err(InboundPollError::Driver)?,
            IngressAdmission::Rejected(reason) => self
                .notify_inbound_admission(&routed, Err(reason))
                .map_err(InboundPollError::Driver)?,
            IngressAdmission::Pending => {}
        }
        Ok(Some(admission))
    }

    /// Offers every ready graph response to the Driver object that admitted
    /// its inbound Call. The response stays retained and its Connector
    /// mailbox stays claimed while a bounded Driver reports `Pending`.
    ///
    /// # Errors
    ///
    /// A missing/crossed ownership or a Call view that disappeared after
    /// binding is an integrity fault.
    pub fn poll_inbound_call_responses(
        &mut self,
        execution: &mut Execution<'plan>,
    ) -> Result<InboundCallResponsePollReport, DriverRouteError> {
        let mut report = InboundCallResponsePollReport::default();
        for response in execution.ready_inbound_call_responses() {
            let Some(id) = self.inbound_calls.get(&response.call_id).copied() else {
                return Err(DriverRouteError::UncorrelatedInboundCall(response.call_id));
            };
            let Some(family) = self.drivers.get_mut(id) else {
                return Err(DriverRouteError::UncorrelatedInboundCall(response.call_id));
            };
            if !family.inbound_paths.contains(&response.connector_path) {
                return Err(DriverRouteError::InboundSourceMismatch {
                    connector: family.contract.clone(),
                    source: response.connector_path,
                });
            }
            let driver = family
                .driver
                .inbound()
                .ok_or_else(|| DriverRouteError::UnavailableInbound(family.contract.clone()))?
                .call()
                .ok_or_else(|| DriverRouteError::UnavailableInboundCall(family.contract.clone()))?;
            match driver.start_response(&response) {
                InboundCallResponseStart::Started => {
                    if !execution.mark_inbound_call_response_started(&response) {
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

    /// Offers every due inbound request deadline to the Driver object that
    /// admitted the Call. A `Pending` attempt remains scheduler-owned and is
    /// offered again without creating another queue or terminal fact.
    ///
    /// # Errors
    ///
    /// Missing or crossed ownership is a binding-integrity fault.
    pub fn poll_inbound_call_timeouts(
        &mut self,
        execution: &mut Execution<'plan>,
    ) -> Result<InboundCallTimeoutPollReport, DriverRouteError> {
        let mut report = InboundCallTimeoutPollReport::default();
        for timeout in execution.ready_inbound_call_timeouts() {
            let Some(id) = self.inbound_calls.get(&timeout.call_id).copied() else {
                return Err(DriverRouteError::UncorrelatedInboundCall(timeout.call_id));
            };
            let Some(family) = self.drivers.get_mut(id) else {
                return Err(DriverRouteError::UncorrelatedInboundCall(timeout.call_id));
            };
            if !family.inbound_paths.contains(&timeout.connector_path) {
                return Err(DriverRouteError::InboundSourceMismatch {
                    connector: family.contract.clone(),
                    source: timeout.connector_path,
                });
            }
            let driver = family
                .driver
                .inbound()
                .ok_or_else(|| DriverRouteError::UnavailableInbound(family.contract.clone()))?
                .call()
                .ok_or_else(|| DriverRouteError::UnavailableInboundCall(family.contract.clone()))?;
            match driver.start_timeout(&timeout) {
                InboundCallTimeoutStart::Started => {
                    if !execution.mark_inbound_call_timeout_started(&timeout) {
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

    /// Drains terminal observations from every inbound Call Driver and
    /// offers them to the Execution's typed, close-once queue. Object ownership is
    /// checked before the Execution sees an event; invalid facts remain non-causal.
    ///
    /// # Errors
    ///
    /// A bound Call view disappeared or a Driver reported another object's
    /// Call reference.
    pub fn poll_inbound_call_events(
        &mut self,
        execution: &mut Execution<'plan>,
    ) -> Result<InboundCallPollReport, DriverRouteError> {
        let mut report = InboundCallPollReport::default();
        for owner in 0..self.drivers.len() {
            let events = {
                let Some(bound) = self.drivers.get_mut(owner) else {
                    continue;
                };
                if !bound.uses_inbound || bound.exchange != ExchangeKind::Call {
                    continue;
                }
                let driver = bound
                    .driver
                    .inbound()
                    .ok_or_else(|| DriverRouteError::UnavailableInbound(bound.contract.clone()))?
                    .call()
                    .ok_or_else(|| {
                        DriverRouteError::UnavailableInboundCall(bound.contract.clone())
                    })?;
                driver.poll_completions()
            };
            for event in events {
                let Some(expected) = self.inbound_calls.get(&event.call_id).copied() else {
                    report.uncorrelated.push(event);
                    continue;
                };
                if expected != owner {
                    return Err(DriverRouteError::InboundCallOwnerMismatch {
                        call_id: event.call_id,
                        expected: self.owner_description(expected),
                        actual: self.owner_description(owner),
                    });
                }
                let call_id = event.call_id;
                let mut observed = execution.queue_inbound_call_events(core::iter::once(event));
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

    fn owner_description(&self, owner: usize) -> String {
        self.drivers.get(owner).map_or_else(
            || String::from("unbound Driver"),
            |bound| format!("{} ({})", bound.name, bound.contract),
        )
    }

    fn outbound_driver(
        &mut self,
        connector: &node::Path,
    ) -> Result<&mut dyn OutboundCall, DriverRouteError> {
        let Some(id) = self.outbound.get(connector) else {
            return Err(DriverRouteError::UnboundOutbound(connector.clone()));
        };
        let Some(family) = self.drivers.get_mut(*id) else {
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
    ) -> Result<(usize, &mut dyn Session), DriverRouteError> {
        let id = self.session_owner(connector)?;
        let Some(family) = self.drivers.get_mut(id) else {
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

    fn session_owner(&self, connector: &node::Path) -> Result<usize, DriverRouteError> {
        self.session_outbound
            .get(connector)
            .copied()
            .ok_or_else(|| DriverRouteError::UnboundSession(connector.clone()))
    }
}

impl OutboundDrivers for RuntimeDrivers<'_> {
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
        let actual = self
            .outbound
            .get(connector)
            .copied()
            .ok_or_else(|| DriverRouteError::UnboundOutbound(connector.clone()))?;
        if let Some(&expected) = self.calls.get(call_id)
            && expected != actual
        {
            return Err(DriverRouteError::CallOwnerMismatch {
                call_id: *call_id,
                expected: self.owner_description(expected),
                actual: self.owner_description(actual),
            });
        }
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
        let Some(id) = self.outbound.get(connector).copied() else {
            return Err(DriverRouteError::UnboundOutbound(connector.clone()));
        };
        if let Some(&expected) = self.calls.get(&intent.call_id)
            && expected != id
        {
            return Err(DriverRouteError::CallOwnerMismatch {
                call_id: intent.call_id,
                expected: self.owner_description(expected),
                actual: self.owner_description(id),
            });
        }
        let start = self.outbound_driver(connector)?.start(intent, request);
        if start == CallStart::Started {
            self.calls.insert(intent.call_id, id);
        }
        Ok(start)
    }

    fn poll_responses(&mut self) -> Result<Vec<CallResponse>, DriverRouteError> {
        let mut responses = Vec::new();
        for owner in 0..self.drivers.len() {
            let batch = {
                let Some(bound) = self.drivers.get_mut(owner) else {
                    continue;
                };
                let Some(path) = &bound.outbound_path else {
                    continue;
                };
                let driver = bound
                    .driver
                    .outbound()
                    .ok_or_else(|| DriverRouteError::UnavailableOutbound(path.clone()))?
                    .call()
                    .ok_or_else(|| DriverRouteError::UnavailableCall(path.clone()))?;
                driver.poll_responses()
            };
            for response in batch {
                if let Some(&expected) = self.calls.get(&response.call_id)
                    && expected != owner
                {
                    return Err(DriverRouteError::CallOwnerMismatch {
                        call_id: response.call_id,
                        expected: self.owner_description(expected),
                        actual: self.owner_description(owner),
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
    ) -> Result<crate::connector::exchange::SessionStart, DriverRouteError> {
        let actual = self.session_owner(&intent.connector_path)?;
        if let Some(&expected) = self.sessions.get(&intent.session_id)
            && expected != actual
        {
            return Err(DriverRouteError::SessionOwnerMismatch {
                session: intent.session_id,
                expected: self.owner_description(expected),
                actual: self.owner_description(actual),
            });
        }
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
        completion: crate::connector::exchange::SchedulerCompletion,
    ) -> Result<crate::connector::exchange::Completion, DriverRouteError> {
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
        let actual = self.session_owner(connector)?;
        if let Some(expected) = self.sessions.get(&session)
            && expected != &actual
        {
            return Err(DriverRouteError::SessionOwnerMismatch {
                session,
                expected: self.owner_description(*expected),
                actual: self.owner_description(actual),
            });
        }
        let (_, driver) = self.session_driver(connector)?;
        Ok(driver.prepare_session_write(connector, session, data, resolved_settings))
    }

    fn write_session(
        &mut self,
        intent: &SessionWrite,
        message: &Envelope,
    ) -> Result<crate::connector::exchange::SessionStart, DriverRouteError> {
        let actual = self.session_owner(&intent.connector_path)?;
        let Some(expected) = self.sessions.get(&intent.session_id) else {
            return Err(DriverRouteError::UncorrelatedSession(intent.session_id));
        };
        if expected != &actual {
            return Err(DriverRouteError::SessionOwnerMismatch {
                session: intent.session_id,
                expected: self.owner_description(*expected),
                actual: self.owner_description(actual),
            });
        }
        let (_, driver) = self.session_driver(&intent.connector_path)?;
        Ok(driver.write_session(intent, message))
    }

    fn close_session(
        &mut self,
        connector: &node::Path,
        session: SessionId,
    ) -> Result<crate::connector::exchange::SessionStart, DriverRouteError> {
        let actual = self.session_owner(connector)?;
        let Some(expected) = self.sessions.get(&session) else {
            return Err(DriverRouteError::UncorrelatedSession(session));
        };
        if expected != &actual {
            return Err(DriverRouteError::SessionOwnerMismatch {
                session,
                expected: self.owner_description(*expected),
                actual: self.owner_description(actual),
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
        let actual = self.session_owner(connector)?;
        let Some(expected) = self.sessions.get(&session) else {
            return Err(DriverRouteError::UncorrelatedSession(session));
        };
        if expected != &actual {
            return Err(DriverRouteError::SessionOwnerMismatch {
                session,
                expected: self.owner_description(*expected),
                actual: self.owner_description(actual),
            });
        }
        let (_, driver) = self.session_driver(connector)?;
        driver.abort_session(session);
        Ok(())
    }

    fn poll_observations(&mut self) -> Result<Vec<SessionObservation>, DriverRouteError> {
        let mut events = Vec::new();
        for owner in 0..self.drivers.len() {
            let batch = {
                let Some(bound) = self.drivers.get_mut(owner) else {
                    continue;
                };
                let Some(path) = &bound.session_path else {
                    continue;
                };
                let driver = bound
                    .driver
                    .outbound()
                    .ok_or_else(|| DriverRouteError::UnavailableOutbound(path.clone()))?
                    .session()
                    .ok_or_else(|| DriverRouteError::UnavailableSession(path.clone()))?;
                driver.poll_observations()
            };
            for event in batch {
                let session = match &event {
                    SessionObservation::Opened(session)
                    | SessionObservation::WriteSettled { session, .. }
                    | SessionObservation::Exited { session, .. } => *session,
                };
                let Some(&expected) = self.sessions.get(&session) else {
                    return Err(DriverRouteError::UncorrelatedSession(session));
                };
                if expected != owner {
                    return Err(DriverRouteError::SessionOwnerMismatch {
                        session,
                        expected: self.owner_description(expected),
                        actual: self.owner_description(owner),
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
            .drivers
            .get_mut(*id)
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

/// Ownership reconstruction failed without replacing any routing state.
#[derive(Debug, Clone, PartialEq)]
pub enum DriverRecoveryError {
    /// The receiving association already owns work or a pending ingress proposal.
    Active,
    /// The journal could not provide the complete prefix.
    Journal(crate::journal::Error),
    /// An exchange record omits its Connector source.
    MissingSource,
    /// The selected object cannot serve this Connector's exchange.
    UnboundConnector {
        /// Exact planned Connector path.
        connector: node::Path,
        /// Exchange required by the recorded obligation.
        exchange: ExchangeKind,
    },
    /// Two facts assign one logical exchange to different objects.
    Ownership,
}

impl core::fmt::Display for DriverRecoveryError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Active => {
                formatter.write_str("Driver ownership can only be restored before active work")
            }
            Self::Journal(error) => error.fmt(formatter),
            Self::MissingSource => {
                formatter.write_str("a recorded ingress exchange has no Connector source")
            }
            Self::UnboundConnector {
                connector,
                exchange,
            } => write!(
                formatter,
                "no selected Driver serves the recorded {exchange} at `{connector}`"
            ),
            Self::Ownership => {
                formatter.write_str("the recorded exchange crosses selected Driver ownership")
            }
        }
    }
}

impl std::error::Error for DriverRecoveryError {}

impl RuntimeDrivers<'_> {
    /// Restores exchange routing from a successfully recovered journal prefix.
    ///
    /// Call this after `Execution::recover_with_drivers`, with the same prefix
    /// and deployment selection, on a newly prepared association. It neither
    /// validates execution semantics nor restores sockets or processes. No
    /// Driver method is called; existing no-recall recovery rules still apply.
    /// Implementation names cannot be recovered from the journal: they are
    /// deployment configuration and must be supplied unchanged by the caller.
    ///
    /// # Errors
    /// Active ownership, a journal read failure, or a recorded exchange which
    /// cannot be associated with the selected objects. All maps remain unchanged.
    pub fn restore_ownership(&mut self, journal: &dyn Journal) -> Result<(), DriverRecoveryError> {
        use crate::journal::{
            CallStatus, EgressRecord, InboundExchangeExitRecord, InboundExchangeRecord, Record,
            SessionIngressRecord, SessionStatus,
        };

        if !self.calls.is_empty()
            || !self.inbound_calls.is_empty()
            || !self.sessions.is_empty()
            || self
                .drivers
                .iter()
                .any(|driver| driver.pending_inbound.is_some())
        {
            return Err(DriverRecoveryError::Active);
        }
        let mut calls = BTreeMap::new();
        let mut inbound_calls = BTreeMap::new();
        let mut sessions = BTreeMap::new();
        let mut result = Ok(());
        journal
            .visit_records(&mut |record| {
                if result.is_err() {
                    return;
                }
                result = (|| {
                    match record {
                        Record::Ingress(record) => {
                            let Some(exchange) = record.exchange.as_deref() else {
                                return Ok(());
                            };
                            let source = record
                                .connector_path
                                .as_ref()
                                .ok_or(DriverRecoveryError::MissingSource)?;
                            let kind = match exchange {
                                InboundExchangeRecord::Call(_) => ExchangeKind::Call,
                                InboundExchangeRecord::Session(_) => ExchangeKind::Session,
                            };
                            let owner = self
                                .drivers
                                .iter()
                                .position(|driver| {
                                    driver.exchange == kind && driver.inbound_paths.contains(source)
                                })
                                .ok_or_else(|| DriverRecoveryError::UnboundConnector {
                                    connector: source.clone(),
                                    exchange: kind,
                                })?;
                            match exchange {
                                InboundExchangeRecord::Call(call) => {
                                    if inbound_calls
                                        .insert(call.reference, owner)
                                        .is_some_and(|old| old != owner)
                                    {
                                        return Err(DriverRecoveryError::Ownership);
                                    }
                                }
                                InboundExchangeRecord::Session(SessionIngressRecord::Open(
                                    session,
                                )) => {
                                    if sessions
                                        .insert(*session, owner)
                                        .is_some_and(|old| old != owner)
                                    {
                                        return Err(DriverRecoveryError::Ownership);
                                    }
                                }
                                InboundExchangeRecord::Session(SessionIngressRecord::Message(
                                    session,
                                )) => {
                                    if sessions.get(session) != Some(&owner) {
                                        return Err(DriverRecoveryError::Ownership);
                                    }
                                }
                            }
                        }
                        Record::Egress(EgressRecord::Call(record)) => {
                            let owner = self
                                .outbound
                                .get(&record.intent.connector_path)
                                .copied()
                                .ok_or_else(|| DriverRecoveryError::UnboundConnector {
                                connector: record.intent.connector_path.clone(),
                                exchange: ExchangeKind::Call,
                            })?;
                            if calls
                                .get(&record.intent.call_id)
                                .is_some_and(|old| *old != owner)
                            {
                                return Err(DriverRecoveryError::Ownership);
                            }
                            match record.status {
                                CallStatus::Open => {
                                    calls.insert(record.intent.call_id, owner);
                                }
                                CallStatus::Closed(_) | CallStatus::Abandoned => {
                                    calls.remove(&record.intent.call_id);
                                }
                            }
                        }
                        Record::Egress(EgressRecord::Session(record)) => {
                            let owner = self
                                .session_outbound
                                .get(&record.connector_path)
                                .copied()
                                .ok_or_else(|| DriverRecoveryError::UnboundConnector {
                                    connector: record.connector_path.clone(),
                                    exchange: ExchangeKind::Session,
                                })?;
                            match &record.status {
                                SessionStatus::Opening(_) => {
                                    if sessions
                                        .insert(record.session_id, owner)
                                        .is_some_and(|old| old != owner)
                                    {
                                        return Err(DriverRecoveryError::Ownership);
                                    }
                                }
                                SessionStatus::Opened
                                | SessionStatus::WriteOpen(_)
                                | SessionStatus::WriteSettled(_)
                                | SessionStatus::WriteAbandoned(_) => {
                                    if sessions.get(&record.session_id) != Some(&owner) {
                                        return Err(DriverRecoveryError::Ownership);
                                    }
                                }
                            }
                        }
                        Record::Transaction(record) => match record.exchange_exit.as_deref() {
                            Some(InboundExchangeExitRecord::Call(call)) => {
                                inbound_calls.remove(call);
                            }
                            Some(InboundExchangeExitRecord::Session(session)) => {
                                sessions.remove(session);
                            }
                            None => {}
                        },
                    }
                    Ok(())
                })();
            })
            .map_err(DriverRecoveryError::Journal)?;
        result?;
        self.calls = calls;
        self.inbound_calls = inbound_calls;
        self.sessions = sessions;
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

fn roles_of(driver: &mut dyn Driver) -> Option<ConnectorDirection> {
    match (driver.inbound().is_some(), driver.outbound().is_some()) {
        (true, true) => Some(ConnectorDirection::InboundAndOutbound),
        (true, false) => Some(ConnectorDirection::Inbound),
        (false, true) => Some(ConnectorDirection::Outbound),
        (false, false) => None,
    }
}

fn outbound_exchange_of(driver: &mut dyn Driver) -> Option<ExchangeKind> {
    let outbound = driver.outbound()?;
    match (outbound.call().is_some(), outbound.session().is_some()) {
        (true, false) => Some(ExchangeKind::Call),
        (false, true) => Some(ExchangeKind::Session),
        (false, false) | (true, true) => None,
    }
}

#[cfg(test)]
mod tests;
