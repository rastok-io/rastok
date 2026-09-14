//! Resident stdio resources. Only this module owns physical child handles;
//! Session identity, admission, deadlines and close-once belong to the core.

use super::{ProcessFailure, ProcessRequestBindings, frame_channel, process_status};
use crate::boundary::*;
use crate::connectors::{BoundConnectorDriver, ConnectorDriverFactory, DriverFactoryError};
use crate::envelope::{Data, Envelope, Namespace, Value};
use crate::node;
use crate::plan::ValidatedExecutionPlan;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
    mpsc,
};

// Local pacing limits, not journal facts. Control has a separate reserved
// atomic slot so full pipes/queues cannot prevent forced resource cleanup.
const MAX_RESOURCES: usize = 64;
const FRAME_SLOTS: usize = 8;
const COOPERATIVE: u8 = 1;
const FORCED: u8 = 2;
const RELEASED: u8 = 3;

#[derive(Serialize, Deserialize)]
struct Opening {
    command: String,
    args: Vec<String>,
    cwd: Option<String>,
    env: BTreeMap<String, String>,
    codec: StreamCodec,
    err_codec: StreamCodec,
    max_frame: u64,
    stdin: Vec<u8>,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum StreamCodec {
    Lines,
    Jsonl,
}

fn failure(failure: ProcessFailure) -> PortOutput {
    // Every Process failure constructor has exactly one exit.
    PortOutput {
        port: node::Port::conventional_exit(),
        data: failure.into_completion().outputs()[0].data.clone(),
        ext: Namespace::new(),
    }
}

pub(super) fn prepare(
    bindings: &ProcessRequestBindings,
    connector: &node::Path,
    data: &Data,
    settings: &BTreeMap<String, Value>,
    opening: bool,
) -> RequestPreparation {
    let resolved = bindings
        .get(connector)
        .ok_or_else(|| "unbound Process service".to_owned())
        .and_then(|binding| binding.resolve(settings).map_err(|e| e.to_string()));
    let settings = match resolved {
        Ok(settings) => settings,
        Err(message) => {
            return RequestPreparation::Rejected(
                ProcessFailure::SpawnFailed { message }.into_completion(),
            );
        }
    };
    let prepared = match bindings.prepare_request(connector, data) {
        RequestPreparation::Prepared(prepared) => prepared,
        rejected => return rejected,
    };
    let bytes = prepared.bytes();
    if bytes.len().saturating_sub(1) as u64 > settings.max_output {
        return RequestPreparation::Rejected(
            ProcessFailure::OutputOverflow {
                limit: settings.max_output,
                reason: OutputOverflowReason::Budget,
            }
            .into_completion(),
        );
    }
    if bytes.strip_suffix(b"\n").unwrap_or(bytes).contains(&b'\n') {
        return RequestPreparation::Rejected(
            ProcessFailure::DecodeFailed {
                message: "one service write must contain one complete line".into(),
                line: None,
            }
            .into_completion(),
        );
    }
    if !opening {
        return RequestPreparation::Prepared(prepared);
    }
    let descriptor = Opening {
        command: settings.command,
        args: settings.args,
        cwd: settings.cwd,
        env: settings.env,
        codec: match settings.codec {
            RequestCodec::Jsonl => StreamCodec::Jsonl,
            RequestCodec::Lines => StreamCodec::Lines,
            RequestCodec::Bytes => {
                return RequestPreparation::Rejected(
                    ProcessFailure::DecodeFailed {
                        message: "raw service framing is not defined".into(),
                        line: None,
                    }
                    .into_completion(),
                );
            }
        },
        err_codec: StreamCodec::Lines,
        max_frame: settings.max_output,
        stdin: bytes.to_vec(),
    };
    match serde_json::to_vec(&descriptor) {
        Ok(bytes) => RequestPreparation::Prepared(PreparedRequest::seal(bytes)),
        Err(error) => RequestPreparation::Rejected(
            ProcessFailure::SpawnFailed {
                message: error.to_string(),
            }
            .into_completion(),
        ),
    }
}

/// The resident, both-role Process contract. Binding performs no effects.
#[derive(Debug)]
pub struct ProcessServiceFactory {
    id: crate::connector::ContractId,
}

impl ProcessServiceFactory {
    /// Parses the built-in resident contract id.
    ///
    /// # Errors
    /// Returns an invalid built-in identity error.
    pub fn try_new() -> Result<Self, crate::connector::ConnectorIdError> {
        Ok(Self {
            id: crate::connector::ContractId::parse(
                crate::connector::PROCESS_SERVICE_CONNECTOR_ID,
            )?,
        })
    }
}

impl ConnectorDriverFactory for ProcessServiceFactory {
    fn connector_id(&self) -> &crate::connector::ContractId {
        &self.id
    }
    fn roles(&self) -> crate::connector::ConnectorRoles {
        crate::connector::ConnectorRoles::InboundAndOutbound
    }
    fn bind<'a>(
        &self,
        plan: &'a ValidatedExecutionPlan,
        nodes: &[node::Path],
    ) -> Result<Box<dyn BoundConnectorDriver + 'a>, DriverFactoryError> {
        if !cfg!(unix) {
            return Err(DriverFactoryError::new(
                "resident Process requires the qualified Unix process-group Driver",
            ));
        }
        let bindings = ProcessRequestBindings::try_bind(plan, nodes)
            .map_err(|e| DriverFactoryError::new(e.to_string()))?;
        Ok(Box::new(Service {
            bindings,
            resources: BTreeMap::new(),
            retiring: Vec::new(),
            events: Vec::new(),
            proposal: None,
            next_proposal: 0,
        }))
    }
}

