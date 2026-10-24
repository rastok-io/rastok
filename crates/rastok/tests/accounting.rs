//! End-to-end M1.8 accounting evidence through the real compiler and Execution.

#![allow(clippy::disallowed_methods, clippy::unwrap_used, clippy::expect_used)]

#[path = "support/call_driver.rs"]
mod call_driver;
use call_driver::CallDriver;

use rastok::accounting::{
    ExactDecimal, PriceBook, PriceBookName, UnitPrice, accounting_observations,
};
use rastok::compiler::compile;
use rastok::connector::contract::ContractCatalog;
use rastok::connector::exchange::EnvelopeDraft;
use rastok::connector::exchange::IngressProposal;
use rastok::connector::exchange::IngressProvenance;
use rastok::connector::exchange::IngressSequence;
use rastok::deployment::{
    BootBindings, DeploymentProfile, SecretBinding, SecretMaterial, SecretProvider,
    SecretProviderError,
};
use rastok::envelope::{Data, Namespace, Value};
use rastok::journal::MemJournal;
use rastok::node;
use rastok::plan::GraphVersion;
use rastok::runtime::execution::Execution;
use rastok::schema_annotation::{CurrencyCode, MeasureId};

struct NoSecrets;

impl SecretProvider for NoSecrets {
    fn resolve(&self, _binding: &SecretBinding) -> Result<SecretMaterial, SecretProviderError> {
        Err(SecretProviderError::ProviderFailure(
            "the accounting probe declares no secret",
        ))
    }
}

