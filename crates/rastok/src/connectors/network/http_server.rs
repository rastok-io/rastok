use crate::boundary::{
    Draft, HttpServerExit, InboundAcceptance, InboundCallConnectorDriver, InboundCallEvent,
    InboundConnectorDriver, InboundDecision, InboundEnvelopeProvenance, InboundExchangeRef,
    InboundProposal, InboundProposalRef, InboundRejection, InboundSubmission, SubmissionSequence,
};
use crate::connectors::http_server::bind_routes;
use crate::connectors::http_shared::{HttpVersion, decode_body};
use crate::connectors::network::pump::{
    EngineLimits, EventPoll, EventReservation, EventSender, HostedNetworkEngine, NetworkRuntime,
    ReserveError,
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
#[cfg(test)]
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

/// Statically linked hosted implementation of `rastok/http-server@1`.
#[derive(Debug)]
pub struct HttpServerFactory {
    id: crate::connector::ConnectorId,
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
            id: crate::connector::ConnectorId::parse(crate::connector::HTTP_SERVER_CONNECTOR_ID)?,
            config,
        })
    }
}

impl crate::connectors::ConnectorDriverFactory for HttpServerFactory {
    fn connector_id(&self) -> &crate::connector::ConnectorId {
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

#[derive(Debug)]
struct ListenerGroup {
    certificate: Option<Vec<u8>>,
    private_key: Option<Vec<u8>>,
    routes: Vec<BoundRoute>,
    versions: BTreeSet<&'static str>,
}

#[derive(Debug)]
struct LiveProposal {
    proposal: InboundProposal,
    decision: tokio::sync::oneshot::Sender<ServerDecision>,
    _reservation: EventReservation,
}

#[derive(Clone, Copy, Debug)]
enum ServerDecision {
    Accepted(crate::boundary::InboundCallRef),
    Rejected(InboundRejection),
}

#[derive(Debug)]
enum ServerEvent {
    Proposal(LiveProposal),
    Terminal(InboundCallEvent),
}

/// Real hosted HTTP/1.1–2 inbound Driver. HTTP/3 is a later H5 checkpoint;
/// no protocol library type escapes this private module.
#[derive(Debug)]
struct HttpServer<'plan> {
    _plan: &'plan crate::plan::ValidatedExecutionPlan,
    engine: HostedNetworkEngine<(), (), ServerEvent>,
    pending: VecDeque<LiveProposal>,
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
                .filter_map(|version| match version {
                    HttpVersion::Http1 => Some("http/1.1"),
                    HttpVersion::Http2 => Some("http/2"),
                    HttpVersion::Http3 => None,
                })
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
                node: route.node,
                path: route.path,
                response_headers,
                versions: route.versions,
            });
        }

        let mut listeners = Vec::new();
        let mut local_addresses = Vec::new();
        for (listen, group) in groups {
            let listener = TcpListener::bind(&listen).map_err(|error| {
                format!("the HttpServer listener `{listen}` cannot bind: {error}")
            })?;
            listener.set_nonblocking(true).map_err(|error| {
                format!("the HttpServer listener `{listen}` is not async: {error}")
            })?;
            local_addresses.push(listener.local_addr().map_err(|error| {
                format!("the HttpServer listener `{listen}` has no address: {error}")
            })?);
            let tls = match (group.certificate, group.private_key) {
                (Some(certificate), Some(private_key)) => {
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
                }
                (None, None) => None,
                _ => {
                    return Err(format!(
                        "the HttpServer TLS pair at `{listen}` is incomplete"
                    ));
                }
            };
            listeners.push(ListenerBinding {
                listener,
                routes: Arc::new(group.routes),
                tls,
            });
        }

        let limits = config.engine_limits().map_err(|error| error.to_string())?;
        let connections = Arc::new(tokio::sync::Semaphore::new(config.max_in_flight));
        let requests = Arc::new(tokio::sync::Semaphore::new(config.max_in_flight));
        let references = Arc::new(AtomicU64::new(1));
        let engine = HostedNetworkEngine::start(
            "rastok-http-server",
            limits,
            move |mut runtime: NetworkRuntime<(), (), ServerEvent>| async move {
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
                while runtime.recv().await.is_some() {}
            },
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            _plan: plan,
            engine,
            pending: VecDeque::new(),
            terminals: Vec::new(),
            #[cfg(test)]
            local_addresses,
        })
    }

    fn poll_events(&mut self) {
        loop {
            match self.engine.try_event() {
                EventPoll::Event(ServerEvent::Proposal(proposal)) => {
                    self.pending.push_back(proposal);
                }
                EventPoll::Event(ServerEvent::Terminal(event)) => self.terminals.push(event),
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
    fn proposal(&mut self) -> Option<InboundProposal> {
        self.poll_events();
        self.pending.front().map(|pending| pending.proposal.clone())
    }

    fn decide(&mut self, reference: InboundProposalRef, decision: InboundDecision) {
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
            InboundDecision::Accepted(InboundAcceptance {
                exchange: Some(InboundExchangeRef::Call(call_ref)),
                ..
            }) => ServerDecision::Accepted(call_ref),
            InboundDecision::Accepted(_) => ServerDecision::Rejected(InboundRejection::UnknownPort),
            InboundDecision::Rejected(reason) => ServerDecision::Rejected(reason),
        };
        if let Err(ServerDecision::Accepted(call_ref)) = pending.decision.send(decision) {
            self.terminals.push(InboundCallEvent {
                call_ref,
                completion: crate::boundary::Completion::http_server_exit(
                    HttpServerExit::PeerDisconnected,
                ),
            });
        }
    }

    fn call(&mut self) -> Option<&mut dyn InboundCallConnectorDriver> {
        Some(self)
    }
}

impl InboundCallConnectorDriver for HttpServer<'_> {
    fn events(&mut self) -> Vec<InboundCallEvent> {
        self.poll_events();
        self.terminals.sort_by_key(|event| event.call_ref);
        std::mem::take(&mut self.terminals)
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
        MetaValue::Scalar(Scalar::Text(parts.method.as_str().to_string())),
    );
    ext.insert(
        String::from("http_uri"),
        MetaValue::Scalar(Scalar::Text(parts.uri.to_string())),
    );
    ext.insert(
        String::from("http_version"),
        MetaValue::Scalar(Scalar::Text(String::from(version.observation()))),
    );
    let proposal = InboundProposal {
        reference,
        submission: InboundSubmission {
            provenance: InboundEnvelopeProvenance::single(route.node.clone()),
            port: crate::node::Port::conventional_out(),
            draft: Draft { data, ext },
            sequence: SubmissionSequence::Next,
        },
        session: None,
    };
    let (decision, accepted) = tokio::sync::oneshot::channel();
    let event = ServerEvent::Proposal(LiveProposal {
        proposal,
        decision,
        _reservation: reservation,
    });
    if events.emit(event, 0).await.is_err() {
        return Ok(simple_response(StatusCode::SERVICE_UNAVAILABLE));
    }
    match accepted.await {
        Ok(ServerDecision::Accepted(call_ref)) => {
            let mut response = simple_response(StatusCode::ACCEPTED);
            *response.headers_mut() = route.response_headers;
            let event = ServerEvent::Terminal(InboundCallEvent {
                call_ref,
                completion: crate::boundary::Completion::http_server_exit(
                    HttpServerExit::Completed { status: 202 },
                ),
            });
            match events.emit(event, 0).await {
                Ok(()) => Ok(response),
                Err(_) => Ok(simple_response(StatusCode::SERVICE_UNAVAILABLE)),
            }
        }
        Ok(ServerDecision::Rejected(reason)) => Ok(simple_response(rejection_status(reason))),
        Err(_) => Ok(simple_response(StatusCode::SERVICE_UNAVAILABLE)),
    }
}

