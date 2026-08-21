// A test harness drives the OS on purpose; the frontier lints
// guard the core modules, not the evidence.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The representative M1 fixture — the pinning the multiport lifts
//! owe (D-2026-08-08-2/-3): one graph carrying a TYPED named
//! boundary input, typed operator ports, and a typed named output,
//! with its Lab scenario. Pinned here: the exact semantic hash (the
//! identity move of the port-table presence tag), the current v9 package
//! bytes, and the full golden trace whose INBOUND record seals the
//! ingress port as key 24.

use rastok::compiler::compile;
use rastok::contract::Registry;
use std::path::{Path, PathBuf};

fn example() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/gauge.rkg")
}

fn compilation() -> rastok::compiler::Compilation {
    let source = std::fs::read_to_string(example()).expect("the example reads");
    compile(
        &[source.as_str()],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid unit")
}

/// The identity move of the typed-port lift, pinned to the byte:
/// this hash includes the port-table presence tags and the pinned
/// types (D-2026-08-08-3). It moves only when the language does —
/// deliberately, with this constant re-blessed in review.
// DELIBERATE MOVE 2026-08-10 (D-2026-08-10-7): trees seal
// [type, root] — see lower.rs's pinned-hash note.
// M1.3 final review keeps this connector-free semantic hash unchanged and
// removes the lowering-only Operator `err` row from its package. That row
// was never canonical HIR identity and never belonged in the plan.
// DELIBERATE CONTAINER MOVE 2026-08-16 (M1.4 P1): the unpublished v8 draft
// transported partition descriptors and lifecycle activations. Gauge declared neither,
// so its semantic hash and causal trace stay fixed while the package checksum
// changes with the format version.
// DELIBERATE CONTAINER MOVE 2026-08-21 (M1.4 R2 checkpoint): categorical
// v9 replaces that draft with the mandatory root owner, reciprocal Node owner
// references and scoped edges. Gauge remains a persistent singleton, so its
// semantic hash stays fixed while the package body moves.
// DELIBERATE TRACE MOVE 2026-08-21 (M1.4 R3): even singleton routes and
// transactions now carry their complete instance references. The semantic
// hash and package remain fixed; only the causal trace moves.
const GAUGE_HASH: &str = "0d3ad43d9fb09c8626146342f6a50d1f4c387a8de52a47a3134a408cf74cb22e";

#[test]
fn the_semantic_hash_is_pinned() {
    let hex: String = compilation()
        .hash
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(hex, GAUGE_HASH, "an identity move must be deliberate");
}

#[test]
fn the_package_matches_its_golden() {
    let compilation = compilation();
    let bytes = rastok::plan::encode(&compilation.plan, &compilation.hash);
    let rendered: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/gauge.rkp");
    if std::env::var("RASTOK_BLESS").is_ok() {
        std::fs::write(&path, format!("{rendered}\n")).expect("a writable golden dir");
        return;
    }
    let golden = std::fs::read_to_string(&path)
        .expect("no package golden; run once with RASTOK_BLESS=1 to bless");
    assert_eq!(golden.trim(), rendered, "the v9 typed package is pinned");
}

#[test]
fn the_lab_trace_matches_its_golden_and_seals_the_ingress() {
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
    // The INBOUND record seals its named ingress as key 24.
    let trace = rastok::golden::project(&roundtrip.records);
    let (tree, _) = rastok::canonical_cbor::decode(trace.record_bytes(0).expect("a record"))
        .expect("canonical bytes");
    let rastok::canonical_cbor::Value::Map(pairs) = tree else {
        panic!("a record map");
    };
    assert_eq!(
        pairs.get(&24),
        Some(&rastok::canonical_cbor::Value::Text(String::from("feed"))),
        "the named ingress is a causal fact"
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
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/gauge-roundtrip.trace");
    if std::env::var("RASTOK_BLESS").is_ok() {
        std::fs::write(&path, &rendered).expect("a writable golden dir");
        return;
    }
    let golden = std::fs::read_to_string(&path)
        .expect("no trace golden; run once with RASTOK_BLESS=1 to bless");
    assert_eq!(golden, rendered, "the multiport trace is pinned");
}
