//! Hosted control of one execution, its journal and its bound Drivers.
//!
//! The CLI supplies input and logical time; the Lab supplies authored actions.
//! Neither owns a second execution loop. Real Drivers and scripted Drivers
//! retain their established polling order: hosted processing admits a bounded
//! batch before running ready work, while the Lab admits one proposal between
//! batches. Interactive polling settles the preceding writes before offering
//! the next scripted frame. Those orders are observable in the journal.
//!
//! This module adds no thread, clock or scheduler. The semantic transitions
//! remain in `rastok-core`; presentation and recording remain with the caller.

use crate::boundary::{DriverRouteError, IngressAdmission, IngressProposal, IngressRejection};
use crate::connectors::{InboundPollError, RuntimeBindings};
use crate::deployment::BootBindings;
use crate::envelope::Id;
use crate::journal::MemJournal;
use crate::plan::{GraphVersion, SemanticHash, ValidatedExecutionPlan};
use crate::runtime::execution::{Execution, ExecutionFault, PollReport, TimeAdvance};
use std::task::Poll;

/// Failure at a hosted execution boundary. Processing can commit earlier
/// transactions before failing; the caller must inspect the journal rather
/// than infer rollback from this result.
#[derive(Debug)]
pub enum Error {
    /// Preparing or committing ready work failed.
    Step(ExecutionFault),
    /// Polling outbound observations failed.
    Poll(ExecutionFault),
    /// An inbound Driver or its admission failed.
    Ingress(InboundPollError),
    /// Routing an inbound Call response or terminal observation failed.
    Driver(DriverRouteError),
    /// The Lab's authored proposal was refused, not left pending.
    RejectedIngress(IngressRejection),
    /// A scripted source supplied an uncorrelated or duplicate terminal.
    UncorrelatedTerminal,
    /// A scripted source terminal contradicted its resolved Contract.
    TerminalContract(String),
    /// Scripted work did not become idle within the bounded run.
    NotSettled,
}

impl core::fmt::Display for Error {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Step(error) | Self::Poll(error) => error.fmt(formatter),
            Self::Ingress(error) => error.fmt(formatter),
            Self::Driver(error) => error.fmt(formatter),
            Self::RejectedIngress(reason) => write!(
                formatter,
                "the source proposal was refused before admission: {reason:?}"
            ),
            Self::UncorrelatedTerminal => {
                formatter.write_str("the Lab source reported an uncorrelated or duplicate terminal")
            }
            Self::TerminalContract(reason) => write!(
                formatter,
                "the source terminal violates its Contract: {reason}"
            ),
            Self::NotSettled => {
                formatter.write_str("execution did not become idle within 64 Driver polling passes")
            }
        }
    }
}

impl std::error::Error for Error {}

/// The Worker's interface to services supplied by its Host during [`Worker::run`].
/// It does not model the machine or its placement: it supplies logical time,
/// App input, recording checkpoints and waiting policy. The CLI implements
/// these services with OS resources; a test can supply deterministic inputs
/// without a clock or I/O. These services stay outside the execution machinery.
pub trait Host {
    /// Input or presentation failure, kept in the host's own error type.
    type Error;

    /// Current logical time in signed nanoseconds, supplied explicitly.
    fn logical_time(&mut self) -> i64;

    /// Non-blocking App input: `Pending` means listen again later,
    /// `Ready(None)` means this input source has ended, not that the graph
    /// or its Connector sources have closed.
    ///
    /// # Errors
    /// The host could not read its next input.
    fn poll_input(&mut self) -> Result<Poll<Option<IngressProposal>>, Self::Error>;

    /// Reports Driver rejections before the subsequent ready-work pass.
    fn observe_poll(&mut self, _report: &PollReport) {}

