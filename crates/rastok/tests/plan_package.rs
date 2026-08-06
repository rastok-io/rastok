//! The plan package's verification contract, per
//! `docs/architecture/plan-package.md`: the round trip preserves the
//! plan and its declared identity, the example's package is pinned like
//! a golden, and every refusal is typed — a bumped version, a
//! corrupted body byte, a truncated package, and foreign bytes.

#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::canon::{Cbor, decode, encode};
use rastok::compiler::compile;
use rastok::plan_package::{self, PlanPackageError};
use rastok::stdlib::Registry;
use std::path::PathBuf;

fn example() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/echo.rkg")
}

fn sealed_example() -> (
    rastok::plan::ExecutionPlan,
    rastok::plan::SemanticHash,
    Vec<u8>,
) {
    let source = std::fs::read_to_string(example()).expect("the example reads");
    let compilation = compile(&[source.as_str()], &Registry::m0()).expect("a valid plan");
    let bytes = plan_package::encode(&compilation.plan, &compilation.hash);
    (compilation.plan, compilation.hash, bytes)
}

#[test]
fn the_round_trip_preserves_the_plan_and_its_identity() {
    let (plan, hash, bytes) = sealed_example();
    let (decoded, declared) = plan_package::decode(&bytes).expect("the package opens");
    assert_eq!(decoded, plan, "structural equality across the trip");
    assert_eq!(declared, hash, "the identity travels intact");
}

#[test]
fn the_example_package_matches_its_golden() {
    let (_, _, bytes) = sealed_example();
    let rendered: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/echo.rkp");
    if std::env::var("RASTOK_BLESS").is_ok() {
        std::fs::write(&path, format!("{rendered}\n")).expect("a writable golden dir");
        return;
    }
    let golden = std::fs::read_to_string(&path)
        .expect("no package golden; run once with RASTOK_BLESS=1 to bless");
    assert_eq!(golden.trim(), rendered, "the sealed bytes are pinned");
}

/// Re-frames a package after editing its envelope — the mutation helper
/// the refusal tests share.
fn reframe(bytes: &[u8], edit: impl FnOnce(&mut Vec<(u64, Cbor)>)) -> Vec<u8> {
    let rest = bytes.strip_prefix(b"RKPB").expect("the magic");
    let (envelope, _) = decode(rest).expect("the envelope decodes");
    let Cbor::Map(mut pairs) = envelope else {
        panic!("the envelope is a map");
    };
    edit(&mut pairs);
    let mut out = b"RKPB".to_vec();
    out.extend_from_slice(&encode(&Cbor::Map(pairs)));
    out
}

#[test]
fn every_refusal_is_typed() {
    let (_, _, bytes) = sealed_example();

    // Foreign bytes.
    let mut foreign = bytes.clone();
    foreign[0] = b'X';
    assert_eq!(
        plan_package::decode(&foreign).unwrap_err(),
        PlanPackageError::BadMagic
    );

    // A future format version — refused by name, seal untouched.
    let bumped = reframe(&bytes, |pairs| {
        pairs[0].1 = Cbor::Uint(2);
    });
    assert_eq!(
        plan_package::decode(&bumped).unwrap_err(),
        PlanPackageError::UnknownVersion(2)
    );

    // One corrupted body byte — the seal catches it before parsing.
    let corrupted = reframe(&bytes, |pairs| {
        let Cbor::Bytes(body) = &mut pairs[2].1 else {
            panic!("the body is bytes");
        };
        let middle = body.len() / 2;
        body[middle] ^= 0x01;
    });
    assert_eq!(
        plan_package::decode(&corrupted).unwrap_err(),
        PlanPackageError::BrokenChecksum
    );

    // A truncated package.
    let truncated = &bytes[..bytes.len() / 2];
    assert!(matches!(
        plan_package::decode(truncated).unwrap_err(),
        PlanPackageError::Truncated | PlanPackageError::Malformed(_)
    ));
}
