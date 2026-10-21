//! The scenario runner — the graph's possible worlds, executed.
//!
//! A scenario is the world's half of a journal, written in advance:
//! injections play the ingress, mocks play the outbound drivers, `advance`
//! plays time. Everything runs on the same [`Execution`] a production run
//! uses — the mock replaces the driver, never the bookkeeping — so
//! a green scenario is a statement about the machine, not about a
//! test double.

use crate::compiler::hir::{
    Cmp, EmitValue, ExpectTarget, Expectation, InjectionTarget, Lab, Mock, Quantifier, Scenario,
    ScenarioStep,
};
use crate::connector::ConnectorDirection;
use crate::connector::ContractId;
use crate::connector::driver::Driver;
use crate::connector::driver::Outbound;
use crate::connector::driver::OutboundCall;
#[cfg(test)]
use crate::connector::drivers::ProcessRequestSettings;
use crate::connector::drivers::{
    CallProtocol, DriverFactory, DriverFactoryError, DriverRegistry, HttpServerLabInput,
    RuntimeDrivers, ScriptedProtocolBinder, inbound_lab_factories, scripted_protocol_binders,
};
use crate::connector::exchange::CallOpen;
use crate::connector::exchange::CallResponse;
use crate::connector::exchange::CallStart;
use crate::connector::exchange::Completion;
use crate::connector::exchange::EnvelopeDraft;
use crate::connector::exchange::IngressProposal;
use crate::connector::exchange::IngressProvenance;
use crate::connector::exchange::IngressSequence;
use crate::connector::exchange::RequestPreparation;
use crate::connector::exchange::SchedulerCompletion;
use crate::envelope::{Data, Envelope, Namespace, Value, WriteTarget};
use crate::journal::{MemJournal, Record};
use crate::plan::{GraphVersion, SemanticHash, ValidatedExecutionPlan};
use crate::runtime::execution::inspection::EnvelopeView;
use crate::runtime::execution::{Execution, TimeAdvance};
use crate::worker::Worker;
use crate::{expr, node, runtime};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

mod process_service;

type MockFaults = Rc<RefCell<Vec<String>>>;
type SharedMocks = Rc<BTreeMap<node::Path, Rc<Mock>>>;

/// The verdict of one scenario: green when `failures` is empty.
#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioReport {
    /// The owning simulation.
    pub simulation: String,
    /// The scenario name.
    pub scenario: String,
    /// The human label, when written.
    pub label: Option<String>,
    /// Every failed expectation, uncovered call, and fault — in
    /// occurrence order. Empty means green.
    pub failures: Vec<String>,
}

impl ScenarioReport {
    /// Whether the scenario held.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

/// One executed scenario: its verdict and the journal it produced —
/// the trace half is what the golden workflow projects and compares.
#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioRun {
    /// The verdict.
    pub report: ScenarioReport,
    /// The causal records of the run, in commit order.
    pub records: Vec<Record>,
}

/// Authored action kind exposed separately from Execution transactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioActionKind {
    /// One root envelope is admitted.
    Inject,
    /// Virtual time advances to the authored target.
    Advance,
    /// One expectation is evaluated without becoming an Execution transaction.
    Expect,
}

/// Read-only evidence for the most recently evaluated assertion.
#[derive(Debug)]
pub struct ExpectationInspection<'a> {
    /// Checked assertion; its source lives in the compiler side table.
    pub expectation: &'a Expectation,
    /// Exact envelopes in this assertion's target window.
    pub envelopes: Vec<&'a Envelope>,
    /// Verdict re-derived by the same pure assertion evaluator as the Lab.
    pub failure: Option<String>,
}

/// Failure while interactively advancing one scenario transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScenarioStepError {
    /// The Execution transaction refused before commit and remains reserved.
    Transaction(String),
    /// The scripted Driver boundary failed; an earlier transaction may have
    /// committed and must not be replayed as though it had not.
    Boundary(String),
}

/// World-authored input which has not been admitted yet. The debugger presents
/// it read-only; admission alone constructs the immutable envelope.
#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioInjectionDraft {
    /// Source Connector, absent for a public App input.
    pub connector: Option<node::Path>,
    /// Source output or App input selected by the authored scenario action.
    pub port: node::Port,
    /// Effective logical type which the edited value must inhabit.
    pub ty: crate::datatype::DataType,
    /// Exact annotation table used for policy-safe presentation.
    pub annotations: crate::schema_annotation::AnnotationTable,
    /// Materialized DATA waiting outside the graph.
    pub data: Data,
    /// World-owned `meta.ext` fields waiting outside the graph.
    pub ext: Namespace,
    /// Whether DATA differs from the authored scenario value.
    pub edited: bool,
}

impl ScenarioStepError {
    /// Whether retrying the same reserved Execution transaction is safe.
    #[must_use]
    pub const fn retryable(&self) -> bool {
        matches!(self, Self::Transaction(_))
    }
}

impl core::fmt::Display for ScenarioStepError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Transaction(message) | Self::Boundary(message) => formatter.write_str(message),
        }
    }
}

/// Executes one Lab scenario through a Worker with scripted Drivers. The
/// Simulator owns authored actions, assertion windows and failures; the Worker
/// owns execution, journal and Driver polling. Batch and interactive controls
/// use the same scenario and execution state.
#[derive(Debug)]
pub struct Simulator<'plan, 'scenario> {
    plan: &'plan ValidatedExecutionPlan,
    simulation: String,
    scenario: &'scenario Scenario,
    worker: Worker<'plan>,
    source_input: Rc<RefCell<HttpServerLabInput>>,
    faults: MockFaults,
    failures: Vec<String>,
    window_start: usize,
    output_window_start: usize,
    record_window_start: usize,
    next_action: usize,
    advance: Option<TimeAdvance>,
    quiescent: bool,
    last_action: Option<ScenarioActionKind>,
    next_injection_override: Option<EnvelopeDraft>,
}

impl<'plan, 'scenario> Simulator<'plan, 'scenario> {
    /// Creates a fresh Execution, journal, virtual clock and scripted Driver world.
    ///
    /// # Errors
    ///
    /// Driver composition is proven before the session can accept an action.
    pub fn try_new(
        plan: &'plan ValidatedExecutionPlan,
        graph_version: GraphVersion,
        simulation: &str,
        shared_mocks: &[Mock],
        scenario: &'scenario Scenario,
        boot: Option<&crate::deployment::BootBindings>,
    ) -> Result<Self, String> {
        Self::try_new_inner(
            plan,
            None,
            graph_version,
            simulation,
            shared_mocks,
            scenario,
            boot,
        )
    }

    /// Creates a scenario over an exact validated program. Retry retains this
    /// original identity beside its prepared operation, just as real execution does.
    ///
    /// # Errors
    /// Invalid Driver composition or missing original program authority.
    pub fn try_new_program(
        plan: &'plan ValidatedExecutionPlan,
        hash: SemanticHash,
        graph_version: GraphVersion,
        simulation: &str,
        shared_mocks: &[Mock],
        scenario: &'scenario Scenario,
        boot: Option<&crate::deployment::BootBindings>,
    ) -> Result<Self, String> {
        Self::try_new_inner(
            plan,
            Some(hash),
            graph_version,
            simulation,
            shared_mocks,
            scenario,
            boot,
        )
    }

    fn try_new_inner(
        plan: &'plan ValidatedExecutionPlan,
        hash: Option<SemanticHash>,
        graph_version: GraphVersion,
        simulation: &str,
        shared_mocks: &[Mock],
        scenario: &'scenario Scenario,
        boot: Option<&crate::deployment::BootBindings>,
    ) -> Result<Self, String> {
        if hash.is_none() && plan.nodes.iter().any(|owner| owner.retry_spec().is_some()) {
            return Err(
                "Retry requires the original program identity; use try_new_program or run_program"
                    .into(),
            );
        }
        // Nearest wins, whole-mock replacement: a scenario mock for a
        // connector shadows the simulation-level one entirely.
        let mut mocks: BTreeMap<node::Path, Mock> = BTreeMap::new();
        for mock in shared_mocks {
            mocks.insert(mock.connector.clone(), mock.clone());
        }
        for mock in &scenario.mocks {
            mocks.insert(mock.connector.clone(), mock.clone());
        }
        for step in &scenario.steps {
            if let ScenarioStep::Inject { target, .. } = step {
                injection_port(plan, target)?;
            }
        }
        let source_input = Rc::new(RefCell::new(HttpServerLabInput::default()));
        let empty_boot = crate::deployment::BootBindings::empty();
        let (driver, faults) = mock_bindings(
            plan,
            mocks,
            boot.unwrap_or(&empty_boot),
            Rc::clone(&source_input),
        )?;
        let worker = Worker::new(plan, hash, graph_version, 0, boot, driver);
        Ok(Self {
            plan,
            simulation: String::from(simulation),
            scenario,
            worker,
            source_input,
            faults,
            failures: Vec::new(),
            window_start: 0,
            output_window_start: 0,
            record_window_start: 0,
            next_action: 0,
            advance: None,
            quiescent: true,
            last_action: None,
            next_injection_override: None,
        })
    }

