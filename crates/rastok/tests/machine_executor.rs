//! The transaction executor and mailboxes, driven through compiled
//! probes — integration tests of the core machinery, living in the
//! std crate because they need the compiler.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
// Harness helpers live outside `#[test]` bodies, where the
// allow-unwrap-in-tests toggle does not reach; a broken fixture's
// correct report is a panic.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use rastok::boundary::{Completion, ProcessExit};
use rastok::envelope::{Data, Envelope, Meta, MetaSys};
use rastok::identity::{InstanceAddress, InstanceFrame, InstanceRef};
use rastok::journal::ids;
use rastok::plan::{GraphVersion, OwnerPath, Slot, ValidatedExecutionPlan};
use rastok::runtime::{Delivery, Fault, InstanceState, Mailboxes, TransactionExecutor, meta_value};
use rastok::{envelope, node};
use std::collections::BTreeMap;

use rastok::compiler::compile;
use rastok::contract::ContractCatalog;
use rastok::envelope::Encoding;

const M0_PROBE: &str = r#"
graph m0_probe : App {
  connector echo : Process {
mode = job ; command = "cat"
codec = lines
effect = idempotent ; behavior = deterministic
  }

  operator shout {
input in: struct { text: text }
var count : int = 0
on in where data.text != "" {
  count = count + 1
  out = data.text
}
  }

  . -> shout -> echo -> .
}
"#;

