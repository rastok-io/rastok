//! The scenario runner — the graph's possible worlds, executed.
//!
//! A scenario is the world's half of a journal, written in advance:
//! injections play the ingress, mocks play the outbound drivers, `advance`
//! plays time. Everything runs on the same [`Vm`] a production run
//! uses — the mock replaces the driver, never the bookkeeping — so
//! a green scenario is a statement about the machine, not about a
//! test double.

use crate::boundary::{
    CallConnectorDriver, CallStart, Completion, Draft, InboundEnvelopeProvenance,
    InboundSubmission, Intent, OutboundConnectorDriver, PortOutput, RequestPreparation, Response,
    SchedulerCompletion, SubmissionSequence,
};
use crate::compiler::hir::{
    Cmp, EmitValue, ExpectTarget, Expectation, Lab, Mock, Quantifier, Scenario, ScenarioStep,
};
use crate::connector::{ConnectorId, ConnectorRoles};
#[cfg(test)]
use crate::connectors::ProcessRequestBindings;
use crate::connectors::{
    BoundConnectorDriver, ConnectorDriverFactory, DriverFactoryError, DriverRegistry,
    RuntimeBindings, ScriptedConnectorProtocol, ScriptedProtocolBinder, passive_lab_factories,
    scripted_protocol_binders,
};
use crate::envelope::{Data, Envelope, Namespace, Value, WriteTarget};
use crate::journal::{MemJournal, Record};
use crate::plan::{GraphVersion, ValidatedExecutionPlan};
use crate::runtime::vm::{Observed, Vm};
use crate::{expr, node, runtime};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

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

/// Runs every scenario of a checked lab against its plan. Each
/// scenario gets a fresh journal and a fresh machine — scenarios
/// share nothing but the graph.
#[must_use]
pub fn run(
    plan: &ValidatedExecutionPlan,
    graph_version: GraphVersion,
    lab: &Lab,
) -> Vec<ScenarioRun> {
    let mut reports = Vec::new();
    for simulation in &lab.simulations {
        for scenario in &simulation.scenarios {
            reports.push(run_scenario(
                plan,
                graph_version,
                &simulation.name,
                &simulation.mocks,
                scenario,
            ));
        }
    }
    reports
}

fn run_scenario(
    plan: &ValidatedExecutionPlan,
    graph_version: GraphVersion,
    simulation: &str,
    shared_mocks: &[Mock],
    scenario: &Scenario,
) -> ScenarioRun {
    // Nearest wins, whole-mock replacement: a scenario mock for a
    // connector shadows the simulation-level one entirely.
    let mut mocks: BTreeMap<node::Path, Mock> = BTreeMap::new();
    for mock in shared_mocks {
        mocks.insert(mock.connector.clone(), mock.clone());
    }
    for mock in &scenario.mocks {
        mocks.insert(mock.connector.clone(), mock.clone());
    }
    let mut journal = MemJournal::new(graph_version);
    let mut vm = Vm::boot(plan, graph_version);
    let mut failures = Vec::new();
    let (mut driver, faults) = match mock_bindings(plan, mocks) {
        Ok(bound) => bound,
        Err(error) => {
            failures.push(format!("the Lab Driver composition refused: {error}"));
            return ScenarioRun {
                report: ScenarioReport {
                    simulation: String::from(simulation),
                    scenario: scenario.name.clone(),
                    label: scenario.label.clone(),
                    failures,
                },
                records: Vec::new(),
            };
        }
    };
    let mut window_start = 0usize;
    for step in &scenario.steps {
        match step {
            ScenarioStep::Inject { port, value, seq } => {
                window_start = vm.observations().len();
                match draft(value) {
                    Ok(draft) => {
                        let submission = InboundSubmission {
                            provenance: InboundEnvelopeProvenance::single(
                                node::Path::new(["lab"])
                                    .unwrap_or_else(|_| unreachable!("`lab` is a valid path")),
                            ),
                            port: port.clone(),
                            draft,
                            sequence: match seq {
                                Some(seq) => SubmissionSequence::Explicit(*seq),
                                None => SubmissionSequence::Next,
                            },
                        };
                        if let Err(error) = vm.inject(&mut journal, submission) {
                            failures.push(format!("the injection was refused: {error}"));
                            continue;
                        }
                        settle(&mut vm, &mut journal, &mut driver, &faults, &mut failures);
                    }
                    Err(message) => failures.push(message),
                }
            }
            ScenarioStep::Advance { nanos } => {
                window_start = vm.observations().len();
                let target = vm.now().saturating_add(*nanos);
                if let Err(error) = vm.advance(&mut journal, &mut driver, target) {
                    failures.push(format!("the advance faulted: {error}"));
                }
                settle(&mut vm, &mut journal, &mut driver, &faults, &mut failures);
            }
            ScenarioStep::Expect(expectation) => {
                if let Some(message) =
                    check_expectation(expectation, &vm.observations()[window_start..])
                {
                    failures.push(message);
                }
            }
        }
    }
    ScenarioRun {
        report: ScenarioReport {
            simulation: String::from(simulation),
            scenario: scenario.name.clone(),
            label: scenario.label.clone(),
            failures,
        },
        records: journal.records().to_vec(),
    }
}