    /// Owning simulation name.
    #[must_use]
    pub fn simulation(&self) -> &str {
        &self.simulation
    }

    /// Scenario source name.
    #[must_use]
    pub fn scenario(&self) -> &str {
        &self.scenario.name
    }

    /// Optional human label.
    #[must_use]
    pub fn label(&self) -> Option<&str> {
        self.scenario.label.as_deref()
    }

    /// Index of the next authored action.
    #[must_use]
    pub const fn next_action_index(&self) -> usize {
        self.next_action
    }

    /// Number of authored actions.
    #[must_use]
    pub fn action_count(&self) -> usize {
        self.scenario.steps.len()
    }

    /// Next authored action without applying it.
    #[must_use]
    pub fn next_action_kind(&self) -> Option<ScenarioActionKind> {
        self.scenario
            .steps
            .get(self.next_action)
            .map(|step| match step {
                ScenarioStep::Inject { .. } => ScenarioActionKind::Inject,
                ScenarioStep::Advance { .. } => ScenarioActionKind::Advance,
                ScenarioStep::Expect(_) => ScenarioActionKind::Expect,
            })
    }

    /// Most recently applied authored action.
    #[must_use]
    pub const fn last_action_kind(&self) -> Option<ScenarioActionKind> {
        self.last_action
    }

    /// Returns the last assertion and its existing observation window without
    /// advancing the scenario or changing any delivery or journal record.
    #[must_use]
    pub fn last_expectation(&self) -> Option<ExpectationInspection<'_>> {
        let ScenarioStep::Expect(expectation) =
            self.scenario.steps.get(self.next_action.checked_sub(1)?)?
        else {
            return None;
        };
        let window = &self.worker.execution().envelope_views()[self.window_start..];
        let outputs = &self.worker.execution().outputs()[self.output_window_start..];
        let envelopes = expectation_envelopes(
            expectation,
            window,
            outputs,
            &self.worker.journal().records()[self.record_window_start..],
        );
        let failure = check_expectation(self.plan, expectation, &envelopes);
        Some(ExpectationInspection {
            expectation,
            envelopes,
            failure,
        })
    }

    /// Whether ready Execution work from the current action has reached a boundary.
    #[must_use]
    pub const fn is_quiescent(&self) -> bool {
        self.quiescent
    }

    /// Whether every authored action has been applied and ready work settled.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.next_action == self.scenario.steps.len()
            && self.quiescent
            && !self.source_input.borrow().is_pending()
    }

    /// Failures accumulated in occurrence order.
    #[must_use]
    pub fn failures(&self) -> &[String] {
        &self.failures
    }

    /// Returns the next authored injection as a pre-admission draft for
    /// inspection. Other action kinds expose no draft.
    ///
    /// # Errors
    ///
    /// An invalid authored injection or a missing resolved ingress refuses
    /// the projection instead of inventing a type in the browser.
    pub fn next_injection_draft(&self) -> Result<Option<ScenarioInjectionDraft>, String> {
        if !self.quiescent {
            return Ok(None);
        }
        let Some(ScenarioStep::Inject { target, value, .. }) =
            self.scenario.steps.get(self.next_action)
        else {
            return Ok(None);
        };
        let resolved = injection_port(self.plan, target)?;
        let (draft, edited) = self.next_injection_override.as_ref().map_or_else(
            || draft(target.port(), value).map(|draft| (draft, false)),
            |draft| Ok((draft.clone(), true)),
        )?;
        Ok(Some(ScenarioInjectionDraft {
            connector: target.connector().cloned(),
            port: target.port().clone(),
            ty: resolved.ty().clone(),
            annotations: resolved.annotations().clone(),
            data: draft.data,
            ext: draft.ext,
            edited,
        }))
    }

    /// Replaces only the `data` of the next authored injection before it is
    /// admitted. The scenario's `meta.ext`, provenance and sequence remain
    /// authored facts; an admitted envelope is never mutated.
    ///
    /// # Errors
    ///
    /// Refuses outside a quiescent injection boundary or when the value does
    /// not inhabit the resolved ingress type.
    pub fn replace_next_injection_data(&mut self, value: Value) -> Result<(), String> {
        if !self.quiescent {
            return Err(String::from(
                "input data can be edited only before the next scenario action",
            ));
        }
        let Some(ScenarioStep::Inject {
            target,
            value: authored,
            ..
        }) = self.scenario.steps.get(self.next_action)
        else {
            return Err(String::from("the next scenario action is not an injection"));
        };
        let resolved = injection_port(self.plan, target)?;
        let port = target.port();
        if let Err(violation) = resolved.ty().check_value_type("edited input data", &value) {
            return Err(format!(
                "the edited input does not match `{port}`: {violation}"
            ));
        }
        let mut replacement = self
            .next_injection_override
            .take()
            .map_or_else(|| draft(target.port(), authored), Ok)?;
        replacement.data = Data::materialized(value);
        self.next_injection_override = Some(replacement);
        Ok(())
    }

    /// Current verdict projection without consuming the inspectable world.
    #[must_use]
    pub fn report(&self) -> ScenarioReport {
        ScenarioReport {
            simulation: self.simulation.clone(),
            scenario: self.scenario.name.clone(),
            label: self.scenario.label.clone(),
            failures: self.failures.clone(),
        }
    }

    /// Current execution, borrowed without granting mutation to the debugger.
    #[must_use]
    pub const fn execution(&self) -> &Execution<'plan> {
        self.worker.execution()
    }

    /// Read-only causal journal.
    #[must_use]
    pub const fn journal(&self) -> &MemJournal {
        self.worker.journal()
    }

    /// Whether an authored clock advance is suspended between its transitions.
    #[must_use]
    pub const fn is_advancing_time(&self) -> bool {
        self.advance.is_some()
    }

    /// Applies one authored action without draining subsequent Execution work.
    ///
    /// # Errors
    ///
    /// A caller must finish ready work from the preceding action first.
    pub fn next_action(&mut self) -> Result<Option<ScenarioActionKind>, String> {
        if self.source_input.borrow().is_pending() {
            return Err(String::from(
                "the current source injection is pending admission under mailbox pressure",
            ));
        }
        if !self.quiescent {
            return Err(String::from(
                "the current scenario action still has ready Execution transactions",
            ));
        }
        let Some(step) = self.scenario.steps.get(self.next_action) else {
            return Ok(None);
        };
        let kind = match step {
            ScenarioStep::Inject { target, value, seq } => {
                self.window_start = self.worker.execution().envelope_views().len();
                self.output_window_start = self.worker.execution().outputs().len();
                self.record_window_start = self.worker.journal().records().len();
                let candidate = self
                    .next_injection_override
                    .take()
                    .map_or_else(|| draft(target.port(), value), Ok);
                match candidate {
                    Ok(draft) => {
                        let submission = IngressProposal {
                            provenance: IngressProvenance::single(
                                target.connector().cloned().unwrap_or_else(|| {
                                    node::Path::new(["lab"])
                                        .unwrap_or_else(|_| unreachable!("`lab` is a valid path"))
                                }),
                            ),
                            draft,
                            sequence: match seq {
                                Some(seq) => IngressSequence::Explicit(*seq),
                                None => IngressSequence::Next,
                            },
                            session: None,
                        };
                        if target.connector().is_some() {
                            let offered = u64::try_from(self.next_action)
                                .map_err(|_| String::from("the source action index exceeds u64"))
                                .and_then(|action| {
                                    self.source_input.borrow_mut().offer(submission, action)
                                });
                            if let Err(error) = offered.and_then(|()| {
                                self.worker.poll_inbound().map_err(worker_error).map(|_| ())
                            }) {
                                self.failures
                                    .push(format!("the source injection was refused: {error}"));
                            }
                        } else if let Err(error) = self.worker.accept_input(submission) {
                            self.failures
                                .push(format!("the injection was refused: {error}"));
                        }
                    }
                    Err(message) => self.failures.push(message),
                }
                self.quiescent = false;
                ScenarioActionKind::Inject
            }
            ScenarioStep::Advance { nanos } => {
                self.window_start = self.worker.execution().envelope_views().len();
                self.output_window_start = self.worker.execution().outputs().len();
                self.record_window_start = self.worker.journal().records().len();
                let target = self
                    .worker
                    .execution()
                    .logical_time()
                    .saturating_add(*nanos);
                self.advance = Some(TimeAdvance::new(target));
                self.quiescent = false;
                ScenarioActionKind::Advance
            }
            ScenarioStep::Expect(expectation) => {
                let envelopes = expectation_envelopes(
                    expectation,
                    &self.worker.execution().envelope_views()[self.window_start..],
                    &self.worker.execution().outputs()[self.output_window_start..],
                    &self.worker.journal().records()[self.record_window_start..],
                );
                if let Some(message) = check_expectation(self.plan, expectation, &envelopes) {
                    self.failures.push(message);
                }
                self.quiescent = true;
                ScenarioActionKind::Expect
            }
        };
        self.next_action = self.next_action.saturating_add(1);
        self.last_action = Some(kind);
        Ok(Some(kind))
    }

    /// Executes at most one ready Execution transaction, then polls the scripted
    /// world only after ready work drains, as the batch Lab does. Driver
    /// polling itself is not a graph transaction.
    ///
    /// # Errors
    ///
    /// A faulting transaction commits nothing and remains reserved for retry.
    pub fn step_transaction(&mut self) -> Result<bool, ScenarioStepError> {
        if let Some(advance) = &mut self.advance {
            let progressed = self.worker.advance_once(advance).map_err(|error| {
                // A deadline can retain a closure before a Driver refuses.
                // Do not advertise transaction rollback for this wider seam.
                ScenarioStepError::Boundary(format!("the advance faulted: {error}"))
            })?;
            if advance.is_complete() {
                self.advance = None;
            }
            self.collect_driver_faults();
            if progressed {
                return Ok(true);
            }
        }
        let ran = self
            .worker
            .step()
            .map_err(|error| ScenarioStepError::Transaction(format!("a step faulted: {error}")))?;
        let progressed = if self.worker.execution().has_ready_work() {
            false
        } else {
            self.poll_driver().map_err(ScenarioStepError::Boundary)?
        };
        // A retained opening or write acknowledgement can make the next
        // scripted frame admissible without queuing an Executor transaction.
        // Confirm that the world is also at rest before advancing the script.
        self.quiescent = !progressed && !self.worker.execution().has_ready_work();
        Ok(ran)
    }

    /// Polls scripted Driver outcomes without executing a graph transaction.
    ///
    /// This is the safe-pause seam: a Call may acquire a retained completion,
    /// but its handler cannot run until an explicit transaction step.
    pub fn poll_world(&mut self) -> Result<bool, String> {
        if self.advance.is_some() {
            return Ok(false);
        }
        let progressed = self.poll_driver()?;
        self.quiescent = !progressed && !self.worker.execution().has_ready_work();
        Ok(progressed)
    }

    /// Runs ready work to the same boundary used by the historical batch Lab.
    pub fn settle_current_action(&mut self) {
        if let Some(mut advance) = self.advance.take() {
            loop {
                match self.worker.advance_once(&mut advance) {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(error) => {
                        self.failures.push(format!("the advance faulted: {error}"));
                        break;
                    }
                }
            }
        }
        let failures = &mut self.failures;
        let faults = &self.faults;
        let result = self.worker.settle_action(|report| {
            failures.extend(faults.borrow_mut().drain(..));
            for (response, reason) in &report.rejected {
                failures.push(format!(
                    "a reply to {:?} was refused before closure: {reason}",
                    response.call_id
                ));
            }
        });
        if let Err(errors) = result {
            self.failures.extend(errors.into_iter().map(worker_error));
        }
        self.collect_driver_faults();
        self.quiescent = true;
        if self.source_input.borrow().is_pending() {
            self.failures.push(String::from("the source injection remains pending under mailbox pressure; the sequential scenario cannot reach its next action"));
        }
    }

    /// Applies every remaining action under the batch runner's established
    /// settle-between-actions semantics.
    pub fn run_to_completion(&mut self) {
        while self.next_action < self.scenario.steps.len() {
            if self.next_action().is_err() {
                self.settle_current_action();
                if self.source_input.borrow().is_pending() {
                    break;
                }
                continue;
            }
            if !self.quiescent {
                self.settle_current_action();
                if self.source_input.borrow().is_pending() {
                    break;
                }
            }
        }
    }

    /// Consumes the session into the public batch result.
    #[must_use]
    pub fn finish(self) -> ScenarioRun {
        ScenarioRun {
            report: ScenarioReport {
                simulation: self.simulation,
                scenario: self.scenario.name.clone(),
                label: self.scenario.label.clone(),
                failures: self.failures,
            },
            records: self.worker.shutdown().records().to_vec(),
        }
    }

    fn poll_driver(&mut self) -> Result<bool, String> {
        let failures = &mut self.failures;
        let faults = &self.faults;
        let source_input = &self.source_input;
        let progressed = self
            .worker
            .poll(|report| {
                failures.extend(faults.borrow_mut().drain(..));
                failures.extend(source_input.borrow_mut().take_fault());
                for (response, reason) in &report.rejected {
                    failures.push(format!(
                        "a reply to {:?} was refused before closure: {reason}",
                        response.call_id
                    ));
                }
            })
            .map_err(worker_error)?;
        if !progressed
            && !self.worker.execution().has_ready_work()
            && self.source_input.borrow().is_pending()
        {
            return Err(String::from(
                "the source injection remains pending under mailbox pressure; no execution work is ready",
            ));
        }
        Ok(progressed)
    }

    fn collect_driver_faults(&mut self) {
        self.failures.extend(self.faults.borrow_mut().drain(..));
        self.failures
            .extend(self.source_input.borrow_mut().take_fault());
    }
}

