//! The reformed literal surface (D-2026-08-10-6/-7): punctuated
//! braces, verbatim text, inline base64, direct JSON with exact
//! numbers — every form's success AND its located refusal.

#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::compiler::compile;
use rastok::stdlib::Registry;

fn emitted(body: &str) -> rastok::envelope::Data {
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, Namespace, Scalar, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let source = format!(
        "graph g : App {{\n  operator o {{\n    on in {{ out = {body} }}\n  }}\n  . -> o -> .\n}}\n"
    );
    let compilation = compile(&[source.as_str()], &Registry::m0()).expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(
        &mut journal,
        InboundSubmission {
            authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Scalar(Scalar::Int(1))),
                ext: Namespace::new(),
            },
            sequence: SubmissionSequence::Next,
        },
    )
    .expect("the injection lands");
    vm.drain(&mut journal).expect("the drain completes");
    vm.outputs()[0].envelope.data().clone()
}

fn hash_of(body: &str) -> String {
    let source = format!(
        "graph g : App {{\n  operator o {{\n    on in {{ out = {body} }}\n  }}\n  . -> o -> .\n}}\n"
    );
    compile(&[source.as_str()], &Registry::m0())
        .expect("a valid plan")
        .hash
        .to_string()
}

#[test]
fn verbatim_text_strips_the_closing_margin() {
    use rastok::envelope::{Data, Scalar, Value};
    // The closing brace sits at column 4; the margin stripped from
    // every line is ITS indentation — lines keep their own extra.
    let data = emitted("text {\n      first line\n        indented\n      last line\n    }");
    assert_eq!(
        data,
        Data::materialized(Value::Scalar(Scalar::Text(String::from(
            "  first line\n    indented\n  last line"
        )))),
        "first newline out, closing-brace margin stripped, no escapes"
    );
}

#[test]
fn base64_decodes_strictly() {
    use rastok::envelope::{Data, Scalar, Value};
    // Whitespace ignored, padding mandatory, standard alphabet.
    let data = emitted("base64 {\n      SGVs\n      bG8=\n    }");
    assert_eq!(
        data,
        Data::materialized(Value::Scalar(Scalar::Bytes(b"Hello".to_vec())))
    );
}

#[test]
fn base64_refusals_are_located() {
    for (body, wrong) in [
        ("base64 { SGVsbG8 }", "padding"),
        ("base64 { SGVs-_8= }", "URL-safe"),
        ("base64 { SG!s }", "alphabet"),
    ] {
        let source = format!(
            "graph g : App {{\n  operator o {{\n    on in {{ out = {body} }}\n  }}\n  . -> o -> .\n}}\n"
        );
        let bag = compile(&[source.as_str()], &Registry::m0()).expect_err("must refuse");
        assert!(
            bag.diagnostics()
                .iter()
                .any(|d| !d.spans().is_empty() && d.code().as_str() == "E-CEL-EXPR"),
            "`{wrong}` refusal carries a span: {bag:?}"
        );
    }
}

#[test]
fn base64_with_a_content_type_is_whole_payload() {
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, MetaValue, Namespace, Scalar, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let source = "graph g : App {\n  operator o {\n    on in { out = base64(\"image/png\") { AAEC } }\n  }\n  . -> o -> .\n}\n";
    let compilation = compile(&[source], &Registry::m0()).expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(
        &mut journal,
        InboundSubmission {
            authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Scalar(Scalar::Int(1))),
                ext: Namespace::new(),
            },
            sequence: SubmissionSequence::Next,
        },
    )
    .expect("the injection lands");
    vm.drain(&mut journal).expect("the drain completes");
    let output = &vm.outputs()[0];
    assert_eq!(
        output.envelope.data(),
        &Data::materialized(Value::Scalar(Scalar::Bytes(vec![0, 1, 2])))
    );
    assert_eq!(
        output.envelope.meta().ext().get("content_type"),
        Some(&MetaValue::Scalar(Scalar::Text(String::from("image/png")))),
        "the content type lands in ext, atomically"
    );
}

#[test]
fn maps_are_colon_punctuated() {
    // The map VALUE parses (punctuation decides) and still defers as
    // an emission — the honest pre-reform status, new spelling.
    let source = "graph g : App {\n  operator o {\n    on in { out = { \"a\": 1, \"b\": 2 } }\n  }\n  . -> o -> .\n}\n";
    let bag = compile(&[source], &Registry::m0()).expect_err("deferred");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-CHECK-DEFERRED"),
        "{bag:?}"
    );
    // The empty map `{:}` parses to the same deferral.
    let source =
        "graph g : App {\n  operator o {\n    on in { out = {:} }\n  }\n  . -> o -> .\n}\n";
    let bag = compile(&[source], &Registry::m0()).expect_err("deferred");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-CHECK-DEFERRED"),
        "{bag:?}"
    );
}