struct Service {
    bindings: ProcessRequestBindings,
    resources: BTreeMap<SessionRef, Resource>,
    retiring: Vec<std::thread::JoinHandle<()>>,
    events: Vec<SessionDriverEvent>,
    proposal: Option<InboundProposal>,
    next_proposal: u64,
}

struct Resource {
    connector: node::Path,
    control: Arc<AtomicU8>,
    writes: mpsc::SyncSender<WriteFrame>,
    events: mpsc::Receiver<Event>,
    waiting: VecDeque<Event>,
    worker: std::thread::JoinHandle<()>,
}
struct WriteFrame {
    reference: SessionWriteRef,
    bytes: Vec<u8>,
}
enum Event {
    Opened,
    Written(SessionWriteRef),
    Frame(node::Port, Data),
    Exit(PortOutput),
}

impl BoundConnectorDriver for Service {
    fn inbound(&mut self) -> Option<&mut dyn InboundConnectorDriver> {
        Some(self)
    }
    fn outbound(&mut self) -> Option<&mut dyn OutboundConnectorDriver> {
        Some(self)
    }
}
impl OutboundConnectorDriver for Service {
    fn call(&mut self) -> Option<&mut dyn CallConnectorDriver> {
        None
    }
    fn session(&mut self) -> Option<&mut dyn SessionConnectorDriver> {
        Some(self)
    }
}
impl SessionConnectorDriver for Service {
    fn prepare_session_open(
        &mut self,
        connector: &node::Path,
        data: &Data,
        settings: &BTreeMap<String, Value>,
    ) -> RequestPreparation {
        prepare(&self.bindings, connector, data, settings, true)
    }
    fn scheduler_session_completion(
        &mut self,
        _: &node::Path,
        completion: SchedulerCompletion,
    ) -> Completion {
        crate::boundary::process_scheduler_completion(completion)
    }
    fn prepare_session_write(
        &mut self,
        connector: &node::Path,
        _: SessionRef,
        data: &Data,
        settings: &BTreeMap<String, Value>,
    ) -> RequestPreparation {
        prepare(&self.bindings, connector, data, settings, false)
    }
    fn open_session(&mut self, intent: &SessionOpenIntent, _: &Envelope) -> SessionStart {
        if self.resources.contains_key(&intent.session_ref) {
            return SessionStart::Started;
        }
        self.reap_finished();
        if self.resources.len() + self.retiring.len() >= MAX_RESOURCES {
            return SessionStart::Pending;
        }
        let opening: Opening = match serde_json::from_slice(intent.prepared.bytes()) {
            Ok(opening) => opening,
            Err(error) => {
                self.events.push(SessionDriverEvent::Exited {
                    session: intent.session_ref,
                    exit: failure(ProcessFailure::SpawnFailed {
                        message: error.to_string(),
                    }),
                });
                return SessionStart::Started;
            }
        };
        let (tx, rx) = mpsc::sync_channel(FRAME_SLOTS);
        let (writes, input) = mpsc::sync_channel(FRAME_SLOTS);
        let control = Arc::new(AtomicU8::new(0));
        let child_control = Arc::clone(&control);
        match std::thread::Builder::new()
            .name("rastok-process-service".into())
            .spawn(move || supervise(opening, input, tx, child_control))
        {
            Ok(worker) => {
                self.resources.insert(
                    intent.session_ref,
                    Resource {
                        connector: intent.connector_path.clone(),
                        control,
                        writes,
                        events: rx,
                        waiting: VecDeque::new(),
                        worker,
                    },
                );
            }
            Err(error) => self.events.push(SessionDriverEvent::Exited {
                session: intent.session_ref,
                exit: failure(ProcessFailure::SpawnFailed {
                    message: error.to_string(),
                }),
            }),
        }
        SessionStart::Started
    }
    fn write_session(&mut self, intent: &SessionWriteIntent, _: &Envelope) -> SessionStart {
        let Some(resource) = self.resources.get_mut(&intent.session_ref) else {
            return SessionStart::Pending;
        };
        if resource.control.load(Ordering::Acquire) != 0 {
            return SessionStart::Pending;
        }
        match resource.writes.try_send(WriteFrame {
            reference: intent.write_ref,
            bytes: intent.prepared.bytes().to_vec(),
        }) {
            Ok(()) => SessionStart::Started,
            Err(_) => SessionStart::Pending,
        }
    }
    fn close_session(&mut self, reference: SessionRef) -> SessionStart {
        if let Some(resource) = self.resources.get(&reference) {
            resource.control.fetch_max(COOPERATIVE, Ordering::Release);
        }
        SessionStart::Started
    }
    fn abort_session(&mut self, reference: SessionRef) {
        if let Some(resource) = self.resources.get(&reference) {
            resource.control.fetch_max(FORCED, Ordering::Release);
        }
    }
    fn release_session(&mut self, reference: SessionRef) {
        if let Some(resource) = self.resources.remove(&reference) {
            resource.control.store(RELEASED, Ordering::Release);
            self.retiring.push(resource.worker);
        }
        if self
            .proposal
            .as_ref()
            .is_some_and(|p| p.session == Some(InboundSession::Message(reference)))
        {
            self.proposal = None;
        }
        self.events.retain(|event| match event {
            SessionDriverEvent::Opened(s)
            | SessionDriverEvent::WriteSettled { session: s, .. }
            | SessionDriverEvent::Exited { session: s, .. } => *s != reference,
        });
    }
    fn session_events(&mut self) -> Vec<SessionDriverEvent> {
        self.poll();
        std::mem::take(&mut self.events)
    }
}

