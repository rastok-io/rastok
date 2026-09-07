//! End-to-end M1.8 accounting evidence through the real compiler and VM.

#![allow(clippy::disallowed_methods, clippy::unwrap_used, clippy::expect_used)]

use rastok::accounting::{
    CostProfile, CostProfileRef, CostReport, ExactDecimal, MeasureQuantity, UnitPrice,
};
use rastok::boundary::{Draft, InboundEnvelopeProvenance, InboundSubmission, SubmissionSequence};
use rastok::compiler::compile;
use rastok::contract::ContractCatalog;
use rastok::deployment::{
    BootBindings, DeploymentProfile, SecretBinding, SecretMaterial, SecretProvider,
    SecretProviderError,
};
use rastok::envelope::{Data, Namespace, Value};
use rastok::journal::MemJournal;
use rastok::node;
use rastok::plan::GraphVersion;
use rastok::runtime::vm::Vm;
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
fn replay_reconstructs_exact_per_flow_measures_and_provider_costs() {
    let source = r#"
type Usage = struct {
  input_tokens: uint @measure(id = gen_ai.usage.input_tokens)
  cached_tokens: uint @measure(id = gen_ai.usage.cache_read.input_tokens)
  charge: decimal(38,18) @cost(currency = USD, basis = provider_reported)
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
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(
        &mut journal,
        InboundSubmission {
            provenance: InboundEnvelopeProvenance::single(
                node::Path::new(["accounting-test"]).expect("valid provenance"),
            ),
            port: node::Port::new("in").expect("conventional port"),
            draft: Draft {
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
            sequence: SubmissionSequence::Next,
        },
    )
    .expect("inbound commits");
    vm.drain(&mut journal).expect("graph drains");

    let first = CostReport::from_records(&compilation.plan, journal.records(), None)
        .expect("the exact report folds");
    let replay = CostReport::from_records(&compilation.plan, journal.records(), None)
        .expect("the same records replay");
    assert_eq!(first, replay, "no exporter state participates in the fold");

    assert_eq!(first.measures().len(), 4, "two measures at each flow point");
    assert!(first.measures().iter().any(|row| {
        row.scope().port().name() == "in"
            && row.id().as_str() == "gen_ai.usage.input_tokens"
            && row.quantity() == MeasureQuantity::Integer(1_000)
    }));
    assert!(first.measures().iter().any(|row| {
        row.scope().port().name() == "out"
            && row.id().as_str() == "gen_ai.usage.cache_read.input_tokens"
            && row
                .subset_of()
                .is_some_and(|parent| parent.as_str() == "gen_ai.usage.input_tokens")
    }));
    assert_eq!(
        first.costs().len(),
        2,
        "currency remains per-flow, not global"
    );
    assert!(first.costs().iter().all(|row| {
        row.money().currency().as_str() == "USD"
            && row.money().amount() == ExactDecimal::new(125, 2).unwrap()
    }));
    assert_eq!(first.unpriced().len(), 2);
    assert!(first.unpriced().iter().all(|row| {
        row.id().as_str() == "gen_ai.usage.input_tokens"
            && row.quantity() == MeasureQuantity::Integer(1_000)
    }));
}

#[test]
fn a_deployment_price_matches_the_nearest_causal_connector_after_typed_decode() {
    let source = r#"
type Tokens = int @measure(id = gen_ai.usage.input_tokens)

graph priced_call : App {
  input in: int
  output out: Tokens

  connector model : Process {
    mode = job
    command = "cat"
    codec = jsonl
    effect = idempotent
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
    let profile = CostProfile::new(
        CostProfileRef::new("pricing/model@2026-08-30").expect("stable profile ref"),
        vec![
            UnitPrice::new(
                model.clone(),
                "example-ai",
                "model-v1",
                MeasureId::new("gen_ai.usage.input_tokens").expect("registered measure"),
                1_000,
                ExactDecimal::new(2, 0).expect("exact price"),
                CurrencyCode::new("USD").expect("ISO currency"),
            )
            .expect("valid unit price"),
        ],
    )
    .expect("canonical price profile");
    let boot = BootBindings::resolve(
        DeploymentProfile::empty().with_cost_profile(profile.clone()),
        &NoSecrets,
    )
    .expect("the secret-free deployment resolves");
    let version = GraphVersion(1);
    let mut journal = MemJournal::new_with_deployment(version, boot.attestation().clone());
    let mut vm = Vm::boot_with_deployment(&compilation.plan, version, &boot);
    let mut world = rastok::connectors::Process::try_new(&compilation.plan).expect("Process binds");
    vm.inject(
        &mut journal,
        InboundSubmission {
            provenance: InboundEnvelopeProvenance::single(
                node::Path::new(["accounting-test"]).expect("valid provenance"),
            ),
            port: node::Port::new("in").expect("conventional port"),
            draft: Draft {
                data: Data::materialized(Value::Int(1_000)),
                ext: Namespace::new(),
            },
            sequence: SubmissionSequence::Next,
        },
    )
    .expect("inbound commits");
    vm.drain_with_bindings(&mut journal, &mut world)
        .expect("the Call opens");
    for _ in 0..200 {
        if vm.is_stable() && !vm.has_open_obligations() {
            break;
        }
        vm.pump(&mut journal, &mut world)
            .expect("the Driver progresses");
        vm.drain_with_bindings(&mut journal, &mut world)
            .expect("the reply routes");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(vm.is_stable(), "the priced Call must settle");

    let report = CostReport::from_journal(&compilation.plan, &journal, Some(&profile))
        .expect("the attested report folds");
    assert_eq!(report.measures().len(), 1);
    let scope = report.measures()[0].scope();
    assert_eq!(
        scope.node().map(ToString::to_string).as_deref(),
        Some("typed_usage")
    );
    assert_eq!(scope.connector(), Some(&model));
    assert_eq!(scope.provider(), Some("example-ai"));
    assert_eq!(scope.model(), Some("model-v1"));
    assert_eq!(report.costs().len(), 1);
    assert_eq!(
        report.costs()[0].money().amount(),
        ExactDecimal::new(2, 0).expect("exact total")
    );
    assert!(report.unpriced().is_empty());
}
