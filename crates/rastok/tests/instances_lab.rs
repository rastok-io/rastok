//! Public M1.4 instance scenarios. These proofs enter through source, the
//! checker, v11 lowering, and the Lab; no test reaches into VM state cells.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
#![allow(clippy::expect_used, clippy::panic)]

use rastok::canonical_cbor::{Value as Cbor, decode, encode};
use rastok::compiler::compile;
use rastok::contract::ContractCatalog;
use rastok::golden::{Trace, project};
use rastok::journal::Record;
use rastok::node;
use rastok::plan::GraphVersion;
use std::collections::BTreeMap;
use std::num::NonZeroU32;

const INSTANCE_LAB: &str = r#"
graph instances : App {
  input counted: struct { tenant: text, user: int }
  input persistent: text
  input idle: text
  input fixed: text
  input one_shot: text
  input window: text
  input cascade: text

  output count: int
  output persistent_started: text
  output idle_stopped: text
  output fixed_stopped: text
  output one_shot_started: text
  output one_shot_stopped: text
  output window_started: text
  output window_stopped: text
  output cascade_stopped: text

  graph sessions {
    input in: struct { tenant: text, user: int }
    output out: int
    instances = { key = data.tenant }

    operator counter {
      input in: struct { tenant: text, user: int }
      output out: int
      instances = { key = data.user }
      var visits: int = 0
      on in { visits = visits + 1 ; out = visits }
    }
    . -> counter -> .
  }

  operator keep {
    input in: text
    output started: text
    on start { started = "persistent" }
    on in { }
  }

  operator idle_scope {
    input in: text
    output stopped: text
    lifetime = { kind = idle ; after = duration("1s") }
    on in { }
    on stop { stopped = "idle" }
  }

  operator fixed_scope {
    input in: text
    output stopped: text
    lifetime = { kind = fixed ; after = duration("1s") }
    on in { }
    on stop { stopped = "fixed" }
  }

  operator one {
    input in: text
    output started: text
    output stopped: text
    lifetime = { kind = single_pass }
    on start { started = "one" }
    on in { }
    on stop { stopped = "one" }
  }

  operator boxes {
    input in: text
    output started: text
    output stopped: text
    lifetime = { kind = tumbling ; every = duration("1s") }
    on start { started = "window" }
    on in { }
    on stop { stopped = "window" }
  }

  graph closing_parent {
    input in: text
    output stopped: text
    lifetime = { kind = fixed ; after = duration("1s") }

    operator child {
      input in: text
      output stopped: text
      on in { }
      on stop { stopped = "child" }
    }
    . -> child
    child.stopped -> .stopped
  }

  .counted -> sessions.in
  sessions.out -> .count
  .persistent -> keep.in
  keep.started -> .persistent_started
  .idle -> idle_scope.in
  idle_scope.stopped -> .idle_stopped
  .fixed -> fixed_scope.in
  fixed_scope.stopped -> .fixed_stopped
  .one_shot -> one.in
  one.started -> .one_shot_started
  one.stopped -> .one_shot_stopped
  .window -> boxes.in
  boxes.started -> .window_started
  boxes.stopped -> .window_stopped
  .cascade -> closing_parent.in
  closing_parent.stopped -> .cascade_stopped
}

lab instances {
  simulation lifecycle on app {
    scenario key_isolation "equivalent composite keys share state; another key does not" {
      app.counted = { tenant = "acme" ; user = 7 }
      expect "first acme/7" app.count count == 1 where data == 1
      app.counted = { user = 8 ; tenant = "acme" }
      expect "acme/8 is isolated" app.count count == 1 where data == 1
      app.counted = { user = 7 ; tenant = "acme" }
      expect "equivalent acme/7 returns" app.count count == 1 where data == 2
    }

    scenario persistent_starts_once {
      app.persistent = "a"
      expect app.persistent_started count == 1
      app.persistent = "b"
      expect nothing app.persistent_started
    }

    scenario idle_closes_after_silence {
      app.idle = "a"
      expect nothing app.idle_stopped
      advance duration("1s")
      expect app.idle_stopped count == 1 where data == "idle"
    }

    scenario fixed_closes_from_start {
      app.fixed = "a"
      advance duration("1s")
      expect app.fixed_stopped count == 1 where data == "fixed"
    }

    scenario single_pass_gets_a_fresh_generation {
      app.one_shot = "a"
      expect app.one_shot_started count == 1
      expect app.one_shot_stopped count == 1
      app.one_shot = "b"
      expect app.one_shot_started count == 1
      expect app.one_shot_stopped count == 1
    }

    scenario tumbling_uses_one_generation_per_nonempty_box {
      app.window = "a"
      expect app.window_started count == 1
      advance duration("1s")
      expect app.window_stopped count == 1
      app.window = "b"
      expect app.window_started count == 1
    }

    scenario graph_close_is_inner_first_and_observable {
      app.cascade = "a"
      advance duration("1s")
      expect app.cascade_stopped count == 1 where data == "child"
    }
  }
}
"#;