impl Service {
    fn reap_finished(&mut self) {
        let mut index = 0;
        while index < self.retiring.len() {
            if self.retiring[index].is_finished() {
                let _ = self.retiring.swap_remove(index).join();
            } else {
                index += 1;
            }
        }
    }

    fn poll(&mut self) {
        self.reap_finished();
        for (reference, resource) in &mut self.resources {
            // Never read beyond a pending frame. Its terminal may not overtake
            // it, even when downstream admission is backpressured.
            if !resource.waiting.is_empty() {
                continue;
            }
            for event in resource.events.try_iter().take(FRAME_SLOTS) {
                match event {
                    Event::Opened => self.events.push(SessionDriverEvent::Opened(*reference)),
                    Event::Written(write) => self.events.push(SessionDriverEvent::WriteSettled {
                        session: *reference,
                        write,
                    }),
                    Event::Exit(exit) => {
                        self.events.push(SessionDriverEvent::Exited {
                            session: *reference,
                            exit,
                        });
                        break;
                    }
                    frame @ Event::Frame(..) => {
                        resource.waiting.push_back(frame);
                        break;
                    }
                }
            }
        }
    }
}
impl InboundConnectorDriver for Service {
    fn proposal(&mut self) -> Option<InboundProposal> {
        if self.proposal.is_some() {
            return self.proposal.clone();
        }
        self.poll();
        for (reference, resource) in &self.resources {
            // A fast child can queue Opened and its first frame in one poll.
            // Hand the opening to the core before proposing that frame:
            // otherwise admission correctly rejects an unopened Session.
            if self.events.iter().any(|event| {
                matches!(event,
                SessionDriverEvent::Opened(opening) if opening == reference)
            }) {
                continue;
            }
            if let Some(Event::Frame(port, data)) = resource.waiting.front() {
                self.next_proposal = self.next_proposal.checked_add(1)?;
                self.proposal = Some(InboundProposal {
                    reference: InboundProposalRef::new(self.next_proposal),
                    submission: InboundSubmission {
                        provenance: InboundEnvelopeProvenance::single(resource.connector.clone()),
                        port: port.clone(),
                        draft: Draft {
                            data: data.clone(),
                            ext: Namespace::new(),
                        },
                        sequence: SubmissionSequence::Next,
                    },
                    session: Some(InboundSession::Message(*reference)),
                });
                break;
            }
        }
        self.proposal.clone()
    }
    fn decide(&mut self, reference: InboundProposalRef, decision: InboundDecision) {
        if self
            .proposal
            .as_ref()
            .is_none_or(|p| p.reference != reference)
        {
            return;
        }
        if let Some(proposal) = self.proposal.take()
            && let Some(InboundSession::Message(session)) = proposal.session
            && let Some(resource) = self.resources.get_mut(&session)
        {
            resource.waiting.pop_front();
            if matches!(decision, InboundDecision::Rejected(_)) {
                resource.control.fetch_max(FORCED, Ordering::Release);
            }
        }
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        for resource in self.resources.values() {
            resource.control.store(RELEASED, Ordering::Release);
        }
        // This is host teardown, outside VM transactions and Session callbacks.
        // Merely setting the flag lets main exit before the workers kill/reap.
        for (_, resource) in std::mem::take(&mut self.resources) {
            let _ = resource.worker.join();
        }
        for worker in self.retiring.drain(..) {
            let _ = worker.join();
        }
    }
}