#[test]
fn mixing_struct_and_map_punctuation_refuses() {
    for body in ["{ a = 1, \"b\": 2 }", "{ \"b\": 2, a = 1 }"] {
        let source = format!(
            "graph g : App {{\n  operator o {{\n    on in {{ out = {body} }}\n  }}\n  . -> o -> .\n}}\n"
        );
        let bag = compile(&[source.as_str()], &Registry::m0()).expect_err("never mixed");
        assert!(
            bag.diagnostics()
                .iter()
                .any(|d| d.message().contains("the two never mix")),
            "`{body}`: {bag:?}"
        );
    }
}

#[test]
fn json_reads_every_form_directly() {
    use rastok::envelope::{Data, Scalar, Value};
    // Scalars.
    assert_eq!(
        emitted("json 42"),
        Data::materialized(Value::Scalar(Scalar::Int(42)))
    );
    assert_eq!(
        emitted("json \"text\""),
        Data::materialized(Value::Scalar(Scalar::Text(String::from("text"))))
    );
    assert_eq!(
        emitted("json true"),
        Data::materialized(Value::Scalar(Scalar::Bool(true)))
    );
    // Object — a MAP (the variant JSON model), single braces.
    let Data::Materialized(Value::Map(map)) = emitted("json { \"name\": \"Ada\" }") else {
        panic!("an object is a map");
    };
    assert_eq!(
        map.get("name"),
        Some(&Value::Scalar(Scalar::Text(String::from("Ada"))))
    );
    // Array.
    assert_eq!(
        emitted("json [1, 2, 3]"),
        Data::materialized(Value::Array(vec![
            Value::Scalar(Scalar::Int(1)),
            Value::Scalar(Scalar::Int(2)),
            Value::Scalar(Scalar::Int(3)),
        ]))
    );
    // An exact fraction survives as a decimal.
    assert_eq!(
        emitted("json 19.99"),
        Data::materialized(Value::Scalar(Scalar::Decimal {
            unscaled: 1999,
            scale: 2
        }))
    );
}

#[test]
fn json_refusals_are_located() {
    for (body, wrong) in [
        ("json { \"a\": 1, \"a\": 2 }", "duplicate key"),
        ("json 123456789012345678901234567890123456789", "39 digits"),
        ("json { \"a\": }", "syntax"),
    ] {
        let source = format!(
            "graph g : App {{\n  operator o {{\n    on in {{ out = {body} }}\n  }}\n  . -> o -> .\n}}\n"
        );
        let bag = compile(&[source.as_str()], &Registry::m0()).expect_err("must refuse");
        assert!(
            bag.diagnostics().iter().any(|d| !d.spans().is_empty()),
            "`{wrong}` refusal carries a span: {bag:?}"
        );
    }
}

#[test]
fn json_one_and_one_are_different_programs() {
    // The static type is identity (D-2026-08-10-7): `json 1` is a
    // variant, `1` an int — same numeric leaf, two programs.
    assert_ne!(hash_of("json 1"), hash_of("1"));
    // Minimally different pairs stay distinct too.
    assert_ne!(hash_of("json 1"), hash_of("json \"1\""));
    assert_ne!(hash_of("json 19.99"), hash_of("json 19.98"));
    // And the canonical collapses ARE one program.
    assert_eq!(hash_of("json 1"), hash_of("json 1.0"));
    assert_eq!(hash_of("json 1"), hash_of("json 1e0"));
    assert_eq!(hash_of("json -0"), hash_of("json 0"));
}

#[test]
fn assets_resolve_at_build_and_seal_by_content() {
    use rastok::compiler::compile_with_assets;
    use std::io::Write;

    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("asset-root");
    std::fs::create_dir_all(root.join("assets")).unwrap();
    let mut file = std::fs::File::create(root.join("assets/logo.bin")).unwrap();
    file.write_all(&[1, 2, 3]).unwrap();
    let mut copy = std::fs::File::create(root.join("assets/same.bin")).unwrap();
    copy.write_all(&[1, 2, 3]).unwrap();

    // Whole-data with a content type, plus a second use of the SAME
    // content under a different path: one deduplicated blob.
    let source = "graph g : App {\n  operator o {\n    on in { out = asset(\"assets/logo.bin\", \"application/octet-stream\") }\n  }\n  operator p {\n    on in { out = asset(\"assets/same.bin\") }\n  }\n  . -> o\n  o -> p\n  p -> .\n}\n";
    let compilation = compile_with_assets(&[source], &Registry::m0(), Some(&root))
        .expect("assets resolve at build");
    assert_eq!(
        compilation.plan.assets.len(),
        1,
        "identical content deduplicates to one blob"
    );
    // The hash seals the CONTENT digest, not the path: the same
    // program via the other spelling is the same identity.
    let swapped = "graph g : App {\n  operator o {\n    on in { out = asset(\"assets/same.bin\", \"application/octet-stream\") }\n  }\n  operator p {\n    on in { out = asset(\"assets/logo.bin\") }\n  }\n  . -> o\n  o -> p\n  p -> .\n}\n";
    let other = compile_with_assets(&[swapped], &Registry::m0(), Some(&root))
        .expect("the swapped spelling compiles");
    assert_eq!(compilation.hash, other.hash, "the path is spelling");
    // And the v5 package round-trips the blob with a verified digest.
    let bytes = rastok::plan::encode(&compilation.plan, &compilation.hash);
    let (decoded, _) = rastok::plan::decode(&bytes).expect("the package round-trips");
    assert_eq!(decoded.assets[0].bytes, vec![1, 2, 3]);
}

