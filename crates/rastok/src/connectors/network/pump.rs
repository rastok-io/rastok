use std::error::Error;
use std::fmt;
use std::future::Future;
use std::thread::JoinHandle;

use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, watch};

/// Physical bounds for one hosted network engine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct EngineLimits {
    work_capacity: usize,
    control_capacity: usize,
    event_capacity: usize,
    buffered_bytes: u32,
}

impl EngineLimits {
    pub(super) fn new(
        work_capacity: usize,
        control_capacity: usize,
        event_capacity: usize,
        buffered_bytes: usize,
    ) -> Result<Self, EngineStartError> {
        if work_capacity == 0 || control_capacity == 0 || event_capacity == 0 {
            return Err(EngineStartError::InvalidLimits(
                "every hosted-network channel capacity must be positive",
            ));
        }
        let buffered_bytes = u32::try_from(buffered_bytes).map_err(|_| {
            EngineStartError::InvalidLimits(
                "the hosted-network byte budget must fit the semaphore counter",
            )
        })?;
        if buffered_bytes == 0 {
            return Err(EngineStartError::InvalidLimits(
                "the hosted-network byte budget must be positive",
            ));
        }
        Ok(Self {
            work_capacity,
            control_capacity,
            event_capacity,
            buffered_bytes,
        })
    }

    pub(super) const fn buffered_bytes(self) -> usize {
        self.buffered_bytes as usize
    }

    pub(super) const fn work_capacity(self) -> usize {
        self.work_capacity
    }

    pub(super) const fn control_capacity(self) -> usize {
        self.control_capacity
    }
}

/// Failure to construct the physical pump before any socket is opened.
#[derive(Debug)]
pub(super) enum EngineStartError {
    InvalidLimits(&'static str),
    Runtime(std::io::Error),
    Thread(std::io::Error),
}

impl fmt::Display for EngineStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimits(reason) => formatter.write_str(reason),
            Self::Runtime(error) => {
                write!(formatter, "the network runtime could not start: {error}")
            }
            Self::Thread(error) => write!(formatter, "the network thread could not start: {error}"),
        }
    }
}

impl Error for EngineStartError {}

#[derive(Debug)]
pub(super) struct Budgeted<T> {
    value: T,
    _permits: Vec<OwnedSemaphorePermit>,
}

impl<T> Budgeted<T> {
    pub(super) const fn value(&self) -> &T {
        &self.value
    }

    pub(super) fn into_inner(self) -> T {
        self.value
    }
}

/// Byte permits acquired before an inbound protocol copies payload bytes.
///
/// A server cannot first allocate a complete request and only then discover
/// that the shared pump budget is exhausted. Reservations therefore grow one
/// observed chunk at a time and move into the event lane on success.
#[derive(Debug, Default)]
pub(super) struct EventReservation {
    permits: Vec<OwnedSemaphorePermit>,
}

impl EventReservation {
    pub(super) fn append(&mut self, mut other: Self) {
        self.permits.append(&mut other.permits);
    }
}

/// A byte reservation could not be made without exceeding the physical pump
/// boundary or after shutdown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReserveError {
    Oversized { bytes: usize, limit: usize },
    Stopped,
}

/// One item selected by the async side of the pump.
#[derive(Debug)]
pub(super) enum EngineInput<W, C> {
    Work(Budgeted<W>),
    Control(C),
}

/// Non-blocking observation of the async-to-synchronous event lane.
#[derive(Debug)]
pub(super) enum EventPoll<E> {
    Event(E),
    Empty,
    Stopped,
}

/// A bounded event drain that preserves whether the producer has stopped.
#[derive(Debug)]
pub(super) struct EventBatch<E> {
    pub(super) events: Vec<E>,
    pub(super) stopped: bool,
}

/// A synchronous submission failed without consuming the caller's item.
#[derive(Debug)]
pub(super) enum SubmitError<T> {
    Full(T),
    ByteBudget(T),
    Oversized {
        value: T,
        bytes: usize,
        limit: usize,
    },
    Stopped(T),
}

impl<T> SubmitError<T> {
    pub(super) fn into_inner(self) -> T {
        match self {
            Self::Full(value)
            | Self::ByteBudget(value)
            | Self::Stopped(value)
            | Self::Oversized { value, .. } => value,
        }
    }
}

/// An async event could not cross back to the synchronous Driver.
#[derive(Debug)]
pub(super) enum EmitError<T> {
    Oversized {
        value: T,
        bytes: usize,
        limit: usize,
    },
    Stopped(T),
}

