use super::*;
use crate::compiler::compile;
use crate::connector::ConnectorDirection;
use crate::connector::ContractId;
use crate::connector::ExchangeKind;
use crate::connector::contract::ContractCatalog;
use crate::connector::driver::InboundCall;
use crate::connector::driver::OutboundCall;
use crate::connector::driver::OutboundDrivers;
use crate::connector::driver::OutboundRole;
use crate::connector::driver::Session;
use crate::connector::drivers::registry::{
    DriverFactory, DriverFactoryError, DriverRegistrationError,
};
use crate::connector::exchange::AdmittedIngress;
use crate::connector::exchange::CallOpen;
use crate::connector::exchange::CallResponse;
use crate::connector::exchange::CallStart;
use crate::connector::exchange::Completion;
use crate::connector::exchange::EnvelopeDraft;
use crate::connector::exchange::InboundCallObservation;
use crate::connector::exchange::InboundCallResponse;
use crate::connector::exchange::InboundCallResponseStart;
use crate::connector::exchange::InboundExchangeId;
use crate::connector::exchange::IngressProposal;
use crate::connector::exchange::IngressProposalHandle;
use crate::connector::exchange::IngressProvenance;
use crate::connector::exchange::IngressRejection;
use crate::connector::exchange::IngressSequence;
use crate::connector::exchange::PendingIngress;
use crate::connector::exchange::RequestPreparation;
use crate::connector::exchange::SchedulerCompletion;
use crate::connector::exchange::SessionId;
use crate::connector::exchange::SessionObservation;
use crate::connector::exchange::SessionOpen;
use crate::connector::exchange::SessionWrite;
use crate::connector::stdlib::HttpServerExit;
use crate::connector::stdlib::ProcessExit;
use crate::envelope::{Data, Envelope, Namespace, Value};
use crate::journal::{Journal, MemJournal, ids};
use crate::node;
use crate::plan::{GraphVersion, ValidatedExecutionPlan};
use crate::runtime::execution::Execution;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

struct FakeFactory {
    id: ContractId,
    roles: ConnectorDirection,
}

impl DriverFactory for FakeFactory {
    fn name(&self) -> &str {
        "test"
    }
    fn exchange(&self) -> crate::connector::ExchangeKind {
        crate::connector::ExchangeKind::Call
    }
    fn modes(&self) -> &[&str] {
        &[]
    }

    fn contract_id(&self) -> &ContractId {
        &self.id
    }

    fn roles(&self) -> ConnectorDirection {
        self.roles
    }

    fn create<'plan>(
        &self,
        _plan: &'plan ValidatedExecutionPlan,
        _nodes: &[node::Path],
    ) -> Result<Box<dyn Driver + 'plan>, DriverFactoryError> {
        Err(DriverFactoryError::new("the registry test never binds"))
    }
}

fn fake(id: &str, roles: ConnectorDirection) -> Box<dyn DriverFactory> {
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
            .require(
                &id,
                ConnectorDirection::Inbound,
                ExchangeKind::Call,
                None,
                None
            )
            .unwrap()
            .1
            .contract_id(),
        &id
    );
    assert_eq!(
        registry
            .require(
                &id,
                ConnectorDirection::Outbound,
                ExchangeKind::Call,
                None,
                None
            )
            .err(),
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
                ExchangeKind::Call,
                None,
                None,
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
        DriverRegistrationError::Duplicate {
            id,
            name: super::DriverName::new("test").unwrap()
        },
    );
}

#[derive(Default)]
struct EmptyInbound;

impl crate::connector::driver::InboundRole for EmptyInbound {
    fn poll_proposal(&mut self) -> Option<PendingIngress> {
        None
    }

    fn notify_admission(
        &mut self,
        _: IngressProposalHandle,
        _: Result<AdmittedIngress, IngressRejection>,
    ) {
    }
}

struct RoutingDriver {
    family: String,
    starts: Arc<Mutex<Vec<String>>>,
    answers: Vec<CallResponse>,
}

impl OutboundCall for RoutingDriver {
    fn prepare_request(
        &mut self,
        _: &node::Path,
        data: &Data,
        _: &BTreeMap<String, crate::value::Value>,
    ) -> RequestPreparation {
        crate::connector::stdlib::prepare_process_request(
            crate::connector::stdlib::RequestCodec::Lines,
            data,
        )
    }

    fn scheduler_completion(
        &mut self,
        _: &node::Path,
        completion: SchedulerCompletion,
    ) -> Completion {
        crate::connector::stdlib::process_scheduler_completion(completion)
    }

