//! Representation is selected by the reader's type, never by guessing bytes.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use rastok::compiler::compile;
use rastok::contract::ContractCatalog;
use rastok::envelope::{Data, Delivery, Encoding, Envelope, Id, Meta, MetaSys, Value};
use rastok::instance;
use rastok::node::{OwnerPath, Port};
use rastok::plan::{GraphVersion, Slot};
use rastok::runtime::{Fault, TransactionPreparer};
use std::collections::BTreeMap;

fn count(ty: &str, data: Data) -> Result<Value, Fault> {
    let source = format!(
        "graph g : App {{\ninput in: {ty}\noutput out: int\n\
         operator count {{ input in: {ty}\noutput out: int\n\
         on in {{ out = size(data) }} }}\nin -> count -> out\n}}"
    );
    let compiled = compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let owner = OwnerPath::new(["count"]).unwrap();
    let generation = instance::Generation::new(instance::InstanceAddress::singleton(&owner), 0);
    let delivery = Delivery {
        port: Port::new("in").unwrap(),
        edge: ".in>count.in".into(),
        seq: 1,
        source_instance: generation.clone(),
        envelope: Envelope::new(Meta::new(MetaSys::root(Id::from_bytes([1; 16]), 1)), data),
    };
    let result = TransactionPreparer::new(&compiled.plan, GraphVersion(1)).fire(
        Slot::new(0),
        &generation,
        &delivery,
        &BTreeMap::new(),
    )?;
    Ok(result.record.emissions[0]
        .envelope
        .data()
        .materialized_value()
        .unwrap()
        .clone())
}

#[test]
fn a_bytes_reader_counts_utf8_bytes_not_characters() {
    assert_eq!(
        count("bytes", Data::encoded(Encoding::Raw, "é\n".as_bytes())),
        Ok(Value::Int(3)),
    );
}

#[test]
fn a_text_reader_counts_characters_and_refuses_invalid_utf8() {
    assert_eq!(
        count("text", Data::encoded(Encoding::Raw, "é\n".as_bytes())),
        Ok(Value::Int(2)),
    );
    assert!(matches!(
        count("text", Data::encoded(Encoding::Raw, [0xff])),
        Err(Fault::InvalidUtf8(_))
    ));
}

#[test]
fn external_bytes_cannot_impersonate_a_graph_born_value() {
    let value = Value::Text("hello".into());
    let bytes = rastok::canonical_cbor::encode(&rastok::canonical::value(&value));
    assert_ne!(
        rastok::canonical::data(&Data::encoded(Encoding::Raw, bytes)),
        rastok::canonical::data(&Data::materialized(value)),
    );
}

#[test]
fn meta_sys_has_no_role_binding() {
    let source = "graph g : App {\ninput in: text\noutput out: text\n\
                  operator read {\ninput in: text\noutput out: text\n\
                  on in { out = meta.sys.role }\n}\nin -> read -> out\n}";
    let errors = compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap_err();
    assert!(format!("{errors:?}").contains("role"), "{errors:?}");
}

#[test]
fn ccbor_materialization_has_one_canonical_form() {
    let value = Value::Text("é\n".into());
    let bytes = rastok::canonical_cbor::encode(&rastok::canonical::value(&value));
    let materialized = Data::materialized(value.clone());
    let encoded = Data::encoded(Encoding::Ccbor, bytes.clone());
    assert_eq!(materialized, encoded);
    assert_eq!(
        rastok::canonical::data(&materialized),
        rastok::canonical::data(&encoded)
    );
    assert_eq!(rastok::runtime::data_value(&encoded), Ok(value));
    let mut trailing = bytes;
    trailing.push(0);
    assert!(rastok::runtime::data_value(&Data::encoded(Encoding::Ccbor, trailing)).is_err());
    // A long tag containing a small integer is valid CBOR, but not Rastok's
    // canonical value form. There is exactly one encoding for that integer.
    assert!(rastok::canonical::decode_data_value(&[0x82, 3, 1]).is_err());
}

#[test]
fn a_shared_raw_payload_keeps_independent_reading_port_types() {
    use rastok::datatype::DataType;
    use rastok::runtime::DataSlot;
    let data = Data::encoded(Encoding::Raw, "é\n".as_bytes());
    let text = DataSlot::typed(&data, &DataType::Text);
    let bytes = DataSlot::typed(&data, &DataType::BYTES);
    assert_eq!(text.materialize(), Ok(Value::Text("é\n".into())));
    assert_eq!(
        bytes.materialize(),
        Ok(Value::Bytes("é\n".as_bytes().to_vec()))
    );
    assert_eq!(text.materialize(), Ok(Value::Text("é\n".into())));
    assert_eq!(
        rastok::runtime::data_value(&data),
        Ok(Value::Bytes("é\n".as_bytes().to_vec()))
    );
}

