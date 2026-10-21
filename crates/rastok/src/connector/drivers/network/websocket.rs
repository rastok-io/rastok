//! Hosted WebSocket Session Drivers.
//!
//! The synchronous half sees only retained Rastok proposals and journaled
//! Session intents. Hyper/Tokio/Tungstenite values remain on the private
//! network thread, and every finite queue is charged by the shared pump.

use crate::connector::driver::Ingress;
use crate::connector::driver::Outbound;
use crate::connector::driver::Session;
use crate::connector::drivers::network::pump::{
    EngineInput, EngineLimits, EventPoll, EventReservation, EventSender, HostedNetworkEngine,
    NetworkRuntime, ReserveError, SubmitError,
};
use crate::connector::drivers::websocket_protocol::{
    WebSocketClientOpening, WebSocketLabDriver, WebSocketMessage, WebSocketPlanBinding,
    WebSocketServerBinding, bind_server_routes, decode_client_open, decode_message,
};
use crate::connector::exchange::AdmittedIngress;
use crate::connector::exchange::EnvelopeDraft;
use crate::connector::exchange::InboundExchangeId;
use crate::connector::exchange::InboundSession;
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
use crate::connector::exchange::SessionStart;
use crate::connector::exchange::SessionWrite;
use crate::connector::stdlib::WebSocketExit;
use crate::envelope::{Data, MetaValue, Namespace, Scalar};
use crate::value::Value;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{Empty, Full};
use hyper::body::Bytes;
use hyper::{Request, Response, StatusCode, Version};
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::convert::Infallible;
use std::fmt;
#[cfg(feature = "hosted-http3")]
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_tungstenite::WebSocketStream;
use tungstenite::Message;
use tungstenite::protocol::Role;
use tungstenite::protocol::frame::CloseFrame;

const DEFAULT_MAX_SESSIONS: usize = 64;
const DEFAULT_MAX_HEADER_BYTES: usize = 64 * 1024;
const DEFAULT_BUFFERED_BYTES: usize = 16 * 1024 * 1024;
const MIN_HTTP1_BUFFER_BYTES: usize = 8 * 1024;
const MAX_EVENTS_PER_POLL: usize = 64;

/// Finite physical bounds shared by the hosted WebSocket client and server.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WebSocketConfig {
    max_sessions: usize,
    max_header_bytes: usize,
    buffered_bytes: usize,
}

impl WebSocketConfig {
    /// Constructs hosted Session bounds before a socket is opened.
    pub fn new(
        max_sessions: usize,
        max_header_bytes: usize,
        buffered_bytes: usize,
    ) -> Result<Self, WebSocketConfigError> {
        if max_sessions == 0 {
            return Err(WebSocketConfigError(
                "the WebSocket session bound must be positive",
            ));
        }
        if max_header_bytes < MIN_HTTP1_BUFFER_BYTES {
            return Err(WebSocketConfigError(
                "the WebSocket header bound must cover HTTP/1.1's 8 KiB minimum",
            ));
        }
        if buffered_bytes == 0 || u32::try_from(buffered_bytes).is_err() {
            return Err(WebSocketConfigError(
                "the WebSocket byte budget must be positive and fit a u32 semaphore",
            ));
        }
        Ok(Self {
            max_sessions,
            max_header_bytes,
            buffered_bytes,
        })
    }

    fn limits(self) -> Result<EngineLimits, WebSocketConfigError> {
        EngineLimits::new(
            self.max_sessions,
            self.max_sessions,
            self.max_sessions,
            self.buffered_bytes,
        )
        .map_err(|_| WebSocketConfigError("the WebSocket engine bounds are invalid"))
    }
}

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self {
            max_sessions: DEFAULT_MAX_SESSIONS,
            max_header_bytes: DEFAULT_MAX_HEADER_BYTES,
            buffered_bytes: DEFAULT_BUFFERED_BYTES,
        }
    }
}

/// Invalid physical WebSocket capacity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WebSocketConfigError(&'static str);

impl fmt::Display for WebSocketConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for WebSocketConfigError {}

/// Statically linked hosted implementation of `std/websocket-client@3`.
#[derive(Debug)]
pub struct WebSocketClientFactory {
    id: crate::connector::ContractId,
    config: WebSocketConfig,
}

impl WebSocketClientFactory {
    /// Constructs a client factory without observing DNS or the network.
    pub fn try_new(config: WebSocketConfig) -> Result<Self, crate::connector::ContractIdError> {
        Ok(Self {
            id: crate::connector::ContractId::parse(
                crate::connector::stdlib::WEBSOCKET_CLIENT_CONNECTOR_ID,
            )?,
            config,
        })
    }
}

/// Statically linked hosted implementation of `std/websocket-server@3`.
#[derive(Debug)]
pub struct WebSocketServerFactory {
    id: crate::connector::ContractId,
    config: WebSocketConfig,
}

impl WebSocketServerFactory {
    /// Constructs a server factory without opening a listener.
    pub fn try_new(config: WebSocketConfig) -> Result<Self, crate::connector::ContractIdError> {
        Ok(Self {
            id: crate::connector::ContractId::parse(
                crate::connector::stdlib::WEBSOCKET_SERVER_CONNECTOR_ID,
            )?,
            config,
        })
    }
}

impl crate::connector::drivers::DriverFactory for WebSocketClientFactory {
    fn name(&self) -> &str {
        "websocket-client"
    }
    fn exchange(&self) -> crate::connector::ExchangeKind {
        crate::connector::ExchangeKind::Session
    }
    fn modes(&self) -> &[&str] {
        &[]
    }

    fn contract_id(&self) -> &crate::connector::ContractId {
        &self.id
    }

    fn roles(&self) -> crate::connector::ConnectorDirection {
        crate::connector::ConnectorDirection::InboundAndOutbound
    }

    fn create<'plan>(
        &self,
        plan: &'plan crate::plan::ValidatedExecutionPlan,
        nodes: &[crate::node::Path],
    ) -> Result<
        Box<dyn crate::connector::driver::Driver + 'plan>,
        crate::connector::drivers::DriverFactoryError,
    > {
        HostedWebSocket::try_bind_client(plan, nodes, self.config)
            .map(|driver| Box::new(driver) as Box<dyn crate::connector::driver::Driver + 'plan>)
            .map_err(crate::connector::drivers::DriverFactoryError::new)
    }
}

impl crate::connector::drivers::DriverFactory for WebSocketServerFactory {
    fn name(&self) -> &str {
        "websocket-server"
    }
    fn exchange(&self) -> crate::connector::ExchangeKind {
        crate::connector::ExchangeKind::Session
    }
    fn modes(&self) -> &[&str] {
        &[]
    }

    fn contract_id(&self) -> &crate::connector::ContractId {
        &self.id
    }

    fn roles(&self) -> crate::connector::ConnectorDirection {
        crate::connector::ConnectorDirection::InboundAndOutbound
    }

    fn create<'plan>(
        &self,
        plan: &'plan crate::plan::ValidatedExecutionPlan,
        nodes: &[crate::node::Path],
    ) -> Result<
        Box<dyn crate::connector::driver::Driver + 'plan>,
        crate::connector::drivers::DriverFactoryError,
    > {
        HostedWebSocket::try_bind_server(plan, nodes, self.config)
            .map(|driver| Box::new(driver) as Box<dyn crate::connector::driver::Driver + 'plan>)
            .map_err(crate::connector::drivers::DriverFactoryError::new)
    }
}

#[derive(Debug)]
enum Work {
    Open {
        intent: SessionOpen,
        opening: Box<WebSocketClientOpening>,
        binding: WebSocketPlanBinding,
    },
    Write {
        intent: SessionWrite,
        message: WebSocketMessage,
    },
}

impl Work {
    fn bytes(&self) -> usize {
        match self {
            Self::Open { opening, .. } => opening.initial_message.len(),
            Self::Write { message, .. } => message.body.len(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Control {
    Close(SessionId),
}

#[derive(Debug)]
enum ActorCommand {
    Write(SessionWrite, WebSocketMessage),
    Close,
}

#[derive(Debug)]
enum DriverEvent {
    Proposal(Box<LiveProposal>),
    Session(SessionObservation),
}

#[derive(Debug)]
struct LiveProposal {
    proposal: PendingIngress,
    decision: oneshot::Sender<Result<AdmittedIngress, IngressRejection>>,
    _reservation: EventReservation,
}

#[derive(Default)]
struct SessionRegistry {
    active: BTreeMap<SessionId, mpsc::Sender<ActorCommand>>,
    close_before_registration: BTreeSet<SessionId>,
}

type SessionTable = Arc<Mutex<SessionRegistry>>;

async fn register_session(
    sessions: &SessionTable,
    session: SessionId,
    sender: mpsc::Sender<ActorCommand>,
) {
    let close = {
        let mut sessions = sessions.lock().await;
        sessions.active.insert(session, sender.clone());
        sessions.close_before_registration.remove(&session)
    };
    if close {
        let _ = sender.send(ActorCommand::Close).await;
    }
}

async fn session_sender(
    sessions: &SessionTable,
    session: SessionId,
) -> Option<mpsc::Sender<ActorCommand>> {
    sessions.lock().await.active.get(&session).cloned()
}

async fn request_session_close(sessions: &SessionTable, session: SessionId) {
    let sender = {
        let mut sessions = sessions.lock().await;
        let sender = sessions.active.get(&session).cloned();
        if sender.is_none() {
            sessions.close_before_registration.insert(session);
        }
        sender
    };
    if let Some(sender) = sender {
        let _ = sender.send(ActorCommand::Close).await;
    }
}

async fn forget_active_session(sessions: &SessionTable, session: SessionId) {
    let mut sessions = sessions.lock().await;
    sessions.active.remove(&session);
    sessions.close_before_registration.remove(&session);
}

#[derive(Debug)]
struct HostedWebSocket<'plan> {
    _plan: &'plan crate::plan::ValidatedExecutionPlan,
    protocol: WebSocketLabDriver,
    plans: BTreeMap<crate::node::Path, WebSocketPlanBinding>,
    engine: HostedNetworkEngine<Work, Control, DriverEvent>,
    pending: VecDeque<LiveProposal>,
    session_events: Vec<SessionObservation>,
    #[cfg(test)]
    local_addresses: Vec<std::net::SocketAddr>,
}

impl crate::connector::driver::Driver for HostedWebSocket<'_> {
    fn inbound(&mut self) -> Option<&mut dyn Ingress> {
        Some(self)
    }

    fn outbound(&mut self) -> Option<&mut dyn Outbound> {
        Some(self)
    }
}

impl Ingress for HostedWebSocket<'_> {
    fn poll_proposal(&mut self) -> Option<PendingIngress> {
        self.poll_events();
        self.pending.front().map(|live| live.proposal.clone())
    }

    fn notify_admission(
        &mut self,
        reference: IngressProposalHandle,
        decision: Result<AdmittedIngress, IngressRejection>,
    ) {
        let Some(live) = self.pending.front() else {
            return;
        };
        if live.proposal.handle != reference {
            return;
        }
        if let Some(live) = self.pending.pop_front() {
            let _ = live.decision.send(decision);
        }
    }
}

impl Outbound for HostedWebSocket<'_> {
    fn call(&mut self) -> Option<&mut dyn crate::connector::driver::OutboundCall> {
        None
    }

    fn session(&mut self) -> Option<&mut dyn Session> {
        Some(self)
    }
}

impl Session for HostedWebSocket<'_> {
    fn prepare_session_open(
        &mut self,
        connector: &crate::node::Path,
        data: &Data,
        settings: &BTreeMap<String, Value>,
    ) -> RequestPreparation {
        match self.protocol.prepare_open(connector, data, settings) {
            Ok(prepared) => RequestPreparation::Prepared(prepared),
            Err(exit) => RequestPreparation::Rejected(
                crate::connector::exchange::Completion::websocket_exit(exit),
            ),
        }
    }

    fn scheduler_session_completion(
        &mut self,
        _: &crate::node::Path,
        completion: SchedulerCompletion,
    ) -> crate::connector::exchange::Completion {
        crate::connector::exchange::Completion::websocket_exit(match completion {
            SchedulerCompletion::TimeoutExpired => WebSocketExit::TimeoutExpired,
            SchedulerCompletion::OutcomeUnknown => WebSocketExit::OutcomeUnknown,
        })
    }

    fn open_session(
        &mut self,
        intent: &SessionOpen,
        _: &crate::envelope::Envelope,
    ) -> SessionStart {
        let Some(opening) = decode_client_open(&intent.prepared) else {
            self.session_events.push(exit_event(
                intent.session_id,
                WebSocketExit::ProtocolFailed,
                Namespace::new(),
            ));
            return SessionStart::Started;
        };
        let Some(binding) = self.plans.get(&intent.connector_path).cloned() else {
            self.session_events.push(exit_event(
                intent.session_id,
                WebSocketExit::ProtocolFailed,
                Namespace::new(),
            ));
            return SessionStart::Started;
        };
        let work = Work::Open {
            intent: intent.clone(),
            opening: Box::new(opening),
            binding,
        };
        let bytes = work.bytes();
        match self.engine.try_submit(work, bytes) {
            Ok(()) => SessionStart::Started,
            Err(SubmitError::Full(_) | SubmitError::ByteBudget(_)) => SessionStart::Pending,
            Err(SubmitError::Oversized { .. }) => {
                self.session_events.push(exit_event(
                    intent.session_id,
                    WebSocketExit::MessageTooLarge {
                        limit: self
                            .plans
                            .get(&intent.connector_path)
                            .map_or(0, |binding| binding.max_message),
                    },
                    Namespace::new(),
                ));
                SessionStart::Started
            }
            Err(SubmitError::Stopped(_)) => {
                self.session_events.push(exit_event(
                    intent.session_id,
                    WebSocketExit::ConnectFailed,
                    Namespace::new(),
                ));
                SessionStart::Started
            }
        }
    }

    fn prepare_session_write(
        &mut self,
        connector: &crate::node::Path,
        _: SessionId,
        data: &Data,
        _: &BTreeMap<String, Value>,
    ) -> RequestPreparation {
        match self.protocol.prepare_message(connector, data) {
            Ok(prepared) => RequestPreparation::Prepared(prepared),
            Err(exit) => RequestPreparation::Rejected(
                crate::connector::exchange::Completion::websocket_exit(exit),
            ),
        }
    }

    fn write_session(
        &mut self,
        intent: &SessionWrite,
        _: &crate::envelope::Envelope,
    ) -> SessionStart {
        let Some(message) = decode_message(&intent.prepared) else {
            self.session_events.push(exit_event(
                intent.session_id,
                WebSocketExit::ProtocolFailed,
                Namespace::new(),
            ));
            return SessionStart::Started;
        };
        let work = Work::Write {
            intent: intent.clone(),
            message,
        };
        let bytes = work.bytes();
        match self.engine.try_submit(work, bytes) {
            Ok(()) => SessionStart::Started,
            Err(SubmitError::Full(_) | SubmitError::ByteBudget(_)) => SessionStart::Pending,
            Err(SubmitError::Oversized { .. } | SubmitError::Stopped(_)) => {
                self.session_events.push(exit_event(
                    intent.session_id,
                    WebSocketExit::TransportLost,
                    Namespace::new(),
                ));
                SessionStart::Started
            }
        }
    }

    fn close_session(&mut self, session: SessionId) -> SessionStart {
        match self.engine.try_control(Control::Close(session)) {
            Ok(()) => SessionStart::Started,
            Err(SubmitError::Full(_)) => SessionStart::Pending,
            Err(
                SubmitError::ByteBudget(_)
                | SubmitError::Oversized { .. }
                | SubmitError::Stopped(_),
            ) => SessionStart::Started,
        }
    }

    fn poll_observations(&mut self) -> Vec<SessionObservation> {
        self.poll_events();
        std::mem::take(&mut self.session_events)
    }
}

impl HostedWebSocket<'_> {
    fn poll_events(&mut self) {
        for _ in 0..MAX_EVENTS_PER_POLL {
            match self.engine.try_event() {
                EventPoll::Event(DriverEvent::Proposal(live)) => self.pending.push_back(*live),
                EventPoll::Event(DriverEvent::Session(event)) => self.session_events.push(event),
                EventPoll::Empty | EventPoll::Stopped => break,
            }
        }
    }
}

