#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use crate::{compiler, connector::contract::ContractCatalog, golden, lab};

fn compile(source: &str) -> Compilation {
    compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap()
}

#[test]
fn echo_retention_bytes_are_pinned_independently_of_the_codec_location() {
    let compilation = compile(include_str!("../../../../examples/echo.rkg"));
    let bytes = canonical_cbor::encode(&codec::list(compilation.labs.iter().flat_map(|world| {
        lab::run(&compilation.plan, GraphVersion(1), world)
            .into_iter()
            .map(|run| codec::list(run.records.iter().map(codec::encode)))
    })));
    // The recording-4 tuple includes inline unsealed deadlines. Its digest
    // moves independently of causal seals; the single approval is inventoried
    // in docs/architecture/human-review-v20-identity.md.
    assert_eq!(
        blake3::hash(&bytes).to_hex().as_str(),
        "6130123972daf8e882d56c726361a0ca26a4b6ac320613e697848e6bca6abb4d"
    );
}

#[test]
fn the_echo_recording_reconstructs_every_prefix_without_the_world() {
    let compilation = compile(include_str!("../../../../examples/echo.rkg"));
    let runs = compilation
        .labs
        .iter()
        .flat_map(|world| lab::run(&compilation.plan, GraphVersion(1), world));
    for run in runs {
        assert!(run.report.passed(), "{:?}", run.report.failures);
        let journal = MemJournal::from_records(GraphVersion(1), None, &run.records).unwrap();
        let recording = Recording::capture(&compilation, &journal, 10_000_000_000).unwrap();
        let decoded = Recording::decode(&recording.encode().unwrap()).unwrap();
        assert_eq!(decoded.definitions(), compilation.source_definitions);
        assert_eq!(
            golden::project(decoded.records()),
            golden::project(&run.records)
        );
        for through in 0..=run.records.len() {
            let (prefix, _) = decoded.inspect(through).unwrap();
            assert_eq!(
                golden::project(prefix.records()),
                golden::project(&run.records[..through])
            );
        }
    }
}

#[test]
fn incomplete_corrupt_and_wrong_version_files_refuse() {
    let compilation = compile("graph g : App { input in: int ; output out: int ; in -> out }");
    let recording = Recording::capture(&compilation, &MemJournal::new(GraphVersion(1)), 0).unwrap();
    let bytes = recording.encode().unwrap();
    for end in [0, 4, 35, bytes.len() - 1] {
        assert!(Recording::decode(&bytes[..end]).is_err());
    }
    let mut corrupt = bytes.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(matches!(
        Recording::decode(&corrupt),
        Err(Error::Invalid("checksum"))
    ));
    let mut rest = &bytes[4..];
    let mut seal = [0; 32];
    let mut tree = read_frame(&mut rest, &mut seal).unwrap();
    let Cbor::Array(fields) = &mut tree else {
        panic!("capture row");
    };
    assert_eq!(fields[0], Cbor::Uint(4));
    for version in [3, 5, u64::MAX] {
        let mut tree = tree.clone();
        let Cbor::Array(fields) = &mut tree else {
            panic!("capture row");
        };
        fields[0] = Cbor::Uint(version);
        let mut forged = MAGIC.to_vec();
        forged.extend(frame(&tree, &mut [0; 32]).unwrap());
        assert!(matches!(
            Recording::decode(&forged),
            Err(Error::Unsupported("recording format version"))
        ));
    }
}

#[test]
fn a_recording_is_not_allowed_to_promote_personal_memory_to_disk() {
    let compilation = compile(
        "graph g : App { input in: text @personal ; output out: text @personal ; in -> out }",
    );
    assert!(matches!(
        Recording::capture(&compilation, &MemJournal::new(GraphVersion(1)), 0),
        Err(Error::PersonalData)
    ));
}