/// Runs every scenario of a checked lab against its plan. Each
/// scenario gets a fresh journal and a fresh machine — scenarios
/// share nothing but the graph.
#[must_use]
pub fn run(
    plan: &ValidatedExecutionPlan,
    graph_version: GraphVersion,
    lab: &Lab,
) -> Vec<ScenarioRun> {
    run_inner(plan, None, graph_version, lab, None)
}

/// Runs every scenario with the same resolved deployment inputs used by real
/// Drivers. Scripted Drivers consume logical secret references and typed boot
/// values but never receive or expose physical secret bytes in expectations.
#[must_use]
pub fn run_with_boot(
    plan: &ValidatedExecutionPlan,
    graph_version: GraphVersion,
    lab: &Lab,
    boot: &crate::deployment::BootBindings,
) -> Vec<ScenarioRun> {
    run_inner(plan, None, graph_version, lab, Some(boot))
}

/// Runs each scenario against the exact original program, including Retry.
#[must_use]
pub fn run_program(
    plan: &ValidatedExecutionPlan,
    hash: SemanticHash,
    graph_version: GraphVersion,
    lab: &Lab,
    boot: Option<&crate::deployment::BootBindings>,
) -> Vec<ScenarioRun> {
    run_inner(plan, Some(hash), graph_version, lab, boot)
}

fn run_inner(
    plan: &ValidatedExecutionPlan,
    hash: Option<SemanticHash>,
    graph_version: GraphVersion,
    lab: &Lab,
    boot: Option<&crate::deployment::BootBindings>,
) -> Vec<ScenarioRun> {
    let mut reports = Vec::new();
    for simulation in &lab.simulations {
        for scenario in &simulation.scenarios {
            reports.push(run_scenario(
                plan,
                hash,
                graph_version,
                &simulation.name,
                &simulation.mocks,
                scenario,
                boot,
            ));
        }
    }
    reports
}

