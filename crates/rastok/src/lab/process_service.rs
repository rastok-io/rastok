//! Effectless resident child: the script replaces replies, not Session state.
use super::*;
use crate::boundary::{
    InboundConnectorDriver, InboundSession, IngressDecision, IngressProposal,
    IngressProposalHandle, SessionConnectorDriver, SessionDriverEvent, SessionId,
    SessionOpenIntent, SessionStart, SessionWriteIntent,
};
use crate::connectors::ProcessRequestBindings;
use std::collections::VecDeque;

pub(super) struct Factory {
    id: ContractId,
    mocks: SharedMocks,
    faults: MockFaults,
}
impl Factory {
    pub(super) fn new(mocks: SharedMocks, faults: MockFaults) -> Result<Self, String> {
        Ok(Self {
            id: ContractId::parse(crate::connector::PROCESS_SERVICE_CONNECTOR_ID)
                .map_err(|e| e.to_string())?,
            mocks,
            faults,
        })
    }
}
impl ConnectorDriverFactory for Factory {
    fn connector_id(&self) -> &ContractId {
        &self.id
    }
    fn roles(&self) -> ConnectorRoles {
        ConnectorRoles::InboundAndOutbound
    }
    fn bind<'a>(
        &self,
        plan: &'a ValidatedExecutionPlan,
        nodes: &[node::Path],
    ) -> Result<Box<dyn BoundConnectorDriver + 'a>, DriverFactoryError> {
        let bindings = ProcessRequestBindings::try_bind(plan, nodes)
            .map_err(|e| DriverFactoryError::new(e.to_string()))?;
        let protocol = ProcessRequestBindings::try_bind(plan, nodes)
            .map_err(|e| DriverFactoryError::new(e.to_string()))?;
        Ok(Box::new(World {
            bindings,
            evaluator: MockOutboundConnectorDriver {
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
struct World {
    bindings: ProcessRequestBindings,
    evaluator: MockOutboundConnectorDriver,
    resources: BTreeMap<SessionId, node::Path>,
    terminating: std::collections::BTreeSet<SessionId>,
    events: VecDeque<SessionDriverEvent>,
    proposals: VecDeque<IngressProposal>,
    next: u64,
}
impl World {
    fn answer(&mut self, session: SessionId, path: &node::Path, message: &Envelope) {
        if let Some(outputs) = self.evaluator.answer_outputs(path, None, message) {
            for output in outputs {
                if output.port.name() == "exit" {
                    self.terminating.insert(session);
                    self.events.push_back(SessionDriverEvent::Exited {
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
                    self.proposals.push_back(IngressProposal {
                        reference: IngressProposalHandle::new(next),
                        submission: IngressSubmission {
                            provenance: IngressEnvelopeProvenance::single(path.clone()),
                            port: output.port,
                            draft: Draft {
                                data: output.data,
                                ext: output.ext,
                            },
                            sequence: SubmissionSequence::Next,
                        },
                        session: Some(InboundSession::Message(session)),
                    });
                }
            }
        }
    }
}
impl BoundConnectorDriver for World {
    fn inbound(&mut self) -> Option<&mut dyn InboundConnectorDriver> {
        Some(self)
    }
    fn outbound(&mut self) -> Option<&mut dyn OutboundConnectorDriver> {
        Some(self)
    }
}
impl OutboundConnectorDriver for World {
    fn call(&mut self) -> Option<&mut dyn CallConnectorDriver> {
        None
    }
    fn session(&mut self) -> Option<&mut dyn SessionConnectorDriver> {
        Some(self)
    }
}
impl SessionConnectorDriver for World {
    fn prepare_session_open(
        &mut self,
        path: &node::Path,
        data: &Data,
        settings: &BTreeMap<String, Value>,
    ) -> RequestPreparation {
        self.bindings.prepare_service(path, data, settings, true)
    }
    fn prepare_session_write(
        &mut self,
        path: &node::Path,
        _: SessionId,
        data: &Data,
        settings: &BTreeMap<String, Value>,
    ) -> RequestPreparation {
        self.bindings.prepare_service(path, data, settings, false)
    }
    fn scheduler_session_completion(
        &mut self,
        _: &node::Path,
        completion: SchedulerCompletion,
    ) -> Completion {
        crate::boundary::process_scheduler_completion(completion)
    }
    fn open_session(&mut self, intent: &SessionOpenIntent, message: &Envelope) -> SessionStart {
        if self.resources.contains_key(&intent.session_id) {
            return SessionStart::Started;
        }
        if self.resources.len() >= 64 {
            return SessionStart::Pending;
        }
        self.resources
            .insert(intent.session_id, intent.connector_path.clone());
        self.events
            .push_back(SessionDriverEvent::Opened(intent.session_id));
        self.answer(intent.session_id, &intent.connector_path, message);
        SessionStart::Started
    }
    fn write_session(&mut self, intent: &SessionWriteIntent, message: &Envelope) -> SessionStart {
        if !self.resources.contains_key(&intent.session_id)
            || self.terminating.contains(&intent.session_id)
            || self.proposals.len() >= 8
        {
            return SessionStart::Pending;
        }
        self.events.push_back(SessionDriverEvent::WriteSettled {
            session: intent.session_id,
            write: intent.write_ref,
        });
        self.answer(intent.session_id, &intent.connector_path, message);
        SessionStart::Started
    }
    fn session_events(&mut self) -> Vec<SessionDriverEvent> {
        let mut ready = Vec::new();
        let mut waiting = VecDeque::new();
        for event in self.events.drain(..) {
            if matches!(&event, SessionDriverEvent::Exited { session, .. } if self.proposals.iter().any(|p| p.session == Some(InboundSession::Message(*session))))
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
            .retain(|p| p.session != Some(InboundSession::Message(session)));
        self.events.retain(|event| match event {
            SessionDriverEvent::Opened(reference)
            | SessionDriverEvent::WriteSettled {
                session: reference, ..
            }
            | SessionDriverEvent::Exited {
                session: reference, ..
            } => *reference != session,
        });
    }
}
impl InboundConnectorDriver for World {
    fn proposal(&mut self) -> Option<IngressProposal> {
        let proposal = self.proposals.front()?;
        // The script may answer during open_session. As with the physical
        // Driver, the opening must reach the core before its first frame.
        if self.events.iter().any(|event| {
            matches!(event,
            SessionDriverEvent::Opened(reference)
                if proposal.session == Some(InboundSession::Message(*reference)))
        }) {
            return None;
        }
        Some(proposal.clone())
    }
    fn decide(&mut self, reference: IngressProposalHandle, decision: IngressDecision) {
        if self
            .proposals
            .front()
            .is_some_and(|p| p.reference == reference)
        {
            self.proposals.pop_front();
            if let IngressDecision::Rejected(reason) = decision {
                self.evaluator
                    .fault(format!("the service frame was refused: {reason:?}"));
            }
        }
    }
}