#[test]
fn raw_text_selects_the_same_key_and_handler_value_inside_a_graph() {
    use rastok::boundary::{
        Draft, IngressEnvelopeProvenance, IngressSubmission, SubmissionSequence,
    };
    use rastok::journal::{MemJournal, Record};
    use rastok::node::{Lifecycle, Path, Trigger};
    use rastok::runtime::vm::Vm;
    let source = r#"
graph g : App {
  input in: text
  output out: int
  graph names {
    input in: text
    output out: int
    instance = { key = data }
    operator count {
      input in: text
      output out: int
      on in { out = size(data) }
    }
    in -> count -> out
  }
  in -> names -> out
}
"#;
    let compiled = compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compiled.plan, GraphVersion(1));
    vm.inject(
        &mut journal,
        IngressSubmission {
            provenance: IngressEnvelopeProvenance::single(Path::new(["cli"]).unwrap()),
            port: Port::new("in").unwrap(),
            draft: Draft {
                data: Data::encoded(Encoding::Raw, "é\n".as_bytes()),
                ext: Default::default(),
            },
            sequence: SubmissionSequence::Next,
        },
    )
    .unwrap();
    vm.run_ready(&mut journal).unwrap();
    assert_eq!(
        vm.outputs()[0].envelope.data(),
        &Data::materialized(Value::Int(2))
    );
    let scope = OwnerPath::new(["names"]).unwrap();
    let expected = instance::InstanceAddress::new(
        &scope,
        &[instance::Segment::SelectedKey {
            scope: scope.clone(),
            ty: rastok::datatype::DataType::Text,
            value: Value::Text("é\n".into()),
        }],
    )
    .unwrap();
    assert!(journal.records().iter().any(|record| matches!(record,
        Record::Transaction(tx) if tx.owner_path == scope
            && tx.trigger == Trigger::Lifecycle(Lifecycle::Start)
            && tx.instance.address() == &expected
    )));
}

#[test]
fn loop_expiry_materializes_last_state_by_the_host_entry_type() {
    use rastok::boundary::{
        Draft, IngressEnvelopeProvenance, IngressSubmission, SubmissionSequence,
    };
    use rastok::journal::MemJournal;
    use rastok::node::Path;
    use rastok::runtime::vm::Vm;

    for ty in ["text", "bytes"] {
        for host in ["operator", "graph"] {
            let body = if host == "operator" {
                "on in { Loop.again }".to_string()
            } else {
                format!(
                    "operator request {{ input in: {ty}\noutput retry: {ty}\n\
                     on in {{ retry = . }} }}\nin -> request\n\
                     request.retry -> Loop.again"
                )
            };
            let source = format!(
                "graph app : App {{\ninput in: {ty}\n\
                 output err: union {{ LoopExpired {{ last_state: {ty} }} }}\n\
                 {host} repeat : Loop {{\ninput in: {ty}\n\
                 output err: union {{ LoopExpired {{ last_state: {ty} }} }}\n\
                 Loop.fuel = 1\n{body}\n}}\n\
                 in -> repeat\nrepeat.err -> err\n}}"
            );
            let compiled = compile(&[&source], &ContractCatalog::try_m0().unwrap()).unwrap();
            let mut journal = MemJournal::new(GraphVersion(1));
            let mut vm = Vm::boot(&compiled.plan, GraphVersion(1));
            vm.inject(
                &mut journal,
                IngressSubmission {
                    provenance: IngressEnvelopeProvenance::single(Path::new(["cli"]).unwrap()),
                    port: Port::new("in").unwrap(),
                    draft: Draft {
                        data: Data::encoded(Encoding::Raw, "é\n".as_bytes()),
                        ext: Default::default(),
                    },
                    sequence: SubmissionSequence::Next,
                },
            )
            .unwrap();
            vm.run_ready(&mut journal).unwrap();
            assert_eq!(vm.outputs().len(), 1, "{host} with {ty} entry");
            let Value::TaggedUnion(output) =
                rastok::runtime::data_value(vm.outputs()[0].envelope.data()).unwrap()
            else {
                unreachable!("expiry is a typed union case")
            };
            assert_eq!(output.case, "LoopExpired");
            let expected = if ty == "text" {
                Value::Text("é\n".into())
            } else {
                Value::Bytes("é\n".as_bytes().to_vec())
            };
            assert_eq!(output.payload, [("last_state".into(), expected)]);
            MemJournal::from_records(GraphVersion(1), None, journal.records()).unwrap();
            let mut recovered = Vm::boot(&compiled.plan, GraphVersion(1));
            recovered.recover_from_journal(&journal).unwrap();
            assert_eq!(recovered.outputs(), vm.outputs());

            // Both readings preserve these raw bytes, but only one agrees
            // with this host's entry and terminal. The journal fold has no
            // plan; plan-bound recovery must reject the other reading.
            let wrong = if ty == "text" {
                Value::Bytes("é\n".as_bytes().to_vec())
            } else {
                Value::Text("é\n".into())
            };
            let mut forged = journal.records().to_vec();
            let mut changed = 0;
            for record in &mut forged {
                let rastok::journal::Record::Transaction(tx) = record else {
                    continue;
                };
                let Some(packet) = tx.loop_record.as_deref_mut() else {
                    continue;
                };
                for fact in &mut packet.facts {
                    if let rastok::journal::loop_protocol::Fact::Expired { output, .. } = fact {
                        let replacement = Envelope::new(
                            output.meta().clone(),
                            Data::materialized(Value::TaggedUnion(Box::new(
                                rastok::value::TaggedUnionValue {
                                    case: "LoopExpired".into(),
                                    payload: vec![("last_state".into(), wrong.clone())],
                                },
                            ))),
                        );
                        *output = replacement.clone();
                        tx.emissions[0].envelope = replacement;
                        changed += 1;
                    }
                }
            }
            assert_eq!(changed, 1);
            let forged = MemJournal::from_records(GraphVersion(1), None, &forged).unwrap();
            let mut recovery = Vm::boot(&compiled.plan, GraphVersion(1));
            assert!(recovery.recover_from_journal(&forged).is_err());
        }
    }
}
