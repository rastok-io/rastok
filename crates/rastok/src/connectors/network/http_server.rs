use crate::boundary::{
    Draft, HttpServerExit, InboundCallConnectorDriver, InboundCallEvent, InboundCallResponse,
    InboundCallResponseStart, InboundCallTimeout, InboundCallTimeoutStart, InboundConnectorDriver,
    InboundExchangeRef, IngressAcceptance, IngressDecision, IngressEnvelopeProvenance,
    IngressProposal, IngressProposalHandle, IngressRejection, IngressSubmission,
    SubmissionSequence,
};
use crate::connectors::http_server::{HttpServerMode, bind_routes};
use crate::connectors::http_shared::{HttpVersion, decode_body};
use crate::connectors::network::pump::{
    EngineInput, EngineLimits, EventPoll, EventReservation, EventSender, HostedNetworkEngine,
    NetworkRuntime, ReserveError, SubmitError,
};
use crate::envelope::{MetaValue, Namespace, Scalar};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::{Body as _, Incoming};
use hyper::{Request, Response, StatusCode, Version};
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::convert::Infallible;
use std::fmt;
#[cfg(any(test, feature = "hosted-http3"))]
use std::net::SocketAddr;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const DEFAULT_MAX_IN_FLIGHT: usize = 64;
const DEFAULT_MAX_HEADER_BYTES: usize = 64 * 1024;
const DEFAULT_BUFFERED_BYTES: usize = 16 * 1024 * 1024;
const MIN_HTTP1_BUFFER_BYTES: usize = 8 * 1024;

/// Physical bounds of one hosted HttpServer Driver.
///
/// These capacities are deployment material, never plan identity. Request
/// bodies reserve the shared byte budget before each copy, while the request
/// count bounds accepted connections and concurrent HTTP streams.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HttpServerConfig {
    max_in_flight: usize,
    max_header_bytes: usize,
    buffered_bytes: usize,
}

impl HttpServerConfig {
    /// Constructs finite hosted-server bounds without opening a socket.
    ///
    /// # Errors
    ///
    /// Zero capacities, a header bound below Hyper's HTTP/1.1 floor, or a
    /// byte budget outside Tokio's semaphore range are rejected.
    pub fn new(
        max_in_flight: usize,
        max_header_bytes: usize,
        buffered_bytes: usize,
    ) -> Result<Self, HttpServerConfigError> {
        if max_in_flight == 0 || u32::try_from(max_in_flight).is_err() {
            return Err(HttpServerConfigError(
                "the HttpServer in-flight bound must be positive and fit HTTP/2's u32 limit",
            ));
        }
        if max_header_bytes < MIN_HTTP1_BUFFER_BYTES || u32::try_from(max_header_bytes).is_err() {
            return Err(HttpServerConfigError(
                "the HttpServer header bound must cover HTTP/1.1's 8 KiB minimum and fit u32",
            ));
        }
        if buffered_bytes == 0 || u32::try_from(buffered_bytes).is_err() {
            return Err(HttpServerConfigError(
                "the HttpServer byte budget must be positive and fit a u32 semaphore",
            ));
        }
        Ok(Self {
            max_in_flight,
            max_header_bytes,
            buffered_bytes,
        })
    }

    fn engine_limits(self) -> Result<EngineLimits, HttpServerConfigError> {
        EngineLimits::new(
            self.max_in_flight,
            self.max_in_flight,
            self.max_in_flight,
            self.buffered_bytes,
        )
        .map_err(|_| HttpServerConfigError("the HttpServer engine bounds are invalid"))
    }
}

impl Default for HttpServerConfig {
    fn default() -> Self {
        Self {
            max_in_flight: DEFAULT_MAX_IN_FLIGHT,
            max_header_bytes: DEFAULT_MAX_HEADER_BYTES,
            buffered_bytes: DEFAULT_BUFFERED_BYTES,
        }
    }
}

/// Invalid physical HttpServer capacity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HttpServerConfigError(&'static str);

impl fmt::Display for HttpServerConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for HttpServerConfigError {}

/// Statically linked hosted implementation of `std/http-server@3`.
#[derive(Debug)]
pub struct HttpServerFactory {
    id: crate::connector::ContractId,
    config: HttpServerConfig,
}

impl HttpServerFactory {
    /// Constructs a hosted server factory without opening a listener.
    ///
    /// # Errors
    ///
    /// The built-in Connector id ceased to be canonical.
    pub fn try_new(config: HttpServerConfig) -> Result<Self, crate::connector::ConnectorIdError> {
        Ok(Self {
            id: crate::connector::ContractId::parse(crate::connector::HTTP_SERVER_CONNECTOR_ID)?,
            config,
        })
    }
}

impl crate::connectors::ConnectorDriverFactory for HttpServerFactory {
    fn connector_id(&self) -> &crate::connector::ContractId {
        &self.id
    }

    fn roles(&self) -> crate::connector::ConnectorRoles {
        crate::connector::ConnectorRoles::Inbound
    }

    fn bind<'plan>(
        &self,
        plan: &'plan crate::plan::ValidatedExecutionPlan,
        nodes: &[crate::node::Path],
    ) -> Result<
        Box<dyn crate::connectors::BoundConnectorDriver + 'plan>,
        crate::connectors::DriverFactoryError,
    > {
        HttpServer::try_bind(plan, nodes, self.config)
            .map(|driver| {
                Box::new(driver) as Box<dyn crate::connectors::BoundConnectorDriver + 'plan>
            })
            .map_err(crate::connectors::DriverFactoryError::new)
    }
}

#[derive(Clone, Debug)]
struct BoundRoute {
    auth_bearer: Option<blake3::Hash>,
    codec: String,
    max_body: u64,
    mode: HttpServerMode,
    node: crate::node::Path,
    path: String,
    response_headers: http::HeaderMap,
    versions: Vec<HttpVersion>,
}

#[derive(Debug)]
struct ListenerBinding {
    listener: TcpListener,
    routes: Arc<Vec<BoundRoute>>,
    tls: Option<Arc<rustls::ServerConfig>>,
}

#[cfg(feature = "hosted-http3")]
#[derive(Debug)]
struct Http3ListenerBinding {
    socket: std::net::UdpSocket,
    routes: Arc<Vec<BoundRoute>>,
    certificate: Vec<u8>,
    private_key: Vec<u8>,
}

#[derive(Debug)]
struct ListenerGroup {
    certificate: Option<Vec<u8>>,
    private_key: Option<Vec<u8>>,
    routes: Vec<BoundRoute>,
    versions: BTreeSet<&'static str>,
}

#[derive(Debug)]
struct LiveProposal {
    proposal: IngressProposal,
    decision: tokio::sync::oneshot::Sender<ServerDecision>,
    response: Option<RetainedResponse>,
    _reservation: EventReservation,
}

#[derive(Debug)]
struct RetainedResponse {
    peer: tokio::sync::oneshot::Sender<Response<Full<Bytes>>>,
    route: BoundRoute,
}

#[derive(Debug)]
struct ServerCommand {
    call_id: crate::boundary::InboundCallId,
    peer: tokio::sync::oneshot::Sender<Response<Full<Bytes>>>,
    response: Response<Full<Bytes>>,
    terminal: HttpServerExit,
}

#[derive(Clone, Copy, Debug)]
enum ServerDecision {
    Accepted(crate::boundary::InboundCallId),
    Rejected(IngressRejection),
}

#[derive(Debug)]
enum ServerEvent {
    Proposal(Box<LiveProposal>),
    Terminal(InboundCallEvent),
}

#[cfg(feature = "hosted-http3")]
struct Http3Request {
    body: Vec<u8>,
    headers: BTreeMap<String, Vec<Vec<u8>>>,
    method: String,
    path: String,
    reservation: EventReservation,
    route: BoundRoute,
    _request: tokio::sync::OwnedSemaphorePermit,
}

#[cfg(feature = "hosted-http3")]
struct Http3Response {
    connection: Vec<u8>,
    response: Response<Full<Bytes>>,
    stream: u64,
}

#[cfg(feature = "hosted-http3")]
struct Http3PendingResponse {
    body: Vec<u8>,
    headers: Vec<quiche::h3::Header>,
    offset: usize,
    started: bool,
}

#[cfg(feature = "hosted-http3")]
struct Http3Peer {
    connection: quiche::Connection,
    h3: Option<quiche::h3::Connection>,
    requests: BTreeMap<u64, Http3Request>,
    responses: BTreeMap<u64, Http3PendingResponse>,
}

#[cfg(feature = "hosted-http3")]
struct Http3Ingress<'a> {
    routes: &'a [BoundRoute],
    events: &'a mut EventSender<ServerEvent>,
    requests: &'a Arc<tokio::sync::Semaphore>,
    references: &'a Arc<AtomicU64>,
    responses: &'a tokio::sync::mpsc::Sender<Http3Response>,
    max_header_bytes: usize,
}

#[cfg(feature = "hosted-http3")]
struct Http3RequestHead {
    method: String,
    path: String,
    headers: BTreeMap<String, Vec<Vec<u8>>>,
    byte_len: usize,
}

/// Real hosted HTTP/1.1–3 inbound Driver.
///
/// No protocol-library type escapes this private module.
#[derive(Debug)]
struct HttpServer<'plan> {
    _plan: &'plan crate::plan::ValidatedExecutionPlan,
    engine: HostedNetworkEngine<ServerCommand, ServerCommand, ServerEvent>,
    pending: VecDeque<LiveProposal>,
    retained: BTreeMap<crate::boundary::InboundCallId, RetainedResponse>,
    writing: BTreeSet<crate::boundary::InboundCallId>,
    terminals: Vec<InboundCallEvent>,
    #[cfg(test)]
    local_addresses: Vec<SocketAddr>,
}

