//! The opening flow exercises real Contracts with an entirely scripted world.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use rastok::accounting::{CostReport, ExactDecimal};
use rastok::compiler::compile;
use rastok::contract::ContractCatalog;
use rastok::deployment::{
    BootBindings, DeploymentProfile, EnvironmentName, PolicyRef, PrivacyProfile, PrivacyProfileRef,
    PrivacyRetention, RecipientPolicy, SecretBinding, SecretMaterial, SecretProvider,
    SecretProviderError, SecretProviderId, SecretRef, SecretVersion, TransferPolicy,
};
use rastok::journal::{OutboundRecord, OutboundStatus, Record};
use rastok::lab;
use rastok::plan::GraphVersion;
use rastok::value::Value;

const SOURCE: &str = include_str!("../../../examples/webhook-to-partner.rkg");
const SECRET: &[u8] = b"webhook-secret-canary-never-journalled";

struct FixtureSecrets;

impl SecretProvider for FixtureSecrets {
    fn resolve(&self, binding: &SecretBinding) -> Result<SecretMaterial, SecretProviderError> {
        assert_eq!(binding.reference().as_str(), "showcase/partner/token");
        SecretMaterial::new(SECRET.to_vec())
    }
}

fn boot(with_privacy: bool) -> BootBindings {
    let policy = |name| PolicyRef::new(name).unwrap();
    let privacy = with_privacy.then(|| {
        PrivacyProfile::new(
            PrivacyProfileRef::new("privacy/showcase-webhook@1").unwrap(),
            policy("showcase/fictional-webhook"),
            vec![policy("purpose/local-synthetic-demonstration")],
            vec![policy("basis/synthetic-fixture-not-production-authority")],
            None,
            None,
            RecipientPolicy::Classes(vec![policy("recipient/simulated-partner")]),
            TransferPolicy::None,
            PrivacyRetention::RunScoped,
        )
        .unwrap()
    });
    let reference = SecretRef::new("showcase/partner/token").unwrap();
    BootBindings::resolve(
        DeploymentProfile::new(
            BTreeMap::from([(
                EnvironmentName::new("PARTNER_URL").unwrap(),
                Value::Text("https://partner.example.test/events/upsert".into()),
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
            privacy,
        )
        .unwrap(),
        &FixtureSecrets,
    )
    .unwrap()
}

#[test]
fn the_six_webhook_scenarios_are_deterministic_and_count_real_outbound_calls() {
    let compilation = compile(&[SOURCE], &ContractCatalog::try_m0().unwrap())
        .unwrap_or_else(|bag| panic!("the flagship checks: {bag:#?}"));
    let boot = boot(true);
    let run = || {
        lab::run_with_boot(
            &compilation.plan,
            GraphVersion(1),
            &compilation.labs[0],
            &boot,
        )
    };
    let first = run();
    assert_eq!(first, run(), "same world-half, same full journal");
    assert_eq!(first.len(), 6);
    for (scenario, calls) in first.iter().zip([1, 2, 2, 2, 0, 2]) {
        assert!(scenario.report.passed(), "{:#?}", scenario.report);
        assert_eq!(
            scenario
                .records
                .iter()
                .filter(
                    |record| matches!(record, Record::Outbound(OutboundRecord::Call(outbound))
                    if matches!(outbound.status, OutboundStatus::Open))
                )
                .count(),
            calls,
            "{}: ACKs are not outbound partner Calls",
            scenario.report.scenario
        );
        for record in &scenario.records {
            if let Record::Inbound(inbound) = record {
                assert_eq!(inbound.provenance.connector_path().to_string(), "listener");
                assert_eq!(inbound.port.name(), "out");
            }
        }
        let trace = rastok::golden::project(&scenario.records);
        for index in 0..trace.len() {
            assert!(
                !trace
                    .record_bytes(index)
                    .unwrap()
                    .windows(SECRET.len())
                    .any(|w| w == SECRET),
                "physical credentials must never enter the canonical journal"
            );
        }
    }
}

#[test]
fn a_secret_binding_does_not_waive_the_required_privacy_attestation() {
    let compilation = compile(&[SOURCE], &ContractCatalog::try_m0().unwrap()).unwrap();
    for run in lab::run_with_boot(
        &compilation.plan,
        GraphVersion(1),
        &compilation.labs[0],
        &boot(false),
    ) {
        assert!(!run.report.passed(), "privacy coverage is mandatory");
        assert!(
            run.records.is_empty(),
            "refusal precedes admission and effects"
        );
    }
}

#[test]
fn charges_are_exact_reported_evidence_not_an_invented_price_for_timeouts() {
    let compilation = compile(&[SOURCE], &ContractCatalog::try_m0().unwrap()).unwrap();
    let runs = lab::run_with_boot(
        &compilation.plan,
        GraphVersion(1),
        &compilation.labs[0],
        &boot(true),
    );
    for (index, cents) in [
        (0, vec![2]),
        (1, vec![2]),
        (2, vec![]),
        (4, vec![]),
        (5, vec![3]),
    ] {
        assert!(runs[index].report.passed(), "{:?}", runs[index].report);
        let report =
            CostReport::from_records(&compilation.plan, &runs[index].records, None).unwrap();
        let mut amounts: Vec<_> = report
            .costs()
            .iter()
            .map(|row| row.money().amount())
            .collect();
        amounts.sort_by_key(|amount| amount.unscaled());
        assert_eq!(
            amounts,
            cents
                .into_iter()
                .map(|cent| ExactDecimal::new(cent, 2).unwrap())
                .collect::<Vec<_>>(),
            "scenario {index}"
        );
        for row in report.costs() {
            assert_eq!(row.money().currency().as_str(), "USD");
            assert_eq!(
                row.scope().connector().unwrap().to_string(),
                "deliver.partner",
                "attribution follows the sealed producer lineage"
            );
        }
    }
}

#[cfg(feature = "hosted-ui")]
#[test]
fn every_webhook_transition_redacts_personal_data_and_the_secret_before_presentation() {
    let compilation = compile(&[SOURCE], &ContractCatalog::try_m0().unwrap()).unwrap();
    let boot = boot(true);
    let simulation = &compilation.labs[0].simulations[0];
    for scenario in &simulation.scenarios {
        let mut session = lab::ScenarioSession::try_new(
            &compilation.plan,
            GraphVersion(1),
            &simulation.name,
            &simulation.mocks,
            scenario,
            Some(&boot),
        )
        .unwrap();
        for transition in 0..500 {
            let view = rastok::ui::snapshot(rastok::ui::SnapshotInput {
                revision: transition,
                mode: "simulate",
                state: rastok::ui::ExecutionState::Paused,
                plan: &compilation.plan,
                definitions: &compilation.source_definitions,
                vm: session.vm(),
                journal: session.journal(),
                cost_profile: None,
                fault: None,
                activity: None,
                scenario: None,
                session: 0,
                boundary: transition,
            })
            .unwrap();
            let rendered = format!("{view:?}");
            assert!(
                !rendered.contains("alice@example.test"),
                "transition {transition}: {rendered}"
            );
            assert!(
                !rendered
                    .as_bytes()
                    .windows(SECRET.len())
                    .any(|w| w == SECRET)
            );
            if session.is_complete() {
                break;
            }
            if session.is_quiescent() {
                session.next_action().unwrap();
            } else {
                session.step_transaction().unwrap();
            }
        }
        assert!(session.is_complete(), "the finite scenario must terminate");
        assert!(session.finish().report.passed());
    }
}
