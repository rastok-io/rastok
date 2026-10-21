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
use crate::connector::driver::SessionConnectorBindings;
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

use super::registry::{DriverBindingError, DriverRegistry};

#[derive(Debug)]
struct FamilyRequirement {
    nodes: Vec<(node::Path, ConnectorDirection, ExchangeKind)>,
    required: ConnectorDirection,
}

struct BoundFamily<'plan> {
    driver: Box<dyn Driver + 'plan>,
    uses_inbound: bool,
    inbound_paths: BTreeSet<node::Path>,
    exchange: ExchangeKind,
    pending_inbound: Option<PendingIngress>,
    outbound_path: Option<node::Path>,
    session_path: Option<node::Path>,
}

/// One retained proposal paired with the exact Driver family that owns it.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutedInboundProposal {
    connector: ContractId,
    proposal: PendingIngress,
}

/// Failure of one inbound Driver/Execution boundary round.
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
/// their exact Driver family.
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
    /// Stable family that must receive the eventual decision callback.
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
    families: BTreeMap<ContractId, BoundFamily<'plan>>,
    /// Last inbound family polled. Cycling from this key prevents one
    /// backpressured transport from blocking an unrelated family.
    inbound_cursor: Option<ContractId>,
    outbound: BTreeMap<node::Path, ContractId>,
    calls: BTreeMap<CallId, ContractId>,
    inbound_calls: BTreeMap<crate::connector::exchange::InboundCallId, ContractId>,
    session_outbound: BTreeMap<node::Path, ContractId>,
    sessions: BTreeMap<SessionId, ContractId>,
}

impl core::fmt::Debug for RuntimeDrivers<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RuntimeDrivers")
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