impl<'plan> HostedWebSocket<'plan> {
    fn try_bind_client(
        plan: &'plan crate::plan::ValidatedExecutionPlan,
        nodes: &[crate::node::Path],
        config: WebSocketConfig,
    ) -> Result<Self, String> {
        let id = crate::connector::ContractId::parse(
            crate::connector::stdlib::WEBSOCKET_CLIENT_CONNECTOR_ID,
        )
        .map_err(|error| error.to_string())?;
        let protocol =
            WebSocketLabDriver::try_bind(plan, nodes, &id).map_err(|error| error.to_string())?;
        let mut plans = BTreeMap::new();
        for node in nodes {
            let binding = protocol.plan_binding(node).ok_or_else(|| {
                format!("the hosted WebSocketClient plan at `{node}` is unresolved")
            })?;
            verify_binding(node, &binding, config)?;
            plans.insert(node.clone(), binding);
        }
        let limits = config.limits().map_err(|error| error.to_string())?;
        let engine =
            HostedNetworkEngine::start("rastok-websocket-client", limits, move |runtime| {
                run_client_engine(runtime, config)
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            _plan: plan,
            protocol,
            plans,
            engine,
            pending: VecDeque::new(),
            session_events: Vec::new(),
            #[cfg(test)]
            local_addresses: Vec::new(),
        })
    }

    fn try_bind_server(
        plan: &'plan crate::plan::ValidatedExecutionPlan,
        nodes: &[crate::node::Path],
        config: WebSocketConfig,
    ) -> Result<Self, String> {
        let id = crate::connector::ContractId::parse(
            crate::connector::stdlib::WEBSOCKET_SERVER_CONNECTOR_ID,
        )
        .map_err(|error| error.to_string())?;
        let protocol =
            WebSocketLabDriver::try_bind(plan, nodes, &id).map_err(|error| error.to_string())?;
        let routes = bind_server_routes(plan, nodes).map_err(|error| error.to_string())?;
        let mut plans = BTreeMap::new();
        for route in &routes {
            let binding = WebSocketPlanBinding {
                codec: route.codec.clone(),
                max_message: route.max_message,
                versions: route.versions.clone(),
            };
            verify_binding(&route.node, &binding, config)?;
            plans.insert(route.node.clone(), binding);
        }
        #[cfg(feature = "hosted-http3")]
        let h3_listeners = bind_h3_listeners(&routes, config)?;
        let listeners = bind_listeners(routes)?;
        #[cfg(test)]
        let mut local_addresses = listeners
            .iter()
            .filter_map(|binding| binding.listener.local_addr().ok())
            .collect::<Vec<_>>();
        #[cfg(all(test, feature = "hosted-http3"))]
        local_addresses.extend(
            h3_listeners
                .iter()
                .filter_map(|binding| binding.socket.local_addr().ok()),
        );
        let limits = config.limits().map_err(|error| error.to_string())?;
        let engine =
            HostedNetworkEngine::start("rastok-websocket-server", limits, move |runtime| {
                run_server_engine(
                    runtime,
                    listeners,
                    #[cfg(feature = "hosted-http3")]
                    h3_listeners,
                    config,
                )
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            _plan: plan,
            protocol,
            plans,
            engine,
            pending: VecDeque::new(),
            session_events: Vec::new(),
            #[cfg(test)]
            local_addresses,
        })
    }
}

#[cfg(test)]
impl HostedWebSocket<'_> {
    fn local_addresses(&self) -> &[std::net::SocketAddr] {
        &self.local_addresses
    }
}

fn verify_binding(
    node: &crate::node::Path,
    binding: &WebSocketPlanBinding,
    config: WebSocketConfig,
) -> Result<(), String> {
    let max_message = usize::try_from(binding.max_message)
        .map_err(|_| format!("the WebSocket message bound at `{node}` is not representable"))?;
    if max_message > config.buffered_bytes {
        return Err(format!(
            "the WebSocket node `{node}` needs {max_message} buffered bytes for one message, but this Driver supplies {}",
            config.buffered_bytes
        ));
    }
    if !binding.versions.iter().any(|version| {
        matches!(version.as_str(), "http/1.1" | "http/2")
            || (cfg!(feature = "hosted-http3") && version == "http/3")
    }) {
        return Err(format!(
            "the WebSocket node `{node}` offers no HTTP version compiled into this Driver"
        ));
    }
    crate::connector::drivers::network::websocket_codec::bounded_config(max_message)
        .map_err(|error| format!("the WebSocket node `{node}` has invalid limits: {error}"))?;
    Ok(())
}

fn exit_event(session: SessionId, exit: WebSocketExit, ext: Namespace) -> SessionObservation {
    SessionObservation::Exited {
        session,
        exit: EnvelopeDraft {
            port: crate::node::Port::conventional_exit(),
            data: Data::materialized(exit.into_value()),
            ext,
        },
    }
}

async fn run_client_engine(
    mut runtime: NetworkRuntime<Work, Control, DriverEvent>,
    config: WebSocketConfig,
) {
    let sessions: SessionTable = Arc::new(Mutex::new(SessionRegistry::default()));
    while let Some(input) = runtime.recv().await {
        match input {
            EngineInput::Work(work) => match work.into_inner() {
                Work::Open {
                    intent,
                    opening,
                    binding,
                } => {
                    let (commands, receiver) = mpsc::channel(1);
                    register_session(&sessions, intent.session_id, commands).await;
                    let events = runtime.event_sender();
                    let table = Arc::clone(&sessions);
                    tokio::spawn(async move {
                        run_client_session(
                            intent, *opening, binding, receiver, events, table, config,
                        )
                        .await;
                    });
                }
                Work::Write { intent, message } => {
                    let sender = session_sender(&sessions, intent.session_id).await;
                    if let Some(sender) = sender {
                        let _ = sender.send(ActorCommand::Write(intent, message)).await;
                    } else {
                        let _ = runtime
                            .emit(
                                DriverEvent::Session(exit_event(
                                    intent.session_id,
                                    WebSocketExit::TransportLost,
                                    Namespace::new(),
                                )),
                                0,
                            )
                            .await;
                    }
                }
            },
            EngineInput::Control(Control::Close(session)) => {
                request_session_close(&sessions, session).await;
            }
        }
    }
    let _ = config;
}

async fn run_server_engine(
    mut runtime: NetworkRuntime<Work, Control, DriverEvent>,
    listeners: Vec<ListenerBinding>,
    #[cfg(feature = "hosted-http3")] h3_listeners: Vec<H3ListenerBinding>,
    config: WebSocketConfig,
) {
    let sessions: SessionTable = Arc::new(Mutex::new(SessionRegistry::default()));
    let references = Arc::new(AtomicU64::new(1));
    for listener in listeners {
        let events = runtime.event_sender();
        let sessions = Arc::clone(&sessions);
        let references = Arc::clone(&references);
        tokio::spawn(async move {
            serve_listener(listener, events, sessions, references, config).await;
        });
    }
    #[cfg(feature = "hosted-http3")]
    for listener in h3_listeners {
        let events = runtime.event_sender();
        let sessions = Arc::clone(&sessions);
        let references = Arc::clone(&references);
        tokio::spawn(async move {
            serve_h3_listener(listener, events, sessions, references, config).await;
        });
    }
    while let Some(input) = runtime.recv().await {
        match input {
            EngineInput::Work(work) => match work.into_inner() {
                Work::Write { intent, message } => {
                    let sender = session_sender(&sessions, intent.session_id).await;
                    if let Some(sender) = sender {
                        let _ = sender.send(ActorCommand::Write(intent, message)).await;
                    }
                }
                Work::Open { intent, .. } => {
                    let _ = runtime
                        .emit(
                            DriverEvent::Session(exit_event(
                                intent.session_id,
                                WebSocketExit::ProtocolFailed,
                                Namespace::new(),
                            )),
                            0,
                        )
                        .await;
                }
            },
            EngineInput::Control(Control::Close(session)) => {
                request_session_close(&sessions, session).await;
            }
        }
    }
}

async fn run_client_session(
    intent: SessionOpen,
    opening: WebSocketClientOpening,
    binding: WebSocketPlanBinding,
    mut commands: mpsc::Receiver<ActorCommand>,
    mut events: EventSender<DriverEvent>,
    sessions: SessionTable,
    config: WebSocketConfig,
) {
    let session = intent.session_id;
    let result = connect_client(
        &intent.connector_path,
        &opening,
        &binding,
        session,
        &mut commands,
        &mut events,
        config,
    )
    .await;
    forget_active_session(&sessions, session).await;
    let (exit, ext) = match result {
        Ok((exit, ext)) => (exit, ext),
        Err(exit) => (
            exit,
            client_metadata(&opening, "unknown", None, BTreeMap::new()),
        ),
    };
    let _ = events
        .emit(DriverEvent::Session(exit_event(session, exit, ext)), 0)
        .await;
}

async fn connect_client(
    connector: &crate::node::Path,
    opening: &WebSocketClientOpening,
    binding: &WebSocketPlanBinding,
    session: SessionId,
    commands: &mut mpsc::Receiver<ActorCommand>,
    events: &mut EventSender<DriverEvent>,
    config: WebSocketConfig,
) -> Result<(WebSocketExit, Namespace), WebSocketExit> {
    #[cfg(not(feature = "hosted-http3"))]
    let _ = config;
    let Some(version) = selected_client_version(&opening.url, &binding.versions) else {
        return Err(WebSocketExit::ProtocolFailed);
    };
    match version {
        "http/2" => connect_client_h2(connector, opening, binding, session, commands, events).await,
        "http/1.1" => {
            connect_client_h1(connector, opening, binding, session, commands, events).await
        }
        #[cfg(feature = "hosted-http3")]
        "http/3" => {
            connect_client_h3(
                connector, opening, binding, session, commands, events, config,
            )
            .await
        }
        _ => Err(WebSocketExit::ProtocolFailed),
    }
}

fn selected_client_version<'a>(url: &str, versions: &'a [String]) -> Option<&'a str> {
    let uri: http::Uri = url.parse().ok()?;
    versions.iter().find_map(|version| match version.as_str() {
        // HTTP/3 WebSocket requires TLS and a linked QUIC backend. Skipping
        // an impossible physical binding observes no peer and is therefore
        // still on the safe side of the opening-visibility boundary.
        "http/3" if cfg!(feature = "hosted-http3") && uri.scheme_str() == Some("wss") => {
            Some("http/3")
        }
        "http/2" if matches!(uri.scheme_str(), Some("ws") | Some("wss")) => Some("http/2"),
        "http/1.1" if matches!(uri.scheme_str(), Some("ws") | Some("wss")) => Some("http/1.1"),
        _ => None,
    })
}

trait AsyncTransport: AsyncRead + AsyncWrite + Unpin + Send + 'static {}
impl<T> AsyncTransport for T where T: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

async fn connect_tcp(uri: &http::Uri) -> Result<tokio::net::TcpStream, WebSocketExit> {
    let host = uri.host().ok_or(WebSocketExit::ProtocolFailed)?;
    let secure = matches!(uri.scheme_str(), Some("wss") | Some("https"));
    let port = uri.port_u16().unwrap_or(if secure { 443 } else { 80 });
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| WebSocketExit::DnsFailed)?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(WebSocketExit::DnsFailed);
    }
    for address in addresses {
        if let Ok(stream) = tokio::net::TcpStream::connect(address).await {
            return Ok(stream);
        }
    }
    Err(WebSocketExit::ConnectFailed)
}

async fn client_transport(
    opening: &WebSocketClientOpening,
    uri: &http::Uri,
    alpn: &'static [u8],
) -> Result<Box<dyn AsyncTransport>, WebSocketExit> {
    let stream = connect_tcp(uri).await?;
    let secure = matches!(uri.scheme_str(), Some("wss") | Some("https"));
    if !secure {
        return Ok(Box::new(stream));
    }
    let identity = match (&opening.tls_certificate, &opening.tls_private_key) {
        (Some(certificate), Some(private_key)) => Some(super::tls::TlsIdentity {
            certificate_chain: certificate,
            private_key,
        }),
        (None, None) => None,
        _ => return Err(WebSocketExit::TlsFailed),
    };
    let mut config = super::tls::rustls_client(opening.tls_ca.as_deref(), identity)
        .map_err(|_| WebSocketExit::TlsFailed)?;
    config.alpn_protocols = vec![alpn.to_vec()];
    let server_name = rustls::pki_types::ServerName::try_from(
        uri.host().ok_or(WebSocketExit::ProtocolFailed)?.to_string(),
    )
    .map_err(|_| WebSocketExit::TlsFailed)?;
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let stream = connector
        .connect(server_name, stream)
        .await
        .map_err(|_| WebSocketExit::TlsFailed)?;
    if stream.get_ref().1.alpn_protocol() != Some(alpn) {
        return Err(WebSocketExit::ProtocolFailed);
    }
    Ok(Box::new(stream))
}

async fn handshake_step<T>(
    step: impl core::future::Future<Output = Result<T, WebSocketExit>>,
    commands: &mut mpsc::Receiver<ActorCommand>,
) -> Result<T, WebSocketExit> {
    tokio::select! {
        result = step => result,
        command = commands.recv() => match command {
            Some(ActorCommand::Close) | None => Err(WebSocketExit::Closed {
                code: 1000,
                reason: String::new(),
            }),
            Some(ActorCommand::Write(_, _)) => Err(WebSocketExit::ProtocolFailed),
        },
    }
}

fn client_headers(opening: &WebSocketClientOpening) -> Result<http::HeaderMap, WebSocketExit> {
    let mut headers = http::HeaderMap::new();
    for (name, values) in &opening.headers {
        if matches!(
            name.as_str(),
            "host"
                | "connection"
                | "upgrade"
                | "sec-websocket-key"
                | "sec-websocket-version"
                | "sec-websocket-protocol"
        ) {
            return Err(WebSocketExit::ProtocolFailed);
        }
        let name = http::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| WebSocketExit::ProtocolFailed)?;
        for value in values {
            headers.append(
                name.clone(),
                http::HeaderValue::from_bytes(value).map_err(|_| WebSocketExit::ProtocolFailed)?,
            );
        }
    }
    if let Some(token) = &opening.auth_bearer {
        headers.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| WebSocketExit::ProtocolFailed)?,
        );
    }
    if !opening.subprotocols.is_empty() {
        headers.insert(
            http::header::SEC_WEBSOCKET_PROTOCOL,
            http::HeaderValue::from_str(&opening.subprotocols.join(", "))
                .map_err(|_| WebSocketExit::ProtocolFailed)?,
        );
    }
    Ok(headers)
}

async fn connect_client_h1(
    connector: &crate::node::Path,
    opening: &WebSocketClientOpening,
    binding: &WebSocketPlanBinding,
    session: SessionId,
    commands: &mut mpsc::Receiver<ActorCommand>,
    events: &mut EventSender<DriverEvent>,
) -> Result<(WebSocketExit, Namespace), WebSocketExit> {
    use tungstenite::client::IntoClientRequest;

    let uri: http::Uri = opening
        .url
        .parse()
        .map_err(|_| WebSocketExit::ProtocolFailed)?;
    if !matches!(uri.scheme_str(), Some("ws") | Some("wss")) {
        return Err(WebSocketExit::ProtocolFailed);
    }
    let stream = handshake_step(client_transport(opening, &uri, b"http/1.1"), commands).await?;
    let mut request = opening
        .url
        .as_str()
        .into_client_request()
        .map_err(|_| WebSocketExit::ProtocolFailed)?;
    for (name, value) in client_headers(opening)? {
        if let Some(name) = name {
            request.headers_mut().append(name, value);
        }
    }
    let config = crate::connector::drivers::network::websocket_codec::bounded_config(
        usize::try_from(binding.max_message).map_err(|_| WebSocketExit::ProtocolFailed)?,
    )
    .map_err(|_| WebSocketExit::ProtocolFailed)?;
    let (socket, response) = handshake_step(
        async {
            tokio_tungstenite::client_async_with_config(request, stream, Some(config))
                .await
                .map_err(handshake_error)
        },
        commands,
    )
    .await?;
    let selected = selected_subprotocol(response.headers(), &opening.subprotocols)?;
    let headers = lossless_headers(response.headers());
    let ext = client_metadata(opening, "http/1.1", selected, headers);
    run_socket(
        socket,
        SocketSession {
            role: Role::Client,
            connector,
            session,
            codec: &binding.codec,
            max_message: binding.max_message,
            initial_message: &opening.initial_message,
            ext,
        },
        commands,
        events,
    )
    .await
}