/// Cloneable async-to-synchronous event capability for concurrent protocol
/// streams. Every clone shares the same finite lane and byte budget.
pub(super) struct EventSender<E> {
    events: mpsc::Sender<Budgeted<E>>,
    byte_budget: std::sync::Arc<Semaphore>,
    shutdown: watch::Receiver<bool>,
    max_buffered_bytes: u32,
}

impl<E> Clone for EventSender<E> {
    fn clone(&self) -> Self {
        Self {
            events: self.events.clone(),
            byte_budget: self.byte_budget.clone(),
            shutdown: self.shutdown.clone(),
            max_buffered_bytes: self.max_buffered_bytes,
        }
    }
}

impl<E> EventSender<E> {
    /// Reserves bytes before an inbound protocol copies them into an event.
    pub(super) async fn reserve(&mut self, bytes: usize) -> Result<EventReservation, ReserveError> {
        if bytes == 0 {
            return Ok(EventReservation::default());
        }
        let permits = match u32::try_from(bytes) {
            Ok(permits) if permits <= self.max_buffered_bytes => permits,
            _ => {
                return Err(ReserveError::Oversized {
                    bytes,
                    limit: self.max_buffered_bytes as usize,
                });
            }
        };
        let budget = self.byte_budget.clone();
        let permit = tokio::select! {
            changed = self.shutdown.changed() => {
                let _ = changed;
                return Err(ReserveError::Stopped);
            }
            acquired = budget.acquire_many_owned(permits) => match acquired {
                Ok(permit) => permit,
                Err(_) => return Err(ReserveError::Stopped),
            }
        };
        Ok(EventReservation {
            permits: vec![permit],
        })
    }

    pub(super) async fn emit(&mut self, event: E, bytes: usize) -> Result<(), EmitError<E>> {
        let reservation = match self.reserve(bytes).await {
            Ok(reservation) => reservation,
            Err(ReserveError::Oversized { bytes, limit }) => {
                return Err(EmitError::Oversized {
                    value: event,
                    bytes,
                    limit,
                });
            }
            Err(ReserveError::Stopped) => return Err(EmitError::Stopped(event)),
        };
        self.emit_reserved(event, reservation).await
    }

    /// Moves an already charged inbound value onto the finite event lane.
    pub(super) async fn emit_reserved(
        &mut self,
        event: E,
        reservation: EventReservation,
    ) -> Result<(), EmitError<E>> {
        let buffered = Budgeted {
            value: event,
            _permits: reservation.permits,
        };
        match self.events.send(buffered).await {
            Ok(()) => Ok(()),
            Err(error) => Err(EmitError::Stopped(error.0.value)),
        }
    }
}

impl<T> EmitError<T> {
    pub(super) fn into_inner(self) -> T {
        match self {
            Self::Stopped(value) | Self::Oversized { value, .. } => value,
        }
    }
}

/// Async half owned exclusively by the hosted network thread.
pub(super) struct NetworkRuntime<W, C, E> {
    work: mpsc::Receiver<Budgeted<W>>,
    control: mpsc::Receiver<C>,
    events: mpsc::Sender<Budgeted<E>>,
    byte_budget: std::sync::Arc<Semaphore>,
    shutdown: watch::Receiver<bool>,
    max_buffered_bytes: u32,
}

impl<W, C, E> NetworkRuntime<W, C, E> {
    pub(super) async fn recv(&mut self) -> Option<EngineInput<W, C>> {
        tokio::select! {
            biased;
            changed = self.shutdown.changed() => {
                let _ = changed;
                None
            }
            control = self.control.recv() => control.map(EngineInput::Control),
            work = self.work.recv() => work.map(EngineInput::Work),
        }
    }

    pub(super) async fn recv_control(&mut self) -> Option<C> {
        tokio::select! {
            changed = self.shutdown.changed() => {
                let _ = changed;
                None
            }
            control = self.control.recv() => control,
        }
    }

    pub(super) async fn recv_work(&mut self) -> Option<Budgeted<W>> {
        tokio::select! {
            changed = self.shutdown.changed() => {
                let _ = changed;
                None
            }
            work = self.work.recv() => work,
        }
    }

    pub(super) async fn emit(&mut self, event: E, bytes: usize) -> Result<(), EmitError<E>> {
        self.event_sender().emit(event, bytes).await
    }

    pub(super) fn event_sender(&self) -> EventSender<E> {
        EventSender {
            events: self.events.clone(),
            byte_budget: self.byte_budget.clone(),
            shutdown: self.shutdown.clone(),
            max_buffered_bytes: self.max_buffered_bytes,
        }
    }

