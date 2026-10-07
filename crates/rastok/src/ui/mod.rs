//! Hosted, capability-protected execution UI.
//!
//! The server is a physical debugger boundary. It receives disposable JSON
//! snapshots and sends serialized control requests to the execution owner;
//! neither direction can mutate the journal directly.

use bytes::Bytes;
use http_body_util::{BodyExt as _, Full};
use hyper::body::{Frame, Incoming};
use hyper::header::{
    AUTHORIZATION, CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, COOKIE, ORIGIN,
    REFERRER_POLICY, SET_COOKIE, X_CONTENT_TYPE_OPTIONS,
};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{Value as JsonValue, json};
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::pin::Pin;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::sync::{RwLock, mpsc, watch};

mod view;

pub use view::{
    ActivityView, ExecutionState, FaultView, SnapshotInput, expectation_json,
    scenario_definition_json, scenario_injection_json, snapshot,
};

const INDEX_HTML: &str = include_str!("index.html");
const APP_CSS: &str = include_str!("app.css");
const GRAPH_LAYOUT_JS: &str = include_str!("graph_layout.js");
const APP_JS: &str = include_str!("app.js");
const COMMAND_CAPACITY: usize = 32;
const SNAPSHOT_CAPACITY: usize = 1;
const SSE_CAPACITY: usize = 8;

type HttpBody = http_body_util::combinators::UnsyncBoxBody<Bytes, Infallible>;

/// One command accepted from the current browser controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiCommand {
    /// Continue normal scheduling.
    Resume,
    /// Request the next safe pause.
    Pause,
    /// Execute one atomic VM transaction.
    Step,
    /// Retry the still-reserved faulting transaction.
    RetryStep,
    /// Continue until the current run or scenario completes.
    RunToCompletion,
    /// Apply one authored Lab action.
    NextScenarioAction,
    /// Continue until immediately before the next Lab assertion.
    RunUntilAssertion,
    /// Recreate the selected Lab scenario from a clean world.
    ResetScenario,
    /// Select one checked simulation/scenario pair.
    SelectScenario {
        /// Checked simulation name.
        simulation: String,
        /// Checked scenario name.
        scenario: String,
    },
    /// End the hosted UI session.
    Close,
}

/// One complete disposable read-model revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiSnapshot {
    revision: u64,
    json: Bytes,
    session: u64,
    boundary: u64,
    kind: String,
    summary: String,
    record_through: Option<u64>,
    action: Option<u64>,
    recording: bool,
}

impl UiSnapshot {
    /// Serializes one complete JSON value for publication.
    ///
    /// # Errors
    ///
    /// JSON serialization failure is returned before the current projection
    /// is changed.
    pub fn new(revision: u64, value: &JsonValue) -> Result<Self, serde_json::Error> {
        let session = value
            .pointer("/timeline/session")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        let boundary = value
            .pointer("/timeline/boundary")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        let kind = value
            .pointer("/topology/activity/kind")
            .and_then(JsonValue::as_str)
            .unwrap_or("initial")
            .to_owned();
        let summary = value
            .pointer("/topology/activity/detail")
            .and_then(JsonValue::as_str)
            .unwrap_or("Before admission")
            .to_owned();
        let record_through = value
            .pointer("/timeline/record_through")
            .and_then(JsonValue::as_u64);
        let action = value
            .pointer("/scenario/current/next_action_index")
            .and_then(JsonValue::as_u64);
        serde_json::to_vec(value).map(|json| Self {
            revision,
            json: Bytes::from(json),
            session,
            boundary,
            kind,
            summary,
            record_through,
            action,
            recording: value.get("mode").and_then(JsonValue::as_str) == Some("replay"),
        })
    }

    /// Revision monotonically assigned by the execution owner.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Physical history-retention cost, not a semantic size or quota.
    pub(crate) fn encoded_len(&self) -> usize {
        self.json.len()
    }
}

/// Complete in-memory debugger history. These are exact disposable
/// projections captured at committed boundaries, never causal authority.
#[derive(Debug)]
struct TimelineStore {
    live: UiSnapshot,
    sessions: BTreeMap<u64, BTreeMap<u64, UiSnapshot>>,
}

impl TimelineStore {
    fn new(initial: UiSnapshot) -> Self {
        let mut store = Self {
            live: initial.clone(),
            sessions: BTreeMap::new(),
        };
        store.retain(initial);
        store
    }

    fn retain(&mut self, snapshot: UiSnapshot) {
        self.sessions
            .entry(snapshot.session)
            .or_default()
            .insert(snapshot.boundary, snapshot.clone());
        if snapshot.revision >= self.live.revision {
            self.live = snapshot;
        }
    }