async fn connect_client_h2(
    connector: &crate::node::Path,
    opening: &WebSocketClientOpening,
    binding: &WebSocketPlanBinding,
    session: SessionId,
    commands: &mut mpsc::Receiver<ActorCommand>,
    events: &mut EventSender<DriverEvent>,
) -> Result<(WebSocketExit, Namespace), WebSocketExit> {
    let uri: http::Uri = opening
        .url
        .parse()
        .map_err(|_| WebSocketExit::ProtocolFailed)?;
    if !matches!(uri.scheme_str(), Some("ws") | Some("wss")) {
        return Err(WebSocketExit::ProtocolFailed);
    }
    let stream = handshake_step(client_transport(opening, &uri, b"h2"), commands).await?;
    let builder = hyper::client::conn::http2::Builder::new(TokioExecutor::new());
    let (mut sender, connection) = handshake_step(
        async {
            builder
                .handshake::<_, Empty<Bytes>>(TokioIo::new(stream))
                .await
                .map_err(|_| WebSocketExit::ProtocolFailed)
        },
        commands,
    )
    .await?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let mut request = Request::builder()
        .method(http::Method::CONNECT)
        .uri(http_uri(&uri)?)
        .version(Version::HTTP_2)
        .body(Empty::new())
        .map_err(|_| WebSocketExit::ProtocolFailed)?;
    request
        .extensions_mut()
        .insert(hyper::ext::Protocol::from_static("websocket"));
    *request.headers_mut() = client_headers(opening)?;
    request.headers_mut().insert(
        http::header::SEC_WEBSOCKET_VERSION,
        http::HeaderValue::from_static("13"),
    );
    let mut response = handshake_step(
        async {
            sender
                .send_request(request)
                .await
                .map_err(|_| WebSocketExit::ProtocolFailed)
        },
        commands,
    )
    .await?;
    if response.status() != StatusCode::OK {
        return Err(WebSocketExit::HandshakeRejected {
            status: response.status().as_u16(),
        });
    }
    let selected = selected_subprotocol(response.headers(), &opening.subprotocols)?;
    let headers = lossless_headers(response.headers());
    let upgraded = handshake_step(
        async {
            hyper::upgrade::on(&mut response)
                .await
                .map_err(|_| WebSocketExit::ProtocolFailed)
        },
        commands,
    )
    .await?;
    let config = crate::connector::drivers::network::websocket_codec::bounded_config(
        usize::try_from(binding.max_message).map_err(|_| WebSocketExit::ProtocolFailed)?,
    )
    .map_err(|_| WebSocketExit::ProtocolFailed)?;
    let socket =
        WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Client, Some(config)).await;
    let ext = client_metadata(opening, "http/2", selected, headers);
    run_socket(
        socket,
        SocketSession {
            role: Role::Client,
            connector,
            session,
            codec: &binding.codec,
            max_message: binding.max_message,
            initial_message: &opening.initial_message,
            ext,
        },
        commands,
        events,
    )
    .await
}

#[cfg(feature = "hosted-http3")]
#[derive(Default)]
struct H3FrameIo {
    incoming: VecDeque<u8>,
    outgoing: Vec<u8>,
}

#[cfg(feature = "hosted-http3")]
impl H3FrameIo {
    fn feed(&mut self, bytes: &[u8]) {
        self.incoming.extend(bytes.iter().copied());
    }

    fn take_outgoing(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.outgoing)
    }
}

#[cfg(feature = "hosted-http3")]
impl Read for H3FrameIo {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if self.incoming.is_empty() {
            return Err(std::io::Error::from(std::io::ErrorKind::WouldBlock));
        }
        let count = output.len().min(self.incoming.len());
        for slot in &mut output[..count] {
            *slot = self.incoming.pop_front().unwrap_or_default();
        }
        Ok(count)
    }
}

#[cfg(feature = "hosted-http3")]
impl Write for H3FrameIo {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.outgoing.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(feature = "hosted-http3")]
async fn connect_client_h3(
    connector: &crate::node::Path,
    opening: &WebSocketClientOpening,
    binding: &WebSocketPlanBinding,
    session: SessionId,
    commands: &mut mpsc::Receiver<ActorCommand>,
    events: &mut EventSender<DriverEvent>,
    config: WebSocketConfig,
) -> Result<(WebSocketExit, Namespace), WebSocketExit> {
    use quiche::h3::NameValue as _;

    let uri: http::Uri = opening
        .url
        .parse()
        .map_err(|_| WebSocketExit::ProtocolFailed)?;
    if uri.scheme_str() != Some("wss") {
        return Err(WebSocketExit::ProtocolFailed);
    }
    let identity = match (&opening.tls_certificate, &opening.tls_private_key) {
        (Some(certificate), Some(private_key)) => Some(super::tls::TlsIdentity {
            certificate_chain: certificate,
            private_key,
        }),
        (None, None) => None,
        _ => return Err(WebSocketExit::TlsFailed),
    };
    let limits = config.limits().map_err(|_| WebSocketExit::ProtocolFailed)?;
    let mut quic_config = super::tls::quiche_client(opening.tls_ca.as_deref(), identity, limits)
        .map_err(|_| WebSocketExit::TlsFailed)?;
    let host = uri.host().ok_or(WebSocketExit::ProtocolFailed)?;
    let port = uri.port_u16().unwrap_or(443);
    let addresses = handshake_step(
        async {
            tokio::net::lookup_host((host, port))
                .await
                .map(|addresses| addresses.collect::<Vec<_>>())
                .map_err(|_| WebSocketExit::DnsFailed)
        },
        commands,
    )
    .await?;
    let remote = addresses
        .iter()
        .copied()
        .find(std::net::SocketAddr::is_ipv4)
        .or_else(|| addresses.first().copied())
        .ok_or(WebSocketExit::DnsFailed)?;
    let socket = tokio::net::UdpSocket::bind(if remote.is_ipv6() {
        "[::]:0"
    } else {
        "0.0.0.0:0"
    })
    .await
    .map_err(|_| WebSocketExit::ConnectFailed)?;
    let local = socket
        .local_addr()
        .map_err(|_| WebSocketExit::ConnectFailed)?;
    let session_id = session.id();
    let scid = quiche::ConnectionId::from_ref(session_id.as_bytes());
    let mut connection = quiche::connect(Some(host), &scid, local, remote, &mut quic_config)
        .map_err(|_| WebSocketExit::TlsFailed)?;
    while !connection.is_established() {
        if connection.is_closed() {
            return Err(WebSocketExit::TlsFailed);
        }
        handshake_step(h3_transport_step(&socket, &mut connection), commands).await?;
    }
    if connection.application_proto() != b"h3" || connection.peer_cert().is_none() {
        return Err(WebSocketExit::TlsFailed);
    }
    let mut h3_config = quiche::h3::Config::new().map_err(|_| WebSocketExit::ProtocolFailed)?;
    h3_config.set_max_field_section_size(config.max_header_bytes as u64);
    h3_config.enable_extended_connect(true);
    let mut h3 = quiche::h3::Connection::with_transport(&mut connection, &h3_config)
        .map_err(|_| WebSocketExit::ProtocolFailed)?;

    while !h3.extended_connect_enabled_by_peer() {
        drain_h3_control(&mut connection, &mut h3)?;
        if h3.extended_connect_enabled_by_peer() {
            break;
        }
        handshake_step(h3_transport_step(&socket, &mut connection), commands).await?;
    }
    let headers = h3_client_headers(opening, &uri)?;
    let stream = h3
        .send_request(&mut connection, &headers, false)
        .map_err(|_| WebSocketExit::ProtocolFailed)?;
    let mut response_status = None;
    let mut response_headers = BTreeMap::new();
    while response_status.is_none() {
        match h3.poll(&mut connection) {
            Ok((incoming, quiche::h3::Event::Headers { list, .. })) if incoming == stream => {
                for header in list {
                    match header.name() {
                        b":status" => {
                            response_status = std::str::from_utf8(header.value())
                                .ok()
                                .and_then(|value| value.parse::<u16>().ok());
                        }
                        name if !name.starts_with(b":") => {
                            let name = std::str::from_utf8(name)
                                .map_err(|_| WebSocketExit::ProtocolFailed)?;
                            response_headers
                                .entry(name.to_string())
                                .or_insert_with(Vec::new)
                                .push(header.value().to_vec());
                        }
                        _ => {}
                    }
                }
            }
            Ok((incoming, _)) if incoming != stream => {
                return Err(WebSocketExit::ProtocolFailed);
            }
            Ok((_, quiche::h3::Event::Reset(_)))
            | Ok((_, quiche::h3::Event::GoAway))
            | Err(quiche::h3::Error::TransportError(_)) => {
                return Err(WebSocketExit::ProtocolFailed);
            }
            Err(quiche::h3::Error::Done) => {
                handshake_step(h3_transport_step(&socket, &mut connection), commands).await?
            }
            Ok(_) => {}
            Err(_) => return Err(WebSocketExit::ProtocolFailed),
        }
    }
    let status = response_status.ok_or(WebSocketExit::ProtocolFailed)?;
    if status != 200 {
        return Err(WebSocketExit::HandshakeRejected { status });
    }
    let selected = response_headers
        .get("sec-websocket-protocol")
        .and_then(|values| values.first())
        .and_then(|value| std::str::from_utf8(value).ok())
        .map(str::to_string);
    if selected
        .as_ref()
        .is_some_and(|value| !opening.subprotocols.contains(value))
    {
        return Err(WebSocketExit::ProtocolFailed);
    }
    let ext = client_metadata(opening, "http/3", selected, response_headers);
    run_h3_client_socket(
        connector,
        session,
        &binding.codec,
        binding.max_message,
        &opening.initial_message,
        commands,
        events,
        ext,
        socket,
        connection,
        h3,
        stream,
    )
    .await
}

#[cfg(feature = "hosted-http3")]
fn h3_client_headers(
    opening: &WebSocketClientOpening,
    uri: &http::Uri,
) -> Result<Vec<quiche::h3::Header>, WebSocketExit> {
    let authority = uri
        .authority()
        .ok_or(WebSocketExit::ProtocolFailed)?
        .as_str();
    let path = uri
        .path_and_query()
        .map_or("/", http::uri::PathAndQuery::as_str);
    let mut headers = vec![
        quiche::h3::Header::new(b":method", b"CONNECT"),
        quiche::h3::Header::new(b":protocol", b"websocket"),
        quiche::h3::Header::new(b":scheme", b"https"),
        quiche::h3::Header::new(b":authority", authority.as_bytes()),
        quiche::h3::Header::new(b":path", path.as_bytes()),
        quiche::h3::Header::new(b"sec-websocket-version", b"13"),
    ];
    for (name, values) in &opening.headers {
        if name.starts_with(':') {
            return Err(WebSocketExit::ProtocolFailed);
        }
        for value in values {
            headers.push(quiche::h3::Header::new(name.as_bytes(), value));
        }
    }
    if let Some(token) = &opening.auth_bearer {
        headers.push(quiche::h3::Header::new(
            b"authorization",
            format!("Bearer {token}").as_bytes(),
        ));
    }
    if !opening.subprotocols.is_empty() {
        headers.push(quiche::h3::Header::new(
            b"sec-websocket-protocol",
            opening.subprotocols.join(", ").as_bytes(),
        ));
    }
    Ok(headers)
}

#[cfg(feature = "hosted-http3")]
fn drain_h3_control(
    connection: &mut quiche::Connection,
    h3: &mut quiche::h3::Connection,
) -> Result<(), WebSocketExit> {
    loop {
        match h3.poll(connection) {
            Err(quiche::h3::Error::Done) => return Ok(()),
            Ok((_, quiche::h3::Event::PriorityUpdate)) => {}
            Ok((_, quiche::h3::Event::GoAway | quiche::h3::Event::Reset(_))) | Err(_) => {
                return Err(WebSocketExit::ProtocolFailed);
            }
            Ok(_) => {}
        }
    }
}

#[cfg(feature = "hosted-http3")]
#[allow(clippy::too_many_arguments)]
async fn run_h3_client_socket(
    connector: &crate::node::Path,
    session: SessionId,
    codec: &str,
    max_message: u64,
    initial_message: &[u8],
    commands: &mut mpsc::Receiver<ActorCommand>,
    events: &mut EventSender<DriverEvent>,
    ext: Namespace,
    socket: tokio::net::UdpSocket,
    mut connection: quiche::Connection,
    mut h3: quiche::h3::Connection,
    stream: u64,
) -> Result<(WebSocketExit, Namespace), WebSocketExit> {
    let config = crate::connector::drivers::network::websocket_codec::bounded_config(
        usize::try_from(max_message).map_err(|_| WebSocketExit::ProtocolFailed)?,
    )
    .map_err(|_| WebSocketExit::ProtocolFailed)?;
    let mut websocket =
        tungstenite::WebSocket::from_raw_socket(H3FrameIo::default(), Role::Client, Some(config));
    websocket
        .send(outbound_message(codec, initial_message.to_vec())?)
        .map_err(|error| socket_error(error, max_message))?;
    websocket
        .flush()
        .map_err(|error| socket_error(error, max_message))?;
    let mut outgoing = websocket.get_mut().take_outgoing();
    let mut offset = 0usize;
    let mut opening = true;
    loop {
        while offset < outgoing.len() {
            match h3.send_body(&mut connection, stream, &outgoing[offset..], false) {
                Ok(written) => offset = offset.saturating_add(written),
                Err(quiche::h3::Error::Done) => break,
                Err(_) => return Ok((WebSocketExit::TransportLost, ext)),
            }
        }
        if offset == outgoing.len() {
            outgoing.clear();
            offset = 0;
            if opening {
                opening = false;
                events
                    .emit(DriverEvent::Session(SessionObservation::Opened(session)), 0)
                    .await
                    .map_err(|_| WebSocketExit::TransportLost)?;
            }
        }
        flush_quiche(&socket, &mut connection).await?;
        let timeout = connection
            .timeout()
            .unwrap_or(std::time::Duration::from_millis(100));
        let mut datagram = vec![0u8; 65_535];
        tokio::select! {
            command = commands.recv(), if outgoing.is_empty() => match command {
                Some(ActorCommand::Write(intent, message)) => {
                    websocket.send(outbound_message(&message.codec, message.body)?)
                        .map_err(|error| socket_error(error, max_message))?;
                    websocket.flush().map_err(|error| socket_error(error, max_message))?;
                    outgoing = websocket.get_mut().take_outgoing();
                    events.emit(DriverEvent::Session(SessionObservation::WriteSettled {
                        session,
                        write: intent.write_ref,
                    }), 0).await.map_err(|_| WebSocketExit::TransportLost)?;
                }
                Some(ActorCommand::Close) | None => {
                    let _ = websocket.close(None);
                    let bytes = websocket.get_mut().take_outgoing();
                    let _ = h3.send_body(&mut connection, stream, &bytes, true);
                    let _ = flush_quiche(&socket, &mut connection).await;
                    return Ok((WebSocketExit::Closed { code: 1000, reason: String::new() }, ext));
                }
            },
            received = socket.recv_from(&mut datagram) => {
                let (length, from) = received.map_err(|_| WebSocketExit::TransportLost)?;
                let to = socket.local_addr().map_err(|_| WebSocketExit::TransportLost)?;
                connection.recv(&mut datagram[..length], quiche::RecvInfo { from, to })
                    .map_err(|_| WebSocketExit::TransportLost)?;
            }
            () = tokio::time::sleep(timeout) => connection.on_timeout(),
        }
        loop {
            match h3.poll(&mut connection) {
                Ok((incoming, quiche::h3::Event::Data)) if incoming == stream => {
                    let mut chunk = [0u8; 16 * 1024];
                    loop {
                        match h3.recv_body(&mut connection, stream, &mut chunk) {
                            Ok(read) if read > 0 => websocket.get_mut().feed(&chunk[..read]),
                            Ok(_) | Err(quiche::h3::Error::Done) => break,
                            Err(_) => return Ok((WebSocketExit::TransportLost, ext)),
                        }
                    }
                    if let Some(exit) = drain_h3_messages(
                        connector,
                        session,
                        codec,
                        max_message,
                        &mut websocket,
                        events,
                        &ext,
                    )
                    .await?
                    {
                        return Ok((exit, ext));
                    }
                    let control = websocket.get_mut().take_outgoing();
                    if !control.is_empty() && outgoing.is_empty() {
                        outgoing = control;
                    }
                }
                Ok((incoming, quiche::h3::Event::Finished | quiche::h3::Event::Reset(_)))
                    if incoming == stream =>
                {
                    return Ok((WebSocketExit::TransportLost, ext));
                }
                Ok((_, quiche::h3::Event::GoAway)) => {
                    return Ok((WebSocketExit::TransportLost, ext));
                }
                Err(quiche::h3::Error::Done) => break,
                Ok(_) => {}
                Err(_) => return Ok((WebSocketExit::ProtocolFailed, ext)),
            }
        }
        if connection.is_closed() {
            return Ok((WebSocketExit::TransportLost, ext));
        }
    }
}