fn run_scenario(
    plan: &ValidatedExecutionPlan,
    hash: Option<SemanticHash>,
    graph_version: GraphVersion,
    simulation: &str,
    shared_mocks: &[Mock],
    scenario: &Scenario,
    boot: Option<&crate::deployment::BootBindings>,
) -> ScenarioRun {
    match Simulator::try_new_inner(
        plan,
        hash,
        graph_version,
        simulation,
        shared_mocks,
        scenario,
        boot,
    ) {
        Ok(mut session) => {
            session.run_to_completion();
            session.finish()
        }
        Err(error) => ScenarioRun {
            report: ScenarioReport {
                simulation: String::from(simulation),
                scenario: scenario.name.clone(),
                label: scenario.label.clone(),
                failures: vec![format!("the Lab Driver composition refused: {error}")],
            },
            records: Vec::new(),
        },
    }
}

// The Lab keeps its authored-action diagnostics; the Worker retains typed
// failures until this presentation boundary.
fn worker_error(error: crate::worker::Error) -> String {
    match error {
        crate::worker::Error::Step(error) => format!("a step faulted: {error}"),
        crate::worker::Error::Poll(error) => format!("Driver polling faulted: {error}"),
        error => error.to_string(),
    }
}

fn injection_port<'plan>(
    plan: &'plan ValidatedExecutionPlan,
    target: &InjectionTarget,
) -> Result<&'plan crate::plan::ResolvedPort, String> {
    let (ports, direction) = match target {
        InjectionTarget::Boundary(_) => {
            (plan.resolved_ports().boundary(), node::PortDirection::Input)
        }
        InjectionTarget::Connector { path, port } => {
            let (index, node) = plan
                .plan()
                .nodes
                .iter()
                .enumerate()
                .find(|(_, node)| &node.path == path)
                .ok_or_else(|| format!("the injection names no source Connector `{path}`"))?;
            let supported = node.connector().is_some_and(|spec| {
                spec.contract.as_str() == crate::connector::stdlib::HTTP_SERVER_CONNECTOR_ID
                    && matches!(spec.attribute("mode"), Some(crate::plan::AttrSetting::Word(mode)) if mode == "ack")
                    && port.name() == "out"
            });
            if !supported {
                return Err(format!(
                    "the injection `{path}.{port}` requires the HttpServer ack Lab ingress model"
                ));
            }
            let slot = crate::node::NodeIndex::new(
                u32::try_from(index).map_err(|_| String::from("source slot exceeds u32"))?,
            );
            (
                plan.resolved_ports()
                    .node(slot)
                    .ok_or_else(|| format!("source `{path}` has no resolved ports"))?,
                node::PortDirection::Output,
            )
        }
    };
    ports
        .iter()
        .find(|port| port.port() == target.port() && port.direction() == direction)
        .ok_or_else(|| format!("the injection has no resolved ingress `{target:?}`"))
}

fn expectation_envelopes<'a>(
    expectation: &Expectation,
    window: &'a [EnvelopeView],
    boundary: &'a [crate::runtime::execution::BoundaryOutput],
    records: &'a [Record],
) -> Vec<&'a Envelope> {
    match &expectation.target {
        ExpectTarget::Boundary(port) => boundary
            .iter()
            .filter(|output| output.port == *port)
            .map(|output| &output.envelope)
            .collect(),
        // An inbound source is an INGRESS fact, not a synthetic emission
        // transaction. Read it directly, once, in this action's journal window.
        ExpectTarget::Node { path, port } => records
            .iter()
            .filter_map(|record| {
                let Record::Ingress(root) = record else {
                    return None;
                };
                (root.connector_path.as_ref() == Some(path) && root.port == *port)
                    .then_some(&root.envelope)
            })
            .chain(
                window
                    .iter()
                    .filter(|observed| {
                        observed.node.as_ref().is_some_and(|node| node == path)
                            && observed.port == *port
                    })
                    .map(|observed| &observed.envelope),
            )
            .collect(),
    }
}

fn check_expectation(
    plan: &ValidatedExecutionPlan,
    expectation: &Expectation,
    deliveries: &[&Envelope],
) -> Option<String> {
    let ty = match &expectation.target {
        ExpectTarget::Boundary(port) => plan
            .resolved_ports()
            .boundary()
            .iter()
            .find(|resolved| resolved.port() == port)
            .map(|port| port.ty()),
        ExpectTarget::Node { path, port } => plan
            .nodes
            .iter()
            .position(|node| node.path == *path)
            .and_then(|index| {
                plan.resolved_ports()
                    .node_port_type(crate::node::NodeIndex::new(index as u32), port)
            }),
    };
    let name = expectation
        .label
        .clone()
        .unwrap_or_else(|| format!("{:?}", expectation.target));
    let mut matches = 0usize;
    for observed in deliveries {
        match &expectation.predicate {
            None => matches += 1,
            Some(tree) => {
                let mut env = BTreeMap::new();
                let data = ty.map_or_else(
                    || runtime::DataSlot::new(observed.data()),
                    |ty| runtime::DataSlot::typed(observed.data(), ty),
                );
                match data.materialize() {
                    Ok(data) => {
                        env.insert(String::from("data"), expr::EvalValue::from(data));
                    }
                    Err(fault) => {
                        return Some(format!(
                            "expect {name}: a delivery's data is unreadable: {fault}"
                        ));
                    }
                }
                env.insert(
                    String::from("meta"),
                    expr::EvalValue::from(runtime::meta_value(observed.meta())),
                );
                // A faulting or non-boolean predicate fails the
                // expectation itself — "did not match" would be a
                // false green with the void quantifier.
                match expr::eval(tree, &env) {
                    Ok(expr::EvalValue::Bool(true)) => matches += 1,
                    Ok(expr::EvalValue::Bool(false)) => {}
                    Ok(_) => {
                        return Some(format!("expect {name}: the predicate is not a bool"));
                    }
                    Err(error) => {
                        return Some(format!("expect {name}: the predicate faulted: {error}"));
                    }
                }
            }
        }
    }
    let held = match &expectation.quantifier {
        Quantifier::Exists => matches >= 1,
        // The void quantifier counts deliveries, predicate-filtered
        // when one is written.
        Quantifier::Nothing => {
            matches == 0 && (expectation.predicate.is_some() || deliveries.is_empty())
        }
        Quantifier::Count(cmp, expected) => {
            let actual = i64::try_from(matches).unwrap_or(i64::MAX);
            match cmp {
                Cmp::Eq => actual == *expected,
                Cmp::Ne => actual != *expected,
                Cmp::Lt => actual < *expected,
                Cmp::Gt => actual > *expected,
                Cmp::Le => actual <= *expected,
                Cmp::Ge => actual >= *expected,
            }
        }
    };
    if held {
        None
    } else {
        Some(format!(
            "expect {name}: {matches} match(es) in a window of {} deliveries",
            deliveries.len()
        ))
    }
}

/// Builds a world-authored draft from an injection value: `data`
/// writes shape the payload, `meta.ext.*` supplies world context. The graph's
/// `meta.app` is not writable here. A draft inherits nothing.
fn draft(port: &node::Port, value: &EmitValue) -> Result<EnvelopeDraft, String> {
    let empty = BTreeMap::new();
    match value {
        EmitValue::Data(tree) => {
            let value =
                expr::eval(tree, &empty).map_err(|error| format!("the injection: {error}"))?;
            Ok(EnvelopeDraft {
                port: port.clone(),
                data: Data::materialized(
                    value
                        .into_value()
                        .map_err(|fault| format!("the injection: {fault}"))?,
                ),
                ext: Namespace::new(),
            })
        }
        EmitValue::FreshData(writes) | EmitValue::Patch(writes) => {
            // A draft is born a fresh STRUCT — the same physical form
            // the handler builders produce (`{…}` builds a struct).
            let mut data = crate::envelope::Value::Struct(Vec::new());
            let mut ext = Namespace::new();
            for write in writes {
                let value = expr::eval(&write.value, &empty)
                    .map_err(|error| format!("the injection: {error}"))?
                    .into_value()
                    .map_err(|error| format!("the injection: {error}"))?;
                match &write.target {
                    WriteTarget::Data(path) => {
                        runtime::apply_struct_write(&mut data, path, value);
                    }
                    WriteTarget::Ext(field) => {
                        // The fixture is the world's half: it seeds
                        // the external context (D-2026-08-10-1).
                        ext.insert(
                            field.clone(),
                            runtime::to_meta_value(&value)
                                .map_err(|fault| format!("the injection: {fault}"))?,
                        );
                    }
                    WriteTarget::App(_) => {
                        // Refused at check; unreachable from a
                        // validated lab.
                        return Err(String::from(
                            "the injection: `meta.app` is the graph's zone, never the world's",
                        ));
                    }
                }
            }
            Ok(EnvelopeDraft {
                port: port.clone(),
                data: Data::materialized(data),
                ext,
            })
        }
        EmitValue::Nothing => Err(String::from("`nothing` injects nothing")),
        EmitValue::Identity => Err(String::from("a draft has no input to re-emit")),
    }
}