fn next_reference(references: &AtomicU64) -> Option<InboundProposalRef> {
    references
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .ok()
        .map(InboundProposalRef::new)
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

fn rejection_status(rejection: InboundRejection) -> StatusCode {
    match rejection {
        InboundRejection::UnknownPort | InboundRejection::SessionUnavailable => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        InboundRejection::InvalidPayload | InboundRejection::InvalidMetadata => {
            StatusCode::BAD_REQUEST
        }
    }
}

fn simple_response(status: StatusCode) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::new()));
    *response.status_mut() = status;
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
    use crate::boundary::{InboundAdmission, InboundExchangeRef};
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
  server -> relay -> .
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

    fn wait_for_proposal(server: &mut HttpServer<'_>) -> InboundProposal {
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
        let InboundAdmission::Accepted(accepted) = vm
            .admit_inbound(journal, &proposal)
            .expect("the request reaches atomic admission")
        else {
            panic!("the valid request must be accepted")
        };
        let Some(InboundExchangeRef::Call(call_ref)) = accepted.exchange else {
            panic!("an accepted server request opens one Call")
        };
        server.decide(proposal.reference, InboundDecision::Accepted(accepted));
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
        assert_eq!(terminal.call_ref, call_ref);
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
                            == Some(&crate::journal::InboundExchangeExitRecord::Call(call_ref))
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

        let InboundAdmission::Accepted(accepted) = vm
            .admit_inbound(&mut journal, &first_proposal)
            .expect("the first request reaches admission")
        else {
            panic!("the first request must be admitted")
        };
        server.decide(
            first_proposal.reference,
            InboundDecision::Accepted(accepted),
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
        let InboundAdmission::Accepted(accepted) = vm
            .admit_inbound(&mut journal, &proposal)
            .expect("the first retained request reaches admission")
        else {
            panic!("the first retained request must be accepted")
        };
        server.decide(proposal.reference, InboundDecision::Accepted(accepted));
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
            InboundDecision::Rejected(InboundRejection::UnknownPort),
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