#[cfg(feature = "hosted-http3")]
async fn drain_h3_messages(
    connector: &crate::node::Path,
    session: SessionId,
    codec: &str,
    max_message: u64,
    websocket: &mut tungstenite::WebSocket<H3FrameIo>,
    events: &mut EventSender<DriverEvent>,
    ext: &Namespace,
) -> Result<Option<WebSocketExit>, WebSocketExit> {
    loop {
        match websocket.read() {
            Ok(Message::Text(text)) if codec != "bytes" => {
                let bytes = text.as_bytes().to_vec();
                let data = decode_inbound(codec, bytes.clone())?;
                propose_message(connector, session, data, ext.clone(), bytes.len(), events).await?;
            }
            Ok(Message::Binary(bytes)) if codec == "bytes" => {
                let bytes = bytes.to_vec();
                propose_message(
                    connector,
                    session,
                    Data::materialized(Value::Bytes(bytes.clone())),
                    ext.clone(),
                    bytes.len(),
                    events,
                )
                .await?;
            }
            Ok(Message::Text(_) | Message::Binary(_)) => {
                return Ok(Some(WebSocketExit::DecodeFailed));
            }
            Ok(Message::Close(frame)) => return Ok(Some(close_exit(frame))),
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {}
            Err(tungstenite::Error::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock =>
            {
                return Ok(None);
            }
            Err(error) => return Ok(Some(socket_error(error, max_message))),
        }
    }
}

#[cfg(feature = "hosted-http3")]
async fn h3_transport_step(
    socket: &tokio::net::UdpSocket,
    connection: &mut quiche::Connection,
) -> Result<(), WebSocketExit> {
    flush_quiche(socket, connection).await?;
    let timeout = connection.timeout();
    let mut datagram = vec![0u8; 65_535];
    let received = match timeout {
        Some(timeout) => tokio::select! {
            received = socket.recv_from(&mut datagram) => Some(received),
            () = tokio::time::sleep(timeout) => None,
        },
        None => Some(socket.recv_from(&mut datagram).await),
    };
    if let Some(received) = received {
        let (length, from) = received.map_err(|_| WebSocketExit::TransportLost)?;
        let to = socket
            .local_addr()
            .map_err(|_| WebSocketExit::TransportLost)?;
        connection
            .recv(&mut datagram[..length], quiche::RecvInfo { from, to })
            .map_err(|_| WebSocketExit::TransportLost)?;
    } else {
        connection.on_timeout();
    }
    Ok(())
}

#[cfg(feature = "hosted-http3")]
async fn flush_quiche(
    socket: &tokio::net::UdpSocket,
    connection: &mut quiche::Connection,
) -> Result<(), WebSocketExit> {
    let mut output = vec![0u8; 65_535];
    loop {
        match connection.send(&mut output) {
            Ok((written, info)) => {
                if socket
                    .send_to(&output[..written], info.to)
                    .await
                    .map_err(|_| WebSocketExit::TransportLost)?
                    != written
                {
                    return Err(WebSocketExit::TransportLost);
                }
            }
            Err(quiche::Error::Done) => return Ok(()),
            Err(_) => return Err(WebSocketExit::TransportLost),
        }
    }
}

fn http_uri(uri: &http::Uri) -> Result<http::Uri, WebSocketExit> {
    let scheme = match uri.scheme_str() {
        Some("ws") => "http",
        Some("wss") => "https",
        _ => return Err(WebSocketExit::ProtocolFailed),
    };
    let authority = uri.authority().ok_or(WebSocketExit::ProtocolFailed)?;
    let path = uri
        .path_and_query()
        .map_or("/", http::uri::PathAndQuery::as_str);
    format!("{scheme}://{authority}{path}")
        .parse()
        .map_err(|_| WebSocketExit::ProtocolFailed)
}

fn handshake_error(error: tungstenite::Error) -> WebSocketExit {
    match error {
        tungstenite::Error::Http(response) => WebSocketExit::HandshakeRejected {
            status: response.status().as_u16(),
        },
        tungstenite::Error::Tls(_) => WebSocketExit::TlsFailed,
        tungstenite::Error::Io(_) => WebSocketExit::ConnectFailed,
        _ => WebSocketExit::ProtocolFailed,
    }
}

struct SocketSession<'a> {
    role: Role,
    connector: &'a crate::node::Path,
    session: SessionId,
    codec: &'a str,
    max_message: u64,
    initial_message: &'a [u8],
    ext: Namespace,
}

async fn run_socket<S>(
    mut socket: WebSocketStream<S>,
    context: SocketSession<'_>,
    commands: &mut mpsc::Receiver<ActorCommand>,
    events: &mut EventSender<DriverEvent>,
) -> Result<(WebSocketExit, Namespace), WebSocketExit>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let SocketSession {
        role,
        connector,
        session,
        codec,
        max_message,
        initial_message,
        ext,
    } = context;
    if role == Role::Client {
        socket
            .send(outbound_message(codec, initial_message.to_vec())?)
            .await
            .map_err(|error| socket_error(error, max_message))?;
        socket
            .flush()
            .await
            .map_err(|error| socket_error(error, max_message))?;
        events
            .emit(DriverEvent::Session(SessionObservation::Opened(session)), 0)
            .await
            .map_err(|_| WebSocketExit::TransportLost)?;
    }
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(ActorCommand::Write(intent, message)) => {
                    socket.send(outbound_message(&message.codec, message.body)?).await
                        .map_err(|error| socket_error(error, max_message))?;
                    socket
                        .flush()
                        .await
                        .map_err(|error| socket_error(error, max_message))?;
                    events.emit(DriverEvent::Session(SessionObservation::WriteSettled {
                        session,
                        write: intent.write_ref,
                    }), 0).await.map_err(|_| WebSocketExit::TransportLost)?;
                }
                Some(ActorCommand::Close) | None => {
                    let _ = socket.close(None).await;
                    return Ok((WebSocketExit::Closed { code: 1000, reason: String::new() }, ext));
                }
            },
            message = socket.next() => match message {
                Some(Ok(Message::Text(text))) if codec != "bytes" => {
                    let bytes = text.as_bytes().to_vec();
                    let data = decode_inbound(codec, bytes.clone())?;
                    propose_message(connector, session, data, ext.clone(), bytes.len(), events)
                        .await?;
                }
                Some(Ok(Message::Binary(bytes))) if codec == "bytes" => {
                    let bytes = bytes.to_vec();
                    let data = Data::materialized(Value::Bytes(bytes.clone()));
                    propose_message(connector, session, data, ext.clone(), bytes.len(), events)
                        .await?;
                }
                Some(Ok(Message::Text(_)) | Ok(Message::Binary(_))) => {
                    return Ok((WebSocketExit::DecodeFailed, ext));
                }
                Some(Ok(Message::Ping(payload))) => {
                    socket
                        .send(Message::Pong(payload))
                        .await
                        .map_err(|error| socket_error(error, max_message))?;
                    socket
                        .flush()
                        .await
                        .map_err(|error| socket_error(error, max_message))?;
                }
                Some(Ok(Message::Pong(_))) | Some(Ok(Message::Frame(_))) => {}
                Some(Ok(Message::Close(frame))) => {
                    let exit = close_exit(frame);
                    // Tungstenite queued the mandatory close reply while
                    // decoding the peer frame. Flushing that reply, rather
                    // than starting a second close, lets independent peers
                    // observe the clean handshake before this task drops the
                    // transport.
                    let _ = socket.flush().await;
                    return Ok((exit, ext));
                }
                Some(Err(error)) => return Ok((socket_error(error, max_message), ext)),
                None => return Ok((WebSocketExit::TransportLost, ext)),
            }
        }
    }
}

fn outbound_message(codec: &str, bytes: Vec<u8>) -> Result<Message, WebSocketExit> {
    match codec {
        "bytes" => Ok(Message::binary(bytes)),
        "json" | "text" => String::from_utf8(bytes)
            .map(Message::text)
            .map_err(|_| WebSocketExit::DecodeFailed),
        _ => Err(WebSocketExit::ProtocolFailed),
    }
}

fn decode_inbound(codec: &str, bytes: Vec<u8>) -> Result<Data, WebSocketExit> {
    crate::connector::drivers::http_shared::decode_body(codec, bytes)
        .ok_or(WebSocketExit::DecodeFailed)
}

fn socket_error(error: tungstenite::Error, max_message: u64) -> WebSocketExit {
    match error {
        tungstenite::Error::Capacity(_) => WebSocketExit::MessageTooLarge { limit: max_message },
        tungstenite::Error::Protocol(
            tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
        ) => WebSocketExit::TransportLost,
        tungstenite::Error::Protocol(_) | tungstenite::Error::Utf8(_) => {
            WebSocketExit::ProtocolFailed
        }
        tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed => {
            WebSocketExit::Closed {
                code: 1000,
                reason: String::new(),
            }
        }
        _ => WebSocketExit::TransportLost,
    }
}

fn close_exit(frame: Option<CloseFrame>) -> WebSocketExit {
    frame.map_or(
        WebSocketExit::Closed {
            code: 1005,
            reason: String::new(),
        },
        |frame| WebSocketExit::Closed {
            code: frame.code.into(),
            reason: frame.reason.to_string(),
        },
    )
}

async fn propose_message(
    connector: &crate::node::Path,
    session: SessionId,
    data: Data,
    ext: Namespace,
    bytes: usize,
    events: &mut EventSender<DriverEvent>,
) -> Result<(), WebSocketExit> {
    let reservation = events.reserve(bytes).await.map_err(|error| match error {
        ReserveError::Oversized { limit, .. } => WebSocketExit::MessageTooLarge {
            limit: limit as u64,
        },
        ReserveError::Stopped => WebSocketExit::TransportLost,
    })?;
    let reference = next_proposal_ref()?;
    let (decision, accepted) = oneshot::channel();
    let proposal = PendingIngress {
        handle: reference,
        proposal: IngressProposal {
            provenance: IngressProvenance::single(connector.clone()),
            draft: EnvelopeDraft {
                port: crate::node::Port::conventional_out(),
                data,
                ext,
            },
            sequence: IngressSequence::Next,
            session: Some(InboundSession::Message(session)),
        },
    };
    events
        .emit(
            DriverEvent::Proposal(Box::new(LiveProposal {
                proposal,
                decision,
                _reservation: reservation,
            })),
            0,
        )
        .await
        .map_err(|_| WebSocketExit::TransportLost)?;
    match accepted.await {
        Ok(Ok(AdmittedIngress {
            exchange: Some(InboundExchangeId::Session(accepted)),
            ..
        })) if accepted == session => Ok(()),
        _ => Err(WebSocketExit::ProtocolFailed),
    }
}

static NEXT_PROPOSAL: AtomicU64 = AtomicU64::new(1);

fn next_proposal_ref() -> Result<IngressProposalHandle, WebSocketExit> {
    let value = NEXT_PROPOSAL.fetch_add(1, Ordering::Relaxed);
    (value != u64::MAX)
        .then_some(IngressProposalHandle::new(value))
        .ok_or(WebSocketExit::TransportLost)
}

#[derive(Debug)]
struct ListenerBinding {
    listener: TcpListener,
    routes: Arc<Vec<WebSocketServerBinding>>,
    tls: Option<Arc<rustls::ServerConfig>>,
}

#[cfg(feature = "hosted-http3")]
#[derive(Debug)]
struct H3ListenerBinding {
    socket: std::net::UdpSocket,
    routes: Arc<Vec<WebSocketServerBinding>>,
    certificate: Vec<u8>,
    private_key: Vec<u8>,
}

#[cfg(feature = "hosted-http3")]
fn bind_h3_listeners(
    routes: &[WebSocketServerBinding],
    config: WebSocketConfig,
) -> Result<Vec<H3ListenerBinding>, String> {
    let mut groups = BTreeMap::<String, Vec<WebSocketServerBinding>>::new();
    for route in routes {
        if route.versions.iter().any(|version| version == "http/3") {
            groups
                .entry(route.listen.clone())
                .or_default()
                .push(route.clone());
        }
    }
    groups
        .into_iter()
        .map(|(listen, routes)| {
            let first = routes.first().ok_or_else(|| {
                format!("the WebSocketServer HTTP/3 listener `{listen}` has no routes")
            })?;
            let (Some(certificate), Some(private_key)) =
                (&first.tls_certificate, &first.tls_private_key)
            else {
                return Err(format!(
                    "the WebSocketServer HTTP/3 listener `{listen}` requires a TLS identity"
                ));
            };
            if routes.iter().any(|route| {
                route.tls_certificate.as_ref() != Some(certificate)
                    || route.tls_private_key.as_ref() != Some(private_key)
            }) {
                return Err(format!(
                    "WebSocketServer HTTP/3 routes sharing `{listen}` must use one TLS identity"
                ));
            }
            let socket = std::net::UdpSocket::bind(&listen).map_err(|error| {
                format!(
                    "the WebSocketServer HTTP/3 listener `{listen}` cannot bind: {error}"
                )
            })?;
            socket.set_nonblocking(true).map_err(|error| {
                format!(
                    "the WebSocketServer HTTP/3 listener `{listen}` is not async: {error}"
                )
            })?;
            let max_message = routes
                .iter()
                .map(|route| route.max_message)
                .max()
                .unwrap_or_default();
            if usize::try_from(max_message).map_or(true, |value| value > config.buffered_bytes) {
                return Err(format!(
                    "the WebSocketServer HTTP/3 route at `{listen}` exceeds the physical byte budget"
                ));
            }
            let certificate = certificate.clone();
            let private_key = private_key.clone();
            Ok(H3ListenerBinding {
                socket,
                routes: Arc::new(routes),
                certificate,
                private_key,
            })
        })
        .collect()
}

#[cfg(feature = "hosted-http3")]
enum H3HandshakeState {
    Pending(oneshot::Receiver<Result<AdmittedIngress, IngressRejection>>),
    Accepted(SessionId),
    Rejected,
}

#[cfg(feature = "hosted-http3")]
struct H3PendingHandshake {
    state: H3HandshakeState,
    ext: Namespace,
    route: WebSocketServerBinding,
    _reservation: EventReservation,
}

#[cfg(feature = "hosted-http3")]
struct H3LiveSession {
    commands: mpsc::Receiver<ActorCommand>,
    ext: Namespace,
    outgoing: Vec<u8>,
    outgoing_offset: usize,
    pending_decision: Option<(
        oneshot::Receiver<Result<AdmittedIngress, IngressRejection>>,
        EventReservation,
    )>,
    route: WebSocketServerBinding,
    session: SessionId,
    websocket: tungstenite::WebSocket<H3FrameIo>,
}

#[cfg(feature = "hosted-http3")]
struct H3Peer {
    connection: quiche::Connection,
    h3: Option<quiche::h3::Connection>,
    pending: BTreeMap<u64, H3PendingHandshake>,
    sessions: BTreeMap<u64, H3LiveSession>,
}

