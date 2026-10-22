//! Effectless resident child: the script replaces replies, not Session state.
use super::*;
use crate::connector::driver::Ingress;
use crate::connector::driver::Session;
use crate::connector::drivers::ProcessRequestSettings;
use crate::connector::exchange::AdmittedIngress;
use crate::connector::exchange::InboundSession;
use crate::connector::exchange::IngressProposalHandle;
use crate::connector::exchange::IngressRejection;
use crate::connector::exchange::PendingIngress;
use crate::connector::exchange::SessionId;
use crate::connector::exchange::SessionObservation;
use crate::connector::exchange::SessionOpen;
use crate::connector::exchange::SessionStart;
use crate::connector::exchange::SessionWrite;
use std::collections::VecDeque;

pub(super) struct Factory {
    id: ContractId,
    mocks: SharedMocks,
    faults: MockFaults,
}
impl Factory {
    pub(super) fn new(mocks: SharedMocks, faults: MockFaults) -> Result<Self, String> {
        Ok(Self {
            id: ContractId::parse(crate::connector::stdlib::PROCESS_CONNECTOR_ID)
                .map_err(|e| e.to_string())?,
            mocks,
            faults,
        })
    }
}
impl DriverFactory for Factory {
    fn name(&self) -> &str {
        "lab-process-service"
    }
    fn exchange(&self) -> crate::connector::ExchangeKind {
        crate::connector::ExchangeKind::Session
    }
    fn modes(&self) -> &[&str] {
        &["service"]
    }