const JOIN_LAB: &str = r#"
type JoinExpiry = union { JoinExpired {} }
graph joined : App {
  input left: int
  input right: int
  output out: int
  output expired: JoinExpiry

  operator pair {
    instances = { key = data }
    input left: int
    input right: int
    output out: int
    output expired: JoinExpiry
    on (left, right) within duration("1s") last 2 {
      out = left.data + right.data
    }
  }
  .left -> pair.left
  .right -> pair.right
  pair.out -> .out
  pair.expired -> .expired
}

lab joined {
  simulation correlation on app {
    scenario one_instance_only {
      app.left = 1
      expect nothing app.out
      app.right = 2
      expect nothing app.out
      app.right = 1
      expect app.out count == 1 where data == 2
    }
    scenario retained_facts_expire_explicitly {
      app.left = 7
      advance duration("1s")
      expect app.expired count == 1 where data is JoinExpired
    }
  }
}
"#;

#[test]
fn the_public_lab_observes_every_instance_profile_and_key_isolation() {
    let compilation = compile(
        &[INSTANCE_LAB],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the complete public instance lab checks");
    let runs = rastok::lab::run(&compilation.plan, GraphVersion(1), &compilation.labs[0]);
    assert_eq!(runs.len(), 7);
    for run in &runs {
        assert!(run.report.passed(), "{:#?}", run.report.failures);
    }

    let cascade = runs
        .iter()
        .find(|run| run.report.scenario == "graph_close_is_inner_first_and_observable")
        .expect("the cascade scenario ran");
    let stops: Vec<_> = cascade
        .records
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction)
                if transaction.trigger == node::Trigger::Lifecycle(node::Lifecycle::Stop) =>
            {
                Some(transaction.owner_path.to_string())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        stops,
        ["closing_parent.child", "closing_parent"],
        "the public trace proves inner-first Graph closure"
    );
}

