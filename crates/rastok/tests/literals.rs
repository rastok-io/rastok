//! The reformed literal surface (D-2026-08-10-6/-7): punctuated
//! braces, verbatim text, inline base64, direct JSON with exact
//! numbers — every form's success AND its located refusal.

#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::compiler::compile;
use rastok::contract::ContractCatalog;

fn emitted(body: &str) -> rastok::envelope::Data {
    use rastok::boundary::{
        Draft, IngressEnvelopeProvenance, IngressSubmission, SubmissionSequence,
    };
    use rastok::envelope::{Data, Namespace, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let source = format!(
        "graph g : App {{\n  input in: variant\n  operator o {{\n    output out: variant\n    on in {{ out = {body} }}\n  }}\n  in -> o -> out\n}}\n"
    );
    let compilation = compile(
        &[source.as_str()],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(
        &mut journal,
        IngressSubmission {
            provenance: IngressEnvelopeProvenance::single(
                rastok::node::Path::new(["probe"]).unwrap(),
            ),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Int(1)),
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
        "graph g : App {{\n  input in: variant\n  operator o {{\n    output out: variant\n    on in {{ out = {body} }}\n  }}\n  in -> o -> out\n}}\n"
    );
    compile(
        &[source.as_str()],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan")
    .hash
    .to_string()
}

#[test]
fn verbatim_text_strips_the_closing_margin() {
    use rastok::envelope::{Data, Value};
    // The closing brace sits at column 4; the margin stripped from
    // every line is ITS indentation — lines keep their own extra.
    let data = emitted("text! {\n      first line\n        indented\n      last line\n    }");
    assert_eq!(
        data,
        Data::materialized(Value::Text(String::from(
            "  first line\n    indented\n  last line"
        ))),
        "first newline out, closing-brace margin stripped, no escapes"
    );
}

#[test]
fn base64_decodes_strictly() {
    use rastok::envelope::{Data, Value};
    // Whitespace ignored, padding mandatory, standard alphabet.
    let data = emitted("base64! {\n      SGVs\n      bG8=\n    }");
    assert_eq!(data, Data::materialized(Value::Bytes(b"Hello".to_vec())));
}

#[test]
fn base64_refusals_are_located() {
    for (body, wrong) in [
        ("base64! { SGVsbG8 }", "padding"),
        ("base64! { SGVs-_8= }", "URL-safe"),
        ("base64! { SG!s }", "alphabet"),
    ] {
        let source = format!(
            "graph g : App {{\n  operator o {{\n    on in {{ out = {body} }}\n  }}\n  in -> o -> out\n}}\n"
        );
        let bag = compile(
            &[source.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect_err("must refuse");
        assert!(
            bag.diagnostics()
                .iter()
                .any(|d| !d.spans().is_empty() && d.code().as_str() == "E-EXPR"),
            "`{wrong}` refusal carries a span: {bag:?}"
        );
    }
}

#[test]
fn base64_with_a_content_type_is_whole_payload() {
    use rastok::boundary::{
        Draft, IngressEnvelopeProvenance, IngressSubmission, SubmissionSequence,
    };
    use rastok::envelope::{Data, MetaValue, Namespace, Scalar, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in { out = base64!(\"image/png\") { AAEC } }\n  }\n  in -> o -> out\n}\n";
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    assert_eq!(
        compilation.hash.to_string(),
        "ee1dab9e3c3fd1924534c5709983f4e4d7c0d80140760f55520ebeafb3262f36",
        "typing the frontend content type is one declared H4.1 identity move"
    );
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(
        &mut journal,
        IngressSubmission {
            provenance: IngressEnvelopeProvenance::single(
                rastok::node::Path::new(["probe"]).unwrap(),
            ),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Int(1)),
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
        &Data::materialized(Value::Bytes(vec![0, 1, 2]))
    );
    assert_eq!(
        output.envelope.meta().ext().get("content_type"),
        Some(&MetaValue::Scalar(Scalar::Text(String::from("image/png")))),
        "the content type lands in ext, atomically"
    );
}

#[test]
fn maps_are_colon_punctuated_and_run() {
    use rastok::envelope::{Data, Value};
    use std::collections::BTreeMap;
    // `{ "k": v }` is an executable emission: a MAP, tag 18 — never
    // silently a struct.
    let Data::Materialized(Value::Map(map)) = emitted("{ \"a\": 1, \"b\": 2 }") else {
        panic!("a map value");
    };
    assert_eq!(map.get("a"), Some(&Value::Int(1)));
    assert_eq!(map.get("b"), Some(&Value::Int(2)));
    // `{:}` is the EMPTY map — distinct from `{}` the empty struct.
    assert_eq!(
        emitted("{:}"),
        Data::materialized(Value::Map(BTreeMap::new()))
    );
    assert_eq!(
        emitted("{}"),
        Data::materialized(Value::Struct(Vec::new())),
        "the two empties stay two values"
    );
}

#[test]
fn mixing_struct_and_map_punctuation_refuses() {
    for body in ["{ a = 1, \"b\": 2 }", "{ \"b\": 2, a = 1 }"] {
        let source = format!(
            "graph g : App {{\n  operator o {{\n    on in {{ out = {body} }}\n  }}\n  in -> o -> out\n}}\n"
        );
        let bag = compile(
            &[source.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect_err("never mixed");
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
    use rastok::envelope::{Data, Value};
    // Scalars are the Rastok expression language's spellings — `json` takes documents only
    // (D-2026-08-13-1).
    assert_eq!(emitted("42"), Data::materialized(Value::Int(42)));
    assert_eq!(
        emitted("\"text\""),
        Data::materialized(Value::Text(String::from("text")))
    );
    assert_eq!(emitted("true"), Data::materialized(Value::Bool(true)));
    // Object — a MAP (the variant JSON model), single braces.
    let Data::Materialized(Value::Map(map)) = emitted("json! { \"name\": \"Ada\" }") else {
        panic!("an object is a map");
    };
    assert_eq!(map.get("name"), Some(&Value::Text(String::from("Ada"))));
    // Array.
    assert_eq!(
        emitted("json! [1, 2, 3]"),
        Data::materialized(Value::Array(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
        ]))
    );
    // An exact fraction is the `decimal(...)` constructor's — and
    // the scale is DECLARED: trailing zeros are significant
    // (D-2026-08-13-1).
    assert_eq!(
        emitted("decimal(\"19.99\")"),
        Data::materialized(Value::Decimal {
            unscaled: 1999,
            scale: 2
        })
    );
    assert_eq!(
        emitted("decimal(\"1.50\")"),
        Data::materialized(Value::Decimal {
            unscaled: 150,
            scale: 2
        })
    );
    assert_eq!(
        emitted("decimal(\"1.5\")"),
        Data::materialized(Value::Decimal {
            unscaled: 15,
            scale: 1
        })
    );
    // The integer fold: a WRITTEN scale of zero or less collapses
    // to the int the program would otherwise spell (D-2026-08-10-7).
    assert_eq!(emitted("decimal(\"2\")"), Data::materialized(Value::Int(2)));
    // A fraction inside a DOCUMENT stays exact too — value-canonical
    // (the document's numbers are standard JSON).
    assert_eq!(
        emitted("json! [19.99]"),
        Data::materialized(Value::Array(vec![Value::Decimal {
            unscaled: 1999,
            scale: 2
        }]))
    );
}

#[test]
fn json_refusals_are_located() {
    for (body, wrong) in [
        ("json! { \"a\": 1, \"a\": 2 }", "duplicate key"),
        (
            "json! [123456789012345678901234567890123456789]",
            "39 digits",
        ),
        ("json! { \"a\": }", "syntax"),
    ] {
        let source = format!(
            "graph g : App {{\n  operator o {{\n    on in {{ out = {body} }}\n  }}\n  in -> o -> out\n}}\n"
        );
        let bag = compile(
            &[source.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect_err("must refuse");
        assert!(
            bag.diagnostics()
                .iter()
                .any(|d| !d.spans().is_empty() && d.code().as_str() == "E-JSON-VALUE"),
            "`{wrong}` refusal carries a span and the JSON code: {bag:?}"
        );
    }
}

#[test]
fn a_json_document_and_its_expression_twin_are_different_programs() {
    // The static type is identity (D-2026-08-10-7): `json! [1]` is a
    // variant, `[1]` a list — same numeric leaf, two programs.
    assert_ne!(hash_of("json! [1]"), hash_of("[1]"));
    // Minimally different pairs stay distinct too.
    assert_ne!(hash_of("json! [1]"), hash_of("json! [\"1\"]"));
    assert_ne!(hash_of("json! [19.99]"), hash_of("json! [19.98]"));
    // And the canonical collapses ARE one program: a document's
    // numbers are standard JSON, VALUE-canonical — unlike
    // `decimal(...)`, whose scale is declared (D-2026-08-13-1).
    assert_eq!(hash_of("json! [1]"), hash_of("json! [1.0]"));
    assert_eq!(hash_of("json! [1]"), hash_of("json! [1e0]"));
    assert_eq!(hash_of("json! [-0]"), hash_of("json! [0]"));
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
    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in { out = asset(\"assets/logo.bin\", \"application/octet-stream\") }\n  }\n  operator p {\n    on in { out = asset(\"assets/same.bin\") }\n  }\n  in -> o\n  o -> p\n  p -> out\n}\n";
    let compilation = compile_with_assets(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        Some(&root),
    )
    .expect("assets resolve at build");
    assert_eq!(
        compilation.plan.assets.len(),
        1,
        "identical content deduplicates to one blob"
    );
    // The hash seals the CONTENT digest, not the path: the same
    // program via the other spelling is the same identity.
    let swapped = "graph g : App {\n  input in: variant\n  operator o {\n    on in { out = asset(\"assets/same.bin\", \"application/octet-stream\") }\n  }\n  operator p {\n    on in { out = asset(\"assets/logo.bin\") }\n  }\n  in -> o\n  o -> p\n  p -> out\n}\n";
    let other = compile_with_assets(
        &[swapped],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        Some(&root),
    )
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
            "graph g : App {{\n  operator o {{\n    on in {{ out = {body} }}\n  }}\n  in -> o -> out\n}}\n"
        );
        let bag = compile_with_assets(
            &[source.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
            root,
        )
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
    use rastok::boundary::{
        Draft, IngressEnvelopeProvenance, IngressSubmission, SubmissionSequence,
    };
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

    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in { out = asset(\"report.bin\", \"application/pdf\") }\n  }\n  in -> o -> out\n}\n";
    let compilation = compile_with_assets(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        Some(&root),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(
        &mut journal,
        IngressSubmission {
            provenance: IngressEnvelopeProvenance::single(
                rastok::node::Path::new(["probe"]).unwrap(),
            ),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Int(1)),
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
        &Data::materialized(Value::Bytes(b"PDFISH".to_vec()))
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
    let aliased = "type Tag = text\n\ngraph g : App {\n  operator o {\n    input evt : Tag\n    on evt { out = data }\n  }\n  in -> o.evt\n  o -> out\n}\n";
    let first = compile(
        &[aliased],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the alias resolves");
    // …and RENAMING it is not an identity move: same structure, same
    // hash (D-2026-08-10-8).
    let renamed = "type Label = text\n\ngraph g : App {\n  operator o {\n    input evt : Label\n    on evt { out = data }\n  }\n  in -> o.evt\n  o -> out\n}\n";
    let second = compile(
        &[renamed],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the rename resolves");
    assert_eq!(first.hash, second.hash, "names are sugar, never identity");
    // The bare structural spelling is the SAME program too.
    let bare = "graph g : App {\n  operator o {\n    input evt : text\n    on evt { out = data }\n  }\n  in -> o.evt\n  o -> out\n}\n";
    let third = compile(
        &[bare],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the bare spelling");
    assert_eq!(first.hash, third.hash, "the expansion IS the identity");
}

#[test]
fn type_declarations_refuse_duplicates_cycles_and_unknowns() {
    // Duplicate.
    let source = "type T = text\ntype T = int\n\ngraph g : App {\n  operator o {\n    on in { out = data }\n  }\n  in -> o -> out\n}\n";
    let bag = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("duplicates refuse");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-DUP-TYPE"),
        "{bag:?}"
    );
    // Cycle — the reform's own Node example refuses in v1.
    let source = "type Node = struct { label: text, children: array<Node> }\n\ngraph g : App {\n  operator o {\n    input evt : Node\n    on evt { out = data }\n  }\n  in -> o.evt\n  o -> out\n}\n";
    let bag = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("recursion refuses");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-TYPE-RECURSIVE"),
        "{bag:?}"
    );
    // Unknown name at a use site.
    let source = "graph g : App {\n  operator o {\n    input evt : Ghost\n    on evt { out = data }\n  }\n  in -> o.evt\n  o -> out\n}\n";
    let bag = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("unknowns refuse");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-RESOLVE-UNKNOWN"),
        "{bag:?}"
    );
}

#[test]
fn bare_forms_embed_in_records_and_typed_forms_refuse_there() {
    use rastok::envelope::{Data, Value};
    // The bare forms are ordinary field values (D-2026-08-10-5/-6).
    let Data::Materialized(Value::Struct(fields)) = emitted(
        "{ signature = base64! { AAEC }, note = text! {\n      hi\n    }, blob = json! [1] }",
    ) else {
        panic!("a struct");
    };
    assert_eq!(
        fields
            .iter()
            .find(|(name, _)| name == "signature")
            .map(|(_, v)| v),
        Some(&Value::Bytes(vec![0, 1, 2]))
    );
    assert_eq!(
        fields
            .iter()
            .find(|(name, _)| name == "note")
            .map(|(_, v)| v),
        Some(&Value::Text(String::from("  hi")))
    );
    assert_eq!(
        fields
            .iter()
            .find(|(name, _)| name == "blob")
            .map(|(_, v)| v),
        Some(&Value::Array(vec![Value::Int(1)]))
    );
    // A content-typed form nested in a record refuses with the rule.
    let source = "graph g : App {\n  operator o {\n    on in { out = { document = base64!(\"image/png\") { AAEC } } }\n  }\n  in -> o -> out\n}\n";
    let bag = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("whole-payload only");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.message().contains("no `meta.ext` of its own")),
        "{bag:?}"
    );
}

#[test]
fn the_carry_survives_a_dead_earlier_write() {
    use rastok::boundary::{
        Draft, IngressEnvelopeProvenance, IngressSubmission, SubmissionSequence,
    };
    use rastok::envelope::{Data, Encoding, MetaValue, Namespace, Scalar};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    // `data = "temp"` then `data = data`: last-write-wins — the
    // final value is the input payload, so the representation and
    // its content type CARRY (the reviewer's counter-example).
    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in {\n      out = {\n        data = \"temp\"\n        data = data\n      }\n    }\n  }\n  in -> o -> out\n}\n";
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut ext = Namespace::new();
    ext.insert(
        String::from("content_type"),
        MetaValue::Scalar(Scalar::Text(String::from("application/avro"))),
    );
    vm.inject(
        &mut journal,
        IngressSubmission {
            provenance: IngressEnvelopeProvenance::single(
                rastok::node::Path::new(["probe"]).unwrap(),
            ),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::encoded(Encoding::Avro, *b"\x01\x02"),
                ext,
            },
            sequence: SubmissionSequence::Next,
        },
    )
    .expect("the injection lands");
    vm.drain(&mut journal).expect("no forced materialization");
    let output = &vm.outputs()[0];
    assert!(
        matches!(
            output.envelope.data(),
            Data::Encoded { encoding: Encoding::Avro, bytes, .. } if bytes.as_ref() == b"\x01\x02"
        ),
        "the final write wins and carries: {:?}",
        output.envelope.data()
    );
    assert_eq!(
        output.envelope.meta().ext().get("content_type"),
        Some(&MetaValue::Scalar(Scalar::Text(String::from(
            "application/avro"
        )))),
    );
}

#[test]
fn inject_holds_the_ext_zone_to_the_schema() {
    use rastok::boundary::{
        Draft, IngressEnvelopeProvenance, IngressSubmission, SubmissionSequence,
    };
    use rastok::envelope::{Data, MetaValue, Namespace, Scalar, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in { out = data }\n  }\n  in -> o -> out\n}\n";
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut ext = Namespace::new();
    ext.insert(
        String::from("invented"),
        MetaValue::Scalar(Scalar::Text(String::from("x"))),
    );
    let refused = vm.inject(
        &mut journal,
        IngressSubmission {
            provenance: IngressEnvelopeProvenance::single(
                rastok::node::Path::new(["probe"]).unwrap(),
            ),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Int(1)),
                ext,
            },
            sequence: SubmissionSequence::Next,
        },
    );
    assert!(refused.is_err(), "an undeclared ext field cannot enter");
    assert_eq!(journal.records().len(), 0, "refused BEFORE the append");
}

#[test]
fn a_zone_alias_cannot_dodge_the_grid() {
    // `val m = meta.app; out = m.ghost` must refuse at CHECK — a
    // zone is not a value (the reviewer's escape).
    let source = "graph g : App {\n  operator o {\n    on in {\n      val m = meta.app\n      out = m.ghost\n    }\n  }\n  in -> o -> out\n}\n";
    let bag = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("the alias refuses");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-META-ZONE"),
        "{bag:?}"
    );
}

#[test]
fn a_non_meta_typed_app_write_refuses_or_converts() {
    // An array<scalar> write now CONVERTS (spec-admitted meta)…
    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in {\n      out = {\n        meta.app.tags = [\"a\", \"b\"]\n        data = data\n      }\n    }\n  }\n  in -> o -> out\n}\n";
    compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("array<scalar> is legal meta");
    // …and an explicitly dynamic value cannot silently enter the precise
    // `media_type` field. The metadata and port gates use the same
    // assignability relation,
    // and ports, so the refusal is static rather than a runtime surprise.
    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in { out = { data = \"x\", meta.ext.content_type = data } }\n  }\n  in -> o -> out\n}\n";
    let bag = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("variant must be narrowed before a media-type write");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-PORT-TYPE"),
        "{bag:?}"
    );
}

#[test]
fn the_meta_type_grid_is_closed_end_to_end() {
    // The reviewer's three static escapes now refuse at CHECK.
    for (body, what) in [
        ("meta.app.bad = [[1]]", "a nested list"),
        ("meta.app.ttl = duration(\"1s\")", "a duration leaf"),
    ] {
        let source = format!(
            "graph g : App {{\n  operator o {{\n    on in {{\n      out = {{\n        {body}\n        data = data\n      }}\n    }}\n  }}\n  in -> o -> out\n}}\n"
        );
        let bag = compile(
            &[source.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect_err(what);
        assert!(
            bag.diagnostics()
                .iter()
                .any(|d| d.code().as_str() == "E-META-ZONE"),
            "`{what}`: {bag:?}"
        );
    }
    // And the ingress holds content_type to MEDIA_TYPE, name and type.
    use rastok::boundary::{
        Draft, IngressEnvelopeProvenance, IngressSubmission, SubmissionSequence,
    };
    use rastok::envelope::{Data, MetaValue, Namespace, Scalar, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;
    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in { out = data }\n  }\n  in -> o -> out\n}\n";
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut ext = Namespace::new();
    ext.insert(
        String::from("content_type"),
        MetaValue::Scalar(Scalar::Int(7)),
    );
    let refused = vm.inject(
        &mut journal,
        IngressSubmission {
            provenance: IngressEnvelopeProvenance::single(
                rastok::node::Path::new(["probe"]).unwrap(),
            ),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Int(1)),
                ext,
            },
            sequence: SubmissionSequence::Next,
        },
    );
    assert!(refused.is_err(), "an int content_type cannot enter");
    assert_eq!(journal.records().len(), 0, "refused BEFORE the append");
}

#[test]
fn a_branch_alias_cannot_dodge_the_grid_either() {
    // The refined escape: a projected BRANCH leaving as a value.
    let source = "graph g : App {\n  operator o {\n    on in {\n      out = {\n        meta.app.audit.origin = \"x\"\n        data = data\n      }\n    }\n  }\n  operator p {\n    on in {\n      val m = meta.app.audit\n      out = m\n    }\n  }\n  in -> o\n  o -> p\n  p -> out\n}\n";
    let bag = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("a branch is not a value");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-META-ZONE"),
        "{bag:?}"
    );
    // And a sys LEAF takes no extra segments.
    let source = "graph g : App {\n  operator o {\n    on in where meta.sys.id.extra == \"x\" { out = data }\n  }\n  in -> o -> out\n}\n";
    let bag = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("a leaf nests nothing");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-META-ZONE"),
        "{bag:?}"
    );
}

#[test]
fn comments_are_trivia_in_every_expression_form() {
    // D-2026-08-13-1 (review round): a comment changes neither the
    // AST nor the hash, whatever the expression's shape — trivia has
    // ONE authority, and Rastok expression recognizes the host's comments.
    // (Line comments in the ONE-LINE harness would comment out the
    // closing brace — correctly; they are proven on the multi-line
    // shapes below.)
    for (label, plain, commented) in [
        ("word + block comment", "data", "data /* note */"),
        ("number + block comment", "1", "1 /* numeric */"),
        ("string + block comment", "\"x\"", "\"x\" /* c */"),
        ("binary + inner block", "1 + 2", "1 /* c */ + 2"),
        ("call + block comment", "size(data)", "size(data) /* c */"),
    ] {
        assert_eq!(
            hash_of(plain),
            hash_of(commented),
            "{label}: trivia is not identity"
        );
    }
    // Before `}` on the same line, and before the line's end.
    let plain =
        "graph g : App {\n  operator o {\n    on in { out = data }\n  }\n  in -> o -> out\n}\n";
    let brace = "graph g : App {\n  operator o {\n    on in { out = data /* same */ }\n  }\n  in -> o -> out\n}\n";
    let eol = "graph g : App {\n  operator o {\n    on in {\n      out = data // end\n    }\n  }\n  in -> o -> out\n}\n";
    let hash = |source: &str| {
        let typed = source.replacen("graph g : App {", "graph g : App {\n  input in: variant", 1);
        compile(
            &[typed.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("valid")
        .hash
        .to_string()
    };
    assert_eq!(hash(plain), hash(brace), "before `}}`");
    assert_eq!(hash(plain), hash(eol), "before the newline");
    // A terminator INSIDE a comment never terminates.
    let guarded = "graph g : App {\n  operator o {\n    on in { out = 1 /* ; } */ + 2 }\n  }\n  in -> o -> out\n}\n";
    let bare =
        "graph g : App {\n  operator o {\n    on in { out = 1 + 2 }\n  }\n  in -> o -> out\n}\n";
    assert_eq!(hash(guarded), hash(bare), "`;`/`}}` inside a comment");
    // A line's end only ends a value at depth ZERO (review round 2's
    // blocker, its exact reproductions): comments inside a
    // multiline array and a parenthesized guard are trivia, and the
    // commented twin is the SAME program.
    let array_bare = "graph g : App {\n  operator o {\n    on in {\n      out = size([\n        1,\n        2\n      ])\n    }\n  }\n  in -> o -> out\n}\n";
    let array_line = "graph g : App {\n  operator o {\n    on in {\n      out = size([\n        1, // premier\n        2\n      ])\n    }\n  }\n  in -> o -> out\n}\n";
    let array_block = "graph g : App {\n  operator o {\n    on in {\n      out = size([\n        1, /* premier */\n        2\n      ])\n    }\n  }\n  in -> o -> out\n}\n";
    assert_eq!(hash(array_bare), hash(array_line), "array + line comment");
    assert_eq!(hash(array_bare), hash(array_block), "array + block comment");
    let guard_bare = "graph g : App {\n  operator o {\n    on in where (\n      true\n      && true\n    ) {\n      out = data\n    }\n  }\n  in -> o -> out\n}\n";
    let guard_line = "graph g : App {\n  operator o {\n    on in where (\n      true // condition\n      && true\n    ) {\n      out = data\n    }\n  }\n  in -> o -> out\n}\n";
    assert_eq!(hash(guard_bare), hash(guard_line), "guard + line comment");
}

#[test]
fn a_named_decimal_is_the_same_program_family() {
    // D-2026-08-13-1 (review round): naming a constant expression
    // must not change validity — the folded declared-scale constant
    // travels the package like the direct call.
    let source = "graph g : App {\n  input in: variant\n  operator o {\n    val p = decimal(\"1.50\")\n    on in { out = p }\n  }\n  in -> o -> out\n}\n";
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a named decimal compiles");
    let bytes = rastok::plan::encode(&compilation.plan, &compilation.hash);
    let (decoded, declared) = rastok::plan::decode(&bytes).expect("the package opens");
    assert_eq!(decoded, compilation.plan, "round trip");
    assert_eq!(declared, compilation.hash, "identity travels");
}

#[test]
fn the_validator_matches_the_decoder_on_decimals_and_digests() {
    use rastok::datatype::{DataType, DecimalType};
    use rastok::plan::{AssetBlob, PlanValidationError, ValidatedExecutionPlan};
    // V7 carries the surrounding declared decimal type, so a scale-zero
    // typed decimal and a trailing-zero typed decimal are both honest. The
    // document-origin integer fold remains a reader rule, not a package
    // decoder rule applied without the schema context.
    let source = "graph g : App {\n  input in: variant\n  operator o {\n    var seen : int = 0\n    on in { seen = seen + 1\n      out = data }\n  }\n  in -> o -> out\n}\n";
    let mut plan = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan")
    .plan
    .into_inner();
    // This test is about the declared initializer carrier. Remove the source
    // program's integer state update so it does not add an unrelated int32
    // constraint after the raw state type is changed below.
    let rastok::plan::Dispatch::Linear(steps) = &mut plan.handlers[0].dispatch else {
        panic!("linear state update")
    };
    steps.remove(0);
    let relay = plan
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "o")
        .expect("the operator");
    relay.vars[0].ty = DataType::Decimal(DecimalType::new(10, 0).unwrap());
    relay.vars[0].init = Some(rastok::value::Value::Decimal {
        unscaled: 20,
        scale: 0,
    });
    let validated = ValidatedExecutionPlan::try_new(plan.clone())
        .expect("a declared scale-zero decimal validates");
    let bytes = rastok::plan::encode(&validated, &rastok::plan::SemanticHash::from_bytes([7; 32]));
    rastok::plan::decode(&bytes).expect("the scale-zero decimal reads back");

    let relay = plan
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "o")
        .expect("the operator");
    relay.vars[0].ty = DataType::Decimal(DecimalType::new(10, 2).unwrap());
    relay.vars[0].init = Some(rastok::value::Value::Decimal {
        unscaled: 100,
        scale: 2,
    });
    let validated =
        ValidatedExecutionPlan::try_new(plan).expect("a declared trailing-zero decimal validates");
    let bytes = rastok::plan::encode(&validated, &rastok::plan::SemanticHash::from_bytes([8; 32]));
    rastok::plan::decode(&bytes).expect("the trailing-zero decimal reads back");
    // A blob lying about its digest cannot obtain validation.
    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in { out = data }\n  }\n  in -> o -> out\n}\n";
    let mut plan = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan")
    .plan
    .into_inner();
    plan.assets.push(AssetBlob {
        digest: [0; 32],
        bytes: vec![1, 2, 3],
    });
    assert!(matches!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::AssetDigestMismatch { .. }
    ));
}

#[test]
fn whole_data_bare_forms_run_in_the_builder() {
    use rastok::envelope::{Data, Value};
    // `out = { data = base64! { … }, meta.app.tag = "x" }` — the
    // D-5 form the review named.
    let Data::Materialized(value) = emitted("{ data = base64! { AAEC }, meta.app.tag = \"x\" }")
    else {
        panic!("materialized");
    };
    assert_eq!(value, Value::Bytes(vec![0, 1, 2]));
}

#[test]
fn asset_order_is_digest_canonical() {
    use rastok::compiler::compile_with_assets;
    use std::io::Write;

    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("asset-order");
    std::fs::create_dir_all(&root).unwrap();
    let mut a = std::fs::File::create(root.join("a.bin")).unwrap();
    a.write_all(b"alpha").unwrap();
    let mut b = std::fs::File::create(root.join("b.bin")).unwrap();
    b.write_all(b"beta").unwrap();

    // The same two assets, used in OPPOSITE source order, produce
    // the same package bytes: the table is digest-sorted at seal.
    let ab = "graph g : App {\n  input in: variant\n  operator o {\n    on in { out = asset(\"a.bin\") }\n  }\n  operator p {\n    on in { out = asset(\"b.bin\") }\n  }\n  in -> o\n  o -> p\n  p -> out\n}\n";
    let ba = "graph g : App {\n  input in: variant\n  operator o {\n    on in { out = asset(\"b.bin\") }\n  }\n  operator p {\n    on in { out = asset(\"a.bin\") }\n  }\n  in -> o\n  o -> p\n  p -> out\n}\n";
    let first = compile_with_assets(
        &[ab],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        Some(&root),
    )
    .expect("compiles");
    let second = compile_with_assets(
        &[ba],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        Some(&root),
    )
    .expect("compiles");
    let table_first: Vec<[u8; 32]> = first.plan.assets.iter().map(|blob| blob.digest).collect();
    let table_second: Vec<[u8; 32]> = second.plan.assets.iter().map(|blob| blob.digest).collect();
    assert_eq!(table_first, table_second, "one canonical table");
    assert!(
        table_first.windows(2).all(|pair| pair[0] < pair[1]),
        "strictly increasing digests"
    );
}

#[test]
fn nested_dispatch_arms_follow_the_canonical_asset_remap() {
    use rastok::compiler::compile_with_assets;
    use rastok::plan::{DispatchStep, Emission};
    use std::io::Write;

    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("dispatch-asset-order");
    std::fs::create_dir_all(&root).unwrap();
    let candidates = [
        ("alpha.bin", b"alpha".as_slice()),
        ("beta.bin", b"beta".as_slice()),
    ];
    for (name, bytes) in candidates {
        let mut file = std::fs::File::create(root.join(name)).unwrap();
        file.write_all(bytes).unwrap();
    }
    let alpha = *blake3::hash(b"alpha").as_bytes();
    let beta = *blake3::hash(b"beta").as_bytes();
    let ((first_name, first_bytes), (second_name, second_bytes)) = if alpha > beta {
        (
            ("alpha.bin", b"alpha".as_slice()),
            ("beta.bin", b"beta".as_slice()),
        )
    } else {
        (
            ("beta.bin", b"beta".as_slice()),
            ("alpha.bin", b"alpha".as_slice()),
        )
    };
    let source = format!(
        r#"
graph app : App {{
  input in: bool
  output out: bytes
  operator choose {{
    input in: bool
    output out: bytes
    on in match data {{
      true => match {{
        data => out = asset("{first_name}")
        _ => nothing
      }}
      false => out = asset("{second_name}")
    }}
  }}
  in -> choose -> out
}}
"#
    );
    let compilation = compile_with_assets(
        &[source.as_str()],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        Some(&root),
    )
    .expect("nested Dispatch asset expressions compile");

    let mut referenced: Vec<_> = compilation
        .plan
        .handlers
        .iter()
        .flat_map(|handler| handler.dispatch.steps())
        .filter_map(|step| {
            let DispatchStep::Emit {
                value: Emission::Data(tree),
                ..
            } = step
            else {
                return None;
            };
            let rastok::expr::Expr::Asset(index) = &tree.root else {
                panic!("the asset call lowers to its table index");
            };
            Some(compilation.plan.assets[*index as usize].bytes.as_slice())
        })
        .collect();
    referenced.sort_unstable();
    let mut expected = vec![first_bytes, second_bytes];
    expected.sort_unstable();
    assert_eq!(referenced, expected);
}

#[test]
fn the_review_round_three_matrix_holds() {
    use rastok::envelope::{Data, Value};
    // uint refuses statically — the runtime cannot represent it.
    let source = "graph g : App {\n  operator o {\n    on in {\n      out = {\n        meta.app.n = 3u\n        data = data\n      }\n    }\n  }\n  in -> o -> out\n}\n";
    let bag = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("uint refuses at check");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-META-ZONE"),
        "{bag:?}"
    );
    // Metadata keeps its deliberately narrower rule: one field has one
    // type. B2 must not hide an int/text conflict behind `variant` or a
    // synthesized union.
    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in {\n      out = {\n        meta.app.x = 1\n        data = data\n      }\n    }\n  }\n  operator p {\n    on in {\n      out = {\n        meta.app.x = \"not an int\"\n        data = data\n      }\n    }\n  }\n  in -> o\n  o -> p\n  p -> out\n}\n";
    let bag = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("one app field cannot change type");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-META-TYPE"),
        "{bag:?}"
    );
    // app.content_type is an ORDINARY app field — no special rule.
    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in {\n      out = {\n        meta.app.content_type = 7\n        data = data\n      }\n    }\n  }\n  in -> o -> out\n}\n";
    compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("app.content_type is application vocabulary, int included");
    // The whole-data content-typed builder forms run.
    let Data::Materialized(value) =
        emitted("{ data = base64!(\"image/png\") { AAEC }, meta.app.tag = \"x\" }")
    else {
        panic!("materialized");
    };
    assert_eq!(value, Value::Bytes(vec![0, 1, 2]));
    // And a map as the whole payload beside metadata.
    let Data::Materialized(Value::Map(map)) =
        emitted("{ data = { \"a\": 1 }, meta.app.tag = \"x\" }")
    else {
        panic!("a map payload");
    };
    assert_eq!(map.get("a"), Some(&Value::Int(1)));
}

#[test]
fn deep_structural_types_meet_the_depth_budget() {
    // 200 nested arrays — beyond the 128 structural levels D-8 pins.
    let mut ty = String::from("text");
    for _ in 0..200 {
        ty = format!("array<{ty}>");
    }
    let source = format!(
        "type Deep = {ty}\n\ngraph g : App {{\n  operator o {{\n    on in {{ out = data }}\n  }}\n  in -> o -> out\n}}\n"
    );
    let bag = compile(
        &[source.as_str()],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("the depth budget");
    let hit = bag
        .diagnostics()
        .iter()
        .find(|d| d.code().as_str() == "E-TYPE-BUDGET")
        .expect("the budget code");
    assert!(
        hit.fields().iter().any(|(name, _)| *name == "depth"),
        "the measured depth rides the payload: {hit:?}"
    );
    assert!(!hit.spans().is_empty(), "located: {hit:?}");
}

#[test]
fn lab_assets_never_reach_the_production_plan() {
    use rastok::compiler::compile_with_assets;
    use std::io::Write;

    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("lab-asset");
    std::fs::create_dir_all(&root).unwrap();
    let mut file = std::fs::File::create(root.join("seed.bin")).unwrap();
    file.write_all(b"seed").unwrap();

    // An asset in a scenario injection AND one in a mock reply: both
    // refuse — the Lab cannot extend the plan (D-2026-08-10-5), so
    // no lab-only blob can ever contaminate the production table.
    for body in [
        "app.in = asset(\"seed.bin\")",
        "mock shell { on in { out = asset(\"seed.bin\") } }\n      app.in = \"x\"",
    ] {
        let source = format!(
            "graph g : App {{\n  connector shell : Process {{\n    mode = job ; command = \"cat\"\n    codec = lines\n    idempotent = true; behavior = deterministic\n  }}\n  in -> shell -> out\n}}\n\nlab g {{\n  simulation probing on app {{\n    scenario s \"lab asset\" {{\n      {body}\n      expect \"out\" app.out count == 1\n    }}\n  }}\n}}\n"
        );
        let bag = compile_with_assets(
            &[source.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
            Some(&root),
        )
        .expect_err("a lab asset refuses");
        assert!(
            bag.diagnostics()
                .iter()
                .any(|d| d.code().as_str() == "E-ASSET"
                    && d.message().contains("a Lab cannot extend the plan")),
            "{bag:?}"
        );
    }
}

#[test]
fn the_validator_only_admits_executable_schemas() {
    use rastok::plan::{PlanValidationError, ValidatedExecutionPlan};
    let base = "graph g : App {\n  input in: variant\n  operator o {\n    on in { out = data }\n  }\n  in -> o -> out\n}\n";
    // A uint schema entry: no value can ever satisfy it — named as
    // the TYPE refusal it is.
    let mut plan = compile(
        &[base],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("valid")
    .plan
    .into_inner();
    plan.app_schema = vec![(String::from("n"), rastok::datatype::DataType::UINT)];
    assert!(matches!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::MetaSchemaType { .. }
    ));
    // A branch under the implicit ext leaf: a PREFIX collision with
    // the implicit `content_type`.
    let mut plan = compile(
        &[base],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("valid")
    .plan
    .into_inner();
    plan.ext_schema = vec![(
        String::from("content_type.foo"),
        rastok::datatype::DataType::Text,
    )];
    assert!(matches!(
        ValidatedExecutionPlan::try_new(plan).unwrap_err(),
        PlanValidationError::MetaPrefixCollision { .. }
    ));
    // Malformed dotted keys — unreachable by the projection.
    for bad in ["", ".origin", "a.", "a..b"] {
        let mut plan = compile(
            &[base],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("valid")
        .plan
        .into_inner();
        plan.app_schema = vec![(String::from(bad), rastok::datatype::DataType::Text)];
        assert!(
            matches!(
                ValidatedExecutionPlan::try_new(plan).unwrap_err(),
                PlanValidationError::MetaFieldMalformed { .. }
            ),
            "`{bad}` must refuse as malformed"
        );
    }
}

#[test]
fn a_decimal_meta_value_round_trips_through_a_copy() {
    // Admitted under variant at the ingress, an exact decimal must
    // survive an explicit handler copy — one physical vocabulary.
    use rastok::boundary::{
        Draft, IngressEnvelopeProvenance, IngressSubmission, SubmissionSequence,
    };
    use rastok::envelope::{Data, MetaValue, Namespace, Scalar, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in {\n      out = {\n        meta.app.price = data\n        data = data\n      }\n    }\n  }\n  in -> o -> out\n}\n";
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(
        &mut journal,
        IngressSubmission {
            provenance: IngressEnvelopeProvenance::single(
                rastok::node::Path::new(["probe"]).unwrap(),
            ),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Decimal {
                    unscaled: 1999,
                    scale: 2,
                }),
                ext: Namespace::new(),
            },
            sequence: SubmissionSequence::Next,
        },
    )
    .expect("the injection lands");
    vm.drain(&mut journal).expect("the copy converts");
    assert_eq!(
        vm.outputs()[0].envelope.meta().app().get("price"),
        Some(&MetaValue::Scalar(Scalar::Decimal {
            unscaled: 1999,
            scale: 2
        })),
    );
}

#[test]
fn values_nest_recursively_across_struct_and_map() {
    use rastok::envelope::{Data, Value};
    // A map inside a struct field.
    let Data::Materialized(Value::Struct(fields)) = emitted("{ nested = { \"a\": 1 } }") else {
        panic!("a struct");
    };
    let Some((_, Value::Map(inner))) = fields.iter().find(|(name, _)| name == "nested") else {
        panic!("a nested map");
    };
    assert_eq!(inner.get("a"), Some(&Value::Int(1)));
    // A map inside a map.
    let Data::Materialized(Value::Map(outer)) = emitted("{ \"row\": { \"x\": 1 } }") else {
        panic!("a map");
    };
    let Some(Value::Map(inner)) = outer.get("row") else {
        panic!("a nested map");
    };
    assert_eq!(inner.get("x"), Some(&Value::Int(1)));
    // A non-empty STRUCT inside a map: inexpressible in v1, deferred
    // BY NAME (the D-6 amendment) — dynamic nesting spells `json`.
    let source = "graph g : App {\n  operator o {\n    on in { out = { \"row\": { x = 1 } } }\n  }\n  in -> o -> out\n}\n";
    let bag = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("deferred by name");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.message().contains("a struct inside a map entry")),
        "{bag:?}"
    );
}

#[test]
fn an_inline_enum_is_a_complete_field_type() {
    let source = "type Capture = struct { reason: enum { budget, capacity } }\n\ngraph g : App {\n  input in: text\n  output out: text\n  operator o {\n    on in { out = data }\n  }\n  in -> o -> out\n}\n";
    compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("an inline enum is already part of the source type grammar");
}

#[test]
fn broken_type_declarations_refuse_even_unused() {
    for (source_ty, code, field_key, field_value, what) in [
        (
            "Ghost",
            "E-RESOLVE-UNKNOWN",
            "name",
            "Ghost",
            "unknown name",
        ),
        (
            "struct { x: int, x: text }",
            "E-TYPE-DUP",
            "field",
            "x",
            "duplicate field",
        ),
        (
            "enum { x, x }",
            "E-TYPE-DUP",
            "field",
            "x",
            "duplicate symbol",
        ),
    ] {
        let source = format!(
            "type Broken = {source_ty}\n\ngraph g : App {{\n  operator o {{\n    on in {{ out = data }}\n  }}\n  in -> o -> out\n}}\n"
        );
        let bag = compile(
            &[source.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect_err(what);
        let hit = bag
            .diagnostics()
            .iter()
            .find(|d| d.code().as_str() == code)
            .unwrap_or_else(|| panic!("`{what}`: {bag:?}"));
        assert_eq!(
            bag.diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.code().as_str() == code)
                .count(),
            1,
            "`{what}` has one diagnostic provenance: {bag:?}"
        );
        assert!(
            hit.fields()
                .iter()
                .any(|(name, value)| *name == field_key && *value == field_value),
            "`{what}` payload: {hit:?}"
        );
        assert!(!hit.spans().is_empty(), "`{what}` located: {hit:?}");
        // The stable payload never carries one key twice.
        let mut keys: Vec<&str> = hit.fields().iter().map(|(name, _)| *name).collect();
        keys.sort_unstable();
        assert!(
            keys.windows(2).all(|pair| pair[0] != pair[1]),
            "`{what}` duplicate field keys: {hit:?}"
        );
    }
    // A wide union spends the node budget alternative by
    // alternative: 5000 branches refuse.
    let wide = (0..5000).map(|_| "text").collect::<Vec<_>>().join(" | ");
    let source = format!(
        "type Wide = {wide}\n\ngraph g : App {{\n  operator o {{\n    on in {{ out = data }}\n  }}\n  in -> o -> out\n}}\n"
    );
    let bag = compile(
        &[source.as_str()],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("node budget");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-TYPE-BUDGET"
                && d.fields().iter().any(|(name, _)| *name == "nodes")),
        "{bag:?}"
    );
}

#[test]
fn map_keys_keep_their_string_semantics() {
    // A duplicate key refuses AT CHECK, on the SECOND key — the
    // registered code, never an internal-plan surprise.
    let source = "graph g : App {\n  operator o {\n    on in { out = { \"a\": 1, \"a\": 2 } }\n  }\n  in -> o -> out\n}\n";
    let bag = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("duplicates refuse");
    let hit = bag
        .diagnostics()
        .iter()
        .find(|d| d.code().as_str() == "E-DUP-KEY")
        .unwrap_or_else(|| panic!("{bag:?}"));
    assert!(!hit.spans().is_empty(), "the second key is underlined");
    // Escapes decode by the ONE string rule: the RKG map spelling
    // and the Rastok expression map spelling are the same program.
    let rkg = hash_of("{ \"a\\nb\": 1 }");
    let expr = hash_of("({ \"a\\nb\": 1 })");
    assert_eq!(rkg, expr, "one escape semantics, one hash");
    // And the decoded key is REALLY the newline form: two escaped
    // twins collide as duplicates.
    let source = "graph g : App {\n  operator o {\n    on in { out = { \"a\\nb\": 1, \"a\\nb\": 2 } }\n  }\n  in -> o -> out\n}\n";
    let bag = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("decoded twins collide");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-DUP-KEY"),
        "{bag:?}"
    );
}

#[test]
fn content_type_arguments_are_literals_only() {
    // An operator expression dressed as text refuses. Parentheses and a
    // trailing comma belong to the one generic frontend argument grammar.
    for body in [
        "base64!(\"image/\" + \"png\") { AAEC }",
        "asset(\"a\" + \".bin\")",
        "base64!() { AAEC }",
        "text!() {\n      hi\n    }",
    ] {
        let source = format!(
            "graph g : App {{\n  input in: variant\n  output out: variant\n  operator o {{\n    on in {{ out = {body} }}\n  }}\n  in -> o -> out\n}}\n"
        );
        let bag = compile(
            &[source.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect_err("shape exactness");
        // ONE contractual diagnostic — never a cascade.
        assert_eq!(
            bag.diagnostics().len(),
            1,
            "`{body}` must refuse exactly once: {bag:?}"
        );
        assert!(
            !bag.diagnostics()[0].spans().is_empty(),
            "`{body}` located: {bag:?}"
        );
    }
    assert_eq!(
        hash_of("base64!((\"image/png\")) { AAEC }"),
        hash_of("base64!(\"image/png\",) { AAEC }"),
        "argument presentation does not change the effective value",
    );
}

#[test]
fn alias_and_bare_spellings_share_the_node_budget() {
    // A Named node is REPLACED by its expansion, never counted on
    // top: the aliased and bare spellings have exactly the same
    // acceptance at the boundary (D-2026-08-10-8).
    let at_budget_bare = format!(
        "type Wide = {}\n\ngraph g : App {{\n  operator o {{\n    on in {{ out = data }}\n  }}\n  in -> o -> out\n}}\n",
        (0..4096).map(|_| "text").collect::<Vec<_>>().join(" | ")
    );
    let at_budget_aliased = format!(
        "type T = text\ntype Wide = {}\n\ngraph g : App {{\n  operator o {{\n    on in {{ out = data }}\n  }}\n  in -> o -> out\n}}\n",
        (0..4096).map(|_| "T").collect::<Vec<_>>().join(" | ")
    );
    assert_eq!(
        compile(
            &[at_budget_bare.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds")
        )
        .is_ok(),
        compile(
            &[at_budget_aliased.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds")
        )
        .is_ok(),
        "sugar carries no budget weight"
    );
    let over_bare = format!(
        "type Wide = {}\n\ngraph g : App {{\n  operator o {{\n    on in {{ out = data }}\n  }}\n  in -> o -> out\n}}\n",
        (0..4097).map(|_| "text").collect::<Vec<_>>().join(" | ")
    );
    let over_aliased = format!(
        "type T = text\ntype Wide = {}\n\ngraph g : App {{\n  operator o {{\n    on in {{ out = data }}\n  }}\n  in -> o -> out\n}}\n",
        (0..4097).map(|_| "T").collect::<Vec<_>>().join(" | ")
    );
    assert!(
        compile(
            &[over_bare.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds")
        )
        .is_err()
    );
    assert!(
        compile(
            &[over_aliased.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds")
        )
        .is_err()
    );
}

#[test]
fn nested_content_type_gets_its_own_diagnostic() {
    // The dedicated code, on the TAG's span — never E-META-ZONE.
    let source = "graph g : App {\n  operator o {\n    on in { out = { \"x\": base64!(\"image/png\") { AAEC } } }\n  }\n  in -> o -> out\n}\n";
    let bag = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("placement");
    let hit = bag
        .diagnostics()
        .iter()
        .find(|d| d.code().as_str() == "E-CONTENT-TYPE")
        .unwrap_or_else(|| panic!("{bag:?}"));
    assert!(
        hit.fields()
            .iter()
            .any(|(k, v)| *k == "tag" && *v == "base64"),
        "{hit:?}"
    );
    assert!(!hit.spans().is_empty(), "the tag is underlined");
    // Arity refuses FIRST: a malformed call is malformed wherever
    // it sits.
    let source = "graph g : App {\n  operator o {\n    on in { out = { \"x\": base64!(\"a\", \"b\") { AAEC } } }\n  }\n  in -> o -> out\n}\n";
    let bag = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("arity");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.message().contains("at most one content-type argument")),
        "{bag:?}"
    );
    assert!(
        !bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-CONTENT-TYPE"),
        "no placement noise before the arity truth: {bag:?}"
    );
}

#[test]
fn alias_chains_meet_their_own_budget() {
    // A 500-link alias chain refuses TYPED — never a compiler stack
    // exhaustion (the alias-follow budget is 256).
    let mut source = String::from("type T0 = text\n");
    for i in 1..500 {
        source.push_str(&format!("type T{i} = T{}\n", i - 1));
    }
    source.push_str(
        "\ngraph g : App {\n  operator o {\n    on in { out = data }\n  }\n  in -> o -> out\n}\n",
    );
    let bag = compile(
        &[source.as_str()],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("the alias budget");
    assert!(
        bag.diagnostics().iter().any(|d| {
            d.code().as_str() == "E-TYPE-BUDGET"
                && d.fields().iter().any(|(name, _)| *name == "aliases")
        }),
        "{bag:?}"
    );
    // And a chain UNDER the budget still resolves as pure sugar.
    let mut source = String::from("type T0 = text\n");
    for i in 1..100 {
        source.push_str(&format!("type T{i} = T{}\n", i - 1));
    }
    source.push_str(
        "\ngraph g : App {\n  operator o {\n    input evt : T99\n    on evt { out = data }\n  }\n  in -> o.evt\n  o -> out\n}\n",
    );
    compile(
        &[source.as_str()],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a hundred links resolve");
}

#[test]
fn a_misplaced_malformed_call_refuses_once_for_the_malformation() {
    // The pipeline order: shape → arguments → position. A nested
    // call with a NON-STRING argument gets the argument truth, one
    // diagnostic, never E-CONTENT-TYPE noise.
    let source = "graph g : App {\n  operator o {\n    on in { out = { \"x\": base64!(\"image/\" + \"png\") { AAEC } } }\n  }\n  in -> o -> out\n}\n";
    let bag = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("argument truth first");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:?}");
    assert!(
        bag.diagnostics()[0].message().contains("string literal"),
        "{bag:?}"
    );
    assert!(
        bag.diagnostics()[0].code().as_str() != "E-CONTENT-TYPE",
        "no placement noise: {bag:?}"
    );
}

#[test]
fn the_alias_boundary_keeps_its_variable() {
    // 255 and 256 links both compile AND keep the variable: the
    // aliased hash equals the bare `text` hash — nothing is
    // silently dropped at the boundary. 257 refuses typed.
    let graph_with = |ty: &str| {
        format!(
            "graph g : App {{\n  input in: text\n  operator o {{\n    var seen : {ty} = \"x\"\n    on in {{ seen = data\n      out = data }}\n  }}\n  in -> o -> out\n}}\n"
        )
    };
    let chain = |n: usize| {
        let mut source = String::from("type T0 = text\n");
        for i in 1..=n {
            source.push_str(&format!("type T{i} = T{}\n", i - 1));
        }
        source.push('\n');
        source.push_str(&graph_with(&format!("T{n}")));
        source
    };
    let bare = compile(
        &[graph_with("text").as_str()],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("bare")
    .hash;
    for n in [255, 256] {
        let aliased = compile(
            &[chain(n).as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap_or_else(|bag| panic!("{n} links must compile: {bag:?}"))
        .hash;
        assert_eq!(aliased, bare, "{n} links: the variable survives as sugar");
    }
    let bag = compile(
        &[chain(257).as_str()],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("257 refuses");
    assert!(
        bag.diagnostics().iter().any(|d| {
            d.code().as_str() == "E-TYPE-BUDGET"
                && d.fields().iter().any(|(name, _)| *name == "aliases")
        }),
        "{bag:?}"
    );
    // The same boundary on a PORT pin.
    let port_with = |ty: &str| {
        format!(
            "graph g : App {{\n  operator o {{\n    input evt : {ty}\n    on evt {{ out = data }}\n  }}\n  in -> o.evt\n  o -> out\n}}\n"
        )
    };
    let port_chain = {
        let mut source = String::from("type T0 = text\n");
        for i in 1..=256 {
            source.push_str(&format!("type T{i} = T{}\n", i - 1));
        }
        source.push('\n');
        source.push_str(&port_with("T256"));
        source
    };
    let bare_port = compile(
        &[port_with("text").as_str()],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("bare port")
    .hash;
    let aliased_port = compile(
        &[port_chain.as_str()],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("256 links pin the port")
    .hash;
    assert_eq!(aliased_port, bare_port, "the port pin survives as sugar");
}

#[test]
fn the_shape_gate_owns_the_body_and_speaks_once() {
    // A content-typed base64 with no payload is a malformed frontend
    // value: framing, never placement, owns the refusal.
    let source = "graph g : App {\n  operator o {\n    on in { out = { \"x\": base64!(\"image/png\") } }\n  }\n  in -> o -> out\n}\n";
    let bag = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("the body is shape");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:?}");
    assert!(
        bag.diagnostics()[0]
            .message()
            .contains("expected a braced frontend payload"),
        "{bag:?}"
    );
    assert!(
        bag.diagnostics()[0].code().as_str() != "E-CONTENT-TYPE",
        "never a placement answer to a shape fault: {bag:?}"
    );
    // A BARE tag (no parens, no body) is the parser's fault alone —
    // one diagnostic, not a parse+check cascade.
    for tag in ["text", "base64", "asset"] {
        let source = format!(
            "graph g : App {{\n  operator o {{\n    on in {{ out = {tag} }}\n  }}\n  in -> o -> out\n}}\n"
        );
        let bag = compile(
            &[source.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect_err("bare refuses");
        assert_eq!(
            bag.diagnostics().len(),
            1,
            "`{tag}` bare speaks once: {bag:?}"
        );
    }
    // Both asset arguments faulty: the PATH speaks first.
    let source = "graph g : App {\n  operator o {\n    on in { out = asset(\"a\" + \".bin\", \"x\" + \"y\") }\n  }\n  in -> o -> out\n}\n";
    let bag = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("path first");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:?}");
    let hit = &bag.diagnostics()[0];
    assert_eq!(
        hit.code().as_str(),
        "E-ASSET",
        "the registry's code: {hit:?}"
    );
    // The span points at the FIRST argument.
    let span = hit.spans()[0];
    assert!(
        span.range().start < 60,
        "the path argument is underlined, not the content type: {hit:?}"
    );
}

#[test]
fn empty_parentheses_speak_once_through_the_checker() {
    // Empty frontend args followed by no payload fail at framing; the
    // ordinary `asset()` expression fails at its function contract.
    for (body, expect) in [
        ("text!()", "expected a braced frontend payload"),
        ("base64!()", "expected a braced frontend payload"),
        ("asset()", "takes a path"),
    ] {
        let source = format!(
            "graph g : App {{\n  operator o {{\n    on in {{ out = {body} }}\n  }}\n  in -> o -> out\n}}\n"
        );
        let bag = compile(
            &[source.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect_err("refuses");
        assert_eq!(bag.diagnostics().len(), 1, "`{body}` speaks once: {bag:?}");
        assert!(
            bag.diagnostics()[0].message().contains(expect),
            "`{body}` says why: {bag:?}"
        );
    }
}
