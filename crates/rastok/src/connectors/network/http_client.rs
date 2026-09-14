use crate::boundary::{
    CallConnectorDriver, CallRef, CallStart, Completion, HttpClientExit, Intent,
    OutboundConnectorDriver, RequestPreparation, Response, SchedulerCompletion,
};
use crate::connectors::http_client::{
    HttpBearer, HttpClientPlanBinding, HttpClientRequestBindings, HttpRequest, decode_request,
};
use crate::connectors::http_shared::{HttpVersion, decode_body};
use crate::connectors::network::pump::{
    EngineInput, EngineLimits, HostedNetworkEngine, NetworkRuntime, SubmitError,
};
use crate::envelope::{Data, Envelope, MetaValue, Namespace, Scalar};
use crate::value::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[cfg(feature = "hosted-http12")]
use bytes::Bytes;
#[cfg(feature = "hosted-http12")]
use http_body_util::{BodyExt, Full};
#[cfg(feature = "hosted-http3")]
use quiche::h3::NameValue;

const DEFAULT_MAX_IN_FLIGHT: usize = 64;
const DEFAULT_MAX_HEADER_BYTES: usize = 64 * 1024;
const DEFAULT_BUFFERED_BYTES: usize = 16 * 1024 * 1024;
const MAX_EVENTS_PER_POLL: usize = 64;
const MIN_HTTP1_BUFFER_BYTES: usize = 8 * 1024;

/// Physical bounds of one hosted HttpClient Driver.
///
/// These values never enter a plan or identity. Binding proves that every
/// declared response can cross them before any network thread or socket
/// exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HttpClientConfig {
    max_in_flight: usize,
    max_header_bytes: usize,
    buffered_bytes: usize,
}

impl HttpClientConfig {
    /// Constructs finite hosted-client bounds.
    ///
    /// # Errors
    ///
    /// Zero capacities and byte budgets outside Tokio's semaphore range are
    /// rejected before a Driver can bind. An HTTP/1.1-capable build also
    /// enforces Hyper's 8 KiB parser-buffer floor.
    pub fn new(
        max_in_flight: usize,
        max_header_bytes: usize,
        buffered_bytes: usize,
    ) -> Result<Self, HttpClientConfigError> {
        if max_in_flight == 0 {
            return Err(HttpClientConfigError(
                "the HttpClient in-flight bound must be positive",
            ));
        }
        if max_header_bytes == 0
            || (cfg!(feature = "hosted-http12") && max_header_bytes < MIN_HTTP1_BUFFER_BYTES)
        {
            return Err(HttpClientConfigError(
                "the HttpClient header bound must be positive and cover HTTP/1.1's 8 KiB \
                 minimum when that backend is compiled",
            ));
        }
        if buffered_bytes == 0 || u32::try_from(buffered_bytes).is_err() {
            return Err(HttpClientConfigError(
                "the HttpClient byte budget must be positive and fit a u32 semaphore",
            ));
        }
        Ok(Self {
            max_in_flight,
            max_header_bytes,
            buffered_bytes,
        })
    }

    fn engine_limits(self) -> Result<EngineLimits, HttpClientConfigError> {
        EngineLimits::new(
            self.max_in_flight,
            self.max_in_flight,
            self.max_in_flight,
            self.buffered_bytes,
        )
        .map_err(|_| HttpClientConfigError("the HttpClient engine bounds are invalid"))
    }
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            max_in_flight: DEFAULT_MAX_IN_FLIGHT,
            max_header_bytes: DEFAULT_MAX_HEADER_BYTES,
            buffered_bytes: DEFAULT_BUFFERED_BYTES,
        }
    }
}

/// Invalid physical HttpClient capacity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HttpClientConfigError(&'static str);

impl fmt::Display for HttpClientConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for HttpClientConfigError {}

/// Statically linked hosted implementation of `rastok/http-client@2`.
#[derive(Debug)]
pub struct HttpClientFactory {
    id: crate::connector::ContractId,
    config: HttpClientConfig,
}

impl HttpClientFactory {
    /// Constructs a hosted client factory without observing the network.
    ///
    /// # Errors
    ///
    /// The built-in Connector id ceased to be canonical.
    pub fn try_new(config: HttpClientConfig) -> Result<Self, crate::connector::ConnectorIdError> {
        Ok(Self {
            id: crate::connector::ContractId::parse(crate::connector::HTTP_CLIENT_CONNECTOR_ID)?,
            config,
        })
    }
}

impl crate::connectors::ConnectorDriverFactory for HttpClientFactory {
    fn connector_id(&self) -> &crate::connector::ContractId {
        &self.id
    }

    fn roles(&self) -> crate::connector::ConnectorRoles {
        crate::connector::ConnectorRoles::Outbound
    }

    fn bind<'plan>(
        &self,
        plan: &'plan crate::plan::ValidatedExecutionPlan,
        nodes: &[crate::node::Path],
    ) -> Result<
        Box<dyn crate::connectors::BoundConnectorDriver + 'plan>,
        crate::connectors::DriverFactoryError,
    > {
        HttpClient::try_bind(plan, nodes, self.config)
            .map(|driver| {
                Box::new(driver) as Box<dyn crate::connectors::BoundConnectorDriver + 'plan>
            })
            .map_err(crate::connectors::DriverFactoryError::new)
    }

    fn bind_with_boot<'plan>(
        &self,
        plan: &'plan crate::plan::ValidatedExecutionPlan,
        nodes: &[crate::node::Path],
        boot: &crate::deployment::BootBindings,
    ) -> Result<
        Box<dyn crate::connectors::BoundConnectorDriver + 'plan>,
        crate::connectors::DriverFactoryError,
    > {
        HttpClient::try_bind_with_boot(plan, nodes, self.config, boot)
            .map(|driver| {
                Box::new(driver) as Box<dyn crate::connectors::BoundConnectorDriver + 'plan>
            })
            .map_err(crate::connectors::DriverFactoryError::new)
    }
}

#[derive(Clone, Debug)]
struct TlsMaterial {
    ca: Option<Vec<u8>>,
    certificate: Option<Vec<u8>>,
    private_key: Option<Vec<u8>>,
}

impl TlsMaterial {
    fn identity(&self) -> Option<crate::connectors::network::tls::TlsIdentity<'_>> {
        Some(crate::connectors::network::tls::TlsIdentity {
            certificate_chain: self.certificate.as_deref()?,
            private_key: self.private_key.as_deref()?,
        })
    }

    #[cfg(feature = "hosted-http12")]
    fn cache_key(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        for value in [
            self.ca.as_deref(),
            self.certificate.as_deref(),
            self.private_key.as_deref(),
        ] {
            match value {
                Some(value) => {
                    hasher.update(&[1]);
                    hasher.update(&(value.len() as u64).to_le_bytes());
                    hasher.update(value);
                }
                None => {
                    hasher.update(&[0]);
                }
            }
        }
        *hasher.finalize().as_bytes()
    }
}

#[derive(Debug)]
struct ClientWork {
    call_ref: CallRef,
    request: HttpRequest,
    binding: HttpClientPlanBinding,
    tls: TlsMaterial,
    cancel: tokio::sync::watch::Receiver<bool>,
}

#[derive(Clone, Copy, Debug)]
struct ClientControl(CallRef);

#[derive(Debug)]
struct ClientEvent {
    response: Response,
}

#[derive(Debug)]
enum CallResult {
    Response {
        status: u16,
        version: HttpVersion,
        headers: BTreeMap<String, Vec<Vec<u8>>>,
        body: Vec<u8>,
    },
    Exit(HttpClientExit),
    Cancelled,
}

#[derive(Debug)]
struct AttemptFailure {
    exit: HttpClientExit,
    request_visible: bool,
}

impl AttemptFailure {
    const fn before_request(exit: HttpClientExit) -> Self {
        Self {
            exit,
            request_visible: false,
        }
    }

    const fn after_request(exit: HttpClientExit) -> Self {
        Self {
            exit,
            request_visible: true,
        }
    }
}

#[cfg(feature = "hosted-http12")]
type H2Sender = hyper::client::conn::http2::SendRequest<Full<Bytes>>;

#[cfg(feature = "hosted-http12")]
type H2Pool = std::sync::Arc<tokio::sync::Mutex<BTreeMap<String, H2Sender>>>;

/// The real hosted HttpClient Driver.
#[derive(Debug)]
struct HttpClient<'plan> {
    _plan: &'plan crate::plan::ValidatedExecutionPlan,
    bindings: HttpClientRequestBindings,
    boot_secrets: BTreeMap<crate::deployment::SecretRef, crate::deployment::SecretMaterial>,
    plans: BTreeMap<crate::node::Path, HttpClientPlanBinding>,
    config: HttpClientConfig,
    engine: HostedNetworkEngine<ClientWork, ClientControl, ClientEvent>,
    cancellations: BTreeMap<CallRef, tokio::sync::watch::Sender<bool>>,
    immediate: Vec<Response>,
}

impl crate::connectors::BoundConnectorDriver for HttpClient<'_> {
    fn inbound(&mut self) -> Option<&mut dyn crate::boundary::InboundConnectorDriver> {
        None
    }

    fn outbound(&mut self) -> Option<&mut dyn OutboundConnectorDriver> {
        Some(self)
    }
}

impl<'plan> HttpClient<'plan> {
    fn try_bind(
        plan: &'plan crate::plan::ValidatedExecutionPlan,
        nodes: &[crate::node::Path],
        config: HttpClientConfig,
    ) -> Result<Self, String> {
        Self::try_bind_with_boot(
            plan,
            nodes,
            config,
            &crate::deployment::BootBindings::empty(),
        )
    }

