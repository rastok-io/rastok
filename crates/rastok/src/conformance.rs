// The harness drives real Drivers against the wall clock, mapped
// onto the virtual clock — the frontier lints stop at this file for
// the same reason they stop at the Driver layer.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
//! Connector conformance tests for the ten contract cases of `stdlib.md` §1,
//! using registered Drivers and the ordinary runtime bindings.
//!
//! A fixture is prepared test material. Here it contains a small graph, inputs
//! and expected results for a Driver; in the Lab, it supplies the data for a
//! scenario. Driver fixtures also state why any terminal case cannot be tested
//! by this suite.
//!
//! The battery proves sheet shape and exact resolved `exit` type, stable
//! Connector identity and outbound role binding, silence before start,
//! response correlation under interleaved calls, typed completion coverage,
//! journal-before-effect, close-once, scheduler-owned timeout, and the
//! no-late-causal-fact rule after cancellation. Every portable terminal case
//! is either provoked or carries a named reason why this host-independent
//! battery cannot force it. The same registry seam serves native, embedded,
//! statically selected, and future plugin-hosted Drivers.

use crate::compiler::compile;
use crate::connector::contract::ConnectorContract;
use crate::connector::driver::OutboundDrivers;
use crate::connector::driver::SessionConnectorBindings;
use crate::connector::exchange::EnvelopeDraft;
use crate::connector::exchange::IngressAdmission;
use crate::connector::exchange::IngressProposal;
use crate::connector::exchange::IngressProvenance;
use crate::connector::exchange::IngressSequence;
use crate::connector::exchange::RequestPreparation;
use crate::connector::exchange::SessionObservation;
use crate::connector::exchange::SessionOpen;
use crate::connector::exchange::SessionWrite;
use crate::connectors::{DriverRegistry, RuntimeDrivers};
use crate::contract::ContractCatalog;
use crate::datatype::DataType;
use crate::envelope::{Data, Namespace, Value};
use crate::journal::{
    self, CallStatus, CloseVerdict, Closure, EgressRecord, Journal, MemJournal, Record,
};
use crate::node;
use crate::plan::{GraphVersion, ValidatedExecutionPlan};
use crate::runtime::execution::Execution;
use std::collections::BTreeSet;

/// Graph source and input chosen to produce one declared Driver exit case.
#[derive(Debug, Clone)]
pub struct ExitCaseTest {
    /// The declared `exit` case expected from this test.
    pub exit_case: &'static str,
    /// DSL source for the graph exercising this exit case.
    pub source: String,
    /// The submission line driving it.
    pub input: &'static str,
}

/// Prepared graphs, inputs and expected exit cases for Call Driver tests.
#[derive(Debug, Clone)]
pub struct DriverTestFixture {
    /// The contract sheet under test.
    pub contract: &'static ConnectorContract,
    /// A minimal probe unit exercising the happy path.
    pub source: String,
    /// The happy submission line.
    pub input: &'static str,
    /// The terminal case produced by the happy path.
    pub happy_exit_case: &'static str,
    /// One provocation per provocable declared terminal case.
    pub provocations: Vec<ExitCaseTest>,
    /// Declared terminal cases this Driver cannot be provoked into, each
    /// with its reason — stated, never silent.
    pub untestable: Vec<(&'static str, &'static str)>,
    /// A probe whose driver stays mute under a declared budget, for
    /// the timeout and cancel disciplines; `None` skips that battery
    /// (and leaves `TimeoutExpired` unproven).
    pub mute_source: Option<String>,
    /// The interleaved-correlation probe: a unit in which ONE
    /// submission opens two calls simultaneously (two connectors,
    /// distinct request bytes per branch). Each closed call's result
    /// must equal the connector-local expectation, so a Driver that
    /// crosses two concurrent replies is caught. `None` skips the
    /// battery and the correlation case stays unproven beyond silence.
    pub interleave: Option<InterleaveProbe>,
}

/// A validated graph and expected outputs for an inbound Event Driver test.
#[derive(Debug)]
pub struct EventTestFixture<'plan> {
    /// The Event contract sheet under test.
    pub contract: &'static ConnectorContract,
    /// Validated probe containing the bound Event source and its route.
    pub plan: &'plan ValidatedExecutionPlan,
    /// Exact root outputs expected after the retained proposal commits.
    pub expected_outputs: Vec<Data>,
}

/// A validated graph, messages and expected terminals for Session Driver tests.
#[derive(Debug)]
pub struct SessionTestFixture<'plan> {
    /// The Session contract sheet under test.
    pub contract: &'static ConnectorContract,
    /// Validated probe containing the bound Session Connector.
    pub plan: &'plan ValidatedExecutionPlan,
    /// Exact Connector node exercised by the profile.
    pub connector: node::Path,
    /// Two distinct opening payloads; concurrency is the point of the probe.
    pub openings: [Data; 2],
    /// One graph-to-world message for each opened session.
    pub writes: [Data; 2],
    /// Expected happy terminal payload for each session.
    pub terminal: Data,
    /// Already-resolved settings presented to the pure framing seam.
    pub resolved_settings: std::collections::BTreeMap<String, crate::value::Value>,
}

