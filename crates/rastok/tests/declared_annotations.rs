//! Annotation identity is nominal; its arguments use ordinary typed values.

#![allow(clippy::expect_used, clippy::panic)]

use rastok::compiler::{Compilation, compile};
use rastok::connector::contract::ContractCatalog;
use rastok::diagnostics::Bag;
use rastok::plan;
use rastok::schema_annotation::{AnnotationBinding, DataAnnotation, SchemaPath};

fn checked(declarations: &str, annotation: &str) -> Result<Compilation, Bag> {
    let source = format!(
        "{declarations}\ntype Data = text {annotation}\ngraph app : App {{\ninput in: Data\noutput out: Data\noperator copy {{ input in: Data\non in {{ out = . }} }}\nin -> copy -> out\n}}"
    );
    compile(
        &[&source],
        &ContractCatalog::try_m0().expect("standard catalogue"),
    )
}

#[test]
fn aliases_and_omitted_nullable_arguments_have_one_canonical_application() {
    let declarations =
        "type note = @struct { text: text, revision: uint? }\ntype A = note\ntype Z = A";
    let direct = checked(declarations, "@note(text = \"caller\")").expect("declared metadata");
    let alias = checked(declarations, "@Z(revision = null, text = \"caller\")")
        .expect("the same declaration and complete constants");
    assert_eq!(direct.plan, alias.plan);
    assert_eq!(direct.hash, alias.hash);
    let bytes = plan::encode(&direct.plan, &direct.hash);
    assert_eq!(bytes, plan::encode(&alias.plan, &alias.hash));
    assert_eq!(
        plan::decode(&bytes).expect("independent validation"),
        (direct.plan, direct.hash)
    );
}

#[test]
fn lookalikes_do_not_share_definition_identity() {
    let declarations =
        "type note = @struct { text: text }\ntype description = @struct { text: text }";
    let note = checked(declarations, "@note(text = \"caller\")").expect("note");
    let description =
        checked(declarations, "@description(text = \"caller\")").expect("description");
    assert_ne!(note.hash, description.hash);
    let both = checked(
        declarations,
        "@note(text = \"caller\") @description(text = \"caller\")",
    )
    .expect("distinct metadata can coexist");
    let swapped = checked(
        declarations,
        "@description(text = \"caller\") @note(text = \"caller\")",
    )
    .expect("application order is not semantic");
    assert_eq!(both.plan, swapped.plan);
    assert_eq!(both.hash, swapped.hash);
}

#[test]
fn declaration_and_argument_order_are_not_semantic_but_values_are() {
    let a = checked(
        "type note = @struct { text: text, revision: uint }",
        "@note(text = \"caller\", revision = 1)",
    )
    .expect("first order");
    let b = checked(
        "type note = @struct { revision: uint, text: text }",
        "@note(revision = 1, text = \"caller\")",
    )
    .expect("second order");
    let changed = checked(
        "type note = @struct { revision: uint, text: text }",
        "@note(revision = 2, text = \"caller\")",
    )
    .expect("other arguments");
    assert_eq!(a.hash, b.hash);
    assert_eq!(
        plan::encode(&a.plan, &a.hash),
        plan::encode(&b.plan, &b.hash)
    );
    assert_ne!(b.hash, changed.hash);
}

#[test]
fn metadata_is_preserved_by_copy_without_gaining_policy_authority() {
    let compiled = checked(
        "type classification = @struct { identifier: bool }",
        "@classification(identifier = true)",
    )
    .expect("descriptive lookalike of a personal policy");
    let output = compiled
        .plan
        .resolved_ports()
        .boundary()
        .iter()
        .find(|port| port.port().name() == "out")
        .expect("App output");
    assert!(!rastok::data_policy::contains_personal(
        output.annotations()
    ));
    assert!(!rastok::data_policy::contains_secret(output.annotations()));
    let set = output
        .annotations()
        .entries()
        .get(&SchemaPath::root())
        .expect("metadata retained");
    let [DataAnnotation::Declared(application)] = set.annotations() else {
        panic!("{set:?}")
    };
    assert_eq!(application.definition(), "classification");
    assert_eq!(application.binding(), AnnotationBinding::Metadata);
    assert_eq!(
        application.arguments(),
        &rastok::value::Value::Struct(vec![(
            "identifier".into(),
            rastok::value::Value::Bool(true)
        )])
    );
}

#[test]
fn duplicate_aliases_and_conflicting_alternative_metadata_refuse() {
    let duplicate = checked(
        "type note = @struct { text: text }\ntype Alias = note",
        "@note(text = \"a\") @Alias(text = \"b\")",
    )
    .expect_err("one declaration twice");
    assert!(
        duplicate
            .diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-DATA-ANNOTATION" && d.message().contains("twice"))
    );
    let source = r#"
type note = @struct { text: text }
graph app : App {
  input a: text @note(text = "a")
  input b: text @note(text = "b")
  output out: text
  operator copy { input in: text
    on in { out = . }
  }
  a -> copy
  b -> copy
  copy -> out
}"#;
    let error = compile(&[source], &ContractCatalog::try_m0().expect("catalogue"))
        .expect_err("an input cannot pretend both different meanings agree");
    assert!(
        error
            .diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-DATA-ANNOTATION"),
        "{error:?}"
    );
}

#[test]
fn policies_on_argument_fields_are_not_silently_erased_or_recursively_interpreted() {
    for declarations in [
        "type note = @struct { text: text @personal }",
        "type Argument = text @secret\ntype note = @struct { text: Argument }",
        "type note = @struct { text: text @note(text = \"recursive\") }",
    ] {
        let error = checked(declarations, "@note(text = \"caller\")")
            .expect_err("argument policies need their own admitted transport");
        assert!(
            error
                .diagnostics()
                .iter()
                .any(|d| d.code().as_str() == "E-CHECK-DEFERRED"
                    && d.message().contains("argument-policy")),
            "{error:?}"
        );
    }
}

#[test]
fn unused_metadata_definitions_add_no_plan_or_hash_bytes() {
    let plain = checked("", "").expect("plain relay");
    let declared = checked("type note = @struct { text: text }", "").expect("unused definition");
    assert_eq!(plain.hash, declared.hash);
    assert_eq!(
        plan::encode(&plain.plan, &plain.hash),
        plan::encode(&declared.plan, &declared.hash)
    );
}
