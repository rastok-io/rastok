//! Business quantities retain their own ids, units and exact signed values.

#![allow(clippy::expect_used, clippy::panic)]

use rastok::accounting::{
    ExactDecimal, PriceBook, PriceBookName, UnitPrice, accounting_observations,
};
use rastok::compiler::{Compilation, compile};
use rastok::connector::contract::ContractCatalog;
use rastok::connector::exchange::EnvelopeDraft;
use rastok::connector::exchange::IngressProposal;
use rastok::connector::exchange::IngressProvenance;
use rastok::connector::exchange::IngressSequence;
use rastok::envelope::{Data, Namespace, Value};
use rastok::journal::MemJournal;
use rastok::plan::{self, GraphVersion};
use rastok::runtime::execution::Execution;
use rastok::{diagnostics::Bag, node};

fn checked(ty: &str, arguments: &str, body: &str) -> Result<Compilation, Bag> {
    compile(
        &[&format!(
            r#"
type Quantity = {ty} @measure({arguments})
graph adjustments : App {{
  input in: int
  output out: Quantity
  operator count {{
    input in: int
    output out: Quantity
    on in {{ {body} }}
  }}
  in -> count -> out
}}
"#
        )],
        &ContractCatalog::try_m0().expect("standard catalogue"),
    )
}

fn execute(compiled: &Compilation) -> MemJournal {
    let version = GraphVersion(1);
    let mut journal = MemJournal::new(version);
    let mut vm = Execution::new(&compiled.plan, version);
    vm.accept_input(
        &mut journal,
        IngressProposal {
            provenance: IngressProvenance::single(
                node::Path::new(["business-test"]).expect("provenance"),
            ),
            draft: EnvelopeDraft {
                port: node::Port::new("in").expect("input"),
                data: Data::materialized(Value::Int(0)),
                ext: Namespace::new(),
            },
            sequence: IngressSequence::Next,
            session: None,
        },
    )
    .expect("admission");
    vm.run_ready(&mut journal).expect("atomic signed emissions");
    journal
}

#[test]
fn quantity_ids_never_supply_an_implicit_unit_or_carrier() {
    for id in [
        "tokens",
        "gen_ai.usage.input_tokens",
        "gen_ai.client.token.usage",
    ] {
        let error = checked("int", &format!("id = {id}"), "nothing")
            .expect_err("every quantity needs an authored unit, even a former standard name");
        let diagnostic = format!("{error:?}");
        assert!(diagnostic.contains("E-DATA-ANNOTATION"), "{diagnostic}");
        assert!(diagnostic.contains("unit"), "{diagnostic}");

        let compiled = checked(
            "decimal(38,18)",
            &format!("id = {id}, unit = \"kg\""),
            "out = decimal(\"-1.25\") as Quantity",
        )
        .expect("an id does not reserve a unit, carrier or runtime instrument");
        let journal = execute(&compiled);
        let observations = accounting_observations(&compiled.plan, journal.records(), None)
            .expect("original signed quantity");
        let sample = &observations[0].measures()[0];
        assert_eq!(sample.id().as_str(), id);
        assert_eq!(sample.quantity().to_string(), "-1.25");
        assert_eq!(sample.unit(), "kg");
    }
}

