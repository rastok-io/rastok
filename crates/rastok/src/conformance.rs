// The harness drives real adapters against the wall clock, mapped
// onto the virtual clock — the frontier lints stop at this file for
// the same reason they stop at the adapter layer.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
//! The connector conformance harness — the ten contract cases of
//! `stdlib.md` §1 as one reusable battery over the
//! [`Partner`] seam.
//!
//! A new connector brings three things: its contract sheet, an
//! adapter, and a [`Profile`] — a minimal probe unit, one happy
//! input, one provocation per provocable declared error kind, and a
//! stated reason for every kind the harness cannot reach.
//!
//! **What the battery holds executable today, stated exactly** —
//! the M0 subset of the ten contract cases, not yet all ten:
//! sheet presence and well-formedness (cases 1–6 and 9 are
//! declarative; the sheet cannot yet express port types, the
//! correlator, the instance model, or the terminal envelope — those
//! widen with the sheet model); the silence discipline and, when the
//! profile opts in, response-to-request correlation under two
//! interleaved calls (case 7); the error-set discipline under the
//! outcome gate with per-kind provocations (case 8); journal-before-
//! effect and close-once observed on the record, the scheduler-minted
//! timeout, and the no-late-causal-fact rule after a cancel
//! (case 10). Single-instance only (case 9 is M0's singleton).
//! The same seam serves a std adapter today and an embedded one
//! later — the battery grows with the milestones toward the full
//! portability contract, and [`assert_conformant`] keeps every gap
//! it cannot yet close explicit.

use crate::boundary::{Draft, InboundSubmission, IngressAuthority, Partner, SubmissionSequence};
use crate::compiler::compile;
use crate::contract::{ConnectorContract, Registry};
use crate::envelope::{Data, Namespace, Value};
use crate::journal::{Closure, MemJournal, OutboundStatus, Record};
use crate::node;
use crate::plan::{GraphVersion, ValidatedExecutionPlan};
use crate::runtime::vm::Vm;
use std::collections::BTreeSet;

/// Builds one fresh adapter per battery run. A trait rather than a
/// closure because the adapter may borrow the probe's plan — the
/// generic method carries that lifetime cleanly.
pub trait AdapterFactory {
    /// A fresh adapter for one compiled probe.
    fn make<'plan>(&mut self, plan: &'plan ValidatedExecutionPlan) -> Box<dyn Partner + 'plan>;
}

/// One way to force one declared error kind out of the adapter.
#[derive(Debug, Clone)]
pub struct Provocation {
    /// The declared kind this provocation must produce.
    pub kind: &'static str,
    /// A probe unit whose run ends with that kind at the root sink.
    pub source: String,
    /// The submission line driving it.
    pub input: &'static str,
}

/// What one connector offers the harness.
#[derive(Debug, Clone)]
pub struct Profile {
    /// The contract sheet under test.
    pub contract: &'static ConnectorContract,
    /// A minimal probe unit exercising the happy path.
    pub source: String,
    /// The happy submission line.
    pub input: &'static str,
    /// One provocation per provocable declared kind.
    pub provocations: Vec<Provocation>,
    /// Declared kinds this adapter cannot be provoked into, each
    /// with its reason — stated, never silent.
    pub unprovable: Vec<(&'static str, &'static str)>,
    /// A probe whose partner stays mute under a declared budget, for
    /// the timeout and cancel disciplines; `None` skips that battery
    /// (and leaves `TimeoutExpired` unproven).
    pub mute_source: Option<String>,
    /// The interleaved-correlation probe: a unit in which ONE
    /// submission opens two calls simultaneously (two connectors,
    /// distinct request bytes per branch) against an echoing
    /// partner — each closed call's response must equal its own
    /// request bytes, so an adapter that crosses two concurrent
    /// replies is caught. `None` skips the battery and the
    /// correlation case stays unproven beyond silence.
    pub interleave: Option<InterleaveProbe>,
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
}