    fn try_bind_with_boot(
        plan: &'plan crate::plan::ValidatedExecutionPlan,
        nodes: &[crate::node::Path],
        config: HttpClientConfig,
        boot: &crate::deployment::BootBindings,
    ) -> Result<Self, String> {
        let bindings = HttpClientRequestBindings::try_bind_with_boot(plan, nodes, boot)
            .map_err(|error| error.to_string())?;
        let mut boot_secrets = BTreeMap::new();
        for reference in bindings.secret_references() {
            let material = boot.secret(reference).ok_or_else(|| {
                format!("secret `{}` disappeared during binding", reference.as_str())
            })?;
            boot_secrets.insert(reference.clone(), material.clone());
        }
        let expected: BTreeSet<_> = nodes.iter().cloned().collect();
        let actual: BTreeSet<_> = bindings.specs.keys().cloned().collect();
        if expected != actual {
            return Err(String::from(
                "the HttpClient binder received an incomplete connector family",
            ));
        }
        let mut plans = BTreeMap::new();
        for path in nodes {
            let binding = bindings.plan_binding(path).ok_or_else(|| {
                format!("the HttpClient plan settings at `{path}` are unresolved")
            })?;
            let body = usize::try_from(binding.max_body).map_err(|_| {
                format!("the HttpClient body bound at `{path}` is not representable")
            })?;
            let response_bytes = body
                .checked_add(config.max_header_bytes)
                .ok_or_else(|| format!("the HttpClient response bound at `{path}` overflows"))?;
            if response_bytes > config.buffered_bytes {
                return Err(format!(
                    "the HttpClient response bound at `{path}` needs {response_bytes} bytes, but \
                     this Driver supplies {}",
                    config.buffered_bytes
                ));
            }
            if !binding.versions.iter().copied().any(version_available) {
                return Err(format!(
                    "the HttpClient at `{path}` offers no HTTP version compiled into this Driver"
                ));
            }
            plans.insert(path.clone(), binding);
        }
        let limits = config.engine_limits().map_err(|error| error.to_string())?;
        #[cfg(feature = "hosted-http12")]
        let h2_pool = H2Pool::default();
        let in_flight = std::sync::Arc::new(tokio::sync::Semaphore::new(config.max_in_flight));
        let engine = HostedNetworkEngine::start(
            "rastok-http-client",
            limits,
            move |mut runtime: NetworkRuntime<ClientWork, ClientControl, ClientEvent>| async move {
                while let Some(input) = runtime.recv().await {
                    match input {
                        EngineInput::Control(ClientControl(_call_ref)) => {
                            // The watch carried by the work item is the wake-up
                            // authority. This independent finite lane preserves
                            // progress when the work queue is saturated.
                        }
                        EngineInput::Work(work) => {
                            let mut events = runtime.event_sender();
                            #[cfg(feature = "hosted-http12")]
                            let h2_pool = h2_pool.clone();
                            let Ok(in_flight) = in_flight.clone().acquire_owned().await else {
                                break;
                            };
                            tokio::spawn(async move {
                                let _in_flight = in_flight;
                                let call_ref = work.value().call_ref;
                                let result = execute_call(
                                    work.value(),
                                    config.max_header_bytes,
                                    limits,
                                    #[cfg(feature = "hosted-http12")]
                                    &h2_pool,
                                )
                                .await;
                                let Some((completion, bytes)) =
                                    completion_for(result, &work.value().request)
                                else {
                                    return;
                                };
                                // Work bytes stay charged for the whole network
                                // operation, then yield their permits before the
                                // result competes for the same finite budget.
                                let _completed_work = work.into_inner();
                                let event = ClientEvent {
                                    response: Response {
                                        call_ref,
                                        completion,
                                    },
                                };
                                let _ = events.emit(event, bytes).await;
                            });
                        }
                    }
                }
            },
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            _plan: plan,
            bindings,
            boot_secrets,
            plans,
            config,
            engine,
            cancellations: BTreeMap::new(),
            immediate: Vec::new(),
        })
    }

    fn setting_value(
        &self,
        connector: &crate::node::Path,
        resolved: &BTreeMap<String, Value>,
        name: &str,
    ) -> Option<Value> {
        let spec = self.bindings.specs.get(connector)?;
        self.bindings.value(connector, spec, resolved, name)
    }

    fn tls_material(
        &self,
        connector: &crate::node::Path,
        resolved: &BTreeMap<String, Value>,
    ) -> Option<TlsMaterial> {
        let bytes = |name| match self.setting_value(connector, resolved, name) {
            Some(Value::Bytes(value)) => Some(Some(value)),
            None => Some(None),
            _ => None,
        };
        let material = TlsMaterial {
            ca: bytes("tls_ca")?,
            certificate: bytes("tls_certificate")?,
            private_key: bytes("tls_private_key")?,
        };
        if material.certificate.is_some() != material.private_key.is_some() {
            return None;
        }
        Some(material)
    }

    fn immediate(&mut self, call_ref: CallRef, exit: HttpClientExit) {
        self.immediate.push(Response {
            call_ref,
            completion: Completion::http_client_exit(exit),
        });
    }

    fn resolve_bearer(&self, request: &mut HttpRequest) -> bool {
        let Some(HttpBearer::Secret(reference)) = request.auth_bearer.as_ref() else {
            return true;
        };
        let Some(material) = self.boot_secrets.get(reference) else {
            return false;
        };
        let Ok(token) = core::str::from_utf8(material.expose_to_driver()) else {
            return false;
        };
        request.auth_bearer = Some(HttpBearer::Inline(token.to_string()));
        true
    }
}

impl CallConnectorDriver for HttpClient<'_> {
    fn prepare_request(
        &mut self,
        connector: &crate::node::Path,
        data: &Data,
        resolved_settings: &BTreeMap<String, Value>,
    ) -> RequestPreparation {
        match self.bindings.prepare(connector, data, resolved_settings) {
            Ok(prepared) => {
                let Some(request) = decode_request(&prepared) else {
                    return RequestPreparation::Rejected(Completion::http_client_exit(
                        HttpClientExit::ProtocolFailed,
                    ));
                };
                let mut physical_request = request.clone();
                if !self.resolve_bearer(&mut physical_request) {
                    return RequestPreparation::Rejected(Completion::http_client_exit(
                        HttpClientExit::ProtocolFailed,
                    ));
                }
                let Some(binding) = self.plans.get(connector) else {
                    return RequestPreparation::Rejected(Completion::http_client_exit(
                        HttpClientExit::ProtocolFailed,
                    ));
                };
                if u64::try_from(request.body.len()).map_or(true, |len| len > binding.max_body) {
                    return RequestPreparation::Rejected(Completion::http_client_exit(
                        HttpClientExit::BodyTooLarge {
                            limit: binding.max_body,
                        },
                    ));
                }
                if request_head_bytes(&physical_request)
                    .is_none_or(|bytes| bytes > self.config.max_header_bytes)
                {
                    return RequestPreparation::Rejected(Completion::http_client_exit(
                        HttpClientExit::ProtocolFailed,
                    ));
                }
                if prepared.bytes().len() > self.config.buffered_bytes {
                    return RequestPreparation::Rejected(Completion::http_client_exit(
                        HttpClientExit::ProtocolFailed,
                    ));
                }
                RequestPreparation::Prepared(prepared)
            }
            Err(rejection) => RequestPreparation::Rejected(rejection.completion()),
        }
    }

    fn scheduler_completion(
        &mut self,
        _connector: &crate::node::Path,
        completion: SchedulerCompletion,
    ) -> Completion {
        Completion::http_client_exit(match completion {
            SchedulerCompletion::TimeoutExpired => HttpClientExit::TimeoutExpired,
            SchedulerCompletion::OutcomeUnknown => HttpClientExit::OutcomeUnknown,
        })
    }

    fn cancel(&mut self, call_ref: &CallRef) {
        if let Some(cancel) = self.cancellations.remove(call_ref) {
            let _ = cancel.send(true);
        }
        let _ = self.engine.try_control(ClientControl(*call_ref));
    }

    fn start(&mut self, intent: &Intent, _request: &Envelope) -> CallStart {
        let Some(mut request) = decode_request(&intent.prepared) else {
            self.immediate(intent.call_ref, HttpClientExit::ProtocolFailed);
            return CallStart::Started;
        };
        if !self.resolve_bearer(&mut request) {
            self.immediate(intent.call_ref, HttpClientExit::ProtocolFailed);
            return CallStart::Started;
        }
        let Some(binding) = self.plans.get(&intent.connector_path).cloned() else {
            self.immediate(intent.call_ref, HttpClientExit::ProtocolFailed);
            return CallStart::Started;
        };
        let Some(tls) = self.tls_material(&intent.connector_path, &intent.resolved_settings) else {
            self.immediate(intent.call_ref, HttpClientExit::TlsFailed);
            return CallStart::Started;
        };
        let (cancel, cancel_rx) = tokio::sync::watch::channel(false);
        let work = ClientWork {
            call_ref: intent.call_ref,
            request,
            binding,
            tls,
            cancel: cancel_rx,
        };
        match self.engine.try_submit(work, intent.prepared.bytes().len()) {
            Ok(()) => {
                self.cancellations.insert(intent.call_ref, cancel);
                CallStart::Started
            }
            Err(SubmitError::Full(_work) | SubmitError::ByteBudget(_work)) => CallStart::Pending,
            Err(SubmitError::Stopped(_work)) => {
                self.immediate(intent.call_ref, HttpClientExit::ConnectFailed);
                CallStart::Started
            }
            Err(SubmitError::Oversized { value: _work, .. }) => {
                self.immediate(intent.call_ref, HttpClientExit::ProtocolFailed);
                CallStart::Started
            }
        }
    }

    fn events(&mut self) -> Vec<Response> {
        let mut responses = std::mem::take(&mut self.immediate);
        for event in self.engine.drain_events(MAX_EVENTS_PER_POLL).events {
            self.cancellations.remove(&event.response.call_ref);
            responses.push(event.response);
        }
        responses.sort_by_key(|response| response.call_ref);
        responses
    }
}