/// The probe of the interleaved-correlation battery. The battery
/// injects one submission whose data is the map `{a = …, b = …}`;
/// the probe's two branches each forward one field to their own
/// connector, so two calls with distinct bytes are in flight at
/// once.
#[derive(Debug, Clone)]
pub struct InterleaveProbe {
    /// The two-branch unit.
    pub source: String,
    /// The first branch's payload.
    pub a: &'static str,
    /// The second branch's payload — must differ from `a`.
    pub b: &'static str,
    /// Expected `out` value for each connector path. This makes the
    /// correlation check protocol-neutral: Process text and File bytes pass
    /// through the same proof.
    pub expected: Vec<InterleaveExpectation>,
}

/// One connector-local result expected by an interleaved probe.
#[derive(Debug, Clone)]
pub struct InterleaveExpectation {
    /// Connector whose call must carry this result.
    pub connector: node::Path,
    /// Exact `out` representation expected on that call.
    pub data: Data,
}

/// The harness verdict: what was proven, what was declared
/// untestable, and every discipline violation found.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// Terminal cases proven by a provocation or the mute battery.
    pub proven_cases: Vec<&'static str>,
    /// Declared terminal cases not proven here, with the stated reason.
    pub unproven_cases: Vec<(&'static str, &'static str)>,
    /// Every violation, in discovery order. Empty means conformant.
    pub violations: Vec<String>,
}