struct MockConnectorFactory {
    id: ContractId,
    bind_protocol: ScriptedProtocolBinder,
    mocks: SharedMocks,
    faults: MockFaults,
}

impl core::fmt::Debug for MockConnectorFactory {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("MockConnectorFactory")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl DriverFactory for MockConnectorFactory {
    fn contract_id(&self) -> &ContractId {
        &self.id
    }

    fn roles(&self) -> ConnectorDirection {
        ConnectorDirection::Outbound
    }

    fn create<'plan>(
        &self,
        plan: &'plan ValidatedExecutionPlan,
        nodes: &[node::Path],
    ) -> Result<Box<dyn Driver + 'plan>, DriverFactoryError> {
        self.create_with_boot(plan, nodes, &crate::deployment::BootBindings::empty())
    }

    fn create_with_boot<'plan>(
        &self,
        plan: &'plan ValidatedExecutionPlan,
        nodes: &[node::Path],
        boot: &crate::deployment::BootBindings,
    ) -> Result<Box<dyn Driver + 'plan>, DriverFactoryError> {
        let protocol = (self.bind_protocol)(plan, nodes, boot)?;
        Ok(Box::new(MockOutboundConnectorDriver {
            plan,
            mocks: self.mocks.clone(),
            protocol,
            pending: Vec::new(),
            faults: Rc::clone(&self.faults),
        }))
    }
}

fn mock_bindings<'plan>(
    plan: &'plan ValidatedExecutionPlan,
    mocks: BTreeMap<node::Path, Mock>,
    boot: &crate::deployment::BootBindings,
    source_input: Rc<RefCell<HttpServerLabInput>>,
) -> Result<(RuntimeDrivers<'plan>, MockFaults), String> {
    let faults = Rc::new(RefCell::new(Vec::new()));
    let mocks = share_mocks(mocks);
    let mut registry = DriverRegistry::default();
    registry = registry
        .try_with(Box::new(process_service::Factory::new(
            Rc::clone(&mocks),
            Rc::clone(&faults),
        )?))
        .map_err(|e| e.to_string())?;
    for (spelling, bind_protocol) in scripted_protocol_binders() {
        let id = ContractId::parse(spelling).map_err(|error| error.to_string())?;
        registry = registry
            .try_with(Box::new(MockConnectorFactory {
                id,
                bind_protocol,
                mocks: Rc::clone(&mocks),
                faults: Rc::clone(&faults),
            }))
            .map_err(|error| error.to_string())?;
    }
    for factory in inbound_lab_factories(source_input).map_err(|error| error.to_string())? {
        registry = registry
            .try_with(factory)
            .map_err(|error| error.to_string())?;
    }
    let bindings = RuntimeDrivers::prepare_with_boot(plan, &registry, boot)
        .map_err(|error| error.to_string())?;
    Ok((bindings, faults))
}

fn share_mocks(mocks: BTreeMap<node::Path, Mock>) -> SharedMocks {
    Rc::new(
        mocks
            .into_iter()
            .map(|(path, mock)| (path, Rc::new(mock)))
            .collect(),
    )
}

/// The scripted Driver: answers journaled intents from mock branches — first
/// match in text order — and reports every uncovered call loudly. Its protocol
/// preparation is the same pure binding used by the real family Driver; only
/// the external effect is replaced.
struct MockOutboundConnectorDriver<'plan> {
    plan: &'plan ValidatedExecutionPlan,
    mocks: SharedMocks,
    protocol: Box<dyn CallProtocol>,
    pending: Vec<CallResponse>,
    faults: MockFaults,
}

impl MockOutboundConnectorDriver<'_> {
    fn fault(&self, message: String) {
        self.faults.borrow_mut().push(message);
    }

    fn answer(&mut self, intent: &CallOpen, request: &Envelope) -> Option<Completion> {
        let outputs =
            self.answer_outputs(&intent.connector_path, Some(intent.logical_time), request)?;
        match Completion::try_new(outputs) {
            Ok(completion) => Some(completion),
            Err(error) => {
                self.fault(format!("the mock completion is invalid: {error}"));
                None
            }
        }
    }

    fn answer_outputs(
        &mut self,
        connector: &node::Path,
        logical_time: Option<i64>,
        request: &Envelope,
    ) -> Option<Vec<EnvelopeDraft>> {
        let Some(mock) = self.mocks.get(connector).cloned() else {
            self.fault(format!(
                "an uncovered call reached `{connector}`; mock it or script silence"
            ));
            return None;
        };
        // The request materializes on the EVALUATOR'S demand, like
        // the machine's own bindings: an identity reply, a meta-only
        // guard, a losing later branch, or a statement past the
        // reply must never force an opaque request to decode. The
        // slot caches across branches — one decode at most.
        let mut env: BTreeMap<String, expr::EvalValue> = BTreeMap::new();
        if let Some(time) = logical_time {
            env.insert(String::from("now"), expr::EvalValue::Timestamp(time));
        }
        env.insert(
            String::from("meta"),
            expr::EvalValue::from(runtime::meta_value(request.meta())),
        );
        let ty = self
            .plan
            .nodes
            .iter()
            .position(|node| node.path == *connector)
            .and_then(|index| {
                self.plan
                    .resolved_ports()
                    .node(crate::node::NodeIndex::new(index as u32))
            })
            .and_then(|ports| {
                ports
                    .iter()
                    .find(|port| port.direction() == node::PortDirection::Input)
            });
        let slot = ty.map_or_else(
            || runtime::DataSlot::new(request.data()),
            |port| runtime::DataSlot::typed(request.data(), port.ty()),
        );
        for handler in &mock.handlers {
            // Mock declarations retain their first-matching-filter protocol.
            // Once a declaration wins, its Dispatch selects one whole reply.
            let (guard, dispatch) = match handler.admission_guard.as_ref() {
                Some(guard) => (Some(guard), &handler.dispatch),
                None => handler
                    .dispatch
                    .filter_parts()
                    .map_or((None, &handler.dispatch), |(predicate, branch)| {
                        (Some(predicate), branch)
                    }),
            };
            let passes = match guard {
                None => true,
                Some(guard) => {
                    let lazy = runtime::LazyBindings {
                        base: &env,
                        data: &slot,
                        assets: &[],
                    };
                    let result = expr::eval(guard, &lazy);
                    if let Some(fault) = slot.take_fault() {
                        self.fault(format!("the mock request is unreadable: {fault}"));
                        return None;
                    }
                    match result {
                        Ok(expr::EvalValue::Bool(passes)) => passes,
                        // A faulting or non-boolean guard is a fault
                        // of the scenario, never a silent
                        // fall-through to the next branch.
                        Ok(_) => {
                            self.fault(format!("a mock guard of `{}` is not a bool", connector));
                            return None;
                        }
                        Err(error) => {
                            self.fault(format!("a mock guard of `{}` faulted: {error}", connector));
                            return None;
                        }
                    }
                }
            };
            if !passes {
                continue;
            }
            let selected = runtime::select_dispatch(
                dispatch,
                &runtime::LazyBindings {
                    base: &env,
                    data: &slot,
                    assets: &[],
                },
            );
            if let Some(fault) = slot.take_fault() {
                self.fault(format!("the mock request is unreadable: {fault}"));
                return None;
            }
            match selected {
                Ok(selected) => {
                    env.extend(selected.locals);
                    return self.reply(selected.statements, request, env, &slot);
                }
                Err(error) => {
                    self.fault(format!("the mock Dispatch faulted: {error}"));
                    return None;
                }
            }
        }
        self.fault(format!(
            "no mock branch of `{}` matched the request; add a catch-all or script silence",
            connector
        ));
        None
    }

    /// Evaluates the first matching branch. `out = nothing` is the
    /// written silence — the call stays open for the timeout
    /// machinery.
    fn reply(
        &mut self,
        body: &[crate::compiler::hir::Statement],
        request: &Envelope,
        mut env: BTreeMap<String, expr::EvalValue>,
        slot: &runtime::DataSlot<'_>,
    ) -> Option<Vec<EnvelopeDraft>> {
        use crate::compiler::hir::Statement;
        let mut outputs = Vec::new();
        for statement in body {
            match statement {
                Statement::Let { name, value, .. } => match self.lazy_eval(value, &env, slot) {
                    Ok(value) => {
                        env.insert(name.clone(), value);
                    }
                    Err(message) => {
                        self.fault(message);
                        return None;
                    }
                },
                Statement::SetVar { .. } => {
                    self.fault(String::from("a mock cannot write instance state"));
                    return None;
                }
                Statement::Emit { port, value } => {
                    match self.mock_data(value, request, &env, slot) {
                        Ok(Some(data)) => outputs.push(EnvelopeDraft {
                            port: port.clone(),
                            data,
                            ext: crate::envelope::Namespace::new(),
                        }),
                        Ok(None) => {}
                        Err(message) => {
                            self.fault(message);
                            return None;
                        }
                    }
                }
                Statement::Stop => {
                    self.fault(String::from(
                        "a mock cannot supervise instance Closing in this build",
                    ));
                    return None;
                }
                Statement::MultiEmit { .. } | Statement::LoopCommand(_) => {
                    self.fault(String::from(
                        "a mock supports data replies, not multi-emission or manager commands",
                    ));
                    return None;
                }
            }
        }
        if outputs.is_empty() {
            return None;
        }
        Some(outputs)
    }

    /// One lazily-bound evaluation; an unreadable request keeps its
    /// dedicated message, any other fault is the mock's own.
    fn lazy_eval(
        &self,
        tree: &expr::Tree,
        env: &BTreeMap<String, expr::EvalValue>,
        slot: &runtime::DataSlot<'_>,
    ) -> Result<expr::EvalValue, String> {
        let lazy = runtime::LazyBindings {
            base: env,
            data: slot,
            assets: &[],
        };
        let result = expr::eval(tree, &lazy);
        if let Some(fault) = slot.take_fault() {
            return Err(format!("the mock request is unreadable: {fault}"));
        }
        result.map_err(|error| format!("the mock faulted: {error}"))
    }

    fn mock_data(
        &self,
        value: &EmitValue,
        request: &Envelope,
        env: &BTreeMap<String, expr::EvalValue>,
        slot: &runtime::DataSlot<'_>,
    ) -> Result<Option<Data>, String> {
        match value {
            EmitValue::Nothing => Ok(None),
            EmitValue::Identity => Ok(Some(request.data().clone())),
            EmitValue::Data(tree) => {
                // The same rule as the machine: a whole payload
                // arriving uninspected — `out = data`, a local carry
                // — ships the REPRESENTATION, whatever the spelling.
                let value = self.lazy_eval(tree, env, slot)?;
                if matches!(value, expr::EvalValue::Payload(_)) {
                    return Ok(Some(request.data().clone()));
                }
                let value = runtime::resolve_payload(value, slot)
                    .map_err(|fault| format!("the mock request is unreadable: {fault}"))?;
                Ok(Some(Data::materialized(value)))
            }
            EmitValue::FreshData(writes) | EmitValue::Patch(writes) => {
                let mut data = if matches!(value, EmitValue::Patch(_)) {
                    match request.data() {
                        Data::Materialized(value) => value.clone(),
                        // A patch base is a DIRECT demand, through
                        // the same cached slot.
                        _ => slot
                            .materialize()
                            .map_err(|fault| format!("the mock request is unreadable: {fault}"))?,
                    }
                } else {
                    crate::envelope::Value::Struct(Vec::new())
                };
                for write in writes {
                    let evaluated =
                        runtime::resolve_payload(self.lazy_eval(&write.value, env, slot)?, slot)
                            .map_err(|fault| format!("the mock request is unreadable: {fault}"))?;
                    match &write.target {
                        WriteTarget::Data(path) => {
                            runtime::apply_struct_write(&mut data, path, evaluated);
                        }
                        // A reply carries data only: its envelope is
                        // constructed by the Connector's own transaction.
                        _ => {
                            return Err(String::from(
                                "the mock faulted: a reply can write data only",
                            ));
                        }
                    }
                }
                Ok(Some(Data::materialized(data)))
            }
        }
    }
}