/// Drives the machine to rest after an action step: drain, let the
/// mocks answer, drain again — until nothing settles. Uncovered
/// calls and mock faults surface as failures.
fn settle(
    vm: &mut Vm<'_>,
    journal: &mut MemJournal,
    driver: &mut RuntimeBindings<'_>,
    faults: &MockFaults,
    failures: &mut Vec<String>,
) {
    let mut rested = false;
    for _ in 0..64 {
        if let Err(error) = vm.drain_with_bindings(journal, driver) {
            failures.push(format!("a step faulted: {error}"));
            return;
        }
        let report = match vm.pump(journal, driver) {
            Ok(report) => report,
            Err(error) => {
                failures.push(format!("the boundary round faulted: {error}"));
                return;
            }
        };
        failures.extend(faults.borrow_mut().drain(..));
        for (response, reason) in &report.rejected {
            failures.push(format!(
                "a reply to {:?} was refused before closure: {reason}",
                response.call_ref
            ));
        }
        if report.settled == 0 {
            rested = true;
            break;
        }
    }
    if !rested {
        // Never a silent truncation: a machine still settling after
        // the cap is a failed scenario, not a shrugged one.
        failures.push(String::from(
            "the machine did not settle within 64 boundary rounds",
        ));
    }
    if let Err(error) = vm.drain_with_bindings(journal, driver) {
        failures.push(format!("a step faulted: {error}"));
    }
}

