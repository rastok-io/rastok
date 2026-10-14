#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::datatype::DataType;
use crate::envelope::{Data, Envelope, Meta, MetaSys};
use crate::instance;
use crate::node;

use crate::journal::Emission;
use crate::node::{Path, Port};
use crate::value::Value;
use alloc::boxed::Box;
use alloc::vec;

fn owner(path: &str) -> node::OwnerPath {
    if path.is_empty() {
        node::OwnerPath::root()
    } else {
        node::OwnerPath::from(&Path::new(path.split('.')).unwrap())
    }
}

fn instance(path: &str, tenant: &str, generation: u64) -> instance::Generation {
    let frame = instance::Segment::SelectedKey {
        scope: owner("conversation"),
        ty: DataType::Text,
        value: Value::Text(tenant.into()),
    };
    instance::Generation::new(
        instance::InstanceAddress::new(&owner(path), &[frame]).unwrap(),
        generation,
    )
}

fn transaction(instance: &instance::Generation, trigger: Trigger) -> TransactionRecord {
    TransactionRecord {
        instance_commands: None,
        deadlines: None,
        owner_path: instance.address().owner().clone(),
        instance: instance.clone(),
        parent_instance: None,
        close_scope: None,
        trigger,
        cause: None,
        exchange_exit: None,
        retry_record: None,
        loop_record: None,
        logical_time: None,
        consumed: vec![],
        join_inputs: vec![],
        state_delta: vec![],
        emissions: vec![],
        routes: vec![],
        deadline_ids: vec![],
    }
}

fn start(reference: &instance::Generation, parent: Option<&instance::Generation>) -> Record {
    let mut record = transaction(reference, Trigger::Lifecycle(Lifecycle::Start));
    record.parent_instance = parent.cloned();
    Record::Transaction(Box::new(record))
}

fn initial() -> (
    Vec<Record>,
    instance::Generation,
    instance::Generation,
    instance::Generation,
) {
    let root = instance::Generation::new(
        instance::InstanceAddress::singleton(&node::OwnerPath::root()),
        0,
    );
    let alice = instance("conversation", "alice", 0);
    let worker = instance("conversation.worker", "alice", 0);
    let decide = instance("conversation.decide", "alice", 0);
    let bob = instance("conversation", "bob", 0);
    let bob_worker = instance("conversation.worker", "bob", 0);
    (
        vec![
            start(&root, None),
            start(&alice, Some(&root)),
            start(&worker, Some(&alice)),
            start(&decide, Some(&alice)),
            start(&bob, Some(&root)),
            start(&bob_worker, Some(&bob)),
        ],
        alice,
        worker,
        decide,
    )
}

fn target(path: &str) -> node::OwnerPath {
    owner(path)
}

fn production(
    source: &instance::Generation,
    scope: &instance::Generation,
    target: &instance::Generation,
    seed: u8,
) -> (Record, Command) {
    let port = Port::new("end").unwrap();
    let cause = Id::derive(crate::envelope::IdKind::Root, &[&[seed]]);
    let command = Command::new(
        SourceKind::Executor,
        port.clone(),
        source.clone(),
        scope.clone(),
        target.clone(),
        cause,
    );
    let mut record = transaction(source, Trigger::Port(Port::new("in").unwrap()));
    record.emissions.push(Emission {
        port,
        index: 0,
        envelope: Envelope::new(
            Meta::new(MetaSys::root(cause, 1)),
            Data::materialized(Value::Int(1)),
        ),
    });
    record.instance_commands = Some(Box::new(InstanceCommands {
        produced: vec![command.clone()],
        consumed: None,
    }));
    (Record::Transaction(Box::new(record)), command)
}

fn close(command: &Command) -> Record {
    let mut record = transaction(&command.target, Trigger::Lifecycle(Lifecycle::Closing));
    record.cause = Some(command.cause);
    record.close_scope = Some(command.target.clone());
    record.instance_commands = Some(Box::new(InstanceCommands {
        produced: vec![],
        consumed: Some(command.id),
    }));
    Record::Transaction(Box::new(record))
}

#[test]
fn target_selection_follows_parent_instance_not_the_connector_path() {
    let (records, alice, worker, decide) = initial();
    let fold = InstanceCommandView::from_records(&records).unwrap();
    assert_eq!(
        fold.resolve(
            &decide,
            &owner("conversation"),
            &target("conversation.worker")
        )
        .unwrap(),
        (alice.clone(), worker)
    );
    assert_eq!(
        fold.resolve(&decide, &owner("conversation"), &target("conversation"))
            .unwrap(),
        (alice.clone(), alice)
    );
    assert_eq!(
        fold.resolve(
            &decide,
            &owner("conversation"),
            &target("conversation.absent")
        ),
        Err(Error::MissingTarget)
    );
}