impl OutboundCall for MockOutboundConnectorDriver<'_> {
    fn prepare_request(
        &mut self,
        connector: &node::Path,
        data: &Data,
        settings: &BTreeMap<String, Value>,
    ) -> RequestPreparation {
        self.protocol.prepare_request(connector, data, settings)
    }

    fn scheduler_completion(
        &mut self,
        _: &node::Path,
        completion: SchedulerCompletion,
    ) -> Completion {
        self.protocol.scheduler_completion(completion)
    }

    fn start(&mut self, intent: &CallOpen, request: &Envelope) -> CallStart {
        if let Some(completion) = self.answer(intent, request) {
            self.pending.push(CallResponse {
                call_id: intent.call_id,
                completion,
            });
        }
        CallStart::Started
    }

    fn poll_responses(&mut self) -> Vec<CallResponse> {
        std::mem::take(&mut self.pending)
    }
}

impl Driver for MockOutboundConnectorDriver<'_> {
    fn inbound(&mut self) -> Option<&mut dyn crate::connector::driver::Ingress> {
        None
    }

    fn outbound(&mut self) -> Option<&mut dyn Outbound> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::compile;
    use crate::connector::contract::ContractCatalog;

    /// Drives the scripted driver directly with an OPAQUE request:
    /// the branch and statement structure decides the demand, so a
    /// pre-scan of every handler would fail both cases below.
    fn opaque_answer(mock_body: &str) -> (Option<Completion>, Vec<String>) {
        let source = format!(
            "graph g : App {{\n  connector echo : Process {{\n    mode = job ; command = \"cat\"\n    codec = lines\n    idempotent = true; behavior = deterministic\n  }}\n  in -> echo -> out\n  echo.err -> nothing\n  echo.exit -> nothing\n}}\n\nlab g {{\n  simulation probing on app {{\n    mock echo {{\n{mock_body}\n    }}\n    scenario s \"unused\" {{\n      app.in = {{v = 1}}\n    }}\n  }}\n}}\n"
        );
        let compilation = compile(
            &[source.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("a valid unit");
        let mock = compilation.labs[0].simulations[0].mocks[0].clone();
        let path = node::Path::new(["echo"]).unwrap();
        let mut mocks = BTreeMap::new();
        mocks.insert(path.clone(), mock);
        let faults = Rc::new(RefCell::new(Vec::new()));
        let nodes = [path.clone()];
        let mut driver = MockOutboundConnectorDriver {
            plan: &compilation.plan,
            mocks: share_mocks(mocks),
            protocol: Box::new(
                ProcessRequestSettings::try_bind(&compilation.plan, &nodes)
                    .expect("the Process mock binding is valid"),
            ),
            pending: Vec::new(),
            faults: Rc::clone(&faults),
        };
        let id = crate::envelope::Id::derive(
            crate::envelope::IdKind::Root,
            &[b"request", &0u64.to_le_bytes()],
        );
        let request = Envelope::new(
            crate::envelope::Meta::new(crate::envelope::MetaSys::root(id, 1)),
            Data::encoded(crate::envelope::Encoding::ArrowIpc, *b"\x01\x02"),
        );
        let prepared = crate::connector::exchange::PreparedRequest::seal(vec![1]);
        let intent = crate::connector::exchange::CallOpen {
            call_id: crate::journal::ids::call_id(
                crate::journal::ids::delivery_id(".in>echo.in", id),
                prepared.digest(),
            ),
            emitting_id: id,
            edge: String::from(".in>echo.in"),
            connector_path: path,
            prepared,
            idempotency_key: None,
            resolved_settings: BTreeMap::new(),
            logical_time: 0,
        };
        let outcome = driver.answer(&intent, &request);
        let reported = faults.borrow().clone();
        (outcome, reported)
    }

    #[test]
    fn a_winning_earlier_branch_never_decodes_for_a_later_one() {
        // The first branch is meta-only and WINS; the second reads
        // `data`. The losing branch's text must not force the opaque
        // request to decode.
        let (outcome, faults) = opaque_answer(
            "      on in where meta.sys.graph_version == 1 {\n        out = \"ok\"\n        exit = Exited { exit_code = 0, signal = null }\n      }\n      on in where data == \"x\" {\n        out = .\n        exit = Exited { exit_code = 0, signal = null }\n      }",
        );
        assert!(faults.is_empty(), "no forced decode: {faults:?}");
        assert!(
            outcome.is_some_and(|completion| completion.outputs().len() == 2),
            "the meta-only branch answered"
        );
    }

    #[test]
    fn a_mock_observes_the_same_injected_logical_time_as_the_machine() {
        let (outcome, faults) = opaque_answer(
            "      on in where now() == timestamp(\"1970-01-01T00:00:00Z\") {\n        out = \"epoch\"\n        exit = Exited { exit_code = 0, signal = null }\n      }",
        );
        assert!(
            faults.is_empty(),
            "the context time is explicit: {faults:?}"
        );
        assert!(outcome.is_some(), "the epoch branch answers");
    }

    #[test]
    fn a_mock_carry_is_as_lazy_as_the_name_too() {
        // The mocks speak the handlers' language: `out = data` and
        // `val copy = data; out = copy` both ship the opaque
        // request's REPRESENTATION, exactly like the machine.
        for body in [
            "      on in {\n        out = data\n        exit = Exited { exit_code = 0, signal = null }\n      }",
            "      on in {\n        val copy = data\n        out = copy\n        exit = Exited { exit_code = 0, signal = null }\n      }",
        ] {
            let (outcome, faults) = opaque_answer(body);
            assert!(faults.is_empty(), "`{body}`: no forced decode: {faults:?}");
            assert!(
                outcome.is_some_and(|completion| matches!(
                    &completion.outputs()[0].data,
                    Data::Encoded { bytes, .. } if bytes.as_ref() == b"\x01\x02"
                )),
                "`{body}` must relay the representation"
            );
        }
    }

    #[test]
    fn nested_mock_dispatch_keeps_prefixes_and_opaque_payloads_lazy() {
        let (outcome, faults) = opaque_answer(
            r#"on in {
              val current = meta.sys.graph_version == 1
              match current {
                false => nothing
                true => {
                  val copy = data
                  match {
                    meta.sys.graph_version == 1 => {
                      out = copy
                      exit = Exited { exit_code = 0, signal = null }
                    }
                    size(data) > 0 => {
                      out = "must not decode"
                      exit = Exited { exit_code = 1, signal = null }
                    }
                    _ => nothing
                  }
                }
              }
            }"#,
        );
        assert!(
            faults.is_empty(),
            "no losing arm forces the request: {faults:?}"
        );
        let completion = outcome.expect("the selected nested arm answered");
        assert_eq!(completion.outputs().len(), 2);
        assert!(matches!(&completion.outputs()[0].data,
            Data::Encoded { bytes, .. } if bytes.as_ref() == b"\x01\x02"));
    }

    #[test]
    fn a_faulting_nested_decision_does_not_fall_through_to_another_mock() {
        let (outcome, faults) = opaque_answer(
            r#"on in match meta.sys.graph_version == 1 {
              false => nothing
              true => match {
                size(data) > 0 => {
                  out = "unreachable"
                  exit = Exited { exit_code = 0, signal = null }
                }
                _ => nothing
              }
            }
            on in {
              out = "must not fall through"
              exit = Exited { exit_code = 0, signal = null }
            }"#,
        );
        assert!(outcome.is_none());
        assert_eq!(faults.len(), 1);
        assert!(faults[0].contains("unreadable"), "{faults:?}");
    }

    #[test]
    fn a_mock_evaluates_the_whole_atomic_completion() {
        // An emission no longer returns from a mock branch: all drafts are
        // built before one atomic completion is handed to the Driver poller.
        let (outcome, faults) = opaque_answer(
            "      on in {\n        out = .\n        val forced = size(data)\n        exit = Exited { exit_code = 0, signal = null }\n      }",
        );
        assert!(outcome.is_none());
        assert_eq!(faults.len(), 1, "the later demand is evaluated once");
    }

    /// The M0 probe and its lab, in one unit — the graph under test
    /// is the production graph; only the world is scripted.
    const PROBE_WITH_LAB: &str = r#"
