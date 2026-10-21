//! The opening flow exercises real Contracts with an entirely scripted world.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use rastok::accounting::{ExactDecimal, accounting_observations};
use rastok::compiler::compile;
use rastok::connector::contract::ContractCatalog;
use rastok::deployment::{
    BootBindings, DeploymentProfile, EnvironmentName, PolicyName, PrivacyDeclaration,
    PrivacyDeclarationName, PrivacyRetention, RecipientPolicy, SecretBinding, SecretMaterial,
    SecretName, SecretProvider, SecretProviderError, SecretProviderId, SecretVersion,
    TransferPolicy,
};
use rastok::journal::{CallStatus, EgressRecord, Record};
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
    let policy = |name| PolicyName::new(name).unwrap();
    let privacy = with_privacy.then(|| {
        PrivacyDeclaration::new(
            PrivacyDeclarationName::new("privacy.example/showcase-webhook@1").unwrap(),
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
    let reference = SecretName::new("showcase/partner/token").unwrap();
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
                    SecretProviderId::new("test.local/test@1").unwrap(),
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
        lab::run_program(
            &compilation.plan,
            compilation.hash,
            GraphVersion(1),
            &compilation.labs[0],
            Some(&boot),
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
                    |record| matches!(record, Record::Egress(EgressRecord::Call(outbound))
                    if matches!(outbound.status, CallStatus::Open))
                )
                .count(),
            calls,
            "{}: ACKs are not outbound partner Calls",
            scenario.report.scenario
        );
        for record in &scenario.records {
            if let Record::Ingress(inbound) = record {
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
    for index in [1, 2, 5] {
        let calls: Vec<_> = first[index]
            .records
            .iter()
            .filter_map(|record| match record {
                Record::Egress(EgressRecord::Call(call))
                    if matches!(call.status, CallStatus::Open) =>
                {
                    Some(&call.intent)
                }
                _ => None,
            })
            .collect();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[0].prepared, calls[1].prepared,
            "a retry never changes the request body"
        );
        assert_eq!(calls[0].resolved_settings, calls[1].resolved_settings);
        assert_eq!(calls[0].idempotency_key, calls[1].idempotency_key);
        assert_ne!(calls[0].call_id, calls[1].call_id);
    }
    let failures: Vec<_> = first[2]
        .records
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(tx) if tx.owner_path.to_string() == "deliver.decide" => {
                Some(&tx.emissions)
            }
            _ => None,
        })
        .flatten()
        .filter(|emission| emission.port.name() == "failed")
        .collect();
    assert_eq!(
        failures.len(),
        1,
        "only the final timeout reaches the business adapter"
    );
    let Some(Value::TaggedUnion(failure)) = failures[0].envelope.data().materialized_value() else {
        panic!("typed business failure");
    };
    assert_eq!(failure.case, "PartnerFailed");
    let Some((_, Value::TaggedUnion(terminal))) =
        failure.payload.iter().find(|(name, _)| name == "terminal")
    else {
        panic!("the original terminal remains typed");
    };
    assert_eq!(
        terminal.case, "TimeoutExpired",
        "no Retry/Loop exhaustion wrapper"
    );
}

#[test]
fn a_secret_binding_does_not_waive_the_required_privacy_attestation() {
    let compilation = compile(&[SOURCE], &ContractCatalog::try_m0().unwrap()).unwrap();
    for run in lab::run_program(
        &compilation.plan,
        compilation.hash,
        GraphVersion(1),
        &compilation.labs[0],
        Some(&boot(false)),
    ) {
        assert!(!run.report.passed(), "privacy coverage is mandatory");
        assert!(
            run.report
                .failures
                .iter()
                .any(|reason| reason.contains("personal-data")
                    && reason.contains("deployment boundary")),
            "refusal must name privacy, not an unrelated boot failure: {:?}",
            run.report
        );
        assert!(
            run.records.is_empty(),
            "refusal precedes admission and effects"
        );
    }
}

#[test]
fn charges_are_exact_reported_evidence_not_an_invented_price_for_timeouts() {
    let compilation = compile(&[SOURCE], &ContractCatalog::try_m0().unwrap()).unwrap();
    assert!(
        compilation
            .plan
            .nodes
            .iter()
            .all(|owner| owner.loop_spec().is_none())
    );
    assert_eq!(
        compilation
            .plan
            .nodes
            .iter()
            .filter(|owner| owner.retry_spec().is_some())
            .count(),
        1
    );
    let runs = lab::run_program(
        &compilation.plan,
        compilation.hash,
        GraphVersion(1),
        &compilation.labs[0],
        Some(&boot(true)),
    );
    for (index, cents) in [
        (0, vec![2]),
        (1, vec![2]),
        (2, vec![]),
        (4, vec![]),
        (5, vec![1, 2]),
    ] {
        assert!(runs[index].report.passed(), "{:?}", runs[index].report);
        let observations =
            accounting_observations(&compilation.plan, &runs[index].records, None).unwrap();
        let mut amounts: Vec<_> = observations
            .iter()
            .flat_map(|observation| observation.measures())
            .map(|row| row.quantity())
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
        for observation in observations
            .iter()
            .filter(|observation| !observation.measures().is_empty())
        {
            assert!(
                observation
                    .measures()
                    .iter()
                    .all(|sample| sample.unit() == "USD")
            );
            assert_eq!(
                observation.scope().connector().unwrap().to_string(),
                "deliver.partner",
                "attribution follows the sealed producer lineage"
            );
            assert_eq!(
                observation.scope().node().unwrap().to_string(),
                "deliver.partner",
                "typed evidence belongs to the Connector, not a downstream extractor"
            );
        }
    }
    let mut batches = Vec::new();
    for record in &runs[5].records {
        if let Record::Transaction(tx) = record {
            for fact in tx.retry_record.iter().flat_map(|packet| &packet.facts) {
                match fact {
                    rastok::journal::retry_protocol::Fact::Held(batch) => {
                        assert!(
                            tx.routes.is_empty(),
                            "the 500 charge is retained but not delivered"
                        );
                        batches.push((batch, "held"));
                    }
                    rastok::journal::retry_protocol::Fact::Published(batch) => {
                        batches.push((batch, "published"))
                    }
                    _ => {}
                }
            }
        }
    }
    assert_eq!(batches.len(), 2);
    assert_eq!((batches[0].1, batches[1].1), ("held", "published"));
    assert_eq!(
        batches[0].0.attempt.operation,
        batches[1].0.attempt.operation
    );
    assert_eq!(
        (batches[0].0.attempt.ordinal, batches[1].0.attempt.ordinal),
        (1, 2)
    );
}

#[cfg(feature = "hosted-ui")]
#[test]
fn every_webhook_transition_redacts_personal_data_and_the_secret_before_presentation() {
    let compilation = compile(&[SOURCE], &ContractCatalog::try_m0().unwrap()).unwrap();
    let boot = boot(true);
    let simulation = &compilation.labs[0].simulations[0];
    let expected = lab::run_program(
        &compilation.plan,
        compilation.hash,
        GraphVersion(1),
        &compilation.labs[0],
        Some(&boot),
    );
    for (scenario, batch) in simulation.scenarios.iter().zip(&expected) {
        let mut session = lab::Simulator::try_new_program(
            &compilation.plan,
            compilation.hash,
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
                execution: session.execution(),
                journal: session.journal(),
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
        assert_eq!(
            &session.finish(),
            batch,
            "same records in batch and interactive inspection"
        );
    }
}