impl Report {
    /// Whether the battery found no violation.
    #[must_use]
    pub fn conformant(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Runs the whole battery over one Driver.
#[must_use]
pub fn verify(
    profile: &DriverTestFixture,
    registry: &ContractCatalog,
    drivers: &DriverRegistry,
) -> Report {
    let mut report = Report::default();
    sheet_completeness(profile.contract, &mut report);

    // The probe must compile against the registry carrying the sheet
    // — the config surface of case 6 is exercised by compilation.
    let compilation = match compile(&[profile.source.as_str()], registry) {
        Ok(compilation) => compilation,
        Err(bag) => {
            report.violations.push(format!(
                "the happy probe does not compile: {:?}",
                bag.diagnostics()
            ));
            return report;
        }
    };
    let exit_cases = match resolved_exit_cases(profile.contract, &compilation.plan) {
        Ok(cases) => cases,
        Err(violation) => {
            report.violations.push(violation);
            return report;
        }
    };

    // The silence half of case 7: a driver that was never started
    // has nothing to say. An event out of nowhere is an invented
    // correlation.
    {
        match bind(&compilation.plan, drivers) {
            Err(violation) => report.violations.push(violation),
            Ok(mut bindings) => match bindings.poll_responses() {
                Err(error) => report
                    .violations
                    .push(format!("the unstarted Driver could not be polled: {error}")),
                Ok(uninvited) if !uninvited.is_empty() => report.violations.push(format!(
                    "the Driver produced {} event(s) without any started call",
                    uninvited.len()
                )),
                Ok(_) => {}
            },
        }
    }

    // The happy path through the real machine: journal-before-effect
    // and close-once are observed on the record, not trusted (case
    // 10), and the completion gate holds every reply to the fixed
    // ProcessExit target (case 8). Correlation (case 7) is
    // implied by arrival: only a response bearing an open call's
    // reference can settle.
    {
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Execution::new(&compilation.plan, GraphVersion(1));
        let mut bindings = match bind(&compilation.plan, drivers) {
            Ok(bindings) => bindings,
            Err(violation) => {
                report.violations.push(violation);
                return report;
            }
        };
        match drive(&mut vm, &mut journal, &mut bindings, profile.input) {
            Err(violation) => report.violations.push(violation),
            Ok(()) => {
                audit_journal(&journal, &mut report);
                match source_exit_case(&vm).as_deref() {
                    Some(case)
                        if case == profile.happy_exit_case
                            && exit_cases.iter().any(|known| known == case) =>
                    {
                        report.proven_cases.push(profile.happy_exit_case);
                    }
                    Some(case) => report.violations.push(format!(
                        "the happy path terminated as `{case}`, expected `{}`",
                        profile.happy_exit_case
                    )),
                    None => report
                        .violations
                        .push("the happy path produced no root `exit` fact".into()),
                }
            }
        }
    }

    // Case 8, the provable half: every provocation must produce its
    // declared terminal case at the root sink.
    for provocation in &profile.provocations {
        match run_provocation(provocation, registry, drivers) {
            Ok(()) => {
                if !report.proven_cases.contains(&provocation.exit_case) {
                    report.proven_cases.push(provocation.exit_case);
                }
            }
            Err(violation) => report.violations.push(violation),
        }
    }

    // The timeout and cancel disciplines: only the runtime constructs a
    // timeout, and a cancelled call gains no late causal fact.
    if let Some(mute_source) = &profile.mute_source {
        match mute_battery(mute_source, registry, drivers) {
            Ok(()) => {
                if exit_cases.iter().any(|case| case == "TimeoutExpired")
                    && !report.proven_cases.contains(&"TimeoutExpired")
                {
                    report.proven_cases.push("TimeoutExpired");
                }
            }
            Err(violation) => report.violations.push(violation),
        }
    }

    // Case 7, the correlation half: two calls in flight, distinct
    // request bytes; every closure must answer its OWN request.
    if let Some(probe) = &profile.interleave
        && let Err(violation) = interleave_battery(probe, registry, drivers)
    {
        report.violations.push(violation);
    }

    // No silent gap: declared terminal cases are proven or stated
    // untestable.
    for (kind, reason) in &profile.untestable {
        if !report.unproven_cases.iter().any(|(known, _)| known == kind)
            && !report.proven_cases.contains(kind)
        {
            report.unproven_cases.push((kind, reason));
        }
    }
    for kind in &exit_cases {
        let covered = report.proven_cases.iter().any(|known| *known == kind)
            || report
                .unproven_cases
                .iter()
                .any(|(known, _)| *known == kind);
        if !covered {
            report.violations.push(format!(
                "declared terminal case `{kind}` is neither provoked nor declared untestable"
            ));
        }
    }
    report
}

/// Runs the Event-shaped battery: retained proposal, commit-before-decision,
/// ordinary bounded routing, and no fabricated call/session fact.
#[must_use]
pub fn verify_event(profile: &EventTestFixture<'_>, drivers: &DriverRegistry) -> Report {
    let mut report = Report::default();
    sheet_completeness(profile.contract, &mut report);
    if profile.contract.exchange != crate::connector::ExchangeKind::Event {
        report
            .violations
            .push("the Event profile names a non-Event contract".into());
        return report;
    }
    let mut bindings = match bind(profile.plan, drivers) {
        Ok(bindings) => bindings,
        Err(violation) => {
            report.violations.push(violation);
            return report;
        }
    };
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Execution::new(profile.plan, GraphVersion(1));
    let admission = match bindings.poll_inbound(&mut vm, &mut journal) {
        Ok(Some(admission)) => admission,
        Ok(None) => {
            report
                .violations
                .push("the Event Driver proposed no fact".into());
            return report;
        }
        Err(error) => {
            report
                .violations
                .push(format!("the Event proposal could not be admitted: {error}"));
            return report;
        }
    };
    let IngressAdmission::Accepted(accepted) = admission else {
        report
            .violations
            .push(format!("the Event proposal did not commit: {admission:?}"));
        return report;
    };
    if accepted.exchange.is_some() {
        report
            .violations
            .push("an Event acceptance invented an exchange reference".into());
    }
    if journal.find_envelope(accepted.envelope).is_none() {
        report
            .violations
            .push("the Driver was accepted before its envelope became journal-visible".into());
    }
    if let Err(error) = vm.run_ready_with_bindings(&mut journal, &mut bindings) {
        report
            .violations
            .push(format!("the admitted Event could not drain: {error}"));
    }
    let outputs: Vec<_> = vm
        .outputs()
        .iter()
        .map(|observed| observed.envelope.data().clone())
        .collect();
    if outputs != profile.expected_outputs {
        report.violations.push(format!(
            "the Event outputs differ: expected {:?}, got {outputs:?}",
            profile.expected_outputs
        ));
    }
    if journal
        .records()
        .iter()
        .any(|record| matches!(record, Record::Egress(_)))
    {
        report
            .violations
            .push("one Event fabricated an outbound lifecycle".into());
    }
    if report.conformant() {
        report.proven_cases.push("event-admission");
    }
    report
}

/// Runs the generic Session protocol battery without a socket implementation:
/// two concurrent opens, crossed observations, one write per session,
/// write-ambiguity abandonment, terminal correlation, and post-exit silence.
#[must_use]
pub fn verify_session(profile: &SessionTestFixture<'_>, drivers: &DriverRegistry) -> Report {
    let mut report = Report::default();
    sheet_completeness(profile.contract, &mut report);
    if profile.contract.exchange != crate::connector::ExchangeKind::Session {
        report
            .violations
            .push("the Session profile names a non-Session contract".into());
        return report;
    }
    let Some((slot, planned)) = profile
        .plan
        .nodes
        .iter()
        .enumerate()
        .find(|(_, planned)| planned.path == profile.connector)
    else {
        report.violations.push(format!(
            "the Session probe has no `{}` node",
            profile.connector
        ));
        return report;
    };
    let Some(spec) = planned.connector() else {
        report
            .violations
            .push(format!("`{}` is not a Connector", profile.connector));
        return report;
    };
    if spec.exchange != crate::connector::ExchangeKind::Session
        || spec.contract.as_str() != profile.contract.id
    {
        report.violations.push(format!(
            "`{}` does not carry the profile's Session contract",
            profile.connector
        ));
        return report;
    }
    let slot = match u32::try_from(slot) {
        Ok(slot) => crate::node::NodeIndex::new(slot),
        Err(_) => {
            report
                .violations
                .push("the Session probe exceeds the plan slot domain".into());
            return report;
        }
    };
    let Some(exit_type) = profile.plan.resolved_ports().node(slot).and_then(|ports| {
        ports
            .iter()
            .find(|port| port.port().name() == "exit")
            .map(|port| port.ty())
    }) else {
        report
            .violations
            .push("the Session probe has no resolved `exit` type".into());
        return report;
    };
    let mut bindings = match bind(profile.plan, drivers) {
        Ok(bindings) => bindings,
        Err(violation) => {
            report.violations.push(violation);
            return report;
        }
    };
    let mut journal = MemJournal::new(GraphVersion(1));
    let seed = match journal.append_ingress(submission("session-conformance")) {
        Ok(committed) => committed.envelope,
        Err(error) => {
            report
                .violations
                .push(format!("the Session seed could not commit: {error}"));
            return report;
        }
    };

    let mut sessions = Vec::new();
    for (index, data) in profile.openings.iter().enumerate() {
        let preparation = match SessionConnectorBindings::prepare_session_open(
            &mut bindings,
            &profile.connector,
            data,
            &profile.resolved_settings,
        ) {
            Ok(preparation) => preparation,
            Err(error) => {
                report.violations.push(format!(
                    "Session opening {} could not prepare: {error}",
                    index + 1
                ));
                return report;
            }
        };
        let RequestPreparation::Prepared(prepared) = preparation else {
            report.violations.push(format!(
                "Session opening {} was rejected before the open proof",
                index + 1
            ));
            return report;
        };
        let emitting_id = crate::envelope::Id::from_bytes([(index as u8).saturating_add(1); 16]);
        let intent = SessionOpen {
            session_id: journal::ids::outbound_session_id(
                &profile.connector,
                emitting_id,
                prepared.digest(),
            ),
            emitting_id,
            connector_path: profile.connector.clone(),
            prepared,
        };
        if let Err(error) = journal.open_outbound_session(intent.clone()) {
            report.violations.push(format!(
                "Session opening {} was not journaled: {error}",
                index + 1
            ));
            return report;
        }
        match SessionConnectorBindings::open_session(&mut bindings, &intent, &seed) {
            Ok(crate::connector::exchange::SessionStart::Started) => {}
            Ok(crate::connector::exchange::SessionStart::Pending) => {
                report.violations.push(format!(
                    "Session opening {} remained pending in the uncongested battery",
                    index + 1
                ));
                return report;
            }
            Err(error) => {
                report.violations.push(format!(
                    "Session opening {} could not start: {error}",
                    index + 1
                ));
                return report;
            }
        }
        sessions.push(intent.session_id);
    }
    let opening_events = match SessionConnectorBindings::poll_observations(&mut bindings) {
        Ok(events) => events,
        Err(error) => {
            report
                .violations
                .push(format!("Session opening observations failed: {error}"));
            return report;
        }
    };
    let opened: BTreeSet<_> = opening_events
        .iter()
        .filter_map(|event| match event {
            SessionObservation::Opened(session) => Some(*session),
            _ => None,
        })
        .collect();
    if opening_events.len() != sessions.len()
        || opened != sessions.iter().copied().collect::<BTreeSet<_>>()
    {
        report.violations.push(format!(
            "concurrent Session openings lost or crossed correlation: {opening_events:?}"
        ));
        return report;
    }
    for session in &sessions {
        if journal.confirm_session_open(session) != Ok(CloseVerdict::Won) {
            report
                .violations
                .push(format!("Session {session} did not open exactly once"));
            return report;
        }
    }

    let mut expected_writes = BTreeSet::new();
    for (index, (session, data)) in sessions.iter().zip(&profile.writes).enumerate() {
        let preparation = match SessionConnectorBindings::prepare_session_write(
            &mut bindings,
            &profile.connector,
            *session,
            data,
            &profile.resolved_settings,
        ) {
            Ok(preparation) => preparation,
            Err(error) => {
                report.violations.push(format!(
                    "Session write {} could not prepare: {error}",
                    index + 1
                ));
                return report;
            }
        };
        let RequestPreparation::Prepared(prepared) = preparation else {
            report
                .violations
                .push(format!("Session write {} was rejected", index + 1));
            return report;
        };
        let emitting_id = crate::envelope::Id::from_bytes([(index as u8).saturating_add(11); 16]);
        let intent = SessionWrite {
            write_ref: journal::ids::session_write_id(*session, emitting_id, prepared.digest()),
            session_id: *session,
            emitting_id,
            connector_path: profile.connector.clone(),
            prepared,
        };
        if let Err(error) = journal.open_session_write(intent.clone()) {
            report.violations.push(format!(
                "Session write {} was not journaled: {error}",
                index + 1
            ));
            return report;
        }
        match SessionConnectorBindings::write_session(&mut bindings, &intent, &seed) {
            Ok(crate::connector::exchange::SessionStart::Started) => {}
            Ok(crate::connector::exchange::SessionStart::Pending) => {
                report.violations.push(format!(
                    "Session write {} remained pending in the uncongested battery",
                    index + 1
                ));
                return report;
            }
            Err(error) => {
                report.violations.push(format!(
                    "Session write {} could not start: {error}",
                    index + 1
                ));
                return report;
            }
        }
        expected_writes.insert((*session, intent.write_ref));
    }
    let write_events = match SessionConnectorBindings::poll_observations(&mut bindings) {
        Ok(events) => events,
        Err(error) => {
            report
                .violations
                .push(format!("Session write observations failed: {error}"));
            return report;
        }
    };
    let settled: BTreeSet<_> = write_events
        .iter()
        .filter_map(|event| match event {
            SessionObservation::WriteSettled { session, write } => Some((*session, *write)),
            _ => None,
        })
        .collect();
    if write_events.len() != expected_writes.len() || settled != expected_writes {
        report.violations.push(format!(
            "concurrent Session writes lost or crossed correlation: {write_events:?}"
        ));
        return report;
    }
    for (_, write) in &expected_writes {
        if journal.settle_session_write(write, false) != Ok(CloseVerdict::Won) {
            report
                .violations
                .push(format!("Session write {write} did not settle exactly once"));
            return report;
        }
    }

    // Crash ambiguity is a journal fact, never permission to replay bytes.
    let ambiguous_data = &profile.writes[0];
    let preparation = SessionConnectorBindings::prepare_session_write(
        &mut bindings,
        &profile.connector,
        sessions[0],
        ambiguous_data,
        &profile.resolved_settings,
    );
    if let Ok(RequestPreparation::Prepared(prepared)) = preparation {
        let emitting_id = crate::envelope::Id::from_bytes([21; 16]);
        let intent = SessionWrite {
            write_ref: journal::ids::session_write_id(sessions[0], emitting_id, prepared.digest()),
            session_id: sessions[0],
            emitting_id,
            connector_path: profile.connector.clone(),
            prepared,
        };
        if journal.open_session_write(intent.clone()).is_err()
            || journal.settle_session_write(&intent.write_ref, true) != Ok(CloseVerdict::Won)
        {
            report
                .violations
                .push("an ambiguous Session write did not close as abandoned".into());
            return report;
        }
    } else {
        report
            .violations
            .push("the ambiguity probe could not prepare its Session write".into());
        return report;
    }

    for session in &sessions {
        match SessionConnectorBindings::close_session(&mut bindings, &profile.connector, *session) {
            Ok(crate::connector::exchange::SessionStart::Started) => {}
            Ok(crate::connector::exchange::SessionStart::Pending) => {
                report.violations.push(format!(
                    "Session {session} close remained pending in the uncongested battery"
                ));
                return report;
            }
            Err(error) => {
                report
                    .violations
                    .push(format!("Session {session} could not close: {error}"));
                return report;
            }
        }
    }
    let exits = match SessionConnectorBindings::poll_observations(&mut bindings) {
        Ok(events) => events,
        Err(error) => {
            report
                .violations
                .push(format!("Session terminal observations failed: {error}"));
            return report;
        }
    };
    let mut terminal_sessions = BTreeSet::new();
    for event in &exits {
        let SessionObservation::Exited { session, exit } = event else {
            report
                .violations
                .push(format!("a non-terminal event followed close: {event:?}"));
            continue;
        };
        if !terminal_sessions.insert(*session) || !sessions.contains(session) {
            report.violations.push(format!(
                "Session {session} produced a crossed or repeated exit"
            ));
        }
        if exit.port.name() != "exit" || exit.data != profile.terminal {
            report.violations.push(format!(
                "Session {session} produced the wrong terminal payload: {exit:?}"
            ));
        }
        match &exit.data {
            Data::Materialized(value) if exit_type.accepts(value) => {}
            Data::Materialized(_) => report.violations.push(format!(
                "Session {session}'s exit violates its resolved type"
            )),
            Data::Encoded { .. } => report.violations.push(format!(
                "Session {session}'s exit was not materialized for validation"
            )),
        }
    }
    if exits.len() != sessions.len() || terminal_sessions.len() != sessions.len() {
        report.violations.push(format!(
            "the Session Driver produced {} terminal events for {} sessions",
            exits.len(),
            sessions.len()
        ));
    }
    match SessionConnectorBindings::poll_observations(&mut bindings) {
        Ok(events) if events.is_empty() => {}
        Ok(events) => report
            .violations
            .push(format!("the Driver spoke after Session exit: {events:?}")),
        Err(error) => report
            .violations
            .push(format!("post-exit Session polling failed: {error}")),
    }
    if report.conformant() {
        report.proven_cases.extend([
            "session-open",
            "session-write",
            "session-ambiguity",
            "session-exit",
        ]);
    }
    report
}

/// Panics with the full violation list unless the battery is clean
/// AND the unproven set is exactly the accepted one — the one-line
/// assertion a connector's suite ends with. The second check keeps
/// the gap explicit: shrinking it is progress, growing it is a
/// deliberate, visible decision.
pub fn assert_conformant(report: &Report, accepted_unproven: &[&str]) {
    assert!(
        report.conformant(),
        "the Driver is not conformant:\n{:#?}",
        report.violations
    );
    let unproven: Vec<&str> = report
        .unproven_cases
        .iter()
        .map(|(kind, _)| *kind)
        .collect();
    assert_eq!(
        unproven, accepted_unproven,
        "the unproven set moved — prove the kind or state why not"
    );
}

/// Cases 1–6 and 9 are declarative; what the harness can hold is
/// their presence and well-formedness on the sheet.
fn sheet_completeness(contract: &ConnectorContract, report: &mut Report) {
    if contract.name.is_empty() {
        report.violations.push("the sheet has no name".into());
    }
    let exits = contract
        .ports
        .iter()
        .filter(|port| port.name == "exit" && port.direction == crate::node::PortDirection::Output)
        .count();
    let terminal_shape_holds = match contract.exchange {
        crate::connector::ExchangeKind::Event => exits == 0,
        crate::connector::ExchangeKind::Call | crate::connector::ExchangeKind::Session => {
            exits == 1
        }
    };
    if !terminal_shape_holds {
        report.violations.push(format!(
            "the {} sheet declares {exits} terminal `exit` outputs",
            contract.exchange
        ));
    }
    let roles_hold = match contract.exchange {
        crate::connector::ExchangeKind::Event => {
            contract.roles == crate::connector::ConnectorDirection::Inbound
        }
        crate::connector::ExchangeKind::Call => true,
        crate::connector::ExchangeKind::Session => contract.roles.has_outbound(),
    };
    if !roles_hold {
        report.violations.push(format!(
            "the {} sheet declares incompatible {} roles",
            contract.exchange, contract.roles
        ));
    }
    for port in contract.ports {
        if node::Port::new(port.name).is_err() {
            report
                .violations
                .push(format!("the sheet names an invalid port `{}`", port.name));
        }
    }
    // Duplicates are judged BY SCOPE: within the common sheet,
    // within one profile, and between common and a profile — never
    // between two sibling profiles (the selected profile
    // disambiguates them).
    let mut common = BTreeSet::new();
    for attribute in contract.common {
        if !common.insert(attribute.name) {
            report.violations.push(format!(
                "the sheet declares attribute `{}` twice",
                attribute.name
            ));
        }
    }
    for profile in contract
        .selector
        .iter()
        .flat_map(|selector| selector.profiles.iter())
    {
        let mut scope = common.clone();
        for attribute in profile.attributes {
            if !scope.insert(attribute.name) {
                report.violations.push(format!(
                    "the sheet declares attribute `{}` twice in `{}`'s scope",
                    attribute.name, profile.word
                ));
            }
        }
    }
}

fn resolved_exit_cases(
    contract: &ConnectorContract,
    plan: &ValidatedExecutionPlan,
) -> Result<Vec<String>, String> {
    let (index, _planned) = plan
        .nodes
        .iter()
        .enumerate()
        .find(|planned| {
            planned
                .1
                .connector()
                .is_some_and(|spec| spec.contract.as_str() == contract.id)
        })
        .ok_or_else(|| format!("the probe instantiates no `{}` connector", contract.name))?;
    let slot = u32::try_from(index)
        .map(crate::node::NodeIndex::new)
        .map_err(|_| String::from("the probe has more nodes than the plan slot domain"))?;
    let exit = plan
        .resolved_ports()
        .node(slot)
        .and_then(|ports| ports.iter().find(|port| port.port().name() == "exit"))
        .map(|port| port.ty().clone())
        .ok_or_else(|| String::from("the validated plan has no typed `exit` port"))?;
    let DataType::TaggedUnion(union) = &exit else {
        return Err(String::from(
            "the resolved `exit` port is not a tagged union",
        ));
    };
    Ok(union
        .cases()
        .iter()
        .map(|case| case.name().to_string())
        .collect())
}

fn bind<'plan>(
    plan: &'plan ValidatedExecutionPlan,
    drivers: &DriverRegistry,
) -> Result<RuntimeDrivers<'plan>, String> {
    RuntimeDrivers::prepare(plan, drivers)
        .map_err(|error| format!("the conformance composition cannot bind: {error}"))
}