#[test]
fn boot_dependent_recording_refuses_instead_of_copying_resolved_values() {
    let compilation = compile(
        r#"graph g : App {
      connector client : HttpClient {
        url = env("SERVICE_URL")
        auth_bearer = secret("providers/http/token")
        codec = text
        timeout = duration("1s")
        idempotent = true
        behavior = deterministic
      }
      in -> client -> out
      client.exit -> nothing
    }"#,
    );
    assert!(matches!(
        Recording::capture(&compilation, &MemJournal::new(GraphVersion(1)), 0),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn keyed_retries_loops_and_bounded_joins_survive_recording_and_prefix_reconstruction() {
    for source in [
        include_str!("../../../../examples/keyed-retry.rkg"),
        include_str!("../../../../examples/countdown.rkg"),
        include_str!("../../../../docs/examples/expense-approval.rkg"),
    ] {
        let compilation = compile(source);
        for world in &compilation.labs {
            for run in lab::run_program(
                &compilation.plan,
                compilation.hash,
                GraphVersion(1),
                world,
                None,
            ) {
                assert!(run.report.passed(), "{:?}", run.report);
                let journal =
                    MemJournal::from_records(GraphVersion(1), None, &run.records).unwrap();
                let recording =
                    Recording::capture(&compilation, &journal, 200_000_000_000_000).unwrap();
                let decoded = Recording::decode(&recording.encode().unwrap()).unwrap();
                assert_eq!(
                    golden::project(decoded.records()),
                    golden::project(&run.records)
                );
                for through in 0..=run.records.len() {
                    decoded.inspect(through).unwrap_or_else(|error| {
                        panic!("{} prefix {through}: {error}", run.report.scenario)
                    });
                }
            }
        }
    }
}

#[test]
fn encoded_data_and_nan_metadata_are_lossless_not_a_golden_only_projection() {
    use crate::connector::exchange::EnvelopeDraft;
    use crate::connector::exchange::IngressProposal;
    use crate::connector::exchange::IngressProvenance;
    use crate::connector::exchange::IngressSequence;
    use crate::envelope::{Data, Encoding, MetaValue, Scalar};
    let mut journal = MemJournal::new(GraphVersion(1));
    for data in [
        Data::encoded(Encoding::Raw, b"opaque bytes".to_vec()),
        Data::Materialized(crate::value::Value::Decimal {
            unscaled: 12345,
            scale: 4,
        }),
    ] {
        journal
            .append_ingress(IngressProposal {
                provenance: IngressProvenance::single(crate::node::Path::new(["test"]).unwrap()),
                draft: EnvelopeDraft {
                    port: crate::node::Port::new("in").unwrap(),
                    data,
                    ext: std::collections::BTreeMap::from([(
                        String::from("nan"),
                        MetaValue::Scalar(Scalar::Double(f64::NAN)),
                    )]),
                },
                sequence: IngressSequence::Next,
                session: None,
            })
            .unwrap();
    }
    let mut reader = codec::Decoder::default();
    let decoded: Vec<_> = journal
        .records()
        .iter()
        .map(|record| reader.decode(&codec::encode(record)).unwrap())
        .collect();
    let restored = MemJournal::from_records(GraphVersion(1), None, &decoded).unwrap();
    for (before, after) in journal.records().iter().zip(restored.records()) {
        assert_eq!(codec::encode(before), codec::encode(after));
    }
    assert!(
        matches!(&decoded[0], Record::Ingress(value) if matches!(value.envelope.data(), Data::Encoded { .. }))
    );
}

#[test]
fn an_unfinished_complete_checkpoint_is_inspectable_but_not_reported_as_finished() {
    let compilation = compile("graph g : App { input in: int ; output out: int ; in -> out }");
    let recording = Recording::capture(&compilation, &MemJournal::new(GraphVersion(1)), 0).unwrap();
    let bytes = recording.encode().unwrap();
    let mut rest = &bytes[4..];
    let mut seal = [0; 32];
    read_frame(&mut rest, &mut seal).unwrap();
    read_frame(&mut rest, &mut seal).unwrap();
    let prefix = &bytes[..bytes.len() - rest.len()];
    let read = Recording::decode(prefix).unwrap();
    assert!(!read.finished());
    assert_eq!(read.encode().unwrap(), prefix);
}

#[test]
fn capture_retains_the_journals_deployment_version() {
    let compilation = compile(include_str!("../../../../examples/echo.rkg"));
    let run = lab::run(&compilation.plan, GraphVersion(7), &compilation.labs[0]).remove(0);
    let journal = MemJournal::from_records(GraphVersion(7), None, &run.records).unwrap();
    let captured = Recording::capture(&compilation, &journal, 10_000_000_000).unwrap();
    let decoded = Recording::decode(&captured.encode().unwrap()).unwrap();
    assert_eq!(decoded.version, GraphVersion(7));
    assert_eq!(
        golden::project(decoded.records()),
        golden::project(&run.records)
    );
}

#[test]
fn repeated_envelope_ids_cannot_change_their_content() {
    let compilation = compile(include_str!("../../../../examples/echo.rkg"));
    let run = lab::run(&compilation.plan, GraphVersion(1), &compilation.labs[0]).remove(0);
    let original = run
        .records
        .iter()
        .find(|record| matches!(record, Record::Ingress(_)))
        .unwrap();
    let row = codec::encode(original);
    let mut reader = codec::Decoder::default();
    reader.decode(&row).unwrap();
    reader.decode(&row).unwrap();
    let mut changed = original.clone();
    let Record::Ingress(inbound) = &mut changed else {
        panic!("inbound fixture")
    };
    inbound.envelope = crate::envelope::Envelope::new(
        inbound.envelope.meta().clone(),
        crate::envelope::Data::Materialized(crate::value::Value::Text("forged".into())),
    );
    assert!(matches!(
        reader.decode(&codec::encode(&changed)).map_err(Error::from),
        Err(Error::Invalid("inconsistent repeated envelope"))
    ));
}

#[test]
fn journal_timing_failures_remain_typed_at_the_recording_boundary() {
    use crate::journal::{codec, deadline};
    assert!(matches!(
        super::Error::from(codec::Error::Read(
            crate::canonical_cbor::DecodeError::Malformed("deadline row"),
        )),
        super::Error::Read(_),
    ));
    assert!(matches!(
        super::Error::from(codec::Error::Deadlines(
            deadline::Error::DeadlinePrecedesCreation,
        )),
        super::Error::Journal(crate::journal::Error::InvalidDeadlines(
            deadline::Error::DeadlinePrecedesCreation,
        )),
    ));
    assert!(matches!(
        super::Error::from(codec::Error::CallTiming(
            deadline::Error::MissingClosureInstant
        )),
        super::Error::Journal(crate::journal::Error::InvalidCallRetention(
            deadline::Error::MissingClosureInstant,
        )),
    ));
}

#[test]
fn record_decoder_rejects_nested_deadlines_without_changing_causal_projection() {
    let compilation = compile(include_str!("../../../../examples/echo.rkg"));
    let mut inspected = 0;
    for run in lab::run(&compilation.plan, GraphVersion(1), &compilation.labs[0]) {
        let mut reader = codec::Decoder::default();
        let mut restored = Vec::new();
        for (position, record) in run.records.iter().enumerate() {
            let tree = codec::encode(record);
            restored.push(reader.decode(&tree).unwrap());
            let field = match record {
                Record::Ingress(value) if value.deadlines.is_some() => 9,
                Record::Transaction(value) if value.deadlines.is_some() => 16,
                Record::Egress(crate::journal::EgressRecord::Call(value))
                    if value.deadlines.is_some() =>
                {
                    4
                }
                _ => continue,
            };
            let Cbor::Array(mut row) = tree else {
                panic!("record tuple")
            };
            assert!(matches!(row[field], Cbor::Array(_)));
            row[field] = Cbor::Bytes(canonical_cbor::encode(&row[field]));
            let mut forged_reader = codec::Decoder::default();
            for earlier in &run.records[..position] {
                forged_reader.decode(&codec::encode(earlier)).unwrap();
            }
            assert!(forged_reader.decode(&Cbor::Array(row)).is_err());
            inspected += 1;
        }
        assert_eq!(restored, run.records);
        assert_eq!(golden::project(&restored), golden::project(&run.records));
    }
    assert!(
        inspected > 0,
        "the echo scenarios must exercise recorded deadlines"
    );
}
