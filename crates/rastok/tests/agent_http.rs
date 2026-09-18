//! Responses is a typed HTTP adapter, not a pricing engine or an idempotence claim.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use rastok::accounting::accounting_observations;
use rastok::compiler::{Compilation, compile};
use rastok::contract::ContractCatalog;
use rastok::deployment::{
    BootBindings, DeploymentProfile, EnvironmentName, SecretBinding, SecretMaterial,
    SecretProvider, SecretProviderError, SecretProviderId, SecretRef, SecretVersion,
};
use rastok::envelope::Value;
use rastok::journal::{MemJournal, OutboundRecord, OutboundStatus, Record};
use rastok::plan::GraphVersion;
use rastok::recording::Recording;

const SOURCE: &str = include_str!("../../../examples/agent-http.rkg");
const SECRET: &[u8] = b"responses-secret-canary-not-a-real-credential";

struct FixtureSecrets;
impl SecretProvider for FixtureSecrets {
    fn resolve(&self, binding: &SecretBinding) -> Result<SecretMaterial, SecretProviderError> {
        assert_eq!(binding.reference().as_str(), "showcase/model/token");
        SecretMaterial::new(SECRET.to_vec())
    }
}

fn boot() -> BootBindings {
    let reference = SecretRef::new("showcase/model/token").unwrap();
    BootBindings::resolve(
        DeploymentProfile::new(
            BTreeMap::from([(
                EnvironmentName::new("MODEL_URL").unwrap(),
                Value::Text("https://api.openai.com/v1/responses".into()),
            )]),
            BTreeMap::from([(
                reference.clone(),
                SecretBinding::new(
                    reference,
                    SecretProviderId::new("rastok/test@1").unwrap(),
                    "fixture-token",
                    SecretVersion::new("fixture-v1").unwrap(),
                )
                .unwrap(),
            )]),
            None,
        )
        .unwrap(),
        &FixtureSecrets,
    )
    .unwrap()
}

fn checked(source: &str) -> Compilation {
    compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap_or_else(|bag| panic!("{bag:#?}"))
}

#[test]
fn responses_keep_original_job_samples_without_inventing_missing_usage_or_prices() {
    let compilation = checked(SOURCE);
    let boot = boot();
    let run = || {
        rastok::lab::run_program(
            &compilation.plan,
            compilation.hash,
            GraphVersion(1),
            &compilation.labs[0],
            Some(&boot),
        )
    };
    let runs = run();
    assert_eq!(runs, run());
    assert_eq!(runs.len(), 6);
    assert!(
        compilation
            .plan
            .owners
            .iter()
            .all(|owner| owner.retry_spec().is_none())
    );
    for ((run, count), calls) in runs.iter().zip([3, 0, 1, 0, 0, 0]).zip([3, 2, 1, 1, 1, 2]) {
        assert!(run.report.passed(), "{:?}", run.report);
        let samples = accounting_observations(&compilation.plan, &run.records, None).unwrap();
        assert_eq!(samples.len(), count, "{:?}", run.report.scenario);
        assert_eq!(run.records.iter().filter(|record| matches!(record,
            Record::Outbound(OutboundRecord::Call(call)) if matches!(call.status, OutboundStatus::Open))).count(), calls);
        for sample in &samples {
            assert!(
                sample
                    .measures()
                    .iter()
                    .all(|measurement| measurement.unit() == "token"),
                "tokens are not a currency amount"
            );
            assert_eq!(sample.measures().len(), 3);
            assert_eq!(sample.scope().connector().unwrap().to_string(), "model");
            assert_eq!(sample.scope().port().name(), "out");
            let ids: Vec<_> = sample
                .measures()
                .iter()
                .map(|measure| measure.id().as_str())
                .collect();
            assert_eq!(ids, ["input_tokens", "output_tokens", "total_tokens"]);
            assert!(
                sample
                    .measures()
                    .iter()
                    .all(|measure| measure.unit() == "token")
            );
        }
        let trace = rastok::golden::project(&run.records);
        for index in 0..trace.len() {
            assert!(
                !trace
                    .record_bytes(index)
                    .unwrap()
                    .windows(SECRET.len())
                    .any(|bytes| bytes == SECRET)
            );
        }
        assert!(!format!("{:?}", run.records).contains(std::str::from_utf8(SECRET).unwrap()));
    }
    let samples = accounting_observations(&compilation.plan, &runs[0].records, None).unwrap();
    assert_eq!(
        samples[0].scope().instance(),
        samples[2].scope().instance(),
        "Alice reuses her instance"
    );
    assert_ne!(
        samples[0].scope().instance(),
        samples[1].scope().instance(),
        "Bob stays isolated"
    );
    assert_ne!(
        samples[0].scope().origin_id(),
        samples[2].scope().origin_id(),
        "two jobs, not one aggregate"
    );
    for sample in samples {
        assert_eq!(
            sample
                .measures()
                .iter()
                .map(|measure| measure.quantity().to_string())
                .collect::<Vec<_>>(),
            ["12", "4", "16"]
        );
    }
    let zero = accounting_observations(&compilation.plan, &runs[2].records, None).unwrap();
    assert!(
        zero[0]
            .measures()
            .iter()
            .all(|measure| measure.quantity().to_string() == "0")
    );
}