    fn frame(&self, session: u64, boundary: u64) -> Option<&UiSnapshot> {
        self.sessions.get(&session)?.get(&boundary)
    }

    fn index_json(&self) -> JsonValue {
        json!({
            "active_session": self.live.session,
            "live_boundary": self.live.boundary,
            "sessions": self.sessions.iter().map(|(session, frames)| json!({
                "session": session,
                "frames": frames.values().map(|snapshot| json!({
                    "boundary": snapshot.boundary,
                    "kind": snapshot.kind,
                    "summary": snapshot.summary,
                    "record_through": snapshot.record_through,
                    "action": snapshot.action,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })
    }
}

/// Failure before the UI has accepted any graph execution.
#[derive(Debug)]
pub enum UiStartError {
    /// The requested loopback port could not be bound.
    Bind(std::io::Error),
    /// The host OS could not provide an authorization capability.
    Entropy(getrandom::Error),
    /// The private Tokio runtime could not be built.
    Runtime(std::io::Error),
    /// The server thread could not be created.
    Thread(std::io::Error),
}

impl fmt::Display for UiStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bind(error) => write!(formatter, "the UI loopback port is unavailable: {error}"),
            Self::Entropy(error) => {
                write!(
                    formatter,
                    "the UI authorization token could not be minted: {error}"
                )
            }
            Self::Runtime(error) => write!(formatter, "the UI runtime could not start: {error}"),
            Self::Thread(error) => write!(formatter, "the UI thread could not start: {error}"),
        }
    }
}

impl std::error::Error for UiStartError {}

/// Running local server plus its non-blocking execution-side seams.
#[derive(Debug)]
pub struct UiServer {
    address: SocketAddr,
    capability: String,
    commands: Receiver<UiCommand>,
    snapshots: SyncSender<UiSnapshot>,
    pending_snapshot: Option<UiSnapshot>,
    timeline: Arc<Mutex<TimelineStore>>,
    shutdown: watch::Sender<bool>,
    thread: Option<JoinHandle<()>>,
}

impl UiServer {
    /// Binds loopback, mints a capability and starts the embedded server.
    ///
    /// `port = None` asks the OS for a free port. The initial snapshot exists
    /// before the first browser can connect.
    pub fn start(port: Option<u16>, initial: UiSnapshot) -> Result<Self, UiStartError> {
        let listener = TcpListener::bind(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            port.unwrap_or(0),
        ))
        .map_err(UiStartError::Bind)?;
        listener.set_nonblocking(true).map_err(UiStartError::Bind)?;
        let address = listener.local_addr().map_err(UiStartError::Bind)?;
        let capability = mint_capability().map_err(UiStartError::Entropy)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(UiStartError::Runtime)?;
        let (command_tx, commands) = std::sync::mpsc::sync_channel(COMMAND_CAPACITY);
        let (snapshot_tx, snapshot_rx) = std::sync::mpsc::sync_channel(SNAPSHOT_CAPACITY);
        let (shutdown, shutdown_rx) = watch::channel(false);
        let initial_revision = initial.revision();
        let timeline = Arc::new(Mutex::new(TimelineStore::new(initial.clone())));
        let context = Arc::new(ServerContext {
            capability: capability.clone(),
            origin: format!("http://{address}"),
            command_tx,
            controller: Mutex::new(None),
            snapshot: RwLock::new(initial),
            revision: watch::channel(initial_revision).0,
            timeline: Arc::clone(&timeline),
        });
        let thread_context = Arc::clone(&context);
        let thread = std::thread::Builder::new()
            .name(String::from("rastok-ui"))
            .spawn(move || {
                runtime.block_on(serve(listener, thread_context, snapshot_rx, shutdown_rx));
            })
            .map_err(UiStartError::Thread)?;
        Ok(Self {
            address,
            capability,
            commands,
            snapshots: snapshot_tx,
            pending_snapshot: None,
            timeline,
            shutdown,
            thread: Some(thread),
        })
    }

    /// Capability-bearing URL printed for the user.
    #[must_use]
    pub fn url(&self) -> String {
        format!("http://{}/?token={}", self.address, self.capability)
    }

    /// Receives one already-authorized command without waiting.
    #[must_use]
    pub fn try_command(&self) -> Option<UiCommand> {
        self.commands.try_recv().ok()
    }

    /// Publishes the newest complete view without waiting for the UI thread.
    ///
    /// A saturated one-slot lane keeps only the newest pending revision. The
    /// execution owner should call [`Self::flush`] in later loop turns.
    pub fn publish(&mut self, snapshot: UiSnapshot) {
        if let Ok(mut timeline) = self.timeline.lock() {
            timeline.retain(snapshot.clone());
        }
        self.flush();
        match self.snapshots.try_send(snapshot) {
            Ok(()) => {}
            Err(TrySendError::Full(snapshot)) => self.pending_snapshot = Some(snapshot),
            Err(TrySendError::Disconnected(_)) => self.pending_snapshot = None,
        }
    }

    /// Retries the newest pending view without blocking execution.
    pub fn flush(&mut self) {
        let Some(snapshot) = self.pending_snapshot.take() else {
            return;
        };
        match self.snapshots.try_send(snapshot) {
            Ok(()) => {}
            Err(TrySendError::Full(snapshot)) => self.pending_snapshot = Some(snapshot),
            Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

impl Drop for UiServer {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Debug)]
struct ServerContext {
    capability: String,
    origin: String,
    command_tx: SyncSender<UiCommand>,
    controller: Mutex<Option<String>>,
    snapshot: RwLock<UiSnapshot>,
    revision: watch::Sender<u64>,
    timeline: Arc<Mutex<TimelineStore>>,
}

async fn serve(
    listener: TcpListener,
    context: Arc<ServerContext>,
    snapshots: Receiver<UiSnapshot>,
    mut shutdown: watch::Receiver<bool>,
) {
    let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
        return;
    };
    let snapshot_context = Arc::clone(&context);
    tokio::spawn(async move {
        loop {
            match snapshots.try_recv() {
                Ok(snapshot) => {
                    let revision = snapshot.revision();
                    *snapshot_context.snapshot.write().await = snapshot;
                    let _ = snapshot_context.revision.send(revision);
                }
                Err(TryRecvError::Empty) => tokio::time::sleep(Duration::from_millis(10)).await,
                Err(TryRecvError::Disconnected) => return,
            }
        }
    });
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
            accepted = listener.accept() => {
                let Ok((stream, peer)) = accepted else {
                    continue;
                };
                if !peer.ip().is_loopback() {
                    continue;
                }
                let context = Arc::clone(&context);
                tokio::spawn(async move {
                    let service = service_fn(move |request| {
                        handle(request, Arc::clone(&context))
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        }
    }
}

async fn handle(
    request: Request<Incoming>,
    context: Arc<ServerContext>,
) -> Result<Response<HttpBody>, Infallible> {
    let response = if !authorized(&request, &context.capability) {
        response(
            StatusCode::UNAUTHORIZED,
            "text/plain; charset=utf-8",
            "unauthorized",
        )
    } else {
        match (request.method(), request.uri().path()) {
            (&Method::GET, "/") => html_response(&context.capability),
            (&Method::GET, "/favicon.ico") => response(StatusCode::NO_CONTENT, "image/x-icon", ""),
            (&Method::GET, "/app.css") => {
                response(StatusCode::OK, "text/css; charset=utf-8", APP_CSS)
            }
            (&Method::GET, "/graph-layout.js") => response(
                StatusCode::OK,
                "text/javascript; charset=utf-8",
                GRAPH_LAYOUT_JS,
            ),
            (&Method::GET, "/app.js") => {
                response(StatusCode::OK, "text/javascript; charset=utf-8", APP_JS)
            }
            (&Method::GET, "/api/snapshot") => historical_snapshot(&request, &context),
            (&Method::GET, "/api/timeline") => timeline_response(&context),
            (&Method::GET, "/api/events") => sse_response(context.revision.subscribe()),
            (&Method::POST, "/api/control/claim") => control_claim(&request, &context, false),
            (&Method::POST, "/api/control/take") => control_claim(&request, &context, true),
            (&Method::POST, "/api/control/release") => control_release(&request, &context),
            (&Method::POST, path) if path.starts_with("/api/command/") => {
                let spelling = String::from(&path[13..]);
                command(request, &context, &spelling).await
            }
            _ => response(
                StatusCode::NOT_FOUND,
                "text/plain; charset=utf-8",
                "not found",
            ),
        }
    };
    Ok(response)
}

fn historical_snapshot(request: &Request<Incoming>, context: &ServerContext) -> Response<HttpBody> {
    let requested = request.uri().query().and_then(|query| {
        let session = query_value(query, "session")?.parse::<u64>().ok()?;
        let boundary = query_value(query, "boundary")?.parse::<u64>().ok()?;
        Some((session, boundary))
    });
    if let Some((session, boundary)) = requested {
        let Ok(timeline) = context.timeline.lock() else {
            return response(
                StatusCode::SERVICE_UNAVAILABLE,
                "text/plain; charset=utf-8",
                "timeline unavailable",
            );
        };
        return timeline.frame(session, boundary).map_or_else(
            || {
                response(
                    StatusCode::NOT_FOUND,
                    "text/plain; charset=utf-8",
                    "unknown recorded transition",
                )
            },
            |snapshot| response(StatusCode::OK, "application/json", snapshot.json.clone()),
        );
    }
    let Ok(timeline) = context.timeline.lock() else {
        return response(
            StatusCode::SERVICE_UNAVAILABLE,
            "text/plain; charset=utf-8",
            "timeline unavailable",
        );
    };
    response(
        StatusCode::OK,
        "application/json",
        timeline.live.json.clone(),
    )
}

fn timeline_response(context: &ServerContext) -> Response<HttpBody> {
    let Ok(timeline) = context.timeline.lock() else {
        return response(
            StatusCode::SERVICE_UNAVAILABLE,
            "text/plain; charset=utf-8",
            "timeline unavailable",
        );
    };
    match serde_json::to_string(&timeline.index_json()) {
        Ok(body) => response(StatusCode::OK, "application/json", body),
        Err(_) => response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "text/plain; charset=utf-8",
            "timeline serialization failed",
        ),
    }
}

fn authorized(request: &Request<Incoming>, capability: &str) -> bool {
    let query_authorized = request.uri().query().is_some_and(|query| {
        query
            .split('&')
            .any(|part| part.strip_prefix("token=") == Some(capability))
    });
    let bearer_authorized = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.strip_prefix("Bearer ") == Some(capability));
    let cookie_authorized = request
        .headers()
        .get(COOKIE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|cookies| {
            cookies
                .split(';')
                .map(str::trim)
                .any(|cookie| cookie.strip_prefix("rastok_ui=") == Some(capability))
        });
    query_authorized || bearer_authorized || cookie_authorized
}

fn same_origin(request: &Request<Incoming>, context: &ServerContext) -> bool {
    request
        .headers()
        .get(ORIGIN)
        .and_then(|value| value.to_str().ok())
        == Some(context.origin.as_str())
}

fn controller_id(request: &Request<Incoming>) -> Option<&str> {
    request
        .headers()
        .get("x-rastok-controller")
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
}

fn control_claim(
    request: &Request<Incoming>,
    context: &ServerContext,
    takeover: bool,
) -> Response<HttpBody> {
    if !same_origin(request, context) {
        return response(
            StatusCode::FORBIDDEN,
            "text/plain; charset=utf-8",
            "bad origin",
        );
    }
    let Some(candidate) = controller_id(request) else {
        return response(
            StatusCode::BAD_REQUEST,
            "text/plain; charset=utf-8",
            "missing controller",
        );
    };
    let Ok(mut controller) = context.controller.lock() else {
        return response(
            StatusCode::SERVICE_UNAVAILABLE,
            "text/plain; charset=utf-8",
            "controller unavailable",
        );
    };
    if takeover
        || controller
            .as_deref()
            .is_none_or(|current| current == candidate)
    {
        *controller = Some(String::from(candidate));
        response(
            StatusCode::OK,
            "application/json",
            r#"{"role":"controller"}"#,
        )
    } else {
        response(
            StatusCode::CONFLICT,
            "application/json",
            r#"{"role":"observer"}"#,
        )
    }
}

fn control_release(request: &Request<Incoming>, context: &ServerContext) -> Response<HttpBody> {
    if !same_origin(request, context) {
        return response(
            StatusCode::FORBIDDEN,
            "text/plain; charset=utf-8",
            "bad origin",
        );
    }
    let Some(candidate) = controller_id(request) else {
        return response(
            StatusCode::BAD_REQUEST,
            "text/plain; charset=utf-8",
            "missing controller",
        );
    };
    let Ok(mut controller) = context.controller.lock() else {
        return response(
            StatusCode::SERVICE_UNAVAILABLE,
            "text/plain; charset=utf-8",
            "controller unavailable",
        );
    };
    if controller.as_deref() == Some(candidate) {
        *controller = None;
        response(StatusCode::OK, "application/json", r#"{"role":"observer"}"#)
    } else {
        response(
            StatusCode::CONFLICT,
            "text/plain; charset=utf-8",
            "stale controller",
        )
    }
}

async fn command(
    request: Request<Incoming>,
    context: &ServerContext,
    spelling: &str,
) -> Response<HttpBody> {
    if !same_origin(&request, context) {
        return response(
            StatusCode::FORBIDDEN,
            "text/plain; charset=utf-8",
            "bad origin",
        );
    }
    let Some(candidate) = controller_id(&request) else {
        return response(
            StatusCode::BAD_REQUEST,
            "text/plain; charset=utf-8",
            "missing controller",
        );
    };
    let is_controller = match context.controller.lock() {
        Ok(controller) => controller.as_deref() == Some(candidate),
        Err(_) => {
            return response(
                StatusCode::SERVICE_UNAVAILABLE,
                "text/plain; charset=utf-8",
                "controller unavailable",
            );
        }
    };
    if !is_controller {
        return response(
            StatusCode::CONFLICT,
            "text/plain; charset=utf-8",
            "stale controller",
        );
    }
    let Some(command) = command_from_request(spelling, &request) else {
        return response(
            StatusCode::BAD_REQUEST,
            "text/plain; charset=utf-8",
            "unknown command",
        );
    };
    if command != UiCommand::Close && context.snapshot.read().await.recording {
        return response(
            StatusCode::CONFLICT,
            "text/plain; charset=utf-8",
            "a recording is read-only; navigate its recorded timeline",
        );
    }
    match context.command_tx.try_send(command) {
        Ok(()) => response(
            StatusCode::ACCEPTED,
            "application/json",
            r#"{"accepted":true}"#,
        ),
        Err(TrySendError::Full(_)) => response(
            StatusCode::TOO_MANY_REQUESTS,
            "text/plain; charset=utf-8",
            "command queue full",
        ),
        Err(TrySendError::Disconnected(_)) => response(
            StatusCode::GONE,
            "text/plain; charset=utf-8",
            "execution ended",
        ),
    }
}

fn command_from_request(spelling: &str, request: &Request<Incoming>) -> Option<UiCommand> {
    match spelling {
        "resume" => Some(UiCommand::Resume),
        "pause" => Some(UiCommand::Pause),
        "step" => Some(UiCommand::Step),
        "retry-step" => Some(UiCommand::RetryStep),
        "run-to-completion" => Some(UiCommand::RunToCompletion),
        "next-scenario-action" => Some(UiCommand::NextScenarioAction),
        "run-until-assertion" => Some(UiCommand::RunUntilAssertion),
        "reset-scenario" => Some(UiCommand::ResetScenario),
        "close" => Some(UiCommand::Close),
        "select-scenario" => {
            let query = request.uri().query()?;
            let simulation = query_value(query, "simulation")?;
            let scenario = query_value(query, "scenario")?;
            Some(UiCommand::SelectScenario {
                simulation: String::from(simulation),
                scenario: String::from(scenario),
            })
        }
        _ => None,
    }
}

fn query_value<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query
        .split('&')
        .find_map(|part| part.strip_prefix(name)?.strip_prefix('='))
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 256
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
}

fn html_response(capability: &str) -> Response<HttpBody> {
    let mut response = response(StatusCode::OK, "text/html; charset=utf-8", INDEX_HTML);
    if let Ok(cookie) = format!("rastok_ui={capability}; HttpOnly; SameSite=Strict; Path=/").parse()
    {
        response.headers_mut().insert(SET_COOKIE, cookie);
    }
    response
}

fn response(
    status: StatusCode,
    content_type: &'static str,
    body: impl Into<Bytes>,
) -> Response<HttpBody> {
    let mut response = Response::new(Full::new(body.into()).boxed_unsync());
    *response.status_mut() = status;
    response.headers_mut().insert(
        CONTENT_TYPE,
        hyper::header::HeaderValue::from_static(content_type),
    );
    security_headers(&mut response);
    response
}

fn sse_response(revisions: watch::Receiver<u64>) -> Response<HttpBody> {
    let (sender, receiver) = mpsc::channel(SSE_CAPACITY);
    tokio::spawn(publish_revisions(sender, revisions));
    let mut response = Response::new(SseBody { receiver }.boxed_unsync());
    response.headers_mut().insert(
        CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("text/event-stream"),
    );
    response.headers_mut().insert(
        CACHE_CONTROL,
        hyper::header::HeaderValue::from_static("no-store"),
    );
    security_headers(&mut response);
    response
}

async fn publish_revisions(sender: mpsc::Sender<Bytes>, mut revisions: watch::Receiver<u64>) {
    loop {
        let revision = *revisions.borrow_and_update();
        let event = Bytes::from(format!("event: revision\ndata: {revision}\n\n"));
        if sender.send(event).await.is_err() || revisions.changed().await.is_err() {
            return;
        }
    }
}

fn security_headers(response: &mut Response<HttpBody>) {
    response.headers_mut().insert(
        CACHE_CONTROL,
        hyper::header::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        CONTENT_SECURITY_POLICY,
        hyper::header::HeaderValue::from_static(
            "default-src 'self'; connect-src 'self'; img-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
        ),
    );
    response.headers_mut().insert(
        REFERRER_POLICY,
        hyper::header::HeaderValue::from_static("no-referrer"),
    );
    response.headers_mut().insert(
        X_CONTENT_TYPE_OPTIONS,
        hyper::header::HeaderValue::from_static("nosniff"),
    );
}

#[derive(Debug)]
struct SseBody {
    receiver: mpsc::Receiver<Bytes>,
}

impl hyper::body::Body for SseBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        self.receiver
            .poll_recv(context)
            .map(|item| item.map(|bytes| Ok(Frame::data(bytes))))
    }
}

fn mint_capability() -> Result<String, getrandom::Error> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)?;
    let mut token = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use fmt::Write as _;
        let _ = write!(token, "{byte:02x}");
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};

    fn initial() -> UiSnapshot {
        UiSnapshot::new(0, &serde_json::json!({"revision": 0, "state": "paused"}))
            .expect("static JSON")
    }

    fn timeline_frame(revision: u64, session: u64, boundary: u64) -> UiSnapshot {
        UiSnapshot::new(
            revision,
            &serde_json::json!({
                "revision": revision,
                "timeline": {
                    "session": session,
                    "boundary": boundary,
                    "record_through": boundary.checked_sub(1),
                },
                "topology": {
                    "activity": (boundary > 0).then(|| serde_json::json!({
                        "kind": "transaction",
                        "detail": format!("boundary {boundary}"),
                    })),
                },
            }),
        )
        .expect("static timeline JSON")
    }

    fn request(address: SocketAddr, request: &str) -> String {
        let mut stream = std::net::TcpStream::connect(address).expect("loopback server accepts");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("bounded response");
        stream
            .write_all(request.as_bytes())
            .expect("request writes");
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .expect("response reads");
        response
    }

    #[test]
    fn capability_is_required_and_assets_are_embedded() {
        let server = UiServer::start(None, initial()).expect("server starts");
        let unauthorized = request(
            server.address,
            &format!(
                "GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                server.address
            ),
        );
        assert!(unauthorized.starts_with("HTTP/1.1 401"), "{unauthorized}");
        let authorized = request(
            server.address,
            &format!(
                "GET /?token={} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                server.capability, server.address
            ),
        );
        assert!(authorized.starts_with("HTTP/1.1 200"), "{authorized}");
        assert!(authorized.contains("Rastok UI"));
        assert!(authorized.contains("SameSite=Strict"));
        let body = authorized
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .unwrap_or_default();
        assert!(!body.contains(&server.capability));
        let icon = request(
            server.address,
            &format!(
                "GET /favicon.ico?token={} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                server.capability, server.address
            ),
        );
        assert!(icon.starts_with("HTTP/1.1 204"), "{icon}");
        assert!(icon.contains("default-src 'self'"));
        assert!(!icon.contains("unsafe-inline"));
        let unauthorized_icon = request(
            server.address,
            &format!(
                "GET /favicon.ico HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                server.address
            ),
        );
        assert!(unauthorized_icon.starts_with("HTTP/1.1 401"));
    }

    #[test]
    fn an_explicit_occupied_port_refuses_before_a_server_starts() {
        let occupied = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .expect("one loopback port is available");
        let port = occupied
            .local_addr()
            .expect("the port has an address")
            .port();
        assert!(matches!(
            UiServer::start(Some(port), initial()),
            Err(UiStartError::Bind(_))
        ));
    }

    #[test]
    fn a_cross_origin_control_request_cannot_enter_the_command_lane() {
        let server = UiServer::start(None, initial()).expect("server starts");
        let response = request(
            server.address,
            &format!(
                "POST /api/control/claim?token={} HTTP/1.1\r\nHost: {}\r\nOrigin: https://attacker.example\r\nX-Rastok-Controller: attacker\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                server.capability, server.address
            ),
        );
        assert!(response.starts_with("HTTP/1.1 403"), "{response}");
        assert_eq!(server.try_command(), None);
    }

    #[test]
    fn one_controller_wins_until_an_explicit_takeover() {
        let server = UiServer::start(None, initial()).expect("server starts");
        let post = |path: &str, controller: &str| {
            request(
                server.address,
                &format!(
                    "POST {path}?token={} HTTP/1.1\r\nHost: {}\r\nOrigin: http://{}\r\nX-Rastok-Controller: {controller}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    server.capability, server.address, server.address
                ),
            )
        };
        assert!(post("/api/control/claim", "first").starts_with("HTTP/1.1 200"));
        assert!(post("/api/control/claim", "second").starts_with("HTTP/1.1 409"));
        assert!(post("/api/control/take", "second").starts_with("HTTP/1.1 200"));
        assert!(post("/api/command/step", "first").starts_with("HTTP/1.1 409"));
        assert!(post("/api/command/step", "second").starts_with("HTTP/1.1 202"));
        assert_eq!(server.try_command(), Some(UiCommand::Step));
        assert!(post("/api/command/next-scenario-action", "second").starts_with("HTTP/1.1 202"));
        assert_eq!(server.try_command(), Some(UiCommand::NextScenarioAction));
    }

    #[test]
    fn a_recording_refuses_execution_commands_even_from_its_controller() {
        let initial = UiSnapshot::new(0, &json!({"mode": "replay", "execution": {"state": "completed", "legal_commands": ["close"]}})).unwrap();
        let server = UiServer::start(None, initial).unwrap();
        let post = |path: &str| {
            request(
                server.address,
                &format!(
                    "POST {path}?token={} HTTP/1.1\r\nHost: {}\r\nOrigin: http://{}\r\nX-Rastok-Controller: reader\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    server.capability, server.address, server.address
                ),
            )
        };
        assert!(post("/api/control/claim").starts_with("HTTP/1.1 200"));
        for command in [
            "resume",
            "step",
            "retry-step",
            "run-to-completion",
            "reset-scenario",
            "next-scenario-action",
        ] {
            assert!(post(&format!("/api/command/{command}")).starts_with("HTTP/1.1 409"));
            assert_eq!(server.try_command(), None);
        }
        assert!(post("/api/command/close").starts_with("HTTP/1.1 202"));
        assert_eq!(server.try_command(), Some(UiCommand::Close));
    }

    #[test]
    fn embedded_ui_uses_system_theme_and_compact_transport_controls() {
        assert!(APP_CSS.contains("prefers-color-scheme: light"));
        assert!(APP_CSS.contains("--bg: #0f1412"));
        assert!(INDEX_HTML.contains("class=\"transport\""));
        assert!(INDEX_HTML.contains("#icon-step"));
        assert!(INDEX_HTML.contains("<span>Next transition</span>"));
        assert!(INDEX_HTML.contains("<span>Next action</span>"));
        assert!(INDEX_HTML.contains("<span>Run to expect</span>"));
        assert!(INDEX_HTML.contains("<span>Finish</span>"));
        assert!(INDEX_HTML.contains("<span>Replay</span>"));
        assert!(INDEX_HTML.contains("<span>Take control</span>"));
        assert!(INDEX_HTML.contains("src=\"/graph-layout.js\""));
        assert!(!INDEX_HTML.contains("id=\"guidance\""));
        assert!(APP_JS.contains("Before admission"));
        assert!(APP_JS.contains("[\"META\", meta], [\"DATA\", data]"));
        assert!(APP_JS.contains("appendDataTree(zone, value)"));
        assert!(APP_JS.contains(
            "[\"record_index\", \"record_type\", \"summary\", \"trigger\", \"origin_id\"]"
        ));
        assert!(APP_JS.contains("selectedTopologyContext(snapshot.topology)"));
        assert!(APP_JS.contains("ownerIds.has(item.owner)"));
        assert!(!APP_JS.contains("path.startsWith(`${selection.path}.`)"));
        assert!(
            APP_JS.contains(
                "graphLayout.curvedCorridorPath(route.presentationPoints || route.points)"
            )
        );
        assert!(APP_JS.contains("showBoundary(session, frames[index - 1].boundary, true)"));
        assert!(!INDEX_HTML.contains("Branch from here"));
        assert!(INDEX_HTML.contains("id=\"timeline-session\""));
        assert!(APP_JS.contains("Historical session unavailable"));
        assert!(GRAPH_LAYOUT_JS.contains("minimizeAdjacentCrossings("));
        assert!(GRAPH_LAYOUT_JS.contains("allocateSideItems("));
        assert!(GRAPH_LAYOUT_JS.contains("occupiedVertical"));
        assert!(GRAPH_LAYOUT_JS.contains("routingPortals("));
        assert!(GRAPH_LAYOUT_JS.contains("placeEdgeLabels(routes, graph, layout)"));
        assert!(GRAPH_LAYOUT_JS.contains("curveLabelCollisionScore("));
        assert!(GRAPH_LAYOUT_JS.contains("curvedCorridorSegments("));
        assert!(GRAPH_LAYOUT_JS.contains("candidate.portals.map"));
        assert!(APP_CSS.contains("min-width: var(--graph-readable-width)"));
        assert!(APP_JS.contains("layout.width * graphLayout.minimumReadableScale"));
        assert!(APP_CSS.contains("max-width: var(--graph-fit-width)"));
        assert!(APP_JS.contains("Math.ceil(layout.width * 1.5)"));
        assert!(APP_JS.contains("keepGraphTextAtInterfaceSize(svg, layout.width)"));
        assert!(APP_CSS.contains("--graph-title-font-size"));
        assert!(APP_JS.contains("history-watermark"));
        assert!(APP_JS.contains("animateMotion"));
        assert!(APP_JS.contains("activity?.routes"));
        assert!(APP_JS.contains("connector-boundary-packet"));
        assert!(APP_JS.contains("pendingCommandRevision"));
        assert!(GRAPH_LAYOUT_JS.contains("scopePortAnchors"));
        assert!(APP_JS.contains("edge.queued_count"));
        assert!(APP_JS.contains("inspectMailbox(edge.route)"));
        assert!(APP_JS.contains("renderMailbox(snapshot.topology)"));
        assert!(!APP_JS.contains("edge.traversal_count"));
        assert!(!APP_JS.contains("edge.envelope_count"));
        assert!(!APP_JS.contains("class: \"edge-port source-port\""));
        assert!(!APP_JS.contains('✉'));
        assert!(INDEX_HTML.contains("data-inspector-tab=\"envelope\""));
        assert!(INDEX_HTML.contains("data-inspector-tab=\"mailbox\""));
        assert!(INDEX_HTML.contains("id=\"workspace-splitter\""));
        assert!(INDEX_HTML.contains("1 event/s"));
        assert!(INDEX_HTML.contains("4 events/s"));
        assert!(APP_JS.contains("installGraphViewport"));
        assert!(APP_JS.contains("installWorkspaceSplitter"));
        assert!(!APP_JS.contains("edit-next-injection"));
        assert!(!INDEX_HTML.contains("Inject now"));
        assert!(!APP_JS.contains("typeBadge("));
        assert!(GRAPH_LAYOUT_JS.contains("edge.target?.kind !== \"discard\""));
        assert!(!GRAPH_LAYOUT_JS.contains("topology.handler_relations"));
        assert!(!APP_JS.contains("graph-port-terminal"));
        assert!(APP_JS.contains("drained to nothing; no downstream mailbox"));
        assert!(APP_JS.contains("Manager commands · not spatial edges"));
        assert!(!APP_JS.contains("inspectHandlerRelation(edge)"));
        assert!(APP_JS.contains("`≡ ${edge.queued_count || \"0\"}`"));
        assert!(APP_JS.contains("`⧉ ${instances.length}`"));
        assert!(APP_JS.contains("Unavailable: ${reason}"));
        assert!(APP_JS.contains("window.addEventListener(\"pagehide\", releaseControl)"));
        assert!(APP_JS.contains("events.addEventListener(\"open\""));
        assert!(APP_JS.contains("slice(0, 8)"));
        assert!(!APP_JS.contains("truncate(boundaryLabel"));
        assert!(!GRAPH_LAYOUT_JS.contains("terminal:${node.id}:${port.name}"));
    }

    #[test]
    fn every_boundary_remains_addressable_and_reset_preserves_the_previous_session() {
        let mut server = UiServer::start(None, timeline_frame(0, 0, 0)).expect("server starts");
        server.publish(timeline_frame(1, 0, 1));
        server.publish(timeline_frame(2, 0, 2));

        let index = request(
            server.address,
            &format!(
                "GET /api/timeline?token={} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                server.capability, server.address
            ),
        );
        assert!(index.contains("\"live_boundary\":2"), "{index}");
        assert!(index.contains("boundary 1"), "{index}");

        let historical = request(
            server.address,
            &format!(
                "GET /api/snapshot?session=0&boundary=1&token={} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                server.capability, server.address
            ),
        );
        assert!(historical.contains("\"boundary\":1"), "{historical}");
        assert!(!historical.contains("\"boundary\":2"), "{historical}");

        server.publish(timeline_frame(3, 1, 0));
        let reset = request(
            server.address,
            &format!(
                "GET /api/snapshot?session=1&boundary=0&token={} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                server.capability, server.address
            ),
        );
        assert!(reset.contains("\"session\":1"), "{reset}");
        assert!(reset.contains("\"boundary\":0"), "{reset}");
        let retained = request(
            server.address,
            &format!(
                "GET /api/snapshot?session=0&boundary=1&token={} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                server.capability, server.address
            ),
        );
        assert_eq!(
            historical.split_once("\r\n\r\n").map(|(_, body)| body),
            retained.split_once("\r\n\r\n").map(|(_, body)| body),
            "reset never rewrites a recorded frame"
        );
    }

    #[test]
    fn newest_snapshot_eventually_replaces_a_saturated_projection_slot() {
        let mut server = UiServer::start(None, initial()).expect("server starts");
        for revision in 1..=8 {
            server.publish(
                UiSnapshot::new(
                    revision,
                    &serde_json::json!({"revision": revision, "state": "paused"}),
                )
                .expect("snapshot JSON"),
            );
        }
        let mut response = String::new();
        for _ in 0..200 {
            server.flush();
            response = request(
                server.address,
                &format!(
                    "GET /api/snapshot?token={} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                    server.capability, server.address
                ),
            );
            if response.contains("\"revision\":8") {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(response.contains("\"revision\":8"));
    }
}