fn probe() -> (ValidatedExecutionPlan, Slot) {
    let compilation = compile(
        &[M0_PROBE],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let slot = Slot::new(
        u32::try_from(
            compilation
                .symbols
                .nodes
                .iter()
                .position(|name| name == "shout")
                .unwrap(),
        )
        .unwrap(),
    );
    (compilation.plan, slot)
}

fn input(text: &str) -> Delivery {
    let mut data = BTreeMap::new();
    data.insert(
        String::from("text"),
        envelope::Value::Text(String::from(text)),
    );
    let id = envelope::Id::derive(0x01, &[b"inbound", &0u64.to_le_bytes()]);
    Delivery {
        port: node::Port::new("in").unwrap(),
        edge: String::from(".in>shout.in"),
        seq: 1,
        source_instance: singleton(),
        envelope: Envelope::new(
            Meta::new(MetaSys::root(id, 1)),
            Data::materialized(envelope::Value::Map(data)),
        ),
    }
}

fn singleton() -> InstanceRef {
    let owner = OwnerPath::new(["probe"]).expect("valid owner");
    InstanceRef::new(InstanceAddress::singleton(&owner), 0)
}

fn text_key(value: &str) -> InstanceRef {
    let owner = OwnerPath::new(["probe"]).expect("valid owner");
    let address = InstanceAddress::new(
        &owner,
        &[InstanceFrame::SelectedKey {
            owner: owner.clone(),
            ty: rastok::datatype::DataType::Text,
            value: rastok::value::Value::Text(String::from(value)),
        }],
    )
    .expect("text is a canonical instance key");
    InstanceRef::new(address, 0)
}

fn trigger_in() -> node::Trigger {
    node::Trigger::Port(node::Port::new("in").unwrap())
}

#[test]
fn an_identity_emission_inherits_everything_but_its_spine() {
    let source = "graph g : App {\n  input in: variant\n  operator relay {\n    on in { out = . }\n  }\n  . -> relay -> .\n}\n";
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let executor = TransactionExecutor::new(&compilation.plan, GraphVersion(1));
    let slot = Slot::new(0);
    let state = InstanceState::new()
        .cell(&compilation.plan, slot, &singleton())
        .unwrap();
    // An ancestry-rich input: app fields, ext fields, a
    // content-type hint on unchanged encoded data.
    let id = envelope::Id::derive(0x01, &[b"inbound", &0u64.to_le_bytes()]);
    let mut app = envelope::Namespace::new();
    app.insert(
        String::from("label"),
        envelope::MetaValue::Scalar(envelope::Scalar::Text(String::from("greeting"))),
    );
    let mut ext = envelope::Namespace::new();
    ext.insert(
        String::from("tenant.org"),
        envelope::MetaValue::Scalar(envelope::Scalar::Text(String::from("acme"))),
    );
    ext.insert(
        String::from("content_type"),
        envelope::MetaValue::Scalar(envelope::Scalar::Text(String::from("text/plain"))),
    );
    let meta = Meta::new(MetaSys::root(id, 1))
        .with_app(app.clone())
        .with_ext(ext.clone());
    let delivery = Delivery {
        port: node::Port::new("in").unwrap(),
        edge: String::from(".in>relay.in"),
        seq: 1,
        source_instance: singleton(),
        envelope: Envelope::new(meta, Data::encoded(Encoding::Raw, *b"salut")),
    };
    let outcome = executor
        .fire(slot, &singleton(), &delivery, &state)
        .unwrap();
    let emitted = &outcome.record.emissions[0].envelope;
    let sys = emitted.meta().sys();
    let parent = delivery.envelope.meta().sys();
    // A fresh spine…
    assert_ne!(sys.id(), parent.id());
    assert_eq!(sys.parent_id(), Some(parent.id()));
    assert_eq!(sys.origin_id(), parent.origin_id());
    // …carrying everything the ancestry attached — the app zone,
    // the ext zone, and (unchanged content) its content type.
    assert_eq!(emitted.meta().app(), &app);
    assert_eq!(emitted.meta().ext(), &ext);
    assert_eq!(emitted.data(), delivery.envelope.data());
}

#[test]
fn the_meta_projection_carries_the_whole_spine() {
    let id = envelope::Id::derive(0x01, &[b"inbound", &0u64.to_le_bytes()]);
    let mut ext = envelope::Namespace::new();
    ext.insert(
        String::from("tenant.org"),
        envelope::MetaValue::Scalar(envelope::Scalar::Text(String::from("acme"))),
    );
    ext.insert(
        String::from("content_type"),
        envelope::MetaValue::Scalar(envelope::Scalar::Text(String::from("text/plain"))),
    );
    let meta = Meta::new(MetaSys::root(id, 3)).with_ext(ext);
    let rastok::value::Value::Map(root) = meta_value(&meta) else {
        panic!("expected a map projection");
    };
    let rastok::value::Value::Map(sys) = &root["sys"] else {
        panic!("expected a sys map");
    };
    assert_eq!(sys["id"], rastok::value::Value::Text(id.to_string()));
    assert_eq!(
        sys["parent_id"],
        rastok::value::Value::Null,
        "a root has no parent"
    );
    assert_eq!(
        sys["role"],
        rastok::value::Value::Text(String::from("default"))
    );
    assert_eq!(sys["graph_version"], rastok::value::Value::Int(3));
    // The five-field spine carries no content type; the ext zone
    // does — and its flat dotted keys NEST in the projection.
    assert!(!sys.contains_key("mime_type"));
    let rastok::value::Value::Map(ext) = &root["ext"] else {
        panic!("the ext zone projects");
    };
    assert_eq!(
        ext["content_type"],
        rastok::value::Value::Text(String::from("text/plain"))
    );
    let rastok::value::Value::Map(tenant) = &ext["tenant"] else {
        panic!("flat dotted keys nest in the projection");
    };
    assert_eq!(
        tenant["org"],
        rastok::value::Value::Text(String::from("acme"))
    );
}

#[test]
fn the_activation_pattern_is_not_the_causal_trigger() {
    let (plan, slot) = probe();
    let executor = TransactionExecutor::new(&plan, GraphVersion(1));
    let state = InstanceState::new()
        .cell(&plan, slot, &singleton())
        .unwrap();
    // A handler is *selected* by its declared activation pattern…
    let activation = node::HandlerActivation::Port(node::Port::new("in").unwrap());
    assert!(executor.handler_for(slot, &activation).is_some());
    // …while the committed record carries the *causal* trigger —
    // a different vocabulary that merely coincides on one port.
    // (The connector timeout transaction is the existing proof of
    // divergence: no activation declares `l:timeout`, yet the
    // record fires under it.)
    let outcome = executor
        .fire(slot, &singleton(), &input("salut"), &state)
        .unwrap();
    assert_eq!(
        outcome.record.trigger,
        node::Trigger::Port(node::Port::new("in").unwrap())
    );
    // Their canonical renderings coincide today — that string is
    // semantic-hash material and pins the byte compatibility.
    assert_eq!(activation.to_string(), outcome.record.trigger.to_string());
}

#[test]
fn the_projection_has_exactly_three_roots() {
    // No alias, no custom root (D-2026-08-10-1): the projection
    // exposes `sys`, `app`, and `ext` — nothing else, whatever the
    // zones contain.
    let id = envelope::Id::derive(0x01, &[b"inbound", &0u64.to_le_bytes()]);
    let mut app = envelope::Namespace::new();
    app.insert(
        String::from("audit.origin"),
        envelope::MetaValue::Scalar(envelope::Scalar::Text(String::from("app-value"))),
    );
    let meta = Meta::new(MetaSys::root(id, 1)).with_app(app);
    let rastok::value::Value::Map(root) = meta_value(&meta) else {
        panic!("a map projection");
    };
    assert_eq!(
        root.keys().collect::<Vec<_>>(),
        vec!["app", "ext", "sys"],
        "three zones, no other root"
    );
    // The flat dotted key nests under its zone.
    let Some(rastok::value::Value::Map(app)) = root.get("app") else {
        panic!("the app zone projects");
    };
    let Some(rastok::value::Value::Map(audit)) = app.get("audit") else {
        panic!("dotted keys nest");
    };
    assert_eq!(
        audit.get("origin"),
        Some(&rastok::value::Value::Text(String::from("app-value")))
    );
}

#[test]
fn metadata_writes_land_where_the_spec_says() {
    // The three-zone model (D-2026-08-10-1/-4): `meta.app.flag`
    // lands in the app zone as a flat dotted key,
    // `meta.ext.content_type` is the standard ext field, and a
    // metadata-only patch never transcodes the payload.
    let source = "graph g : App {\n  input in: variant\n  operator o {\n    on in {\n      out = {\n        meta.app.flag = \"yes\"\n        meta.app.audit.origin = \"trace\"\n        meta.ext.content_type = \"text/x-rastok\"\n      }\n    }\n  }\n  . -> o -> .\n}\n";
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let executor = TransactionExecutor::new(&compilation.plan, GraphVersion(1));
    let slot = Slot::new(0);
    let state = InstanceState::new()
        .cell(&compilation.plan, slot, &singleton())
        .unwrap();
    let id = envelope::Id::derive(0x01, &[b"inbound", &0u64.to_le_bytes()]);
    let delivery = Delivery {
        port: node::Port::new("in").unwrap(),
        edge: String::from(".in>o.in"),
        seq: 1,
        source_instance: singleton(),
        envelope: Envelope::new(
            Meta::new(MetaSys::root(id, 1)),
            Data::encoded(Encoding::Raw, *b"payload"),
        ),
    };
    let outcome = executor
        .fire(slot, &singleton(), &delivery, &state)
        .unwrap();
    let emitted = &outcome.record.emissions[0].envelope;
    // The app zone holds its flat dotted keys.
    assert_eq!(
        emitted.meta().app().get("flag"),
        Some(&envelope::MetaValue::Scalar(envelope::Scalar::Text(
            String::from("yes")
        )))
    );
    assert_eq!(
        emitted.meta().app().get("audit.origin"),
        Some(&envelope::MetaValue::Scalar(envelope::Scalar::Text(
            String::from("trace")
        )))
    );
    // The standard ext field takes the explicit write.
    assert_eq!(
        emitted.meta().ext().get("content_type"),
        Some(&envelope::MetaValue::Scalar(envelope::Scalar::Text(
            String::from("text/x-rastok")
        )))
    );
    // A metadata-only patch never transcodes: the encoded bytes
    // ship untouched.
    assert_eq!(
        emitted.data(),
        &Data::encoded(Encoding::Raw, *b"payload"),
        "no structural read, no transcode"
    );
}

#[test]
fn a_transaction_commits_everything_together() {
    let (plan, slot) = probe();
    let executor = TransactionExecutor::new(&plan, GraphVersion(1));
    let state = InstanceState::new()
        .cell(&plan, slot, &singleton())
        .unwrap();
    assert_eq!(state.get("count"), Some(&rastok::value::Value::Int(0)));
    let delivery = input("salut");
    let outcome = executor
        .fire(slot, &singleton(), &delivery, &state)
        .unwrap();
    // The commit packet: consumed cursor, state delta, emission.
    assert_eq!(outcome.record.consumed[0].edge, ".in>shout.in");
    assert_eq!(outcome.record.consumed[0].seq, 1);
    assert_eq!(
        outcome.record.state_delta,
        vec![(String::from("count"), rastok::value::Value::Int(1))]
    );
    assert_eq!(outcome.record.emissions.len(), 1);
    let emission = &outcome.record.emissions[0];
    assert_eq!(emission.port.name(), "out");
    assert_eq!(emission.index, 0);
    // The emitted envelope follows the recipes: derived id,
    // parented on the input, origin inherited.
    let parent = delivery.envelope.meta().sys();
    let expected = ids::emission_id(
        &node::Path::new(["shout"]).unwrap(),
        &trigger_in(),
        parent.id(),
        &node::Port::new("out").unwrap(),
        0,
    );
    let sys = emission.envelope.meta().sys();
    assert_eq!(sys.id(), expected);
    assert_eq!(sys.parent_id(), Some(parent.id()));
    assert_eq!(sys.origin_id(), parent.origin_id());
}

#[test]
fn execution_is_deterministic() {
    let (plan, slot) = probe();
    let executor = TransactionExecutor::new(&plan, GraphVersion(1));
    let state = InstanceState::new()
        .cell(&plan, slot, &singleton())
        .unwrap();
    let delivery = input("salut");
    let once = executor
        .fire(slot, &singleton(), &delivery, &state)
        .unwrap();
    let twice = executor
        .fire(slot, &singleton(), &delivery, &state)
        .unwrap();
    assert_eq!(once, twice, "same inputs, same transaction, same ids");
}

#[test]
fn a_false_guard_consumes_without_effect() {
    let (plan, slot) = probe();
    let executor = TransactionExecutor::new(&plan, GraphVersion(1));
    let state = InstanceState::new()
        .cell(&plan, slot, &singleton())
        .unwrap();
    let outcome = executor
        .fire(slot, &singleton(), &input(""), &state)
        .unwrap();
    assert!(outcome.record.emissions.is_empty());
    assert!(outcome.record.state_delta.is_empty());
    assert_eq!(
        outcome.record.consumed.len(),
        1,
        "the cursor still advances"
    );
}

#[test]
fn a_faulted_transaction_changes_nothing() {
    let (plan, slot) = probe();
    let executor = TransactionExecutor::new(&plan, GraphVersion(1));
    let state = InstanceState::new()
        .cell(&plan, slot, &singleton())
        .unwrap();
    // Avro payloads cannot be materialized in this build: the
    // handler faults, and the caller applies nothing.
    let delivery = Delivery {
        port: node::Port::new("in").unwrap(),
        edge: String::from(".in>shout.in"),
        seq: 1,
        source_instance: singleton(),
        envelope: Envelope::new(
            Meta::new(MetaSys::root(envelope::Id::from_bytes([1; 16]), 1)),
            Data::encoded(Encoding::Avro, *b"opaque"),
        ),
    };
    let fault = executor
        .fire(slot, &singleton(), &delivery, &state)
        .unwrap_err();
    assert!(matches!(fault, Fault::Opaque(_)));
    assert_eq!(
        state.get("count"),
        Some(&rastok::value::Value::Int(0)),
        "untouched"
    );
}

#[test]
fn state_is_isolated_between_keys() {
    let (plan, slot) = probe();
    let mut instances = InstanceState::new();
    let alice = text_key("alice");
    let bob = text_key("bob");
    instances
        .apply(
            &plan,
            slot,
            &alice,
            &[(String::from("count"), rastok::value::Value::Int(7))],
        )
        .unwrap();
    assert_eq!(
        instances.cell(&plan, slot, &alice).unwrap().get("count"),
        Some(&rastok::value::Value::Int(7))
    );
    assert_eq!(
        instances.cell(&plan, slot, &bob).unwrap().get("count"),
        Some(&rastok::value::Value::Int(0)),
        "bob's cell starts from the plan defaults"
    );
}

#[test]
fn mailboxes_are_fifo_bounded_and_edge_sequenced() {
    let (plan, slot) = probe();
    let _ = &plan;
    let mut mailboxes = Mailboxes::new(2);
    let port = node::Port::new("in").unwrap();
    let envelope = input("a").envelope;
    let key = singleton();
    let source = singleton();
    mailboxes
        .deliver(
            slot,
            &key,
            &source,
            port.clone(),
            String::from("e"),
            envelope.clone(),
        )
        .unwrap();
    mailboxes
        .deliver(
            slot,
            &key,
            &source,
            port.clone(),
            String::from("e"),
            envelope.clone(),
        )
        .unwrap();
    // Bounded: the third delivery is refused, not dropped.
    assert!(!mailboxes.has_room(slot, &key, 0));
    assert!(
        mailboxes
            .deliver(
                slot,
                &key,
                &source,
                port.clone(),
                String::from("e"),
                envelope.clone(),
            )
            .is_err()
    );
    // Peek reserves without removing; pop removes.
    let head = mailboxes.peek_next().unwrap();
    assert_eq!(head.2.seq, 1);
    let first = mailboxes.pop(slot, &key).unwrap();
    let second = mailboxes.pop(slot, &key).unwrap();
    assert_eq!(first.seq, 1);
    assert_eq!(second.seq, 2, "per-edge sequence, FIFO order");
    assert!(mailboxes.pop(slot, &key).is_none());
    assert!(mailboxes.is_empty());
}

#[test]
fn the_delivery_reference_selects_among_same_envelope_deliveries() {
    // One envelope, two edges into one node: (edge, id) is the
    // reference — find and remove touch only their own delivery.
    let (plan, slot) = probe();
    let _ = &plan;
    let mut mailboxes = Mailboxes::new(8);
    let envelope = input("x").envelope;
    let id = envelope.meta().sys().id();
    let a = node::Port::new("a").unwrap();
    let b = node::Port::new("b").unwrap();
    let key = singleton();
    let source = singleton();
    mailboxes
        .deliver(
            slot,
            &key,
            &source,
            a,
            String::from("s.out>n.a"),
            envelope.clone(),
        )
        .unwrap();
    mailboxes
        .deliver(
            slot,
            &key,
            &source,
            b,
            String::from("s.out>n.b"),
            envelope.clone(),
        )
        .unwrap();
    assert!(mailboxes.find_by_ref(slot, &key, "s.out>n.b", id).is_some());
    let removed = mailboxes
        .remove_by_ref(slot, &key, "s.out>n.b", id)
        .expect("the b delivery");
    assert_eq!(removed.edge, "s.out>n.b");
    // The sibling on edge a is untouched.
    let survivor = mailboxes
        .find_by_ref(slot, &key, "s.out>n.a", id)
        .expect("the a delivery survives");
    assert_eq!(survivor.edge, "s.out>n.a");
    assert!(
        mailboxes
            .remove_by_ref(slot, &key, "s.out>n.b", id)
            .is_none()
    );
}

#[test]
fn one_node_mailbox_keeps_arrival_order_across_ports() {
    let (plan, slot) = probe();
    let _ = &plan;
    let mut mailboxes = Mailboxes::new(8);
    let z = node::Port::new("z").unwrap();
    let a = node::Port::new("a").unwrap();
    let envelope = input("x").envelope;
    let key = singleton();
    let source = singleton();
    mailboxes
        .deliver(
            slot,
            &key,
            &source,
            z.clone(),
            String::from("ez"),
            envelope.clone(),
        )
        .unwrap();
    mailboxes
        .deliver(
            slot,
            &key,
            &source,
            a.clone(),
            String::from("ea"),
            envelope.clone(),
        )
        .unwrap();
    // One FIFO per node instance: `z` arrived first, `z` fires
    // first — the port never reorders shared-state work.
    assert_eq!(mailboxes.pop(slot, &key).unwrap().port, z);
    assert_eq!(mailboxes.pop(slot, &key).unwrap().port, a);
}

#[test]
fn a_claimed_mailbox_is_skipped_until_released() {
    let (plan, slot) = probe();
    let _ = &plan;
    let mut mailboxes = Mailboxes::new(8);
    let port = node::Port::new("in").unwrap();
    let key = singleton();
    let source = singleton();
    mailboxes
        .deliver(
            slot,
            &key,
            &source,
            port,
            String::from("e"),
            input("x").envelope,
        )
        .unwrap();
    mailboxes.claim(slot, &key);
    assert!(
        mailboxes.peek_next().is_none(),
        "a suspended call keeps its mailbox claimed"
    );
    mailboxes.release(slot, &key);
    assert!(mailboxes.peek_next().is_some());
}

#[test]
fn the_connector_completion_matches_the_reference_trace() {
    let (plan, _) = probe();
    let executor = TransactionExecutor::new(&plan, GraphVersion(1));
    let request = input("salut");
    let connector_path = node::Path::new(["echo"]).unwrap();
    let connector_slot = Slot::new(
        u32::try_from(
            plan.nodes
                .iter()
                .position(|planned| planned.path == connector_path)
                .expect("the probe contains its Process connector"),
        )
        .expect("the tiny probe slot is representable"),
    );
    let completion = Completion::process(
        vec![
            Data::materialized(envelope::Value::Text(String::from("first"))),
            Data::materialized(envelope::Value::Text(String::from("second"))),
        ],
        vec![Data::materialized(envelope::Value::Text(String::from(
            "diagnostic",
        )))],
        ProcessExit::Exited {
            exit_code: 1,
            signal: None,
        },
    );
    let trigger = node::Trigger::Port(node::Port::new("in").unwrap());
    let outcome = executor
        .connector_completion(
            connector_slot,
            &singleton(),
            &request,
            &trigger,
            &completion,
        )
        .expect("the validated connector slot settles");
    assert_eq!(
        outcome.record.owner_path,
        rastok_core::plan::OwnerPath::from(&connector_path)
    );
    assert_eq!(outcome.record.trigger.to_string(), "p:in");
    assert_eq!(outcome.record.emissions.len(), 4);
    let ports_and_indexes: Vec<_> = outcome
        .record
        .emissions
        .iter()
        .map(|emission| (emission.port.name(), emission.index))
        .collect();
    assert_eq!(
        ports_and_indexes,
        [("out", 0), ("out", 1), ("err", 0), ("exit", 0)]
    );
    let emission = &outcome.record.emissions[0];
    assert_eq!(emission.port.name(), "out");
    let expected = ids::emission_id(
        &connector_path,
        &outcome.record.trigger,
        request.envelope.meta().sys().id(),
        &node::Port::new("out").unwrap(),
        0,
    );
    assert_eq!(emission.envelope.meta().sys().id(), expected);
    assert_eq!(
        emission.envelope.meta().sys().origin_id(),
        request.envelope.meta().sys().origin_id()
    );
}

#[test]
fn a_caller_supplied_slot_is_verified_not_trusted() {
    // Slot is freely constructible; the validated plan proves only
    // its OWN references. The reviewer's one-liner panicked here —
    // every public door now verifies and refuses typed.
    let (plan, _) = probe();
    let ghost = Slot::new(999);
    assert_eq!(
        InstanceState::new()
            .cell(&plan, ghost, &singleton())
            .unwrap_err(),
        Fault::UnknownSlot(ghost)
    );
    assert_eq!(
        InstanceState::new()
            .apply(&plan, ghost, &singleton(), &[])
            .unwrap_err(),
        Fault::UnknownSlot(ghost)
    );
    let executor = TransactionExecutor::new(&plan, GraphVersion(1));
    assert_eq!(
        executor
            .fire(ghost, &singleton(), &input("salut"), &BTreeMap::new())
            .unwrap_err(),
        Fault::UnknownSlot(ghost)
    );
}