const fn version_available(version: HttpVersion) -> bool {
    match version {
        HttpVersion::Http1 | HttpVersion::Http2 => cfg!(feature = "hosted-http12"),
        HttpVersion::Http3 => cfg!(feature = "hosted-http3"),
    }
}

async fn execute_call(
    work: &ClientWork,
    max_header_bytes: usize,
    _limits: EngineLimits,
    #[cfg(feature = "hosted-http12")] h2_pool: &H2Pool,
) -> CallResult {
    if *work.cancel.borrow() {
        return CallResult::Cancelled;
    }
    let uri: http::Uri = match work.request.url.parse() {
        Ok(uri) => uri,
        Err(_) => return CallResult::Exit(HttpClientExit::ProtocolFailed),
    };
    let Some(scheme) = uri.scheme_str() else {
        return CallResult::Exit(HttpClientExit::ProtocolFailed);
    };
    if !matches!(scheme, "http" | "https") || uri.authority().is_none() {
        return CallResult::Exit(HttpClientExit::ProtocolFailed);
    }
    if scheme == "http"
        && (work.tls.ca.is_some()
            || work.tls.certificate.is_some()
            || work.tls.private_key.is_some())
    {
        return CallResult::Exit(HttpClientExit::ProtocolFailed);
    }
    let mut last = HttpClientExit::ProtocolFailed;
    for version in work
        .binding
        .versions
        .iter()
        .copied()
        .filter(|version| version_available(*version))
    {
        let attempt = match version {
            #[cfg(feature = "hosted-http12")]
            HttpVersion::Http1 | HttpVersion::Http2 => {
                execute_hyper(
                    work,
                    &uri,
                    version,
                    max_header_bytes,
                    _limits.buffered_bytes(),
                    h2_pool,
                )
                .await
            }
            #[cfg(feature = "hosted-http3")]
            HttpVersion::Http3 => execute_h3(work, &uri, max_header_bytes, _limits).await,
            #[allow(unreachable_patterns)]
            _ => continue,
        };
        match attempt {
            Ok(response) => return response,
            Err(AttemptFailure {
                exit: HttpClientExit::OutcomeUnknown,
                ..
            }) if *work.cancel.borrow() => return CallResult::Cancelled,
            Err(failure) if failure.request_visible => return CallResult::Exit(failure.exit),
            Err(failure) => last = failure.exit,
        }
    }
    CallResult::Exit(last)
}

fn completion_for(result: CallResult, request: &HttpRequest) -> Option<(Completion, usize)> {
    match result {
        CallResult::Cancelled => None,
        CallResult::Exit(exit) => Some((Completion::http_client_exit(exit), 0)),
        CallResult::Response {
            status,
            version,
            headers,
            body,
        } => {
            let Some(observed_bytes) = response_header_bytes(&headers)
                .and_then(|header_bytes| header_bytes.checked_add(body.len()))
            else {
                return Some((
                    Completion::http_client_exit(HttpClientExit::ProtocolFailed),
                    0,
                ));
            };
            let Some(data) = decode_body(&request.codec, body) else {
                return Some((
                    Completion::http_client_exit(HttpClientExit::DecodeFailed),
                    0,
                ));
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
                String::from("http_status"),
                MetaValue::Scalar(Scalar::Int(i64::from(status))),
            );
            ext.insert(
                String::from("http_version"),
                MetaValue::Scalar(Scalar::Text(String::from(version.observation()))),
            );
            Some((
                Completion::http_client(data, ext, HttpClientExit::Completed { status }),
                observed_bytes,
            ))
        }
    }
}

fn response_header_bytes(headers: &BTreeMap<String, Vec<Vec<u8>>>) -> Option<usize> {
    headers.iter().try_fold(0usize, |total, (name, values)| {
        values.iter().try_fold(total, |total, value| {
            total.checked_add(name.len())?.checked_add(value.len())
        })
    })
}

fn request_headers(request: &HttpRequest) -> Result<http::HeaderMap, AttemptFailure> {
    let mut headers = http::HeaderMap::new();
    for (name, values) in &request.headers {
        if matches!(
            name.as_str(),
            "host" | "content-length" | "transfer-encoding" | "connection" | "upgrade"
        ) {
            return Err(AttemptFailure::before_request(
                HttpClientExit::ProtocolFailed,
            ));
        }
        let name = http::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| AttemptFailure::before_request(HttpClientExit::ProtocolFailed))?;
        for value in values {
            let value = http::HeaderValue::from_bytes(value)
                .map_err(|_| AttemptFailure::before_request(HttpClientExit::ProtocolFailed))?;
            headers.append(name.clone(), value);
        }
    }
    if let Some(token) = &request.auth_bearer {
        if headers.contains_key(http::header::AUTHORIZATION) {
            return Err(AttemptFailure::before_request(
                HttpClientExit::ProtocolFailed,
            ));
        }
        let HttpBearer::Inline(token) = token else {
            return Err(AttemptFailure::before_request(
                HttpClientExit::ProtocolFailed,
            ));
        };
        let value = http::HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| AttemptFailure::before_request(HttpClientExit::ProtocolFailed))?;
        headers.insert(http::header::AUTHORIZATION, value);
    }
    if let Some(key) = &request.idempotency_key {
        const IDEMPOTENCY_KEY: &str = "idempotency-key";
        if headers.contains_key(IDEMPOTENCY_KEY) {
            return Err(AttemptFailure::before_request(
                HttpClientExit::ProtocolFailed,
            ));
        }
        let value = http::HeaderValue::from_str(key)
            .map_err(|_| AttemptFailure::before_request(HttpClientExit::ProtocolFailed))?;
        headers.insert(IDEMPOTENCY_KEY, value);
    }
    Ok(headers)
}

fn request_head_bytes(request: &HttpRequest) -> Option<usize> {
    let _: http::Method = request.method.parse().ok()?;
    let uri: http::Uri = request.url.parse().ok()?;
    let scheme = uri.scheme_str()?;
    let authority = uri.authority()?;
    if !matches!(scheme, "http" | "https") {
        return None;
    }
    let path = uri
        .path_and_query()
        .map_or("/", http::uri::PathAndQuery::as_str);
    let content_length = request.body.len().to_string();
    // Count the conservative H2/H3 field form, including framing bytes. H1's
    // request-line + Host form is no larger for the same semantic request.
    let mut total = [
        (":method", request.method.as_str()),
        (":scheme", scheme),
        (":authority", authority.as_str()),
        (":path", path),
        ("content-length", content_length.as_str()),
    ]
    .into_iter()
    .try_fold(0usize, |total, (name, value)| {
        total
            .checked_add(name.len())?
            .checked_add(value.len())?
            .checked_add(4)
    })?;
    let headers = request_headers(request).ok()?;
    for (name, value) in &headers {
        total = total
            .checked_add(name.as_str().len())?
            .checked_add(value.as_bytes().len())?
            .checked_add(4)?;
    }
    Some(total)
}

#[cfg(feature = "hosted-http12")]
trait AsyncTransport: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static {}

#[cfg(feature = "hosted-http12")]
impl<T> AsyncTransport for T where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static
{
}

#[cfg(feature = "hosted-http12")]
async fn connect_tcp(
    uri: &http::Uri,
    cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<tokio::net::TcpStream, AttemptFailure> {
    let host = uri
        .host()
        .ok_or_else(|| AttemptFailure::before_request(HttpClientExit::ProtocolFailed))?;
    let port = uri.port_u16().unwrap_or_else(|| {
        if uri.scheme_str() == Some("https") {
            443
        } else {
            80
        }
    });
    let addresses = tokio::select! {
        _ = cancel.changed() => {
            return Err(AttemptFailure::before_request(HttpClientExit::OutcomeUnknown));
        }
        result = tokio::net::lookup_host((host, port)) => result
            .map_err(|_| AttemptFailure::before_request(HttpClientExit::DnsFailed))?
            .collect::<Vec<_>>()
    };
    if addresses.is_empty() {
        return Err(AttemptFailure::before_request(HttpClientExit::DnsFailed));
    }
    for address in addresses {
        let connected = tokio::select! {
            _ = cancel.changed() => {
                return Err(AttemptFailure::before_request(HttpClientExit::OutcomeUnknown));
            }
            connected = tokio::net::TcpStream::connect(address) => connected
        };
        if let Ok(stream) = connected {
            return Ok(stream);
        }
    }
    Err(AttemptFailure::before_request(
        HttpClientExit::ConnectFailed,
    ))
}

#[cfg(feature = "hosted-http12")]
async fn transport(
    work: &ClientWork,
    uri: &http::Uri,
    version: HttpVersion,
) -> Result<Box<dyn AsyncTransport>, AttemptFailure> {
    let tls_config = if uri.scheme_str() == Some("https") {
        let mut config = crate::connectors::network::tls::rustls_client(
            work.tls.ca.as_deref(),
            work.tls.identity(),
        )
        .map_err(|_| AttemptFailure::before_request(HttpClientExit::TlsFailed))?;
        config.alpn_protocols = vec![match version {
            HttpVersion::Http1 => b"http/1.1".to_vec(),
            HttpVersion::Http2 => b"h2".to_vec(),
            HttpVersion::Http3 => {
                return Err(AttemptFailure::before_request(
                    HttpClientExit::ProtocolFailed,
                ));
            }
        }];
        Some(config)
    } else {
        None
    };
    let mut cancel = work.cancel.clone();
    let stream = connect_tcp(uri, &mut cancel).await?;
    let Some(config) = tls_config else {
        return Ok(Box::new(stream));
    };
    let server_name = rustls::pki_types::ServerName::try_from(
        uri.host()
            .ok_or_else(|| AttemptFailure::before_request(HttpClientExit::ProtocolFailed))?
            .to_string(),
    )
    .map_err(|_| AttemptFailure::before_request(HttpClientExit::TlsFailed))?;
    let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config));
    let tls = tokio::select! {
        _ = cancel.changed() => {
            return Err(AttemptFailure::before_request(HttpClientExit::OutcomeUnknown));
        }
        connected = connector.connect(server_name, stream) => connected
            .map_err(|_| AttemptFailure::before_request(HttpClientExit::TlsFailed))?
    };
    let expected = match version {
        HttpVersion::Http1 => b"http/1.1".as_slice(),
        HttpVersion::Http2 => b"h2".as_slice(),
        HttpVersion::Http3 => {
            return Err(AttemptFailure::before_request(
                HttpClientExit::ProtocolFailed,
            ));
        }
    };
    if tls.get_ref().1.alpn_protocol() != Some(expected) {
        return Err(AttemptFailure::before_request(
            HttpClientExit::ProtocolFailed,
        ));
    }
    Ok(Box::new(tls))
}