#[test]
fn projected_plan_and_samples_survive_every_recording_prefix_without_a_driver() {
    // Boot-dependent recordings deliberately remain refused in M1. This twin
    // qualifies the codec/recording composition without pretending to persist credentials.
    let source = SOURCE
        .replace(
            "url = env(\"MODEL_URL\")",
            "url = \"https://example.test/responses\"",
        )
        .replace("    auth_bearer = secret(\"showcase/model/token\")\n", "");
    let compilation = checked(&source);
    for run in rastok::lab::run_program(
        &compilation.plan,
        compilation.hash,
        GraphVersion(1),
        &compilation.labs[0],
        None,
    ) {
        assert!(run.report.passed(), "{:?}", run.report);
        let journal = MemJournal::from_records(GraphVersion(1), None, &run.records).unwrap();
        let recording = Recording::capture(&compilation, &journal, 10_000_000_000).unwrap();
        let decoded = Recording::decode(&recording.encode().unwrap()).unwrap();
        assert_eq!(decoded.plan(), &compilation.plan);
        assert_eq!(decoded.records(), run.records);
        for through in 0..=run.records.len() {
            let (prefix, _) = decoded.inspect(through).unwrap_or_else(|error| {
                panic!("{} prefix {through}: {error}", run.report.scenario)
            });
            assert_eq!(prefix.records(), &run.records[..through]);
            assert_eq!(
                accounting_observations(decoded.plan(), prefix.records(), None).unwrap(),
                accounting_observations(&compilation.plan, &run.records[..through], None).unwrap()
            );
        }
    }
    assert!(
        Recording::capture(&checked(SOURCE), &MemJournal::new(GraphVersion(1)), 0).is_err(),
        "no hidden new boot-recording capability"
    );
}

#[cfg(feature = "hosted-ui")]
#[test]
fn responses_debugger_projects_each_transition_without_materializing_the_bearer() {
    use rastok::ui::{ExecutionState, SnapshotInput, snapshot};
    let compilation = checked(SOURCE);
    let boot = boot();
    let simulation = &compilation.labs[0].simulations[0];
    for scenario in &simulation.scenarios {
        let mut session = rastok::lab::ScenarioSession::try_new_program(
            &compilation.plan,
            compilation.hash,
            GraphVersion(1),
            &simulation.name,
            &simulation.mocks,
            scenario,
            Some(&boot),
        )
        .unwrap();
        for transition in 0..100 {
            let view = snapshot(SnapshotInput {
                revision: transition,
                mode: "simulate",
                state: ExecutionState::Paused,
                plan: &compilation.plan,
                definitions: &compilation.source_definitions,
                vm: session.vm(),
                journal: session.journal(),
                fault: None,
                activity: None,
                scenario: None,
                session: 0,
                boundary: transition,
            })
            .unwrap();
            assert!(!format!("{view:?}").contains(std::str::from_utf8(SECRET).unwrap()));
            if session.is_complete() {
                break;
            }
            if session.is_quiescent() {
                session.next_action().unwrap();
            } else {
                session.step_transaction().unwrap();
            }
        }
        assert!(session.is_complete());
        assert!(session.finish().report.passed());
    }
}
