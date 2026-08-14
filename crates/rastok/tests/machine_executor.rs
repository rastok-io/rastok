//! The transaction executor and mailboxes, driven through compiled
//! probes — integration tests of the core machinery, living in the
//! std crate because they need the compiler.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
// Harness helpers live outside `#[test]` bodies, where the
// allow-unwrap-in-tests toggle does not reach; a broken fixture's
// correct report is a panic.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use rastok::boundary::{BoundaryEvent, CallOutcome};
use rastok::envelope::{Data, Envelope, Meta, MetaSys};
use rastok::journal::ConsumedPosition;
use rastok::journal::ids;
use rastok::plan::{GraphVersion, Slot, ValidatedExecutionPlan};
use rastok::runtime::{
    Delivery, Fault, Mailboxes, PartitionState, TransactionExecutor, meta_value,
};
use rastok::{envelope, node};
use std::collections::BTreeMap;

use rastok::boundary::TypedError;
use rastok::compiler::compile;
use rastok::contract::Registry;
use rastok::envelope::Encoding;

const M0_PROBE: &str = r#"
graph m0_probe : App {
  connector echo : Process {
mode = job ; command = "cat"
codec = lines
effect = idempotent ; behavior = deterministic
  }

  operator shout {
var count : int = 0
on in where data.text != "" {
  count = count + 1
  out = { data.text = data.text }
}
  }

  . -> shout -> echo -> .
}
"#;

fn probe() -> (ValidatedExecutionPlan, Slot) {
    let compilation = compile(
        &[M0_PROBE],
        &Registry::try_m0().expect("the standard catalogue holds"),
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
        envelope: Envelope::new(
            Meta::new(MetaSys::root(id, 1)),
            Data::materialized(envelope::Value::Map(data)),
        ),
    }
}

fn trigger_in() -> node::Trigger {
    node::Trigger::Port(node::Port::new("in").unwrap())
}

#[test]
fn an_identity_emission_inherits_everything_but_its_spine() {
    let source =
        "graph g : App {\n  operator relay {\n    on in { out = . }\n  }\n  . -> relay -> .\n}\n";
    let compilation = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let executor = TransactionExecutor::new(&compilation.plan, GraphVersion(1));
    let slot = Slot::new(0);
    let state = PartitionState::new()
        .cell(&compilation.plan, slot, "")
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
        envelope: Envelope::new(meta, Data::encoded(Encoding::Raw, *b"salut")),
    };
    let outcome = executor.fire(slot, &delivery, &state).unwrap();
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
    let state = PartitionState::new().cell(&plan, slot, "").unwrap();
    // A handler is *selected* by its declared activation pattern…
    let activation = node::HandlerActivation::Port(node::Port::new("in").unwrap());
    assert!(executor.handler_for(slot, &activation).is_some());
    // …while the committed record carries the *causal* trigger —
    // a different vocabulary that merely coincides on one port.
    // (The connector timeout transaction is the existing proof of
    // divergence: no activation declares `l:timeout`, yet the
    // record fires under it.)
    let outcome = executor.fire(slot, &input("salut"), &state).unwrap();
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
    let source = "graph g : App {\n  operator o {\n    on in {\n      out = {\n        meta.app.flag = \"yes\"\n        meta.app.audit.origin = \"trace\"\n        meta.ext.content_type = \"text/x-rastok\"\n      }\n    }\n  }\n  . -> o -> .\n}\n";
    let compilation = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let executor = TransactionExecutor::new(&compilation.plan, GraphVersion(1));
    let slot = Slot::new(0);
    let state = PartitionState::new()
        .cell(&compilation.plan, slot, "")
        .unwrap();
    let id = envelope::Id::derive(0x01, &[b"inbound", &0u64.to_le_bytes()]);
    let delivery = Delivery {
        port: node::Port::new("in").unwrap(),
        edge: String::from(".in>o.in"),
        seq: 1,
        envelope: Envelope::new(
            Meta::new(MetaSys::root(id, 1)),
            Data::encoded(Encoding::Raw, *b"payload"),
        ),
    };
    let outcome = executor.fire(slot, &delivery, &state).unwrap();
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
    let state = PartitionState::new().cell(&plan, slot, "").unwrap();
    assert_eq!(state.get("count"), Some(&rastok::value::Value::Int(0)));
    let delivery = input("salut");
    let outcome = executor.fire(slot, &delivery, &state).unwrap();
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
    let state = PartitionState::new().cell(&plan, slot, "").unwrap();
    let delivery = input("salut");
    let once = executor.fire(slot, &delivery, &state).unwrap();
    let twice = executor.fire(slot, &delivery, &state).unwrap();
    assert_eq!(once, twice, "same inputs, same transaction, same ids");
}