#[cfg(feature = "hosted-http12")]
fn hyper_request(
    request: &HttpRequest,
    uri: &http::Uri,
    version: HttpVersion,
) -> Result<http::Request<Full<Bytes>>, AttemptFailure> {
    let method = http::Method::from_bytes(request.method.as_bytes())
        .map_err(|_| AttemptFailure::before_request(HttpClientExit::ProtocolFailed))?;
    let target = match version {
        HttpVersion::Http1 => uri
            .path_and_query()
            .map_or_else(|| http::Uri::from_static("/"), |value| value.clone().into()),
        HttpVersion::Http2 => uri.clone(),
        HttpVersion::Http3 => {
            return Err(AttemptFailure::before_request(
                HttpClientExit::ProtocolFailed,
            ));
        }
    };
    let mut built = http::Request::builder().method(method).uri(target);
    let headers = built
        .headers_mut()
        .ok_or_else(|| AttemptFailure::before_request(HttpClientExit::ProtocolFailed))?;
    *headers = request_headers(request)?;
    if version == HttpVersion::Http1 {
        let authority = uri
            .authority()
            .ok_or_else(|| AttemptFailure::before_request(HttpClientExit::ProtocolFailed))?;
        headers.insert(
            http::header::HOST,
            http::HeaderValue::from_str(authority.as_str())
                .map_err(|_| AttemptFailure::before_request(HttpClientExit::ProtocolFailed))?,
        );
    }
    built
        .body(Full::new(Bytes::copy_from_slice(&request.body)))
        .map_err(|_| AttemptFailure::before_request(HttpClientExit::ProtocolFailed))
}

#[cfg(feature = "hosted-http12")]
async fn execute_hyper(
    work: &ClientWork,
    uri: &http::Uri,
    version: HttpVersion,
    max_header_bytes: usize,
    max_buffered_bytes: usize,
    h2_pool: &H2Pool,
) -> Result<CallResult, AttemptFailure> {
    let request = hyper_request(&work.request, uri, version)?;
    let response = match version {
        HttpVersion::Http1 => {
            let io = transport(work, uri, version).await?;
            let mut builder = hyper::client::conn::http1::Builder::new();
            builder.max_buf_size(max_header_bytes);
            builder.max_headers(max_header_bytes / 4);
            let (mut sender, connection) = builder
                .handshake(hyper_util::rt::TokioIo::new(io))
                .await
                .map_err(|_| AttemptFailure::before_request(HttpClientExit::ProtocolFailed))?;
            tokio::spawn(async move {
                let _ = connection.await;
            });
            let mut cancel = work.cancel.clone();
            tokio::select! {
                _ = cancel.changed() => {
                    return Err(AttemptFailure::after_request(HttpClientExit::OutcomeUnknown));
                }
                response = sender.send_request(request) => response
                    .map_err(|_| AttemptFailure::after_request(HttpClientExit::ProtocolFailed))?
            }
        }
        HttpVersion::Http2 => {
            let key = format!(
                "{}|{}|{:02x?}",
                uri.scheme_str().unwrap_or_default(),
                uri.authority().map_or("", http::uri::Authority::as_str),
                work.tls.cache_key(),
            );
            let mut sender = {
                let mut pool = h2_pool.lock().await;
                if let Some(sender) = pool.get(&key) {
                    sender.clone()
                } else {
                    let io = transport(work, uri, version).await?;
                    let mut builder = hyper::client::conn::http2::Builder::new(
                        hyper_util::rt::TokioExecutor::new(),
                    );
                    builder.max_header_list_size(u32::try_from(max_header_bytes).map_err(
                        |_| AttemptFailure::before_request(HttpClientExit::ProtocolFailed),
                    )?);
                    let flow_window = u32::try_from(max_buffered_bytes).map_err(|_| {
                        AttemptFailure::before_request(HttpClientExit::ProtocolFailed)
                    })?;
                    builder.initial_stream_window_size(flow_window);
                    builder.initial_connection_window_size(flow_window);
                    builder.max_send_buf_size(max_buffered_bytes);
                    let (sender, connection) = builder
                        .handshake(hyper_util::rt::TokioIo::new(io))
                        .await
                        .map_err(|_| {
                            AttemptFailure::before_request(HttpClientExit::ProtocolFailed)
                        })?;
                    tokio::spawn(async move {
                        let _ = connection.await;
                    });
                    pool.insert(key.clone(), sender.clone());
                    sender
                }
            };
            let mut cancel = work.cancel.clone();
            let sent = tokio::select! {
                _ = cancel.changed() => {
                    return Err(AttemptFailure::after_request(HttpClientExit::OutcomeUnknown));
                }
                response = sender.send_request(request) => response
            };
            match sent {
                Ok(response) => response,
                Err(_) => {
                    h2_pool.lock().await.remove(&key);
                    return Err(AttemptFailure::after_request(
                        HttpClientExit::ProtocolFailed,
                    ));
                }
            }
        }
        HttpVersion::Http3 => {
            return Err(AttemptFailure::before_request(
                HttpClientExit::ProtocolFailed,
            ));
        }
    };
    collect_hyper_response(work, response, version, max_header_bytes).await
}

#[cfg(feature = "hosted-http12")]
async fn collect_hyper_response(
    work: &ClientWork,
    response: http::Response<hyper::body::Incoming>,
    version: HttpVersion,
    max_header_bytes: usize,
) -> Result<CallResult, AttemptFailure> {
    let status = response.status().as_u16();
    let mut headers = BTreeMap::<String, Vec<Vec<u8>>>::new();
    for (name, value) in response.headers() {
        headers
            .entry(name.as_str().to_string())
            .or_default()
            .push(value.as_bytes().to_vec());
    }
    if response_header_bytes(&headers).is_none_or(|bytes| bytes > max_header_bytes) {
        return Err(AttemptFailure::after_request(
            HttpClientExit::ProtocolFailed,
        ));
    }
    let mut body = response.into_body();
    let mut bytes = Vec::new();
    let max_body = usize::try_from(work.binding.max_body).unwrap_or(usize::MAX);
    loop {
        let mut cancel = work.cancel.clone();
        let frame = tokio::select! {
            _ = cancel.changed() => {
                return Err(AttemptFailure::after_request(HttpClientExit::OutcomeUnknown));
            }
            frame = body.frame() => frame
        };
        let Some(frame) = frame else {
            break;
        };
        let frame =
            frame.map_err(|_| AttemptFailure::after_request(HttpClientExit::ProtocolFailed))?;
        if let Some(data) = frame.data_ref() {
            let observed = bytes.len().checked_add(data.len()).ok_or_else(|| {
                AttemptFailure::after_request(HttpClientExit::BodyTooLarge {
                    limit: work.binding.max_body,
                })
            })?;
            if observed > max_body {
                return Err(AttemptFailure::after_request(
                    HttpClientExit::BodyTooLarge {
                        limit: work.binding.max_body,
                    },
                ));
            }
            bytes
                .try_reserve(data.len())
                .map_err(|_| AttemptFailure::after_request(HttpClientExit::ProtocolFailed))?;
            bytes.extend_from_slice(data);
        }
        if let Some(trailers) = frame.trailers_ref() {
            for (name, value) in trailers {
                headers
                    .entry(name.as_str().to_string())
                    .or_default()
                    .push(value.as_bytes().to_vec());
            }
            if response_header_bytes(&headers).is_none_or(|bytes| bytes > max_header_bytes) {
                return Err(AttemptFailure::after_request(
                    HttpClientExit::ProtocolFailed,
                ));
            }
        }
    }
    Ok(CallResult::Response {
        status,
        version,
        headers,
        body: bytes,
    })
}