#[test]
fn integer_and_decimal_adjustments_keep_each_exact_signed_sample() {
    for (ty, body, expected) in [
        ("int", "out = 10; out = -3", ["10", "-3"]),
        ("int", "out = 3; out = -3", ["3", "-3"]),
        ("int", "out = 3; out = -10", ["3", "-10"]),
        ("int", "out = 0; out = 3", ["0", "3"]),
        (
            "decimal(38,18)",
            "out = decimal(\"10.125\") as Quantity; out = decimal(\"-3.375\") as Quantity",
            ["10.125", "-3.375"],
        ),
    ] {
        let compiled = checked(ty, "id = carrots, unit = \"item\"", body)
            .expect("a typed, additive business quantity");
        let journal = execute(&compiled);
        let observations = accounting_observations(&compiled.plan, journal.records(), None)
            .expect("signed original evidence");
        assert_eq!(observations.len(), 2);
        assert_ne!(observations[0].envelope_id(), observations[1].envelope_id());
        for (observation, expected) in observations.iter().zip(expected) {
            assert_eq!(observation.measures().len(), 1);
            assert_eq!(observation.measures()[0].id().as_str(), "carrots");
            assert_eq!(observation.measures()[0].quantity().to_string(), expected);
            assert_eq!(observation.measures()[0].unit(), "item");
            assert!(
                observation
                    .measures()
                    .iter()
                    .all(|sample| sample.unit() == "item"),
                "a decimal is not implicitly money"
            );
        }
        let (decoded, hash) = plan::decode(&plan::encode(&compiled.plan, &compiled.hash))
            .expect("independent binding and target validation");
        assert_eq!(hash, compiled.hash);
        assert_eq!(decoded, compiled.plan);
        assert_eq!(
            observations,
            accounting_observations(&decoded, journal.records(), None).expect("retained samples")
        );
    }
}

#[test]
fn argument_order_is_identity_neutral_but_ids_and_units_are_not() {
    let implicit =
        checked("int", "id = carrots, unit = \"item\"", "out = 1").expect("explicit unit");
    let explicit =
        checked("int", "unit = \"item\", id = carrots", "out = 1").expect("reordered arguments");
    assert_eq!(implicit.hash, explicit.hash);
    assert_eq!(
        plan::encode(&implicit.plan, &implicit.hash),
        plan::encode(&explicit.plan, &explicit.hash)
    );
    for arguments in [
        "id = cabbages, unit = \"item\"",
        "id = carrots, unit = \"kg\"",
    ] {
        assert_ne!(
            implicit.hash,
            checked("int", arguments, "out = 1")
                .expect("distinct meaning")
                .hash
        );
    }
}

#[test]
fn incompatible_business_profiles_refuse_instead_of_dropping_arguments() {
    for (ty, arguments, detail) in [
        ("int", "id = carrots", "unit"),
        (
            "text",
            "id = carrots, unit = \"item\"",
            "integer or exact decimal",
        ),
        (
            "float",
            "id = carrots, unit = \"item\"",
            "integer or exact decimal",
        ),
        ("int", "id = carrots, unit = \"\"", "unit"),
        ("int", "id = carrots, unit = \" item \"", "unit"),
        (
            "int",
            "id = carrots, unit = \"item\", aggregation = last",
            "aggregation",
        ),
        (
            "int",
            "id = carrots, unit = \"item\", subset_of = stock",
            "subset",
        ),
        (
            "int",
            "id = carrots, unit = \"item\", provider = example",
            "provider",
        ),
    ] {
        let error = checked(ty, arguments, "nothing").expect_err("unsupported measurement");
        assert!(format!("{error:?}").contains(detail), "{error:?}");
    }
}

#[test]
fn monetary_and_quantity_profiles_cannot_duplicate_the_same_measure_definition() {
    let error = checked(
        "decimal(38,18) @measure(id = amount, unit = \"EUR\")",
        "id = carrots, unit = \"kg\"",
        "out = decimal(\"1\") as Quantity",
    )
    .expect_err("one @measure application per resolved path");
    assert!(format!("{error:?}").contains("appears twice"), "{error:?}");
    let source = r#"
type Charge = decimal(38,18) @measure(id = amount, unit = "EUR")
type Weight = int @measure(id = amount, unit = "kg")
graph app : App {
  input in: Charge
  input other: Weight
  in -> nothing
  other -> nothing
}
"#;
    let error = compile(&[source], &ContractCatalog::try_m0().expect("catalogue"))
        .expect_err("one id cannot alternate between money and an unrelated unit");
    assert!(
        format!("{error:?}").contains("inconsistent units"),
        "{error:?}"
    );
}