#[test]
fn closure_is_once_per_generation_and_repeated_commands_are_retained() {
    let (mut records, alice, worker, decide) = initial();
    let (first, command) = production(&decide, &alice, &worker, 1);
    records.push(first);
    let crash_prefix = records.clone();
    let (second, duplicate_request) = production(&decide, &alice, &worker, 2);
    records.push(second);
    let closing_index = records.len();
    records.push(close(&command));
    let mut stop = transaction(&worker, Trigger::Lifecycle(Lifecycle::Stop));
    stop.cause = Some(command.cause);
    records.push(Record::Transaction(Box::new(stop)));
    let (late, late_command) = production(&decide, &alice, &worker, 3);
    records.push(late);
    let fold = InstanceCommandView::from_records(&records).unwrap();
    assert_eq!(
        fold.commands()
            .map(|(command, state)| (command.id, state))
            .collect::<Vec<_>>(),
        vec![
            (
                command.id,
                Disposition::Consumed {
                    record: closing_index
                }
            ),
            (
                duplicate_request.id,
                Disposition::Superseded {
                    record: closing_index
                }
            ),
            (
                late_command.id,
                Disposition::Superseded {
                    record: closing_index
                }
            ),
        ]
    );
    let mut recovered = InstanceCommandView::from_records(&crash_prefix).unwrap();
    assert_eq!(
        recovered
            .commands()
            .filter(|(_, state)| *state == Disposition::Pending)
            .count(),
        1
    );
    for record in &records[crash_prefix.len()..] {
        recovered.apply(record).unwrap();
    }
    assert_eq!(recovered, fold);
    let unchanged = recovered.clone();
    assert_eq!(recovered.apply(&close(&command)), Err(Error::Consumption));
    assert_eq!(
        recovered, unchanged,
        "a repeated consumption changes nothing"
    );
}

#[test]
fn multiple_keyed_children_require_an_exact_source_binding() {
    let (mut records, alice, _, decide) = initial();
    let sibling_key = instance::Segment::SelectedKey {
        scope: owner("conversation.worker"),
        ty: DataType::INT,
        value: Value::Int(2),
    };
    let parent_key = instance::Segment::SelectedKey {
        scope: owner("conversation"),
        ty: DataType::Text,
        value: Value::Text("alice".into()),
    };
    let second = instance::Generation::new(
        instance::InstanceAddress::new(&owner("conversation.worker"), &[parent_key, sibling_key])
            .unwrap(),
        0,
    );
    records.push(start(&second, Some(&alice)));
    let fold = InstanceCommandView::from_records(&records).unwrap();
    assert_eq!(
        fold.resolve(
            &decide,
            &owner("conversation"),
            &target("conversation.worker")
        ),
        Err(Error::AmbiguousTarget)
    );
    assert_eq!(
        fold.resolve(
            &second,
            &owner("conversation"),
            &target("conversation.worker")
        )
        .unwrap(),
        (alice, second)
    );
}

#[test]
fn a_retained_command_cannot_stop_bob_or_a_successor() {
    let (records, alice, worker, decide) = initial();
    for wrong in [
        instance("conversation.worker", "bob", 0),
        instance("conversation.worker", "alice", 1),
    ] {
        let (record, _) = production(&decide, &alice, &wrong, 1);
        let mut fold = InstanceCommandView::from_records(&records).unwrap();
        let before = fold.clone();
        assert_eq!(fold.apply(&record), Err(Error::Target));
        assert_eq!(fold, before);
    }
    let (record, command) = production(&decide, &alice, &worker, 1);
    let mut fold = InstanceCommandView::from_records(&records).unwrap();
    fold.apply(&record).unwrap();
    let Record::Transaction(mut forged) = close(&command) else {
        unreachable!()
    };
    forged.instance = decide.clone();
    assert_eq!(
        fold.apply(&Record::Transaction(forged)),
        Err(Error::Consumption)
    );
}

#[test]
fn consumption_preserves_pending_order_across_distinct_instances() {
    let (mut records, alice, worker, decide) = initial();
    let (first, first_command) = production(&decide, &alice, &worker, 1);
    let bob = instance("conversation", "bob", 0);
    let bob_worker = instance("conversation.worker", "bob", 0);
    let (second, second_command) = production(&bob_worker, &bob, &bob_worker, 2);
    records.extend([first, second]);
    let mut fold = InstanceCommandView::from_records(&records).unwrap();
    let before = fold.clone();
    assert_eq!(fold.apply(&close(&second_command)), Err(Error::Consumption));
    assert_eq!(fold, before);
    fold.apply(&close(&first_command)).unwrap();
    fold.apply(&close(&second_command)).unwrap();
}