#[cfg(feature = "hosted-http3")]
async fn execute_h3(
    work: &ClientWork,
    uri: &http::Uri,
    max_header_bytes: usize,
    limits: EngineLimits,
) -> Result<CallResult, AttemptFailure> {
    if uri.scheme_str() != Some("https") {
        return Err(AttemptFailure::before_request(
            HttpClientExit::ProtocolFailed,
        ));
    }
    // Trust and identity parsing precede DNS and socket creation. A malformed
    // deployment value can therefore never weaken or partially start TLS.
    let mut config = crate::connectors::network::tls::quiche_client(
        work.tls.ca.as_deref(),
        work.tls.identity(),
        limits,
    )
    .map_err(|_| AttemptFailure::before_request(HttpClientExit::TlsFailed))?;
    let host = uri
        .host()
        .ok_or_else(|| AttemptFailure::before_request(HttpClientExit::ProtocolFailed))?;
    let port = uri.port_u16().unwrap_or(443);
    let mut cancel = work.cancel.clone();
    let addresses = tokio::select! {
        _ = cancel.changed() => {
            return Err(AttemptFailure::before_request(HttpClientExit::OutcomeUnknown));
        }
        result = tokio::net::lookup_host((host, port)) => result
            .map_err(|_| AttemptFailure::before_request(HttpClientExit::DnsFailed))?
            .collect::<Vec<_>>()
    };
    let remote = addresses
        .first()
        .copied()
        .ok_or_else(|| AttemptFailure::before_request(HttpClientExit::DnsFailed))?;
    let bind = if remote.is_ipv6() {
        "[::]:0"
    } else {
        "0.0.0.0:0"
    };
    let socket = tokio::net::UdpSocket::bind(bind)
        .await
        .map_err(|_| AttemptFailure::before_request(HttpClientExit::ConnectFailed))?;
    let local = socket
        .local_addr()
        .map_err(|_| AttemptFailure::before_request(HttpClientExit::ConnectFailed))?;
    let call_id = work.call_ref.id();
    let scid = quiche::ConnectionId::from_ref(call_id.as_bytes());
    let mut connection = quiche::connect(Some(host), &scid, local, remote, &mut config)
        .map_err(|_| AttemptFailure::before_request(HttpClientExit::TlsFailed))?;

    while !connection.is_established() {
        if connection.is_closed() {
            return Err(AttemptFailure::before_request(HttpClientExit::TlsFailed));
        }
        h3_transport_step(&socket, &mut connection, &mut cancel, false).await?;
    }
    if connection.application_proto() != b"h3" || connection.peer_cert().is_none() {
        return Err(AttemptFailure::before_request(HttpClientExit::TlsFailed));
    }
    let mut h3_config = quiche::h3::Config::new()
        .map_err(|_| AttemptFailure::before_request(HttpClientExit::ProtocolFailed))?;
    h3_config.set_max_field_section_size(max_header_bytes as u64);
    let mut h3 = quiche::h3::Connection::with_transport(&mut connection, &h3_config)
        .map_err(|_| AttemptFailure::before_request(HttpClientExit::ProtocolFailed))?;
    let headers = h3_request_headers(&work.request, uri)?;
    let body_empty = work.request.body.is_empty();
    let stream = h3
        .send_request(&mut connection, &headers, body_empty)
        .map_err(|_| AttemptFailure::before_request(HttpClientExit::ProtocolFailed))?;

    let mut response = H3ResponseState::default();
    let max_body = usize::try_from(work.binding.max_body).unwrap_or(usize::MAX);
    let mut sent = 0usize;
    while sent < work.request.body.len() {
        match h3.send_body(&mut connection, stream, &work.request.body[sent..], true) {
            Ok(0) | Err(quiche::h3::Error::Done) => {
                h3_transport_step(&socket, &mut connection, &mut cancel, true).await?;
                poll_h3_response(
                    &mut connection,
                    &mut h3,
                    stream,
                    &mut response,
                    max_header_bytes,
                    max_body,
                    work.binding.max_body,
                )?;
                if response.finished {
                    let _ = connection.stream_shutdown(stream, quiche::Shutdown::Write, 0);
                    return response.into_result();
                }
            }
            Ok(written) => {
                sent = sent
                    .checked_add(written)
                    .ok_or_else(|| AttemptFailure::after_request(HttpClientExit::ProtocolFailed))?;
            }
            Err(_) => {
                return Err(AttemptFailure::after_request(
                    HttpClientExit::ProtocolFailed,
                ));
            }
        }
    }

    loop {
        let made_progress = poll_h3_response(
            &mut connection,
            &mut h3,
            stream,
            &mut response,
            max_header_bytes,
            max_body,
            work.binding.max_body,
        )?;
        if response.finished {
            return response.into_result();
        }
        if connection.is_closed() {
            return Err(AttemptFailure::after_request(
                HttpClientExit::ProtocolFailed,
            ));
        }
        if !made_progress {
            h3_transport_step(&socket, &mut connection, &mut cancel, true).await?;
        }
    }
}

#[cfg(all(test, feature = "hosted-http3"))]
pub(super) async fn probe_http3_server(
    url: String,
    body: &[u8],
    ca: &[u8],
) -> Result<(u16, Vec<u8>), String> {
    let (_cancel_tx, cancel) = tokio::sync::watch::channel(false);
    let work = ClientWork {
        call_ref: CallRef::new(crate::envelope::Id::from_bytes([0x35; 16])),
        request: HttpRequest {
            auth_bearer: None,
            body: body.to_vec(),
            codec: String::from("bytes"),
            headers: BTreeMap::new(),
            idempotency_key: None,
            method: String::from("POST"),
            url: url.clone(),
        },
        binding: HttpClientPlanBinding {
            max_body: 1024 * 1024,
            versions: vec![HttpVersion::Http3],
        },
        tls: TlsMaterial {
            ca: Some(ca.to_vec()),
            certificate: None,
            private_key: None,
        },
        cancel,
    };
    let uri = url
        .parse()
        .map_err(|error| format!("the HTTP/3 probe URL is invalid: {error}"))?;
    let limits = EngineLimits::new(8, 2, 8, 1024 * 1024).map_err(|error| error.to_string())?;
    match execute_h3(&work, &uri, 64 * 1024, limits)
        .await
        .map_err(|error| format!("the HTTP/3 probe failed: {error:?}"))?
    {
        CallResult::Response { status, body, .. } => Ok((status, body)),
        CallResult::Exit(exit) => Err(format!("the HTTP/3 probe exited as {exit:?}")),
        CallResult::Cancelled => Err(String::from("the HTTP/3 probe was cancelled")),
    }
}

#[cfg(feature = "hosted-http3")]
#[derive(Default)]
struct H3ResponseState {
    status: Option<u16>,
    headers: BTreeMap<String, Vec<Vec<u8>>>,
    body: Vec<u8>,
    finished: bool,
}

#[cfg(feature = "hosted-http3")]
impl H3ResponseState {
    fn into_result(self) -> Result<CallResult, AttemptFailure> {
        let status = self
            .status
            .ok_or_else(|| AttemptFailure::after_request(HttpClientExit::ProtocolFailed))?;
        Ok(CallResult::Response {
            status,
            version: HttpVersion::Http3,
            headers: self.headers,
            body: self.body,
        })
    }
}

#[cfg(feature = "hosted-http3")]
fn h3_request_headers(
    request: &HttpRequest,
    uri: &http::Uri,
) -> Result<Vec<quiche::h3::Header>, AttemptFailure> {
    let method = http::Method::from_bytes(request.method.as_bytes())
        .map_err(|_| AttemptFailure::before_request(HttpClientExit::ProtocolFailed))?;
    let regular = request_headers(request)?;
    let authority = uri
        .authority()
        .ok_or_else(|| AttemptFailure::before_request(HttpClientExit::ProtocolFailed))?
        .as_str();
    let path = uri
        .path_and_query()
        .map_or("/", http::uri::PathAndQuery::as_str);
    let mut headers = vec![
        quiche::h3::Header::new(b":method", method.as_str().as_bytes()),
        quiche::h3::Header::new(b":scheme", b"https"),
        quiche::h3::Header::new(b":authority", authority.as_bytes()),
        quiche::h3::Header::new(b":path", path.as_bytes()),
    ];
    for (name, value) in &regular {
        headers.push(quiche::h3::Header::new(
            name.as_str().as_bytes(),
            value.as_bytes(),
        ));
    }
    let content_length = request.body.len().to_string();
    headers.push(quiche::h3::Header::new(
        b"content-length",
        content_length.as_bytes(),
    ));
    Ok(headers)
}

#[cfg(feature = "hosted-http3")]
async fn h3_transport_step(
    socket: &tokio::net::UdpSocket,
    connection: &mut quiche::Connection,
    cancel: &mut tokio::sync::watch::Receiver<bool>,
    request_visible: bool,
) -> Result<(), AttemptFailure> {
    flush_h3(socket, connection, request_visible).await?;
    let timeout = connection.timeout();
    let mut datagram = vec![0u8; 65_535];
    let received = match timeout {
        Some(timeout) => {
            tokio::select! {
                _ = cancel.changed() => {
                    return Err(if request_visible {
                        AttemptFailure::after_request(HttpClientExit::OutcomeUnknown)
                    } else {
                        AttemptFailure::before_request(HttpClientExit::OutcomeUnknown)
                    });
                }
                result = socket.recv_from(&mut datagram) => Some(result),
                () = tokio::time::sleep(timeout) => None,
            }
        }
        None => {
            tokio::select! {
                _ = cancel.changed() => {
                    return Err(if request_visible {
                        AttemptFailure::after_request(HttpClientExit::OutcomeUnknown)
                    } else {
                        AttemptFailure::before_request(HttpClientExit::OutcomeUnknown)
                    });
                }
                result = socket.recv_from(&mut datagram) => Some(result),
            }
        }
    };
    if let Some(received) = received {
        let (length, from) = received.map_err(|_| {
            if request_visible {
                AttemptFailure::after_request(HttpClientExit::ProtocolFailed)
            } else {
                AttemptFailure::before_request(HttpClientExit::ConnectFailed)
            }
        })?;
        datagram.truncate(length);
        let to = socket.local_addr().map_err(|_| {
            if request_visible {
                AttemptFailure::after_request(HttpClientExit::ProtocolFailed)
            } else {
                AttemptFailure::before_request(HttpClientExit::ConnectFailed)
            }
        })?;
        connection
            .recv(&mut datagram, quiche::RecvInfo { from, to })
            .map_err(|_| {
                if request_visible {
                    AttemptFailure::after_request(HttpClientExit::ProtocolFailed)
                } else {
                    AttemptFailure::before_request(HttpClientExit::TlsFailed)
                }
            })?;
    } else {
        connection.on_timeout();
        if connection.is_timed_out() {
            return Err(if request_visible {
                AttemptFailure::after_request(HttpClientExit::ProtocolFailed)
            } else {
                AttemptFailure::before_request(HttpClientExit::ConnectFailed)
            });
        }
    }
    Ok(())
}

