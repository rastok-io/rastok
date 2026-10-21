//! Amounts are ordinary signed measurements; no billing interpretation is inferred.

#![allow(clippy::expect_used, clippy::panic)]

use rastok::compiler::{Compilation, compile};
use rastok::contract::ContractCatalog;
use rastok::diagnostics::Bag;
use rastok::plan;
use rastok::schema_annotation::{DataAnnotation, SchemaPath};

fn checked(ty: &str, annotation: &str) -> Result<Compilation, Bag> {
    let source = format!(
        "type Amount = {ty} {annotation}\ngraph app : App {{\ninput in: Amount\noutput out: Amount\noperator copy {{ input in: Amount\non in {{ out = . }} }}\nin -> copy -> out\n}}"
    );
    compile(
        &[&source],
        &ContractCatalog::try_m0().expect("the standard catalogue"),
    )
}

#[test]
fn monetary_measure_explicitly_moves_from_compact_cost_to_generic_metadata() {
    let current = checked("decimal(38,18)", "@measure(id = amount, unit = \"USD\")")
        .expect("ordinary measured amount");
    // D-2026-09-18-1 retires the compact cost interpretation; its old witness
    // must change, while the value type and retained causal value do not.
    assert_ne!(
        current.hash.to_string(),
        "490b9e2bd295b8c438cb533665aa4bbd431f42d23a3084123ed3b096420f6cf6"
    );
    let bytes = plan::encode(&current.plan, &current.hash);
    assert_ne!(
        blake3::hash(&bytes).to_hex().as_str(),
        "6ca1648297851e06dbd678bee3ddf0ad6f3ae79a727cce5ac8a410615751535a"
    );
    assert_eq!(
        plan::decode(&bytes).expect("independent validation"),
        (current.plan, current.hash)
    );
}

#[test]
fn amount_argument_order_does_not_change_identity() {
    for ty in ["int", "decimal(18,6)", "decimal(38,20)"] {
        let original = checked(ty, "@measure(id = amount, unit = \"EUR\")").expect("amount");
        let reordered =
            checked(ty, "@measure(unit = \"EUR\", id = amount)").expect("same metadata");
        assert_eq!(original.plan, reordered.plan);
        assert_eq!(original.hash, reordered.hash);
        assert_eq!(
            plan::encode(&original.plan, &original.hash),
            plan::encode(&reordered.plan, &reordered.hash)
        );
    }
}

#[test]
fn currency_units_have_no_special_annotation_authority() {
    for unit in ["USD", "EUR", "ZZZ", "carrot"] {
        let result = checked(
            "decimal(38,20)",
            &format!("@measure(id = amount, unit = \"{unit}\")"),
        )
        .expect("opaque unit and ordinary exact decimal");
        let set = result.plan.plan().nodes[1]
            .ports
            .iter()
            .find(|port| port.port.name() == "in")
            .expect("input")
            .annotations
            .entries()
            .get(&SchemaPath::root())
            .expect("measurement");
        let [DataAnnotation::Declared(application)] = set.annotations() else {
            panic!("one generic measurement: {set:?}");
        };
        assert_eq!(application.definition(), "std/measure@3");
        assert_eq!(application.quantity_measure(), Some(("amount", unit)));
    }
}

#[test]
fn invalid_amount_annotations_refuse_by_path_without_disclosing_values() {
    for (ty, arguments, detail) in [
        ("decimal(38,18)", "id = amount", "unit"),
        ("decimal(38,18)", "unit = \"USD\"", "id"),
        (
            "decimal(38,18)",
            "id = amount, currency = USD, basis = provider_reported",
            "id and unit",
        ),
        (
            "decimal(38,18)",
            "id = amount, unit = \"USD\", aggregation = sum",
            "id and unit",
        ),
        (
            "decimal(38,18)",
            "id = amount, unit = env(\"PRIVATE_CURRENCY\")",
            "constant",
        ),
        (
            "struct { amount: decimal(38,18), currency: text }",
            "id = amount, unit = \"USD\"",
            "integer or exact decimal",
        ),
    ] {
        let bag = checked(ty, &format!("@measure({arguments})")).expect_err("invalid metadata");
        let fault = bag
            .diagnostics()
            .iter()
            .find(|fault| {
                fault.code().as_str() == "E-DATA-ANNOTATION" && fault.message().contains(detail)
            })
            .unwrap_or_else(|| panic!("{arguments}: {bag:?}"));
        assert!(
            fault
                .fields()
                .iter()
                .any(|(key, value)| *key == "path" && value == "$")
        );
        assert!(!fault.message().contains("PRIVATE_CURRENCY"));
    }
}