impl crate::connectors::BoundConnectorDriver for HttpServer<'_> {
    fn inbound(&mut self) -> Option<&mut dyn InboundConnectorDriver> {
        Some(self)
    }

    fn outbound(&mut self) -> Option<&mut dyn crate::boundary::OutboundConnectorDriver> {
        None
    }
}

impl<'plan> HttpServer<'plan> {
    fn try_bind(
        plan: &'plan crate::plan::ValidatedExecutionPlan,
        nodes: &[crate::node::Path],
        config: HttpServerConfig,
    ) -> Result<Self, String> {
        let routes = bind_routes(plan, nodes).map_err(|error| error.to_string())?;
        let mut groups: BTreeMap<String, ListenerGroup> = BTreeMap::new();
        for route in routes {
            let supported = route
                .versions
                .iter()
                .filter(|version| cfg!(feature = "hosted-http3") || **version != HttpVersion::Http3)
                .map(|version| version.observation())
                .collect::<BTreeSet<_>>();
            if supported.is_empty() {
                return Err(format!(
                    "the HttpServer at `{}` offers no HTTP version compiled into this Driver",
                    route.node
                ));
            }
            let request_bound = usize::try_from(route.max_body)
                .ok()
                .and_then(|body| body.checked_add(config.max_header_bytes))
                .ok_or_else(|| {
                    format!(
                        "the HttpServer request bound at `{}` is not representable",
                        route.node
                    )
                })?;
            if request_bound > config.buffered_bytes {
                return Err(format!(
                    "the HttpServer at `{}` needs {request_bound} buffered bytes for one request, \
                     but this Driver supplies {}",
                    route.node, config.buffered_bytes
                ));
            }
            let response_headers = header_map(&route.response_headers).map_err(|reason| {
                format!(
                    "the HttpServer response headers at `{}` are invalid: {reason}",
                    route.node
                )
            })?;
            let group = groups
                .entry(route.listen.clone())
                .or_insert_with(|| ListenerGroup {
                    certificate: route.tls_certificate.clone(),
                    private_key: route.tls_private_key.clone(),
                    routes: Vec::new(),
                    versions: BTreeSet::new(),
                });
            if group.certificate != route.tls_certificate
                || group.private_key != route.tls_private_key
            {
                return Err(format!(
                    "HttpServer routes sharing `{}` must use one TLS identity",
                    route.listen
                ));
            }
            if group.routes.iter().any(|bound| bound.path == route.path) {
                return Err(format!(
                    "HttpServer routes sharing `{}` declare `{}` twice",
                    route.listen, route.path
                ));
            }
            group.versions.extend(supported);
            group.routes.push(BoundRoute {
                auth_bearer: route
                    .auth_bearer
                    .map(|token| blake3::hash(format!("Bearer {token}").as_bytes())),
                codec: route.codec,
                max_body: route.max_body,
                mode: route.mode,
                node: route.node,
                path: route.path,
                response_headers,
                versions: route.versions,
            });
        }

        let mut listeners = Vec::new();
        #[cfg(feature = "hosted-http3")]
        let mut http3_listeners = Vec::new();
        let mut local_addresses = Vec::new();
        for (listen, group) in groups {
            let tcp_enabled =
                group.versions.contains("http/1.1") || group.versions.contains("http/2");
            let listener = if tcp_enabled {
                let listener = TcpListener::bind(&listen).map_err(|error| {
                    format!("the HttpServer listener `{listen}` cannot bind: {error}")
                })?;
                listener.set_nonblocking(true).map_err(|error| {
                    format!("the HttpServer listener `{listen}` is not async: {error}")
                })?;
                Some(listener)
            } else {
                None
            };
            let physical_address = match &listener {
                Some(listener) => listener.local_addr().map_err(|error| {
                    format!("the HttpServer listener `{listen}` has no address: {error}")
                })?,
                None => listen.parse().map_err(|error| {
                    format!("the HttpServer UDP listener `{listen}` has no socket address: {error}")
                })?,
            };
            if listener.is_some() {
                local_addresses.push(physical_address);
            }
            let tls = match (group.certificate, group.private_key) {
                (Some(certificate), Some(private_key)) => {
                    let tls = if tcp_enabled {
                        let mut tls = super::tls::rustls_server(super::tls::TlsIdentity {
                            certificate_chain: &certificate,
                            private_key: &private_key,
                        })
                        .map_err(|error| error.to_string())?;
                        tls.alpn_protocols = [
                            ("http/2", b"h2".as_slice()),
                            ("http/1.1", b"http/1.1".as_slice()),
                        ]
                        .into_iter()
                        .filter(|(version, _)| group.versions.contains(version))
                        .map(|(_, protocol)| protocol.to_vec())
                        .collect();
                        Some(Arc::new(tls))
                    } else {
                        None
                    };
                    #[cfg(feature = "hosted-http3")]
                    if group.versions.contains("http/3") {
                        let socket = std::net::UdpSocket::bind(physical_address).map_err(|error| {
                            format!(
                                "the HttpServer HTTP/3 listener `{physical_address}` cannot bind: {error}"
                            )
                        })?;
                        socket.set_nonblocking(true).map_err(|error| {
                            format!(
                                "the HttpServer HTTP/3 listener `{physical_address}` is not async: {error}"
                            )
                        })?;
                        if listener.is_none() {
                            local_addresses.push(socket.local_addr().map_err(|error| {
                                format!("the HttpServer HTTP/3 listener has no address: {error}")
                            })?);
                        }
                        http3_listeners.push(Http3ListenerBinding {
                            socket,
                            routes: Arc::new(group.routes.clone()),
                            certificate,
                            private_key,
                        });
                    }
                    tls
                }
                (None, None) => {
                    #[cfg(feature = "hosted-http3")]
                    if group.versions.contains("http/3") {
                        return Err(format!(
                            "the HttpServer HTTP/3 listener `{listen}` requires a TLS identity"
                        ));
                    }
                    None
                }
                _ => {
                    return Err(format!(
                        "the HttpServer TLS pair at `{listen}` is incomplete"
                    ));
                }
            };
            if let Some(listener) = listener {
                listeners.push(ListenerBinding {
                    listener,
                    routes: Arc::new(group.routes),
                    tls,
                });
            }
        }

        let limits = config.engine_limits().map_err(|error| error.to_string())?;
        let connections = Arc::new(tokio::sync::Semaphore::new(config.max_in_flight));
        let requests = Arc::new(tokio::sync::Semaphore::new(config.max_in_flight));
        let references = Arc::new(AtomicU64::new(1));
        let engine = HostedNetworkEngine::start(
            "rastok-http-server",
            limits,
            move |mut runtime: NetworkRuntime<ServerCommand, ServerCommand, ServerEvent>| async move {
                for binding in listeners {
                    let events = runtime.event_sender();
                    let connections = Arc::clone(&connections);
                    let requests = Arc::clone(&requests);
                    let references = Arc::clone(&references);
                    tokio::spawn(async move {
                        serve_listener(
                            binding,
                            events,
                            connections,
                            requests,
                            references,
                            config.max_header_bytes,
                            config.max_in_flight,
                        )
                        .await;
                    });
                }
                #[cfg(feature = "hosted-http3")]
                for binding in http3_listeners {
                    let events = runtime.event_sender();
                    let requests = Arc::clone(&requests);
                    let references = Arc::clone(&references);
                    tokio::spawn(async move {
                        serve_http3_listener(
                            binding,
                            events,
                            requests,
                            references,
                            config.max_header_bytes,
                            config.max_in_flight,
                            limits,
                        )
                        .await;
                    });
                }
                while let Some(input) = runtime.recv().await {
                    let command = match input {
                        EngineInput::Work(work) => work.into_inner(),
                        EngineInput::Control(command) => command,
                    };
                    let terminal = if command.peer.send(command.response).is_ok() {
                        command.terminal
                    } else {
                        HttpServerExit::PeerDisconnected
                    };
                    if runtime
                        .emit(
                            ServerEvent::Terminal(InboundCallEvent {
                                call_id: command.call_id,
                                completion: crate::boundary::Completion::http_server_exit(
                                    terminal,
                                ),
                            }),
                            0,
                        )
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            },
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            _plan: plan,
            engine,
            pending: VecDeque::new(),
            retained: BTreeMap::new(),
            writing: BTreeSet::new(),
            terminals: Vec::new(),
            #[cfg(test)]
            local_addresses,
        })
    }

    fn poll_events(&mut self) {
        loop {
            match self.engine.try_event() {
                EventPoll::Event(ServerEvent::Proposal(proposal)) => {
                    self.pending.push_back(*proposal);
                }
                EventPoll::Event(ServerEvent::Terminal(event)) => {
                    self.retained.remove(&event.call_id);
                    self.writing.remove(&event.call_id);
                    self.terminals.push(event);
                }
                EventPoll::Empty | EventPoll::Stopped => break,
            }
        }
    }

    #[cfg(test)]
    fn local_addresses(&self) -> &[SocketAddr] {
        &self.local_addresses
    }
}

