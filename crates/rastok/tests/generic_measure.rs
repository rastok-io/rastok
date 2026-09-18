//! A measure describes a numerical field, not a financial/reporting policy.

#![allow(clippy::expect_used, clippy::panic)]

use rastok::compiler::{Compilation, compile};
use rastok::contract::ContractCatalog;
use rastok::diagnostics::Bag;
use rastok::plan;
use rastok::schema_annotation::{AnnotationApplication, AnnotationBinding, measure_argument_type};
use rastok::value::Value;

fn checked(declarations: &str, ty: &str) -> Result<Compilation, Bag> {
    compile(
        &[&format!(
            "{declarations}\ngraph app : App {{\ninput in: {ty}\noutput out: {ty}\noperator copy {{ input in: {ty}\non in {{ out = . }} }}\nin -> copy -> out\n}}"
        )],
        &ContractCatalog::try_m0().expect("catalogue"),
    )
}

#[test]
fn standard_measure_is_exactly_two_required_text_fields() {
    let standard = checked("", "measure").expect("standard definition as data");
    let plain = checked("", "struct { id: text, unit: text }").expect("ordinary flat data");
    assert_eq!(standard.plan, plain.plan);
    assert_eq!(standard.hash, plain.hash);
}

#[test]
fn amounts_are_ordinary_signed_quantities_with_an_explicit_unit() {
    for ty in [
        "int @measure(id = amount, unit = \"USD\")",
        "decimal(38,20) @measure(id = refund, unit = \"EUR\")",
        "decimal(18,6) @measure(id = amount, unit = \"carrot\")",
    ] {
        let direct = checked("", ty).expect("no special monetary carrier or scale");
        let alias = checked(
            "type Quantity = measure",
            &ty.replace("@measure", "@Quantity"),
        )
        .expect("same definition through an alias");
        assert_eq!(direct.hash, alias.hash);
        let bytes = plan::encode(&direct.plan, &direct.hash);
        assert_eq!(bytes, plan::encode(&alias.plan, &alias.hash));
        assert_eq!(
            plan::decode(&bytes).expect("independent validation"),
            (direct.plan, direct.hash)
        );
    }
}

#[test]
fn removed_measure_arguments_refuse_with_a_migration_explanation() {
    for argument in [
        "currency = USD",
        "basis = provider_reported",
        "aggregation = sum",
        "subset_of = total",
    ] {
        let error = checked(
            "",
            &format!("int @measure(id = amount, unit = \"USD\", {argument})"),
        )
        .expect_err("only id and unit belong to a measure");
        assert!(
            error.diagnostics().iter().any(|diagnostic| {
                diagnostic.code().as_str() == "E-DATA-ANNOTATION"
                    && diagnostic.message().contains("id and unit")
            }),
            "{error:?}"
        );
    }
}

#[test]
fn the_old_protected_definition_cannot_claim_the_new_schema() {
    let arguments = Value::Struct(vec![
        ("id".into(), Value::Text("amount".into())),
        ("unit".into(), Value::Text("USD".into())),
    ]);
    let schema = measure_argument_type().expect("standard schema");
    assert!(
        AnnotationApplication::new(
            AnnotationBinding::Measure,
            "rastok/measure@2".into(),
            schema.clone(),
            arguments.clone(),
        )
        .is_err()
    );
    AnnotationApplication::new(
        AnnotationBinding::Measure,
        "rastok/measure@3".into(),
        schema,
        arguments,
    )
    .expect("new definition explicitly owns the two-field schema");
}

#[test]
fn a_measure_describes_the_numeric_field_not_a_money_shaped_container() {
    let error = checked(
        "type Money = struct { amount: decimal(18,6), currency: text }",
        "Money @measure(id = amount, unit = \"USD\")",
    )
    .expect_err("the generic interpretation requires a numerical field");
    assert!(
        error.diagnostics().iter().any(|diagnostic| {
            diagnostic.code().as_str() == "E-DATA-ANNOTATION"
                && diagnostic.message().contains("integer or exact decimal")
        }),
        "{error:?}"
    );
}