#[test]
fn retired_cost_spelling_has_a_migration_diagnostic() {
    let bag = checked(
        "decimal(38,18)",
        "@cost(currency = USD, basis = provider_reported)",
    )
    .expect_err("cost is not an alias");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|fault| fault.code().as_str() == "E-DATA-ANNOTATION"
                && fault.message().contains("retired")
                && fault.message().contains("@measure(id = amount, ...)")),
        "{bag:?}"
    );
}

#[test]
fn monetary_ids_are_not_silently_collapsed_into_amount() {
    for ty in ["int", "decimal(38,18)"] {
        let invoice = checked(ty, "@measure(id = invoice, unit = \"USD\")").expect("custom id");
        let reordered =
            checked(ty, "@measure(unit = \"USD\", id = invoice)").expect("same metadata");
        assert_eq!(invoice.plan, reordered.plan);
        assert_eq!(invoice.hash, reordered.hash);
        for id in ["amount", "refund"] {
            let other =
                checked(ty, &format!("@measure(id = {id}, unit = \"USD\")")).expect("distinct id");
            assert_ne!(invoice.hash, other.hash);
        }
        let bytes = plan::encode(&invoice.plan, &invoice.hash);
        assert_eq!(
            plan::decode(&bytes).expect("independent validation"),
            (invoice.plan, invoice.hash)
        );
    }
}

#[test]
fn monetary_names_currencies_and_original_samples_remain_distinct() {
    use rastok::accounting::accounting_observations;
    let source = r#"
type Decimal = decimal(38,18)
type Receipt = struct {
  billed: Decimal @measure(id = invoice, unit = "EUR")
  returned: Decimal @measure(id = refund, unit = "EUR")
  foreign: Decimal @measure(id = invoice_usd, unit = "USD")
  money: struct { amount: Decimal @measure(id = refund, unit = "EUR"), currency: text }
}
graph app : App {
  operator account {
    input in: int
    output out: Receipt
    on in {
      out = { billed = decimal("12.50") as Decimal; returned = decimal("-5.00") as Decimal; foreign = decimal("2.25") as Decimal; money = { amount = decimal("-7.50") as Decimal; currency = "EUR" } }
    }
  }
  in -> account -> out
}
lab app { simulation billing on app { scenario adjustment {
  app.in = 1
  expect app.out count == 1
} } }
"#;
    let compiled = compile(&[source], &ContractCatalog::try_m0().expect("catalogue"))
        .expect("one unit per id, including an explicitly annotated nested amount");
    let (decoded, hash) =
        plan::decode(&plan::encode(&compiled.plan, &compiled.hash)).expect("retained binding");
    let runs = rastok::lab::run_program(
        &decoded,
        hash,
        plan::GraphVersion(1),
        &compiled.labs[0],
        None,
    );
    assert!(runs[0].report.passed(), "{:?}", runs[0].report);
    let observations = accounting_observations(&decoded, &runs[0].records, None)
        .expect("original monetary samples");
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].measures().len(), 4);
    let rows: Vec<_> = observations[0]
        .measures()
        .iter()
        .map(|row| (row.id().as_str(), row.unit(), row.quantity().to_string()))
        .collect();
    assert_eq!(
        rows,
        [
            ("invoice", "EUR", "12.5".into()),
            ("invoice_usd", "USD", "2.25".into()),
            ("refund", "EUR", "-7.5".into()),
            ("refund", "EUR", "-5".into())
        ]
    );
}