/// The harness verdict: what was proven, what was declared
/// unprovable, and every discipline violation found.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// Error kinds proven by a provocation or the mute battery.
    pub proven_kinds: Vec<&'static str>,
    /// Declared kinds not proven here, with the stated reason.
    pub unproven_kinds: Vec<(&'static str, &'static str)>,
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

/// Runs the whole battery over one adapter.
#[must_use]
pub fn verify(profile: &Profile, registry: &Registry, factory: &mut dyn AdapterFactory) -> Report {
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

    // The silence half of case 7: a partner that was never started
    // has nothing to say. An event out of nowhere is an invented
    // correlation.
    {
        let mut partner = factory.make(&compilation.plan);
        let uninvited = partner.events();
        if !uninvited.is_empty() {
            report.violations.push(format!(
                "the adapter produced {} event(s) without any started call",
                uninvited.len()
            ));
        }
    }

    // The happy path through the real machine: journal-before-effect
    // and close-once are observed on the record, not trusted (case
    // 10), and the outcome gate holds every reply to the declared
    // error set along the way (case 8). Correlation (case 7) is
    // implied by arrival: only a response bearing an open call's
    // reference can settle.
    {
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        let mut partner = factory.make(&compilation.plan);
        match drive(&mut vm, &mut journal, partner.as_mut(), profile.input) {
            Err(violation) => report.violations.push(violation),
            Ok(()) => {
                audit_journal(&journal, &mut report);
                if !vm.errors().is_empty() {
                    report.violations.push(format!(
                        "the happy path ended with {} error(s) at the root sink",
                        vm.errors().len()
                    ));
                }
            }
        }
    }

    // Case 8, the provable half: every provocation must produce its
    // declared kind at the root sink.
    for provocation in &profile.provocations {
        match run_provocation(provocation, registry, factory) {
            Ok(()) => report.proven_kinds.push(provocation.kind),
            Err(violation) => report.violations.push(violation),
        }
    }

    // The timeout and cancel disciplines: only the machine mints a
    // timeout, and a cancelled call gains no late causal fact.
    if let Some(mute_source) = &profile.mute_source {
        match mute_battery(mute_source, registry, factory) {
            Ok(()) => {
                if profile.contract.error_set.contains(&"TimeoutExpired") {
                    report.proven_kinds.push("TimeoutExpired");
                }
            }
            Err(violation) => report.violations.push(violation),
        }
    }

    // Case 7, the correlation half: two calls in flight, distinct
    // request bytes; every closure must answer its OWN request.
    if let Some(probe) = &profile.interleave
        && let Err(violation) = interleave_battery(probe, registry, factory)
    {
        report.violations.push(violation);
    }

    // No silent gap: declared kinds are proven or stated unprovable.
    // `OutcomeUnknown` alone is exempt — only the recovery fold
    // mints it, which needs a crash outside this harness's reach;
    // the fold carries its own tests.
    for (kind, reason) in &profile.unprovable {
        report.unproven_kinds.push((kind, reason));
    }
    for kind in profile.contract.error_set {
        let covered = report.proven_kinds.contains(kind)
            || report.unproven_kinds.iter().any(|(k, _)| k == kind)
            || *kind == "OutcomeUnknown";
        if !covered {
            report.violations.push(format!(
                "declared kind `{kind}` is neither provoked nor declared unprovable"
            ));
        }
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
        "the adapter is not conformant:\n{:#?}",
        report.violations
    );
    let unproven: Vec<&str> = report
        .unproven_kinds
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
    if contract.error_set.is_empty() {
        report
            .violations
            .push("the sheet declares no error set (case 8 is mandatory)".into());
    }
    for port in contract.input_ports.iter().chain(contract.output_ports) {
        if node::Port::new(*port).is_err() {
            report
                .violations
                .push(format!("the sheet names an invalid port `{port}`"));
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

fn submission(text: &str) -> InboundSubmission {
    InboundSubmission {
        authority: IngressAuthority::single(
            node::Path::new(["conformance"]).unwrap_or_else(|_| unreachable!("a valid path")),
        ),
        port: node::Port::new("in").unwrap_or_else(|_| unreachable!("a valid port")),
        draft: Draft {
            data: Data::materialized(Value::Struct(vec![(
                String::from("text"),
                Value::Text(String::from(text)),
            )])),
            ext: Namespace::new(),
        },
        sequence: SubmissionSequence::Next,
    }
}

/// Drives one submission to stability, mapping the wall clock onto
/// the virtual clock in small steps so declared budgets expire for
/// real. Any gate rejection is a violation: the adapter answered
/// outside its declared error set.
fn drive(
    vm: &mut Vm<'_>,
    journal: &mut MemJournal,
    partner: &mut dyn Partner,
    input: &str,
) -> Result<(), String> {
    vm.inject(journal, submission(input))
        .map_err(|error| format!("the submission was refused: {error}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        vm.drain(journal)
            .map_err(|error| format!("a step faulted: {error}"))?;
        let report = vm
            .pump(journal, partner)
            .map_err(|error| format!("the boundary round faulted: {error}"))?;
        if let Some((_, reason)) = report.rejected.first() {
            return Err(format!(
                "the outcome gate refused a reply: {reason} — the adapter \
                 produced an outcome outside its declared contract"
            ));
        }
        vm.drain(journal)
            .map_err(|error| format!("a step faulted: {error}"))?;
        if vm.is_stable() {
            return Ok(());
        }
        if report.settled == 0 {
            if std::time::Instant::now() > deadline {
                return Err("the run wedged: neither settled nor expired".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            vm.advance(journal, partner, vm.now() + 10_000_000)
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
        if let Record::Outbound(outbound) = record {
            let call = outbound.intent.call_ref;
            match &outbound.status {
                OutboundStatus::Open => {
                    if !open.insert(call) {
                        report
                            .violations
                            .push(format!("call {call} has two open facts"));
                    }
                }
                OutboundStatus::Closed(_) => {
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
                OutboundStatus::Abandoned => {}
            }
        }
    }
    if open.is_empty() {
        report
            .violations
            .push("the happy path opened no call — the probe never reaches the adapter".into());
    } else if !any_closed {
        report
            .violations
            .push("no call closed on the happy path".into());
    }
}

fn run_provocation(
    provocation: &Provocation,
    registry: &Registry,
    factory: &mut dyn AdapterFactory,
) -> Result<(), String> {
    let compilation = compile(&[provocation.source.as_str()], registry).map_err(|bag| {
        format!(
            "the `{}` provocation does not compile: {:?}",
            provocation.kind,
            bag.diagnostics()
        )
    })?;
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut partner = factory.make(&compilation.plan);
    drive(&mut vm, &mut journal, partner.as_mut(), provocation.input)
        .map_err(|violation| format!("the `{}` provocation: {violation}", provocation.kind))?;
    let Some(first) = vm.errors().first() else {
        return Err(format!(
            "the `{}` provocation produced no error at the root sink",
            provocation.kind
        ));
    };
    let Data::Materialized(Value::Map(payload)) = first.data() else {
        return Err(format!(
            "the `{}` provocation: the error payload is not the typed map",
            provocation.kind
        ));
    };
    match payload.get("kind") {
        Some(Value::Text(kind)) if kind == provocation.kind => Ok(()),
        other => Err(format!(
            "the `{}` provocation produced kind {other:?}",
            provocation.kind
        )),
    }
}

/// The mute battery: under a declared budget and a silent partner,
/// the machine mints the timeout — never the adapter — the call
/// closes exactly once, and a cancelled call gains no late causal
/// fact.
fn mute_battery(
    source: &str,
    registry: &Registry,
    factory: &mut dyn AdapterFactory,
) -> Result<(), String> {
    let compilation = compile(&[source], registry)
        .map_err(|bag| format!("the mute probe does not compile: {:?}", bag.diagnostics()))?;
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut partner = factory.make(&compilation.plan);
    drive(&mut vm, &mut journal, partner.as_mut(), "conformance")
        .map_err(|violation| format!("the mute battery: {violation}"))?;
    let timed_out = journal.records().iter().any(|record| {
        matches!(
            record,
            Record::Outbound(outbound)
                if matches!(outbound.status, OutboundStatus::Closed(Closure::Timeout))
        )
    });
    if !timed_out {
        return Err("the mute call never closed with the machine's Timeout fact".into());
    }
    // The partner was cancelled at the closure; later rounds must
    // add nothing causal — no settled event, no new record.
    let sealed_records = journal.records().len();
    for _ in 0..3 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        let report = vm
            .pump(&mut journal, partner.as_mut())
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

/// The interleaved-correlation battery: one submission opens two
/// calls with distinct request bytes (the machine serializes calls
/// per connector, so simultaneity needs two connectors served by
/// the one adapter); for an echoing partner, each
/// `Closed(Response(Success))` must carry exactly its own intent's
/// prepared bytes. An adapter that swaps two concurrent responses —
/// correct call references, wrong payloads — is caught here and
/// nowhere else, and the journal must show both opens before the
/// first closure, or the battery never actually interleaved.
fn interleave_battery(
    probe: &InterleaveProbe,
    registry: &Registry,
    factory: &mut dyn AdapterFactory,
) -> Result<(), String> {
    let compilation = compile(&[probe.source.as_str()], registry).map_err(|bag| {
        format!(
            "the interleave probe does not compile: {:?}",
            bag.diagnostics()
        )
    })?;
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut partner = factory.make(&compilation.plan);
    if probe.a == probe.b {
        return Err("the interleave payloads must differ, or a swap is invisible".into());
    }
    let data = vec![
        (String::from("a"), Value::Text(String::from(probe.a))),
        (String::from("b"), Value::Text(String::from(probe.b))),
    ];
    let paired = InboundSubmission {
        authority: IngressAuthority::single(
            node::Path::new(["conformance"]).unwrap_or_else(|_| unreachable!("a valid path")),
        ),
        port: node::Port::new("in").unwrap_or_else(|_| unreachable!("a valid port")),
        draft: Draft {
            data: Data::materialized(Value::Struct(data)),
            ext: Namespace::new(),
        },
        sequence: SubmissionSequence::Next,
    };
    vm.inject(&mut journal, paired)
        .map_err(|error| format!("the interleave submission was refused: {error}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        vm.drain(&mut journal)
            .map_err(|error| format!("a step faulted: {error}"))?;
        let report = vm
            .pump(&mut journal, partner.as_mut())
            .map_err(|error| format!("the boundary round faulted: {error}"))?;
        vm.drain(&mut journal)
            .map_err(|error| format!("a step faulted: {error}"))?;
        if vm.is_stable() {
            break;
        }
        if report.settled == 0 {
            if std::time::Instant::now() > deadline {
                return Err("the interleave run wedged".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            vm.advance(&mut journal, partner.as_mut(), vm.now() + 10_000_000)
                .map_err(|error| format!("the advance faulted: {error}"))?;
        }
    }
    let mut closed = 0usize;
    let mut opens_before_first_close = 0usize;
    let mut any_close_seen = false;
    let mut requests = BTreeSet::new();
    for record in journal.records() {
        let Record::Outbound(outbound) = record else {
            continue;
        };
        if matches!(outbound.status, OutboundStatus::Open) {
            if !any_close_seen {
                opens_before_first_close += 1;
            }
            requests.insert(outbound.intent.prepared.bytes().to_vec());
            continue;
        }
        let OutboundStatus::Closed(Closure::Response(outcome)) = &outbound.status else {
            continue;
        };
        any_close_seen = true;
        closed += 1;
        let crate::boundary::CallOutcome::Success(data) = outcome else {
            return Err(format!(
                "interleave call {} closed with a failure; the probe expects echoes",
                outbound.intent.call_ref
            ));
        };
        let request = String::from_utf8_lossy(outbound.intent.prepared.bytes()).into_owned();
        let response = match data {
            Data::Materialized(Value::Text(text)) => text.clone(),
            other => format!("{other:?}"),
        };
        if response != request {
            return Err(format!(
                "call {} answered with another call's payload: request {request:?}, \
                 response {response:?} — the adapter crossed two concurrent replies",
                outbound.intent.call_ref
            ));
        }
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
    Ok(())
}