#[test]
fn parent_closure_settles_only_its_own_descendant_requests() {
    let (mut records, alice, worker, decide) = initial();
    let (production, child_command) = production(&decide, &alice, &worker, 1);
    records.push(production);
    let mut close_parent = transaction(&alice, Trigger::Lifecycle(Lifecycle::Closing));
    close_parent.close_scope = Some(alice.clone());
    records.push(Record::Transaction(Box::new(close_parent)));
    let fold = InstanceCommandView::from_records(&records).unwrap();
    assert_eq!(
        fold.commands().next().unwrap(),
        (&child_command, Disposition::Superseded { record: 7 })
    );
    let bob = instance("conversation", "bob", 0);
    let bob_worker = instance("conversation.worker", "bob", 0);
    assert!(fold.instances[&bob_worker].closing.is_none());
    assert!(fold.instances[&bob].closing.is_none());
}

#[test]
fn command_identity_includes_cause_scope_target_and_generation() {
    let (_, alice, worker, decide) = initial();
    let (_, original) = production(&decide, &alice, &worker, 1);
    let mut variants = Vec::new();
    let mut command = original.clone();
    command.cause = Id::derive(crate::envelope::IdKind::Root, &[b"other"]);
    variants.push(command);
    let mut command = original.clone();
    command.target = instance("conversation.worker", "alice", 1);
    variants.push(command);
    let mut command = original.clone();
    command.scope = instance("conversation", "alice", 1);
    variants.push(command);
    let mut command = original.clone();
    command.source = Port::new("other").unwrap();
    variants.push(command);
    for command in variants {
        assert_ne!(command.derived_id(), original.id);
    }
}

#[test]
fn codec_round_trip_rejects_changed_authority_and_empty_commands() {
    use crate::canonical_cbor::{Value as Cbor, decode, encode};
    let (_, alice, worker, decide) = initial();
    let (_, command) = production(&decide, &alice, &worker, 1);
    for packet in [
        InstanceCommands {
            produced: vec![command.clone()],
            consumed: None,
        },
        InstanceCommands {
            produced: vec![],
            consumed: Some(command.id),
        },
    ] {
        let bytes = encode(&packet.tree());
        let (tree, consumed) = decode(&bytes).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(InstanceCommands::from_tree(&tree).unwrap(), packet);
    }
    assert_eq!(
        InstanceCommands::from_tree(&Cbor::Map(BTreeMap::new())),
        Err(Error::CommandsShape)
    );
    let mut forged = command;
    forged.target = instance("conversation.worker", "bob", 0);
    assert_eq!(
        InstanceCommands::from_tree(
            &InstanceCommands {
                produced: vec![forged],
                consumed: None
            }
            .tree()
        ),
        Err(Error::CommandIdentity)
    );
}

#[test]
fn command_encoding_depends_on_paths_and_source_kind_not_plan_indices() {
    use crate::canonical_cbor::{Value as Cbor, encode};
    let (_, scope, target, source) = initial();
    let cause = Id::derive(crate::envelope::IdKind::Root, &[b"closure"]);
    for (kind, tag) in [(SourceKind::Graph, 1_u8), (SourceKind::Executor, 2)] {
        let command = Command::new(
            kind,
            Port::new("end").unwrap(),
            source.clone(),
            scope.clone(),
            target.clone(),
            cause,
        );
        let expected_id = Id::derive(
            crate::envelope::IdKind::InstanceStop,
            &[
                &[tag],
                b"end",
                source.address().as_bytes(),
                &source.generation().to_le_bytes(),
                scope.address().as_bytes(),
                &scope.generation().to_le_bytes(),
                target.address().as_bytes(),
                &target.generation().to_le_bytes(),
                cause.as_bytes(),
            ],
        );
        assert_eq!(command.id, expected_id);
        let expected = Cbor::Map(BTreeMap::from([(
            1,
            Cbor::Array(vec![Cbor::Array(vec![
                Cbor::Bytes(expected_id.as_bytes().to_vec()),
                Cbor::Uint(u64::from(tag)),
                Cbor::Text("end".into()),
                source.tree(),
                scope.tree(),
                target.tree(),
                Cbor::Bytes(cause.as_bytes().to_vec()),
            ])]),
        )]));
        let commands = InstanceCommands {
            produced: vec![command],
            consumed: None,
        };
        assert_eq!(encode(&commands.tree()), encode(&expected));
        assert_eq!(InstanceCommands::from_tree(&expected).unwrap(), commands);
    }
}