impl InboundConnectorDriver for HttpServer<'_> {
    fn proposal(&mut self) -> Option<IngressProposal> {
        self.poll_events();
        self.pending.front().map(|pending| pending.proposal.clone())
    }

    fn decide(&mut self, reference: IngressProposalHandle, decision: IngressDecision) {
        let Some(pending) = self.pending.front() else {
            return;
        };
        if pending.proposal.reference != reference {
            return;
        }
        let Some(pending) = self.pending.pop_front() else {
            return;
        };
        let decision = match decision {
            IngressDecision::Accepted(IngressAcceptance {
                exchange: Some(InboundExchangeRef::Call(call_id)),
                ..
            }) => ServerDecision::Accepted(call_id),
            IngressDecision::Accepted(_) => ServerDecision::Rejected(IngressRejection::UnknownPort),
            IngressDecision::Rejected(reason) => ServerDecision::Rejected(reason),
        };
        match pending.decision.send(decision) {
            Ok(()) => {
                if let (ServerDecision::Accepted(call_id), Some(response)) =
                    (decision, pending.response)
                {
                    self.retained.insert(call_id, response);
                }
            }
            Err(ServerDecision::Accepted(call_id)) => {
                self.terminals.push(InboundCallEvent {
                    call_id,
                    completion: crate::boundary::Completion::http_server_exit(
                        HttpServerExit::PeerDisconnected,
                    ),
                });
            }
            Err(ServerDecision::Rejected(_)) => {}
        }
    }

    fn call(&mut self) -> Option<&mut dyn InboundCallConnectorDriver> {
        Some(self)
    }
}

impl InboundCallConnectorDriver for HttpServer<'_> {
    fn start_response(&mut self, response: &InboundCallResponse) -> InboundCallResponseStart {
        self.poll_events();
        if self.writing.contains(&response.call_id) {
            return InboundCallResponseStart::Started;
        }
        let Some(retained) = self.retained.remove(&response.call_id) else {
            // A peer disconnect may already have queued the sole terminal.
            return InboundCallResponseStart::Started;
        };
        if retained.route.node != response.connector_path {
            self.retained.insert(response.call_id, retained);
            return InboundCallResponseStart::Pending;
        }
        let route = retained.route.clone();
        let framed = graph_response(&route, &response.response);
        let (protocol_response, terminal, bytes) = match framed {
            Ok(response) => response,
            Err(ServerResponseError::Oversized) => (
                response_with_headers(StatusCode::SERVICE_UNAVAILABLE, &route.response_headers),
                HttpServerExit::Overloaded,
                0,
            ),
            Err(ServerResponseError::Invalid) => (
                response_with_headers(StatusCode::INTERNAL_SERVER_ERROR, &route.response_headers),
                HttpServerExit::OutcomeUnknown,
                0,
            ),
        };
        let command = ServerCommand {
            call_id: response.call_id,
            peer: retained.peer,
            response: protocol_response,
            terminal,
        };
        match self.engine.try_submit(command, bytes) {
            Ok(()) => {
                self.writing.insert(response.call_id);
                InboundCallResponseStart::Started
            }
            Err(SubmitError::Full(command) | SubmitError::ByteBudget(command)) => {
                self.retained.insert(
                    response.call_id,
                    RetainedResponse {
                        peer: command.peer,
                        route,
                    },
                );
                InboundCallResponseStart::Pending
            }
            Err(SubmitError::Oversized { value, .. }) => {
                let overload = ServerCommand {
                    call_id: response.call_id,
                    peer: value.peer,
                    response: response_with_headers(
                        StatusCode::SERVICE_UNAVAILABLE,
                        &route.response_headers,
                    ),
                    terminal: HttpServerExit::Overloaded,
                };
                match self.start_control(response.call_id, route, overload) {
                    InboundCallTimeoutStart::Started => InboundCallResponseStart::Started,
                    InboundCallTimeoutStart::Pending => InboundCallResponseStart::Pending,
                }
            }
            Err(SubmitError::Stopped(command)) => {
                self.finish_without_engine(command);
                InboundCallResponseStart::Started
            }
        }
    }

    fn start_timeout(&mut self, timeout: &InboundCallTimeout) -> InboundCallTimeoutStart {
        self.poll_events();
        if self.writing.contains(&timeout.call_id) {
            return InboundCallTimeoutStart::Started;
        }
        let Some(retained) = self.retained.remove(&timeout.call_id) else {
            return InboundCallTimeoutStart::Started;
        };
        if retained.route.node != timeout.connector_path {
            self.retained.insert(timeout.call_id, retained);
            return InboundCallTimeoutStart::Pending;
        }
        let route = retained.route.clone();
        let command = ServerCommand {
            call_id: timeout.call_id,
            peer: retained.peer,
            response: response_with_headers(StatusCode::GATEWAY_TIMEOUT, &route.response_headers),
            terminal: HttpServerExit::RequestExpired,
        };
        self.start_control(timeout.call_id, route, command)
    }

    fn events(&mut self) -> Vec<InboundCallEvent> {
        self.poll_events();
        self.terminals.sort_by_key(|event| event.call_id);
        std::mem::take(&mut self.terminals)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ServerResponseError {
    Invalid,
    Oversized,
}

impl HttpServer<'_> {
    fn start_control(
        &mut self,
        call_id: crate::boundary::InboundCallId,
        route: BoundRoute,
        command: ServerCommand,
    ) -> InboundCallTimeoutStart {
        match self.engine.try_control(command) {
            Ok(()) => {
                self.writing.insert(call_id);
                InboundCallTimeoutStart::Started
            }
            Err(SubmitError::Full(command) | SubmitError::ByteBudget(command)) => {
                self.retained.insert(
                    call_id,
                    RetainedResponse {
                        peer: command.peer,
                        route,
                    },
                );
                InboundCallTimeoutStart::Pending
            }
            Err(SubmitError::Oversized { value, .. } | SubmitError::Stopped(value)) => {
                self.finish_without_engine(value);
                InboundCallTimeoutStart::Started
            }
        }
    }

    fn finish_without_engine(&mut self, command: ServerCommand) {
        let terminal = if command.peer.send(command.response).is_ok() {
            command.terminal
        } else {
            HttpServerExit::PeerDisconnected
        };
        self.terminals.push(InboundCallEvent {
            call_id: command.call_id,
            completion: crate::boundary::Completion::http_server_exit(terminal),
        });
    }
}

fn graph_response(
    route: &BoundRoute,
    envelope: &crate::envelope::Envelope,
) -> Result<(Response<Full<Bytes>>, HttpServerExit, usize), ServerResponseError> {
    let body = crate::boundary::network_payload_bytes(&route.codec, envelope.data())
        .map_err(|_| ServerResponseError::Invalid)?;
    if u64::try_from(body.len()).map_or(true, |length| length > route.max_body) {
        return Err(ServerResponseError::Oversized);
    }
    let status = match envelope.meta().ext().get("http_status") {
        None => StatusCode::OK,
        Some(MetaValue::Scalar(Scalar::Int(status))) => u16::try_from(*status)
            .ok()
            .and_then(|status| StatusCode::from_u16(status).ok())
            .ok_or(ServerResponseError::Invalid)?,
        Some(_) => return Err(ServerResponseError::Invalid),
    };
    let bytes = body.len();
    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() = status;
    *response.headers_mut() = route.response_headers.clone();
    if !response.headers().contains_key(http::header::CONTENT_TYPE)
        && let Some(MetaValue::Scalar(Scalar::Text(content_type))) =
            envelope.meta().ext().get("content_type")
        && let Ok(content_type) = http::HeaderValue::from_str(content_type)
    {
        response
            .headers_mut()
            .insert(http::header::CONTENT_TYPE, content_type);
    }
    Ok((
        response,
        HttpServerExit::Completed {
            status: status.as_u16(),
        },
        bytes,
    ))
}