// Blocking pipe operations live only on workers. The sleep paces OS polling;
// it chooses no semantic deadline and never creates TimeoutExpired.
fn pace() {
    std::thread::sleep(std::time::Duration::from_millis(1));
}
fn send<T>(tx: &mpsc::SyncSender<T>, mut event: T, control: &AtomicU8) -> bool {
    loop {
        if control.load(Ordering::Acquire) >= RELEASED {
            return false;
        }
        match tx.try_send(event) {
            Ok(()) => return true,
            Err(mpsc::TrySendError::Full(retained)) => {
                event = retained;
                pace();
            }
            Err(mpsc::TrySendError::Disconnected(_)) => return false,
        }
    }
}

fn supervise(
    opening: Opening,
    input: mpsc::Receiver<WriteFrame>,
    tx: mpsc::SyncSender<Event>,
    control: Arc<AtomicU8>,
) {
    let mut command = Command::new(&opening.command);
    command
        .args(&opening.args)
        .envs(&opening.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = &opening.cwd {
        command.current_dir(cwd);
    }
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    if control.load(Ordering::Acquire) != 0 {
        return;
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            send(
                &tx,
                Event::Exit(failure(ProcessFailure::SpawnFailed {
                    message: error.to_string(),
                })),
                &control,
            );
            return;
        }
    };
    let (frames, drained) = mpsc::sync_channel(FRAME_SLOTS);
    let (writer_events, writes_done) = mpsc::sync_channel(FRAME_SLOTS);
    let mut threads = Vec::new();
    let mut outgoing = VecDeque::from([Event::Opened]);
    if let Some(mut stdin) = child.stdin.take() {
        let control = Arc::clone(&control);
        let first = opening.stdin;
        threads.push(
            std::thread::Builder::new()
                .name("rastok-stdin".into())
                .spawn(move || {
                    if stdin.write_all(&first).is_err() {
                        send(
                            &writer_events,
                            Event::Exit(failure(ProcessFailure::CaptureFailed)),
                            &control,
                        );
                        return;
                    }
                    while control.load(Ordering::Acquire) == 0 {
                        match input.recv_timeout(std::time::Duration::from_millis(10)) {
                            Ok(write) => {
                                if stdin.write_all(&write.bytes).is_err() {
                                    send(
                                        &writer_events,
                                        Event::Exit(failure(ProcessFailure::CaptureFailed)),
                                        &control,
                                    );
                                    break;
                                }
                                if !send(&writer_events, Event::Written(write.reference), &control)
                                {
                                    break;
                                }
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    }
                    // Keep stdin open until resource termination. EOF is not our
                    // cooperative shutdown protocol.
                    while control.load(Ordering::Acquire) < FORCED {
                        pace();
                    }
                }),
        );
    }
    let limit = opening.max_frame;
    if let Some(stdout) = child.stdout.take() {
        let tx = frames.clone();
        let control = Arc::clone(&control);
        let codec = opening.codec;
        threads.push(
            std::thread::Builder::new()
                .name("rastok-stdout".into())
                .spawn(move || {
                    read_frames(
                        stdout,
                        codec,
                        limit,
                        node::Port::conventional_out(),
                        tx,
                        control,
                    )
                }),
        );
    }
    if let Some(stderr) = child.stderr.take() {
        let tx = frames.clone();
        let control = Arc::clone(&control);
        let codec = opening.err_codec;
        threads.push(
            std::thread::Builder::new()
                .name("rastok-stderr".into())
                .spawn(move || {
                    read_frames(
                        stderr,
                        codec,
                        limit,
                        node::Port::conventional_err(),
                        tx,
                        control,
                    )
                }),
        );
    }
    drop(frames);
    let mut terminal = threads
        .iter()
        .any(Result::is_err)
        .then(|| failure(ProcessFailure::CaptureFailed));
    let mut status = None;
    let mut signaled = 0;
    let mut readers_done = false;
    loop {
        let requested = control.load(Ordering::Acquire);
        let signal = if terminal.is_some() || status.is_some() || requested >= FORCED {
            FORCED
        } else {
            requested
        };
        if signal > signaled {
            signal_group(child.id(), signal);
            signaled = signal;
        }
        if requested >= RELEASED {
            let _ = child.kill();
            let _ = child.wait();
            for thread in threads.into_iter().flatten() {
                let _ = thread.join();
            }
            return;
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(observed) => status = observed,
                Err(_) => {
                    terminal = Some(failure(ProcessFailure::CaptureFailed));
                }
            }
        }
        if outgoing.is_empty() {
            for event in writes_done.try_iter().take(FRAME_SLOTS) {
                match event {
                    Event::Exit(error) => {
                        terminal.get_or_insert(error);
                    }
                    event => outgoing.push_back(event),
                }
            }
            match drained.try_recv() {
                Ok(Ok((port, data))) => outgoing.push_back(Event::Frame(port, data)),
                Ok(Err(error)) => {
                    terminal.get_or_insert_with(|| failure(error));
                }
                Err(mpsc::TryRecvError::Disconnected) => readers_done = true,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(event) = outgoing.pop_front() {
            match tx.try_send(event) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(event)) => outgoing.push_front(event),
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    control.store(RELEASED, Ordering::Release);
                }
            }
        }
        if status.is_some() && readers_done && outgoing.is_empty() {
            control.fetch_max(FORCED, Ordering::Release);
            if let Some(status) = status {
                let exit = terminal.unwrap_or_else(|| {
                    let (exit_code, signal) = process_status(status);
                    Completion::process_exit(ProcessExit::Exited { exit_code, signal }).outputs()[0]
                        .clone()
                });
                send(&tx, Event::Exit(exit), &control);
            }
            return;
        }
        pace();
    }
}