#[test]
fn the_public_lab_observes_join_correlation_and_expiry() {
    let compilation = compile(
        &[JOIN_LAB],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the public join lab checks");
    let runs = rastok::lab::run(&compilation.plan, GraphVersion(1), &compilation.labs[0]);
    assert_eq!(runs.len(), 2);
    for run in &runs {
        assert!(run.report.passed(), "{:#?}", run.report.failures);
    }

    let correlated = runs
        .iter()
        .find(|run| run.report.scenario == "one_instance_only")
        .expect("the correlation scenario ran");
    let (index, transaction) = correlated
        .records
        .iter()
        .enumerate()
        .find_map(|(index, record)| match record {
            Record::Transaction(transaction) if transaction.join_inputs.len() == 2 => {
                Some((index, transaction))
            }
            _ => None,
        })
        .expect("one join transaction is projected");
    let trace = project(&correlated.records);
    let (tree, used) = decode(
        trace
            .record_bytes(index)
            .expect("the matching projected record exists"),
    )
    .expect("the canonical trace record decodes");
    assert_eq!(used, trace.record_bytes(index).expect("record").len());
    let Cbor::Map(fields) = tree else {
        panic!("a transaction projects as one map")
    };
    assert_eq!(
        fields.get(&34),
        Some(&Cbor::Array(
            transaction
                .join_inputs
                .iter()
                .map(|id| Cbor::Bytes(id.as_bytes().to_vec()))
                .collect()
        )),
        "trace key 34 preserves declared join-port order"
    );
    let expected_address = encode(&Cbor::Array(vec![
        Cbor::Array(vec![Cbor::Text(String::from("pair"))]),
        Cbor::Array(vec![Cbor::Array(vec![
            Cbor::Array(vec![Cbor::Text(String::from("pair"))]),
            Cbor::Uint(1),
            Cbor::Array(vec![
                Cbor::Array(vec![Cbor::Uint(3), Cbor::Uint(1), Cbor::Uint(32)]),
                Cbor::Array(vec![Cbor::Uint(2), Cbor::Uint(1)]),
            ]),
        ])]),
    ]));
    assert_eq!(
        fields.get(&31),
        Some(&Cbor::Array(vec![
            Cbor::Bytes(expected_address),
            Cbor::Uint(0),
        ])),
        "trace key 31 independently pins owner/key frame order and generation"
    );
}

#[test]
fn moving_one_virtual_shard_changes_no_semantic_or_replay_input() {
    let registry = ContractCatalog::try_m0().expect("the standard catalogue holds");
    let before = compile(&[INSTANCE_LAB], &registry).expect("the public instance lab checks");
    let after = compile(&[INSTANCE_LAB], &registry).expect("the same source checks again");
    assert_eq!(
        before.hash, after.hash,
        "physical placement is not a compiler input"
    );

    let run = |compilation: &rastok::compiler::Compilation| {
        rastok::lab::run(&compilation.plan, GraphVersion(1), &compilation.labs[0])
            .into_iter()
            .find(|run| run.report.scenario == "key_isolation")
            .expect("the keyed scenario ran")
    };
    let before_run = run(&before);
    let after_run = run(&after);
    assert!(
        before_run.report.passed(),
        "{:#?}",
        before_run.report.failures
    );
    assert!(
        after_run.report.passed(),
        "{:#?}",
        after_run.report.failures
    );

    let address = before_run
        .records
        .iter()
        .find_map(|record| match record {
            Record::Transaction(transaction)
                if transaction.owner_path.to_string() == "sessions.counter" =>
            {
                Some(transaction.instance.address().clone())
            }
            _ => None,
        })
        .expect("the public trace contains one keyed child instance");
    let shard_count = NonZeroU32::new(4093).expect("the proof uses a fixed non-zero count");
    let shard = address.virtual_shard(shard_count);

    // Placement is deliberately a lookup owned by the future coordinator,
    // outside the compiler, address and journal APIs. Reassign the exact same
    // virtual shard between two physical workers without feeding either fact
    // back into execution.
    let placement_before = BTreeMap::from([(shard, ("host-a", "worker-1"))]);
    let placement_after = BTreeMap::from([(shard, ("host-b", "worker-7"))]);
    assert_ne!(
        placement_before.get(&shard),
        placement_after.get(&shard),
        "the physical owner really moved"
    );
    assert_eq!(
        address.virtual_shard(shard_count),
        shard,
        "Worker count and assignment cannot perturb virtual sharding"
    );

    assert_eq!(
        before_run.records, after_run.records,
        "the journal facts are placement-neutral"
    );
    let before_trace = project(&before_run.records);
    let after_trace = project(&after_run.records);
    assert_eq!(before_trace, after_trace);
    let stored: Vec<Vec<u8>> = (0..before_trace.len())
        .map(|index| {
            before_trace
                .record_bytes(index)
                .expect("every projected record has bytes")
                .to_vec()
        })
        .collect();
    assert_eq!(
        Trace::from_stored(&stored),
        Some(after_trace),
        "the canonical replay input contains no Worker or Host fact"
    );
}

#[test]
fn invalid_and_missing_instance_key_paths_refuse_at_the_source_boundary() {
    let registry = ContractCatalog::try_m0().expect("the standard catalogue holds");
    for (key, expected) in [
        ("data.missing", "no declared member `missing`"),
        ("[data.tenant]", "canonical scalar/enum leaves"),
    ] {
        let source = format!(
            r#"
graph invalid : App {{
  input in: struct {{ tenant: text }}
  output out: struct {{ tenant: text }}
  operator relay {{
    input in: struct {{ tenant: text }}
    output out: struct {{ tenant: text }}
    instances = {{ key = {key} }}
    on in {{ out = . }}
  }}
  . -> relay -> .
}}
"#
        );
        let bag = compile(&[source.as_str()], &registry).expect_err("the invalid key refuses");
        assert!(
            bag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.message().contains(expected)),
            "expected `{expected}`, got {:#?}",
            bag.diagnostics()
        );
    }
}

#[test]
fn a_cycle_through_a_nested_graph_refuses_after_composition() {
    let source = r#"
graph cyclic : App {
  input in: int
  output out: int

  operator outer {
    input in: int
    output out: int
    on in { out = data }
  }
  graph nested {
    input in: int
    output out: int
    operator inner {
      input in: int
      output out: int
      on in { out = data }
    }
    .in -> inner.in
    inner.out -> .out
  }

  .in -> outer.in
  outer.out -> nested.in
  nested.out -> outer.in
  outer.out -> .out
}
"#;
    let bag = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("composition must not hide a spatial cycle");
    assert!(
        bag.diagnostics().iter().any(|diagnostic| {
            diagnostic.code().as_str() == "E-GRAPH-CYCLE" && diagnostic.message().contains("DAG")
        }),
        "{bag:?}"
    );
}
