// A test harness drives the OS on purpose; the frontier lints
// guard the core modules, not the evidence.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The jsonl golden fixture (D-2026-08-11-1): one graph whose Lab
//! run opens a REAL outbound call framed by the canonical writer.
//! Pinned here: the exact semantic hash, the package bytes, and the
//! full golden trace whose EGRESS record seals the jsonl
//! `request_digest` (key 30) — the writer's bytes ARE identity, so
//! a change to the canonical form requires renewed approval of this fixture
//! with its reason.

use rastok::compiler::compile;
use rastok::contract::ContractCatalog;
use std::path::{Path, PathBuf};

fn example() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/jsonl.rkg")
}

fn compilation() -> rastok::compiler::Compilation {
    let source = std::fs::read_to_string(example()).expect("the example reads");
    compile(
        &[source.as_str()],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid unit")
}

// The standard Contract namespace movement is approved and inventoried in
// docs/architecture/human-review-v20-identity.md.
const JSONL_HASH: &str = "0aeff047b9efde5c6dc4efd6f411c5541a786fb24f816108e110eb29d00e026f";

#[test]
fn the_semantic_hash_is_pinned() {
    let hex: String = compilation()
        .hash
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(hex, JSONL_HASH, "an identity move must be deliberate");
}

#[test]
fn the_package_matches_its_golden() {
    let compilation = compilation();
    let bytes = rastok::plan::encode(&compilation.plan, &compilation.hash);
    let rendered: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/jsonl.rkp");
    if std::env::var("RASTOK_SAVE_GOLDENS").is_ok() {
        std::fs::write(&path, format!("{rendered}\n")).expect("a writable golden dir");
        return;
    }
    let golden = std::fs::read_to_string(&path)
        .expect("no package golden; run once with RASTOK_SAVE_GOLDENS=1 to save");
    assert_eq!(golden.trim(), rendered, "the jsonl package is pinned");
}

#[test]
fn the_lab_trace_matches_its_golden_and_seals_the_request_digest() {
    let compilation = compilation();
    let runs = rastok::lab::run(
        &compilation.plan,
        rastok::plan::GraphVersion(1),
        &compilation.labs[0],
    );
    let roundtrip = runs
        .iter()
        .find(|run| run.report.scenario == "roundtrip")
        .expect("the roundtrip scenario");
    assert!(
        roundtrip.report.passed(),
        "{:#?}",
        roundtrip.report.failures
    );
    // The EGRESS record seals the canonical line's digest as
    // key 30 — the writer's bytes are boundary identity.
    let trace = rastok::golden::project(&roundtrip.records);
    let mut sealed_digest = None;
    for index in 0..trace.len() {
        let (tree, _) = rastok::canonical_cbor::decode(trace.record_bytes(index).unwrap())
            .expect("canonical bytes");
        let rastok::canonical_cbor::Value::Map(pairs) = tree else {
            panic!("a record map");
        };
        if let Some(rastok::canonical_cbor::Value::Bytes(digest)) = pairs.get(&30) {
            sealed_digest = Some(digest.clone());
        }
    }
    let sealed = sealed_digest.expect("an EGRESS record carries key 30");
    // The digest is DERIVABLE: it is the canonical line's seal, so
    // the golden pins the writer's exact bytes end to end.
    let request = rastok::envelope::Data::materialized(rastok::envelope::Value::Struct(vec![
        (
            String::from("price"),
            rastok::envelope::Value::Decimal {
                unscaled: 1999,
                scale: 2,
            },
        ),
        (String::from("n"), rastok::envelope::Value::Int(42)),
    ]));
    let prepared = rastok::boundary::prepare_jsonl(&request).expect("the request frames");
    assert_eq!(
        sealed.as_slice(),
        prepared.digest().as_slice(),
        "the sealed digest IS the canonical line's"
    );
    // And the whole trace is pinned.
    let mut rendered = String::new();
    for index in 0..trace.len() {
        let bytes = trace.record_bytes(index).unwrap();
        for byte in bytes {
            rendered.push_str(&format!("{byte:02x}"));
        }
        rendered.push('\n');
    }
    rendered.push_str("seal ");
    rendered.push_str(&trace.seal_hex());
    rendered.push('\n');
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/jsonl-roundtrip.trace");
    if std::env::var("RASTOK_SAVE_GOLDENS").is_ok() {
        std::fs::write(&path, &rendered).expect("a writable golden dir");
        return;
    }
    let golden = std::fs::read_to_string(&path)
        .expect("no trace golden; run once with RASTOK_SAVE_GOLDENS=1 to save");
    assert_eq!(golden, rendered, "the jsonl trace is pinned");
}