#[cfg(feature = "hosted-http3")]
async fn flush_h3(
    socket: &tokio::net::UdpSocket,
    connection: &mut quiche::Connection,
    request_visible: bool,
) -> Result<(), AttemptFailure> {
    let mut datagram = vec![0u8; 65_535];
    loop {
        match connection.send(&mut datagram) {
            Ok((written, info)) => {
                let sent = socket
                    .send_to(&datagram[..written], info.to)
                    .await
                    .map_err(|_| {
                        if request_visible {
                            AttemptFailure::after_request(HttpClientExit::ProtocolFailed)
                        } else {
                            AttemptFailure::before_request(HttpClientExit::ConnectFailed)
                        }
                    })?;
                if sent != written {
                    return Err(if request_visible {
                        AttemptFailure::after_request(HttpClientExit::ProtocolFailed)
                    } else {
                        AttemptFailure::before_request(HttpClientExit::ConnectFailed)
                    });
                }
            }
            Err(quiche::Error::Done) => return Ok(()),
            Err(_) => {
                return Err(if request_visible {
                    AttemptFailure::after_request(HttpClientExit::ProtocolFailed)
                } else {
                    AttemptFailure::before_request(HttpClientExit::TlsFailed)
                });
            }
        }
    }
}

#[cfg(feature = "hosted-http3")]
fn poll_h3_response(
    connection: &mut quiche::Connection,
    h3: &mut quiche::h3::Connection,
    request_stream: u64,
    response: &mut H3ResponseState,
    max_header_bytes: usize,
    max_body: usize,
    declared_max_body: u64,
) -> Result<bool, AttemptFailure> {
    let mut made_progress = false;
    loop {
        match h3.poll(connection) {
            Err(quiche::h3::Error::Done) => return Ok(made_progress),
            Ok((stream, _)) if stream != request_stream => {
                return Err(AttemptFailure::after_request(
                    HttpClientExit::ProtocolFailed,
                ));
            }
            Ok((_, quiche::h3::Event::Headers { list, .. })) => {
                absorb_h3_headers(
                    &list,
                    &mut response.status,
                    &mut response.headers,
                    max_header_bytes,
                )?;
                made_progress = true;
            }
            Ok((_, quiche::h3::Event::Data)) => {
                drain_h3_body(
                    connection,
                    h3,
                    request_stream,
                    &mut response.body,
                    max_body,
                    declared_max_body,
                )?;
                made_progress = true;
            }
            Ok((_, quiche::h3::Event::Finished)) => {
                response.finished = true;
                return Ok(true);
            }
            Ok((_, quiche::h3::Event::Reset(_))) | Ok((_, quiche::h3::Event::GoAway)) | Err(_) => {
                return Err(AttemptFailure::after_request(
                    HttpClientExit::ProtocolFailed,
                ));
            }
            Ok((_, quiche::h3::Event::PriorityUpdate)) => {
                made_progress = true;
            }
        }
    }
}

#[cfg(feature = "hosted-http3")]
fn absorb_h3_headers(
    list: &[quiche::h3::Header],
    status: &mut Option<u16>,
    headers: &mut BTreeMap<String, Vec<Vec<u8>>>,
    max_header_bytes: usize,
) -> Result<(), AttemptFailure> {
    let incoming_status = match list.iter().find(|header| header.name() == b":status") {
        Some(header) => Some(
            std::str::from_utf8(header.value())
                .ok()
                .and_then(|value| value.parse::<u16>().ok())
                .filter(|value| (100..=999).contains(value))
                .ok_or_else(|| AttemptFailure::after_request(HttpClientExit::ProtocolFailed))?,
        ),
        None => None,
    };
    if incoming_status.is_some_and(|value| value < 200) {
        return Ok(());
    }
    if let Some(incoming_status) = incoming_status {
        if status.replace(incoming_status).is_some() {
            return Err(AttemptFailure::after_request(
                HttpClientExit::ProtocolFailed,
            ));
        }
    } else if status.is_none() {
        return Err(AttemptFailure::after_request(
            HttpClientExit::ProtocolFailed,
        ));
    }
    for header in list {
        if header.name().starts_with(b":") {
            if header.name() != b":status" {
                return Err(AttemptFailure::after_request(
                    HttpClientExit::ProtocolFailed,
                ));
            }
            continue;
        }
        if !header.name().iter().all(|byte| !byte.is_ascii_uppercase()) {
            return Err(AttemptFailure::after_request(
                HttpClientExit::ProtocolFailed,
            ));
        }
        let name = http::HeaderName::from_bytes(header.name())
            .map_err(|_| AttemptFailure::after_request(HttpClientExit::ProtocolFailed))?;
        http::HeaderValue::from_bytes(header.value())
            .map_err(|_| AttemptFailure::after_request(HttpClientExit::ProtocolFailed))?;
        headers
            .entry(name.as_str().to_string())
            .or_default()
            .push(header.value().to_vec());
    }
    if response_header_bytes(headers).is_none_or(|bytes| bytes > max_header_bytes) {
        return Err(AttemptFailure::after_request(
            HttpClientExit::ProtocolFailed,
        ));
    }
    Ok(())
}