fn check_expectation(expectation: &Expectation, window: &[Observed]) -> Option<String> {
    let deliveries: Vec<&Observed> = window
        .iter()
        .filter(|observed| match &expectation.target {
            ExpectTarget::RootSink(port) => observed.node.is_none() && observed.port == *port,
            ExpectTarget::Boundary(port) => observed.node.is_none() && observed.port == *port,
            ExpectTarget::Node { path, port } => {
                observed.node.as_ref() == Some(path) && observed.port == *port
            }
        })
        .collect();
    let name = expectation
        .label
        .clone()
        .unwrap_or_else(|| format!("{:?}", expectation.target));
    let mut matches = 0usize;
    for observed in &deliveries {
        match &expectation.predicate {
            None => matches += 1,
            Some(tree) => {
                let mut env = BTreeMap::new();
                match runtime::data_value(observed.envelope.data()) {
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
                    expr::EvalValue::from(runtime::meta_value(observed.envelope.meta())),
                );
                // A faulting or non-boolean predicate fails the
                // expectation itself — "did not match" would be a
                // false green with the void quantifier.
                match expr::eval(tree, &env) {
                    Ok(expr::EvalValue::Bool(true)) => matches += 1,
                    Ok(expr::EvalValue::Bool(false)) => {}
                    Ok(other) => {
                        return Some(format!(
                            "expect {name}: the predicate is not a bool (got {other:?})"
                        ));
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
            window.len()
        ))
    }
}

/// Builds a world-authored draft from an injection value: `data`
/// writes shape the payload, `meta.app.*` writes fill the writable
/// namespace. There is no input — a draft inherits nothing.
fn draft(value: &EmitValue) -> Result<Draft, String> {
    let empty = BTreeMap::new();
    match value {
        EmitValue::Data(tree) => {
            let value =
                expr::eval(tree, &empty).map_err(|error| format!("the injection: {error}"))?;
            Ok(Draft {
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
            Ok(Draft {
                data: Data::materialized(data),
                ext,
            })
        }
        EmitValue::Nothing => Err(String::from("`nothing` injects nothing")),
        EmitValue::Identity => Err(String::from("a draft has no input to re-emit")),
    }
}

struct MockConnectorFactory {
    id: ConnectorId,
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

impl ConnectorDriverFactory for MockConnectorFactory {
    fn connector_id(&self) -> &ConnectorId {
        &self.id
    }

    fn roles(&self) -> ConnectorRoles {
        ConnectorRoles::Outbound
    }

    fn bind<'plan>(
        &self,
        plan: &'plan ValidatedExecutionPlan,
        nodes: &[node::Path],
    ) -> Result<Box<dyn BoundConnectorDriver + 'plan>, DriverFactoryError> {
        let protocol = (self.bind_protocol)(plan, nodes)?;
        Ok(Box::new(MockOutboundConnectorDriver {
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
) -> Result<(RuntimeBindings<'plan>, MockFaults), String> {
    let faults = Rc::new(RefCell::new(Vec::new()));
    let mocks = share_mocks(mocks);
    let mut registry = DriverRegistry::default();
    for (spelling, bind_protocol) in scripted_protocol_binders() {
        let id = ConnectorId::parse(spelling).map_err(|error| error.to_string())?;
        registry = registry
            .try_with(Box::new(MockConnectorFactory {
                id,
                bind_protocol,
                mocks: Rc::clone(&mocks),
                faults: Rc::clone(&faults),
            }))
            .map_err(|error| error.to_string())?;
    }
    for factory in passive_lab_factories().map_err(|error| error.to_string())? {
        registry = registry
            .try_with(factory)
            .map_err(|error| error.to_string())?;
    }
    let bindings = RuntimeBindings::try_new(plan, &registry).map_err(|error| error.to_string())?;
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
struct MockOutboundConnectorDriver {
    mocks: SharedMocks,
    protocol: Box<dyn ScriptedConnectorProtocol>,
    pending: Vec<Response>,
    faults: MockFaults,
}

impl MockOutboundConnectorDriver {
    fn fault(&self, message: String) {
        self.faults.borrow_mut().push(message);
    }

    fn answer(&mut self, intent: &Intent, request: &Envelope) -> Option<Completion> {
        let Some(mock) = self.mocks.get(&intent.connector_path).cloned() else {
            self.fault(format!(
                "an uncovered call reached `{}` (digest {:02x?}…); mock it or script silence",
                intent.connector_path,
                &intent.request_digest()[..4]
            ));
            return None;
        };
        // The request materializes on the EVALUATOR'S demand, like
        // the machine's own bindings: an identity reply, a meta-only
        // guard, a losing later branch, or a statement past the
        // reply must never force an opaque request to decode. The
        // slot caches across branches — one decode at most.
        let mut env: BTreeMap<String, expr::EvalValue> = BTreeMap::new();
        env.insert(
            String::from("now"),
            expr::EvalValue::Timestamp(intent.logical_time),
        );
        env.insert(
            String::from("meta"),
            expr::EvalValue::from(runtime::meta_value(request.meta())),
        );
        let slot = runtime::DataSlot::new(request.data());
        for handler in &mock.handlers {
            let passes = match &handler.guard {
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
                        Ok(other) => {
                            self.fault(format!(
                                "a mock guard of `{}` is not a bool (got {other:?})",
                                intent.connector_path
                            ));
                            return None;
                        }
                        Err(error) => {
                            self.fault(format!(
                                "a mock guard of `{}` faulted: {error}",
                                intent.connector_path
                            ));
                            return None;
                        }
                    }
                }
            };
            if !passes {
                continue;
            }
            return self.reply(handler, request, env, &slot);
        }
        self.fault(format!(
            "no mock branch of `{}` matched the request; add a catch-all or script silence",
            intent.connector_path
        ));
        None
    }

    /// Evaluates the first matching branch. `out = nothing` is the
    /// written silence — the call stays open for the timeout
    /// machinery.
    fn reply(
        &mut self,
        handler: &crate::compiler::hir::Handler,
        request: &Envelope,
        mut env: BTreeMap<String, expr::EvalValue>,
        slot: &runtime::DataSlot<'_>,
    ) -> Option<Completion> {
        use crate::compiler::hir::Statement;
        let mut outputs = Vec::new();
        for statement in &handler.body {
            match statement {
                Statement::Let { name, value } => match self.lazy_eval(value, &env, slot) {
                    Ok(value) => {
                        env.insert(name.clone(), value);
                    }
                    Err(message) => {
                        self.fault(message);
                        return None;
                    }
                },
                Statement::SetVar { .. } => {
                    // The checker refuses vars on a mock's synthetic
                    // node; nothing reaches here.
                }
                Statement::Emit { port, value } => {
                    match self.mock_data(value, request, &env, slot) {
                        Ok(Some(data)) => outputs.push(PortOutput {
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
            }
        }
        if outputs.is_empty() {
            return None;
        }
        match Completion::try_new(outputs) {
            Ok(completion) => Some(completion),
            Err(error) => {
                self.fault(format!("the mock completion is invalid: {error}"));
                None
            }
        }
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
                        // minted by the connector's own transaction.
                        other => {
                            return Err(format!(
                                "the mock faulted: a reply cannot write {other:?}"
                            ));
                        }
                    }
                }
                Ok(Some(Data::materialized(data)))
            }
        }
    }
}

impl CallConnectorDriver for MockOutboundConnectorDriver {
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

    fn start(&mut self, intent: &Intent, request: &Envelope) -> CallStart {
        if let Some(completion) = self.answer(intent, request) {
            self.pending.push(Response {
                call_ref: intent.call_ref,
                completion,
            });
        }
        CallStart::Started
    }

    fn events(&mut self) -> Vec<Response> {
        std::mem::take(&mut self.pending)
    }
}

impl BoundConnectorDriver for MockOutboundConnectorDriver {
    fn inbound(&mut self) -> Option<&mut dyn crate::boundary::InboundConnectorDriver> {
        None
    }

    fn outbound(&mut self) -> Option<&mut dyn OutboundConnectorDriver> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::compile;
    use crate::contract::ContractCatalog;

    /// Drives the scripted driver directly with an OPAQUE request:
    /// the branch and statement structure decides the demand, so a
    /// pre-scan of every handler would fail both cases below.
    fn opaque_answer(mock_body: &str) -> (Option<Completion>, Vec<String>) {
        let source = format!(
            "graph g : App {{\n  connector echo : Process {{\n    mode = job ; command = \"cat\"\n    codec = lines\n    effect = idempotent ; behavior = deterministic\n  }}\n  . -> echo -> .\n}}\n\nlab g {{\n  simulation probing on app {{\n    mock echo {{\n{mock_body}\n    }}\n    scenario s \"unused\" {{\n      app.in = {{v = 1}}\n    }}\n  }}\n}}\n"
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
            mocks: share_mocks(mocks),
            protocol: Box::new(
                ProcessRequestBindings::try_bind(&compilation.plan, &nodes)
                    .expect("the Process mock binding is valid"),
            ),
            pending: Vec::new(),
            faults: Rc::clone(&faults),
        };
        let id = crate::envelope::Id::derive(0x01, &[b"request", &0u64.to_le_bytes()]);
        let request = Envelope::new(
            crate::envelope::Meta::new(crate::envelope::MetaSys::root(id, 1)),
            Data::encoded(crate::envelope::Encoding::Avro, *b"\x01\x02"),
        );
        let prepared = crate::boundary::PreparedRequest::seal(vec![1]);
        let intent = crate::boundary::Intent {
            call_ref: crate::journal::ids::call_ref(
                crate::journal::ids::delivery_ref(".in>echo.in", id),
                prepared.digest(),
            ),
            emitting_id: id,
            edge: String::from(".in>echo.in"),
            connector_path: path,
            prepared,
            idempotency_key: None,
            effect: node::EffectClass::Idempotent,
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
                    Data::Encoded { bytes, .. } if bytes.as_slice() == b"\x01\x02"
                )),
                "`{body}` must relay the representation"
            );
        }
    }

    #[test]
    fn a_mock_evaluates_the_whole_atomic_completion() {
        // An emission no longer returns from a mock branch: all drafts are
        // built before one atomic completion is handed to the dispatcher.
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
    effect = idempotent ; behavior = deterministic
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

  . -> shout -> echo -> .
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
      expect nothing app.err
    }

    scenario silence_expires "a silent driver times out" {
      mock echo {
        on in { out = nothing }
      }
      app.in = { text = "salut" }
      expect nothing app.out
      advance duration("5s")
      expect "the budget expired" app.exit where data is TimeoutExpired
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
    fn every_h2_network_family_binds_through_its_lab_twin() {
        let sources = [
            r#"
graph network_call : App {
  connector network : HttpClient {
    url = "https://example.test/v1"
    timeout = duration("5s")
    codec = json
    effect = idempotent ; behavior = deterministic
  }
  . -> network -> .
}
"#,
            r#"
graph http_source : App {
  connector network : HttpServer {
    listen = "127.0.0.1:8080"
    codec = json
  }
  network -> .
}
"#,
            r#"
graph websocket_client_source : App {
  connector network : WebSocketClient {
    url = "wss://example.test/socket"
    handshake_timeout = duration("5s")
    codec = json
    effect = idempotent ; behavior = deterministic
  }
  network -> .
}
"#,
            r#"
graph websocket_server_source : App {
  connector network : WebSocketServer {
    listen = "127.0.0.1:8081"
    handshake_timeout = duration("5s")
    codec = json
  }
  network -> .
}
"#,
        ];
        for source in sources {
            let compilation = compile(
                &[source],
                &ContractCatalog::try_m0().expect("the standard catalogue holds"),
            )
            .unwrap_or_else(|bag| panic!("the H2 source and Lab must check: {bag:#?}"));
            mock_bindings(&compilation.plan, BTreeMap::new())
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
    effect = idempotent ; behavior = deterministic
  }
  . -> client -> .
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
      expect "one HTTP terminal closes the call" app.exit where data is Completed
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
  . -> wide -> .
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
    effect = idempotent ; behavior = deterministic
  }

  operator shout {
    input in: struct { text: text }
    on in { out = data.text }
  }

  . -> shout -> echo -> .
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