impl<'plan> RuntimeDrivers<'plan> {
    /// Whether this statically bound image contains at least one external
    /// source that must be polled even when no CLI submission is pending.
    pub fn has_runnable_inbound(&self) -> bool {
        self.families.values().any(|family| family.uses_inbound)
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
        let mut requirements: BTreeMap<ContractId, FamilyRequirement> = BTreeMap::new();
        for (index, planned) in plan.plan().nodes.iter().enumerate() {
            let Some(spec) = planned.connector() else {
                continue;
            };
            let callable = plan.plan().edges().iter().any(|edge| {
                edge.to
                    .as_ref()
                    .map(|endpoint| endpoint.node)
                    .is_some_and(|slot| slot.index() == index)
            });
            if !spec.roles.has_inbound() && !(spec.roles.has_outbound() && callable) {
                continue;
            }
            let Some(path) = planned.executor_path() else {
                continue;
            };
            requirements
                .entry(spec.contract.clone())
                .and_modify(|requirement| {
                    requirement.required = union_roles(requirement.required, spec.roles);
                    requirement
                        .nodes
                        .push((path.clone(), spec.roles, spec.exchange));
                })
                .or_insert_with(|| FamilyRequirement {
                    nodes: vec![(path.clone(), spec.roles, spec.exchange)],
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
                .create_with_boot(plan, &paths, boot)
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
            if requirement.required.has_inbound() && exchange == ExchangeKind::Call {
                let has_call_view = driver.inbound().and_then(Ingress::call).is_some();
                if !has_call_view {
                    return Err(DriverBindingError::BoundExchangeMismatch {
                        id,
                        required: ExchangeKind::Call,
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
                if roles.has_outbound() && exchange == ExchangeKind::Call {
                    outbound_path.get_or_insert_with(|| path.clone());
                    outbound.insert(path, id.clone());
                } else if roles.has_outbound() && exchange == ExchangeKind::Session {
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
            if let Some(proposal) = driver.poll_proposal() {
                let source = proposal.proposal.provenance.connector_path();
                if !family.inbound_paths.contains(source) {
                    return Err(DriverRouteError::InboundSourceMismatch {
                        connector: id.clone(),
                        source: source.clone(),
                    });
                }
                let framing_matches = match family.exchange {
                    ExchangeKind::Event | ExchangeKind::Call => proposal.proposal.session.is_none(),
                    ExchangeKind::Session => proposal.proposal.session.is_some(),
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
        decision: Result<AdmittedIngress, IngressRejection>,
    ) -> Result<(), DriverRouteError> {
        let Some(family) = self.families.get_mut(&routed.connector) else {
            return Err(DriverRouteError::UnknownIngressProposal {
                connector: routed.connector.clone(),
                reference: routed.proposal.handle,
            });
        };
        let Some(pending) = &family.pending_inbound else {
            return Err(DriverRouteError::UnknownIngressProposal {
                connector: routed.connector.clone(),
                reference: routed.proposal.handle,
            });
        };
        if pending != &routed.proposal {
            return Err(DriverRouteError::UnknownIngressProposal {
                connector: routed.connector.clone(),
                reference: routed.proposal.handle,
            });
        }
        if let Ok(accepted) = decision {
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
        driver.notify_admission(routed.proposal.handle, decision);
        family.pending_inbound = None;
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
        vm: &mut Execution<'plan>,
        journal: &mut dyn Journal,
    ) -> Result<Option<IngressAdmission>, InboundPollError> {
        let Some(routed) = self.next_inbound().map_err(InboundPollError::Driver)? else {
            return Ok(None);
        };
        let admission = vm
            .accept_connector_output(journal, routed.proposal())
            .map_err(InboundPollError::Machine)?;
        match admission {
            IngressAdmission::Accepted(accepted) => self
                .decide_inbound(&routed, Ok(accepted))
                .map_err(InboundPollError::Driver)?,
            IngressAdmission::Rejected(reason) => self
                .decide_inbound(&routed, Err(reason))
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
        vm: &mut Execution<'plan>,
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
        vm: &mut Execution<'plan>,
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
    /// offers them to the Execution's typed, close-once queue. Family correlation is
    /// checked before the Execution sees an event; invalid facts remain non-causal.
    ///
    /// # Errors
    ///
    /// A bound Call view disappeared or a family reported another family's
    /// Call reference.
    pub fn poll_inbound_call_events(
        &mut self,
        vm: &mut Execution<'plan>,
    ) -> Result<InboundCallPollReport, DriverRouteError> {
        let ids: Vec<_> = self
            .families
            .iter()
            .filter_map(|(id, family)| {
                (family.uses_inbound && family.exchange == ExchangeKind::Call).then_some(id.clone())
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
                driver.poll_completions()
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
    ) -> Result<&mut dyn OutboundCall, DriverRouteError> {
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
    ) -> Result<(ContractId, &mut dyn Session), DriverRouteError> {
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

    fn poll_responses(&mut self) -> Result<Vec<CallResponse>, DriverRouteError> {
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
            for response in driver.poll_responses() {
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
    ) -> Result<crate::connector::exchange::SessionStart, DriverRouteError> {
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
    ) -> Result<crate::connector::exchange::SessionStart, DriverRouteError> {
        SessionConnectorBindings::write_session(self, intent, message)
    }

    fn close_session(
        &mut self,
        connector: &node::Path,
        session: SessionId,
    ) -> Result<crate::connector::exchange::SessionStart, DriverRouteError> {
        SessionConnectorBindings::close_session(self, connector, session)
    }

    fn poll_observations(&mut self) -> Result<Vec<SessionObservation>, DriverRouteError> {
        SessionConnectorBindings::poll_observations(self)
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

impl SessionConnectorBindings for RuntimeDrivers<'_> {
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
    ) -> Result<crate::connector::exchange::SessionStart, DriverRouteError> {
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
    ) -> Result<crate::connector::exchange::SessionStart, DriverRouteError> {
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

    fn poll_observations(&mut self) -> Result<Vec<SessionObservation>, DriverRouteError> {
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
            for event in driver.poll_observations() {
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
