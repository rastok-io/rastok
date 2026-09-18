//! Standard metadata uses the same declarations and aliases as ordinary data.

#![allow(clippy::expect_used, clippy::panic)]

use rastok::compiler::{Compilation, compile};
use rastok::contract::ContractCatalog;
use rastok::diagnostics::Bag;
use rastok::plan;

fn checked(declarations: &str, ty: &str) -> Result<Compilation, Bag> {
    compile(
        &[&format!(
            "{declarations}\ngraph app : App {{\ninput in: {ty}\noutput out: {ty}\noperator copy {{ input in: {ty}\non in {{ out = . }} }}\nin -> copy -> out\n}}"
        )],
        &ContractCatalog::try_m0().expect("standard catalogue"),
    )
}

fn same_identity(a: &Compilation, b: &Compilation) {
    assert_eq!(a.hash, b.hash);
    assert_eq!(a.plan, b.plan);
    let bytes = plan::encode(&a.plan, &a.hash);
    assert_eq!(bytes, plan::encode(&b.plan, &b.hash));
    assert_eq!(
        plan::decode(&bytes).expect("independently validated"),
        (a.plan.clone(), a.hash)
    );
}

#[test]
fn standard_definitions_are_ordinary_structural_data() {
    for (name, fields) in [
        ("secret", ""),
        (
            "sensitive",
            "level: enum { internal, confidential, restricted }",
        ),
        (
            "personal",
            "special: bool, criminal: bool, identifier: bool",
        ),
        ("measure", "id: text, unit: text"),
    ] {
        let standard = checked("", name).expect("standard signature as ordinary data");
        let alias = checked(&format!("type A = {name}\ntype B = A"), "B")
            .expect("aliases have the same data meaning");
        let structural =
            checked("", &format!("struct {{ {fields} }}")).expect("equivalent ordinary struct");
        same_identity(&standard, &alias);
        same_identity(&standard, &structural);
        for port in standard.plan.resolved_ports().boundary() {
            assert!(port.annotations().is_empty(), "data use grants no policy");
        }
    }
}

#[test]
fn standard_aliases_keep_their_interpretation_and_compact_identity() {
    for (name, carrier, arguments) in [
        ("sensitive", "text", "level = confidential"),
        ("personal", "text", "identifier = true"),
        ("measure", "int", "id = carrots, unit = \"item\""),
        ("measure", "decimal(38,18)", "id = amount, unit = \"USD\""),
        ("measure", "decimal(38,18)", "id = refund, unit = \"EUR\""),
    ] {
        let direct = checked("", &format!("{carrier} @{name}({arguments})"))
            .expect("standard interpretation");
        let alias = checked(
            &format!("type Z = {name}\ntype A = Z\ntype B = A"),
            &format!("{carrier} @B({arguments})"),
        )
        .expect("the alias retains its protected interpretation");
        same_identity(&direct, &alias);
    }
}

#[test]
fn alias_defaults_and_unused_definitions_do_not_change_identity() {
    let personal = checked("", "text @personal").expect("default privacy flags");
    let explicit = checked(
        "type Pii = personal",
        "text @Pii(identifier = false, special = false, criminal = false)",
    )
    .expect("same false flags");
    same_identity(&personal, &explicit);
    let measure =
        checked("", "int @measure(id = carrots, unit = \"item\")").expect("explicit metadata");
    let explicit = checked(
        "type Quantity = measure",
        "int @Quantity(unit = \"item\", id = carrots)",
    )
    .expect("same arguments in another order");
    same_identity(&measure, &explicit);
    same_identity(
        &checked("", "text").expect("plain graph"),
        &checked("type Pii = personal\ntype Quantity = measure", "text").expect("unused aliases"),
    );
}

#[test]
fn alias_cannot_bypass_secret_or_duplicate_policy_refusals() {
    let secret = checked("type Credential = secret", "text @Credential")
        .expect_err("raw secret still refuses before execution");
    assert!(
        secret
            .diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-CHECK-DEFERRED" && d.message().contains("secret")),
        "{secret:?}"
    );
    for ty in [
        "text @personal @Pii",
        "text @sensitive(level = internal) @Sensitive(level = restricted)",
        "int @measure(id = carrots, unit = \"item\") @Quantity(id = carrots, unit = \"item\")",
    ] {
        let error = checked(
            "type Pii = personal\ntype Sensitive = sensitive\ntype Quantity = measure",
            ty,
        )
        .expect_err("aliases cannot introduce the same policy twice");
        assert!(
            error
                .diagnostics()
                .iter()
                .any(|d| d.code().as_str() == "E-DATA-ANNOTATION" && d.message().contains("twice")),
            "{error:?}"
        );
    }
}

#[test]
fn standard_names_cannot_be_replaced_by_other_data_types() {
    for name in ["sensitive", "secret", "personal", "measure", "cost"] {
        let error = checked(&format!("type {name} = text"), "text")
            .expect_err("the standard definition has one authority");
        assert!(
            error
                .diagnostics()
                .iter()
                .any(|d| d.code().as_str() == "E-NAME-RESERVED"),
            "{error:?}"
        );
    }
}