#[cfg(feature = "hosted-http3")]
async fn serve_h3_listener(
    binding: H3ListenerBinding,
    mut events: EventSender<DriverEvent>,
    sessions: SessionTable,
    references: Arc<AtomicU64>,
    config: WebSocketConfig,
) {
    let Ok(socket) = tokio::net::UdpSocket::from_std(binding.socket) else {
        return;
    };
    let Ok(local) = socket.local_addr() else {
        return;
    };
    let limits = match config.limits() {
        Ok(limits) => limits,
        Err(_) => return,
    };
    let identity = super::tls::TlsIdentity {
        certificate_chain: &binding.certificate,
        private_key: &binding.private_key,
    };
    let mut quic_config = match super::tls::quiche_server(identity, limits) {
        Ok(config) => config,
        Err(_) => return,
    };
    let mut h3_config = match quiche::h3::Config::new() {
        Ok(config) => config,
        Err(_) => return,
    };
    h3_config.set_max_field_section_size(config.max_header_bytes as u64);
    h3_config.enable_extended_connect(true);
    let mut peers = BTreeMap::<Vec<u8>, H3Peer>::new();
    let mut packet = vec![0u8; 65_535];
    let mut output = vec![0u8; 65_535];

    loop {
        for peer in peers.values_mut() {
            if progress_h3_peer(peer, &binding.routes, &mut events, &sessions, &references)
                .await
                .is_err()
            {
                let _ = peer.connection.close(true, 0x100, b"websocket failed");
            }
            flush_h3_peer(&socket, peer, &mut output).await;
        }
        peers.retain(|_, peer| {
            !peer.connection.is_closed() || !peer.pending.is_empty() || !peer.sessions.is_empty()
        });
        let timeout = peers
            .values()
            .filter_map(|peer| peer.connection.timeout())
            .min()
            .unwrap_or(std::time::Duration::from_millis(10))
            .min(std::time::Duration::from_millis(10));
        tokio::select! {
            received = socket.recv_from(&mut packet) => {
                let Ok((length, from)) = received else { return; };
                let bytes = &mut packet[..length];
                let Ok(header) = quiche::Header::from_slice(bytes, quiche::MAX_CONN_ID_LEN) else {
                    continue;
                };
                let derived = h3_connection_id(&header.dcid, from);
                let key = if peers.contains_key(header.dcid.as_ref()) {
                    header.dcid.as_ref().to_vec()
                } else if peers.contains_key(&derived) {
                    derived.clone()
                } else {
                    if header.ty != quiche::Type::Initial || peers.len() >= config.max_sessions {
                        continue;
                    }
                    if !quiche::version_is_supported(header.version) {
                        if let Ok(written) = quiche::negotiate_version(&header.scid, &header.dcid, &mut output) {
                            let _ = socket.send_to(&output[..written], from).await;
                        }
                        continue;
                    }
                    let scid = quiche::ConnectionId::from_ref(&derived);
                    let Ok(connection) = quiche::accept(
                        &scid,
                        Some(&header.dcid),
                        local,
                        from,
                        &mut quic_config,
                    ) else {
                        continue;
                    };
                    peers.insert(derived.clone(), H3Peer {
                        connection,
                        h3: None,
                        pending: BTreeMap::new(),
                        sessions: BTreeMap::new(),
                    });
                    derived
                };
                let Some(peer) = peers.get_mut(&key) else { continue; };
                if peer.connection.recv(bytes, quiche::RecvInfo { from, to: local }).is_err() {
                    continue;
                }
                if peer.connection.is_established() && peer.h3.is_none() {
                    peer.h3 = quiche::h3::Connection::with_transport(
                        &mut peer.connection,
                        &h3_config,
                    ).ok();
                }
            }
            () = tokio::time::sleep(timeout) => {
                for peer in peers.values_mut() {
                    peer.connection.on_timeout();
                }
            }
        }
    }
}

#[cfg(feature = "hosted-http3")]
fn h3_connection_id(dcid: &quiche::ConnectionId<'_>, peer: std::net::SocketAddr) -> Vec<u8> {
    let mut material = Vec::with_capacity(dcid.len() + 18);
    material.extend_from_slice(dcid.as_ref());
    match peer.ip() {
        std::net::IpAddr::V4(address) => material.extend_from_slice(&address.octets()),
        std::net::IpAddr::V6(address) => material.extend_from_slice(&address.octets()),
    }
    material.extend_from_slice(&peer.port().to_be_bytes());
    blake3::hash(&material).as_bytes()[..quiche::MAX_CONN_ID_LEN].to_vec()
}

#[cfg(feature = "hosted-http3")]
async fn progress_h3_peer(
    peer: &mut H3Peer,
    routes: &[WebSocketServerBinding],
    events: &mut EventSender<DriverEvent>,
    session_table: &SessionTable,
    references: &AtomicU64,
) -> Result<(), ()> {
    progress_h3_handshakes(peer, events, session_table).await?;
    if peer.connection.is_closed() {
        terminate_h3_sessions(peer, events, session_table, WebSocketExit::TransportLost).await?;
        return Ok(());
    }
    progress_h3_sessions(peer, events, session_table).await?;
    let Some(h3) = peer.h3.as_mut() else {
        return Ok(());
    };
    loop {
        match h3.poll(&mut peer.connection) {
            Ok((stream, quiche::h3::Event::Headers { list, .. })) => {
                if peer.pending.contains_key(&stream) || peer.sessions.contains_key(&stream) {
                    return Err(());
                }
                let head = parse_h3_websocket_head(&list).ok_or(())?;
                let route = routes
                    .iter()
                    .find(|route| {
                        route.path == head.path
                            && route.versions.iter().any(|version| version == "http/3")
                    })
                    .cloned()
                    .ok_or(())?;
                if !h3_headers_authorized(&route, &head.headers)
                    || !h3_origin_allowed(&route, &head.headers)
                {
                    h3.send_response(
                        &mut peer.connection,
                        stream,
                        &[quiche::h3::Header::new(b":status", b"401")],
                        true,
                    )
                    .map_err(|_| ())?;
                    continue;
                }
                let selected = h3_select_subprotocol(&route.subprotocols, &head.headers)?;
                let reference_value = references.fetch_add(1, Ordering::Relaxed);
                if reference_value == u64::MAX {
                    return Err(());
                }
                let reservation = events.reserve(head.byte_len).await.map_err(|_| ())?;
                let ext = metadata(
                    head.headers.clone(),
                    head.path.clone(),
                    "http/3",
                    h3_text_header(&head.headers, "origin"),
                    selected.clone(),
                );
                let (decision, accepted) = oneshot::channel();
                let proposal = PendingIngress {
                    handle: IngressProposalHandle::new(reference_value),
                    proposal: IngressProposal {
                        provenance: IngressProvenance::single(route.node.clone()),
                        draft: EnvelopeDraft {
                            port: crate::node::Port::conventional_out(),
                            data: neutral_data(&route.codec),
                            ext: ext.clone(),
                        },
                        sequence: IngressSequence::Next,
                        session: Some(InboundSession::Open),
                    },
                };
                events
                    .emit(
                        DriverEvent::Proposal(Box::new(LiveProposal {
                            proposal,
                            decision,
                            _reservation: EventReservation::default(),
                        })),
                        0,
                    )
                    .await
                    .map_err(|_| ())?;
                peer.pending.insert(
                    stream,
                    H3PendingHandshake {
                        state: H3HandshakeState::Pending(accepted),
                        ext,
                        route,
                        _reservation: reservation,
                    },
                );
            }
            Ok((stream, quiche::h3::Event::Data)) => {
                let live = peer.sessions.get_mut(&stream).ok_or(())?;
                if live.pending_decision.is_some() {
                    continue;
                }
                let mut chunk = [0u8; 16 * 1024];
                loop {
                    match h3.recv_body(&mut peer.connection, stream, &mut chunk) {
                        Ok(read) if read > 0 => live.websocket.get_mut().feed(&chunk[..read]),
                        Ok(_) | Err(quiche::h3::Error::Done) => break,
                        Err(_) => return Err(()),
                    }
                }
                if let Some(exit) = drain_h3_server_messages(live, events).await? {
                    let live = peer.sessions.remove(&stream).ok_or(())?;
                    forget_active_session(session_table, live.session).await;
                    events
                        .emit(
                            DriverEvent::Session(exit_event(live.session, exit, live.ext)),
                            0,
                        )
                        .await
                        .map_err(|_| ())?;
                }
            }
            Ok((stream, quiche::h3::Event::Finished | quiche::h3::Event::Reset(_))) => {
                if let Some(live) = peer.sessions.remove(&stream) {
                    forget_active_session(session_table, live.session).await;
                    events
                        .emit(
                            DriverEvent::Session(exit_event(
                                live.session,
                                WebSocketExit::TransportLost,
                                live.ext,
                            )),
                            0,
                        )
                        .await
                        .map_err(|_| ())?;
                }
                peer.pending.remove(&stream);
            }
            Ok((_, quiche::h3::Event::GoAway)) => return Err(()),
            Ok((_, quiche::h3::Event::PriorityUpdate)) => {}
            Err(quiche::h3::Error::Done) => break,
            Err(_) => return Err(()),
        }
    }
    Ok(())
}

#[cfg(feature = "hosted-http3")]
async fn progress_h3_handshakes(
    peer: &mut H3Peer,
    events: &mut EventSender<DriverEvent>,
    session_table: &SessionTable,
) -> Result<(), ()> {
    let streams = peer.pending.keys().copied().collect::<Vec<_>>();
    for stream in streams {
        let transition = {
            let pending = peer.pending.get_mut(&stream).ok_or(())?;
            match &mut pending.state {
                H3HandshakeState::Pending(decision) => match decision.try_recv() {
                    Ok(Ok(AdmittedIngress {
                        exchange: Some(InboundExchangeId::Session(session)),
                        ..
                    })) => Some(H3HandshakeState::Accepted(session)),
                    Ok(_) | Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                        Some(H3HandshakeState::Rejected)
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => None,
                },
                H3HandshakeState::Accepted(session) => Some(H3HandshakeState::Accepted(*session)),
                H3HandshakeState::Rejected => Some(H3HandshakeState::Rejected),
            }
        };
        let Some(transition) = transition else {
            continue;
        };
        peer.pending.get_mut(&stream).ok_or(())?.state = transition;
        let state = &peer.pending.get(&stream).ok_or(())?.state;
        if peer.connection.is_closed() {
            let pending = peer.pending.remove(&stream).ok_or(())?;
            if let H3HandshakeState::Accepted(session) = pending.state {
                events
                    .emit(
                        DriverEvent::Session(exit_event(
                            session,
                            WebSocketExit::TransportLost,
                            pending.ext,
                        )),
                        0,
                    )
                    .await
                    .map_err(|_| ())?;
            }
            continue;
        }
        let Some(h3) = peer.h3.as_mut() else {
            return Err(());
        };
        match state {
            H3HandshakeState::Accepted(session) => {
                let session = *session;
                let pending = peer.pending.get(&stream).ok_or(())?;
                let selected =
                    pending
                        .ext
                        .get("websocket_subprotocol")
                        .and_then(|value| match value {
                            MetaValue::Scalar(Scalar::Text(value)) => Some(value.as_str()),
                            _ => None,
                        });
                let mut headers = vec![quiche::h3::Header::new(b":status", b"200")];
                if let Some(selected) = selected {
                    headers.push(quiche::h3::Header::new(
                        b"sec-websocket-protocol",
                        selected.as_bytes(),
                    ));
                }
                for (name, values) in &pending.route.headers {
                    for value in values {
                        headers.push(quiche::h3::Header::new(name.as_bytes(), value));
                    }
                }
                match h3.send_response(&mut peer.connection, stream, &headers, false) {
                    Ok(()) => {}
                    Err(quiche::h3::Error::Done) => continue,
                    Err(_) => return Err(()),
                }
                let pending = peer.pending.remove(&stream).ok_or(())?;
                let config = crate::connector::drivers::network::websocket_codec::bounded_config(
                    usize::try_from(pending.route.max_message).map_err(|_| ())?,
                )
                .map_err(|_| ())?;
                let websocket = tungstenite::WebSocket::from_raw_socket(
                    H3FrameIo::default(),
                    Role::Server,
                    Some(config),
                );
                let (commands, receiver) = mpsc::channel(1);
                register_session(session_table, session, commands).await;
                peer.sessions.insert(
                    stream,
                    H3LiveSession {
                        commands: receiver,
                        ext: pending.ext,
                        outgoing: Vec::new(),
                        outgoing_offset: 0,
                        pending_decision: None,
                        route: pending.route,
                        session,
                        websocket,
                    },
                );
            }
            H3HandshakeState::Rejected => {
                match h3.send_response(
                    &mut peer.connection,
                    stream,
                    &[quiche::h3::Header::new(b":status", b"503")],
                    true,
                ) {
                    Ok(()) => {
                        peer.pending.remove(&stream);
                    }
                    Err(quiche::h3::Error::Done) => continue,
                    Err(_) => return Err(()),
                }
            }
            H3HandshakeState::Pending(_) => {}
        }
    }
    let _ = events;
    Ok(())
}

#[cfg(feature = "hosted-http3")]
async fn terminate_h3_sessions(
    peer: &mut H3Peer,
    events: &mut EventSender<DriverEvent>,
    session_table: &SessionTable,
    exit: WebSocketExit,
) -> Result<(), ()> {
    let sessions = core::mem::take(&mut peer.sessions);
    for (_, live) in sessions {
        forget_active_session(session_table, live.session).await;
        events
            .emit(
                DriverEvent::Session(exit_event(live.session, exit.clone(), live.ext)),
                0,
            )
            .await
            .map_err(|_| ())?;
    }
    Ok(())
}

#[cfg(feature = "hosted-http3")]
async fn progress_h3_sessions(
    peer: &mut H3Peer,
    events: &mut EventSender<DriverEvent>,
    session_table: &SessionTable,
) -> Result<(), ()> {
    let streams = peer.sessions.keys().copied().collect::<Vec<_>>();
    let mut terminals = Vec::new();
    for stream in streams {
        let Some(live) = peer.sessions.get_mut(&stream) else {
            continue;
        };
        if let Some((decision, _)) = &mut live.pending_decision {
            match decision.try_recv() {
                Ok(Ok(AdmittedIngress {
                    exchange: Some(InboundExchangeId::Session(session)),
                    ..
                })) if session == live.session => {
                    live.pending_decision = None;
                    if let Some(exit) = drain_h3_server_messages(live, events).await? {
                        terminals.push((stream, exit));
                        continue;
                    }
                }
                Ok(_) | Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    terminals.push((stream, WebSocketExit::ProtocolFailed));
                    continue;
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => continue,
            }
        }
        if live.outgoing.is_empty() {
            match live.commands.try_recv() {
                Ok(ActorCommand::Write(intent, message)) => {
                    live.websocket
                        .send(outbound_message(&message.codec, message.body).map_err(|_| ())?)
                        .map_err(|_| ())?;
                    live.websocket.flush().map_err(|_| ())?;
                    live.outgoing = live.websocket.get_mut().take_outgoing();
                    events
                        .emit(
                            DriverEvent::Session(SessionObservation::WriteSettled {
                                session: live.session,
                                write: intent.write_ref,
                            }),
                            0,
                        )
                        .await
                        .map_err(|_| ())?;
                }
                Ok(ActorCommand::Close) | Err(mpsc::error::TryRecvError::Disconnected) => {
                    let _ = live.websocket.close(None);
                    live.outgoing = live.websocket.get_mut().take_outgoing();
                    terminals.push((
                        stream,
                        WebSocketExit::Closed {
                            code: 1000,
                            reason: String::new(),
                        },
                    ));
                }
                Err(mpsc::error::TryRecvError::Empty) => {}
            }
        }
        while live.outgoing_offset < live.outgoing.len() {
            let Some(h3) = peer.h3.as_mut() else {
                return Err(());
            };
            match h3.send_body(
                &mut peer.connection,
                stream,
                &live.outgoing[live.outgoing_offset..],
                false,
            ) {
                Ok(written) => live.outgoing_offset = live.outgoing_offset.saturating_add(written),
                Err(quiche::h3::Error::Done) => break,
                Err(_) => {
                    terminals.push((stream, WebSocketExit::TransportLost));
                    break;
                }
            }
        }
        if live.outgoing_offset == live.outgoing.len() {
            live.outgoing.clear();
            live.outgoing_offset = 0;
        }
    }
    for (stream, exit) in terminals {
        if let Some(live) = peer.sessions.remove(&stream) {
            forget_active_session(session_table, live.session).await;
            if let Some(h3) = peer.h3.as_mut() {
                let _ = h3.send_body(&mut peer.connection, stream, &live.outgoing, true);
            }
            events
                .emit(
                    DriverEvent::Session(exit_event(live.session, exit, live.ext)),
                    0,
                )
                .await
                .map_err(|_| ())?;
        }
    }
    Ok(())
}