#[test]
fn individual_values_are_retained_even_if_their_sum_would_overflow() {
    let compiled = checked(
        "decimal(38,0)",
        "id = balance, unit = \"item\"",
        "out = decimal(\"99999999999999999999999999999999999999\") as Quantity; out = decimal(\"1\") as Quantity",
    ).expect("each individual value fits");
    let journal = execute(&compiled);
    let observations = accounting_observations(&compiled.plan, journal.records(), None)
        .expect("inspection does not impose an aggregate domain on independent values");
    let values: Vec<_> = observations
        .iter()
        .map(|observation| observation.measures()[0].quantity().to_string())
        .collect();
    assert_eq!(values, ["99999999999999999999999999999999999999", "1"]);
}

#[test]
fn one_business_id_cannot_change_unit_in_a_plan_or_in_a_forged_package() {
    let compiled = checked("int", "id = carrots, unit = \"item\"", "out = 1").expect("one unit");
    let different =
        checked("int", "id = carrots, unit = \"kg\"", "out = 1").expect("a distinct valid program");
    let mut forged = compiled.plan.plan().clone();
    let mut extra = different
        .plan
        .plan()
        .boundary()
        .iter()
        .find(|port| port.port.name() == "out")
        .expect("measured boundary")
        .clone();
    extra.port = node::Port::new("z_contradiction").expect("port");
    extra.direction = node::PortDirection::Input;
    forged.nodes[0].ports.push(extra);
    let error = plan::ValidatedExecutionPlan::try_new(forged)
        .expect_err("independent validator checks every declared site");
    assert!(error.to_string().contains("inconsistent units"), "{error}");

    let source = r#"
type Items = int @measure(id = carrots, unit = "item")
type Weight = int @measure(id = carrots, unit = "kg")
graph app : App {
  input in: Items
  input other: Weight
  in -> nothing
  other -> nothing
}
"#;
    let error = compile(&[source], &ContractCatalog::try_m0().expect("catalogue"))
        .expect_err("conflicting source definitions");
    assert!(
        format!("{error:?}").contains("inconsistent units"),
        "{error:?}"
    );
}

#[test]
fn a_deployment_tariff_does_not_turn_a_quantity_into_an_observed_charge() {
    let source = r#"
type Weight = decimal(38,3) @measure(id = carrots, unit = "kg")
graph app : App {
  connector weighing : Process { command = "cat"; codec = lines; behavior = deterministic; timeout = duration("1s") }
  operator decode {
    input in: text
    output out: Weight
    on in { out = decimal(data) as Weight }
  }
  in -> weighing -> decode -> out
  weighing.err -> nothing
  weighing.exit -> nothing
}
lab app {
  simulation weighing on app {
    mock weighing {
      on in {
        out = "-1.250"
        exit = Exited { exit_code = 0; signal = null }
      }
    }
    scenario correction {
      app.in = "weigh"
      expect app.out count == 1
    }
  }
}
"#;
    let compiled = compile(&[source], &ContractCatalog::try_m0().expect("catalogue"))
        .expect("a declared exact quantity downstream of the generic Connector");
    let profile = PriceBook::new(
        PriceBookName::new("pricing.example/produce@1").expect("profile identity"),
        vec![
            UnitPrice::new(
                node::Path::new(["weighing"]).expect("Connector path"),
                "market",
                "carrots",
                rastok::schema_annotation::MeasureId::new("carrots").expect("business id"),
                1,
                ExactDecimal::new(240, 2).expect("EUR 2.40/kg"),
                rastok::schema_annotation::CurrencyCode::new("EUR").expect("currency"),
            )
            .expect("nonnegative tariff for a signed business quantity"),
        ],
    )
    .expect("price profile");
    let runs = rastok::lab::run_program(
        &compiled.plan,
        compiled.hash,
        GraphVersion(1),
        &compiled.labs[0],
        None,
    );
    assert!(runs[0].report.passed(), "{:?}", runs[0].report);
    let observations = accounting_observations(&compiled.plan, &runs[0].records, Some(&profile))
        .expect("the original signed quantity");
    assert_eq!(observations.len(), 1);
    assert_eq!(
        observations[0].measures()[0].quantity().to_string(),
        "-1.25"
    );
    assert_eq!(observations[0].measures()[0].unit(), "kg");
    assert!(
        observations[0].measures().len() == 1,
        "no price calculation is performed"
    );
}
