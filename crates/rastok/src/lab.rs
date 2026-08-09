//! The scenario runner — the graph's possible worlds, executed.
//!
//! A scenario is the world's half of a journal, written in advance:
//! injections play the ingress, mocks play the partners, `advance`
//! plays time. Everything runs on the same [`Vm`] a production run
//! uses — the mock replaces the partner, never the bookkeeping — so
//! a green scenario is a statement about the machine, not about a
//! test double.

use crate::boundary::{
    CallOutcome, Draft, InboundSubmission, IngressAuthority, Intent, Partner, Response,
    SubmissionSequence,
};
use crate::compiler::hir::{
    Cmp, EmitValue, ExpectTarget, Expectation, Lab, Mock, Quantifier, Scenario, ScenarioStep,
};
use crate::envelope::{Data, Envelope, Namespace, WriteTarget};
use crate::journal::{MemJournal, Record};
use crate::plan::{GraphVersion, ValidatedExecutionPlan};
use crate::runtime::vm::{Observed, Vm};
use crate::{cel, node, runtime};
use std::collections::BTreeMap;

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
    let mut mocks: BTreeMap<node::Path, &Mock> = BTreeMap::new();
    for mock in shared_mocks {
        mocks.insert(mock.connector.clone(), mock);
    }
    for mock in &scenario.mocks {
        mocks.insert(mock.connector.clone(), mock);
    }
    let mut journal = MemJournal::new(graph_version);
    let mut vm = Vm::boot(plan, graph_version);
    let mut partner = MockPartner {
        mocks,
        pending: Vec::new(),
        faults: Vec::new(),
    };
    let mut failures = Vec::new();
    let mut window_start = 0usize;
    for step in &scenario.steps {
        match step {
            ScenarioStep::Inject { port, value, seq } => {
                window_start = vm.observations().len();
                match draft(value) {
                    Ok(draft) => {
                        let submission = InboundSubmission {
                            authority: IngressAuthority::single(
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
                        settle(&mut vm, &mut journal, &mut partner, &mut failures);
                    }
                    Err(message) => failures.push(message),
                }
            }
            ScenarioStep::Advance { nanos } => {
                window_start = vm.observations().len();
                let target = vm.now().saturating_add(*nanos);
                if let Err(error) = vm.advance(&mut journal, &mut partner, target) {
                    failures.push(format!("the advance faulted: {error}"));
                }
                settle(&mut vm, &mut journal, &mut partner, &mut failures);
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
    partner: &mut MockPartner<'_>,
    failures: &mut Vec<String>,
) {
    let mut rested = false;
    for _ in 0..64 {
        if let Err(error) = vm.drain(journal) {
            failures.push(format!("a step faulted: {error}"));
            return;
        }
        let report = match vm.pump(journal, partner) {
            Ok(report) => report,
            Err(error) => {
                failures.push(format!("the boundary round faulted: {error}"));
                return;
            }
        };
        failures.append(&mut partner.faults);
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
    if let Err(error) = vm.drain(journal) {
        failures.push(format!("a step faulted: {error}"));
    }
}

fn check_expectation(expectation: &Expectation, window: &[Observed]) -> Option<String> {
    let deliveries: Vec<&Observed> = window
        .iter()
        .filter(|observed| match &expectation.target {
            ExpectTarget::RootErr => observed.node.is_none() && observed.port.name() == "err",
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
                        env.insert(String::from("data"), data);
                    }
                    Err(fault) => {
                        return Some(format!(
                            "expect {name}: a delivery's data is unreadable: {fault}"
                        ));
                    }
                }
                env.insert(
                    String::from("meta"),
                    runtime::meta_value(observed.envelope.meta()),
                );
                // A faulting or non-boolean predicate fails the
                // expectation itself — "did not match" would be a
                // false green with the void quantifier.
                match cel::eval(tree, &env) {
                    Ok(cel::Value::Bool(true)) => matches += 1,
                    Ok(cel::Value::Bool(false)) => {}
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
                cel::eval(tree, &empty).map_err(|error| format!("the injection: {error}"))?;
            Ok(Draft {
                data: Data::materialized(
                    runtime::to_envelope_value(&value)
                        .map_err(|fault| format!("the injection: {fault}"))?,
                ),
                mime_type: None,
                app: Namespace::new(),
            })
        }
        EmitValue::Record(writes) | EmitValue::Patch(writes) => {
            // A draft is born a fresh STRUCT — the same physical form
            // the handler builders produce (`{…}` builds a struct).
            let mut data = crate::envelope::Value::Struct(Vec::new());
            let mut app = Namespace::new();
            let mut mime = None;
            for write in writes {
                let value = cel::eval(&write.value, &empty)
                    .map_err(|error| format!("the injection: {error}"))?;
                match &write.target {
                    WriteTarget::Data(path) => {
                        let leaf = runtime::to_envelope_value(&value)
                            .map_err(|fault| format!("the injection: {fault}"))?;
                        runtime::apply_struct_write(&mut data, path, leaf);
                    }
                    WriteTarget::AppMeta(field) => {
                        app.insert(
                            field.clone(),
                            runtime::to_meta_value(&value)
                                .map_err(|fault| format!("the injection: {fault}"))?,
                        );
                    }
                    WriteTarget::MimeType => match value {
                        cel::Value::Text(text) => mime = Some(text),
                        other => {
                            return Err(format!(
                                "the injection: mime_type takes text, got {other:?}"
                            ));
                        }
                    },
                    WriteTarget::CustomMeta(..) => {
                        return Err(String::from(
                            "the injection: a draft has no custom namespaces yet",
                        ));
                    }
                }
            }
            Ok(Draft {
                data: Data::materialized(data),
                mime_type: mime,
                app,
            })
        }
        EmitValue::Nothing => Err(String::from("`nothing` injects nothing")),
        EmitValue::Identity => Err(String::from("a draft has no input to re-emit")),
    }
}

/// The scripted partner: answers journaled intents from mock
/// branches — first match in text order — and reports every
/// uncovered call loudly. Deterministic and stateless, a pure
/// function of the request.
struct MockPartner<'lab> {
    mocks: BTreeMap<node::Path, &'lab Mock>,
    pending: Vec<Response>,
    faults: Vec<String>,
}

impl MockPartner<'_> {
    fn answer(&mut self, intent: &Intent, request: &Envelope) -> Option<CallOutcome> {
        let Some(mock) = self.mocks.get(&intent.connector_path) else {
            self.faults.push(format!(
                "an uncovered call reached `{}` (digest {:02x?}…); mock it or script silence",
                intent.connector_path,
                &intent.request_digest()[..4]
            ));
            return None;
        };
        // The request materializes ON DEMAND, like the machine's own
        // bindings: an identity reply or a meta-only guard must
        // never force an opaque request to decode.
        use crate::compiler::hir::Statement;
        let needs_data = mock.handlers.iter().any(|handler| {
            handler.guard.as_ref().is_some_and(runtime::reads_data)
                || handler.body.iter().any(|statement| match statement {
                    Statement::Emit { value, .. } => match value {
                        EmitValue::Identity | EmitValue::Nothing => false,
                        EmitValue::Data(tree) => runtime::reads_data(tree),
                        EmitValue::Patch(writes) | EmitValue::Record(writes) => {
                            writes.iter().any(|write| runtime::reads_data(&write.value))
                        }
                    },
                    Statement::Let { value, .. } => runtime::reads_data(value),
                    Statement::SetVar { value, .. } => runtime::reads_data(value),
                })
        });
        let mut env: BTreeMap<String, cel::Value> = BTreeMap::new();
        if needs_data {
            match runtime::data_value(request.data()) {
                Ok(data) => {
                    env.insert(String::from("data"), data);
                }
                Err(fault) => {
                    self.faults
                        .push(format!("the mock request is unreadable: {fault}"));
                    return None;
                }
            }
        }
        env.insert(String::from("meta"), runtime::meta_value(request.meta()));
        for handler in &mock.handlers {
            let passes = match &handler.guard {
                None => true,
                Some(guard) => match cel::eval(guard, &env) {
                    Ok(cel::Value::Bool(passes)) => passes,
                    // A faulting or non-boolean guard is a fault of
                    // the scenario, never a silent fall-through to
                    // the next branch.
                    Ok(other) => {
                        self.faults.push(format!(
                            "a mock guard of `{}` is not a bool (got {other:?})",
                            intent.connector_path
                        ));
                        return None;
                    }
                    Err(error) => {
                        self.faults.push(format!(
                            "a mock guard of `{}` faulted: {error}",
                            intent.connector_path
                        ));
                        return None;
                    }
                },
            };
            if !passes {
                continue;
            }
            return self.reply(handler, request, env);
        }
        self.faults.push(format!(
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
        mut env: BTreeMap<String, cel::Value>,
    ) -> Option<CallOutcome> {
        use crate::compiler::hir::Statement;
        for statement in &handler.body {
            match statement {
                Statement::Let { name, value } => match cel::eval(value, &env) {
                    Ok(value) => {
                        env.insert(name.clone(), value);
                    }
                    Err(error) => {
                        self.faults.push(format!("the mock faulted: {error}"));
                        return None;
                    }
                },
                Statement::SetVar { .. } => {
                    // The checker refuses vars on a mock's synthetic
                    // node; nothing reaches here.
                }
                Statement::Emit { value, .. } => {
                    return match self.mock_data(value, request, &env) {
                        Ok(Some(data)) => Some(CallOutcome::Success(data)),
                        Ok(None) => None,
                        Err(message) => {
                            self.faults.push(message);
                            None
                        }
                    };
                }
            }
        }
        None
    }

    fn mock_data(
        &self,
        value: &EmitValue,
        request: &Envelope,
        env: &BTreeMap<String, cel::Value>,
    ) -> Result<Option<Data>, String> {
        match value {
            EmitValue::Nothing => Ok(None),
            EmitValue::Identity => Ok(Some(request.data().clone())),
            EmitValue::Data(tree) => {
                let value =
                    cel::eval(tree, env).map_err(|error| format!("the mock faulted: {error}"))?;
                Ok(Some(Data::materialized(
                    runtime::to_envelope_value(&value)
                        .map_err(|fault| format!("the mock faulted: {fault}"))?,
                )))
            }
            EmitValue::Record(writes) | EmitValue::Patch(writes) => {
                let mut data = if matches!(value, EmitValue::Patch(_)) {
                    match request.data() {
                        Data::Materialized(value) => value.clone(),
                        encoded => runtime::to_envelope_value(
                            &runtime::data_value(encoded)
                                .map_err(|fault| format!("the mock faulted: {fault}"))?,
                        )
                        .map_err(|fault| format!("the mock faulted: {fault}"))?,
                    }
                } else {
                    crate::envelope::Value::Struct(Vec::new())
                };
                for write in writes {
                    let evaluated = cel::eval(&write.value, env)
                        .map_err(|error| format!("the mock faulted: {error}"))?;
                    match &write.target {
                        WriteTarget::Data(path) => {
                            let leaf = runtime::to_envelope_value(&evaluated)
                                .map_err(|fault| format!("the mock faulted: {fault}"))?;
                            runtime::apply_struct_write(&mut data, path, leaf);
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

impl Partner for MockPartner<'_> {
    fn start(&mut self, intent: &Intent, request: &Envelope) {
        if let Some(outcome) = self.answer(intent, request) {
            self.pending.push(Response {
                call_ref: intent.call_ref,
                outcome,
            });
        }
    }

    fn events(&mut self) -> Vec<Response> {
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::compile;
    use crate::stdlib::Registry;

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
        out = { text = "salut", echoed = true }
      }
      on in { out = . }
    }

    scenario roundtrip "one message through a scripted world" {
      app.in = { text = "salut" }
      expect "the echo came back" app.out count == 1 where data.echoed == true
      expect nothing app.err
    }

    scenario silence_expires "a silent partner times out" {
      mock echo {
        on in { out = nothing }
      }
      app.in = { text = "salut" }
      expect nothing app.out
      advance duration("5s")
      expect "the budget expired" app.err where data.kind == "TimeoutExpired"
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
        let compilation = compile(&[PROBE_WITH_LAB], &Registry::m0()).expect("a valid unit");
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
    fn an_emission_is_observable_at_its_source_port() {
        // `expect relay.out` watches departures — the fifth review
        // showed a real emission invisible to its own port.
        let source = PROBE_WITH_LAB.replace(
            "expect \"the echo came back\" app.out count == 1 where data.echoed == true",
            "expect \"the departure\" shout.out count == 1 where data == \"salut\"",
        );
        let compilation = compile(&[source.as_str()], &Registry::m0()).expect("a valid unit");
        let runs = run(&compilation.plan, GraphVersion(1), &compilation.labs[0]);
        assert!(
            runs[0].report.passed(),
            "the source port sees its own emission: {:#?}",
            runs[0].report.failures
        );
    }

    #[test]
    fn a_faulting_predicate_fails_the_expectation_loudly() {
        // `data.echoed` faults on the failure envelope of the mute
        // scenario? No — build it directly: a predicate reading a
        // field the delivery does not carry must fail, not shrug.
        let source = PROBE_WITH_LAB.replace(
            "where data.echoed == true",
            "where data.missing_field == true",
        );
        let compilation = compile(&[source.as_str()], &Registry::m0()).expect("a valid unit");
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
        let source = PROBE_WITH_LAB.replace("data.echoed == true", "data.echoed == false");
        let compilation = compile(&[source.as_str()], &Registry::m0()).expect("a valid unit");
        let runs = run(&compilation.plan, GraphVersion(1), &compilation.labs[0]);
        let roundtrip = &runs[0].report;
        assert!(!roundtrip.passed());
        assert!(roundtrip.failures[0].contains("the echo came back"));
    }

    #[test]
    fn a_declared_int_cannot_silently_become_a_long() {
        // The fifth review's repro: count = i32::MAX, then + 1 —
        // previously widened silently and sealed as a long.
        let source = r#"
graph m0_probe : App {
  operator wide {
    var count : int = 2147483647
    on in {
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
      expect "the overflow is typed" app.err where data.kind == "RuntimeFault"
    }
  }
}
"#;
        let compilation = compile(&[source], &Registry::m0()).expect("a valid unit");
        let runs = run(&compilation.plan, GraphVersion(1), &compilation.labs[0]);
        assert!(
            runs[0].report.passed(),
            "the overflow leaves as a typed error: {:#?}",
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
        let compilation = compile(&[source], &Registry::m0()).expect("a valid unit");
        let runs = run(&compilation.plan, GraphVersion(1), &compilation.labs[0]);
        assert!(!runs[0].report.passed());
        assert!(runs[0].report.failures[0].contains("uncovered call"));
    }
}
