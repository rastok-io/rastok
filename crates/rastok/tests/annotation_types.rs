//! Flat declaration constraints reuse ordinary data identity and type checking.

#![allow(clippy::expect_used, clippy::panic)]

use rastok::compiler::parser::parse;
use rastok::compiler::syntax::{BaseType, TopDecl};
use rastok::compiler::{Compilation, compile};
use rastok::contract::ContractCatalog;
use rastok::diagnostics::{Bag, SourceId};
use rastok::plan;

fn checked(source: &str) -> Result<Compilation, Bag> {
    compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue"),
    )
}

fn relay(declarations: &str, ty: &str) -> String {
    format!(
        "{declarations}\ngraph app : App {{\ninput in: {ty}\noutput out: {ty}\noperator copy {{ input in: {ty}\non in {{ out = . }} }}\nin -> copy -> out\n}}"
    )
}

#[test]
fn flat_struct_data_has_the_same_plan_and_hash_as_an_ordinary_struct() {
    let flat = relay(
        "type Mode = enum { sum, last }\ntype Label = @struct { name: text, weight: decimal(38,18)?, mode: Mode }",
        "Label",
    );
    let plain = flat.replace("@struct", "struct");
    let flat = checked(&flat).expect("flat data uses the ordinary logical type");
    let plain = checked(&plain).expect("the equivalent ordinary struct");
    assert_eq!(flat.plan, plain.plan);
    assert_eq!(flat.hash, plain.hash);
    assert_eq!(
        plan::encode(&flat.plan, &flat.hash),
        plan::encode(&plain.plan, &plain.hash)
    );
}

#[test]
fn flat_struct_fields_refuse_composites_even_through_aliases() {
    for field_type in [
        "array<int>",
        "map<text, int>",
        "struct { value: int }",
        "union { Some { value: int }, None }",
        "variant",
        "int | text",
        "array<int>?",
    ] {
        let source = relay(
            &format!("type Hidden = {field_type}\ntype Label = @struct {{ bad: Hidden }}"),
            "text",
        );
        let bag = checked(&source).expect_err("an unused invalid declaration also refuses");
        assert!(
            bag.diagnostics().iter().any(|diagnostic| {
                diagnostic.code().as_str() == "E-TYPE-INVALID"
                    && diagnostic.message().contains("flat")
                    && diagnostic
                        .fields()
                        .iter()
                        .any(|(key, value)| *key == "field" && value == "bad")
            }),
            "{field_type}: {bag:?}"
        );
    }
}

#[test]
fn flat_struct_scalar_aliases_and_postfix_policies_remain_ordinary_data() {
    let source = relay(
        "type Name = text\ntype Label = @struct { name: Name @personal(identifier = true), count: uint }\ntype Alias = Label",
        "Alias",
    );
    let flat = checked(&source).expect("a flat struct can carry the existing field policies");
    let plain = checked(&source.replace("@struct", "struct")).expect("ordinary annotated struct");
    assert_eq!(flat.plan, plain.plan);
    assert_eq!(flat.hash, plain.hash);
}

#[test]
fn declared_annotations_retain_their_typed_binding() {
    let source = relay(
        "type note = @struct { text: text }\ntype Alias = note",
        "text @Alias(text = \"caller\")",
    );
    let compiled = checked(&source).expect("declared descriptive metadata");
    let bytes = plan::encode(&compiled.plan, &compiled.hash);
    assert_eq!(
        plan::decode(&bytes).expect("independent binding validation"),
        (compiled.plan, compiled.hash)
    );
}

#[test]
fn declared_annotation_arguments_are_typed_by_path() {
    for (declaration, arguments, detail, location) in [
        ("text: text", "text = 1", "text", "text"),
        (
            "revision: int",
            "revision = 2147483648",
            "int32",
            "revision",
        ),
        ("revision: uint", "revision = -1", "uint64", "revision"),
        ("mode: Mode", "mode = last", "enum", "mode"),
        ("enabled: bool", "enabled = \"true\"", "bool", "enabled"),
        (
            "text: text",
            "wrong = \"x\"",
            "unexpected argument `wrong`",
            "wrong",
        ),
        (
            "text: text",
            "",
            "missing required argument `text`",
            "@note",
        ),
        ("text: text", "text = data.message", "constant", "text"),
        (
            "text: text",
            "text = env(\"PRIVATE_NAME\")",
            "constant",
            "text",
        ),
        (
            "amount: decimal(5,2)",
            "amount = decimal(\"0.001\")",
            "decimal",
            "amount",
        ),
        (
            "amount: decimal(5,2)",
            "amount = decimal(\"1000\")",
            "decimal",
            "amount",
        ),
    ] {
        let annotation = if arguments.is_empty() {
            "@note".to_owned()
        } else {
            format!("@note({arguments})")
        };
        let source = relay(
            &format!("type Mode = enum {{ sum }}\ntype note = @struct {{ {declaration} }}"),
            &format!("text {annotation}"),
        );
        let bag = checked(&source).expect_err("a malformed typed application refuses first");
        let fault = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code().as_str() == "E-DATA-ANNOTATION")
            .unwrap_or_else(|| panic!("{arguments}: {bag:?}"));
        assert!(fault.message().contains(detail), "{arguments}: {fault:?}");
        assert_eq!(&source[fault.spans()[0].range()], location, "{fault:?}");
        assert!(
            fault
                .fields()
                .iter()
                .any(|(key, value)| *key == "path" && value == "$")
        );
        assert!(
            !bag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code().as_str() == "E-CHECK-DEFERRED"),
            "the binding gate must not conceal a known type defect: {bag:?}"
        );
        assert!(!fault.message().contains("PRIVATE_NAME"));
    }
}