#[test]
fn replay_reads_each_original_measurement_without_combining_flow_points() {
    let source = r#"
type Usage = struct {
  input_tokens: uint @measure(id = input_tokens, unit = "token")
  cached_tokens: uint @measure(id = cached_tokens, unit = "token")
  charge: decimal(38,18) @measure(id = amount, unit = "USD")
}

graph accounting_probe : App {
  input in: Usage
  output out: Usage
  operator copy {
    input in: Usage
    output out: Usage
    on in { out = data }
  }
  in -> copy -> out
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the annotated probe compiles");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
    execution
        .accept_input(
            &mut journal,
            IngressProposal {
                provenance: IngressProvenance::single(
                    node::Path::new(["accounting-test"]).expect("valid provenance"),
                ),
                draft: EnvelopeDraft {
                    port: node::Port::new("in").expect("conventional port"),
                    data: Data::materialized(Value::Struct(vec![
                        (String::from("input_tokens"), Value::Uint(1_000)),
                        (String::from("cached_tokens"), Value::Uint(200)),
                        (
                            String::from("charge"),
                            Value::Decimal {
                                unscaled: 1_250_000_000_000_000_000,
                                scale: 18,
                            },
                        ),
                    ])),
                    ext: Namespace::new(),
                },
                sequence: IngressSequence::Next,
                session: None,
            },
        )
        .expect("inbound commits");
    execution.run_ready(&mut journal).expect("graph drains");

    let first = accounting_observations(&compilation.plan, journal.records(), None)
        .expect("individual typed observations");
    let retained = MemJournal::from_records(
        GraphVersion(1),
        rastok::time::Timestamp::UNIX_EPOCH,
        None,
        journal.records(),
    )
    .unwrap();
    let replay = accounting_observations(&compilation.plan, retained.records(), None)
        .expect("the same original records remain readable");
    assert_eq!(first, replay);
    assert_eq!(first.len(), 2, "one admission and one copy emission");
    assert_ne!(first[0].envelope_id(), first[1].envelope_id());
    assert_eq!(first[0].scope().origin_id(), first[1].scope().origin_id());
    for (observation, port) in first.iter().zip(["in", "out"]) {
        assert_eq!(observation.scope().port().name(), port);
        assert_eq!(observation.measures().len(), 3);
        assert!(observation.measures().iter().any(|sample| {
            sample.id().as_str() == "input_tokens"
                && sample.quantity() == ExactDecimal::new(1_000, 0).unwrap()
        }));
        assert!(observation.measures().iter().any(|sample| {
            sample.id().as_str() == "cached_tokens"
                && sample.quantity() == ExactDecimal::new(200, 0).unwrap()
        }));
        let amount = observation
            .measures()
            .iter()
            .find(|sample| sample.id().as_str() == "amount")
            .expect("the retained charge");
        assert_eq!(amount.unit(), "USD");
        assert_eq!(amount.quantity(), ExactDecimal::new(125, 2).unwrap());
    }
}

#[test]
fn independent_quantities_do_not_imply_a_subset_relationship() {
    let compilation = compile(
        &[r#"
type Usage = struct {
  input_tokens: uint @measure(id = input_tokens, unit = "token")
  cached_tokens: uint @measure(id = cached_tokens, unit = "token")
}
graph app : App {
  input in: Usage
  in -> nothing
}
lab app { simulation usage on app { scenario original_values {
  app.in = { input_tokens = 10u; cached_tokens = 20u }
} } }
"#],
        &ContractCatalog::try_m0().unwrap(),
    )
    .unwrap();
    let runs = rastok::lab::run(&compilation.plan, GraphVersion(1), &compilation.labs[0]);
    assert!(runs[0].report.passed(), "{:?}", runs[0].report);
    let observations = accounting_observations(&compilation.plan, &runs[0].records, None)
        .expect("M1 retains original values without enforcing a report relationship");
    assert_eq!(observations.len(), 1);
    let values: Vec<_> = observations[0]
        .measures()
        .iter()
        .map(|sample| (sample.id().as_str(), sample.quantity()))
        .collect();
    assert_eq!(
        values,
        [
            ("cached_tokens", ExactDecimal::new(20, 0).unwrap()),
            ("input_tokens", ExactDecimal::new(10, 0).unwrap()),
        ]
    );
}

#[test]
fn typed_decode_preserves_connector_attribution_without_applying_deployment_prices() {
    let source = r#"
type Tokens = int @measure(id = input_tokens, unit = "token")

graph priced_call : App {
  input in: int
  output out: Tokens

  connector model : Process {
    mode = job
    command = "cat"
    codec = jsonl
    idempotent = true
    behavior = deterministic
    timeout = duration("5s")
  }

  operator typed_usage {
    input in: variant
    output out: Tokens
    on in { out = data as int }
  }

  in -> model -> typed_usage -> out
  model.err -> nothing
  model.exit -> nothing
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the typed Connector probe compiles");
    let model = node::Path::new(["model"]).expect("canonical Connector path");
    let profile = PriceBook::new(
        PriceBookName::new("pricing.example/model@2026-08-30").expect("stable profile ref"),
        vec![
            UnitPrice::new(
                model.clone(),
                "example-ai",
                "model-v1",
                MeasureId::new("input_tokens").expect("authored measure id"),
                1_000,
                ExactDecimal::new(2, 0).expect("exact price"),
                CurrencyCode::new("USD").expect("ISO currency"),
            )
            .expect("valid unit price"),
        ],
    )
    .expect("canonical price profile");
    let boot = BootBindings::resolve(
        DeploymentProfile::empty().with_price_book(profile.clone()),
        &NoSecrets,
    )
    .expect("the secret-free deployment resolves");
    let version = GraphVersion(1);
    let mut journal = MemJournal::new_with_deployment(
        version,
        rastok::time::Timestamp::UNIX_EPOCH,
        boot.attestation().clone(),
    );
    let mut execution = Execution::new_with_deployment(&compilation.plan, version, &boot);
    let mut world =
        rastok::connector::drivers::Process::try_new(&compilation.plan).expect("Process binds");
    execution
        .accept_input(
            &mut journal,
            IngressProposal {
                provenance: IngressProvenance::single(
                    node::Path::new(["accounting-test"]).expect("valid provenance"),
                ),
                draft: EnvelopeDraft {
                    port: node::Port::new("in").expect("conventional port"),
                    data: Data::materialized(Value::Int(1_000)),
                    ext: Namespace::new(),
                },
                sequence: IngressSequence::Next,
                session: None,
            },
        )
        .expect("inbound commits");
    execution
        .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
        .expect("the Call opens");
    for _ in 0..200 {
        if execution.is_stable() && !execution.has_open_obligations() {
            break;
        }
        execution
            .poll_drivers(&mut journal, &mut CallDriver(&mut world))
            .expect("the Driver progresses");
        execution
            .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut world))
            .expect("the reply routes");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(execution.is_stable(), "the priced Call must settle");

    let observations =
        accounting_observations(&compilation.plan, journal.records(), Some(&profile))
            .expect("retained values and causal attribution");
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].measures().len(), 1);
    assert_eq!(
        observations[0].measures()[0].quantity(),
        ExactDecimal::new(1_000, 0).unwrap()
    );
    let scope = observations[0].scope();
    assert_eq!(
        scope.node().map(ToString::to_string).as_deref(),
        Some("typed_usage")
    );
    assert_eq!(scope.connector(), Some(&model));
    assert_eq!(scope.provider(), Some("example-ai"));
    assert_eq!(scope.model(), Some("model-v1"));
    assert!(
        observations[0]
            .measures()
            .iter()
            .all(|sample| sample.unit() == "token"),
        "a tariff is not an observed charge"
    );
}