#[cfg(feature = "hosted-http3")]
async fn serve_http3_listener(
    binding: Http3ListenerBinding,
    mut events: EventSender<ServerEvent>,
    requests: Arc<tokio::sync::Semaphore>,
    references: Arc<AtomicU64>,
    max_header_bytes: usize,
    max_in_flight: usize,
    limits: EngineLimits,
) {
    let Ok(socket) = tokio::net::UdpSocket::from_std(binding.socket) else {
        return;
    };
    let local = match socket.local_addr() {
        Ok(local) => local,
        Err(_) => return,
    };
    let identity = super::tls::TlsIdentity {
        certificate_chain: &binding.certificate,
        private_key: &binding.private_key,
    };
    let mut config = match super::tls::quiche_server(identity, limits) {
        Ok(config) => config,
        Err(_) => return,
    };
    let mut h3_config = match quiche::h3::Config::new() {
        Ok(config) => config,
        Err(_) => return,
    };
    h3_config.set_max_field_section_size(max_header_bytes as u64);
    h3_config.enable_extended_connect(true);
    let (responses, mut response_events) =
        tokio::sync::mpsc::channel::<Http3Response>(max_in_flight);
    let mut peers = BTreeMap::<Vec<u8>, Http3Peer>::new();
    let mut packet = vec![0u8; 65_535];
    let mut output = vec![0u8; 65_535];

    loop {
        flush_http3_peers(&socket, &mut peers, &mut output).await;
        peers.retain(|_, peer| !peer.connection.is_closed());
        let timeout = peers
            .values()
            .filter_map(|peer| peer.connection.timeout())
            .min()
            .unwrap_or(std::time::Duration::from_millis(100));
        tokio::select! {
            response = response_events.recv() => {
                let Some(response) = response else {
                    return;
                };
                if let Some(peer) = peers.get_mut(&response.connection)
                    && let Some(pending) = http3_pending_response(response.response).await
                {
                    peer.responses.insert(response.stream, pending);
                }
            }
            received = socket.recv_from(&mut packet) => {
                let Ok((length, from)) = received else {
                    return;
                };
                let bytes = &mut packet[..length];
                let Ok(header) = quiche::Header::from_slice(bytes, quiche::MAX_CONN_ID_LEN) else {
                    continue;
                };
                let derived = http3_connection_id(&header.dcid, from);
                let key = if peers.contains_key(header.dcid.as_ref()) {
                    header.dcid.as_ref().to_vec()
                } else if peers.contains_key(&derived) {
                    derived.clone()
                } else {
                    if header.ty != quiche::Type::Initial || peers.len() >= max_in_flight {
                        continue;
                    }
                    if !quiche::version_is_supported(header.version) {
                        if let Ok(written) = quiche::negotiate_version(
                            &header.scid,
                            &header.dcid,
                            &mut output,
                        ) {
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
                        &mut config,
                    ) else {
                        continue;
                    };
                    peers.insert(
                        derived.clone(),
                        Http3Peer {
                            connection,
                            h3: None,
                            requests: BTreeMap::new(),
                            responses: BTreeMap::new(),
                        },
                    );
                    derived
                };
                let Some(peer) = peers.get_mut(&key) else {
                    continue;
                };
                if peer
                    .connection
                    .recv(bytes, quiche::RecvInfo { from, to: local })
                    .is_err()
                {
                    continue;
                }
                if peer.connection.is_established() && peer.h3.is_none() {
                    peer.h3 = quiche::h3::Connection::with_transport(
                        &mut peer.connection,
                        &h3_config,
                    )
                    .ok();
                }
                if poll_http3_requests(&key, peer, Http3Ingress {
                    routes: &binding.routes,
                    events: &mut events,
                    requests: &requests,
                    references: &references,
                    responses: &responses,
                    max_header_bytes,
                })
                .await
                .is_err()
                {
                    let _ = peer.connection.close(true, 0x100, b"request refused");
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
fn http3_connection_id(dcid: &quiche::ConnectionId<'_>, peer: SocketAddr) -> Vec<u8> {
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
async fn poll_http3_requests(
    connection_key: &[u8],
    peer: &mut Http3Peer,
    ingress: Http3Ingress<'_>,
) -> Result<(), ()> {
    let Some(h3) = peer.h3.as_mut() else {
        return Ok(());
    };
    loop {
        match h3.poll(&mut peer.connection) {
            Ok((stream, quiche::h3::Event::Headers { list, .. })) => {
                if peer.requests.contains_key(&stream) {
                    append_http3_headers(
                        &mut peer.requests.get_mut(&stream).ok_or(())?.headers,
                        &list,
                    )?;
                    continue;
                }
                let Some(head) = http3_request_head(&list) else {
                    return Err(());
                };
                if head.byte_len > ingress.max_header_bytes {
                    return Err(());
                }
                let Some(route) = ingress
                    .routes
                    .iter()
                    .find(|route| {
                        route.path == head.path && route.versions.contains(&HttpVersion::Http3)
                    })
                    .cloned()
                else {
                    return Err(());
                };
                if !authorized_headers(&head.headers, route.auth_bearer.as_ref()) {
                    return Err(());
                }
                let request = Arc::clone(ingress.requests)
                    .acquire_owned()
                    .await
                    .map_err(|_| ())?;
                let reservation = ingress
                    .events
                    .reserve(head.byte_len)
                    .await
                    .map_err(|_| ())?;
                peer.requests.insert(
                    stream,
                    Http3Request {
                        body: Vec::new(),
                        headers: head.headers,
                        method: head.method,
                        path: head.path,
                        reservation,
                        route,
                        _request: request,
                    },
                );
            }
            Ok((stream, quiche::h3::Event::Data)) => {
                let Some(request) = peer.requests.get_mut(&stream) else {
                    return Err(());
                };
                let mut chunk = [0u8; 16 * 1024];
                loop {
                    match h3.recv_body(&mut peer.connection, stream, &mut chunk) {
                        Ok(0) | Err(quiche::h3::Error::Done) => break,
                        Ok(read) => {
                            let length = request.body.len().checked_add(read).ok_or(())?;
                            if u64::try_from(length)
                                .map_or(true, |length| length > request.route.max_body)
                            {
                                return Err(());
                            }
                            let more = ingress.events.reserve(read).await.map_err(|_| ())?;
                            request.reservation.append(more);
                            request.body.try_reserve(read).map_err(|_| ())?;
                            request.body.extend_from_slice(&chunk[..read]);
                        }
                        Err(_) => return Err(()),
                    }
                }
            }
            Ok((stream, quiche::h3::Event::Finished)) => {
                let Some(request) = peer.requests.remove(&stream) else {
                    continue;
                };
                dispatch_http3_request(
                    connection_key.to_vec(),
                    stream,
                    request,
                    ingress.events.clone(),
                    Arc::clone(ingress.references),
                    ingress.responses.clone(),
                )
                .await?;
            }
            Ok((stream, quiche::h3::Event::Reset(_))) => {
                peer.requests.remove(&stream);
            }
            Ok((_, quiche::h3::Event::GoAway)) => return Err(()),
            Ok((_, quiche::h3::Event::PriorityUpdate)) => {}
            Err(quiche::h3::Error::Done) => break,
            Err(_) => return Err(()),
        }
    }
    flush_http3_responses(peer);
    Ok(())
}

#[cfg(feature = "hosted-http3")]
fn http3_request_head(headers: &[quiche::h3::Header]) -> Option<Http3RequestHead> {
    use quiche::h3::NameValue as _;
    let mut method = None;
    let mut path = None;
    let mut values = BTreeMap::<String, Vec<Vec<u8>>>::new();
    let mut bytes = 0usize;
    for header in headers {
        bytes = bytes
            .checked_add(header.name().len())?
            .checked_add(header.value().len())?;
        match header.name() {
            b":method" => method = String::from_utf8(header.value().to_vec()).ok(),
            b":path" => path = String::from_utf8(header.value().to_vec()).ok(),
            name if name.starts_with(b":") => {}
            name => {
                let name = String::from_utf8(name.to_vec()).ok()?.to_ascii_lowercase();
                values
                    .entry(name)
                    .or_default()
                    .push(header.value().to_vec());
            }
        }
    }
    Some(Http3RequestHead {
        method: method?,
        path: path?,
        headers: values,
        byte_len: bytes,
    })
}

#[cfg(feature = "hosted-http3")]
fn append_http3_headers(
    values: &mut BTreeMap<String, Vec<Vec<u8>>>,
    headers: &[quiche::h3::Header],
) -> Result<(), ()> {
    use quiche::h3::NameValue as _;
    for header in headers {
        if header.name().starts_with(b":") {
            return Err(());
        }
        let name = String::from_utf8(header.name().to_vec())
            .map_err(|_| ())?
            .to_ascii_lowercase();
        values
            .entry(name)
            .or_default()
            .push(header.value().to_vec());
    }
    Ok(())
}

#[cfg(feature = "hosted-http3")]
fn authorized_headers(
    headers: &BTreeMap<String, Vec<Vec<u8>>>,
    expected: Option<&blake3::Hash>,
) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    headers
        .get("authorization")
        .and_then(|values| values.first())
        .is_some_and(|value| blake3::hash(value) == *expected)
}

#[cfg(feature = "hosted-http3")]
async fn dispatch_http3_request(
    connection: Vec<u8>,
    stream: u64,
    request: Http3Request,
    mut events: EventSender<ServerEvent>,
    references: Arc<AtomicU64>,
    responses: tokio::sync::mpsc::Sender<Http3Response>,
) -> Result<(), ()> {
    let Some(data) = decode_body(&request.route.codec, request.body) else {
        return Err(());
    };
    let Some(reference) = next_reference(&references) else {
        return Err(());
    };
    let ext = http_request_metadata(
        request.headers,
        request.method,
        request.path,
        HttpVersion::Http3,
    );
    let proposal = IngressProposal {
        reference,
        submission: IngressSubmission {
            provenance: IngressEnvelopeProvenance::single(request.route.node.clone()),
            port: crate::node::Port::conventional_out(),
            draft: Draft { data, ext },
            sequence: SubmissionSequence::Next,
        },
        session: None,
    };
    let (decision, accepted) = tokio::sync::oneshot::channel();
    let (retained, response) = tokio::sync::oneshot::channel();
    let live = LiveProposal {
        proposal,
        decision,
        response: (request.route.mode == HttpServerMode::Respond).then_some(RetainedResponse {
            peer: retained,
            route: request.route.clone(),
        }),
        _reservation: request.reservation,
    };
    events
        .emit(ServerEvent::Proposal(Box::new(live)), 0)
        .await
        .map_err(|_| ())?;
    let ServerDecision::Accepted(call_id) = accepted.await.map_err(|_| ())? else {
        return Err(());
    };
    let response = match request.route.mode {
        HttpServerMode::Ack => {
            let response =
                response_with_headers(StatusCode::ACCEPTED, &request.route.response_headers);
            let terminal = InboundCallEvent {
                call_id,
                completion: crate::boundary::Completion::http_server_exit(
                    HttpServerExit::Completed { status: 202 },
                ),
            };
            events
                .emit(ServerEvent::Terminal(terminal), 0)
                .await
                .map_err(|_| ())?;
            response
        }
        HttpServerMode::Respond => response.await.map_err(|_| ())?,
    };
    responses
        .send(Http3Response {
            connection,
            response,
            stream,
        })
        .await
        .map_err(|_| ())
}

#[cfg(feature = "hosted-http3")]
async fn http3_pending_response(response: Response<Full<Bytes>>) -> Option<Http3PendingResponse> {
    let (parts, body) = response.into_parts();
    let body = body.collect().await.ok()?.to_bytes().to_vec();
    let mut headers = vec![quiche::h3::Header::new(
        b":status",
        parts.status.as_str().as_bytes(),
    )];
    for (name, value) in &parts.headers {
        headers.push(quiche::h3::Header::new(
            name.as_str().as_bytes(),
            value.as_bytes(),
        ));
    }
    Some(Http3PendingResponse {
        body,
        headers,
        offset: 0,
        started: false,
    })
}

#[cfg(feature = "hosted-http3")]
fn flush_http3_responses(peer: &mut Http3Peer) {
    let Some(h3) = peer.h3.as_mut() else {
        return;
    };
    let streams = peer.responses.keys().copied().collect::<Vec<_>>();
    for stream in streams {
        let Some(response) = peer.responses.get_mut(&stream) else {
            continue;
        };
        if !response.started {
            match h3.send_response(
                &mut peer.connection,
                stream,
                &response.headers,
                response.body.is_empty(),
            ) {
                Ok(()) => response.started = true,
                Err(quiche::h3::Error::Done) => continue,
                Err(_) => {
                    peer.responses.remove(&stream);
                    continue;
                }
            }
        }
        while response.offset < response.body.len() {
            match h3.send_body(
                &mut peer.connection,
                stream,
                &response.body[response.offset..],
                true,
            ) {
                Ok(written) => response.offset += written,
                Err(quiche::h3::Error::Done) => break,
                Err(_) => {
                    response.offset = response.body.len();
                    break;
                }
            }
        }
        if response.started && response.offset == response.body.len() {
            peer.responses.remove(&stream);
        }
    }
}

#[cfg(feature = "hosted-http3")]
async fn flush_http3_peers(
    socket: &tokio::net::UdpSocket,
    peers: &mut BTreeMap<Vec<u8>, Http3Peer>,
    output: &mut [u8],
) {
    for peer in peers.values_mut() {
        flush_http3_responses(peer);
        loop {
            match peer.connection.send(output) {
                Ok((written, info)) => {
                    if socket.send_to(&output[..written], info.to).await.is_err() {
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
}

async fn serve_listener(
    binding: ListenerBinding,
    events: EventSender<ServerEvent>,
    connections: Arc<tokio::sync::Semaphore>,
    requests: Arc<tokio::sync::Semaphore>,
    references: Arc<AtomicU64>,
    max_header_bytes: usize,
    max_in_flight: usize,
) {
    let Ok(listener) = tokio::net::TcpListener::from_std(binding.listener) else {
        return;
    };
    loop {
        let Ok(connection) = Arc::clone(&connections).acquire_owned().await else {
            return;
        };
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let routes = Arc::clone(&binding.routes);
        let tls = binding.tls.clone();
        let events = events.clone();
        let requests = Arc::clone(&requests);
        let references = Arc::clone(&references);
        tokio::spawn(async move {
            let _connection = connection;
            match tls {
                Some(config) => {
                    let acceptor = tokio_rustls::TlsAcceptor::from(config);
                    if let Ok(stream) = acceptor.accept(stream).await {
                        serve_connection(
                            stream,
                            routes,
                            events,
                            requests,
                            references,
                            max_header_bytes,
                            max_in_flight,
                        )
                        .await;
                    }
                }
                None => {
                    serve_connection(
                        stream,
                        routes,
                        events,
                        requests,
                        references,
                        max_header_bytes,
                        max_in_flight,
                    )
                    .await;
                }
            }
        });
    }
}

async fn serve_connection<I>(
    stream: I,
    routes: Arc<Vec<BoundRoute>>,
    events: EventSender<ServerEvent>,
    requests: Arc<tokio::sync::Semaphore>,
    references: Arc<AtomicU64>,
    max_header_bytes: usize,
    max_in_flight: usize,
) where
    I: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let service = hyper::service::service_fn(move |request| {
        handle_request(
            request,
            Arc::clone(&routes),
            events.clone(),
            Arc::clone(&requests),
            Arc::clone(&references),
            max_header_bytes,
        )
    });
    let mut builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
    builder
        .http1()
        .max_headers(128)
        .max_buf_size(max_header_bytes);
    builder
        .http2()
        .max_header_list_size(u32::try_from(max_header_bytes).unwrap_or(u32::MAX))
        .max_concurrent_streams(u32::try_from(max_in_flight).unwrap_or(u32::MAX));
    let _ = builder
        .serve_connection(TokioIo::new(stream), service)
        .await;
}

async fn handle_request(
    request: Request<Incoming>,
    routes: Arc<Vec<BoundRoute>>,
    mut events: EventSender<ServerEvent>,
    requests: Arc<tokio::sync::Semaphore>,
    references: Arc<AtomicU64>,
    max_header_bytes: usize,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let version = match request.version() {
        Version::HTTP_11 => HttpVersion::Http1,
        Version::HTTP_2 => HttpVersion::Http2,
        _ => return Ok(simple_response(StatusCode::HTTP_VERSION_NOT_SUPPORTED)),
    };
    let Some(route) = routes
        .iter()
        .find(|route| route.path == request.uri().path())
        .cloned()
    else {
        return Ok(simple_response(StatusCode::NOT_FOUND));
    };
    if !route.versions.contains(&version) {
        return Ok(simple_response(StatusCode::HTTP_VERSION_NOT_SUPPORTED));
    }
    if !authorized(&request, route.auth_bearer.as_ref()) {
        return Ok(simple_response(StatusCode::UNAUTHORIZED));
    }
    let initial_header_bytes = header_bytes(request.headers());
    if initial_header_bytes.is_none_or(|bytes| bytes > max_header_bytes) {
        return Ok(simple_response(StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE));
    }
    if request
        .body()
        .size_hint()
        .upper()
        .is_some_and(|length| length > route.max_body)
    {
        return Ok(simple_response(StatusCode::PAYLOAD_TOO_LARGE));
    }
    let Ok(_request) = requests.acquire_owned().await else {
        return Ok(simple_response(StatusCode::SERVICE_UNAVAILABLE));
    };
    let mut reservation = match events.reserve(initial_header_bytes.unwrap_or(0)).await {
        Ok(reservation) => reservation,
        Err(ReserveError::Oversized { .. }) => {
            return Ok(simple_response(StatusCode::PAYLOAD_TOO_LARGE));
        }
        Err(ReserveError::Stopped) => return Ok(simple_response(StatusCode::SERVICE_UNAVAILABLE)),
    };
    let (parts, mut incoming) = request.into_parts();
    let mut headers = header_values(&parts.headers);
    let mut body = Vec::new();
    let mut total_header_bytes = initial_header_bytes.unwrap_or(0);
    while let Some(frame) = incoming.frame().await {
        let Ok(frame) = frame else {
            return Ok(simple_response(StatusCode::BAD_REQUEST));
        };
        match frame.into_data() {
            Ok(chunk) => {
                let Some(length) = body.len().checked_add(chunk.len()) else {
                    return Ok(simple_response(StatusCode::PAYLOAD_TOO_LARGE));
                };
                if u64::try_from(length).map_or(true, |length| length > route.max_body) {
                    return Ok(simple_response(StatusCode::PAYLOAD_TOO_LARGE));
                }
                let more = match events.reserve(chunk.len()).await {
                    Ok(reservation) => reservation,
                    Err(ReserveError::Oversized { .. }) => {
                        return Ok(simple_response(StatusCode::PAYLOAD_TOO_LARGE));
                    }
                    Err(ReserveError::Stopped) => {
                        return Ok(simple_response(StatusCode::SERVICE_UNAVAILABLE));
                    }
                };
                reservation.append(more);
                if body.try_reserve(chunk.len()).is_err() {
                    return Ok(simple_response(StatusCode::SERVICE_UNAVAILABLE));
                }
                body.extend_from_slice(&chunk);
            }
            Err(frame) => {
                if let Ok(trailers) = frame.into_trailers() {
                    let Some(trailer_bytes) = header_bytes(&trailers) else {
                        return Ok(simple_response(StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE));
                    };
                    let Some(next) = total_header_bytes.checked_add(trailer_bytes) else {
                        return Ok(simple_response(StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE));
                    };
                    if next > max_header_bytes {
                        return Ok(simple_response(StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE));
                    }
                    let more = match events.reserve(trailer_bytes).await {
                        Ok(reservation) => reservation,
                        Err(ReserveError::Oversized { .. }) => {
                            return Ok(simple_response(
                                StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
                            ));
                        }
                        Err(ReserveError::Stopped) => {
                            return Ok(simple_response(StatusCode::SERVICE_UNAVAILABLE));
                        }
                    };
                    reservation.append(more);
                    total_header_bytes = next;
                    append_header_values(&mut headers, &trailers);
                }
            }
        }
    }
    let Some(data) = decode_body(&route.codec, body) else {
        return Ok(simple_response(StatusCode::BAD_REQUEST));
    };
    let Some(reference) = next_reference(&references) else {
        return Ok(simple_response(StatusCode::SERVICE_UNAVAILABLE));
    };
    let ext = http_request_metadata(
        headers,
        parts.method.as_str().to_string(),
        parts.uri.to_string(),
        version,
    );
    let proposal = IngressProposal {
        reference,
        submission: IngressSubmission {
            provenance: IngressEnvelopeProvenance::single(route.node.clone()),
            port: crate::node::Port::conventional_out(),
            draft: Draft { data, ext },
            sequence: SubmissionSequence::Next,
        },
        session: None,
    };
    let (decision, accepted) = tokio::sync::oneshot::channel();
    let (response, peer_response) = match route.mode {
        HttpServerMode::Ack => (None, None),
        HttpServerMode::Respond => {
            let (peer, response) = tokio::sync::oneshot::channel();
            (
                Some(RetainedResponse {
                    peer,
                    route: route.clone(),
                }),
                Some(response),
            )
        }
    };
    let event = ServerEvent::Proposal(Box::new(LiveProposal {
        proposal,
        decision,
        response,
        _reservation: reservation,
    }));
    if events.emit(event, 0).await.is_err() {
        return Ok(simple_response(StatusCode::SERVICE_UNAVAILABLE));
    }
    match accepted.await {
        Ok(ServerDecision::Accepted(call_id)) => match peer_response {
            None => {
                let response = response_with_headers(StatusCode::ACCEPTED, &route.response_headers);
                let event = ServerEvent::Terminal(InboundCallEvent {
                    call_id,
                    completion: crate::boundary::Completion::http_server_exit(
                        HttpServerExit::Completed { status: 202 },
                    ),
                });
                match events.emit(event, 0).await {
                    Ok(()) => Ok(response),
                    Err(_) => Ok(simple_response(StatusCode::SERVICE_UNAVAILABLE)),
                }
            }
            Some(response) => match response.await {
                Ok(response) => Ok(response),
                Err(_) => Ok(simple_response(StatusCode::SERVICE_UNAVAILABLE)),
            },
        },
        Ok(ServerDecision::Rejected(reason)) => Ok(simple_response(rejection_status(reason))),
        Err(_) => Ok(simple_response(StatusCode::SERVICE_UNAVAILABLE)),
    }
}

fn http_request_metadata(
    headers: BTreeMap<String, Vec<Vec<u8>>>,
    method: String,
    uri: String,
    version: HttpVersion,
) -> Namespace {
    let mut ext = Namespace::new();
    if let Some(content_type) = headers
        .get("content-type")
        .and_then(|values| match values.as_slice() {
            [only] => Some(only),
            _ => None,
        })
        .and_then(|value| String::from_utf8(value.clone()).ok())
        .and_then(|value| crate::datatype::canonical_media_type(&value).ok())
    {
        ext.insert(
            String::from("content_type"),
            MetaValue::Scalar(Scalar::Text(content_type)),
        );
    }
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
        String::from("http_method"),
        MetaValue::Scalar(Scalar::Text(method)),
    );
    ext.insert(
        String::from("http_uri"),
        MetaValue::Scalar(Scalar::Text(uri)),
    );
    ext.insert(
        String::from("http_version"),
        MetaValue::Scalar(Scalar::Text(String::from(version.observation()))),
    );
    ext
}

fn next_reference(references: &AtomicU64) -> Option<IngressProposalHandle> {
    references
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .ok()
        .map(IngressProposalHandle::new)
}

fn authorized(request: &Request<Incoming>, expected: Option<&blake3::Hash>) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    request
        .headers()
        .get(http::header::AUTHORIZATION)
        .is_some_and(|value| blake3::hash(value.as_bytes()) == *expected)
}

fn header_map(values: &BTreeMap<String, Vec<Vec<u8>>>) -> Result<http::HeaderMap, &'static str> {
    let mut headers = http::HeaderMap::new();
    for (name, values) in values {
        let name =
            http::HeaderName::from_bytes(name.as_bytes()).map_err(|_| "invalid field name")?;
        for value in values {
            let value = http::HeaderValue::from_bytes(value).map_err(|_| "invalid field value")?;
            headers.append(name.clone(), value);
        }
    }
    Ok(headers)
}

fn header_values(headers: &http::HeaderMap) -> BTreeMap<String, Vec<Vec<u8>>> {
    let mut values = BTreeMap::new();
    append_header_values(&mut values, headers);
    values
}

fn append_header_values(values: &mut BTreeMap<String, Vec<Vec<u8>>>, headers: &http::HeaderMap) {
    for (name, value) in headers {
        values
            .entry(name.as_str().to_string())
            .or_default()
            .push(value.as_bytes().to_vec());
    }
}

fn header_bytes(headers: &http::HeaderMap) -> Option<usize> {
    headers.iter().try_fold(0usize, |total, (name, value)| {
        total
            .checked_add(name.as_str().len())?
            .checked_add(value.as_bytes().len())
    })
}

fn rejection_status(rejection: IngressRejection) -> StatusCode {
    match rejection {
        IngressRejection::UnknownPort | IngressRejection::SessionUnavailable => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        IngressRejection::InvalidPayload | IngressRejection::InvalidMetadata => {
            StatusCode::BAD_REQUEST
        }
    }
}

fn simple_response(status: StatusCode) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::new()));
    *response.status_mut() = status;
    response
}

fn response_with_headers(status: StatusCode, headers: &http::HeaderMap) -> Response<Full<Bytes>> {
    let mut response = simple_response(status);
    *response.headers_mut() = headers.clone();
    response
}

#[cfg(test)]
// Wall time is only a liveness guard around local hosted sockets. Neither the
// observed duration nor thread order enters a Rastok fact.
#[allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::expect_used,
    clippy::panic
)]
mod tests {
    use super::*;
    use crate::boundary::{InboundExchangeRef, IngressAdmission};
    use crate::compiler::compile;
    use crate::contract::ContractCatalog;
    use crate::journal::{Journal, MemJournal};
    use crate::plan::GraphVersion;
    use crate::runtime::vm::Vm;
    use crate::value::Value;
    use std::io::{Read, Write};
    use std::sync::mpsc::{Receiver, TryRecvError};
    use std::time::{Duration, Instant};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct ObservedResponse {
        status: u16,
        version: Version,
    }

    #[derive(Debug, Eq, PartialEq)]
    struct ObservedBodyResponse {
        status: u16,
        body: Vec<u8>,
    }

    fn server_plan_with(codec: &str, extra_settings: &str) -> crate::plan::ValidatedExecutionPlan {
        let source = format!(
            r#"
graph api : App {{
  connector server : HttpServer {{
    listen = "127.0.0.1:0"
    path = "/hook"
    codec = {codec}
    versions = ["http/2", "http/1.1"]
    {extra_settings}
  }}
  operator relay {{ input in: {input_type} ; on in {{ out = data }} }}
  server -> relay -> out
  server.exit -> nothing
}}
"#,
            input_type = if codec == "json" { "variant" } else { codec },
        );
        compile(
            &[source.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("the HTTP server source checks")
        .plan
    }

    fn server_plan() -> crate::plan::ValidatedExecutionPlan {
        server_plan_with("text", "")
    }

    fn respond_plan() -> crate::plan::ValidatedExecutionPlan {
        let source = r#"
graph api : App {
  connector server : HttpServer {
    mode = respond
    listen = "127.0.0.1:0"
    path = "/hook"
    codec = text
    timeout = duration("1s")
    versions = ["http/2", "http/1.1"]
  }
  operator relay { input in: text ; on in { out = "response" } }
  server -> relay -> server
  server.exit -> nothing
}
"#;
        compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("the respond-mode source checks")
        .plan
    }

    #[cfg(feature = "hosted-http3")]
    fn http3_respond_plan() -> crate::plan::ValidatedExecutionPlan {
        const CERTIFICATE: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../spikes/h3-quinn-http3/certs/server.crt"
        ));
        const PRIVATE_KEY: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../spikes/h3-quinn-http3/certs/server.key"
        ));
        let mut raw = respond_plan().plan().clone();
        let server_node = raw
            .nodes
            .iter_mut()
            .find(|node| node.path.to_string() == "server")
            .expect("the respond plan contains its server");
        let crate::plan::NodeExecution::Connector(server) = &mut server_node.execution else {
            panic!("the server node is a connector")
        };
        let replace = |attributes: &mut Vec<(String, crate::plan::AttrSetting)>,
                       name: &str,
                       setting: crate::plan::AttrSetting| {
            if let Some((_, value)) = attributes.iter_mut().find(|(field, _)| field == name) {
                *value = setting;
            } else {
                attributes.push((name.to_string(), setting));
            }
        };
        // The probe verifies localhost's certificate; bind its first resolved
        // address rather than assuming that localhost selects IPv4.
        let listen = std::net::ToSocketAddrs::to_socket_addrs(&("localhost", 0))
            .expect("localhost resolves for the loopback fixture")
            .next()
            .expect("localhost has a loopback address");
        assert!(listen.ip().is_loopback());
        replace(
            &mut server.attributes,
            "listen",
            crate::plan::AttrSetting::Value(crate::value::Value::Text(listen.to_string())),
        );
        replace(
            &mut server.attributes,
            "versions",
            crate::plan::AttrSetting::Value(crate::value::Value::Array(vec![
                crate::value::Value::Text(String::from("http/3")),
            ])),
        );
        replace(
            &mut server.attributes,
            "tls_certificate",
            crate::plan::AttrSetting::Value(crate::value::Value::Bytes(CERTIFICATE.to_vec())),
        );
        replace(
            &mut server.attributes,
            "tls_private_key",
            crate::plan::AttrSetting::Value(crate::value::Value::Bytes(PRIVATE_KEY.to_vec())),
        );
        server
            .attributes
            .sort_by(|left, right| left.0.cmp(&right.0));
        crate::plan::ValidatedExecutionPlan::try_new(raw)
            .expect("the HTTP/3-only respond plan remains Registry-free valid")
    }

    fn wait_for_proposal(server: &mut HttpServer<'_>) -> IngressProposal {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(proposal) = server.proposal() {
                return proposal;
            }
            assert!(
                Instant::now() < deadline,
                "the hosted server did not expose its request"
            );
            std::thread::yield_now();
        }
    }

    fn wait_for_terminal(server: &mut HttpServer<'_>) -> InboundCallEvent {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(event) = server.events().into_iter().next() {
                return event;
            }
            assert!(
                Instant::now() < deadline,
                "the hosted server did not expose its terminal observation"
            );
            std::thread::yield_now();
        }
    }

    fn admit_and_close(
        server: &mut HttpServer<'_>,
        vm: &mut Vm<'_>,
        journal: &mut MemJournal,
        response: &Receiver<Result<ObservedResponse, String>>,
        expected_version: Version,
        expected_body: &str,
    ) {
        let proposal = wait_for_proposal(server);
        assert_eq!(
            proposal.submission.draft.data.materialized_value(),
            Some(&Value::Text(expected_body.to_string()))
        );
        assert_eq!(
            proposal.submission.draft.ext.get("content_type"),
            Some(&MetaValue::Scalar(Scalar::Text(String::from("text/plain"))))
        );
        assert_eq!(
            proposal.submission.draft.ext.get("http_method"),
            Some(&MetaValue::Scalar(Scalar::Text(String::from("POST"))))
        );
        assert_eq!(
            proposal.submission.draft.ext.get("http_version"),
            Some(&MetaValue::Scalar(Scalar::Text(String::from(
                match expected_version {
                    Version::HTTP_11 => "http/1.1",
                    Version::HTTP_2 => "http/2",
                    version => panic!("the test does not serve {version:?}"),
                }
            ))))
        );
        let Some(MetaValue::Map(headers)) = proposal.submission.draft.ext.get("http_headers")
        else {
            panic!("the admitted request must carry lossless headers")
        };
        assert_eq!(
            headers.get("content-type"),
            Some(&vec![Scalar::Bytes(b"text/plain".to_vec())])
        );
        assert_eq!(
            response.try_recv(),
            Err(TryRecvError::Empty),
            "the peer cannot observe 202 before causal admission"
        );
        let IngressAdmission::Accepted(accepted) = vm
            .admit_ingress(journal, &proposal)
            .expect("the request reaches atomic admission")
        else {
            panic!("the valid request must be accepted")
        };
        let Some(InboundExchangeRef::Call(call_id)) = accepted.exchange else {
            panic!("an accepted server request opens one Call")
        };
        server.decide(proposal.reference, IngressDecision::Accepted(accepted));
        assert_eq!(
            response
                .recv_timeout(Duration::from_secs(3))
                .expect("the admitted request receives one response")
                .expect("the loopback client completes"),
            ObservedResponse {
                status: 202,
                version: expected_version,
            }
        );

        let terminal = wait_for_terminal(server);
        assert_eq!(terminal.call_id, call_id);
        let report = vm.queue_inbound_call_events([terminal]);
        assert_eq!(report.queued, 1);
        assert!(report.discarded.is_empty());
        assert!(report.uncorrelated.is_empty());
        assert!(report.rejected.is_empty());
        vm.drain(journal)
            .expect("the request and its terminal exit commit");
        assert!(journal.open_inbound_calls().is_empty());
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
            "one admitted request has exactly one typed terminal exit"
        );
    }

    fn spawn_http1(
        address: SocketAddr,
        body: &'static str,
    ) -> Receiver<Result<ObservedResponse, String>> {
        spawn_raw_http1(address, "Content-Type: text/plain\r\n", body)
    }

    fn spawn_http1_with_body(
        address: SocketAddr,
        body: &'static str,
    ) -> Receiver<Result<ObservedBodyResponse, String>> {
        let (sent, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| -> Result<ObservedBodyResponse, String> {
                let mut stream = std::net::TcpStream::connect(address)
                    .map_err(|error| format!("the HTTP/1.1 client cannot connect: {error}"))?;
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .map_err(|error| format!("the HTTP/1.1 timeout cannot be set: {error}"))?;
                write!(
                    stream,
                    "POST /hook HTTP/1.1\r\nHost: localhost\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .map_err(|error| format!("the HTTP/1.1 request cannot be sent: {error}"))?;
                let mut bytes = Vec::new();
                stream
                    .read_to_end(&mut bytes)
                    .map_err(|error| format!("the HTTP/1.1 response cannot be read: {error}"))?;
                let marker = bytes
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .ok_or_else(|| String::from("the HTTP/1.1 response has no header boundary"))?;
                let head = std::str::from_utf8(&bytes[..marker])
                    .map_err(|_| String::from("the HTTP/1.1 response head is not text"))?;
                let status = head
                    .lines()
                    .next()
                    .and_then(|line| line.split_ascii_whitespace().nth(1))
                    .and_then(|status| status.parse().ok())
                    .ok_or_else(|| format!("unexpected HTTP/1.1 response: {head:?}"))?;
                Ok(ObservedBodyResponse {
                    status,
                    body: bytes[marker + 4..].to_vec(),
                })
            })();
            let _ = sent.send(result);
        });
        received
    }

    fn spawn_raw_http1(
        address: SocketAddr,
        headers: &'static str,
        body: &'static str,
    ) -> Receiver<Result<ObservedResponse, String>> {
        let (sent, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| -> Result<ObservedResponse, String> {
                let mut stream = std::net::TcpStream::connect(address)
                    .map_err(|error| format!("the HTTP/1.1 client cannot connect: {error}"))?;
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .map_err(|error| format!("the HTTP/1.1 timeout cannot be set: {error}"))?;
                write!(
                    stream,
                    "POST /hook HTTP/1.1\r\nHost: localhost\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .map_err(|error| format!("the HTTP/1.1 request cannot be sent: {error}"))?;
                let mut bytes = Vec::new();
                stream
                    .read_to_end(&mut bytes)
                    .map_err(|error| format!("the HTTP/1.1 response cannot be read: {error}"))?;
                let response = std::str::from_utf8(&bytes)
                    .map_err(|_| String::from("the HTTP/1.1 response is not text"))?;
                let status = response
                    .lines()
                    .next()
                    .and_then(|line| line.split_ascii_whitespace().nth(1))
                    .and_then(|status| status.parse().ok())
                    .ok_or_else(|| format!("unexpected HTTP/1.1 response: {response:?}"))?;
                Ok(ObservedResponse {
                    status,
                    version: Version::HTTP_11,
                })
            })();
            let _ = sent.send(result);
        });
        received
    }

    fn spawn_http2(
        address: SocketAddr,
        body: &'static str,
    ) -> Receiver<Result<ObservedResponse, String>> {
        let (sent, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| -> Result<ObservedResponse, String> {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| format!("the HTTP/2 client runtime cannot start: {error}"))?;
                runtime.block_on(async move {
                    let stream = tokio::net::TcpStream::connect(address)
                        .await
                        .map_err(|error| format!("the HTTP/2 client cannot connect: {error}"))?;
                    let (mut sender, connection) =
                        hyper::client::conn::http2::Builder::new(TokioExecutor::new())
                            .handshake(TokioIo::new(stream))
                            .await
                            .map_err(|error| format!("the HTTP/2 handshake fails: {error}"))?;
                    tokio::spawn(async move {
                        let _ = connection.await;
                    });
                    let request = Request::builder()
                        .method("POST")
                        .uri("http://localhost/hook")
                        .version(Version::HTTP_2)
                        .header(http::header::CONTENT_TYPE, "text/plain")
                        .body(Full::new(Bytes::copy_from_slice(body.as_bytes())))
                        .map_err(|error| format!("the HTTP/2 request is invalid: {error}"))?;
                    let response = sender
                        .send_request(request)
                        .await
                        .map_err(|error| format!("the HTTP/2 response fails: {error}"))?;
                    let status = response.status().as_u16();
                    let version = response.version();
                    response
                        .into_body()
                        .collect()
                        .await
                        .map_err(|error| format!("the HTTP/2 body fails: {error}"))?;
                    Ok(ObservedResponse { status, version })
                })
            })();
            let _ = sent.send(result);
        });
        received
    }

    #[test]
    fn http1_and_http2_ack_only_after_commit_and_close_the_same_call_shape() {
        let plan = server_plan();
        let node = crate::node::Path::new(["server"]).expect("the server path is canonical");
        let mut server = HttpServer::try_bind(&plan, &[node], HttpServerConfig::default())
            .expect("the hosted server binds before execution");
        let address = server.local_addresses()[0];
        let mut vm = Vm::boot(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));

        let http1 = spawn_http1(address, "one");
        admit_and_close(
            &mut server,
            &mut vm,
            &mut journal,
            &http1,
            Version::HTTP_11,
            "one",
        );

        let http2 = spawn_http2(address, "two");
        admit_and_close(
            &mut server,
            &mut vm,
            &mut journal,
            &http2,
            Version::HTTP_2,
            "two",
        );
    }

    #[test]
    fn respond_returns_the_journaled_graph_body_and_closes_exactly_once() {
        let plan = respond_plan();
        let node = crate::node::Path::new(["server"]).expect("the server path is canonical");
        let mut server = HttpServer::try_bind(&plan, &[node], HttpServerConfig::default())
            .expect("the respond server binds before execution");
        let response = spawn_http1_with_body(server.local_addresses()[0], "request");
        let proposal = wait_for_proposal(&mut server);
        let mut vm = Vm::boot(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        let IngressAdmission::Accepted(accepted) = vm
            .admit_ingress(&mut journal, &proposal)
            .expect("the request reaches causal admission")
        else {
            panic!("the request must be admitted")
        };
        let Some(InboundExchangeRef::Call(call_id)) = accepted.exchange else {
            panic!("the admitted request opens one Call")
        };
        server.decide(proposal.reference, IngressDecision::Accepted(accepted));
        assert_eq!(response.try_recv(), Err(TryRecvError::Empty));

        vm.drain(&mut journal)
            .expect("the graph produces its response");
        let ready = vm.ready_inbound_call_responses();
        let [ready] = ready.as_slice() else {
            panic!("one response must be retained")
        };
        let ready = ready.clone();
        assert_eq!(
            server.start_response(&ready),
            InboundCallResponseStart::Started
        );
        assert!(vm.mark_inbound_call_response_started(&ready));
        assert_eq!(
            response
                .recv_timeout(Duration::from_secs(3))
                .expect("the peer receives one response")
                .expect("the response is valid"),
            ObservedBodyResponse {
                status: 200,
                body: b"response".to_vec(),
            }
        );
        let terminal = wait_for_terminal(&mut server);
        assert_eq!(terminal.call_id, call_id);
        assert_eq!(vm.queue_inbound_call_events([terminal]).queued, 1);
        vm.drain(&mut journal)
            .expect("the one response cursor and Call terminal commit");
        assert!(journal.open_inbound_calls().is_empty());
    }

    #[test]
    fn the_scheduler_timeout_wins_before_an_unstarted_graph_response() {
        let plan = respond_plan();
        let node = crate::node::Path::new(["server"]).expect("the server path is canonical");
        let mut server = HttpServer::try_bind(
            &plan,
            core::slice::from_ref(&node),
            HttpServerConfig::default(),
        )
        .expect("the respond server binds before execution");
        let response = spawn_http1_with_body(server.local_addresses()[0], "request");
        let proposal = wait_for_proposal(&mut server);
        let mut vm = Vm::boot(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        let IngressAdmission::Accepted(accepted) = vm
            .admit_ingress(&mut journal, &proposal)
            .expect("the request reaches causal admission")
        else {
            panic!("the request must be admitted")
        };
        let Some(InboundExchangeRef::Call(call_id)) = accepted.exchange else {
            panic!("the admitted request opens one Call")
        };
        server.decide(proposal.reference, IngressDecision::Accepted(accepted));

        let registry = crate::connectors::DriverRegistry::default()
            .try_with(Box::new(
                HttpServerFactory::try_new(HttpServerConfig::default())
                    .expect("the standard server id is valid"),
            ))
            .expect("one hosted family registers");
        let mut bindings = crate::connectors::RuntimeBindings::try_new(&plan, &registry)
            .expect("the validated server plan binds");
        vm.advance(&mut journal, &mut bindings, 1_000_000_000)
            .expect("the request deadline advances deterministically");
        assert!(vm.ready_inbound_call_responses().is_empty());
        let timeouts = vm.ready_inbound_call_timeouts();
        let [timeout] = timeouts.as_slice() else {
            panic!("exactly one request deadline must be due")
        };
        assert_eq!(timeout.call_id, call_id);
        assert_eq!(
            server.start_timeout(timeout),
            InboundCallTimeoutStart::Started
        );
        assert!(vm.mark_inbound_call_timeout_started(timeout));
        assert_eq!(
            response
                .recv_timeout(Duration::from_secs(3))
                .expect("the peer receives one timeout response")
                .expect("the timeout response is valid"),
            ObservedBodyResponse {
                status: 504,
                body: Vec::new(),
            }
        );
        let terminal = wait_for_terminal(&mut server);
        assert_eq!(terminal.call_id, call_id);
        assert_eq!(vm.queue_inbound_call_events([terminal]).queued, 1);
        vm.drain(&mut journal)
            .expect("the timeout closes the retained response cursor once");
        assert!(journal.open_inbound_calls().is_empty());
    }

    #[cfg(feature = "hosted-http3")]
    #[test]
    fn http3_uses_the_same_retained_call_and_graph_response() {
        const CA: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../spikes/h3-quinn-http3/certs/ca.crt"
        ));
        let plan = http3_respond_plan();
        let node = crate::node::Path::new(["server"]).expect("the server path is canonical");
        let mut server = HttpServer::try_bind(&plan, &[node], HttpServerConfig::default())
            .expect("the HTTP/3 respond server binds before execution");
        let address = server.local_addresses()[0];
        let (sent, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("the HTTP/3 client runtime builds");
            let result =
                runtime.block_on(crate::connectors::network::http_client::probe_http3_server(
                    format!("https://localhost:{}/hook", address.port()),
                    b"request",
                    CA,
                ));
            let _ = sent.send(result);
        });
        let proposal = wait_for_proposal(&mut server);
        let mut vm = Vm::boot(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        let IngressAdmission::Accepted(accepted) = vm
            .admit_ingress(&mut journal, &proposal)
            .expect("the HTTP/3 request reaches causal admission")
        else {
            panic!("the HTTP/3 request must be admitted")
        };
        server.decide(proposal.reference, IngressDecision::Accepted(accepted));
        vm.drain(&mut journal)
            .expect("the graph produces the HTTP/3 response");
        let ready = vm.ready_inbound_call_responses();
        let [ready] = ready.as_slice() else {
            panic!("one HTTP/3 graph response must be retained")
        };
        let ready = ready.clone();
        assert_eq!(
            server.start_response(&ready),
            InboundCallResponseStart::Started
        );
        assert!(vm.mark_inbound_call_response_started(&ready));
        assert_eq!(
            received
                .recv_timeout(Duration::from_secs(5))
                .expect("the HTTP/3 client completes")
                .expect("the HTTP/3 response succeeds"),
            (200, b"response".to_vec()),
        );
        let terminal = wait_for_terminal(&mut server);
        assert_eq!(vm.queue_inbound_call_events([terminal]).queued, 1);
        vm.drain(&mut journal)
            .expect("the HTTP/3 Call closes exactly once");
        assert!(journal.open_inbound_calls().is_empty());
    }

    #[test]
    fn auth_codec_and_body_refusals_never_invent_an_inbound_call() {
        let plan = server_plan_with("json", "auth_bearer = \"secret\"\n    max_body = \"4B\"");
        let node = crate::node::Path::new(["server"]).expect("the server path is canonical");
        let mut server = HttpServer::try_bind(&plan, &[node], HttpServerConfig::default())
            .expect("the guarded server binds");
        let address = server.local_addresses()[0];

        for (headers, body, status) in [
            ("Content-Type: application/json\r\n", "{}", 401),
            (
                "Authorization: Bearer secret\r\nContent-Type: application/json\r\n",
                "?",
                400,
            ),
            (
                "Authorization: Bearer secret\r\nContent-Type: application/json\r\n",
                "[1,2]",
                413,
            ),
        ] {
            let response = spawn_raw_http1(address, headers, body);
            assert_eq!(
                response
                    .recv_timeout(Duration::from_secs(3))
                    .expect("the pre-admission refusal responds")
                    .expect("the loopback refusal is readable"),
                ObservedResponse {
                    status,
                    version: Version::HTTP_11,
                }
            );
            assert!(
                server.proposal().is_none(),
                "a request without a typed payload cannot open a causal Call"
            );
        }
    }

    #[test]
    fn bounded_accept_pressure_waits_in_the_protocol_instead_of_a_side_queue() {
        let plan = server_plan_with("text", "max_body = \"4B\"");
        let node = crate::node::Path::new(["server"]).expect("the server path is canonical");
        let config = HttpServerConfig::new(1, MIN_HTTP1_BUFFER_BYTES, MIN_HTTP1_BUFFER_BYTES + 4)
            .expect("one request exactly fits the physical byte bound");
        let mut server =
            HttpServer::try_bind(&plan, &[node], config).expect("the bounded server binds");
        let address = server.local_addresses()[0];
        let mut vm = Vm::boot(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));

        let first = spawn_http1(address, "one");
        let first_proposal = wait_for_proposal(&mut server);
        let second = spawn_http1(address, "two");
        assert_eq!(first.try_recv(), Err(TryRecvError::Empty));
        assert_eq!(second.try_recv(), Err(TryRecvError::Empty));

        let IngressAdmission::Accepted(accepted) = vm
            .admit_ingress(&mut journal, &first_proposal)
            .expect("the first request reaches admission")
        else {
            panic!("the first request must be admitted")
        };
        server.decide(
            first_proposal.reference,
            IngressDecision::Accepted(accepted),
        );
        assert_eq!(
            first
                .recv_timeout(Duration::from_secs(3))
                .expect("the admitted first request responds")
                .expect("the first response is readable")
                .status,
            202
        );
        let first_terminal = wait_for_terminal(&mut server);
        assert_eq!(vm.queue_inbound_call_events([first_terminal]).queued, 1);
        vm.drain(&mut journal).expect("the first Call settles");

        admit_and_close(
            &mut server,
            &mut vm,
            &mut journal,
            &second,
            Version::HTTP_11,
            "two",
        );
    }

    #[test]
    fn a_pending_request_never_masks_an_earlier_terminal_event() {
        let plan = server_plan();
        let node = crate::node::Path::new(["server"]).expect("the server path is canonical");
        let config = HttpServerConfig::new(2, DEFAULT_MAX_HEADER_BYTES, DEFAULT_BUFFERED_BYTES)
            .expect("two requests fit the physical bounds");
        let mut server =
            HttpServer::try_bind(&plan, &[node], config).expect("the concurrent server binds");
        let address = server.local_addresses()[0];
        let first = spawn_http1(address, "one");
        let second = spawn_http1(address, "two");
        let deadline = Instant::now() + Duration::from_secs(3);
        while server.pending.len() != 2 {
            server.poll_events();
            assert!(
                Instant::now() < deadline,
                "the two retained requests did not reach the Driver"
            );
            std::thread::yield_now();
        }

        let proposal = server
            .pending
            .front()
            .expect("one request is first")
            .proposal
            .clone();
        let body = proposal
            .submission
            .draft
            .data
            .materialized_value()
            .expect("the request body is materialized");
        let (accepted_response, rejected_response) = if body == &Value::Text(String::from("one")) {
            (&first, &second)
        } else {
            (&second, &first)
        };
        let mut vm = Vm::boot(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        let IngressAdmission::Accepted(accepted) = vm
            .admit_ingress(&mut journal, &proposal)
            .expect("the first retained request reaches admission")
        else {
            panic!("the first retained request must be accepted")
        };
        server.decide(proposal.reference, IngressDecision::Accepted(accepted));
        assert_eq!(
            accepted_response
                .recv_timeout(Duration::from_secs(3))
                .expect("the first response arrives")
                .expect("the first response is readable")
                .status,
            202
        );

        let terminal = wait_for_terminal(&mut server);
        assert_eq!(
            server.pending.len(),
            1,
            "the second proposal remains retained"
        );
        assert_eq!(vm.queue_inbound_call_events([terminal]).queued, 1);
        vm.drain(&mut journal).expect("the first Call settles");

        let second_reference = server
            .pending
            .front()
            .expect("the second proposal remains")
            .proposal
            .reference;
        server.decide(
            second_reference,
            IngressDecision::Rejected(IngressRejection::UnknownPort),
        );
        assert_eq!(
            rejected_response
                .recv_timeout(Duration::from_secs(3))
                .expect("the rejected response arrives")
                .expect("the rejected response is readable")
                .status,
            503
        );
    }
}