graph m0_probe : App {
  connector echo : Process {
    mode = job ; command = "cat"
    codec = lines
    idempotent = true; behavior = deterministic
    timeout = duration("5s")
  }

  operator shout {
    input in: struct { text: text }
    var count : int = 0
    on in where data.text != "" {
      count = count + 1
      out = data.text
    }
  }

  in -> shout -> echo -> out
  echo.err -> nothing
  echo.exit -> nothing
}

lab m0_probe {
  simulation smoke on app {
    mock echo {
      on in where data == "salut" {
        out = "salut"
        exit = Exited { exit_code = 0, signal = null }
      }
      on in {
        out = .
        exit = Exited { exit_code = 0, signal = null }
      }
    }

    scenario roundtrip "one message through a scripted world" {
      app.in = { text = "salut" }
      expect "the echo came back" app.out count == 1 where data == "salut"
      expect nothing echo.err
    }

    scenario silence_expires "a silent driver times out" {
      mock echo {
        on in { out = nothing }
      }
      app.in = { text = "salut" }
      expect nothing app.out
      advance duration("5s")
      expect "the budget expired" echo.exit where data is TimeoutExpired
    }

    scenario filtered "an empty text never reaches the world" {
      app.in = { text = "" }
      expect nothing echo.in
      expect nothing app.out
    }
  }
}
"#;

    fn reports() -> Vec<ScenarioReport> {
        let compilation = compile(
            &[PROBE_WITH_LAB],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("a valid unit");
        assert_eq!(compilation.labs.len(), 1);
        run(&compilation.plan, GraphVersion(1), &compilation.labs[0])
            .into_iter()
            .map(|run| run.report)
            .collect()
    }

    #[test]
    fn recursive_mock_dispatch_preserves_the_program_and_its_causal_run() {
        let linear = r#"on in where data == "salut" {
        out = "salut"
        exit = Exited { exit_code = 0, signal = null }
      }
      on in {
        out = .
        exit = Exited { exit_code = 0, signal = null }
      }"#;
        let recursive = r#"on in match data == "salut" {
        true => {
          out = "salut"
          exit = Exited { exit_code = 0, signal = null }
        }
        false => {
          out = .
          exit = Exited { exit_code = 0, signal = null }
        }
      }"#;
        assert!(PROBE_WITH_LAB.contains(linear));
        let replaced = PROBE_WITH_LAB.replace(linear, recursive);
        let catalog = ContractCatalog::try_m0().unwrap();
        let original = compile(&[PROBE_WITH_LAB], &catalog).unwrap();
        let changed = compile(&[&replaced], &catalog).unwrap();
        assert_eq!(original.hash, changed.hash);
        assert_eq!(original.plan.plan(), changed.plan.plan());
        let original_runs = run(&original.plan, GraphVersion(1), &original.labs[0]);
        let changed_runs = run(&changed.plan, GraphVersion(1), &changed.labs[0]);
        assert!(changed_runs.iter().all(|run| run.report.passed()));
        assert_eq!(original_runs, changed_runs);
    }

    #[test]
    fn the_lab_runs_its_scenarios_green() {
        let reports = reports();
        assert_eq!(reports.len(), 3);
        for report in &reports {
            assert!(
                report.passed(),
                "{}.{} failed: {:#?}",
                report.simulation,
                report.scenario,
                report.failures
            );
        }
    }

    #[test]
    fn the_lab_is_repeatable() {
        assert_eq!(reports(), reports());
    }

    #[test]
    fn transaction_stepping_and_batch_execution_produce_one_scenario_history() {
        let compilation = compile(
            &[PROBE_WITH_LAB],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("a valid unit");
        let lab = &compilation.labs[0];
        let batched = run(&compilation.plan, GraphVersion(1), lab);
        let simulation = &lab.simulations[0];

        for (scenario, expected) in simulation.scenarios.iter().zip(batched) {
            let mut interactive = Simulator::try_new(
                &compilation.plan,
                GraphVersion(1),
                &simulation.name,
                &simulation.mocks,
                scenario,
                None,
            )
            .expect("the scripted world binds");
            while !interactive.is_complete() {
                if interactive.is_quiescent() {
                    interactive
                        .next_action()
                        .expect("the next authored action applies");
                }
                while !interactive.is_quiescent() {
                    interactive
                        .step_transaction()
                        .expect("one interactive transaction runs");
                }
            }
            assert_eq!(
                interactive.finish(),
                expected,
                "scenario `{}`",
                scenario.name
            );
        }
    }

    #[test]
    fn an_edited_injection_is_validated_before_admission_and_never_mutates_the_scenario() {
        let compilation = compile(
            &[PROBE_WITH_LAB],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("a valid unit");
        let simulation = &compilation.labs[0].simulations[0];
        let scenario = &simulation.scenarios[0];
        let new_session = || {
            Simulator::try_new(
                &compilation.plan,
                GraphVersion(1),
                &simulation.name,
                &simulation.mocks,
                scenario,
                None,
            )
            .expect("the scripted world binds")
        };
        let mut session = new_session();
        let authored = session
            .next_injection_draft()
            .expect("the draft evaluates")
            .expect("the first action injects");
        assert!(!authored.edited);
        assert!(
            session
                .replace_next_injection_data(Value::Text(String::from("wrong shape")))
                .is_err(),
            "the resolved struct type is the edit authority"
        );
        session
            .replace_next_injection_data(Value::Struct(vec![(
                String::from("text"),
                Value::Text(String::from("edited")),
            )]))
            .expect("the typed replacement conforms");
        assert!(
            session
                .next_injection_draft()
                .expect("the edited draft projects")
                .expect("the injection remains pending")
                .edited
        );
        assert_eq!(session.next_action(), Ok(Some(ScenarioActionKind::Inject)));
        assert!(matches!(
            session.execution().envelope_views().first().map(|observed| observed.envelope.data()),
            Some(Data::Materialized(Value::Struct(fields)))
                if fields == &vec![(String::from("text"), Value::Text(String::from("edited")))]
        ));

        let reset = new_session();
        let reset = reset
            .next_injection_draft()
            .expect("the authored draft still evaluates")
            .expect("the first action still injects");
        assert!(!reset.edited, "a fresh world restores authored DATA");
    }

    #[test]
    fn every_h2_network_family_binds_through_its_lab_twin() {
        let sources = [
            r#"
graph network_call : App {
  connector network : HttpClient {
    url = "https://example.test/v1"
    timeout = duration("5s")
    codec = json
    idempotent = true; behavior = deterministic
  }
  in -> network -> out
  network.exit -> nothing
}
"#,
            r#"
graph http_source : App {
  connector network : HttpServer {
    listen = "127.0.0.1:8080"
    codec = json
  }
  network -> out
  network.exit -> nothing
}
"#,
            r#"
graph websocket_client_source : App {
  connector network : WebSocketClient {
    url = "wss://example.test/socket"
    handshake_timeout = duration("5s")
    codec = json
     behavior = deterministic
  }
  network -> out
  network.exit -> nothing
}
"#,
            r#"
graph websocket_server_source : App {
  connector network : WebSocketServer {
    listen = "127.0.0.1:8081"
    handshake_timeout = duration("5s")
    codec = json
  }
  network -> out
  network.exit -> nothing
}
"#,
        ];
        for source in sources {
            let compilation = compile(
                &[source],
                &ContractCatalog::try_m0().expect("the standard catalogue holds"),
            )
            .unwrap_or_else(|bag| panic!("the H2 source and Lab must check: {bag:#?}"));
            mock_bindings(
                &compilation.plan,
                BTreeMap::new(),
                &crate::deployment::BootBindings::empty(),
                Rc::new(RefCell::new(HttpServerLabInput::default())),
            )
            .unwrap_or_else(|error| panic!("the family-owned Lab twin must bind: {error}"));
        }
    }

    #[test]
    fn the_http_client_lab_twin_runs_after_the_same_pre_open_framing() {
        let source = r#"
graph h2_http : App {
  connector client : HttpClient {
    url = "https://example.test/v1"
    method = "POST"
    timeout = duration("5s")
    codec = json
    idempotent = true; behavior = deterministic
  }
  in -> client -> out
  client.exit -> nothing
}

lab h2_http {
  simulation sheet on app {
    mock client {
      on in {
        out = .
        exit = Completed { status = 200 }
      }
    }
    scenario call "the network is replaced, not the contract" {
      app.in = { request = true }
      expect "the response is ordinary data" app.out count == 1
      expect "one HTTP terminal closes the call" client.exit where data is Completed
    }
  }
}
"#;
        let compilation = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap_or_else(|bag| panic!("the H2 HttpClient Lab must check: {bag:#?}"));
        let reports = run(&compilation.plan, GraphVersion(1), &compilation.labs[0]);
        assert_eq!(reports.len(), 1);
        assert!(
            reports[0].report.passed(),
            "the H2 Lab scenario failed: {:#?}",
            reports[0].report.failures
        );
    }

    #[test]
    fn an_emission_is_observable_at_its_source_port() {
        // `expect relay.out` watches departures — the fifth review
        // showed a real emission invisible to its own port.
        let source = PROBE_WITH_LAB.replace(
            "expect \"the echo came back\" app.out count == 1 where data == \"salut\"",
            "expect \"the departure\" shout.out count == 1 where data == \"salut\"",
        );
        let compilation = compile(
            &[source.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("a valid unit");
        let runs = run(&compilation.plan, GraphVersion(1), &compilation.labs[0]);
        assert!(
            runs[0].report.passed(),
            "the source port sees its own emission: {:#?}",
            runs[0].report.failures
        );
    }

    #[test]
    fn a_faulting_predicate_fails_the_expectation_loudly() {
        // Build an explicit failed narrowing: dynamic structure cannot be
        // inspected without a type, and a failed `as` is the runtime fault
        // this expectation is meant to surface.
        let source = PROBE_WITH_LAB.replace(
            "expect \"the echo came back\" app.out count == 1 where data == \"salut\"",
            "expect \"the echo came back\" app.out count == 1 where data as bool",
        );
        let compilation = compile(
            &[source.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("a valid unit");
        let runs = run(&compilation.plan, GraphVersion(1), &compilation.labs[0]);
        assert!(!runs[0].report.passed());
        assert!(
            runs[0].report.failures[0].contains("faulted"),
            "the fault is named, never a silent non-match: {:#?}",
            runs[0].report.failures
        );
    }

    #[test]
    fn a_broken_expectation_fails_loudly_not_silently() {
        let source = PROBE_WITH_LAB.replace(
            "expect \"the echo came back\" app.out count == 1 where data == \"salut\"",
            "expect \"the echo came back\" app.out count == 1 where data == \"nope\"",
        );
        let compilation = compile(
            &[source.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("a valid unit");
        let runs = run(&compilation.plan, GraphVersion(1), &compilation.labs[0]);
        let roundtrip = &runs[0].report;
        assert!(!roundtrip.passed());
        assert!(roundtrip.failures[0].contains("the echo came back"));
    }

    #[test]
    fn a_declared_int_overflow_faults_the_scenario_without_an_implicit_err() {
        // The fifth review's repro: count = i32::MAX, then + 1 —
        // previously widened silently and sealed as a long.
        let source = r#"
graph m0_probe : App {
  operator wide {
    input in: struct { text: text }
    var count : int = 2147483647
    on in where data.text != "" {
      count = count + 1
      out = data.text
    }
  }
  in -> wide -> out
}

lab m0_probe {
  simulation s on app {
    scenario overflow {
      app.in = { text = "x" }
      expect nothing app.out
    }
  }
}
"#;
        let compilation = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("a valid unit");
        let runs = run(&compilation.plan, GraphVersion(1), &compilation.labs[0]);
        assert!(!runs[0].report.passed());
        assert!(
            runs[0]
                .report
                .failures
                .iter()
                .any(|failure| failure.contains("a step faulted")),
            "the engine fault must reach scenario supervision: {:#?}",
            runs[0].report.failures
        );
    }

    #[test]
    fn an_uncovered_call_fails_the_scenario_loudly() {
        let source = r#"
graph m0_probe : App {
  connector echo : Process {
    mode = job ; command = "cat"
    codec = lines
    idempotent = true; behavior = deterministic
  }

  operator shout {
    input in: struct { text: text }
    on in { out = data.text }
  }

  in -> shout -> echo -> out
  echo.err -> nothing
  echo.exit -> nothing
}

lab m0_probe {
  simulation smoke on app {
    scenario unmocked {
      app.in = { text = "salut" }
    }
  }
}
"#;
        let compilation = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("a valid unit");
        let runs = run(&compilation.plan, GraphVersion(1), &compilation.labs[0]);
        assert!(!runs[0].report.passed());
        assert!(runs[0].report.failures[0].contains("uncovered call"));
    }
}