#[cfg(feature = "hosted-http3")]
async fn drain_h3_server_messages(
    live: &mut H3LiveSession,
    events: &mut EventSender<DriverEvent>,
) -> Result<Option<WebSocketExit>, ()> {
    if live.pending_decision.is_some() {
        return Ok(None);
    }
    loop {
        let (data, bytes) = match live.websocket.read() {
            Ok(Message::Text(text)) if live.route.codec != "bytes" => {
                let bytes = text.as_bytes().to_vec();
                let data = decode_inbound(&live.route.codec, bytes.clone()).map_err(|_| ())?;
                (data, bytes)
            }
            Ok(Message::Binary(bytes)) if live.route.codec == "bytes" => {
                let bytes = bytes.to_vec();
                (Data::materialized(Value::Bytes(bytes.clone())), bytes)
            }
            Ok(Message::Text(_) | Message::Binary(_)) => {
                return Ok(Some(WebSocketExit::DecodeFailed));
            }
            Ok(Message::Close(frame)) => return Ok(Some(close_exit(frame))),
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {
                live.outgoing = live.websocket.get_mut().take_outgoing();
                continue;
            }
            Err(tungstenite::Error::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock =>
            {
                return Ok(None);
            }
            Err(error) => {
                return Ok(Some(socket_error(error, live.route.max_message)));
            }
        };
        let reservation = events.reserve(bytes.len()).await.map_err(|_| ())?;
        let reference = next_proposal_ref().map_err(|_| ())?;
        let (decision, accepted) = oneshot::channel();
        let proposal = PendingIngress {
            handle: reference,
            proposal: IngressProposal {
                provenance: IngressProvenance::single(live.route.node.clone()),
                draft: EnvelopeDraft {
                    port: crate::node::Port::conventional_out(),
                    data,
                    ext: live.ext.clone(),
                },
                sequence: IngressSequence::Next,
                session: Some(InboundSession::Message(live.session)),
            },
        };
        events
            .emit(
                DriverEvent::Proposal(Box::new(LiveProposal {
                    proposal,
                    decision,
                    _reservation: EventReservation::default(),
                })),
                0,
            )
            .await
            .map_err(|_| ())?;
        live.pending_decision = Some((accepted, reservation));
        return Ok(None);
    }
}

#[cfg(feature = "hosted-http3")]
struct H3WebSocketHead {
    byte_len: usize,
    headers: BTreeMap<String, Vec<Vec<u8>>>,
    path: String,
}

#[cfg(feature = "hosted-http3")]
fn parse_h3_websocket_head(headers: &[quiche::h3::Header]) -> Option<H3WebSocketHead> {
    use quiche::h3::NameValue as _;
    let mut method = None;
    let mut protocol = None;
    let mut path = None;
    let mut version = None;
    let mut regular = BTreeMap::<String, Vec<Vec<u8>>>::new();
    let mut byte_len = 0usize;
    for header in headers {
        byte_len = byte_len
            .checked_add(header.name().len())?
            .checked_add(header.value().len())?;
        match header.name() {
            b":method" => method = std::str::from_utf8(header.value()).ok(),
            b":protocol" => protocol = std::str::from_utf8(header.value()).ok(),
            b":path" => path = std::str::from_utf8(header.value()).ok().map(str::to_string),
            b"sec-websocket-version" => version = std::str::from_utf8(header.value()).ok(),
            name if name.starts_with(b":") => {}
            name => {
                let name = std::str::from_utf8(name).ok()?.to_string();
                regular
                    .entry(name)
                    .or_default()
                    .push(header.value().to_vec());
            }
        }
    }
    (method == Some("CONNECT") && protocol == Some("websocket") && version == Some("13")).then_some(
        H3WebSocketHead {
            byte_len,
            headers: regular,
            path: path?,
        },
    )
}

#[cfg(feature = "hosted-http3")]
fn h3_text_header(headers: &BTreeMap<String, Vec<Vec<u8>>>, name: &str) -> Option<String> {
    headers
        .get(name)?
        .first()
        .and_then(|value| std::str::from_utf8(value).ok())
        .map(str::to_string)
}

#[cfg(feature = "hosted-http3")]
fn h3_headers_authorized(
    route: &WebSocketServerBinding,
    headers: &BTreeMap<String, Vec<Vec<u8>>>,
) -> bool {
    route.auth_bearer.as_ref().is_none_or(|token| {
        headers
            .get("authorization")
            .and_then(|values| values.first())
            .is_some_and(|value| value == format!("Bearer {token}").as_bytes())
    })
}

#[cfg(feature = "hosted-http3")]
fn h3_origin_allowed(
    route: &WebSocketServerBinding,
    headers: &BTreeMap<String, Vec<Vec<u8>>>,
) -> bool {
    route.allowed_origins.is_empty()
        || h3_text_header(headers, "origin").is_some_and(|origin| {
            route
                .allowed_origins
                .iter()
                .any(|allowed| allowed == &origin)
        })
}

#[cfg(feature = "hosted-http3")]
fn h3_select_subprotocol(
    allowed: &[String],
    headers: &BTreeMap<String, Vec<Vec<u8>>>,
) -> Result<Option<String>, ()> {
    if allowed.is_empty() {
        return Ok(None);
    }
    h3_text_header(headers, "sec-websocket-protocol")
        .into_iter()
        .flat_map(|value| {
            value
                .split(',')
                .map(str::trim)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .find(|requested| allowed.contains(requested))
        .map(Some)
        .ok_or(())
}

#[cfg(feature = "hosted-http3")]
async fn flush_h3_peer(socket: &tokio::net::UdpSocket, peer: &mut H3Peer, output: &mut [u8]) {
    loop {
        match peer.connection.send(output) {
            Ok((written, info)) => {
                if socket.send_to(&output[..written], info.to).await.is_err() {
                    let _ = peer.connection.close(false, 0x100, b"send failed");
                    break;
                }
            }
            Err(quiche::Error::Done) => break,
            Err(_) => {
                let _ = peer.connection.close(false, 0x100, b"send failed");
                break;
            }
        }
    }
}

fn bind_listeners(routes: Vec<WebSocketServerBinding>) -> Result<Vec<ListenerBinding>, String> {
    let mut groups: BTreeMap<String, Vec<WebSocketServerBinding>> = BTreeMap::new();
    for route in routes {
        if !route
            .versions
            .iter()
            .any(|version| matches!(version.as_str(), "http/1.1" | "http/2"))
        {
            continue;
        }
        groups.entry(route.listen.clone()).or_default().push(route);
    }
    groups
        .into_iter()
        .map(|(listen, routes)| {
            let first = routes
                .first()
                .ok_or_else(|| format!("the WebSocketServer listener `{listen}` has no routes"))?;
            if routes.iter().any(|route| {
                route.tls_certificate != first.tls_certificate
                    || route.tls_private_key != first.tls_private_key
            }) {
                return Err(format!(
                    "WebSocketServer routes sharing `{listen}` must use one TLS identity"
                ));
            }
            let mut seen = BTreeSet::new();
            for route in &routes {
                if !seen.insert(route.path.clone()) {
                    return Err(format!(
                        "WebSocketServer routes sharing `{listen}` declare `{}` twice",
                        route.path
                    ));
                }
            }
            let listener = TcpListener::bind(&listen).map_err(|error| {
                format!("the WebSocketServer listener `{listen}` cannot bind: {error}")
            })?;
            listener.set_nonblocking(true).map_err(|error| {
                format!("the WebSocketServer listener `{listen}` is not async: {error}")
            })?;
            let tls = match (&first.tls_certificate, &first.tls_private_key) {
                (Some(certificate), Some(private_key)) => {
                    let mut config = super::tls::rustls_server(super::tls::TlsIdentity {
                        certificate_chain: certificate,
                        private_key,
                    })
                    .map_err(|error| error.to_string())?;
                    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
                    Some(Arc::new(config))
                }
                (None, None) => None,
                _ => {
                    return Err(format!(
                        "the WebSocketServer TLS pair at `{listen}` is incomplete"
                    ));
                }
            };
            Ok(ListenerBinding {
                listener,
                routes: Arc::new(routes),
                tls,
            })
        })
        .collect()
}

async fn serve_listener(
    binding: ListenerBinding,
    events: EventSender<DriverEvent>,
    sessions: SessionTable,
    references: Arc<AtomicU64>,
    config: WebSocketConfig,
) {
    let Ok(listener) = tokio::net::TcpListener::from_std(binding.listener) else {
        return;
    };
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            break;
        };
        let routes = Arc::clone(&binding.routes);
        let events = events.clone();
        let sessions = Arc::clone(&sessions);
        let references = Arc::clone(&references);
        if let Some(tls) = &binding.tls {
            let acceptor = tokio_rustls::TlsAcceptor::from(Arc::clone(tls));
            tokio::spawn(async move {
                if let Ok(stream) = acceptor.accept(stream).await {
                    serve_connection(stream, routes, events, sessions, references, config).await;
                }
            });
        } else {
            tokio::spawn(async move {
                serve_connection(stream, routes, events, sessions, references, config).await;
            });
        }
    }
}

async fn serve_connection<S>(
    stream: S,
    routes: Arc<Vec<WebSocketServerBinding>>,
    events: EventSender<DriverEvent>,
    sessions: SessionTable,
    references: Arc<AtomicU64>,
    config: WebSocketConfig,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let service = hyper::service::service_fn(move |request| {
        serve_handshake(
            request,
            Arc::clone(&routes),
            events.clone(),
            Arc::clone(&sessions),
            Arc::clone(&references),
        )
    });
    let mut builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
    builder.http1().max_buf_size(config.max_header_bytes);
    builder.http2().enable_connect_protocol();
    let _ = builder
        .serve_connection_with_upgrades(TokioIo::new(stream), service)
        .await;
}

async fn serve_handshake(
    mut request: Request<hyper::body::Incoming>,
    routes: Arc<Vec<WebSocketServerBinding>>,
    mut events: EventSender<DriverEvent>,
    sessions: SessionTable,
    references: Arc<AtomicU64>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let version = request.version();
    let Some(route) = routes
        .iter()
        .find(|route| {
            route.path == request.uri().path()
                && route.versions.iter().any(|candidate| {
                    matches!(
                        (candidate.as_str(), version),
                        ("http/1.1", Version::HTTP_11) | ("http/2", Version::HTTP_2)
                    )
                })
        })
        .cloned()
    else {
        return Ok(empty_response(StatusCode::NOT_FOUND));
    };
    if !valid_upgrade_request(&request) {
        return Ok(empty_response(StatusCode::BAD_REQUEST));
    }
    if !authorized(&route, request.headers()) || !origin_allowed(&route, request.headers()) {
        return Ok(empty_response(StatusCode::UNAUTHORIZED));
    }
    let selected = match select_server_subprotocol(&route.subprotocols, request.headers()) {
        Ok(selected) => selected,
        Err(()) => return Ok(empty_response(StatusCode::BAD_REQUEST)),
    };
    let reference_value = references.fetch_add(1, Ordering::Relaxed);
    if reference_value == u64::MAX {
        return Ok(empty_response(StatusCode::SERVICE_UNAVAILABLE));
    }
    let reference = IngressProposalHandle::new(reference_value);
    let ext = server_metadata(&request, selected.as_deref());
    let draft = EnvelopeDraft {
        port: crate::node::Port::conventional_out(),
        data: neutral_data(&route.codec),
        ext: ext.clone(),
    };
    let (decision, accepted) = oneshot::channel();
    let proposal = PendingIngress {
        handle: reference,
        proposal: IngressProposal {
            provenance: IngressProvenance::single(route.node.clone()),
            draft,
            sequence: IngressSequence::Next,
            session: Some(InboundSession::Open),
        },
    };
    if events
        .emit(
            DriverEvent::Proposal(Box::new(LiveProposal {
                proposal,
                decision,
                _reservation: EventReservation::default(),
            })),
            0,
        )
        .await
        .is_err()
    {
        return Ok(empty_response(StatusCode::SERVICE_UNAVAILABLE));
    }
    let session = match accepted.await {
        Ok(Ok(AdmittedIngress {
            exchange: Some(InboundExchangeId::Session(session)),
            ..
        })) => session,
        Ok(Err(_)) | Ok(Ok(_)) | Err(_) => {
            return Ok(empty_response(StatusCode::SERVICE_UNAVAILABLE));
        }
    };
    let upgraded = hyper::upgrade::on(&mut request);
    let (commands, mut receiver) = mpsc::channel(1);
    register_session(&sessions, session, commands).await;
    let table = Arc::clone(&sessions);
    let mut actor_events = events.clone();
    let codec = route.codec.clone();
    let max_message = route.max_message;
    let connector = route.node.clone();
    let response = upgrade_response(&request, &route, selected.as_deref());
    tokio::spawn(async move {
        let exit = match upgraded.await {
            Ok(upgraded) => {
                match crate::connector::drivers::network::websocket_codec::bounded_config(
                    usize::try_from(max_message).unwrap_or(0),
                ) {
                    Ok(config) => {
                        let socket = WebSocketStream::from_raw_socket(
                            TokioIo::new(upgraded),
                            Role::Server,
                            Some(config),
                        )
                        .await;
                        match run_socket(
                            socket,
                            SocketSession {
                                role: Role::Server,
                                connector: &connector,
                                session,
                                codec: &codec,
                                max_message,
                                initial_message: &[],
                                ext: ext.clone(),
                            },
                            &mut receiver,
                            &mut actor_events,
                        )
                        .await
                        {
                            Ok((exit, _)) | Err(exit) => exit,
                        }
                    }
                    Err(_) => WebSocketExit::ProtocolFailed,
                }
            }
            Err(_) => WebSocketExit::TransportLost,
        };
        forget_active_session(&table, session).await;
        let _ = actor_events
            .emit(DriverEvent::Session(exit_event(session, exit, ext)), 0)
            .await;
    });
    Ok(response)
}

fn valid_upgrade_request(request: &Request<hyper::body::Incoming>) -> bool {
    match request.version() {
        Version::HTTP_11 => {
            request.method() == http::Method::GET
                && request
                    .headers()
                    .get(http::header::UPGRADE)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
                && request
                    .headers()
                    .get(http::header::SEC_WEBSOCKET_VERSION)
                    .is_some_and(|value| value == "13")
                && request
                    .headers()
                    .contains_key(http::header::SEC_WEBSOCKET_KEY)
        }
        Version::HTTP_2 => {
            request.method() == http::Method::CONNECT
                && request
                    .extensions()
                    .get::<hyper::ext::Protocol>()
                    .is_some_and(|protocol| protocol.as_str() == "websocket")
                && request
                    .headers()
                    .get(http::header::SEC_WEBSOCKET_VERSION)
                    .is_some_and(|value| value == "13")
        }
        _ => false,
    }
}

fn upgrade_response(
    request: &Request<hyper::body::Incoming>,
    route: &WebSocketServerBinding,
    subprotocol: Option<&str>,
) -> Response<Full<Bytes>> {
    let status = if request.version() == Version::HTTP_11 {
        StatusCode::SWITCHING_PROTOCOLS
    } else {
        StatusCode::OK
    };
    let mut response = Response::builder()
        .status(status)
        .body(Full::new(Bytes::new()))
        .unwrap_or_else(|_| empty_response(StatusCode::INTERNAL_SERVER_ERROR));
    for (name, values) in &route.headers {
        let Ok(name) = http::HeaderName::from_bytes(name.as_bytes()) else {
            return empty_response(StatusCode::INTERNAL_SERVER_ERROR);
        };
        for value in values {
            let Ok(value) = http::HeaderValue::from_bytes(value) else {
                return empty_response(StatusCode::INTERNAL_SERVER_ERROR);
            };
            response.headers_mut().append(name.clone(), value);
        }
    }
    if request.version() == Version::HTTP_11 {
        response.headers_mut().insert(
            http::header::UPGRADE,
            http::HeaderValue::from_static("websocket"),
        );
        response.headers_mut().insert(
            http::header::CONNECTION,
            http::HeaderValue::from_static("Upgrade"),
        );
        if let Some(key) = request.headers().get(http::header::SEC_WEBSOCKET_KEY) {
            let accept = tungstenite::handshake::derive_accept_key(key.as_bytes());
            if let Ok(value) = http::HeaderValue::from_str(&accept) {
                response
                    .headers_mut()
                    .insert(http::header::SEC_WEBSOCKET_ACCEPT, value);
            }
        }
    }
    if let Some(subprotocol) = subprotocol
        && let Ok(value) = http::HeaderValue::from_str(subprotocol)
    {
        response
            .headers_mut()
            .insert(http::header::SEC_WEBSOCKET_PROTOCOL, value);
    }
    response
}

fn empty_response(status: StatusCode) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .body(Full::new(Bytes::new()))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
}