#[test]
fn asset_refusals_are_named() {
    use rastok::compiler::compile_with_assets;

    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("asset-refusals");
    std::fs::create_dir_all(&root).unwrap();
    let refuses = |body: &str, root: Option<&std::path::Path>, want: &str| {
        let source = format!(
            "graph g : App {{\n  operator o {{\n    on in {{ out = {body} }}\n  }}\n  . -> o -> .\n}}\n"
        );
        let bag = compile_with_assets(&[source.as_str()], &Registry::m0(), root)
            .expect_err("must refuse");
        assert!(
            bag.diagnostics()
                .iter()
                .any(|d| d.code().as_str() == "E-ASSET"),
            "`{want}`: {bag:?}"
        );
    };
    // Missing file.
    refuses("asset(\"missing.bin\")", Some(&root), "missing");
    // Root escape.
    refuses("asset(\"../outside.bin\")", Some(&root), "escape");
    // No root at all.
    refuses("asset(\"anything.bin\")", None, "no root");
}

#[test]
fn an_asset_emission_runs_and_carries_its_type() {
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::compiler::compile_with_assets;
    use rastok::envelope::{Data, MetaValue, Namespace, Scalar, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;
    use std::io::Write;

    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("asset-run");
    std::fs::create_dir_all(&root).unwrap();
    let mut file = std::fs::File::create(root.join("report.bin")).unwrap();
    file.write_all(b"PDFISH").unwrap();

    let source = "graph g : App {\n  operator o {\n    on in { out = asset(\"report.bin\", \"application/pdf\") }\n  }\n  . -> o -> .\n}\n";
    let compilation =
        compile_with_assets(&[source], &Registry::m0(), Some(&root)).expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(
        &mut journal,
        InboundSubmission {
            authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Scalar(Scalar::Int(1))),
                ext: Namespace::new(),
            },
            sequence: SubmissionSequence::Next,
        },
    )
    .expect("the injection lands");
    vm.drain(&mut journal).expect("the drain completes");
    let output = &vm.outputs()[0];
    assert_eq!(
        output.envelope.data(),
        &Data::materialized(Value::Scalar(Scalar::Bytes(b"PDFISH".to_vec())))
    );
    assert_eq!(
        output.envelope.meta().ext().get("content_type"),
        Some(&MetaValue::Scalar(Scalar::Text(String::from(
            "application/pdf"
        )))),
    );
}

#[test]
fn named_types_are_structural_sugar() {
    // An alias resolves to its structure in a pin…
    let aliased = "type Tag = text\n\ngraph g : App {\n  operator o {\n    input evt : Tag\n    on evt { out = data }\n  }\n  . -> o.evt\n  o -> .\n}\n";
    let first = compile(&[aliased], &Registry::m0()).expect("the alias resolves");
    // …and RENAMING it is not an identity move: same structure, same
    // hash (D-2026-08-10-8).
    let renamed = "type Label = text\n\ngraph g : App {\n  operator o {\n    input evt : Label\n    on evt { out = data }\n  }\n  . -> o.evt\n  o -> .\n}\n";
    let second = compile(&[renamed], &Registry::m0()).expect("the rename resolves");
    assert_eq!(first.hash, second.hash, "names are sugar, never identity");
    // The bare structural spelling is the SAME program too.
    let bare = "graph g : App {\n  operator o {\n    input evt : text\n    on evt { out = data }\n  }\n  . -> o.evt\n  o -> .\n}\n";
    let third = compile(&[bare], &Registry::m0()).expect("the bare spelling");
    assert_eq!(first.hash, third.hash, "the expansion IS the identity");
}

#[test]
fn type_declarations_refuse_duplicates_cycles_and_unknowns() {
    // Duplicate.
    let source = "type T = text\ntype T = int\n\ngraph g : App {\n  operator o {\n    on in { out = data }\n  }\n  . -> o -> .\n}\n";
    let bag = compile(&[source], &Registry::m0()).expect_err("duplicates refuse");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-DUP-TYPE"),
        "{bag:?}"
    );
    // Cycle — the reform's own Node example refuses in v1.
    let source = "type Node = struct { label: text, children: array<Node> }\n\ngraph g : App {\n  operator o {\n    input evt : Node\n    on evt { out = data }\n  }\n  . -> o.evt\n  o -> .\n}\n";
    let bag = compile(&[source], &Registry::m0()).expect_err("recursion refuses");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-TYPE-RECURSIVE"),
        "{bag:?}"
    );
    // Unknown name at a use site.
    let source = "graph g : App {\n  operator o {\n    input evt : Ghost\n    on evt { out = data }\n  }\n  . -> o.evt\n  o -> .\n}\n";
    let bag = compile(&[source], &Registry::m0()).expect_err("unknowns refuse");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-RESOLVE-UNKNOWN"),
        "{bag:?}"
    );
}