#[test]
fn well_typed_annotation_constants_are_retained() {
    for (declaration, arguments) in [
        ("revision: uint", "revision = 255"),
        ("revision: int", "revision = -2147483648"),
        ("amount: decimal(5,2)", "amount = decimal(\"1.20\")"),
        ("amount: decimal(5,2)", "amount = 12"),
        ("mode: Mode", "mode = sum"),
        ("text: text", r#"text = "line\nquote\"""#),
        ("enabled: bool", "enabled = false"),
        ("enabled: bool?", "enabled = null"),
        ("amount: decimal(5,2)?", "amount = decimal(\"1.20\")"),
    ] {
        let source = relay(
            &format!(
                "type Mode = enum {{ sum }}\ntype note = @struct {{ {declaration} }}\ntype Alias = note"
            ),
            &format!("text @Alias({arguments})"),
        );
        let compiled = checked(&source).unwrap_or_else(|bag| panic!("{arguments}: {bag:?}"));
        let bytes = plan::encode(&compiled.plan, &compiled.hash);
        assert_eq!(
            plan::decode(&bytes).expect("typed arguments roundtrip"),
            (compiled.plan, compiled.hash)
        );
    }
}

#[test]
fn flat_struct_prefix_and_field_annotations_have_distinct_parser_positions() {
    let catalogue = ContractCatalog::try_m0().expect("the standard catalogue");
    let mut bag = Bag::new();
    let tree = parse(
        SourceId::new(0),
        "type Label = @struct { value: text @sensitive(level = confidential) }?",
        &catalogue,
        &mut bag,
    );
    assert!(!bag.has_errors(), "{bag:?}");
    let TopDecl::Type(declaration) = &tree.decls[0] else {
        panic!("the type declaration");
    };
    assert!(declaration.ty.optional);
    assert!(declaration.ty.annotations.is_empty());
    let BaseType::FlatStruct { fields, open } = &declaration.ty.alternatives[0] else {
        panic!("the prefix selects the flat struct production");
    };
    assert!(!open);
    assert_eq!(fields[0].ty.annotations[0].name.text, "sensitive");
}

#[test]
fn malformed_flat_structs_recover_at_the_next_declaration() {
    let catalogue = ContractCatalog::try_m0().expect("the standard catalogue");
    for (bad, found) in [("@int", "int"), ("struct@ {}", "@"), ("@struct [", "[")] {
        let source = format!("type Bad = {bad}\ntype Good = @struct {{ value: int }}");
        let mut bag = Bag::new();
        let tree = parse(SourceId::new(0), &source, &catalogue, &mut bag);
        let fault = bag.diagnostics().first().expect("located parser refusal");
        assert_eq!(fault.code().as_str(), "E-PARSE-EXPECTED");
        assert_eq!(&source[fault.spans()[0].range()], found);
        assert_eq!(bag.diagnostics().len(), 1, "{bag:?}");
        assert!(
            tree.decls.iter().any(|declaration| {
                matches!(declaration, TopDecl::Type(ty) if ty.name.text == "Good")
            }),
            "{bad}: {tree:?}"
        );
    }
}

#[test]
fn flat_structs_are_closed_and_share_duplicate_and_cycle_checks() {
    for (declaration, code) in [
        ("type Label = @struct { value: int, .. }", "E-TYPE-INVALID"),
        (
            "type Label = @struct { value: int, value: text }",
            "E-TYPE-DUP",
        ),
        ("type Label = @struct { value: Label }", "E-TYPE-RECURSIVE"),
    ] {
        let bag = checked(&relay(declaration, "text")).expect_err("invalid flat type");
        assert!(
            bag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code().as_str() == code),
            "{bag:?}"
        );
    }
}

#[test]
fn user_annotation_definitions_cannot_shadow_stdlib_interpretations() {
    for name in ["sensitive", "secret", "personal", "measure", "cost"] {
        for declaration in [
            format!("type {name} = @struct {{ text: text }}"),
            format!("type Note = @struct {{ text: text }}\ntype {name} = Note"),
        ] {
            let source = relay(&declaration, "text");
            let bag = checked(&source)
                .expect_err("a user definition cannot replace a protected annotation");
            let fault = bag
                .diagnostics()
                .iter()
                .find(|diagnostic| {
                    diagnostic.code().as_str() == "E-NAME-RESERVED"
                        && diagnostic.message().contains(if name == "cost" {
                            "retired"
                        } else {
                            "stdlib annotation"
                        })
                })
                .unwrap_or_else(|| panic!("{declaration}: {bag:?}"));
            assert_eq!(&source[fault.spans()[0].range()], name);
        }
    }
}