fn authorized(route: &WebSocketServerBinding, headers: &http::HeaderMap) -> bool {
    route.auth_bearer.as_ref().is_none_or(|token| {
        headers
            .get(http::header::AUTHORIZATION)
            .is_some_and(|value| value.as_bytes() == format!("Bearer {token}").as_bytes())
    })
}

fn origin_allowed(route: &WebSocketServerBinding, headers: &http::HeaderMap) -> bool {
    route.allowed_origins.is_empty()
        || headers
            .get(http::header::ORIGIN)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|origin| {
                route
                    .allowed_origins
                    .iter()
                    .any(|allowed| allowed == origin)
            })
}

fn select_server_subprotocol(
    allowed: &[String],
    headers: &http::HeaderMap,
) -> Result<Option<String>, ()> {
    let requested = headers
        .get(http::header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(',').map(str::trim).collect::<Vec<_>>())
        .unwrap_or_default();
    if allowed.is_empty() {
        return Ok(None);
    }
    requested
        .into_iter()
        .find(|candidate| allowed.iter().any(|allowed| allowed == candidate))
        .map(|value| Some(value.to_string()))
        .ok_or(())
}

fn selected_subprotocol(
    headers: &http::HeaderMap,
    offered: &[String],
) -> Result<Option<String>, WebSocketExit> {
    let Some(selected) = headers
        .get(http::header::SEC_WEBSOCKET_PROTOCOL)
        .map(|value| value.to_str().map(str::to_string))
        .transpose()
        .map_err(|_| WebSocketExit::ProtocolFailed)?
    else {
        return Ok(None);
    };
    offered
        .iter()
        .any(|offered| offered == &selected)
        .then_some(Some(selected))
        .ok_or(WebSocketExit::ProtocolFailed)
}

fn neutral_data(codec: &str) -> Data {
    Data::materialized(match codec {
        "bytes" => Value::Bytes(Vec::new()),
        "text" => Value::Text(String::new()),
        _ => Value::Null,
    })
}

fn lossless_headers(headers: &http::HeaderMap) -> BTreeMap<String, Vec<Vec<u8>>> {
    let mut values = BTreeMap::<String, Vec<Vec<u8>>>::new();
    for (name, value) in headers {
        values
            .entry(name.as_str().to_string())
            .or_default()
            .push(value.as_bytes().to_vec());
    }
    values
}

fn metadata(
    headers: BTreeMap<String, Vec<Vec<u8>>>,
    uri: String,
    version: &str,
    origin: Option<String>,
    subprotocol: Option<String>,
) -> Namespace {
    let mut ext = Namespace::new();
    ext.insert(
        String::from("http_headers"),
        MetaValue::Map(
            headers
                .into_iter()
                .map(|(name, values)| {
                    (
                        name,
                        values.into_iter().map(Scalar::Bytes).collect::<Vec<_>>(),
                    )
                })
                .collect(),
        ),
    );
    ext.insert(
        String::from("http_uri"),
        MetaValue::Scalar(Scalar::Text(uri)),
    );
    ext.insert(
        String::from("http_version"),
        MetaValue::Scalar(Scalar::Text(version.to_string())),
    );
    if let Some(origin) = origin {
        ext.insert(
            String::from("websocket_origin"),
            MetaValue::Scalar(Scalar::Text(origin)),
        );
    }
    if let Some(subprotocol) = subprotocol {
        ext.insert(
            String::from("websocket_subprotocol"),
            MetaValue::Scalar(Scalar::Text(subprotocol)),
        );
    }
    ext
}

fn client_metadata(
    opening: &WebSocketClientOpening,
    version: &str,
    subprotocol: Option<String>,
    headers: BTreeMap<String, Vec<Vec<u8>>>,
) -> Namespace {
    metadata(headers, opening.url.clone(), version, None, subprotocol)
}

fn server_metadata(
    request: &Request<hyper::body::Incoming>,
    subprotocol: Option<&str>,
) -> Namespace {
    metadata(
        lossless_headers(request.headers()),
        request.uri().to_string(),
        match request.version() {
            Version::HTTP_2 => "http/2",
            _ => "http/1.1",
        },
        request
            .headers()
            .get(http::header::ORIGIN)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string),
        subprotocol.map(str::to_string),
    )
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::expect_used,
    clippy::panic
)]
mod tests {
    use super::*;
    use crate::compiler::compile;
    use crate::connector::contract::ContractCatalog;
    use crate::envelope::Id;
    use std::sync::mpsc::{Receiver, TryRecvError};
    use std::time::{Duration, Instant};

    #[test]
    fn physical_capability_selection_precedes_any_websocket_handshake() {
        let versions = vec![
            String::from("http/3"),
            String::from("http/2"),
            String::from("http/1.1"),
        ];
        assert_eq!(
            selected_client_version("ws://example.test/socket", &versions),
            Some("http/2"),
            "cleartext cannot select QUIC merely because HTTP/3 is preferred"
        );
        assert_eq!(
            selected_client_version("ftp://example.test/socket", &versions),
            None
        );
        #[cfg(feature = "hosted-http3")]
        assert_eq!(
            selected_client_version("wss://example.test/socket", &versions),
            Some("http/3")
        );
    }

    fn server_plan(version: &str) -> crate::plan::ValidatedExecutionPlan {
        server_plan_with_limit(version, "1MiB")
    }

    fn server_plan_with_limit(
        version: &str,
        max_message: &str,
    ) -> crate::plan::ValidatedExecutionPlan {
        let source = format!(
            r#"
graph websocket : App {{
  input in: text
  connector socket : WebSocketServer {{
    listen = "127.0.0.1:0"
    path = "/socket"
    handshake_timeout = duration("5s")
    codec = text
    max_message = "{max_message}"
    versions = ["{version}"]
  }}
  in -> socket -> out
  socket.exit -> nothing
}}
"#
        );
        compile(
            &[&source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("the WebSocketServer source checks")
        .plan
    }

    fn client_plan(url: &str, version: &str) -> crate::plan::ValidatedExecutionPlan {
        let source = format!(
            r#"
graph websocket : App {{
  input in: text
  connector socket : WebSocketClient {{
    url = "{url}"
    handshake_timeout = duration("5s")
    codec = text
     behavior = deterministic
    versions = ["{version}"]
  }}
  in -> socket -> out
  socket.exit -> nothing
}}
"#
        );
        compile(
            &[&source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("the WebSocketClient source checks")
        .plan
    }

    fn wait_for_proposal(server: &mut HostedWebSocket<'_>) -> PendingIngress {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(proposal) = server.poll_proposal() {
                return proposal;
            }
            assert!(
                Instant::now() < deadline,
                "the hosted WebSocket server did not expose its proposal"
            );
            std::thread::yield_now();
        }
    }

    fn wait_for_exit(server: &mut HostedWebSocket<'_>) -> SessionObservation {
        wait_for_session_events(server, 1).remove(0)
    }

    fn wait_for_session_events(
        server: &mut HostedWebSocket<'_>,
        count: usize,
    ) -> Vec<SessionObservation> {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut observed = Vec::new();
        loop {
            observed.extend(server.poll_observations());
            if observed.len() >= count {
                return observed;
            }
            assert!(
                Instant::now() < deadline,
                "the hosted WebSocket server exposed only {} of {count} terminal events",
                observed.len()
            );
            std::thread::yield_now();
        }
    }

    fn client_h1(
        address: std::net::SocketAddr,
        close: tokio::sync::oneshot::Receiver<()>,
    ) -> Receiver<Result<(), String>> {
        client_h1_message(address, "hello", close, true)
    }

    fn client_h1_message(
        address: std::net::SocketAddr,
        text: &'static str,
        close: tokio::sync::oneshot::Receiver<()>,
        clean: bool,
    ) -> Receiver<Result<(), String>> {
        let (sent, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string());
            let result = runtime.and_then(|runtime| {
                runtime.block_on(async move {
                    use tungstenite::client::IntoClientRequest;
                    let stream = tokio::net::TcpStream::connect(address)
                        .await
                        .map_err(|error| error.to_string())?;
                    let request = format!("ws://{address}/socket")
                        .into_client_request()
                        .map_err(|error| error.to_string())?;
                    let (mut socket, response) = tokio_tungstenite::client_async(request, stream)
                        .await
                        .map_err(|error| error.to_string())?;
                    if response.status() != StatusCode::SWITCHING_PROTOCOLS {
                        return Err(format!("unexpected H1 status {}", response.status()));
                    }
                    socket
                        .send(Message::text(text))
                        .await
                        .map_err(|error| error.to_string())?;
                    close
                        .await
                        .map_err(|_| String::from("close signal vanished"))?;
                    if clean {
                        socket
                            .close(None)
                            .await
                            .map_err(|error| error.to_string())?;
                    }
                    Ok(())
                })
            });
            let _ = sent.send(result);
        });
        received
    }