#[test]
fn amount_targets_are_checked_independently_and_units_cannot_conflict() {
    use rastok::datatype::{DataType, DecimalType, StructType, TypeField};
    use rastok::schema_annotation::{
        AnnotatedSchema, AnnotationApplication, AnnotationBinding, AnnotationSet, AnnotationTable,
        measure_argument_type,
    };
    use rastok::value::Value;
    let table = || {
        AnnotationTable::new(vec![(
            SchemaPath::root(),
            AnnotationSet::new(vec![DataAnnotation::Declared(Box::new(
                AnnotationApplication::new(
                    AnnotationBinding::Measure,
                    "std/measure@3".into(),
                    measure_argument_type().expect("standard signature"),
                    Value::Struct(vec![
                        ("id".into(), Value::Text("amount".into())),
                        ("unit".into(), Value::Text("USD".into())),
                    ]),
                )
                .expect("typed metadata"),
            ))])
            .expect("one annotation"),
        )])
        .expect("one row")
    };
    let decimal = DataType::Decimal(DecimalType::new(38, 20).expect("ordinary decimal"));
    assert!(AnnotatedSchema::new(DataType::INT, table()).is_ok());
    assert!(AnnotatedSchema::new(decimal.clone(), table()).is_ok());
    let money = DataType::Struct(
        StructType::new(vec![
            TypeField::new("amount", decimal),
            TypeField::new("currency", DataType::Text),
        ])
        .expect("ordinary struct"),
    );
    assert!(AnnotatedSchema::new(money, table()).is_err());
    let errors = compile(
        &[r#"
graph app : App {
  input in: decimal(38,18) @measure(id = invoice, unit = "EUR")
  input other: int @measure(id = invoice, unit = "USD")
  in -> nothing
  other -> nothing
}
"#],
        &ContractCatalog::try_m0().expect("catalogue"),
    )
    .expect_err("one id keeps one unit");
    assert!(
        format!("{errors:?}").contains("inconsistent units"),
        "{errors:?}"
    );
}

#[test]
fn signed_refunds_remain_distinct_original_samples() {
    use rastok::accounting::{ExactDecimal, accounting_observations};
    use rastok::connector::exchange::EnvelopeDraft;
    use rastok::connector::exchange::IngressProposal;
    use rastok::connector::exchange::IngressProvenance;
    use rastok::connector::exchange::IngressSequence;
    use rastok::envelope::{Data, Namespace, Value};
    use rastok::journal::MemJournal;
    use rastok::node;
    use rastok::plan::GraphVersion;
    use rastok::runtime::execution::Execution;

    for money_shaped in [false, true] {
        let ty = if money_shaped {
            "struct { amount: DecimalAmount @measure(id = amount, unit = \"EUR\"), currency: text }"
        } else {
            "DecimalAmount @measure(id = amount, unit = \"EUR\")"
        };
        let value = |literal: &str| {
            if money_shaped {
                format!(
                    "{{ amount = decimal(\"{literal}\") as DecimalAmount; currency = \"EUR\" }}"
                )
            } else {
                format!("decimal(\"{literal}\") as Amount")
            }
        };
        for (refund, refund_cents) in [("-5.00", -500), ("-12.50", -1250), ("-20.00", -2000)] {
            let source = format!(
                r#"
type DecimalAmount = decimal(38,18)
type Amount = {ty}
graph adjustments : App {{
  input in: int
  output out: Amount
  operator account {{
    input in: int
    output out: Amount
    on in {{
      out = {charge}
      out = {refund}
    }}
  }}
  in -> account -> out
}}
"#,
                charge = value("12.50"),
                refund = value(refund),
            );
            let compiled = compile(
                &[&source],
                &ContractCatalog::try_m0().expect("standard catalogue"),
            )
            .expect("signed amounts compile at scalar and nested field paths");
            let (decoded, hash) = plan::decode(&plan::encode(&compiled.plan, &compiled.hash))
                .expect("independent monetary plan validation");
            assert_eq!(hash, compiled.hash);
            let version = GraphVersion(1);
            let mut journal = MemJournal::new(version);
            let mut vm = Execution::new(&decoded, version);
            vm.accept_input(
                &mut journal,
                IngressProposal {
                    provenance: IngressProvenance::single(
                        node::Path::new(["refund-test"]).expect("provenance"),
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
            vm.run_ready(&mut journal)
                .expect("both signed emissions commit");

            let observations = accounting_observations(&decoded, journal.records(), None)
                .expect("signed samples remain exact");
            assert_eq!(observations.len(), 2, "the refund is not an overwrite");
            assert_ne!(observations[0].envelope_id(), observations[1].envelope_id());
            for (observation, cents) in observations.iter().zip([1250, refund_cents]) {
                assert_eq!(observation.measures().len(), 1);
                let sample = &observation.measures()[0];
                assert_eq!(sample.unit(), "EUR");
                assert_eq!(
                    sample.quantity(),
                    ExactDecimal::new(cents, 2).expect("cents")
                );
            }

            assert_eq!(
                observations,
                accounting_observations(&compiled.plan, journal.records(), None)
                    .expect("the retained samples replay without live state")
            );
        }
    }
}
