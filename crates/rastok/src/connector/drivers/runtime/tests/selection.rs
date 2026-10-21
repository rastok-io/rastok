use super::*;
use crate::connector::drivers::DriverName;

#[derive(Default)]
struct CallState {
    created: usize,
    prepared: usize,
    starts: Vec<CallOpen>,
    cancelled: Vec<crate::connector::exchange::CallId>,
    responses: Vec<CallResponse>,
    polls: usize,
}

struct NamedCallFactory {
    id: ContractId,
    name: &'static str,
    modes: &'static [&'static str],
    selector: Option<&'static str>,
    roles: ConnectorDirection,
    exchange: ExchangeKind,
    state: Arc<Mutex<CallState>>,
}

impl DriverFactory for NamedCallFactory {
    fn contract_id(&self) -> &ContractId {
        &self.id
    }
    fn name(&self) -> &str {
        self.name
    }
    fn roles(&self) -> ConnectorDirection {
        self.roles
    }
    fn exchange(&self) -> ExchangeKind {
        self.exchange
    }
    fn modes(&self) -> &[&str] {
        self.modes
    }
    fn mode_attribute(&self) -> Option<&str> {
        self.selector
    }
    fn create<'plan>(
        &self,
        _: &'plan ValidatedExecutionPlan,
        _: &[node::Path],
    ) -> Result<Box<dyn Driver + 'plan>, DriverFactoryError> {
        self.state.lock().unwrap().created += 1;
        Ok(Box::new(NamedCallDriver(Arc::clone(&self.state))))
    }
}

struct NamedCallDriver(Arc<Mutex<CallState>>);

impl Driver for NamedCallDriver {
    fn inbound(&mut self) -> Option<&mut dyn crate::connector::driver::Ingress> {
        None
    }
    fn outbound(&mut self) -> Option<&mut dyn Outbound> {
        Some(self)
    }
}