    fn client_h2(
        address: std::net::SocketAddr,
        close: tokio::sync::oneshot::Receiver<()>,
    ) -> Receiver<Result<(), String>> {
        let (sent, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string());
            let result = runtime.and_then(|runtime| {
                runtime.block_on(async move {
                    let stream = tokio::net::TcpStream::connect(address)
                        .await
                        .map_err(|error| error.to_string())?;
                    let builder = hyper::client::conn::http2::Builder::new(TokioExecutor::new());
                    let (mut sender, connection) = builder
                        .handshake::<_, Empty<Bytes>>(TokioIo::new(stream))
                        .await
                        .map_err(|error| error.to_string())?;
                    tokio::spawn(async move {
                        let _ = connection.await;
                    });
                    let mut request = Request::builder()
                        .method(http::Method::CONNECT)
                        .uri(format!("http://{address}/socket"))
                        .version(Version::HTTP_2)
                        .header(http::header::SEC_WEBSOCKET_VERSION, "13")
                        .body(Empty::new())
                        .map_err(|error| error.to_string())?;
                    request
                        .extensions_mut()
                        .insert(hyper::ext::Protocol::from_static("websocket"));
                    let mut response = sender
                        .send_request(request)
                        .await
                        .map_err(|error| error.to_string())?;
                    if response.status() != StatusCode::OK {
                        return Err(format!("unexpected H2 status {}", response.status()));
                    }
                    let upgraded = hyper::upgrade::on(&mut response)
                        .await
                        .map_err(|error| error.to_string())?;
                    let mut socket = WebSocketStream::from_raw_socket(
                        TokioIo::new(upgraded),
                        Role::Client,
                        None,
                    )
                    .await;
                    socket
                        .send(Message::text("hello"))
                        .await
                        .map_err(|error| error.to_string())?;
                    close
                        .await
                        .map_err(|_| String::from("close signal vanished"))?;
                    socket
                        .close(None)
                        .await
                        .map_err(|error| error.to_string())?;
                    Ok(())
                })
            });
            let _ = sent.send(result);
        });
        received
    }

    fn prove_server_session(version: &str) {
        let plan = server_plan(version);
        let node = crate::node::Path::new(["socket"]).expect("the path is valid");
        let mut server = HostedWebSocket::try_bind_server(
            &plan,
            std::slice::from_ref(&node),
            WebSocketConfig::default(),
        )
        .expect("the hosted server binds");
        let address = server.local_addresses()[0];
        let (close, closing) = tokio::sync::oneshot::channel();
        let peer = if version == "http/2" {
            client_h2(address, closing)
        } else {
            client_h1(address, closing)
        };

        let opening = wait_for_proposal(&mut server);
        assert_eq!(opening.proposal.session, Some(InboundSession::Open));
        assert_eq!(
            peer.try_recv(),
            Err(TryRecvError::Empty),
            "the peer cannot observe upgrade success before causal acceptance"
        );
        let session = SessionId::new(Id::from_bytes([9; 16]));
        server.notify_admission(
            opening.handle,
            Ok(AdmittedIngress {
                envelope: Id::from_bytes([7; 16]),
                exchange: Some(InboundExchangeId::Session(session)),
            }),
        );

        let message = wait_for_proposal(&mut server);
        assert_eq!(
            message.proposal.session,
            Some(InboundSession::Message(session))
        );
        assert_eq!(
            message.proposal.draft.data.materialized_value(),
            Some(&Value::Text(String::from("hello")))
        );
        server.notify_admission(
            message.handle,
            Ok(AdmittedIngress {
                envelope: Id::from_bytes([8; 16]),
                exchange: Some(InboundExchangeId::Session(session)),
            }),
        );
        close.send(()).expect("the peer remains live");
        let event = wait_for_exit(&mut server);
        let SessionObservation::Exited {
            session: exited,
            exit,
        } = event
        else {
            panic!("the session must terminate exactly once")
        };
        assert_eq!(exited, session);
        assert_eq!(exit.port, crate::node::Port::conventional_exit());
        peer.recv_timeout(Duration::from_secs(3))
            .expect("the peer completes")
            .expect("the peer observes a clean close");
        assert!(server.poll_observations().is_empty());
    }

    #[test]
    fn h1_upgrade_waits_for_commit_and_preserves_one_logical_session() {
        prove_server_session("http/1.1");
    }

    #[test]
    fn h2_extended_connect_waits_for_commit_and_preserves_one_logical_session() {
        prove_server_session("http/2");
    }

    #[test]
    fn one_server_routes_two_live_peers_by_logical_session() {
        let plan = server_plan("http/1.1");
        let node = crate::node::Path::new(["socket"]).expect("the path is valid");
        let mut server = HostedWebSocket::try_bind_server(
            &plan,
            std::slice::from_ref(&node),
            WebSocketConfig::default(),
        )
        .expect("the hosted server binds");
        let address = server.local_addresses()[0];

        let (close_alpha, alpha_closing) = tokio::sync::oneshot::channel();
        let alpha = client_h1_message(address, "alpha", alpha_closing, true);
        let alpha_open = wait_for_proposal(&mut server);
        let alpha_session = SessionId::new(Id::from_bytes([0x51; 16]));
        server.notify_admission(
            alpha_open.handle,
            Ok(AdmittedIngress {
                envelope: Id::from_bytes([0x52; 16]),
                exchange: Some(InboundExchangeId::Session(alpha_session)),
            }),
        );
        let alpha_message = wait_for_proposal(&mut server);
        assert_eq!(
            alpha_message.proposal.session,
            Some(InboundSession::Message(alpha_session))
        );
        assert_eq!(
            alpha_message.proposal.draft.data.materialized_value(),
            Some(&Value::Text(String::from("alpha")))
        );
        server.notify_admission(
            alpha_message.handle,
            Ok(AdmittedIngress {
                envelope: Id::from_bytes([0x53; 16]),
                exchange: Some(InboundExchangeId::Session(alpha_session)),
            }),
        );

        let (close_beta, beta_closing) = tokio::sync::oneshot::channel();
        let beta = client_h1_message(address, "beta", beta_closing, true);
        let beta_open = wait_for_proposal(&mut server);
        let beta_session = SessionId::new(Id::from_bytes([0x61; 16]));
        server.notify_admission(
            beta_open.handle,
            Ok(AdmittedIngress {
                envelope: Id::from_bytes([0x62; 16]),
                exchange: Some(InboundExchangeId::Session(beta_session)),
            }),
        );
        let beta_message = wait_for_proposal(&mut server);
        assert_eq!(
            beta_message.proposal.session,
            Some(InboundSession::Message(beta_session))
        );
        assert_eq!(
            beta_message.proposal.draft.data.materialized_value(),
            Some(&Value::Text(String::from("beta")))
        );
        assert_ne!(alpha_session, beta_session);
        server.notify_admission(
            beta_message.handle,
            Ok(AdmittedIngress {
                envelope: Id::from_bytes([0x63; 16]),
                exchange: Some(InboundExchangeId::Session(beta_session)),
            }),
        );

        close_alpha.send(()).expect("alpha remains live");
        close_beta.send(()).expect("beta remains live");
        let exits = wait_for_session_events(&mut server, 2);
        let exited = exits
            .iter()
            .filter_map(|event| match event {
                SessionObservation::Exited { session, .. } => Some(*session),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(exited, BTreeSet::from([alpha_session, beta_session]));
        alpha
            .recv_timeout(Duration::from_secs(3))
            .expect("alpha completes")
            .expect("alpha closes cleanly");
        beta.recv_timeout(Duration::from_secs(3))
            .expect("beta completes")
            .expect("beta closes cleanly");
        assert!(server.poll_observations().is_empty());
    }

    #[test]
    fn an_unclean_peer_close_has_one_transport_lost_exit() {
        let plan = server_plan("http/1.1");
        let node = crate::node::Path::new(["socket"]).expect("the path is valid");
        let mut server = HostedWebSocket::try_bind_server(
            &plan,
            std::slice::from_ref(&node),
            WebSocketConfig::default(),
        )
        .expect("the hosted server binds");
        let address = server.local_addresses()[0];
        let (drop_peer, dropping) = tokio::sync::oneshot::channel();
        let peer = client_h1_message(address, "before-loss", dropping, false);
        let opening = wait_for_proposal(&mut server);
        let session = SessionId::new(Id::from_bytes([0x71; 16]));
        server.notify_admission(
            opening.handle,
            Ok(AdmittedIngress {
                envelope: Id::from_bytes([0x72; 16]),
                exchange: Some(InboundExchangeId::Session(session)),
            }),
        );
        let message = wait_for_proposal(&mut server);
        server.notify_admission(
            message.handle,
            Ok(AdmittedIngress {
                envelope: Id::from_bytes([0x73; 16]),
                exchange: Some(InboundExchangeId::Session(session)),
            }),
        );
        drop_peer.send(()).expect("the peer remains live");
        peer.recv_timeout(Duration::from_secs(3))
            .expect("the peer drops")
            .expect("dropping a socket needs no protocol write");
        let SessionObservation::Exited {
            session: exited,
            exit,
        } = wait_for_exit(&mut server)
        else {
            panic!("the transport loss must terminate the session")
        };
        assert_eq!(exited, session);
        assert!(matches!(
            exit.data.materialized_value(),
            Some(Value::TaggedUnion(value)) if value.case == "TransportLost"
        ));
        assert!(server.poll_observations().is_empty());
    }

    #[test]
    fn an_oversized_message_has_one_typed_terminal_without_an_envelope() {
        let plan = server_plan_with_limit("http/1.1", "4B");
        let node = crate::node::Path::new(["socket"]).expect("the path is valid");
        let mut server = HostedWebSocket::try_bind_server(
            &plan,
            std::slice::from_ref(&node),
            WebSocketConfig::default(),
        )
        .expect("the hosted server binds");
        let address = server.local_addresses()[0];
        let (release_peer, releasing) = tokio::sync::oneshot::channel();
        let peer = client_h1_message(address, "hello", releasing, true);
        let opening = wait_for_proposal(&mut server);
        let session = SessionId::new(Id::from_bytes([0x74; 16]));
        server.notify_admission(
            opening.handle,
            Ok(AdmittedIngress {
                envelope: Id::from_bytes([0x75; 16]),
                exchange: Some(InboundExchangeId::Session(session)),
            }),
        );
        let SessionObservation::Exited {
            session: exited,
            exit,
        } = wait_for_exit(&mut server)
        else {
            panic!("the capacity refusal must terminate the session")
        };
        assert_eq!(exited, session);
        assert!(matches!(
            exit.data.materialized_value(),
            Some(Value::TaggedUnion(value))
                if value.case == "MessageTooLarge"
                    && value.payload == vec![(String::from("limit"), Value::Uint(4))]
        ));
        assert!(
            server.poll_proposal().is_none(),
            "no oversized message is admitted"
        );
        let _ = release_peer.send(());
        let _ = peer.recv_timeout(Duration::from_secs(3));
        assert!(server.poll_observations().is_empty());
    }

    #[test]
    fn a_close_cancels_an_opening_handshake_even_when_control_overtakes_work() {
        use crate::journal::{Journal, MemJournal};
        use crate::plan::GraphVersion;
        use std::io::Read as _;

        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback port is available");
        let address = listener.local_addr().expect("the listener has an address");
        let (accepted, connected) = std::sync::mpsc::channel();
        let peer = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("the client connects");
            accepted.send(()).expect("the test remains live");
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .expect("the peer read is bounded");
            let mut bytes = Vec::new();
            let _ = stream.read_to_end(&mut bytes);
        });

        let plan = client_plan(&format!("ws://{address}/socket"), "http/1.1");
        let node = crate::node::Path::new(["socket"]).expect("the path is valid");
        let mut client = HostedWebSocket::try_bind_client(
            &plan,
            std::slice::from_ref(&node),
            WebSocketConfig::default(),
        )
        .expect("the hosted client binds");
        let data = Data::materialized(Value::Text(String::from("hello")));
        let RequestPreparation::Prepared(prepared) =
            client.prepare_session_open(&node, &data, &BTreeMap::new())
        else {
            panic!("the opening prepares")
        };
        let session = SessionId::new(Id::from_bytes([0x41; 16]));
        let intent = SessionOpen {
            session_id: session,
            emitting_id: Id::from_bytes([0x40; 16]),
            connector_path: node.clone(),
            prepared,
        };
        let mut journal = MemJournal::new(GraphVersion(1));
        let seed = journal
            .append_ingress(IngressProposal {
                provenance: IngressProvenance::single(node),
                draft: EnvelopeDraft {
                    port: crate::node::Port::conventional_out(),
                    data,
                    ext: Namespace::new(),
                },
                sequence: IngressSequence::Next,
                session: None,
            })
            .expect("the seed commits")
            .envelope;
        assert_eq!(client.open_session(&intent, &seed), SessionStart::Started);
        connected
            .recv_timeout(Duration::from_secs(3))
            .expect("the physical handshake starts");
        assert_eq!(client.close_session(session), SessionStart::Started);
        let SessionObservation::Exited {
            session: exited,
            exit,
        } = wait_for_exit(&mut client)
        else {
            panic!("cancellation must terminate the session")
        };
        assert_eq!(exited, session);
        assert!(matches!(
            exit.data.materialized_value(),
            Some(Value::TaggedUnion(value)) if value.case == "Closed"
        ));
        peer.join()
            .expect("the stalled peer observes socket closure");
    }

    #[cfg(feature = "hosted-http3")]
    fn replace_setting(
        plan: &mut crate::plan::ExecutionPlan,
        node_path: &str,
        name: &str,
        setting: crate::plan::AttrSetting,
    ) {
        let node = plan
            .nodes
            .iter_mut()
            .find(|node| node.path.to_string() == node_path)
            .expect("the fixture contains the connector");
        let crate::plan::NodeKind::Connector(spec) = &mut node.body else {
            panic!("the fixture node is a connector")
        };
        if let Some((_, value)) = spec.attributes.iter_mut().find(|(field, _)| field == name) {
            *value = setting;
        } else {
            spec.attributes.push((name.to_string(), setting));
            spec.attributes.sort_by(|left, right| left.0.cmp(&right.0));
        }
    }

    #[cfg(feature = "hosted-http3")]
    fn h3_server_plan() -> crate::plan::ValidatedExecutionPlan {
        const CERTIFICATE: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../spikes/h3-quinn-http3/certs/server.crt"
        ));
        const PRIVATE_KEY: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../spikes/h3-quinn-http3/certs/server.key"
        ));
        let mut plan = server_plan("http/1.1").plan().clone();
        replace_setting(
            &mut plan,
            "socket",
            "versions",
            crate::plan::AttrSetting::Value(Value::Array(vec![Value::Text(String::from(
                "http/3",
            ))])),
        );
        replace_setting(
            &mut plan,
            "socket",
            "tls_certificate",
            crate::plan::AttrSetting::Value(Value::Bytes(CERTIFICATE.to_vec())),
        );
        replace_setting(
            &mut plan,
            "socket",
            "tls_private_key",
            crate::plan::AttrSetting::Value(Value::Bytes(PRIVATE_KEY.to_vec())),
        );
        crate::plan::ValidatedExecutionPlan::try_new(plan)
            .expect("the HTTP/3 WebSocketServer plan validates")
    }

    #[cfg(feature = "hosted-http3")]
    fn h3_client_plan(address: std::net::SocketAddr) -> crate::plan::ValidatedExecutionPlan {
        const CA_CERTIFICATE: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../spikes/h3-quinn-http3/certs/ca.crt"
        ));
        let source = r#"
graph websocket : App {
  input in: text
  connector socket : WebSocketClient {
    url = "wss://localhost:443/socket"
    handshake_timeout = duration("5s")
    codec = text
     behavior = deterministic
    versions = ["http/3"]
  }
  in -> socket -> out
  socket.exit -> nothing
}
"#;
        let mut plan = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("the HTTP/3 WebSocketClient source checks")
        .plan
        .plan()
        .clone();
        replace_setting(
            &mut plan,
            "socket",
            "url",
            crate::plan::AttrSetting::Value(Value::Text(format!(
                "wss://localhost:{}/socket",
                address.port()
            ))),
        );
        replace_setting(
            &mut plan,
            "socket",
            "tls_ca",
            crate::plan::AttrSetting::Value(Value::Bytes(CA_CERTIFICATE.to_vec())),
        );
        crate::plan::ValidatedExecutionPlan::try_new(plan)
            .expect("the HTTP/3 WebSocketClient plan validates")
    }

    #[cfg(feature = "hosted-http3")]
    #[test]
    fn h3_extended_connect_runs_the_same_bounded_session_machine() {
        use crate::journal::{Journal, MemJournal};
        use crate::plan::GraphVersion;

        let server_plan = h3_server_plan();
        let node = crate::node::Path::new(["socket"]).expect("the path is valid");
        let mut server = HostedWebSocket::try_bind_server(
            &server_plan,
            std::slice::from_ref(&node),
            WebSocketConfig::default(),
        )
        .expect("the HTTP/3 server binds");
        let address = server.local_addresses()[0];
        let client_plan = h3_client_plan(address);
        let mut client = HostedWebSocket::try_bind_client(
            &client_plan,
            std::slice::from_ref(&node),
            WebSocketConfig::default(),
        )
        .expect("the HTTP/3 client binds");
        let data = Data::materialized(Value::Text(String::from("hello-h3")));
        let RequestPreparation::Prepared(prepared) =
            client.prepare_session_open(&node, &data, &BTreeMap::new())
        else {
            panic!("the HTTP/3 opening must prepare")
        };
        let client_session = SessionId::new(Id::from_bytes([0x31; 16]));
        let intent = SessionOpen {
            session_id: client_session,
            emitting_id: Id::from_bytes([0x30; 16]),
            connector_path: node.clone(),
            prepared,
        };
        let mut journal = MemJournal::new(GraphVersion(1));
        let seed = journal
            .append_ingress(IngressProposal {
                provenance: IngressProvenance::single(node.clone()),
                draft: EnvelopeDraft {
                    port: crate::node::Port::conventional_out(),
                    data: data.clone(),
                    ext: Namespace::new(),
                },
                sequence: IngressSequence::Next,
                session: None,
            })
            .expect("the seed commits")
            .envelope;
        assert_eq!(client.open_session(&intent, &seed), SessionStart::Started);

        let deadline = Instant::now() + Duration::from_secs(5);
        let opening = loop {
            if let Some(proposal) = server.poll_proposal() {
                break proposal;
            }
            let client_events = client.poll_observations();
            assert!(
                client_events.is_empty(),
                "the HTTP/3 client failed before the server proposal: {client_events:?}"
            );
            assert!(
                Instant::now() < deadline,
                "the hosted HTTP/3 server did not expose its proposal"
            );
            std::thread::yield_now();
        };
        let server_session = SessionId::new(Id::from_bytes([0x32; 16]));
        server.notify_admission(
            opening.handle,
            Ok(AdmittedIngress {
                envelope: Id::from_bytes([0x33; 16]),
                exchange: Some(InboundExchangeId::Session(server_session)),
            }),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if client
                .poll_observations()
                .iter()
                .any(|event| matches!(event, SessionObservation::Opened(session) if *session == client_session))
            {
                break;
            }
            assert!(Instant::now() < deadline, "the H3 opening did not settle");
            std::thread::yield_now();
        }
        let message = wait_for_proposal(&mut server);
        assert_eq!(
            message.proposal.draft.data.materialized_value(),
            Some(&Value::Text(String::from("hello-h3")))
        );
        assert_eq!(
            message.proposal.session,
            Some(InboundSession::Message(server_session))
        );
        server.notify_admission(
            message.handle,
            Ok(AdmittedIngress {
                envelope: Id::from_bytes([0x34; 16]),
                exchange: Some(InboundExchangeId::Session(server_session)),
            }),
        );
        assert_eq!(client.close_session(client_session), SessionStart::Started);
        let server_exit = wait_for_exit(&mut server);
        assert!(matches!(
            server_exit,
            SessionObservation::Exited { session, .. } if session == server_session
        ));
        let client_exit = wait_for_exit(&mut client);
        assert!(matches!(
            client_exit,
            SessionObservation::Exited { session, .. } if session == client_session
        ));
        assert!(server.poll_observations().is_empty());
        assert!(client.poll_observations().is_empty());
    }

    #[test]
    fn sealed_websocket_descriptors_round_trip_without_reconstruction() {
        let source = r#"
graph websocket : App {
  input in: text
  connector socket : WebSocketClient {
    url = "ws://example.test/socket"
    handshake_timeout = duration("5s")
    codec = text
     behavior = deterministic
    versions = ["http/2", "http/1.1"]
  }
  in -> socket -> out
  socket.exit -> nothing
}
"#;
        let plan = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("the WebSocketClient source checks")
        .plan;
        let node = crate::node::Path::new(["socket"]).expect("the path is valid");
        let id = crate::connector::ContractId::parse(
            crate::connector::stdlib::WEBSOCKET_CLIENT_CONNECTOR_ID,
        )
        .expect("the id is canonical");
        let binding = WebSocketLabDriver::try_bind(&plan, std::slice::from_ref(&node), &id)
            .expect("the protocol binds");
        let prepared = binding
            .prepare_open(
                &node,
                &Data::materialized(Value::Text(String::from("hello"))),
                &BTreeMap::from([(
                    String::from("url"),
                    Value::Text(String::from("ws://example.test/socket")),
                )]),
            )
            .expect("the opening frames");
        let decoded = decode_client_open(&prepared).expect("the exact descriptor decodes");
        assert_eq!(decoded.initial_message, b"hello");
        assert_eq!(decoded.versions, ["http/2", "http/1.1"]);
        assert_eq!(
            decode_message(
                &binding
                    .prepare_message(
                        &node,
                        &Data::materialized(Value::Text(String::from("again"))),
                    )
                    .expect("the message frames"),
            ),
            Some(WebSocketMessage {
                codec: String::from("text"),
                body: b"again".to_vec(),
            })
        );
    }

    #[test]
    fn the_physical_message_bound_refuses_zero_and_overflow() {
        assert!(crate::connector::drivers::network::websocket_codec::bounded_config(0).is_err());
        assert!(
            crate::connector::drivers::network::websocket_codec::bounded_config(usize::MAX)
                .is_err()
        );
    }
}