    /// Observes a successful processing pass. The host may record the
    /// committed prefix and display new outputs; neither is a transaction.
    ///
    /// # Errors
    /// Recording or another host-side observation failed.
    fn checkpoint(
        &mut self,
        execution: &Execution<'_>,
        journal: &MemJournal,
    ) -> Result<(), Self::Error>;

    /// Waits before polling again when no input or Driver progress occurred.
    /// This does not advance logical time inside the Worker.
    fn wait(&mut self);
}

/// The stage at which continuous host control failed. In particular,
/// `Processing` can leave committed records which the host must still save;
/// a failure never implies rollback of the preceding journal prefix.
#[derive(Debug)]
pub enum RunError<E> {
    /// Applying the host-supplied logical instant failed.
    Advance(ExecutionFault),
    /// Reading an App input failed before admission.
    Input(E),
    /// An App proposal could not be admitted.
    Admission(ExecutionFault),
    /// Driver polling or execution failed after earlier work may have committed.
    Processing(Error),
    /// Saving or displaying the committed prefix failed.
    Checkpoint(E),
}

impl<E: core::fmt::Display> core::fmt::Display for RunError<E> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Advance(error) => error.fmt(formatter),
            Self::Input(error) | Self::Checkpoint(error) => error.fmt(formatter),
            Self::Admission(error) => write!(formatter, "the submission was refused: {error}"),
            Self::Processing(error) => error.fmt(formatter),
        }
    }
}

impl<E: core::fmt::Debug + core::fmt::Display> std::error::Error for RunError<E> {}

/// Distinguishes the hosted polling/wait policy from a change in available
/// work. A pending ingress counts as activity for the existing host loop,
/// but must not keep `run_until_idle` busy. Rejected observations, conversely,
/// consume Driver work without necessarily appending a journal record.
#[derive(Default)]
struct PollProgress {
    host_activity: bool,
    work_changed: bool,
}

/// One single-threaded hosted execution. Driver binding is completed by the
/// caller before construction; the Worker then creates the journal and live
/// state with the same deployment attestation and logical starting instant.
#[derive(Debug)]
pub struct Worker<'plan> {
    execution: Execution<'plan>,
    journal: MemJournal,
    bindings: RuntimeBindings<'plan>,
}

impl<'plan> Worker<'plan> {
    /// Creates the execution and its journal after Driver binding has succeeded.
    /// `hash` is required for Retry, as enforced by execution admission. The
    /// Lab's plan-only API can omit it for plans without Retry.
    #[must_use]
    pub fn new(
        plan: &'plan ValidatedExecutionPlan,
        hash: Option<SemanticHash>,
        version: GraphVersion,
        logical_time: i64,
        boot: Option<&BootBindings>,
        bindings: RuntimeBindings<'plan>,
    ) -> Self {
        let journal = boot.map_or_else(
            || MemJournal::new(version),
            |boot| MemJournal::new_with_deployment(version, boot.attestation().clone()),
        );
        let execution = hash.map_or_else(
            || {
                boot.map_or_else(
                    || Execution::new_at(plan, version, logical_time),
                    |boot| Execution::new_at_with_deployment(plan, version, logical_time, boot),
                )
            },
            |hash| Execution::new_program(plan, hash, version, logical_time, boot),
        );
        Self {
            execution,
            journal,
            bindings,
        }
    }