    pub(super) const fn max_buffered_bytes(&self) -> usize {
        self.max_buffered_bytes as usize
    }
}

/// Synchronous execution-facing handle for one client or server network pump.
pub(super) struct HostedNetworkEngine<W, C, E> {
    work: Option<mpsc::Sender<Budgeted<W>>>,
    control: Option<mpsc::Sender<C>>,
    events: Option<mpsc::Receiver<Budgeted<E>>>,
    byte_budget: std::sync::Arc<Semaphore>,
    shutdown: watch::Sender<bool>,
    thread: Option<JoinHandle<()>>,
    max_buffered_bytes: u32,
}

impl<W, C, E> fmt::Debug for HostedNetworkEngine<W, C, E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostedNetworkEngine")
            .field("max_buffered_bytes", &self.max_buffered_bytes)
            .finish_non_exhaustive()
    }
}

impl<W, C, E> HostedNetworkEngine<W, C, E>
where
    W: Send + 'static,
    C: Send + 'static,
    E: Send + 'static,
{
    pub(super) fn start<F, Fut>(
        thread_name: &str,
        limits: EngineLimits,
        run: F,
    ) -> Result<Self, EngineStartError>
    where
        F: FnOnce(NetworkRuntime<W, C, E>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(EngineStartError::Runtime)?;
        let (work_tx, work_rx) = mpsc::channel(limits.work_capacity);
        let (control_tx, control_rx) = mpsc::channel(limits.control_capacity);
        let (event_tx, event_rx) = mpsc::channel(limits.event_capacity);
        let byte_budget = std::sync::Arc::new(Semaphore::new(limits.buffered_bytes()));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let mut supervisor_shutdown = shutdown_rx.clone();
        let async_budget = byte_budget.clone();
        let max_buffered_bytes = limits.buffered_bytes;
        let network_runtime = NetworkRuntime {
            work: work_rx,
            control: control_rx,
            events: event_tx,
            byte_budget: async_budget,
            shutdown: shutdown_rx,
            max_buffered_bytes,
        };
        let thread = std::thread::Builder::new()
            .name(thread_name.to_string())
            .spawn(move || {
                runtime.block_on(async move {
                    tokio::select! {
                        biased;
                        _ = supervisor_shutdown.changed() => {}
                        _ = run(network_runtime) => {}
                    }
                });
            })
            .map_err(EngineStartError::Thread)?;
        Ok(Self {
            work: Some(work_tx),
            control: Some(control_tx),
            events: Some(event_rx),
            byte_budget,
            shutdown: shutdown_tx,
            thread: Some(thread),
            max_buffered_bytes,
        })
    }

    pub(super) fn try_submit(&self, value: W, bytes: usize) -> Result<(), SubmitError<W>> {
        let Some(work) = &self.work else {
            return Err(SubmitError::Stopped(value));
        };
        let permits = match u32::try_from(bytes) {
            Ok(permits) if permits <= self.max_buffered_bytes => permits,
            _ => {
                return Err(SubmitError::Oversized {
                    value,
                    bytes,
                    limit: self.max_buffered_bytes as usize,
                });
            }
        };
        let permit = match self.byte_budget.clone().try_acquire_many_owned(permits) {
            Ok(permit) => permit,
            Err(_) => return Err(SubmitError::ByteBudget(value)),
        };
        let buffered = Budgeted {
            value,
            _permits: vec![permit],
        };
        work.try_send(buffered).map_err(|error| match error {
            mpsc::error::TrySendError::Full(item) => SubmitError::Full(item.value),
            mpsc::error::TrySendError::Closed(item) => SubmitError::Stopped(item.value),
        })
    }

    pub(super) fn try_control(&self, value: C) -> Result<(), SubmitError<C>> {
        let Some(control) = &self.control else {
            return Err(SubmitError::Stopped(value));
        };
        control.try_send(value).map_err(|error| match error {
            mpsc::error::TrySendError::Full(item) => SubmitError::Full(item),
            mpsc::error::TrySendError::Closed(item) => SubmitError::Stopped(item),
        })
    }

    pub(super) fn try_event(&mut self) -> EventPoll<E> {
        let Some(events) = self.events.as_mut() else {
            return EventPoll::Stopped;
        };
        match events.try_recv() {
            Ok(event) => EventPoll::Event(event.into_inner()),
            Err(mpsc::error::TryRecvError::Empty) => EventPoll::Empty,
            Err(mpsc::error::TryRecvError::Disconnected) => EventPoll::Stopped,
        }
    }

    pub(super) fn drain_events(&mut self, limit: usize) -> EventBatch<E> {
        let mut events = Vec::new();
        let mut stopped = false;
        while events.len() < limit {
            match self.try_event() {
                EventPoll::Event(event) => events.push(event),
                EventPoll::Empty => break,
                EventPoll::Stopped => {
                    stopped = true;
                    break;
                }
            }
        }
        EventBatch { events, stopped }
    }
}