#[test]
fn a_false_guard_consumes_without_effect() {
    let (plan, slot) = probe();
    let executor = TransactionExecutor::new(&plan, GraphVersion(1));
    let state = PartitionState::new().cell(&plan, slot, "").unwrap();
    let outcome = executor.fire(slot, &input(""), &state).unwrap();
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
    let state = PartitionState::new().cell(&plan, slot, "").unwrap();
    // Avro payloads cannot be materialized in this build: the
    // handler faults, and the caller applies nothing.
    let delivery = Delivery {
        port: node::Port::new("in").unwrap(),
        edge: String::from(".in>shout.in"),
        seq: 1,
        envelope: Envelope::new(
            Meta::new(MetaSys::root(envelope::Id::from_bytes([1; 16]), 1)),
            Data::encoded(Encoding::Avro, *b"opaque"),
        ),
    };
    let fault = executor.fire(slot, &delivery, &state).unwrap_err();
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
    let mut partitions = PartitionState::new();
    partitions
        .apply(
            &plan,
            slot,
            "alice",
            &[(String::from("count"), rastok::value::Value::Int(7))],
        )
        .unwrap();
    assert_eq!(
        partitions.cell(&plan, slot, "alice").unwrap().get("count"),
        Some(&rastok::value::Value::Int(7))
    );
    assert_eq!(
        partitions.cell(&plan, slot, "bob").unwrap().get("count"),
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
    mailboxes
        .deliver(slot, "", port.clone(), String::from("e"), envelope.clone())
        .unwrap();
    mailboxes
        .deliver(slot, "", port.clone(), String::from("e"), envelope.clone())
        .unwrap();
    // Bounded: the third delivery is refused, not dropped.
    assert!(!mailboxes.has_room(slot, "", 0));
    assert!(
        mailboxes
            .deliver(slot, "", port.clone(), String::from("e"), envelope.clone())
            .is_err()
    );
    // Peek reserves without removing; pop removes.
    let head = mailboxes.peek_next().unwrap();
    assert_eq!(head.2.seq, 1);
    let first = mailboxes.pop(slot, "").unwrap();
    let second = mailboxes.pop(slot, "").unwrap();
    assert_eq!(first.seq, 1);
    assert_eq!(second.seq, 2, "per-edge sequence, FIFO order");
    assert!(mailboxes.pop(slot, "").is_none());
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
    mailboxes
        .deliver(slot, "", a, String::from("s.out>n.a"), envelope.clone())
        .unwrap();
    mailboxes
        .deliver(slot, "", b, String::from("s.out>n.b"), envelope.clone())
        .unwrap();
    assert!(mailboxes.find_by_ref(slot, "", "s.out>n.b", id).is_some());
    let removed = mailboxes
        .remove_by_ref(slot, "", "s.out>n.b", id)
        .expect("the b delivery");
    assert_eq!(removed.edge, "s.out>n.b");
    // The sibling on edge a is untouched.
    let survivor = mailboxes
        .find_by_ref(slot, "", "s.out>n.a", id)
        .expect("the a delivery survives");
    assert_eq!(survivor.edge, "s.out>n.a");
    assert!(mailboxes.remove_by_ref(slot, "", "s.out>n.b", id).is_none());
}

#[test]
fn one_node_mailbox_keeps_arrival_order_across_ports() {
    let (plan, slot) = probe();
    let _ = &plan;
    let mut mailboxes = Mailboxes::new(8);
    let z = node::Port::new("z").unwrap();
    let a = node::Port::new("a").unwrap();
    let envelope = input("x").envelope;
    mailboxes
        .deliver(slot, "", z.clone(), String::from("ez"), envelope.clone())
        .unwrap();
    mailboxes
        .deliver(slot, "", a.clone(), String::from("ea"), envelope.clone())
        .unwrap();
    // One FIFO per node instance: `z` arrived first, `z` fires
    // first — the port never reorders shared-state work.
    assert_eq!(mailboxes.pop(slot, "").unwrap().port, z);
    assert_eq!(mailboxes.pop(slot, "").unwrap().port, a);
}

#[test]
fn a_claimed_mailbox_is_skipped_until_released() {
    let (plan, slot) = probe();
    let _ = &plan;
    let mut mailboxes = Mailboxes::new(8);
    let port = node::Port::new("in").unwrap();
    mailboxes
        .deliver(slot, "", port, String::from("e"), input("x").envelope)
        .unwrap();
    mailboxes.claim(slot, "");
    assert!(
        mailboxes.peek_next().is_none(),
        "a suspended call keeps its mailbox claimed"
    );
    mailboxes.release(slot, "");
    assert!(mailboxes.peek_next().is_some());
}

#[test]
fn the_connector_outcome_matches_the_reference_trace() {
    let (plan, _) = probe();
    let executor = TransactionExecutor::new(&plan, GraphVersion(1));
    let request = input("salut").envelope;
    let connector_path = node::Path::new(["echo"]).unwrap();
    let event = BoundaryEvent {
        call_ref: ids::call_ref(
            ids::delivery_ref("shout.out>echo.in", request.meta().sys().id()),
            &[7; 32],
        ),
        connector_path: connector_path.clone(),
        emitting_id: request.meta().sys().id(),
        port: node::Port::new("err").unwrap(),
        outcome: CallOutcome::Failure(TypedError {
            kind: String::from("ExecFailed"),
            data: Data::encoded(Encoding::Raw, *b"exit 1"),
        }),
    };
    let outcome = executor.connector_outcome(
        &event,
        &node::Trigger::Port(node::Port::new("in").unwrap()),
        request.meta(),
        ConsumedPosition {
            edge: String::from("shout.out>echo.in"),
            seq: 1,
        },
    );
    assert_eq!(outcome.record.node_path, connector_path);
    assert_eq!(outcome.record.trigger.to_string(), "p:in");
    let emission = &outcome.record.emissions[0];
    assert_eq!(emission.port.name(), "err");
    let expected = ids::emission_id(
        &connector_path,
        &outcome.record.trigger,
        request.meta().sys().id(),
        &node::Port::new("err").unwrap(),
        0,
    );
    assert_eq!(emission.envelope.meta().sys().id(), expected);
    assert_eq!(
        emission.envelope.meta().sys().origin_id(),
        request.meta().sys().origin_id()
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
        PartitionState::new().cell(&plan, ghost, "").unwrap_err(),
        Fault::UnknownSlot(ghost)
    );
    assert_eq!(
        PartitionState::new()
            .apply(&plan, ghost, "", &[])
            .unwrap_err(),
        Fault::UnknownSlot(ghost)
    );
    let executor = TransactionExecutor::new(&plan, GraphVersion(1));
    assert_eq!(
        executor
            .fire(ghost, &input("salut"), &BTreeMap::new())
            .unwrap_err(),
        Fault::UnknownSlot(ghost)
    );
}