    /// Read-only live execution, including its detached inspection views.
    #[must_use]
    pub const fn execution(&self) -> &Execution<'plan> {
        &self.execution
    }

    /// The authoritative committed prefix, never a debugger-owned copy.
    #[must_use]
    pub const fn journal(&self) -> &MemJournal {
        &self.journal
    }

    /// Whether source Drivers require continued polling after App input ends.
    #[must_use]
    pub fn has_runnable_inbound(&self) -> bool {
        self.bindings.has_runnable_inbound()
    }

    /// Admits one App input using this Worker's journal.
    ///
    /// # Errors
    /// The same pre-commit refusals as [`Execution::accept_input`].
    pub fn accept_input(&mut self, proposal: IngressProposal) -> Result<Id, ExecutionFault> {
        self.execution.accept_input(&mut self.journal, proposal)
    }

    /// Applies an explicit logical time advance, including due work.
    ///
    /// # Errors
    /// The same refusals as [`Execution::advance_to`]; earlier work may commit.
    pub fn advance_to(&mut self, target: i64) -> Result<(), ExecutionFault> {
        self.execution
            .advance_to(&mut self.journal, &mut self.bindings, target)
    }

    /// Advances one transition of the Lab's suspended logical-time action.
    pub(crate) fn advance_once(
        &mut self,
        advance: &mut TimeAdvance,
    ) -> Result<bool, ExecutionFault> {
        advance.advance_once(&mut self.execution, &mut self.journal, &mut self.bindings)
    }

    /// Performs at most one ready execution step, without polling the world.
    /// The caller can inspect the resulting prefix before continuing.
    ///
    /// # Errors
    /// The same transaction refusals as [`Execution::step_with_bindings`].
    pub fn step(&mut self) -> Result<bool, ExecutionFault> {
        self.execution
            .step_with_bindings(&mut self.journal, &mut self.bindings)
    }

    /// Drives the hosted execution continuously. Unlike [`Self::run_until_idle`],
    /// this keeps listening when App input is pending or a source Driver is
    /// bound, and waits for open Calls and Sessions after App input ends.
    /// The host supplies time, input and the waiting policy; no thread is added.
    ///
    /// Returns after App input ends, no source Driver needs polling and no
    /// Call or Session remains open, following the CLI's completion rule.
    /// Returning does not close instances or release Drivers; [`Self::shutdown`]
    /// releases host resources explicitly.
    ///
    /// # Errors
    /// Input, admission, time advance, execution or a host checkpoint failed.
    /// Earlier records remain committed and accessible through [`Self::journal`].
    pub fn run<H: Host>(&mut self, host: &mut H) -> Result<(), RunError<H::Error>> {
        let polls_inbound = self.has_runnable_inbound();
        let mut input_open = true;
        loop {
            let mut progressed = false;
            let target = host.logical_time();
            if target > self.execution.logical_time() {
                self.advance_to(target).map_err(RunError::Advance)?;
            }
            while input_open {
                match host.poll_input().map_err(RunError::Input)? {
                    Poll::Ready(Some(proposal)) => {
                        self.accept_input(proposal).map_err(RunError::Admission)?;
                        progressed = true;
                    }
                    Poll::Ready(None) => input_open = false,
                    Poll::Pending => break,
                }
            }
            progressed |= self
                .process_ready(|report| host.observe_poll(report))
                .map_err(RunError::Processing)?
                .host_activity;
            host.checkpoint(&self.execution, &self.journal)
                .map_err(RunError::Checkpoint)?;
            if !input_open && !polls_inbound && !self.execution.has_open_obligations() {
                return Ok(());
            }
            if !progressed {
                host.wait();
            }
        }
    }

    /// Runs ready work and polls Drivers without waiting or advancing time.
    /// Returns when a complete pass makes no progress and leaves no ready
    /// execution work. A Driver's `Pending` proposal is not progress: its
    /// transport remains with that Driver until another call can admit it.
    ///
    /// Idle is not complete or stable: Calls, Sessions and future deadlines
    /// can remain open. The caller resumes after input, a Driver observation
    /// or an explicit time advance. An uninterrupted source need not go idle.
    ///
    /// This public control serves embedders with their own event loop, instead
    /// of delegating input, time and waiting to [`Self::run`] and a [`Host`].
    ///
    /// # Errors
    /// Admission, execution or Driver routing failed; earlier records stand.
    pub fn run_until_idle(&mut self, mut observe: impl FnMut(&PollReport)) -> Result<(), Error> {
        loop {
            let before = self.journal.records().len();
            let progress = self.process_ready(&mut observe)?;
            if !progress.work_changed
                && self.journal.records().len() == before
                && !self.execution.has_ready_work()
            {
                return Ok(());
            }
        }
    }

    /// Performs one bounded hosted polling pass. Report rejected observations
    /// before the final ready-work pass, even if that pass subsequently faults.
    ///
    /// # Errors
    /// Admission, execution or Driver routing failed; earlier records stand.
    fn process_ready(
        &mut self,
        mut observe: impl FnMut(&PollReport),
    ) -> Result<PollProgress, Error> {
        let mut progress = PollProgress::default();
        for _ in 0..64 {
            match self
                .bindings
                .poll_inbound(&mut self.execution, &mut self.journal)
                .map_err(Error::Ingress)?
            {
                Some(admission) => {
                    progress.host_activity = true;
                    progress.work_changed |= !matches!(admission, IngressAdmission::Pending);
                }
                None => break,
            }
        }
        self.execution
            .run_ready_with_bindings(&mut self.journal, &mut self.bindings)
            .map_err(Error::Step)?;
        let responses = self
            .bindings
            .poll_inbound_call_responses(&mut self.execution)
            .map_err(Error::Driver)?;
        let timeouts = self
            .bindings
            .poll_inbound_call_timeouts(&mut self.execution)
            .map_err(Error::Driver)?;
        let inbound_events = self
            .bindings
            .poll_inbound_call_events(&mut self.execution)
            .map_err(Error::Driver)?;
        let outbound = self
            .execution
            .poll_drivers(&mut self.journal, &mut self.bindings)
            .map_err(Error::Poll)?;
        observe(&outbound);
        let settled = responses.started > 0
            || timeouts.started > 0
            || inbound_events.queued > 0
            || outbound.settled > 0;
        progress.host_activity |= settled;
        progress.work_changed |= settled || !outbound.rejected.is_empty();
        self.execution
            .run_ready_with_bindings(&mut self.journal, &mut self.bindings)
            .map_err(Error::Step)?;
        Ok(progress)
    }

    /// Polls at a safe pause without starting a graph transaction. Complete
    /// the preceding scripted batch before admitting the next source frame,
    /// so WriteSettled stays before that frame in both batch and step modes.
    pub(crate) fn poll(&mut self, mut observe: impl FnMut(&PollReport)) -> Result<bool, Error> {
        let report = self
            .execution
            .poll_drivers(&mut self.journal, &mut self.bindings)
            .map_err(Error::Poll)?;
        let inbound = self.poll_inbound()?;
        observe(&report);
        Ok(inbound || report.settled > 0 || !report.rejected.is_empty())
    }

    /// Settles one authored action. The final ready-work pass still runs
    /// after the polling limit, so both exhaustion and a final-step fault can
    /// be reported, in that order. The error list preserves both faults.
    pub(crate) fn settle_action(
        &mut self,
        mut observe: impl FnMut(&PollReport),
    ) -> Result<(), Vec<Error>> {
        let mut rested = false;
        for _ in 0..64 {
            let inbound = match self.poll_inbound() {
                Ok(progressed) => progressed,
                Err(error) => return Err(vec![error]),
            };
            if let Err(error) = self
                .execution
                .run_ready_with_bindings(&mut self.journal, &mut self.bindings)
            {
                return Err(vec![Error::Step(error)]);
            }
            let report = match self
                .execution
                .poll_drivers(&mut self.journal, &mut self.bindings)
            {
                Ok(report) => report,
                Err(error) => return Err(vec![Error::Poll(error)]),
            };
            observe(&report);
            if report.settled == 0 && !inbound {
                rested = true;
                break;
            }
        }
        let mut errors = Vec::new();
        if !rested {
            errors.push(Error::NotSettled);
        }
        if let Err(error) = self
            .execution
            .run_ready_with_bindings(&mut self.journal, &mut self.bindings)
        {
            errors.push(Error::Step(error));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Offers one authored source proposal and its terminal observations.
    /// A pending proposal remains with the Driver under mailbox pressure.
    pub(crate) fn poll_inbound(&mut self) -> Result<bool, Error> {
        let admitted = match self
            .bindings
            .poll_inbound(&mut self.execution, &mut self.journal)
            .map_err(Error::Ingress)?
        {
            Some(IngressAdmission::Accepted(_)) => true,
            Some(IngressAdmission::Rejected(reason)) => return Err(Error::RejectedIngress(reason)),
            Some(IngressAdmission::Pending) | None => false,
        };
        let report = self
            .bindings
            .poll_inbound_call_events(&mut self.execution)
            .map_err(Error::Driver)?;
        if !report.uncorrelated.is_empty() || !report.discarded.is_empty() {
            return Err(Error::UncorrelatedTerminal);
        }
        if let Some((_, reason)) = report.rejected.first() {
            return Err(Error::TerminalContract(reason.clone()));
        }
        Ok(admitted || report.queued > 0)
    }

    /// Ends host control and releases the physical Drivers, returning the
    /// journal for reporting or recording. This does not synthesize Closing,
    /// cancellation or terminal records: graph-level closure is explicit.
    #[must_use]
    pub fn shutdown(self) -> MemJournal {
        drop(self.bindings);
        self.journal
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::{EnvelopeDraft, IngressProvenance, IngressSequence};
    use crate::connectors::DriverRegistry;
    use crate::envelope::{Data, Namespace, Value};
    use crate::node;
    use std::collections::VecDeque;

    #[derive(Default)]
    struct TestHost {
        now: i64,
        inputs: VecDeque<Poll<Option<IngressProposal>>>,
        checkpoints: Vec<usize>,
        waits: usize,
        fail_checkpoint: bool,
    }

    impl Host for TestHost {
        type Error = &'static str;

        fn logical_time(&mut self) -> i64 {
            self.now
        }

        fn poll_input(&mut self) -> Result<Poll<Option<IngressProposal>>, Self::Error> {
            Ok(self.inputs.pop_front().unwrap_or(Poll::Ready(None)))
        }

        fn checkpoint(
            &mut self,
            _: &Execution<'_>,
            journal: &MemJournal,
        ) -> Result<(), Self::Error> {
            if self.fail_checkpoint {
                return Err("recording failed");
            }
            self.checkpoints.push(journal.records().len());
            Ok(())
        }

        fn wait(&mut self) {
            self.waits += 1;
            assert!(self.waits <= 3, "the test must not wait indefinitely");
            self.now += 1_000_000_000;
        }
    }

    #[test]
    fn continuous_idle_scripted_and_step_controls_keep_the_same_journal() {
        let compilation = crate::compiler::compile(
            &[r#"graph app : App {
              input in: int
              output out: int
              operator count {
                input in: int
                output out: int
                var total: int = 0
                on in { total = total + data ; out = total }
              }
              in -> count -> out
            }"#],
            &crate::contract::ContractCatalog::try_m0().unwrap(),
        )
        .unwrap();
        let plan = &compilation.plan;
        let version = GraphVersion(1);
        let bindings = || RuntimeBindings::try_new(plan, &DriverRegistry::default()).unwrap();
        let new_worker = || Worker::new(plan, Some(compilation.hash), version, 0, None, bindings());
        let mut hosted = new_worker();
        let mut continuous = new_worker();
        let mut idle = new_worker();
        let mut scripted = new_worker();
        let mut stepped = new_worker();
        let mut reference = Execution::new_program(plan, compilation.hash, version, 0, None);
        let mut journal = MemJournal::new(version);
        for value in [1, 2, 3] {
            let proposal = IngressProposal {
                provenance: IngressProvenance::single(node::Path::new(["lab"]).unwrap()),
                draft: EnvelopeDraft {
                    port: node::Port::new("in").unwrap(),
                    data: Data::materialized(Value::Int(value)),
                    ext: Namespace::new(),
                },
                sequence: IngressSequence::Next,
                session: None,
            };
            let id = reference
                .accept_input(&mut journal, proposal.clone())
                .unwrap();
            assert_eq!(hosted.accept_input(proposal.clone()).unwrap(), id);
            assert_eq!(scripted.accept_input(proposal.clone()).unwrap(), id);
            assert_eq!(stepped.accept_input(proposal.clone()).unwrap(), id);
            assert_eq!(idle.accept_input(proposal.clone()).unwrap(), id);
            let mut host = TestHost {
                inputs: VecDeque::from([Poll::Ready(Some(proposal))]),
                ..TestHost::default()
            };
            continuous.run(&mut host).unwrap();
            assert_eq!(host.waits, 0);
            assert_eq!(host.checkpoints.len(), 1);
            idle.run_until_idle(|_| {}).unwrap();
            reference.run_ready(&mut journal).unwrap();
            hosted
                .process_ready(|report| assert_eq!(report, &PollReport::default()))
                .unwrap();
            assert!(
                scripted
                    .settle_action(|report| assert_eq!(report, &PollReport::default()))
                    .is_ok()
            );
            while stepped.step().unwrap() {}
            for worker in [&hosted, &continuous, &idle, &scripted, &stepped] {
                assert_eq!(worker.journal().records(), journal.records());
                assert_eq!(worker.execution().outputs(), reference.outputs());
                assert_eq!(worker.execution().inspection(), reference.inspection());
            }
        }
    }

    /// A bound Process that starts a Call but never provides a reply. The
    /// execution alone decides when its explicit timeout becomes due.
    struct SilentProcess(crate::connector::ContractId);

    impl crate::connectors::ConnectorDriverFactory for SilentProcess {
        fn connector_id(&self) -> &crate::connector::ContractId {
            &self.0
        }

        fn roles(&self) -> crate::connector::ConnectorDirection {
            crate::connector::ConnectorDirection::Outbound
        }

        fn bind<'plan>(
            &self,
            _: &'plan ValidatedExecutionPlan,
            _: &[node::Path],
        ) -> Result<
            Box<dyn crate::connectors::BoundConnectorDriver + 'plan>,
            crate::connectors::DriverFactoryError,
        > {
            Ok(Box::new(Self(self.0.clone())))
        }
    }

    impl crate::connectors::BoundConnectorDriver for SilentProcess {
        fn inbound(&mut self) -> Option<&mut dyn crate::boundary::InboundConnectorDriver> {
            None
        }

        fn outbound(&mut self) -> Option<&mut dyn crate::boundary::OutboundConnectorDriver> {
            Some(self)
        }
    }

    impl crate::boundary::CallConnectorDriver for SilentProcess {
        fn prepare_request(
            &mut self,
            _: &node::Path,
            data: &Data,
            _: &std::collections::BTreeMap<String, Value>,
        ) -> crate::boundary::RequestPreparation {
            crate::boundary::prepare_process_request(crate::boundary::RequestCodec::Lines, data)
        }

        fn scheduler_completion(
            &mut self,
            _: &node::Path,
            completion: crate::boundary::SchedulerCompletion,
        ) -> crate::boundary::Completion {
            crate::boundary::process_scheduler_completion(completion)
        }

        fn start(
            &mut self,
            _: &crate::boundary::CallOpen,
            _: &crate::envelope::Envelope,
        ) -> crate::boundary::CallStart {
            crate::boundary::CallStart::Started
        }

        fn events(&mut self) -> Vec<crate::boundary::Response> {
            Vec::new()
        }
    }

    #[test]
    fn idle_returns_with_an_open_call_but_run_waits_after_input_ends() {
        let compilation = crate::compiler::compile(
            &[r#"graph app : App {
              input in: text
              output exit: variant
              connector call : Process {
                command = "cat"; codec = lines; behavior = deterministic
                timeout = duration("1s")
              }
              in -> call
              call.out -> nothing; call.err -> nothing; call.exit -> exit
            }"#],
            &crate::contract::ContractCatalog::try_m0().unwrap(),
        )
        .unwrap();
        let registry = DriverRegistry::default()
            .try_with(Box::new(SilentProcess(
                crate::connector::ContractId::parse(crate::connector::PROCESS_CONNECTOR_ID)
                    .unwrap(),
            )))
            .unwrap();
        let bindings = RuntimeBindings::try_new(&compilation.plan, &registry).unwrap();
        let mut worker = Worker::new(
            &compilation.plan,
            Some(compilation.hash),
            GraphVersion(1),
            0,
            None,
            bindings,
        );
        worker
            .accept_input(IngressProposal {
                provenance: IngressProvenance::single(node::Path::new(["lab"]).unwrap()),
                draft: EnvelopeDraft {
                    port: node::Port::new("in").unwrap(),
                    data: Data::materialized(Value::Text("request".into())),
                    ext: Namespace::new(),
                },
                sequence: IngressSequence::Next,
                session: None,
            })
            .unwrap();
        worker.run_until_idle(|_| {}).unwrap();
        assert!(worker.execution().has_open_obligations());
        assert!(!worker.execution().has_ready_work());
        assert!(!worker.execution().is_stable());
        assert_eq!(worker.execution().logical_time(), 0);
        let records = worker.journal().records().to_vec();
        worker.run_until_idle(|_| {}).unwrap();
        assert_eq!(worker.journal().records(), records);
        assert!(worker.execution().outputs().is_empty());

        let mut host = TestHost::default();
        worker.run(&mut host).unwrap();
        assert_eq!(host.waits, 1, "EOF must not abandon an open Call");
        assert_eq!(host.checkpoints.len(), 2);
        assert_eq!(worker.execution().logical_time(), 1_000_000_000);
        assert!(!worker.execution().has_open_obligations());
        assert_eq!(worker.execution().outputs().len(), 1);
        assert_eq!(
            worker.execution().outputs()[0].envelope.data(),
            &Data::materialized(crate::boundary::ProcessExit::TimeoutExpired.into_value())
        );
    }

    #[test]
    fn pending_host_input_is_not_end_of_input() {
        let compilation = crate::compiler::compile(
            &["graph app : App { input in: int; output out: int; in -> out }"],
            &crate::contract::ContractCatalog::try_m0().unwrap(),
        )
        .unwrap();
        let bindings =
            RuntimeBindings::try_new(&compilation.plan, &DriverRegistry::default()).unwrap();
        let mut worker = Worker::new(
            &compilation.plan,
            Some(compilation.hash),
            GraphVersion(1),
            0,
            None,
            bindings,
        );
        let mut host = TestHost {
            inputs: VecDeque::from([
                Poll::Pending,
                Poll::Ready(Some(IngressProposal {
                    provenance: IngressProvenance::single(node::Path::new(["lab"]).unwrap()),
                    draft: EnvelopeDraft {
                        port: node::Port::new("in").unwrap(),
                        data: Data::materialized(Value::Int(42)),
                        ext: Namespace::new(),
                    },
                    sequence: IngressSequence::Next,
                    session: None,
                })),
            ]),
            ..TestHost::default()
        };
        worker.run(&mut host).unwrap();
        assert_eq!(host.waits, 1);
        assert_eq!(host.checkpoints.len(), 2);
        assert_eq!(worker.execution().logical_time(), 1_000_000_000);
        assert_eq!(worker.execution().outputs().len(), 1);
        assert_eq!(
            worker.execution().outputs()[0].envelope.data(),
            &Data::materialized(Value::Int(42))
        );
    }

    #[test]
    fn run_faults_preserve_the_committed_prefix_and_the_failure_stage() {
        let compilation = crate::compiler::compile(
            &[r#"graph app : App {
              input in: int
              output out: int
              operator divide {
                input in: int; output out: int
                on in { out = 1 / data }
              }
              in -> divide -> out
            }"#],
            &crate::contract::ContractCatalog::try_m0().unwrap(),
        )
        .unwrap();
        for (input, recording_fails) in [(0, false), (1, true)] {
            let bindings =
                RuntimeBindings::try_new(&compilation.plan, &DriverRegistry::default()).unwrap();
            let mut worker = Worker::new(
                &compilation.plan,
                Some(compilation.hash),
                GraphVersion(1),
                0,
                None,
                bindings,
            );
            let mut host = TestHost {
                inputs: VecDeque::from([Poll::Ready(Some(IngressProposal {
                    provenance: IngressProvenance::single(node::Path::new(["lab"]).unwrap()),
                    draft: EnvelopeDraft {
                        port: node::Port::new("in").unwrap(),
                        data: Data::materialized(Value::Int(input)),
                        ext: Namespace::new(),
                    },
                    sequence: IngressSequence::Next,
                    session: None,
                }))]),
                fail_checkpoint: recording_fails,
                ..TestHost::default()
            };
            let error = worker.run(&mut host).unwrap_err();
            if recording_fails {
                assert!(matches!(error, RunError::Checkpoint("recording failed")));
                assert_eq!(worker.execution().outputs().len(), 1);
            } else {
                assert!(matches!(error, RunError::Processing(Error::Step(_))));
                assert!(worker.execution().outputs().is_empty());
                assert!(worker.execution().has_ready_work());
            }
            assert!(host.checkpoints.is_empty());
            assert_eq!(host.waits, 0);
            assert!(
                worker
                    .journal()
                    .records()
                    .iter()
                    .any(|record| matches!(record, crate::journal::Record::Ingress(_))),
                "input admission remains committed even when later processing or recording fails"
            );
        }
    }

    #[test]
    fn a_fault_keeps_its_input_and_shutdown_adds_no_closure() {
        let compilation = crate::compiler::compile(
            &[r#"graph app : App {
              input in: int
              output out: int
              operator divide {
                input in: int
                output out: int
                on in { out = 1 / data }
              }
              in -> divide -> out
            }"#],
            &crate::contract::ContractCatalog::try_m0().unwrap(),
        )
        .unwrap();
        let bindings =
            RuntimeBindings::try_new(&compilation.plan, &DriverRegistry::default()).unwrap();
        let mut worker = Worker::new(
            &compilation.plan,
            Some(compilation.hash),
            GraphVersion(1),
            0,
            None,
            bindings,
        );
        worker.process_ready(|_| {}).unwrap();
        worker
            .accept_input(IngressProposal {
                provenance: IngressProvenance::single(node::Path::new(["lab"]).unwrap()),
                draft: EnvelopeDraft {
                    port: node::Port::new("in").unwrap(),
                    data: Data::materialized(Value::Int(0)),
                    ext: Namespace::new(),
                },
                sequence: IngressSequence::Next,
                session: None,
            })
            .unwrap();
        // Admission also creates lifecycle work. Advance it normally before
        // the faulting division, then compare that exact pre-commit prefix.
        for _ in 0..8 {
            let records = worker.journal().records().to_vec();
            let inspection = worker.execution().inspection();
            match worker.step() {
                Ok(true) => continue,
                Ok(false) => panic!("the admitted division must run"),
                Err(error) => assert!(matches!(error, ExecutionFault::Fault(_)), "{error}"),
            }
            assert_eq!(worker.journal().records(), records);
            assert_eq!(worker.execution().inspection(), inspection);
            assert!(worker.execution().has_ready_work());
            let journal = worker.shutdown();
            assert_eq!(
                journal.records(),
                records,
                "host shutdown is not instance.stop"
            );
            return;
        }
        panic!("the division must fault within its lifecycle and transaction steps");
    }
}