impl<W, C, E> Drop for HostedNetworkEngine<W, C, E> {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        self.work.take();
        self.control.take();
        self.events.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
// Wall time is only a liveness guard around a hosted OS thread here. It never
// enters a Rastok fact or scheduler decision.
#[allow(clippy::disallowed_types, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Side {
        Client,
        Server,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Control {
        Release,
        Cancel,
    }

    struct DropSignal(Arc<AtomicBool>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    fn limits() -> EngineLimits {
        EngineLimits::new(2, 1, 1, 8).expect("the test limits are valid")
    }

    #[test]
    fn every_physical_bound_is_positive_before_the_network_thread_starts() {
        for invalid in [
            EngineLimits::new(0, 1, 1, 1),
            EngineLimits::new(1, 0, 1, 1),
            EngineLimits::new(1, 1, 0, 1),
            EngineLimits::new(1, 1, 1, 0),
        ] {
            assert!(matches!(invalid, Err(EngineStartError::InvalidLimits(_))));
        }
    }

    fn wait_for_event<W, C, E>(engine: &mut HostedNetworkEngine<W, C, E>) -> E
    where
        W: Send + 'static,
        C: Send + 'static,
        E: Send + 'static,
    {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match engine.try_event() {
                EventPoll::Event(event) => return event,
                EventPoll::Empty => {}
                EventPoll::Stopped => panic!("the bounded pump stopped before its event"),
            }
            assert!(
                Instant::now() < deadline,
                "the bounded pump made no progress"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn one_bounded_pump_shape_serves_client_and_server() {
        for side in [Side::Client, Side::Server] {
            let mut engine = HostedNetworkEngine::start(
                "rastok-network-shared-shape",
                limits(),
                move |mut runtime| async move {
                    while let Some(input) = runtime.recv().await {
                        match input {
                            EngineInput::Work(work) => {
                                let bytes = *work.value();
                                drop(work);
                                if runtime.emit((side, bytes), bytes).await.is_err() {
                                    break;
                                }
                            }
                            EngineInput::Control(Control::Cancel) => {
                                let _ = runtime.emit((side, 0), 0).await;
                                break;
                            }
                            EngineInput::Control(Control::Release) => {}
                        }
                    }
                },
            )
            .expect("the shared pump must start");
            engine.try_submit(3, 3).expect("one work item fits");
            assert_eq!(wait_for_event(&mut engine), (side, 3));
            engine
                .try_control(Control::Cancel)
                .expect("the control lane is independent");
            assert_eq!(wait_for_event(&mut engine), (side, 0));
        }
    }

    #[test]
    fn work_count_and_bytes_apply_backpressure_without_blocking_control() {
        let mut engine = HostedNetworkEngine::start(
            "rastok-network-pressure",
            limits(),
            |mut runtime| async move {
                if runtime.recv_control().await == Some(Control::Cancel) {
                    let _ = runtime.emit("cancelled", 0).await;
                }
            },
        )
        .expect("the pressure pump must start");

        engine.try_submit("first", 4).expect("the first item fits");
        engine
            .try_submit("second", 4)
            .expect("the second item fits");
        let error = engine
            .try_submit("third", 1)
            .expect_err("the shared byte budget is exhausted");
        assert_eq!(error.into_inner(), "third");
        let error = engine
            .try_submit("oversized", 9)
            .expect_err("one item cannot exceed the physical byte budget");
        assert_eq!(error.into_inner(), "oversized");
        engine
            .try_control(Control::Cancel)
            .expect("cancellation has its own bounded lane");
        assert_eq!(wait_for_event(&mut engine), "cancelled");
    }

    #[test]
    fn work_remains_charged_while_the_network_runtime_owns_it() {
        let received = Arc::new(AtomicBool::new(false));
        let observed = received.clone();
        let mut engine = HostedNetworkEngine::start(
            "rastok-network-owned-work",
            limits(),
            move |mut runtime| async move {
                let work = runtime.recv_work().await.expect("one work item arrives");
                observed.store(true, Ordering::SeqCst);
                assert_eq!(runtime.recv_control().await, Some(Control::Release));
                drop(work);
                runtime.emit("released", 8).await.expect("the permit moved");
            },
        )
        .expect("the ownership pump must start");

        engine.try_submit("held", 8).expect("the first item fits");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !received.load(Ordering::SeqCst) {
            assert!(
                Instant::now() < deadline,
                "the runtime did not receive its work item"
            );
            std::thread::yield_now();
        }
        let error = engine
            .try_submit("blocked", 1)
            .expect_err("in-flight work still owns the shared byte budget");
        assert!(matches!(error, SubmitError::ByteBudget("blocked")));
        engine
            .try_control(Control::Release)
            .expect("the control lane remains available");
        assert_eq!(wait_for_event(&mut engine), "released");
    }

    #[test]
    fn event_memory_remains_owned_until_the_vm_drains_it() {
        let mut engine = HostedNetworkEngine::<(), _, _>::start(
            "rastok-network-events",
            limits(),
            |mut runtime| async move {
                if runtime.recv_control().await == Some(Control::Release) {
                    runtime.emit("first", 8).await.expect("first event fits");
                    runtime
                        .emit("second", 8)
                        .await
                        .expect("draining releases budget");
                }
            },
        )
        .expect("the event pump must start");
        engine
            .try_control(Control::Release)
            .expect("the engine can be released");
        assert_eq!(wait_for_event(&mut engine), "first");
        assert_eq!(wait_for_event(&mut engine), "second");
        let drained = engine.drain_events(1);
        assert!(drained.events.is_empty());
    }

    #[test]
    fn inbound_bytes_are_charged_before_copy_and_until_event_drain() {
        let reserved = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&reserved);
        let mut engine = HostedNetworkEngine::<&'static str, _, _>::start(
            "rastok-network-reserved-intake",
            limits(),
            move |mut runtime| async move {
                let mut events = runtime.event_sender();
                let reservation = events.reserve(8).await.expect("the request fits");
                observed.store(true, Ordering::SeqCst);
                assert_eq!(runtime.recv_control().await, Some(Control::Release));
                events
                    .emit_reserved("first", reservation)
                    .await
                    .expect("the reservation moves into the event");
                let reservation = events
                    .reserve(8)
                    .await
                    .expect("draining the first event releases its bytes");
                events
                    .emit_reserved("second", reservation)
                    .await
                    .expect("the second reserved event fits");
            },
        )
        .expect("the reservation pump starts");

        let deadline = Instant::now() + Duration::from_secs(2);
        while !reserved.load(Ordering::SeqCst) {
            assert!(
                Instant::now() < deadline,
                "the network side did not reserve its intake"
            );
            std::thread::yield_now();
        }
        assert!(matches!(
            engine.try_submit("would-copy", 1),
            Err(SubmitError::ByteBudget("would-copy"))
        ));
        engine
            .try_control(Control::Release)
            .expect("the control lane remains independent");
        assert_eq!(wait_for_event(&mut engine), "first");
        assert_eq!(wait_for_event(&mut engine), "second");
    }

    #[test]
    fn dropping_the_sync_handle_cancels_and_joins_the_network_thread() {
        let started = Arc::new(AtomicBool::new(false));
        let entered = started.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let observed = stopped.clone();
        let engine = HostedNetworkEngine::<(), (), ()>::start(
            "rastok-network-drop",
            limits(),
            move |_| async move {
                let _drop_signal = DropSignal(observed);
                entered.store(true, Ordering::SeqCst);
                std::future::pending::<()>().await;
            },
        )
        .expect("the cancellation pump must start");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !started.load(Ordering::SeqCst) {
            assert!(
                Instant::now() < deadline,
                "the supervised network future did not start"
            );
            std::thread::yield_now();
        }
        drop(engine);
        assert!(stopped.load(Ordering::SeqCst));
    }

    #[test]
    fn a_finished_network_task_is_distinct_from_an_empty_event_queue() {
        let mut engine = HostedNetworkEngine::<(), (), ()>::start(
            "rastok-network-stopped",
            limits(),
            |_| async {},
        )
        .expect("the finite pump must start");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match engine.try_event() {
                EventPoll::Stopped => break,
                EventPoll::Empty => assert!(
                    Instant::now() < deadline,
                    "the finished task did not close its event lane"
                ),
                EventPoll::Event(()) => panic!("the finite task emitted an impossible event"),
            }
            std::thread::yield_now();
        }
        let batch = engine.drain_events(1);
        assert!(batch.events.is_empty());
        assert!(batch.stopped);
    }
}