#[cfg(feature = "hosted-http3")]
fn drain_h3_body(
    connection: &mut quiche::Connection,
    h3: &mut quiche::h3::Connection,
    stream: u64,
    body: &mut Vec<u8>,
    max_body: usize,
    declared_max_body: u64,
) -> Result<(), AttemptFailure> {
    let mut chunk = [0u8; 8 * 1024];
    loop {
        match h3.recv_body(connection, stream, &mut chunk) {
            Ok(0) | Err(quiche::h3::Error::Done) => return Ok(()),
            Ok(read) => {
                let observed = body.len().checked_add(read).ok_or_else(|| {
                    AttemptFailure::after_request(HttpClientExit::BodyTooLarge {
                        limit: declared_max_body,
                    })
                })?;
                if observed > max_body {
                    return Err(AttemptFailure::after_request(
                        HttpClientExit::BodyTooLarge {
                            limit: declared_max_body,
                        },
                    ));
                }
                body.try_reserve(read)
                    .map_err(|_| AttemptFailure::after_request(HttpClientExit::ProtocolFailed))?;
                body.extend_from_slice(&chunk[..read]);
            }
            Err(_) => {
                return Err(AttemptFailure::after_request(
                    HttpClientExit::ProtocolFailed,
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(url: String, body: &[u8]) -> HttpRequest {
        HttpRequest {
            auth_bearer: None,
            body: body.to_vec(),
            codec: String::from("bytes"),
            headers: BTreeMap::new(),
            idempotency_key: None,
            method: String::from("POST"),
            url,
        }
    }

    fn work(
        url: String,
        body: &[u8],
        version: HttpVersion,
        call: u8,
    ) -> (tokio::sync::watch::Sender<bool>, ClientWork) {
        let (cancel_tx, cancel) = tokio::sync::watch::channel(false);
        (
            cancel_tx,
            ClientWork {
                call_ref: CallRef::new(crate::envelope::Id::from_bytes([call; 16])),
                request: request(url, body),
                binding: HttpClientPlanBinding {
                    max_body: 1024,
                    versions: vec![version],
                },
                tls: TlsMaterial {
                    ca: None,
                    certificate: None,
                    private_key: None,
                },
                cancel,
            },
        )
    }

    #[test]
    fn physical_bounds_are_explicit_and_finite() {
        assert!(HttpClientConfig::new(1, MIN_HTTP1_BUFFER_BYTES, 16 * 1024).is_ok());
        assert!(HttpClientConfig::new(0, MIN_HTTP1_BUFFER_BYTES, 16 * 1024).is_err());
        assert!(HttpClientConfig::new(1, 0, 16 * 1024).is_err());
        assert_eq!(
            HttpClientConfig::new(1, MIN_HTTP1_BUFFER_BYTES - 1, 16 * 1024).is_err(),
            cfg!(feature = "hosted-http12"),
        );
        assert!(HttpClientConfig::new(1, MIN_HTTP1_BUFFER_BYTES, 0).is_err());
    }

    #[test]
    fn the_request_head_is_validated_and_bounded_before_network_visibility() {
        let mut request = request(String::from("https://example.test/value"), b"request");
        request.headers.insert(
            String::from("x-repeat"),
            vec![b"one".to_vec(), b"two".to_vec()],
        );
        request.auth_bearer = Some(HttpBearer::Inline(String::from("token")));
        request.idempotency_key = Some(String::from("stable"));
        let bytes = request_head_bytes(&request).expect("the canonical request head is valid");
        assert!(bytes > request.method.len() + request.url.len());

        request.url = String::from("not an absolute HTTP URL");
        assert_eq!(request_head_bytes(&request), None);

        request.url = String::from("https://example.test/value");
        request
            .headers
            .insert(String::from("host"), vec![b"forged".to_vec()]);
        assert_eq!(request_head_bytes(&request), None);
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn the_declared_body_limit_applies_before_a_large_request_can_open() {
        let version = if cfg!(feature = "hosted-http12") {
            "http/1.1"
        } else {
            "http/3"
        };
        let source = r#"
graph bounded_request : App {
  connector client : HttpClient {
    url = "http://127.0.0.1/request"
    method = "POST"
    timeout = duration("1s")
    codec = bytes
    max_body = "4B"
    versions = ["VERSION"]
    effect = idempotent ; behavior = deterministic
  }
  in -> client -> out
  client.exit -> nothing
}
"#
        .replace("VERSION", version);
        let compilation = crate::compiler::compile(
            &[source.as_str()],
            &crate::contract::ContractCatalog::try_m0().expect("the catalogue holds"),
        )
        .expect("the bounded client compiles");
        let path = crate::node::Path::new(["client"]).expect("the path is canonical");
        let mut client = HttpClient::try_bind(
            &compilation.plan,
            std::slice::from_ref(&path),
            HttpClientConfig::new(1, MIN_HTTP1_BUFFER_BYTES, 16 * 1024)
                .expect("the physical limits hold"),
        )
        .expect("the live client binds");
        let settings = BTreeMap::from([
            (
                String::from("url"),
                Value::Text(String::from("http://127.0.0.1/request")),
            ),
            (String::from("method"), Value::Text(String::from("POST"))),
        ]);

        assert_eq!(
            client.prepare_request(
                &path,
                &Data::materialized(Value::Bytes(vec![1, 2, 3, 4, 5])),
                &settings,
            ),
            RequestPreparation::Rejected(Completion::http_client_exit(
                HttpClientExit::BodyTooLarge { limit: 4 },
            )),
        );
    }

    #[test]
    fn wire_version_changes_only_the_external_observation() {
        let request = request(String::from("https://example.test/value"), b"request");
        let headers = BTreeMap::from([(
            String::from("content-type"),
            vec![b" Application/Octet-Stream ".to_vec()],
        )]);
        let completions =
            [HttpVersion::Http1, HttpVersion::Http2, HttpVersion::Http3].map(|version| {
                completion_for(
                    CallResult::Response {
                        status: 200,
                        version,
                        headers: headers.clone(),
                        body: b"same-value".to_vec(),
                    },
                    &request,
                )
                .expect("a usable response produces one completion")
                .0
            });
        let outputs = completions.map(|completion| completion.into_outputs());
        assert!(outputs.iter().all(|outputs| outputs.len() == 2));
        let expected = Data::materialized(Value::Bytes(b"same-value".to_vec()));
        assert!(outputs.iter().all(|outputs| outputs[0].data == expected));
        assert!(outputs.iter().all(|outputs| {
            outputs[0].ext.get("content_type")
                == Some(&MetaValue::Scalar(Scalar::Text(String::from(
                    "application/octet-stream",
                ))))
        }));
        let versions = outputs.map(|outputs| {
            outputs[0]
                .ext
                .get("http_version")
                .cloned()
                .expect("the selected wire version is observed")
        });
        assert_eq!(
            versions,
            [
                MetaValue::Scalar(Scalar::Text(String::from("http/1.1"))),
                MetaValue::Scalar(Scalar::Text(String::from("http/2"))),
                MetaValue::Scalar(Scalar::Text(String::from("http/3"))),
            ],
        );

        for values in [
            vec![b"not a media type".to_vec()],
            vec![b"text/plain".to_vec(), b"application/json".to_vec()],
        ] {
            let completion = completion_for(
                CallResult::Response {
                    status: 200,
                    version: HttpVersion::Http1,
                    headers: BTreeMap::from([(String::from("content-type"), values)]),
                    body: b"same-value".to_vec(),
                },
                &request,
            )
            .expect("an invalid typed projection does not discard the response")
            .0;
            let outputs = completion.into_outputs();
            assert_eq!(outputs.len(), 2);
            assert_eq!(outputs[0].ext.get("content_type"), None);
            assert!(outputs[0].ext.contains_key("http_headers"));
        }
    }

    #[cfg(feature = "hosted-http12")]
    #[test]
    #[allow(clippy::expect_used)]
    fn http1_streams_one_bounded_response_with_lossless_metadata() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("the test runtime builds");
        runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("the loopback listener binds");
            let address = listener.local_addr().expect("the listener has an address");
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.expect("one client connects");
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(
                        hyper_util::rt::TokioIo::new(stream),
                        hyper::service::service_fn(|request| async move {
                            assert_eq!(request.uri().path(), "/echo");
                            let response = http::Response::builder()
                                .status(418)
                                .header("content-type", "application/octet-stream")
                                .header("x-repeat", "a")
                                .header("x-repeat", "b")
                                .body(Full::new(Bytes::from_static(b"h1-response")))
                                .expect("the fixed response builds");
                            Ok::<_, std::convert::Infallible>(response)
                        }),
                    )
                    .await
                    .expect("the HTTP/1.1 connection completes");
            });
            let (_cancel, work) = work(
                format!("http://{address}/echo"),
                b"request",
                HttpVersion::Http1,
                1,
            );
            let pool = H2Pool::default();
            let result = execute_hyper(
                &work,
                &work.request.url.parse().expect("the URL is valid"),
                HttpVersion::Http1,
                MIN_HTTP1_BUFFER_BYTES,
                16 * 1024,
                &pool,
            )
            .await
            .expect("the HTTP/1.1 call succeeds");
            let CallResult::Response {
                status,
                version,
                headers,
                body,
            } = result
            else {
                panic!("the call must return a response");
            };
            assert_eq!(status, 418);
            assert_eq!(version, HttpVersion::Http1);
            assert_eq!(body, b"h1-response");
            assert_eq!(
                headers.get("x-repeat"),
                Some(&vec![b"a".to_vec(), b"b".to_vec()])
            );
            server.await.expect("the server task joins");
        });
    }

    #[cfg(feature = "hosted-http12")]
    #[test]
    #[allow(clippy::disallowed_types, clippy::expect_used)]
    fn http2_multiplexes_without_crossing_reverse_completions() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        const CA: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../spikes/h3-quinn-http3/certs/ca.crt"
        ));
        const CERTIFICATE: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../spikes/h3-quinn-http3/certs/server.crt"
        ));
        const PRIVATE_KEY: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../spikes/h3-quinn-http3/certs/server.key"
        ));

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("the test runtime builds");
        runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("the loopback listener binds");
            let address = listener.local_addr().expect("the listener has an address");
            let accepted = Arc::new(AtomicUsize::new(0));
            let observed = Arc::clone(&accepted);
            let identity = crate::connectors::network::tls::TlsIdentity {
                certificate_chain: CERTIFICATE,
                private_key: PRIVATE_KEY,
            };
            let mut server_config = crate::connectors::network::tls::rustls_server(identity)
                .expect("the fixture identity is valid");
            server_config.alpn_protocols = vec![b"h2".to_vec()];
            let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
            let server = tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        break;
                    };
                    observed.fetch_add(1, Ordering::SeqCst);
                    let acceptor = acceptor.clone();
                    tokio::spawn(async move {
                        let Ok(stream) = acceptor.accept(stream).await else {
                            return;
                        };
                        let service = hyper::service::service_fn(|request| async move {
                            let (delay, body) = if request.uri().path() == "/first" {
                                (30, b"first".as_slice())
                            } else {
                                (1, b"second".as_slice())
                            };
                            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                            Ok::<_, std::convert::Infallible>(http::Response::new(Full::new(
                                Bytes::copy_from_slice(body),
                            )))
                        });
                        let _ = hyper::server::conn::http2::Builder::new(
                            hyper_util::rt::TokioExecutor::new(),
                        )
                        .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                        .await;
                    });
                }
            });
            let pool = H2Pool::default();
            let (_first_cancel, mut first) = work(
                format!("https://localhost:{}/first", address.port()),
                b"one",
                HttpVersion::Http2,
                2,
            );
            let (_second_cancel, mut second) = work(
                format!("https://localhost:{}/second", address.port()),
                b"two",
                HttpVersion::Http2,
                3,
            );
            first.tls.ca = Some(CA.to_vec());
            second.tls.ca = Some(CA.to_vec());
            let first_uri = first.request.url.parse().expect("the first URL is valid");
            let second_uri = second.request.url.parse().expect("the second URL is valid");
            let (first_result, second_result) = tokio::join!(
                execute_hyper(
                    &first,
                    &first_uri,
                    HttpVersion::Http2,
                    1024,
                    16 * 1024,
                    &pool,
                ),
                execute_hyper(
                    &second,
                    &second_uri,
                    HttpVersion::Http2,
                    1024,
                    16 * 1024,
                    &pool,
                ),
            );
            let CallResult::Response { body: first, .. } =
                first_result.expect("the first stream completes")
            else {
                panic!("the first stream must carry a response");
            };
            let CallResult::Response { body: second, .. } =
                second_result.expect("the second stream completes")
            else {
                panic!("the second stream must carry a response");
            };
            assert_eq!(first, b"first");
            assert_eq!(second, b"second");
            assert_eq!(
                accepted.load(Ordering::SeqCst),
                1,
                "one H2 connection multiplexes"
            );
            server.abort();
        });
    }

    #[cfg(feature = "hosted-http12")]
    #[test]
    #[allow(clippy::expect_used)]
    fn ordered_versions_fall_back_before_request_bytes_only() {
        const CA: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../spikes/h3-quinn-http3/certs/ca.crt"
        ));
        const CERTIFICATE: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../spikes/h3-quinn-http3/certs/server.crt"
        ));
        const PRIVATE_KEY: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../spikes/h3-quinn-http3/certs/server.key"
        ));

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("the test runtime builds");
        runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("the fallback listener binds");
            let address = listener.local_addr().expect("the listener has an address");
            let identity = crate::connectors::network::tls::TlsIdentity {
                certificate_chain: CERTIFICATE,
                private_key: PRIVATE_KEY,
            };
            let mut config = crate::connectors::network::tls::rustls_server(identity)
                .expect("the fixture identity is valid");
            config.alpn_protocols = vec![b"http/1.1".to_vec()];
            let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config));
            let server = tokio::spawn(async move {
                let mut successful = None;
                for _ in 0..2 {
                    let (stream, _) = listener.accept().await.expect("one client connects");
                    if let Ok(stream) = acceptor.accept(stream).await {
                        successful = Some(stream);
                        break;
                    }
                }
                let stream = successful.expect("the HTTP/1.1 fallback negotiates TLS");
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(
                        hyper_util::rt::TokioIo::new(stream),
                        hyper::service::service_fn(|_| async {
                            Ok::<_, std::convert::Infallible>(http::Response::new(Full::new(
                                Bytes::from_static(b"fallback"),
                            )))
                        }),
                    )
                    .await
                    .expect("the fallback response completes");
            });
            let (_cancel, mut work) = work(
                format!("https://localhost:{}/fallback", address.port()),
                b"request",
                HttpVersion::Http2,
                5,
            );
            work.binding.versions = vec![HttpVersion::Http2, HttpVersion::Http1];
            work.tls.ca = Some(CA.to_vec());
            let result = execute_call(
                &work,
                64 * 1024,
                EngineLimits::new(8, 2, 8, 64 * 1024).expect("the test limits are valid"),
                &H2Pool::default(),
            )
            .await;
            assert!(matches!(
                result,
                CallResult::Response {
                    version: HttpVersion::Http1,
                    body,
                    ..
                } if body == b"fallback"
            ));
            server.await.expect("the fallback server joins");
        });
    }

    #[cfg(feature = "hosted-http12")]
    #[test]
    #[allow(clippy::expect_used)]
    fn a_visible_request_failure_never_falls_back() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("the test runtime builds");
        runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("the refusal listener binds");
            let address = listener.local_addr().expect("the listener has an address");
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.expect("one client connects");
                let mut bytes = [0u8; 4096];
                let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut bytes)
                    .await
                    .expect("the server observes request bytes");
                drop(stream);
                tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
                    .await
                    .is_ok()
            });
            let (_cancel, mut work) = work(
                format!("http://{address}/visible"),
                b"request",
                HttpVersion::Http1,
                6,
            );
            work.binding.versions = vec![HttpVersion::Http1, HttpVersion::Http2];
            let result = execute_call(
                &work,
                64 * 1024,
                EngineLimits::new(8, 2, 8, 64 * 1024).expect("the test limits are valid"),
                &H2Pool::default(),
            )
            .await;
            assert!(matches!(
                result,
                CallResult::Exit(HttpClientExit::ProtocolFailed)
            ));
            assert!(
                !server.await.expect("the refusal server joins"),
                "no second protocol may start after request bytes became visible",
            );
        });
    }

    #[cfg(feature = "hosted-http3")]
    #[test]
    #[allow(clippy::expect_used)]
    fn http3_uses_verified_tls_and_preserves_the_response_stream() {
        const CA: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../spikes/h3-quinn-http3/certs/ca.crt"
        ));
        const CERTIFICATE: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../spikes/h3-quinn-http3/certs/server.crt"
        ));
        const PRIVATE_KEY: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../spikes/h3-quinn-http3/certs/server.key"
        ));

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("the test runtime builds");
        runtime.block_on(async {
            // Match the client's localhost resolution without dropping DNS-name
            // certificate verification or assuming IPv4 is preferred by the host.
            let listen = tokio::net::lookup_host(("localhost", 0))
                .await
                .expect("localhost resolves for the loopback fixture")
                .next()
                .expect("localhost has a loopback address");
            assert!(listen.ip().is_loopback());
            let socket = tokio::net::UdpSocket::bind(listen)
                .await
                .expect("the HTTP/3 server socket binds");
            let address = socket.local_addr().expect("the server has an address");
            let server = tokio::spawn(async move {
                let identity = crate::connectors::network::tls::TlsIdentity {
                    certificate_chain: CERTIFICATE,
                    private_key: PRIVATE_KEY,
                };
                let limits = EngineLimits::new(8, 2, 8, 64 * 1024)
                    .expect("the HTTP/3 test bounds are valid");
                let mut config = crate::connectors::network::tls::quiche_server(identity, limits)
                    .expect("the fixture identity configures quiche");
                let mut packet = vec![0u8; 65_535];
                let (length, peer) = socket
                    .recv_from(&mut packet)
                    .await
                    .expect("the server receives the Initial packet");
                packet.truncate(length);
                let header = quiche::Header::from_slice(&mut packet, quiche::MAX_CONN_ID_LEN)
                    .expect("the Initial packet has a QUIC header");
                assert_eq!(header.ty, quiche::Type::Initial);
                let local = socket.local_addr().expect("the server keeps its address");
                let server_id_bytes = [0x53; quiche::MAX_CONN_ID_LEN];
                let server_id = quiche::ConnectionId::from_ref(&server_id_bytes);
                let mut connection =
                    quiche::accept(&server_id, Some(&header.dcid), local, peer, &mut config)
                        .expect("the server accepts the QUIC connection");
                connection
                    .recv(
                        &mut packet,
                        quiche::RecvInfo {
                            from: peer,
                            to: local,
                        },
                    )
                    .expect("the server consumes the Initial packet");
                let h3_config = quiche::h3::Config::new().expect("the HTTP/3 server config builds");
                let mut h3 = None;
                let mut response_sent = false;
                for _ in 0..256 {
                    if connection.is_established() && h3.is_none() {
                        h3 = Some(
                            quiche::h3::Connection::with_transport(&mut connection, &h3_config)
                                .expect("the server opens HTTP/3"),
                        );
                    }
                    if let Some(h3) = h3.as_mut() {
                        loop {
                            match h3.poll(&mut connection) {
                                Ok((stream, quiche::h3::Event::Headers { list, .. })) => {
                                    assert_eq!(
                                        list.iter()
                                            .find(|header| header.name() == b":path")
                                            .map(quiche::h3::NameValue::value),
                                        Some(b"/h3".as_slice()),
                                    );
                                    let response = [
                                        quiche::h3::Header::new(b":status", b"207"),
                                        quiche::h3::Header::new(
                                            b"content-type",
                                            b"application/octet-stream",
                                        ),
                                        quiche::h3::Header::new(b"x-repeat", b"a"),
                                        quiche::h3::Header::new(b"x-repeat", b"b"),
                                    ];
                                    h3.send_response(&mut connection, stream, &response, false)
                                        .expect("the server sends response headers");
                                    let body = b"h3-response";
                                    assert_eq!(
                                        h3.send_body(&mut connection, stream, body, true,)
                                            .expect("the server sends the response body"),
                                        body.len(),
                                    );
                                    response_sent = true;
                                }
                                Ok((stream, quiche::h3::Event::Data)) => {
                                    let mut body = [0u8; 128];
                                    while h3.recv_body(&mut connection, stream, &mut body).is_ok() {
                                    }
                                }
                                Ok((_, quiche::h3::Event::Finished))
                                | Ok((_, quiche::h3::Event::PriorityUpdate)) => {}
                                Ok((_, quiche::h3::Event::Reset(_)))
                                | Ok((_, quiche::h3::Event::GoAway)) => {
                                    panic!("the HTTP/3 request stream closed unexpectedly");
                                }
                                Err(quiche::h3::Error::Done) => break,
                                Err(error) => panic!("the HTTP/3 server failed: {error}"),
                            }
                        }
                    }
                    let mut wrote = false;
                    loop {
                        match connection.send(&mut packet) {
                            Ok((written, info)) => {
                                wrote = true;
                                assert_eq!(
                                    socket
                                        .send_to(&packet[..written], info.to)
                                        .await
                                        .expect("the server sends one QUIC datagram"),
                                    written,
                                );
                            }
                            Err(quiche::Error::Done) => break,
                            Err(error) => panic!("the server QUIC send failed: {error}"),
                        }
                    }
                    if response_sent && !wrote {
                        return;
                    }
                    let timeout = connection
                        .timeout()
                        .unwrap_or(std::time::Duration::from_millis(100));
                    match tokio::time::timeout(timeout, socket.recv_from(&mut packet)).await {
                        Ok(Ok((length, from))) => {
                            packet.truncate(length);
                            connection
                                .recv(&mut packet, quiche::RecvInfo { from, to: local })
                                .expect("the server consumes a QUIC datagram");
                            packet.resize(65_535, 0);
                        }
                        Ok(Err(error)) => panic!("the HTTP/3 server receive failed: {error}"),
                        Err(_) => connection.on_timeout(),
                    }
                }
                panic!("the bounded HTTP/3 server loop did not finish");
            });

            let (_cancel, mut work) = work(
                format!("https://localhost:{}/h3", address.port()),
                b"request",
                HttpVersion::Http3,
                4,
            );
            work.tls.ca = Some(CA.to_vec());
            let limits =
                EngineLimits::new(8, 2, 8, 64 * 1024).expect("the HTTP/3 client bounds are valid");
            let result = execute_h3(
                &work,
                &work.request.url.parse().expect("the URL is valid"),
                1024,
                limits,
            )
            .await
            .expect("the HTTP/3 call succeeds");
            let CallResult::Response {
                status,
                version,
                headers,
                body,
            } = result
            else {
                panic!("the call must return a response");
            };
            assert_eq!(status, 207);
            assert_eq!(version, HttpVersion::Http3);
            assert_eq!(body, b"h3-response");
            assert_eq!(
                headers.get("x-repeat"),
                Some(&vec![b"a".to_vec(), b"b".to_vec()]),
            );
            server.await.expect("the HTTP/3 server joins");
        });
    }
}
