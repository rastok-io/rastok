//! The plan package's verification contract, per
//! `docs/architecture/plan-package.md`: the round trip preserves the
//! plan, its declared identity, AND its execution — the boundary-value
//! matrix runs on both sides of the trip; the example's package is
//! pinned like a golden; and every refusal is typed — a bumped
//! version, a corrupted body byte, a corrupted declared hash, an
//! out-of-bounds slot, a non-canonical envelope, a truncated package,
//! and foreign bytes.

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

    // A future format version — refused by name.
    let bumped = reframe(&bytes, |pairs| {
        pairs[0].1 = Cbor::Uint(3);
    });
    assert_eq!(
        plan_package::decode(&bumped).unwrap_err(),
        PlanPackageError::UnknownVersion(3)
    );

    // A corrupted DECLARED HASH — the checksum covers the whole
    // association, not the body alone (the v1 hole: this byte flip
    // was accepted).
    let lying = reframe(&bytes, |pairs| {
        let Cbor::Bytes(hash) = &mut pairs[1].1 else {
            panic!("the hash is bytes");
        };
        hash[0] ^= 0x01;
    });
    assert_eq!(
        plan_package::decode(&lying).unwrap_err(),
        PlanPackageError::BrokenChecksum
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

#[test]
fn the_boundary_value_matrix_executes_identically_across_the_trip() {
    // The refuting counter-example first: v1 decoded the long literal
    // as a uint, and the sum stopped evaluating. Then the rest of the
    // integer boundary.
    for literal in [
        "2147483648 + -2147483648",
        "2147483647",
        "2147483648",
        "-2147483648",
        "-2147483649",
        "9223372036854775807",
        "-9223372036854775807",
        "0 - 9223372036854775807",
    ] {
        let source = format!(
            "graph probe : App {{\n  operator o {{\n    on in {{ out = {literal} }}\n  }}\n\n  . -> o -> .\n}}\n"
        );
        let compilation = compile(&[source.as_str()], &Registry::m0()).expect("a valid plan");
        let bytes = plan_package::encode(&compilation.plan, &compilation.hash);
        let (decoded, _) = plan_package::decode(&bytes).expect("the package opens");
        assert_eq!(
            decoded, compilation.plan,
            "structural equality for `{literal}`"
        );
        let compiled_run = run_boundary_probe(&compilation.plan);
        let decoded_run = run_boundary_probe(&decoded);
        assert_eq!(
            compiled_run, decoded_run,
            "`{literal}` must execute identically across the trip"
        );
        assert_eq!(
            compiled_run.1, 0,
            "`{literal}` is a valid program; the run has no errors"
        );
    }
}

/// One injection through a boundary-only plan; returns the rendered
/// outputs and the error count — the execution fingerprint the matrix
/// compares.
fn run_boundary_probe(plan: &rastok::plan::ExecutionPlan) -> (Vec<String>, usize) {
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, Namespace, Scalar, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(plan, GraphVersion(1));
    let submission = InboundSubmission {
        authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
        draft: Draft {
            data: Data::materialized(Value::Scalar(Scalar::Text(String::from("go")))),
            mime_type: None,
            app: Namespace::new(),
        },
        sequence: SubmissionSequence::Next,
    };
    vm.inject(&mut journal, submission)
        .expect("the injection lands");
    vm.drain(&mut journal).expect("the drain completes");
    let outputs = vm
        .outputs()
        .iter()
        .map(|envelope| format!("{:?}", envelope.data()))
        .collect();
    (outputs, vm.errors().len())
}

#[test]
fn an_out_of_bounds_slot_refuses_typed() {
    let (mut plan, hash, _) = sealed_example();
    // A well-shaped plan naming impossible structure: the machine
    // would index past the node table (the review's Slot(999)).
    plan.edges[0].to = Some(rastok::plan::EndpointRef {
        node: Some(rastok::plan::Slot::new(999)),
        port: rastok::node::Port::new("in").unwrap(),
    });
    let bytes = plan_package::encode(&plan, &hash);
    assert!(matches!(
        plan_package::decode(&bytes).unwrap_err(),
        PlanPackageError::InvalidPlan(_)
    ));
}

#[test]
fn a_non_canonical_envelope_refuses() {
    let (_, _, bytes) = sealed_example();

    // Reordered header keys: same fields, same checksum coverage,
    // different bytes — not the canonical encoding. Raw splice,
    // because the canonical encoder itself refuses to write an
    // unsorted map: entry (1, uint 2) is bytes 5..7, entry
    // (2, bytes32) is the 35 bytes after it; swap them.
    assert_eq!(
        &bytes[4..7],
        &[0xa4, 0x01, 0x02],
        "the envelope head is stable"
    );
    assert_eq!(&bytes[7..10], &[0x02, 0x58, 0x20], "the hash entry follows");
    let mut reordered = bytes.clone();
    let version_entry: Vec<u8> = bytes[5..7].to_vec();
    let hash_entry: Vec<u8> = bytes[7..42].to_vec();
    reordered.splice(5..42, hash_entry.into_iter().chain(version_entry));
    // The canonical reader itself refuses an unsorted map — the
    // refusal surfaces at the outer parse, before the envelope
    // check even runs.
    assert!(matches!(
        plan_package::decode(&reordered).unwrap_err(),
        PlanPackageError::Malformed(_) | PlanPackageError::Truncated
    ));

    // A non-minimal integer in the header: uint 2 written on two
    // bytes (0x18 0x02) instead of one. The checksum still holds —
    // the VALUE is unchanged — so only the canonicity check refuses.
    assert_eq!(
        &bytes[4..7],
        &[0xa4, 0x01, 0x02],
        "the envelope head is stable"
    );
    let mut widened = bytes.clone();
    widened.splice(6..7, [0x18, 0x02]);
    assert!(matches!(
        plan_package::decode(&widened).unwrap_err(),
        PlanPackageError::Malformed(_)
    ));
}