fn signal_group(pid: u32, signal: u8) {
    #[cfg(unix)]
    {
        let _ = Command::new("/bin/kill")
            .args([
                if signal >= FORCED { "-KILL" } else { "-TERM" },
                "--",
                &format!("-{pid}"),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, signal);
    }
}

type ReadEvent = Result<(node::Port, Data), ProcessFailure>;
fn read_frames(
    reader: impl Read,
    codec: StreamCodec,
    limit: u64,
    port: node::Port,
    tx: mpsc::SyncSender<ReadEvent>,
    control: Arc<AtomicU8>,
) {
    let mut reader = BufReader::with_capacity(8192, reader);
    let mut frame = Vec::new();
    loop {
        let available = match reader.fill_buf() {
            Ok(bytes) => bytes,
            Err(_) => {
                send(&tx, Err(ProcessFailure::CaptureFailed), &control);
                return;
            }
        };
        let eof = available.is_empty();
        let count = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |i| i + 1);
        let terminated = count > 0 && available[count - 1] == b'\n';
        let content_len = (frame.len() as u64)
            .saturating_add(count as u64)
            .saturating_sub(u64::from(terminated));
        if content_len > limit {
            send(
                &tx,
                Err(ProcessFailure::OutputOverflow {
                    limit,
                    reason: OutputOverflowReason::Budget,
                }),
                &control,
            );
            return;
        }
        if frame.try_reserve(count).is_err() {
            send(
                &tx,
                Err(ProcessFailure::OutputOverflow {
                    limit,
                    reason: OutputOverflowReason::Capacity,
                }),
                &control,
            );
            return;
        }
        frame.extend_from_slice(&available[..count]);
        reader.consume(count);
        if terminated || (eof && !frame.is_empty()) {
            let codec = match codec {
                StreamCodec::Jsonl => RequestCodec::Jsonl,
                StreamCodec::Lines => RequestCodec::Lines,
            };
            match frame_channel(std::mem::take(&mut frame), codec, "service frame") {
                Ok(values) => {
                    for data in values {
                        if !send(&tx, Ok((port.clone(), data)), &control) {
                            return;
                        }
                    }
                }
                Err(error) => {
                    send(&tx, Err(error), &control);
                    return;
                }
            }
        }
        if eof || control.load(Ordering::Acquire) >= RELEASED {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_immediate_frame_cannot_overtake_the_session_opened_event() {
        let compiled = crate::compiler::compile(
            &[include_str!("../../../../../examples/echo.rkg")],
            &crate::contract::ContractCatalog::try_m0().unwrap(),
        )
        .unwrap();
        let reference = SessionRef::new(crate::envelope::Id::from_bytes([7; 16]));
        let (sender, events) = mpsc::sync_channel(FRAME_SLOTS);
        sender.send(Event::Opened).unwrap();
        sender
            .send(Event::Frame(
                node::Port::conventional_out(),
                Data::materialized(Value::Int(1)),
            ))
            .unwrap();
        let (writes, _input) = mpsc::sync_channel(FRAME_SLOTS);
        let mut service = Service {
            bindings: ProcessRequestBindings::try_bind(&compiled.plan, &[]).unwrap(),
            resources: BTreeMap::from([(
                reference,
                Resource {
                    connector: node::Path::new(["resident"]).unwrap(),
                    control: Arc::new(AtomicU8::new(0)),
                    writes,
                    events,
                    waiting: VecDeque::new(),
                    worker: std::thread::spawn(|| {}),
                },
            )]),
            retiring: Vec::new(),
            events: Vec::new(),
            proposal: None,
            next_proposal: 0,
        };
        assert!(
            service.proposal().is_none(),
            "the journal has not received Opened yet"
        );
        assert_eq!(
            service.session_events(),
            vec![SessionDriverEvent::Opened(reference)]
        );
        assert!(
            service.proposal().is_some(),
            "the retained opening precedes the child frame"
        );
    }

    /// The host must not exit while a released resource still owns a child.
    #[test]
    fn host_teardown_waits_for_retired_cleanup_without_blocking_session_release() {
        use std::sync::atomic::AtomicBool;
        let compiled = crate::compiler::compile(
            &[include_str!("../../../../../examples/echo.rkg")],
            &crate::contract::ContractCatalog::try_m0().unwrap(),
        )
        .unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let finished = Arc::clone(&done);
        let worker = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(10));
            finished.store(true, Ordering::Release);
        });
        let service = Service {
            bindings: ProcessRequestBindings::try_bind(&compiled.plan, &[]).unwrap(),
            resources: BTreeMap::new(),
            retiring: vec![worker],
            events: Vec::new(),
            proposal: None,
            next_proposal: 0,
        };
        drop(service);
        assert!(done.load(Ordering::Acquire));
    }

    #[test]
    fn framing_is_bounded_by_complete_frames_not_read_chunks() {
        for (bytes, codec, limit, count, failed) in [
            (b"ab\n\nc".as_slice(), StreamCodec::Lines, 2, 3, false),
            (b"abc\n".as_slice(), StreamCodec::Lines, 2, 0, true),
            (b"12\n3".as_slice(), StreamCodec::Jsonl, 2, 2, false),
            (b"1\n{".as_slice(), StreamCodec::Jsonl, 2, 1, true),
            (b"\xff\n".as_slice(), StreamCodec::Lines, 2, 0, true),
            (b"\n".as_slice(), StreamCodec::Jsonl, 2, 0, true),
        ] {
            let (tx, rx) = mpsc::sync_channel(FRAME_SLOTS);
            read_frames(
                bytes,
                codec,
                limit,
                node::Port::conventional_out(),
                tx,
                Arc::new(AtomicU8::new(0)),
            );
            let values: Vec<_> = rx.try_iter().collect();
            assert_eq!(values.iter().filter(|v| v.is_ok()).count(), count);
            assert_eq!(values.last().is_some_and(Result::is_err), failed);
        }
    }

    /// OS integration: reserved control must survive a full publication pipe.
    #[cfg(unix)]
    #[test]
    fn cleanup_reaps_the_child_even_when_frame_delivery_is_backpressured() {
        use std::time::{Duration, Instant};
        let (tx, rx) = mpsc::sync_channel(1);
        let (_writes, input) = mpsc::sync_channel(1);
        let control = Arc::new(AtomicU8::new(0));
        let worker_control = Arc::clone(&control);
        let worker = std::thread::spawn(move || {
            supervise(
                Opening {
                    command: "/bin/sh".into(),
                    args: vec![
                        "-c".into(),
                        "read line; while :; do printf 'frame\\n'; done".into(),
                    ],
                    cwd: None,
                    env: BTreeMap::new(),
                    codec: StreamCodec::Lines,
                    err_codec: StreamCodec::Lines,
                    max_frame: 16,
                    stdin: b"start\n".to_vec(),
                },
                input,
                tx,
                worker_control,
            )
        });
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(15)).unwrap(),
            Event::Opened
        ));
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(15)).unwrap(),
            Event::Frame(..)
        ));
        // Stop draining the bounded channel, then issue control independently.
        control.store(RELEASED, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(15);
        while !worker.is_finished() {
            assert!(
                Instant::now() < deadline,
                "cleanup waited on a full frame pipe"
            );
            pace();
        }
        worker.join().unwrap();
    }
}