fn submission(text: &str) -> IngressProposal {
    IngressProposal {
        provenance: IngressProvenance::single(
            node::Path::new(["conformance"]).unwrap_or_else(|_| unreachable!("a valid path")),
        ),
        draft: EnvelopeDraft {
            port: node::Port::new("in").unwrap_or_else(|_| unreachable!("a valid port")),
            data: Data::materialized(Value::Struct(vec![(
                String::from("text"),
                Value::Text(String::from(text)),
            )])),
            ext: Namespace::new(),
        },
        sequence: IngressSequence::Next,
        session: None,
    }
}

/// Drives one submission to stability, mapping the wall clock onto
/// the virtual clock in small steps so declared budgets expire for
/// real. Any gate rejection is a violation: the Driver answered outside its
/// resolved completion contract.
fn drive(
    vm: &mut Execution<'_>,
    journal: &mut MemJournal,
    driver: &mut dyn OutboundDrivers,
    input: &str,
) -> Result<(), String> {
    vm.accept_input(journal, submission(input))
        .map_err(|error| format!("the submission was refused: {error}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        vm.run_ready_with_bindings(journal, driver)
            .map_err(|error| format!("a step faulted: {error}"))?;
        let report = vm
            .poll_drivers(journal, driver)
            .map_err(|error| format!("Driver polling faulted: {error}"))?;
        if let Some((_, reason)) = report.rejected.first() {
            return Err(format!(
                "the completion gate refused a reply: {reason} — the Driver \
                 produced a completion outside its declared contract"
            ));
        }
        vm.run_ready_with_bindings(journal, driver)
            .map_err(|error| format!("a step faulted: {error}"))?;
        if vm.is_stable() {
            return Ok(());
        }
        if report.settled == 0 {
            if std::time::Instant::now() > deadline {
                return Err("the run wedged: neither settled nor expired".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            vm.advance_to(journal, driver, vm.logical_time() + 10_000_000)
                .map_err(|error| format!("the advance faulted: {error}"))?;
        }
    }
}

/// Case 10, observed on the record: every closure follows its own
/// open fact, and no call closes twice.
fn audit_journal(journal: &MemJournal, report: &mut Report) {
    let mut open = BTreeSet::new();
    let mut closed = BTreeSet::new();
    let mut any_closed = false;
    for record in journal.records() {
        if let Record::Egress(EgressRecord::Call(outbound)) = record {
            let call = outbound.intent.call_id;
            match &outbound.status {
                CallStatus::Open => {
                    if !open.insert(call) {
                        report
                            .violations
                            .push(format!("call {call} has two open facts"));
                    }
                }
                CallStatus::Closed(_) => {
                    any_closed = true;
                    if !open.contains(&call) {
                        report
                            .violations
                            .push(format!("call {call} closed without an open fact"));
                    }
                    if !closed.insert(call) {
                        report.violations.push(format!("call {call} closed twice"));
                    }
                }
                CallStatus::Abandoned => {}
            }
        }
    }
    if open.is_empty() {
        report
            .violations
            .push("the happy path opened no call — the probe never reaches the Driver".into());
    } else if !any_closed {
        report
            .violations
            .push("no call closed on the happy path".into());
    }
}

fn run_provocation(
    provocation: &ExitCaseTest,
    registry: &ContractCatalog,
    drivers: &DriverRegistry,
) -> Result<(), String> {
    let compilation = compile(&[provocation.source.as_str()], registry).map_err(|bag| {
        format!(
            "the `{}` provocation does not compile: {:?}",
            provocation.exit_case,
            bag.diagnostics()
        )
    })?;
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Execution::new(&compilation.plan, GraphVersion(1));
    let mut bindings = bind(&compilation.plan, drivers)?;
    drive(&mut vm, &mut journal, &mut bindings, provocation.input)
        .map_err(|violation| format!("the `{}` provocation: {violation}", provocation.exit_case))?;
    match source_exit_case(&vm) {
        Some(case) if case == provocation.exit_case => Ok(()),
        Some(case) => Err(format!(
            "the `{}` provocation terminated as `{case}`",
            provocation.exit_case
        )),
        None => Err(format!(
            "the `{}` provocation produced no root `exit` fact",
            provocation.exit_case
        )),
    }
}

fn source_exit_case(vm: &Execution<'_>) -> Option<String> {
    let mut cases = vm.envelope_views().iter().filter_map(|observed| {
        if observed.node.is_none() || observed.port.name() != "exit" {
            return None;
        }
        let Data::Materialized(Value::TaggedUnion(tagged)) = observed.envelope.data() else {
            return None;
        };
        Some(tagged.case.clone())
    });
    let case = cases.next()?;
    if cases.next().is_some() {
        return None;
    }
    Some(case)
}

/// The mute battery: under a declared budget and a silent driver,
/// the runtime constructs the timeout — never the Driver — the Call
/// closes exactly once, and a cancelled call gains no late causal
/// fact.
fn mute_battery(
    source: &str,
    registry: &ContractCatalog,
    drivers: &DriverRegistry,
) -> Result<(), String> {
    let compilation = compile(&[source], registry)
        .map_err(|bag| format!("the mute probe does not compile: {:?}", bag.diagnostics()))?;
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Execution::new(&compilation.plan, GraphVersion(1));
    let mut bindings = bind(&compilation.plan, drivers)?;
    drive(&mut vm, &mut journal, &mut bindings, "conformance")
        .map_err(|violation| format!("the mute battery: {violation}"))?;
    let timed_out = journal.records().iter().any(|record| {
        matches!(
            record,
            Record::Egress(EgressRecord::Call(outbound))
                if matches!(outbound.status, CallStatus::Closed(Closure::Timeout))
        )
    });
    if !timed_out {
        return Err("the mute call never closed with the machine's Timeout fact".into());
    }
    if source_exit_case(&vm).as_deref() != Some("TimeoutExpired") {
        return Err("the scheduler did not publish `TimeoutExpired` on `exit`".into());
    }
    // The driver was cancelled at the closure; later rounds must
    // add nothing causal — no settled event, no new record.
    let sealed_records = journal.records().len();
    for _ in 0..3 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        let report = vm
            .poll_drivers(&mut journal, &mut bindings)
            .map_err(|error| format!("a post-cancel round faulted: {error}"))?;
        if report.settled != 0 {
            return Err("a cancelled call settled again — a late causal fact".into());
        }
    }
    if journal.records().len() != sealed_records {
        return Err("the journal grew after the cancelled closure".into());
    }
    Ok(())
}

/// The interleaved-correlation battery: one submission opens two calls with
/// distinct request bytes (the machine serializes calls per connector, so
/// simultaneity needs two connectors served by one Driver). Each completion
/// must carry the result declared for its own connector, with its own terminal
/// `exit`. A Driver that swaps two concurrent responses — correct call
/// references, wrong payloads — is caught here and nowhere else, and the
/// journal must show both opens before the first closure, or the battery never
/// actually interleaved.
fn interleave_battery(
    probe: &InterleaveProbe,
    registry: &ContractCatalog,
    drivers: &DriverRegistry,
) -> Result<(), String> {
    let compilation = compile(&[probe.source.as_str()], registry).map_err(|bag| {
        format!(
            "the interleave probe does not compile: {:?}",
            bag.diagnostics()
        )
    })?;
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Execution::new(&compilation.plan, GraphVersion(1));
    let mut bindings = bind(&compilation.plan, drivers)?;
    if probe.a == probe.b {
        return Err("the interleave payloads must differ, or a swap is invisible".into());
    }
    let mut expected = std::collections::BTreeMap::new();
    for expectation in &probe.expected {
        if expected
            .insert(expectation.connector.clone(), expectation.data.clone())
            .is_some()
        {
            return Err(format!(
                "the interleave profile expects `{}` more than once",
                expectation.connector
            ));
        }
    }
    if expected.len() < 2 {
        return Err("the interleave profile must distinguish at least two connectors".into());
    }
    if probe
        .expected
        .iter()
        .enumerate()
        .any(|(left, expectation)| {
            probe.expected[left + 1..]
                .iter()
                .any(|other| expectation.data == other.data)
        })
    {
        return Err(
            "the interleave expected outputs must differ, or a swapped reply is invisible".into(),
        );
    }
    let data = vec![
        (String::from("a"), Value::Text(String::from(probe.a))),
        (String::from("b"), Value::Text(String::from(probe.b))),
    ];
    let paired = IngressProposal {
        provenance: IngressProvenance::single(
            node::Path::new(["conformance"]).unwrap_or_else(|_| unreachable!("a valid path")),
        ),
        draft: EnvelopeDraft {
            port: node::Port::new("in").unwrap_or_else(|_| unreachable!("a valid port")),
            data: Data::materialized(Value::Struct(data)),
            ext: Namespace::new(),
        },
        sequence: IngressSequence::Next,
        session: None,
    };
    vm.accept_input(&mut journal, paired)
        .map_err(|error| format!("the interleave submission was refused: {error}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        vm.run_ready_with_bindings(&mut journal, &mut bindings)
            .map_err(|error| format!("a step faulted: {error}"))?;
        let report = vm
            .poll_drivers(&mut journal, &mut bindings)
            .map_err(|error| format!("Driver polling faulted: {error}"))?;
        vm.run_ready_with_bindings(&mut journal, &mut bindings)
            .map_err(|error| format!("a step faulted: {error}"))?;
        if vm.is_stable() {
            break;
        }
        if report.settled == 0 {
            if std::time::Instant::now() > deadline {
                return Err("the interleave run wedged".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            vm.advance_to(&mut journal, &mut bindings, vm.logical_time() + 10_000_000)
                .map_err(|error| format!("the advance faulted: {error}"))?;
        }
    }
    let mut closed = 0usize;
    let mut opens_before_first_close = 0usize;
    let mut any_close_seen = false;
    let mut requests = BTreeSet::new();
    let mut answered = BTreeSet::new();
    for record in journal.records() {
        let Record::Egress(EgressRecord::Call(outbound)) = record else {
            continue;
        };
        if matches!(outbound.status, CallStatus::Open) {
            if !any_close_seen {
                opens_before_first_close += 1;
            }
            requests.insert(outbound.intent.prepared.bytes().to_vec());
            continue;
        }
        let CallStatus::Closed(Closure::Response(completion)) = &outbound.status else {
            continue;
        };
        any_close_seen = true;
        closed += 1;
        let Some(data) = completion
            .outputs()
            .iter()
            .find(|output| output.port.name() == "out")
            .map(|output| &output.data)
        else {
            return Err(format!(
                "interleave call {} completed without an out frame; the probe expects echoes",
                outbound.intent.call_id
            ));
        };
        let Some(expected_data) = expected.get(&outbound.intent.connector_path) else {
            return Err(format!(
                "interleave call {} came from unexpected connector `{}`",
                outbound.intent.call_id, outbound.intent.connector_path
            ));
        };
        if data != expected_data {
            return Err(format!(
                "call {} on `{}` answered with another call's payload: expected \
                 {expected_data:?}, got {data:?} — the Driver crossed two concurrent replies",
                outbound.intent.call_id, outbound.intent.connector_path
            ));
        }
        answered.insert(outbound.intent.connector_path.clone());
    }
    if closed < 2 {
        return Err(format!(
            "the interleave battery closed {closed} call(s); two in-flight calls are its point"
        ));
    }
    if opens_before_first_close < 2 {
        return Err(
            "the interleave calls never overlapped — the probe must open both before either \
             closes"
                .into(),
        );
    }
    if requests.len() < 2 {
        return Err(
            "the interleave requests were byte-identical; a swap would be invisible".into(),
        );
    }
    if answered.len() != expected.len() {
        return Err(format!(
            "the interleave battery answered {} of {} expected connectors",
            answered.len(),
            expected.len()
        ));
    }
    Ok(())
}