    fn start(&mut self, intent: &CallOpen, _: &Envelope) -> CallStart {
        self.starts
            .lock()
            .unwrap()
            .push(format!("{}:{}", self.family, intent.connector_path));
        self.answers.push(CallResponse {
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

    fn poll_responses(&mut self) -> Vec<CallResponse> {
        core::mem::take(&mut self.answers)
    }
}

struct RoutingBound {
    inbound: Option<EmptyInbound>,
    outbound: Option<RoutingDriver>,
}

impl Driver for RoutingBound {
    fn inbound_role(&mut self) -> Option<&mut dyn crate::connector::driver::InboundRole> {
        self.inbound
            .as_mut()
            .map(|driver| driver as &mut dyn crate::connector::driver::InboundRole)
    }

    fn outbound_role(&mut self) -> Option<&mut dyn OutboundRole> {
        self.outbound
            .as_mut()
            .map(|driver| driver as &mut dyn OutboundRole)
    }
}

struct RoutingFactory {
    id: ContractId,
    advertised: ConnectorDirection,
    actual: Option<ConnectorDirection>,
    starts: Arc<Mutex<Vec<String>>>,
}

impl DriverFactory for RoutingFactory {
    fn name(&self) -> &str {
        "test-routing"
    }
    fn exchange(&self) -> crate::connector::ExchangeKind {
        crate::connector::ExchangeKind::Call
    }
    fn modes(&self) -> &[&str] {
        &["job"]
    }
    fn mode_attribute(&self) -> Option<&str> {
        Some("mode")
    }

    fn contract_id(&self) -> &ContractId {
        &self.id
    }

    fn roles(&self) -> ConnectorDirection {
        self.advertised
    }

    fn create<'plan>(
        &self,
        _: &'plan ValidatedExecutionPlan,
        _: &[node::Path],
    ) -> Result<Box<dyn Driver + 'plan>, DriverFactoryError> {
        let inbound = self
            .actual
            .filter(|roles| roles.has_inbound())
            .map(|_| EmptyInbound);
        let outbound = self
            .actual
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
) -> Box<dyn DriverFactory> {
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
        let Some(spec) = (match &mut planned.body {
            crate::plan::NodeKind::Connector(spec) => Some(spec),
            crate::plan::NodeKind::Graph(_) | crate::plan::NodeKind::Operator(_) => None,
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
    let crate::plan::NodeKind::Connector(spec) = &mut first.body else {
        panic!("the first node is a connector")
    };
    spec.exchange = ExchangeKind::Session;
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
    let crate::plan::NodeKind::Connector(spec) = &mut source.body else {
        panic!("the source node is a connector")
    };
    spec.contract = ContractId::parse("org.rastok.test/event-source@1").unwrap();
    spec.idempotent = None;
    spec.attributes.retain(|(name, _)| name != "idempotent");
    spec.roles = ConnectorDirection::Inbound;
    spec.exchange = ExchangeKind::Event;
    source.ports.retain(|port| port.port.name() != "exit");
    for node in &mut raw.nodes {
        if let crate::plan::NodeKind::Graph(graph) = &mut node.body {
            graph
                .wiring
                .retain(|rule| rule.source.port.name() != "exit");
        }
    }
    ValidatedExecutionPlan::try_new(raw).expect("the foreign Event contract is structural")
}

struct DuplexDriver {
    observations: Arc<Mutex<Vec<&'static str>>>,
}

impl crate::connector::driver::InboundRole for DuplexDriver {
    fn poll_proposal(&mut self) -> Option<PendingIngress> {
        self.observations.lock().unwrap().push("inbound");
        None
    }

    fn notify_admission(
        &mut self,
        _: IngressProposalHandle,
        _: Result<AdmittedIngress, IngressRejection>,
    ) {
    }

    fn call(&mut self) -> Option<&mut dyn InboundCall> {
        Some(self)
    }
}

impl InboundCall for DuplexDriver {
    fn start_response(&mut self, _: &InboundCallResponse) -> InboundCallResponseStart {
        InboundCallResponseStart::Pending
    }

    fn start_timeout(
        &mut self,
        _: &crate::connector::exchange::InboundCallTimeout,
    ) -> crate::connector::exchange::InboundCallTimeoutStart {
        crate::connector::exchange::InboundCallTimeoutStart::Pending
    }

    fn poll_completions(&mut self) -> Vec<InboundCallObservation> {
        Vec::new()
    }
}

impl OutboundCall for DuplexDriver {
    fn prepare_request(
        &mut self,
        _: &node::Path,
        data: &Data,
        _: &BTreeMap<String, crate::value::Value>,
    ) -> RequestPreparation {
        self.observations.lock().unwrap().push("outbound");
        crate::connector::stdlib::prepare_process_request(
            crate::connector::stdlib::RequestCodec::Lines,
            data,
        )
    }

    fn scheduler_completion(
        &mut self,
        _: &node::Path,
        completion: SchedulerCompletion,
    ) -> Completion {
        crate::connector::stdlib::process_scheduler_completion(completion)
    }

    fn start(&mut self, _: &CallOpen, _: &Envelope) -> CallStart {
        CallStart::Started
    }

    fn poll_responses(&mut self) -> Vec<CallResponse> {
        Vec::new()
    }
}

impl Driver for DuplexDriver {
    fn inbound_role(&mut self) -> Option<&mut dyn crate::connector::driver::InboundRole> {
        Some(self)
    }

    fn outbound_role(&mut self) -> Option<&mut dyn OutboundRole> {
        Some(self)
    }
}

struct DuplexFactory {
    id: ContractId,
    observations: Arc<Mutex<Vec<&'static str>>>,
}

impl DriverFactory for DuplexFactory {
    fn name(&self) -> &str {
        "test-routing"
    }
    fn exchange(&self) -> crate::connector::ExchangeKind {
        crate::connector::ExchangeKind::Call
    }
    fn modes(&self) -> &[&str] {
        &["job"]
    }
    fn mode_attribute(&self) -> Option<&str> {
        Some("mode")
    }

    fn contract_id(&self) -> &ContractId {
        &self.id
    }

    fn roles(&self) -> ConnectorDirection {
        ConnectorDirection::InboundAndOutbound
    }

    fn create<'plan>(
        &self,
        _: &'plan ValidatedExecutionPlan,
        _: &[node::Path],
    ) -> Result<Box<dyn Driver + 'plan>, DriverFactoryError> {
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
    let mut drivers = RuntimeDrivers::prepare(&plan, &registry).unwrap();

    assert!(drivers.next_inbound().unwrap().is_none());
    let preparation = OutboundDrivers::prepare_request(
        &mut drivers,
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

impl crate::connector::driver::InboundRole for SessionDuplexDriver {
    fn poll_proposal(&mut self) -> Option<PendingIngress> {
        self.observations.lock().unwrap().push("inbound");
        None
    }

    fn notify_admission(
        &mut self,
        _: IngressProposalHandle,
        _: Result<AdmittedIngress, IngressRejection>,
    ) {
        self.observations.lock().unwrap().push("decision");
    }
}

impl Session for SessionDuplexDriver {
    fn prepare_session_open(
        &mut self,
        _: &node::Path,
        data: &Data,
        _: &BTreeMap<String, crate::value::Value>,
    ) -> RequestPreparation {
        crate::connector::stdlib::prepare_process_request(
            crate::connector::stdlib::RequestCodec::Lines,
            data,
        )
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
    ) -> crate::connector::exchange::SessionStart {
        self.observations.lock().unwrap().push("open");
        self.events
            .insert(0, SessionObservation::Opened(intent.session_id));
        crate::connector::exchange::SessionStart::Started
    }

    fn prepare_session_write(
        &mut self,
        _: &node::Path,
        _: SessionId,
        data: &Data,
        _: &BTreeMap<String, crate::value::Value>,
    ) -> RequestPreparation {
        crate::connector::stdlib::prepare_process_request(
            crate::connector::stdlib::RequestCodec::Lines,
            data,
        )
    }

    fn write_session(
        &mut self,
        intent: &SessionWrite,
        _: &Envelope,
    ) -> crate::connector::exchange::SessionStart {
        self.observations.lock().unwrap().push("write");
        self.events.insert(
            0,
            SessionObservation::WriteSettled {
                session: intent.session_id,
                write: intent.write_ref,
            },
        );
        crate::connector::exchange::SessionStart::Started
    }

    fn close_session(&mut self, session: SessionId) -> crate::connector::exchange::SessionStart {
        if self.closed.insert(session) {
            self.observations.lock().unwrap().push("close");
            self.events.insert(
                0,
                SessionObservation::Exited {
                    session,
                    exit: crate::connector::exchange::EnvelopeDraft {
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
        crate::connector::exchange::SessionStart::Started
    }

    fn poll_observations(&mut self) -> Vec<SessionObservation> {
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

impl OutboundRole for SessionDuplexDriver {
    fn call(&mut self) -> Option<&mut dyn OutboundCall> {
        None
    }

    fn session(&mut self) -> Option<&mut dyn Session> {
        Some(self)
    }
}

impl Driver for SessionDuplexDriver {
    fn inbound_role(&mut self) -> Option<&mut dyn crate::connector::driver::InboundRole> {
        Some(self)
    }

    fn outbound_role(&mut self) -> Option<&mut dyn OutboundRole> {
        Some(self)
    }
}

struct SessionDuplexFactory {
    id: ContractId,
    observations: Arc<Mutex<Vec<&'static str>>>,
}

impl DriverFactory for SessionDuplexFactory {
    fn name(&self) -> &str {
        "test-routing"
    }
    fn exchange(&self) -> crate::connector::ExchangeKind {
        crate::connector::ExchangeKind::Session
    }
    fn modes(&self) -> &[&str] {
        &["job"]
    }
    fn mode_attribute(&self) -> Option<&str> {
        Some("mode")
    }

    fn contract_id(&self) -> &ContractId {
        &self.id
    }

    fn roles(&self) -> ConnectorDirection {
        ConnectorDirection::InboundAndOutbound
    }

    fn create<'plan>(
        &self,
        _: &'plan ValidatedExecutionPlan,
        _: &[node::Path],
    ) -> Result<Box<dyn Driver + 'plan>, DriverFactoryError> {
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
    let mut drivers = RuntimeDrivers::prepare(&plan, &registry).unwrap();
    assert!(drivers.next_inbound().unwrap().is_none());

    let mut journal = MemJournal::new(GraphVersion(1));
    let request = journal
        .append_ingress(IngressProposal {
            provenance: IngressProvenance::single(node::Path::new(["world"]).unwrap()),
            draft: EnvelopeDraft {
                port: node::Port::new("in").unwrap(),
                data: Data::materialized(Value::Text(String::from("hello"))),
                ext: Namespace::new(),
            },
            sequence: IngressSequence::Next,
            session: None,
        })
        .unwrap()
        .envelope;
    let connector = node::Path::new(["first"]).unwrap();
    let opening = |byte| {
        let prepared = crate::connector::exchange::PreparedRequest::seal(vec![byte]);
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
        OutboundDrivers::open_session(&mut drivers, intent, &request).unwrap();
    }
    assert_eq!(
        OutboundDrivers::poll_observations(&mut drivers).unwrap(),
        [
            SessionObservation::Opened(second.session_id),
            SessionObservation::Opened(first.session_id),
        ],
        "reverse arrival remains correlated by SessionId"
    );
    journal.confirm_session_open(&first.session_id).unwrap();
    journal.confirm_session_open(&second.session_id).unwrap();

    let write = |session: SessionId, byte| {
        let prepared = crate::connector::exchange::PreparedRequest::seal(vec![byte]);
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
        OutboundDrivers::write_session(&mut drivers, intent, &request).unwrap();
    }
    assert_eq!(
        OutboundDrivers::poll_observations(&mut drivers).unwrap(),
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

    OutboundDrivers::abort_session(&mut drivers, &connector, first.session_id).unwrap();
    OutboundDrivers::abort_session(&mut drivers, &connector, first.session_id).unwrap();
    assert_eq!(
        observations
            .lock()
            .unwrap()
            .iter()
            .filter(|observation| **observation == "abort")
            .count(),
        1
    );
    assert!(drivers.sessions.contains_key(&first.session_id));
    assert!(
        OutboundDrivers::poll_observations(&mut drivers)
            .unwrap()
            .is_empty()
    );
    OutboundDrivers::forget_session(&mut drivers, first.session_id).unwrap();
    OutboundDrivers::forget_session(&mut drivers, first.session_id).unwrap();
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
    assert!(!drivers.sessions.contains_key(&first.session_id));
    assert!(drivers.sessions.contains_key(&second.session_id));
    assert!(matches!(
        OutboundDrivers::abort_session(&mut drivers, &connector, first.session_id),
        Err(crate::connector::driver::DriverRouteError::UncorrelatedSession(reference)) if reference == first.session_id
    ));
    OutboundDrivers::write_session(&mut drivers, &second_write, &request).unwrap();
}

#[test]
fn a_session_driver_passes_the_exchange_specific_conformance_fixture() {
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
    let contract = Box::leak(Box::new(crate::connector::contract::ConnectorContract {
        id: "org.rastok.test/alpha@1",
        name: "SessionProbe",
        roles: ConnectorDirection::InboundAndOutbound,
        exchange: ExchangeKind::Session,
        ..crate::connector::stdlib::PROCESS.clone()
    }));
    let terminal = Data::materialized(
        ProcessExit::Exited {
            exit_code: 0,
            signal: None,
        }
        .into_value(),
    );
    let fixture = crate::connector::drivers::conformance::SessionTestFixture {
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

    let report = crate::connector::drivers::conformance::verify_session(&fixture, &registry);
    crate::connector::drivers::conformance::assert_conformant(&report, &[]);
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
    proposals: VecDeque<PendingIngress>,
    decisions: Vec<(
        IngressProposalHandle,
        Result<AdmittedIngress, IngressRejection>,
    )>,
}

struct RetainedInboundDriver {
    state: Arc<Mutex<InboundQueueState>>,
}

impl crate::connector::driver::InboundRole for RetainedInboundDriver {
    fn poll_proposal(&mut self) -> Option<PendingIngress> {
        self.state.lock().unwrap().proposals.front().cloned()
    }

    fn notify_admission(
        &mut self,
        reference: IngressProposalHandle,
        decision: Result<AdmittedIngress, IngressRejection>,
    ) {
        let mut state = self.state.lock().unwrap();
        let retained = state
            .proposals
            .pop_front()
            .expect("a decision follows a retained proposal");
        assert_eq!(retained.handle, reference);
        state.decisions.push((reference, decision));
    }
}

impl Driver for RetainedInboundDriver {
    fn inbound_role(&mut self) -> Option<&mut dyn crate::connector::driver::InboundRole> {
        Some(self)
    }

    fn outbound_role(&mut self) -> Option<&mut dyn OutboundRole> {
        None
    }
}

struct RetainedInboundFactory {
    id: ContractId,
    state: Arc<Mutex<InboundQueueState>>,
}

#[derive(Default)]
struct RetainedCallState {
    proposals: VecDeque<PendingIngress>,
    decisions: Vec<(
        IngressProposalHandle,
        Result<AdmittedIngress, IngressRejection>,
    )>,
    events: Vec<InboundCallObservation>,
    responses: Vec<InboundCallResponse>,
    acknowledge_on_accept: bool,
    hold_first_response: bool,
}

struct RetainedCallDriver {
    state: Arc<Mutex<RetainedCallState>>,
}

impl crate::connector::driver::InboundRole for RetainedCallDriver {
    fn poll_proposal(&mut self) -> Option<PendingIngress> {
        self.state.lock().unwrap().proposals.front().cloned()
    }

    fn notify_admission(
        &mut self,
        reference: IngressProposalHandle,
        decision: Result<AdmittedIngress, IngressRejection>,
    ) {
        let mut state = self.state.lock().unwrap();
        let retained = state
            .proposals
            .pop_front()
            .expect("a decision follows a retained proposal");
        assert_eq!(retained.handle, reference);
        if state.acknowledge_on_accept
            && let Ok(accepted) = decision
        {
            let Some(InboundExchangeId::Call(call_id)) = accepted.exchange else {
                panic!("an inbound Call acceptance carries its Call ref")
            };
            state.events.push(InboundCallObservation {
                call_id,
                completion: Completion::http_server_exit(HttpServerExit::Completed { status: 202 }),
            });
        }
        state.decisions.push((reference, decision));
    }

    fn call(&mut self) -> Option<&mut dyn InboundCall> {
        Some(self)
    }
}

impl InboundCall for RetainedCallDriver {
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
        _: &crate::connector::exchange::InboundCallTimeout,
    ) -> crate::connector::exchange::InboundCallTimeoutStart {
        crate::connector::exchange::InboundCallTimeoutStart::Pending
    }

    fn poll_completions(&mut self) -> Vec<InboundCallObservation> {
        core::mem::take(&mut self.state.lock().unwrap().events)
    }
}

impl Driver for RetainedCallDriver {
    fn inbound_role(&mut self) -> Option<&mut dyn crate::connector::driver::InboundRole> {
        Some(self)
    }

    fn outbound_role(&mut self) -> Option<&mut dyn OutboundRole> {
        None
    }
}

struct RetainedCallFactory {
    id: ContractId,
    state: Arc<Mutex<RetainedCallState>>,
}

impl DriverFactory for RetainedCallFactory {
    fn name(&self) -> &str {
        "test-inbound-call"
    }
    fn exchange(&self) -> crate::connector::ExchangeKind {
        crate::connector::ExchangeKind::Call
    }
    fn modes(&self) -> &[&str] {
        &["ack", "respond"]
    }
    fn mode_attribute(&self) -> Option<&str> {
        Some("mode")
    }

    fn contract_id(&self) -> &ContractId {
        &self.id
    }

    fn roles(&self) -> ConnectorDirection {
        ConnectorDirection::Inbound
    }

    fn create<'plan>(
        &self,
        _: &'plan ValidatedExecutionPlan,
        _: &[node::Path],
    ) -> Result<Box<dyn Driver + 'plan>, DriverFactoryError> {
        Ok(Box::new(RetainedCallDriver {
            state: Arc::clone(&self.state),
        }))
    }
}

impl DriverFactory for RetainedInboundFactory {
    fn name(&self) -> &str {
        "test-inbound"
    }
    fn exchange(&self) -> ExchangeKind {
        if self.id.as_str() == crate::connector::stdlib::HTTP_SERVER_CONNECTOR_ID {
            ExchangeKind::Call
        } else {
            ExchangeKind::Event
        }
    }
    fn modes(&self) -> &[&str] {
        if self.id.as_str() == crate::connector::stdlib::HTTP_SERVER_CONNECTOR_ID {
            &["ack", "respond"]
        } else {
            &["job"]
        }
    }
    fn mode_attribute(&self) -> Option<&str> {
        Some("mode")
    }

    fn contract_id(&self) -> &ContractId {
        &self.id
    }

    fn roles(&self) -> ConnectorDirection {
        ConnectorDirection::Inbound
    }

    fn create<'plan>(
        &self,
        _: &'plan ValidatedExecutionPlan,
        _: &[node::Path],
    ) -> Result<Box<dyn Driver + 'plan>, DriverFactoryError> {
        Ok(Box::new(RetainedInboundDriver {
            state: Arc::clone(&self.state),
        }))
    }
}

fn inbound_proposal(reference: u64, port: &str) -> PendingIngress {
    PendingIngress {
        handle: IngressProposalHandle::new(reference),
        proposal: IngressProposal {
            provenance: IngressProvenance::single(node::Path::new(["source"]).unwrap()),
            draft: EnvelopeDraft {
                port: node::Port::new(port).unwrap(),
                data: Data::materialized(Value::Text(format!("event-{reference}"))),
                ext: Namespace::new(),
            },
            sequence: IngressSequence::Next,
            session: None,
        },
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
        .map(crate::node::NodeIndex::new)
        .expect("the server slot is representable");
    let relay_slot = raw
        .nodes
        .iter()
        .position(|planned| planned.path.to_string() == "relay")
        .and_then(|index| u32::try_from(index).ok())
        .map(crate::node::NodeIndex::new)
        .expect("the relay slot is representable");
    let server = raw
        .nodes
        .get_mut(server_slot.index())
        .expect("the server node exists");
    let crate::plan::NodeKind::Connector(spec) = &mut server.body else {
        panic!("the server node is a connector")
    };
    spec.contract = ContractId::parse("org.rastok.test/inbound-call@1").unwrap();

    let crate::plan::NodeKind::Graph(graph) = &mut raw.nodes[0].body else {
        panic!("the App is a Graph")
    };
    let response = graph
        .wiring
        .iter_mut()
        .find(|rule| rule.source.node == relay_slot)
        .expect("the relay boundary response exists");
    response.selection = crate::plan::RouteSelection::All(vec![crate::plan::Destination::Port(
        crate::plan::Endpoint {
            node: server_slot,
            port: node::Port::new("in").unwrap(),
        },
    )]);
    graph
        .wiring
        .sort_by_key(crate::plan::RouteRule::canonical_order_key);
    raw.nodes[0].ports.retain(|decl| decl.port.name() != "out");
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
            id: ContractId::parse(crate::connector::stdlib::HTTP_SERVER_CONNECTOR_ID).unwrap(),
            state: Arc::new(Mutex::new(InboundQueueState::default())),
        }))
        .unwrap();
    assert!(matches!(
        RuntimeDrivers::prepare(&plan, &event_only_registry),
        Err(DriverBindingError::BoundExchangeMismatch {
            required: ExchangeKind::Call,
            actual: None,
            ..
        })
    ));
    let state = Arc::new(Mutex::new(RetainedCallState {
        proposals: VecDeque::from([PendingIngress {
            handle: IngressProposalHandle::new(1),
            proposal: IngressProposal {
                provenance: IngressProvenance::single(node::Path::new(["server"]).unwrap()),
                draft: EnvelopeDraft {
                    port: node::Port::conventional_out(),
                    data: Data::materialized(Value::Text(String::from("request"))),
                    ext: Namespace::new(),
                },
                sequence: IngressSequence::Next,
                session: None,
            },
        }]),
        acknowledge_on_accept: true,
        ..RetainedCallState::default()
    }));
    let registry = DriverRegistry::default()
        .try_with(Box::new(RetainedCallFactory {
            id: ContractId::parse(crate::connector::stdlib::HTTP_SERVER_CONNECTOR_ID).unwrap(),
            state: Arc::clone(&state),
        }))
        .unwrap();
    let mut drivers = RuntimeDrivers::prepare(&plan, &registry).unwrap();
    let mut execution = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));

    let Some(crate::connector::exchange::IngressAdmission::Accepted(accepted)) = drivers
        .poll_inbound(&mut execution, &mut journal)
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

    let report = drivers.poll_inbound_call_events(&mut execution).unwrap();
    assert_eq!(report.queued, 1);
    assert!(report.rejected.is_empty());
    execution
        .run_ready_with_drivers(&mut journal, &mut drivers)
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
    let duplicate = drivers.poll_inbound_call_events(&mut execution).unwrap();
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
        proposals: VecDeque::from([PendingIngress {
            handle: IngressProposalHandle::new(1),
            proposal: IngressProposal {
                provenance: IngressProvenance::single(node::Path::new(["server"]).unwrap()),
                draft: EnvelopeDraft {
                    port: node::Port::conventional_out(),
                    data: Data::materialized(Value::Text(String::from("request"))),
                    ext: Namespace::new(),
                },
                sequence: IngressSequence::Next,
                session: None,
            },
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
    let mut drivers = RuntimeDrivers::prepare(&plan, &registry).unwrap();
    let mut execution = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));

    let Some(crate::connector::exchange::IngressAdmission::Accepted(accepted)) = drivers
        .poll_inbound(&mut execution, &mut journal)
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
    execution
        .run_ready_with_drivers(&mut journal, &mut drivers)
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
    let premature = drivers.poll_inbound_call_events(&mut execution).unwrap();
    assert_eq!(premature.queued, 0);
    assert_eq!(premature.rejected.len(), 1);

    let first = drivers.poll_inbound_call_responses(&mut execution).unwrap();
    assert_eq!((first.started, first.pending), (0, 1));
    assert!(state.lock().unwrap().responses.is_empty());
    let second = drivers.poll_inbound_call_responses(&mut execution).unwrap();
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
    let report = drivers.poll_inbound_call_events(&mut execution).unwrap();
    assert_eq!(report.queued, 1);
    assert!(report.rejected.is_empty());
    execution
        .run_ready_with_drivers(&mut journal, &mut drivers)
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
        proposals: VecDeque::from([PendingIngress {
            handle: IngressProposalHandle::new(1),
            proposal: IngressProposal {
                provenance: IngressProvenance::single(node::Path::new(["server"]).unwrap()),
                draft: EnvelopeDraft {
                    port: node::Port::conventional_out(),
                    data: Data::materialized(Value::Text(String::from("late"))),
                    ext: Namespace::new(),
                },
                sequence: IngressSequence::Next,
                session: None,
            },
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
    let mut drivers = RuntimeDrivers::prepare(&plan, &registry).unwrap();
    let mut execution = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));

    let Some(crate::connector::exchange::IngressAdmission::Accepted(accepted)) = drivers
        .poll_inbound(&mut execution, &mut journal)
        .expect("the request reaches atomic admission")
    else {
        panic!("the request must be accepted")
    };
    let Some(InboundExchangeId::Call(call_id)) = accepted.exchange else {
        panic!("the accepted request opens one inbound Call")
    };
    assert_eq!(
        drivers
            .poll_inbound_call_events(&mut execution)
            .unwrap()
            .queued,
        1
    );
    execution
        .run_ready_with_drivers(&mut journal, &mut drivers)
        .expect("the accepted terminal and late graph response settle deterministically");

    let response_report = drivers.poll_inbound_call_responses(&mut execution).unwrap();
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
                && transaction.trigger == node::Trigger::Port(node::Port::new("in").unwrap())
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
    let mut drivers = RuntimeDrivers::prepare(&plan, &registry).unwrap();
    let mut execution = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));

    for expected in 0..1024 {
        let Some(crate::connector::exchange::IngressAdmission::Accepted(accepted)) = drivers
            .poll_inbound(&mut execution, &mut journal)
            .expect("the bounded intake stays healthy")
        else {
            panic!("proposal {expected} should fit")
        };
        let state = state.lock().unwrap();
        assert_eq!(state.decisions.len(), expected as usize + 1);
        assert_eq!(state.decisions.last().unwrap().0.get(), expected);
        assert!(matches!(
            state.decisions.last().unwrap().1,
            Ok(observed) if observed == accepted
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
        drivers.poll_inbound(&mut execution, &mut journal).unwrap(),
        Some(crate::connector::exchange::IngressAdmission::Pending)
    );
    assert_eq!(journal.records().len(), records_before);
    assert_eq!(state.lock().unwrap().decisions.len(), 1024);
    let retained = drivers.next_inbound().unwrap().unwrap();
    assert_eq!(retained.proposal().handle.get(), 1024);
    assert_eq!(
        drivers.poll_inbound(&mut execution, &mut journal).unwrap(),
        Some(crate::connector::exchange::IngressAdmission::Pending),
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
    let mut drivers = RuntimeDrivers::prepare(&plan, &registry).unwrap();
    let mut execution = Execution::new(&plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    assert_eq!(
        drivers.poll_inbound(&mut execution, &mut journal).unwrap(),
        Some(crate::connector::exchange::IngressAdmission::Rejected(
            crate::connector::exchange::IngressRejection::UnknownPort
        ))
    );
    assert!(journal.records().is_empty());
    assert_eq!(
        state.lock().unwrap().decisions,
        [(
            IngressProposalHandle::new(7),
            Err(crate::connector::exchange::IngressRejection::UnknownPort)
        )]
    );
}

#[test]
fn inbound_session_framing_cannot_contradict_the_exchange_fact() {
    let plan = inbound_event_plan();
    let mut proposal = inbound_proposal(9, "out");
    proposal.proposal.session = Some(crate::connector::exchange::InboundSession::Open);
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
    let mut drivers = RuntimeDrivers::prepare(&plan, &registry).unwrap();

    assert!(matches!(
        drivers.next_inbound(),
        Err(
            crate::connector::driver::DriverRouteError::InboundExchangeMismatch {
                exchange: ExchangeKind::Event,
                ..
            }
        )
    ));
}

#[test]
fn an_inbound_family_cannot_forge_another_connector_as_its_source() {
    let plan = inbound_event_plan();
    let mut proposal = inbound_proposal(11, "out");
    proposal.proposal.provenance = IngressProvenance::single(node::Path::new(["other"]).unwrap());
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
    let mut drivers = RuntimeDrivers::prepare(&plan, &registry).unwrap();

    assert!(matches!(
        drivers.next_inbound(),
        Err(crate::connector::driver::DriverRouteError::InboundSourceMismatch {
            source,
            ..
        }) if source.to_string() == "other"
    ));
}

#[test]
fn an_event_driver_passes_the_exchange_specific_conformance_fixture() {
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
    let ports = crate::connector::stdlib::PROCESS
        .ports
        .iter()
        .filter(|port| port.name != "exit")
        .cloned()
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let contract = Box::leak(Box::new(crate::connector::contract::ConnectorContract {
        id: "org.rastok.test/event-source@1",
        name: "EventSource",
        roles: ConnectorDirection::Inbound,
        exchange: ExchangeKind::Event,
        ports: Box::leak(ports),
        common: crate::connector::stdlib::PROCESS.common,
        selector: crate::connector::stdlib::PROCESS.selector.clone(),
    }));
    let fixture = crate::connector::drivers::conformance::EventTestFixture {
        contract,
        plan: &plan,
        expected_outputs: vec![Data::materialized(Value::Text(String::from("event-4")))],
    };

    let report = crate::connector::drivers::conformance::verify_event(&fixture, &registry);
    crate::connector::drivers::conformance::assert_conformant(&report, &[]);
    assert_eq!(report.proven_cases, ["event-admission"]);
}

#[test]
fn two_connector_families_bind_and_route_without_execution_name_dispatch() {
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
    let mut drivers = RuntimeDrivers::prepare(&plan, &registry).unwrap();
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&plan, GraphVersion(1));
    execution
        .accept_input(
            &mut journal,
            IngressProposal {
                provenance: IngressProvenance::single(node::Path::new(["test"]).unwrap()),
                draft: EnvelopeDraft {
                    port: node::Port::new("in").unwrap(),
                    data: Data::materialized(Value::Text(String::from("request"))),
                    ext: Namespace::new(),
                },
                sequence: IngressSequence::Next,
                session: None,
            },
        )
        .unwrap();
    for _ in 0..2 {
        execution
            .run_ready_with_drivers(&mut journal, &mut drivers)
            .unwrap();
        execution.poll_drivers(&mut journal, &mut drivers).unwrap();
    }
    execution
        .run_ready_with_drivers(&mut journal, &mut drivers)
        .unwrap();

    assert_eq!(
        *starts.lock().unwrap(),
        [
            String::from("org.rastok.test/alpha@1:first"),
            String::from("org.rastok.test/beta@1:second"),
        ]
    );
    assert_eq!(
        execution.outputs()[0].envelope.data(),
        &Data::materialized(Value::Text(String::from("org.rastok.test/beta@1")))
    );
    assert!(
        drivers.calls.is_empty(),
        "settled calls do not accumulate in the routing table"
    );
}

#[test]
fn binding_refuses_missing_wrong_and_extra_roles_before_execution() {
    let plan = two_family_plan();
    let starts = Arc::new(Mutex::new(Vec::new()));
    assert!(matches!(
        RuntimeDrivers::prepare(&plan, &DriverRegistry::default()),
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
        RuntimeDrivers::prepare(&plan, &wrong),
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
        RuntimeDrivers::prepare(&plan, &extra),
        Err(DriverBindingError::BoundRolesMismatch { .. })
    ));
    assert!(
        starts.lock().unwrap().is_empty(),
        "no Driver effect started"
    );
}

mod selection;