    fn contract_id(&self) -> &ContractId {
        &self.id
    }
    fn roles(&self) -> ConnectorDirection {
        ConnectorDirection::InboundAndOutbound
    }
    fn create<'a>(
        &self,
        plan: &'a ValidatedExecutionPlan,
        nodes: &[node::Path],
    ) -> Result<Box<dyn Driver + 'a>, DriverFactoryError> {
        let request_settings = ProcessRequestSettings::try_bind(plan, nodes)
            .map_err(|e| DriverFactoryError::new(e.to_string()))?;
        let protocol = ProcessRequestSettings::try_bind(plan, nodes)
            .map_err(|e| DriverFactoryError::new(e.to_string()))?;
        Ok(Box::new(World {
            request_settings,
            evaluator: MockOutboundDriver {
                plan,
                mocks: Rc::clone(&self.mocks),
                faults: Rc::clone(&self.faults),
                protocol: Box::new(protocol),
                pending: Vec::new(),
            },
            resources: BTreeMap::new(),
            terminating: std::collections::BTreeSet::new(),
            events: VecDeque::new(),
            proposals: VecDeque::new(),
            next: 0,
        }))
    }
}
struct World<'plan> {
    request_settings: ProcessRequestSettings,
    evaluator: MockOutboundDriver<'plan>,
    resources: BTreeMap<SessionId, node::Path>,
    terminating: std::collections::BTreeSet<SessionId>,
    events: VecDeque<SessionObservation>,
    proposals: VecDeque<PendingIngress>,
    next: u64,
}
impl World<'_> {
    fn answer(&mut self, session: SessionId, path: &node::Path, message: &Envelope) {
        if let Some(outputs) = self.evaluator.answer_outputs(path, None, message) {
            for output in outputs {
                if output.port.name() == "exit" {
                    self.terminating.insert(session);
                    self.events.push_back(SessionObservation::Exited {
                        session,
                        exit: output,
                    });
                } else {
                    let Some(next) = self.next.checked_add(1) else {
                        self.evaluator
                            .fault("the service world's proposal identity is exhausted".into());
                        return;
                    };
                    self.next = next;
                    self.proposals.push_back(PendingIngress {
                        handle: IngressProposalHandle::new(next),
                        proposal: IngressProposal {
                            provenance: IngressProvenance::single(path.clone()),
                            draft: EnvelopeDraft {
                                port: output.port,
                                data: output.data,
                                ext: output.ext,
                            },
                            sequence: IngressSequence::Next,
                            session: Some(InboundSession::Message(session)),
                        },
                    });
                }
            }
        }
    }
}
impl Driver for World<'_> {
    fn inbound(&mut self) -> Option<&mut dyn Ingress> {
        Some(self)
    }
    fn outbound(&mut self) -> Option<&mut dyn Outbound> {
        Some(self)
    }
}
impl Outbound for World<'_> {
    fn call(&mut self) -> Option<&mut dyn OutboundCall> {
        None
    }
    fn session(&mut self) -> Option<&mut dyn Session> {
        Some(self)
    }
}
impl Session for World<'_> {
    fn prepare_session_open(
        &mut self,
        path: &node::Path,
        data: &Data,
        settings: &BTreeMap<String, Value>,
    ) -> RequestPreparation {
        self.request_settings
            .prepare_service(path, data, settings, true)
    }
    fn prepare_session_write(
        &mut self,
        path: &node::Path,
        _: SessionId,
        data: &Data,
        settings: &BTreeMap<String, Value>,
    ) -> RequestPreparation {
        self.request_settings
            .prepare_service(path, data, settings, false)
    }
    fn scheduler_session_completion(
        &mut self,
        _: &node::Path,
        completion: SchedulerCompletion,
    ) -> Completion {
        crate::connector::stdlib::process_scheduler_completion(completion)
    }
    fn open_session(&mut self, intent: &SessionOpen, message: &Envelope) -> SessionStart {
        if self.resources.contains_key(&intent.session_id) {
            return SessionStart::Started;
        }
        if self.resources.len() >= 64 {
            return SessionStart::Pending;
        }
        self.resources
            .insert(intent.session_id, intent.connector_path.clone());
        self.events
            .push_back(SessionObservation::Opened(intent.session_id));
        self.answer(intent.session_id, &intent.connector_path, message);
        SessionStart::Started
    }
    fn write_session(&mut self, intent: &SessionWrite, message: &Envelope) -> SessionStart {
        if !self.resources.contains_key(&intent.session_id)
            || self.terminating.contains(&intent.session_id)
            || self.proposals.len() >= 8
        {
            return SessionStart::Pending;
        }
        self.events.push_back(SessionObservation::WriteSettled {
            session: intent.session_id,
            write: intent.write_ref,
        });
        self.answer(intent.session_id, &intent.connector_path, message);
        SessionStart::Started
    }
    fn poll_observations(&mut self) -> Vec<SessionObservation> {
        let mut ready = Vec::new();
        let mut waiting = VecDeque::new();
        for event in self.events.drain(..) {
            if matches!(&event, SessionObservation::Exited { session, .. } if self.proposals.iter().any(|p| p.proposal.session == Some(InboundSession::Message(*session))))
            {
                waiting.push_back(event);
            } else {
                ready.push(event);
            }
        }
        self.events = waiting;
        ready
    }
    // Silence on system shutdown is explicit: no OS fact is invented by the
    // Lab. An authored quit can return Exited; otherwise Closing wins unknown.
    fn release_session(&mut self, session: SessionId) {
        self.resources.remove(&session);
        self.terminating.remove(&session);
        self.proposals
            .retain(|p| p.proposal.session != Some(InboundSession::Message(session)));
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
impl Ingress for World<'_> {
    fn poll_proposal(&mut self) -> Option<PendingIngress> {
        let proposal = self.proposals.front()?;
        // The script may answer during open_session. As with the physical
        // Driver, the opening must reach the core before its first frame.
        if self.events.iter().any(|event| {
            matches!(event,
            SessionObservation::Opened(reference)
                if proposal.proposal.session == Some(InboundSession::Message(*reference)))
        }) {
            return None;
        }
        Some(proposal.clone())
    }
    fn notify_admission(
        &mut self,
        reference: IngressProposalHandle,
        decision: Result<AdmittedIngress, IngressRejection>,
    ) {
        if self
            .proposals
            .front()
            .is_some_and(|p| p.handle == reference)
        {
            self.proposals.pop_front();
            if let Err(reason) = decision {
                self.evaluator
                    .fault(format!("the service frame was refused: {reason:?}"));
            }
        }
    }
}