impl OutboundCall for NamedCallDriver {
    fn prepare_request(
        &mut self,
        _: &node::Path,
        data: &Data,
        _: &BTreeMap<String, Value>,
    ) -> RequestPreparation {
        self.0.lock().unwrap().prepared += 1;
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
    fn start(&mut self, call: &CallOpen, _: &Envelope) -> CallStart {
        self.0.lock().unwrap().starts.push(call.clone());
        CallStart::Started
    }
    fn poll_responses(&mut self) -> Vec<CallResponse> {
        let mut state = self.0.lock().unwrap();
        state.polls += 1;
        core::mem::take(&mut state.responses)
    }
    fn cancel(&mut self, call: &crate::connector::exchange::CallId) {
        self.0.lock().unwrap().cancelled.push(*call);
    }
}

fn named_call_factory(name: &'static str, state: &Arc<Mutex<CallState>>) -> NamedCallFactory {
    NamedCallFactory {
        id: ContractId::parse(crate::connector::stdlib::PROCESS_CONNECTOR_ID).unwrap(),
        name,
        modes: &["job"],
        selector: Some("mode"),
        roles: ConnectorDirection::Outbound,
        exchange: ExchangeKind::Call,
        state: Arc::clone(state),
    }
}

fn named_call_plan() -> crate::compiler::Compilation {
    compile(
        &[r#"
graph app : App {
  input in: text
  output out: text
  instance = { key = data }
  connector first : Process { command = "unused"; codec = lines; behavior = deterministic }
  connector second : Process { command = "unused"; codec = lines; behavior = deterministic }
  in -> first -> out
  in -> second -> out
  first.err -> nothing; first.exit -> nothing
  second.err -> nothing; second.exit -> nothing
}
"#],
        &ContractCatalog::try_m0().unwrap(),
    )
    .unwrap()
}

fn selection() -> BTreeMap<node::Path, DriverName> {
    [("first", "a"), ("second", "b")]
        .into_iter()
        .map(|(path, name)| {
            (
                node::Path::new([path]).unwrap(),
                DriverName::new(name).unwrap(),
            )
        })
        .collect()
}

fn inject_tenant(execution: &mut Execution<'_>, journal: &mut MemJournal, tenant: &str) {
    execution
        .accept_input(
            journal,
            IngressProposal {
                provenance: IngressProvenance::single(node::Path::new(["test"]).unwrap()),
                draft: EnvelopeDraft {
                    port: node::Port::new("in").unwrap(),
                    data: Data::materialized(Value::Text(String::from(tenant))),
                    ext: Namespace::new(),
                },
                sequence: IngressSequence::Next,
                session: None,
            },
        )
        .unwrap();
}

fn answer(call: &CallOpen, text: &str) -> CallResponse {
    CallResponse {
        call_id: call.call_id,
        completion: Completion::process(
            vec![Data::materialized(Value::Text(String::from(text)))],
            vec![],
            ProcessExit::Exited {
                exit_code: 0,
                signal: None,
            },
        ),
    }
}

#[test]
fn same_contract_selection_is_explicit_or_uniquely_compatible_never_insertion_order() {
    let compiled = named_call_plan();
    let a = Arc::new(Mutex::new(CallState::default()));
    let b = Arc::new(Mutex::new(CallState::default()));
    let mut reference = None;
    for reverse in [false, true] {
        let mut factories = vec![named_call_factory("a", &a), named_call_factory("b", &b)];
        if reverse {
            factories.reverse();
        }
        let mut registry = DriverRegistry::default();
        for factory in factories {
            registry = registry.try_with(Box::new(factory)).unwrap();
        }
        let manifest = registry.manifest().unwrap();
        if let Some(expected) = &reference {
            assert_eq!(&manifest, expected);
        }
        reference = Some(manifest);
        assert!(matches!(
            RuntimeDrivers::prepare(&compiled.plan, &registry),
            Err(DriverBindingError::Ambiguous { .. })
        ));
        assert_eq!(a.lock().unwrap().created, 0);
        assert_eq!(b.lock().unwrap().created, 0);
    }
    let mut incompatible = named_call_factory("b", &b);
    incompatible.modes = &["service"];
    let registry = DriverRegistry::default()
        .try_with(Box::new(incompatible))
        .unwrap()
        .try_with(Box::new(named_call_factory("a", &a)))
        .unwrap();
    RuntimeDrivers::prepare(&compiled.plan, &registry).unwrap();
    assert_eq!(
        a.lock().unwrap().created,
        1,
        "the sole compatible object serves both Nodes"
    );
    assert_eq!(b.lock().unwrap().created, 0);
}

#[test]
fn every_selection_is_checked_before_any_factory_is_called() {
    let compiled = named_call_plan();
    for failure in [
        "missing",
        "unknown",
        "mode",
        "direction",
        "exchange",
        "path",
    ] {
        let a = Arc::new(Mutex::new(CallState::default()));
        let b = Arc::new(Mutex::new(CallState::default()));
        let mut second = named_call_factory("b", &b);
        match failure {
            "mode" => second.modes = &["service"],
            "direction" => second.roles = ConnectorDirection::Inbound,
            "exchange" => second.exchange = ExchangeKind::Session,
            _ => {}
        }
        let registry = DriverRegistry::default()
            .try_with(Box::new(named_call_factory("a", &a)))
            .unwrap()
            .try_with(Box::new(second))
            .unwrap();
        let mut chosen = selection();
        if failure == "missing" {
            chosen.remove(&node::Path::new(["second"]).unwrap());
        }
        if failure == "unknown" {
            chosen.insert(
                node::Path::new(["second"]).unwrap(),
                DriverName::new("unknown").unwrap(),
            );
        }
        if failure == "path" {
            chosen.insert(
                node::Path::new(["absent"]).unwrap(),
                DriverName::new("a").unwrap(),
            );
        }
        let error = RuntimeDrivers::prepare_with_selection(
            &compiled.plan,
            &registry,
            &crate::deployment::BootBindings::empty(),
            &chosen,
        )
        .unwrap_err();
        assert!(matches!(
            (failure, error),
            ("missing", DriverBindingError::Ambiguous { .. })
                | ("unknown", DriverBindingError::UnknownImplementation { .. })
                | ("mode", DriverBindingError::UnsupportedMode { .. })
                | ("direction", DriverBindingError::MissingRoles { .. })
                | ("exchange", DriverBindingError::IncompatibleExchange { .. })
                | ("path", DriverBindingError::UnknownConnector(_))
        ));
        assert_eq!(
            a.lock().unwrap().created,
            0,
            "{failure}: even the earlier valid Node must not bind"
        );
        assert_eq!(b.lock().unwrap().created, 0, "{failure}");
    }
}

#[test]
fn implementation_metadata_cannot_redefine_a_standard_mode_selector() {
    let state = Arc::new(Mutex::new(CallState::default()));
    let mut factory = named_call_factory("a", &state);
    factory.selector = Some("codec");
    assert!(matches!(
        DriverRegistry::default().try_with(Box::new(factory)),
        Err(DriverRegistrationError::ModeAttribute { .. })
    ));
    assert_eq!(state.lock().unwrap().created, 0);
}

#[test]
fn a_nonstandard_contract_uses_its_declared_selector_without_hardcoding_mode() {
    let compiled = named_call_plan();
    let id = ContractId::parse("test.local/selected-call@1").unwrap();
    let mut raw = compiled.plan.plan().clone();
    for planned in &mut raw.nodes {
        if let crate::plan::NodeKind::Connector(spec) = &mut planned.body {
            spec.contract = id.clone();
            // This test selects an implementation; it does not claim the
            // standard Process repetition guarantees for a foreign Contract.
            spec.idempotent = None;
            spec.attributes.retain(|(name, _)| name != "idempotent");
            let selector = spec
                .attributes
                .iter_mut()
                .find(|(name, _)| name == "mode")
                .unwrap();
            selector.0 = String::from("transport");
            spec.attributes.sort_by(|left, right| left.0.cmp(&right.0));
        }
    }
    let plan = ValidatedExecutionPlan::try_new(raw).unwrap();
    let state = Arc::new(Mutex::new(CallState::default()));
    let mut factory = named_call_factory("a", &state);
    factory.id = id.clone();
    factory.selector = Some("transport");
    let registry = DriverRegistry::default()
        .try_with(Box::new(factory))
        .unwrap();
    RuntimeDrivers::prepare(&plan, &registry).unwrap();
    assert_eq!(state.lock().unwrap().created, 1);
    let mut incompatible = named_call_factory("b", &state);
    incompatible.id = id;
    assert!(matches!(
        registry.try_with(Box::new(incompatible)),
        Err(DriverRegistrationError::ModeAttribute { .. })
    ));
    assert_eq!(state.lock().unwrap().created, 1);
}

#[test]
fn json_selection_keeps_interleaved_calls_and_instance_generations_with_their_objects() {
    let compiled = named_call_plan();
    let a = Arc::new(Mutex::new(CallState::default()));
    let b = Arc::new(Mutex::new(CallState::default()));
    let registry = DriverRegistry::default()
        .try_with(Box::new(named_call_factory("b", &b)))
        .unwrap()
        .try_with(Box::new(named_call_factory("a", &a)))
        .unwrap();
    let (_, selected) = crate::deployment_config::parse(
        r#"{"environment":[],"secrets":[],"drivers":[
        {"connector":"first","implementation":"a"},
        {"connector":"second","implementation":"b"}]}"#,
    )
    .unwrap();
    let mut drivers = RuntimeDrivers::prepare_with_selection(
        &compiled.plan,
        &registry,
        &crate::deployment::BootBindings::empty(),
        &selected,
    )
    .unwrap();
    let mut execution = Execution::new(&compiled.plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    for tenant in ["alice", "bob"] {
        inject_tenant(&mut execution, &mut journal, tenant);
        execution
            .run_ready_with_bindings(&mut journal, &mut drivers)
            .unwrap();
        execution.poll_drivers(&mut journal, &mut drivers).unwrap();
    }
    let first = a.lock().unwrap().starts.clone();
    let second = b.lock().unwrap().starts.clone();
    assert_eq!(first.len(), 2);
    assert_eq!(second.len(), 2);
    assert!(
        first
            .iter()
            .all(|call| call.connector_path.to_string() == "first")
    );
    assert!(
        second
            .iter()
            .all(|call| call.connector_path.to_string() == "second")
    );
    let inspection = execution.inspection();
    let instance = |call: &CallOpen| {
        inspection
            .calls
            .iter()
            .find(|active| {
                active.reference
                    == crate::runtime::execution::inspection::CallReferenceInspection::Outbound(
                        call.call_id,
                    )
            })
            .unwrap()
            .instance
            .clone()
    };
    assert_ne!(instance(&first[0]), instance(&first[1]));
    assert_ne!(instance(&second[0]), instance(&second[1]));
    let wrong = node::Path::new(["second"]).unwrap();
    assert!(matches!(
        drivers.cancel(&wrong, &first[0].call_id),
        Err(DriverRouteError::CallOwnerMismatch { .. })
    ));
    assert!(a.lock().unwrap().cancelled.is_empty());
    assert!(b.lock().unwrap().cancelled.is_empty());
    a.lock().unwrap().responses = first.iter().rev().map(|call| answer(call, "a")).collect();
    b.lock().unwrap().responses = second.iter().rev().map(|call| answer(call, "b")).collect();
    execution.poll_drivers(&mut journal, &mut drivers).unwrap();
    execution
        .run_ready_with_bindings(&mut journal, &mut drivers)
        .unwrap();
    assert_eq!(execution.outputs().len(), 4);
    assert_eq!(
        execution
            .outputs()
            .iter()
            .filter(|output| output.envelope.data()
                == &Data::materialized(Value::Text(String::from("a"))))
            .count(),
        2
    );
    assert_eq!(
        execution
            .outputs()
            .iter()
            .filter(|output| output.envelope.data()
                == &Data::materialized(Value::Text(String::from("b"))))
            .count(),
        2
    );
}

#[test]
fn same_contract_response_and_cancellation_cannot_cross_implementation_ownership() {
    let compiled = named_call_plan();
    let a = Arc::new(Mutex::new(CallState::default()));
    let b = Arc::new(Mutex::new(CallState::default()));
    let registry = DriverRegistry::default()
        .try_with(Box::new(named_call_factory("a", &a)))
        .unwrap()
        .try_with(Box::new(named_call_factory("b", &b)))
        .unwrap();
    let mut drivers = RuntimeDrivers::prepare_with_selection(
        &compiled.plan,
        &registry,
        &crate::deployment::BootBindings::empty(),
        &selection(),
    )
    .unwrap();
    let mut execution = Execution::new(&compiled.plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject_tenant(&mut execution, &mut journal, "alice");
    execution
        .run_ready_with_bindings(&mut journal, &mut drivers)
        .unwrap();
    execution.poll_drivers(&mut journal, &mut drivers).unwrap();
    let call = a.lock().unwrap().starts[0].clone();
    let wrong = node::Path::new(["second"]).unwrap();
    assert!(matches!(
        drivers.cancel(&wrong, &call.call_id),
        Err(DriverRouteError::CallOwnerMismatch { .. })
    ));
    assert!(a.lock().unwrap().cancelled.is_empty());
    assert!(b.lock().unwrap().cancelled.is_empty());
    b.lock().unwrap().responses.push(answer(&call, "crossed"));
    assert!(matches!(
        drivers.poll_responses(),
        Err(DriverRouteError::CallOwnerMismatch { .. })
    ));
    assert!(execution.outputs().is_empty());
}

#[test]
fn call_ownership_reconstructs_without_preparing_or_starting_external_operations() {
    let compiled = named_call_plan();
    let a = Arc::new(Mutex::new(CallState::default()));
    let b = Arc::new(Mutex::new(CallState::default()));
    let registry = DriverRegistry::default()
        .try_with(Box::new(named_call_factory("a", &a)))
        .unwrap()
        .try_with(Box::new(named_call_factory("b", &b)))
        .unwrap();
    let mut original = RuntimeDrivers::prepare_with_selection(
        &compiled.plan,
        &registry,
        &crate::deployment::BootBindings::empty(),
        &selection(),
    )
    .unwrap();
    let mut execution = Execution::new(&compiled.plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject_tenant(&mut execution, &mut journal, "alice");
    execution
        .run_ready_with_bindings(&mut journal, &mut original)
        .unwrap();
    execution.poll_drivers(&mut journal, &mut original).unwrap();
    let first = a.lock().unwrap().starts[0].clone();
    let second = b.lock().unwrap().starts[0].clone();
    let polls = (a.lock().unwrap().polls, b.lock().unwrap().polls);
    let prepared = (a.lock().unwrap().prepared, b.lock().unwrap().prepared);
    let mut recovered = RuntimeDrivers::prepare_with_selection(
        &compiled.plan,
        &registry,
        &crate::deployment::BootBindings::empty(),
        &selection(),
    )
    .unwrap();
    let mut restored = Execution::new(&compiled.plan, GraphVersion(1));
    restored
        .recover_with_bindings(&journal, &mut recovered)
        .unwrap();
    recovered.restore_ownership(&journal).unwrap();
    assert_eq!(a.lock().unwrap().starts.len(), 1);
    assert_eq!(b.lock().unwrap().starts.len(), 1);
    assert_eq!((a.lock().unwrap().polls, b.lock().unwrap().polls), polls);
    assert_eq!(
        (a.lock().unwrap().prepared, b.lock().unwrap().prepared),
        prepared
    );
    assert!(matches!(
        recovered.restore_ownership(&journal),
        Err(super::super::DriverRecoveryError::Active)
    ));
    assert!(matches!(
        recovered.cancel(&second.connector_path, &first.call_id),
        Err(DriverRouteError::CallOwnerMismatch { .. })
    ));
    recovered
        .cancel(&first.connector_path, &first.call_id)
        .unwrap();
    assert_eq!(a.lock().unwrap().cancelled, [first.call_id]);
    assert!(b.lock().unwrap().cancelled.is_empty());
    recovered
        .cancel(&second.connector_path, &second.call_id)
        .unwrap();
    assert_eq!(b.lock().unwrap().cancelled, [second.call_id]);
}

#[test]
fn completed_call_prefix_restores_no_live_driver_ownership() {
    let compiled = named_call_plan();
    let a = Arc::new(Mutex::new(CallState::default()));
    let b = Arc::new(Mutex::new(CallState::default()));
    let registry = DriverRegistry::default()
        .try_with(Box::new(named_call_factory("a", &a)))
        .unwrap()
        .try_with(Box::new(named_call_factory("b", &b)))
        .unwrap();
    let mut original = RuntimeDrivers::prepare_with_selection(
        &compiled.plan,
        &registry,
        &crate::deployment::BootBindings::empty(),
        &selection(),
    )
    .unwrap();
    let mut execution = Execution::new(&compiled.plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject_tenant(&mut execution, &mut journal, "alice");
    execution
        .run_ready_with_bindings(&mut journal, &mut original)
        .unwrap();
    execution.poll_drivers(&mut journal, &mut original).unwrap();
    let first = a.lock().unwrap().starts[0].clone();
    let second = b.lock().unwrap().starts[0].clone();
    a.lock().unwrap().responses.push(answer(&first, "a"));
    b.lock().unwrap().responses.push(answer(&second, "b"));
    execution.poll_drivers(&mut journal, &mut original).unwrap();
    execution
        .run_ready_with_bindings(&mut journal, &mut original)
        .unwrap();
    assert_eq!(execution.outputs().len(), 2);
    assert!(journal.open_calls().is_empty());
    let polls = (a.lock().unwrap().polls, b.lock().unwrap().polls);
    let prepared = (a.lock().unwrap().prepared, b.lock().unwrap().prepared);
    let mut recovered = RuntimeDrivers::prepare_with_selection(
        &compiled.plan,
        &registry,
        &crate::deployment::BootBindings::empty(),
        &selection(),
    )
    .unwrap();
    let mut restored = Execution::new(&compiled.plan, GraphVersion(1));
    restored
        .recover_with_bindings(&journal, &mut recovered)
        .unwrap();
    recovered.restore_ownership(&journal).unwrap();
    // A closed prefix leaves no active owner which would refuse a second restore.
    recovered.restore_ownership(&journal).unwrap();
    assert_eq!(a.lock().unwrap().starts.len(), 1);
    assert_eq!(b.lock().unwrap().starts.len(), 1);
    assert_eq!((a.lock().unwrap().polls, b.lock().unwrap().polls), polls);
    assert_eq!(
        (a.lock().unwrap().prepared, b.lock().unwrap().prepared),
        prepared
    );
}

use crate::connector::drivers::websocket_protocol::WebSocketLabDriver;
use crate::connector::exchange::{InboundSession, SessionStart};

#[derive(Default)]
struct SessionState {
    created: usize,
    prepared: usize,
    openings: Vec<SessionOpen>,
    writes: Vec<SessionWrite>,
    closes: Vec<SessionId>,
    aborts: Vec<SessionId>,
    released: Vec<SessionId>,
    events: Vec<SessionObservation>,
    proposals: VecDeque<PendingIngress>,
    decisions: Vec<(
        IngressProposalHandle,
        Result<AdmittedIngress, IngressRejection>,
    )>,
    pending_open: bool,
    polls: usize,
}

struct NamedSessionFactory {
    id: ContractId,
    name: &'static str,
    state: Arc<Mutex<SessionState>>,
}

impl DriverFactory for NamedSessionFactory {
    fn contract_id(&self) -> &ContractId {
        &self.id
    }
    fn name(&self) -> &str {
        self.name
    }
    fn roles(&self) -> ConnectorDirection {
        ConnectorDirection::InboundAndOutbound
    }
    fn exchange(&self) -> ExchangeKind {
        ExchangeKind::Session
    }
    fn modes(&self) -> &[&str] {
        &[]
    }
    fn create<'plan>(
        &self,
        plan: &'plan ValidatedExecutionPlan,
        nodes: &[node::Path],
    ) -> Result<Box<dyn Driver + 'plan>, DriverFactoryError> {
        let protocol = WebSocketLabDriver::try_bind(plan, nodes, &self.id)?;
        self.state.lock().unwrap().created += 1;
        Ok(Box::new(NamedSessionDriver {
            protocol,
            state: Arc::clone(&self.state),
        }))
    }
}

struct NamedSessionDriver {
    protocol: WebSocketLabDriver,
    state: Arc<Mutex<SessionState>>,
}

impl Driver for NamedSessionDriver {
    fn inbound(&mut self) -> Option<&mut dyn crate::connector::driver::Ingress> {
        Some(self)
    }
    fn outbound(&mut self) -> Option<&mut dyn Outbound> {
        Some(self)
    }
}
impl Outbound for NamedSessionDriver {
    fn call(&mut self) -> Option<&mut dyn OutboundCall> {
        None
    }
    fn session(&mut self) -> Option<&mut dyn Session> {
        Some(self)
    }
}
impl crate::connector::driver::Ingress for NamedSessionDriver {
    fn poll_proposal(&mut self) -> Option<PendingIngress> {
        self.state.lock().unwrap().proposals.front().cloned()
    }
    fn notify_admission(
        &mut self,
        handle: IngressProposalHandle,
        decision: Result<AdmittedIngress, IngressRejection>,
    ) {
        let mut state = self.state.lock().unwrap();
        let pending = state.proposals.pop_front().unwrap();
        assert_eq!(pending.handle, handle);
        state.decisions.push((handle, decision));
    }
}
impl Session for NamedSessionDriver {
    fn prepare_session_open(
        &mut self,
        path: &node::Path,
        data: &Data,
        settings: &BTreeMap<String, Value>,
    ) -> RequestPreparation {
        self.state.lock().unwrap().prepared += 1;
        Session::prepare_session_open(&mut self.protocol, path, data, settings)
    }
    fn scheduler_session_completion(
        &mut self,
        path: &node::Path,
        completion: SchedulerCompletion,
    ) -> Completion {
        Session::scheduler_session_completion(&mut self.protocol, path, completion)
    }
    fn open_session(&mut self, opening: &SessionOpen, _: &Envelope) -> SessionStart {
        let mut state = self.state.lock().unwrap();
        state.openings.push(opening.clone());
        if state.pending_open {
            SessionStart::Pending
        } else {
            SessionStart::Started
        }
    }
    fn prepare_session_write(
        &mut self,
        path: &node::Path,
        session: SessionId,
        data: &Data,
        settings: &BTreeMap<String, Value>,
    ) -> RequestPreparation {
        self.state.lock().unwrap().prepared += 1;
        Session::prepare_session_write(&mut self.protocol, path, session, data, settings)
    }
    fn write_session(&mut self, write: &SessionWrite, _: &Envelope) -> SessionStart {
        self.state.lock().unwrap().writes.push(write.clone());
        SessionStart::Started
    }
    fn close_session(&mut self, session: SessionId) -> SessionStart {
        self.state.lock().unwrap().closes.push(session);
        SessionStart::Started
    }
    fn abort_session(&mut self, session: SessionId) {
        self.state.lock().unwrap().aborts.push(session);
    }
    fn release_session(&mut self, session: SessionId) {
        self.state.lock().unwrap().released.push(session);
    }
    fn poll_observations(&mut self) -> Vec<SessionObservation> {
        let mut state = self.state.lock().unwrap();
        state.polls += 1;
        core::mem::take(&mut state.events)
    }
}

fn named_session_factory(
    name: &'static str,
    state: &Arc<Mutex<SessionState>>,
) -> NamedSessionFactory {
    NamedSessionFactory {
        id: ContractId::parse(crate::connector::stdlib::WEBSOCKET_CLIENT_CONNECTOR_ID).unwrap(),
        name,
        state: Arc::clone(state),
    }
}

fn named_session_plan() -> crate::compiler::Compilation {
    compile(&[r#"
graph app : App {
  input in: text
  output out: text
  instance = { key = data }
  connector first : WebSocketClient { url = "ws://example.test/first"; codec = text; handshake_timeout = duration("1s"); behavior = deterministic }
  connector second : WebSocketClient { url = "ws://example.test/second"; codec = text; handshake_timeout = duration("1s"); behavior = deterministic }
  in -> first -> out
  in -> second -> out
  first.exit -> nothing; second.exit -> nothing
}
"#], &ContractCatalog::try_m0().unwrap()).unwrap()
}

fn session_proposal(path: &str, session: SessionId) -> PendingIngress {
    PendingIngress {
        handle: IngressProposalHandle::new(1),
        proposal: IngressProposal {
            provenance: IngressProvenance::single(node::Path::new([path]).unwrap()),
            draft: EnvelopeDraft {
                port: node::Port::new("out").unwrap(),
                data: Data::materialized(Value::Text(String::from("alice"))),
                ext: Namespace::new(),
            },
            sequence: IngressSequence::Next,
            session: Some(InboundSession::Message(session)),
        },
    }
}

#[test]
fn same_contract_sessions_keep_openings_writes_and_equal_ingress_handles_with_their_objects() {
    let compiled = named_session_plan();
    let a = Arc::new(Mutex::new(SessionState::default()));
    let b = Arc::new(Mutex::new(SessionState::default()));
    let registry = DriverRegistry::default()
        .try_with(Box::new(named_session_factory("b", &b)))
        .unwrap()
        .try_with(Box::new(named_session_factory("a", &a)))
        .unwrap();
    let mut drivers = RuntimeDrivers::prepare_with_selection(
        &compiled.plan,
        &registry,
        &crate::deployment::BootBindings::empty(),
        &selection(),
    )
    .unwrap();
    let mut execution = Execution::new(&compiled.plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    for tenant in ["alice", "bob"] {
        inject_tenant(&mut execution, &mut journal, tenant);
        execution
            .run_ready_with_bindings(&mut journal, &mut drivers)
            .unwrap();
    }
    let first = a.lock().unwrap().openings.clone();
    let second = b.lock().unwrap().openings.clone();
    assert_eq!(first.len(), 2);
    assert_eq!(second.len(), 2);
    for state in [&a, &b] {
        let mut state = state.lock().unwrap();
        state.events = state
            .openings
            .iter()
            .rev()
            .map(|open| SessionObservation::Opened(open.session_id))
            .collect();
    }
    execution.poll_drivers(&mut journal, &mut drivers).unwrap();
    let snapshot = execution.inspection();
    let instance = |id| {
        snapshot
            .sessions
            .iter()
            .find(|session| session.reference == id)
            .unwrap()
            .instance
            .clone()
    };
    assert_ne!(instance(first[0].session_id), instance(first[1].session_id));
    assert_ne!(
        instance(second[0].session_id),
        instance(second[1].session_id)
    );
    for tenant in ["bob", "alice"] {
        inject_tenant(&mut execution, &mut journal, tenant);
        execution
            .run_ready_with_bindings(&mut journal, &mut drivers)
            .unwrap();
    }
    assert_eq!(a.lock().unwrap().writes.len(), 2);
    assert_eq!(b.lock().unwrap().writes.len(), 2);
    let first_ids: BTreeSet<_> = first.iter().map(|open| open.session_id).collect();
    let second_ids: BTreeSet<_> = second.iter().map(|open| open.session_id).collect();
    assert!(first_ids.is_disjoint(&second_ids));
    assert!(
        a.lock()
            .unwrap()
            .writes
            .iter()
            .all(|write| first_ids.contains(&write.session_id))
    );
    assert!(
        b.lock()
            .unwrap()
            .writes
            .iter()
            .all(|write| second_ids.contains(&write.session_id))
    );

    let wrong = &second[0].connector_path;
    assert!(matches!(
        drivers.close_session(wrong, first[0].session_id),
        Err(DriverRouteError::SessionOwnerMismatch { .. })
    ));
    assert!(matches!(
        drivers.abort_session(wrong, first[0].session_id),
        Err(DriverRouteError::SessionOwnerMismatch { .. })
    ));
    let data = Data::materialized(Value::Text(String::from("hello")));
    assert!(matches!(
        drivers.prepare_session_write(wrong, first[0].session_id, &data, &BTreeMap::new()),
        Err(DriverRouteError::SessionOwnerMismatch { .. })
    ));
    let mut crossed = a.lock().unwrap().writes[0].clone();
    crossed.connector_path = wrong.clone();
    let message = journal.find_envelope(crossed.emitting_id).unwrap();
    assert!(matches!(
        drivers.write_session(&crossed, &message),
        Err(DriverRouteError::SessionOwnerMismatch { .. })
    ));
    assert!(b.lock().unwrap().closes.is_empty());
    assert!(b.lock().unwrap().aborts.is_empty());
    assert_eq!(b.lock().unwrap().writes.len(), 2);

    a.lock()
        .unwrap()
        .proposals
        .push_back(session_proposal("first", first[0].session_id));
    b.lock()
        .unwrap()
        .proposals
        .push_back(session_proposal("second", second[0].session_id));
    let one = drivers.next_inbound().unwrap().unwrap();
    let two = drivers.next_inbound().unwrap().unwrap();
    assert_eq!(one.proposal().handle, two.proposal().handle);
    assert_ne!(
        one.proposal().proposal.provenance,
        two.proposal().proposal.provenance
    );
    drivers
        .decide_inbound(&two, Err(IngressRejection::InvalidPayload))
        .unwrap();
    assert!(a.lock().unwrap().decisions.is_empty());
    assert_eq!(b.lock().unwrap().decisions.len(), 1);
    drivers
        .decide_inbound(&one, Err(IngressRejection::InvalidPayload))
        .unwrap();
    assert_eq!(a.lock().unwrap().decisions.len(), 1);
    assert_eq!(b.lock().unwrap().decisions.len(), 1);
    assert!(matches!(
        drivers.decide_inbound(&one, Err(IngressRejection::InvalidPayload)),
        Err(DriverRouteError::UnknownIngressProposal { .. })
    ));

    drivers
        .close_session(&first[0].connector_path, first[0].session_id)
        .unwrap();
    drivers
        .abort_session(&second[0].connector_path, second[0].session_id)
        .unwrap();
    assert_eq!(a.lock().unwrap().closes, [first[0].session_id]);
    assert_eq!(b.lock().unwrap().aborts, [second[0].session_id]);
    b.lock()
        .unwrap()
        .events
        .push(SessionObservation::Opened(first[0].session_id));
    assert!(matches!(
        drivers.poll_observations(),
        Err(DriverRouteError::SessionOwnerMismatch { .. })
    ));
}

#[test]
fn pending_session_openings_recover_ownership_without_opening_or_polling_again() {
    let compiled = named_session_plan();
    let a = Arc::new(Mutex::new(SessionState {
        pending_open: true,
        ..SessionState::default()
    }));
    let b = Arc::new(Mutex::new(SessionState {
        pending_open: true,
        ..SessionState::default()
    }));
    let registry = DriverRegistry::default()
        .try_with(Box::new(named_session_factory("a", &a)))
        .unwrap()
        .try_with(Box::new(named_session_factory("b", &b)))
        .unwrap();
    let mut drivers = RuntimeDrivers::prepare_with_selection(
        &compiled.plan,
        &registry,
        &crate::deployment::BootBindings::empty(),
        &selection(),
    )
    .unwrap();
    let mut execution = Execution::new(&compiled.plan, GraphVersion(1));
    let mut journal = MemJournal::new(GraphVersion(1));
    inject_tenant(&mut execution, &mut journal, "alice");
    execution
        .run_ready_with_bindings(&mut journal, &mut drivers)
        .unwrap();
    let first = a.lock().unwrap().openings[0].clone();
    let second = b.lock().unwrap().openings[0].clone();
    let counts = (
        a.lock().unwrap().openings.len(),
        b.lock().unwrap().openings.len(),
    );
    let prepared = (a.lock().unwrap().prepared, b.lock().unwrap().prepared);
    assert!(
        journal.live_sessions().is_empty(),
        "both openings remain locally pending"
    );
    let mut recovered = RuntimeDrivers::prepare_with_selection(
        &compiled.plan,
        &registry,
        &crate::deployment::BootBindings::empty(),
        &selection(),
    )
    .unwrap();
    let mut restored = Execution::new(&compiled.plan, GraphVersion(1));
    restored
        .recover_with_bindings(&journal, &mut recovered)
        .unwrap();
    recovered.restore_ownership(&journal).unwrap();
    assert_eq!(
        (
            a.lock().unwrap().openings.len(),
            b.lock().unwrap().openings.len()
        ),
        counts
    );
    assert_eq!(
        (a.lock().unwrap().prepared, b.lock().unwrap().prepared),
        prepared
    );
    assert_eq!(a.lock().unwrap().polls, 0);
    assert_eq!(b.lock().unwrap().polls, 0);
    assert!(matches!(
        recovered.abort_session(&second.connector_path, first.session_id),
        Err(DriverRouteError::SessionOwnerMismatch { .. })
    ));
    recovered
        .abort_session(&first.connector_path, first.session_id)
        .unwrap();
    recovered
        .abort_session(&second.connector_path, second.session_id)
        .unwrap();
    assert_eq!(a.lock().unwrap().aborts, [first.session_id]);
    assert_eq!(b.lock().unwrap().aborts, [second.session_id]);
}
