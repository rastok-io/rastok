//! The public isolation chapter repeats an operation, not a mutable Loop state.
#![allow(clippy::expect_used, clippy::panic)]

use rastok::compiler::compile;
use rastok::connector::contract::ContractCatalog;
use rastok::journal::retry_protocol::Fact;
use rastok::journal::{CallStatus, EgressRecord, Record};
use rastok::plan::GraphVersion;

#[test]
fn two_tenants_keep_the_request_fixed_and_only_alice_retries() {
    let compilation = compile(
        &[include_str!("../../../examples/keyed-retry.rkg")],
        &ContractCatalog::try_m0().expect("contracts"),
    )
    .expect("the public tour checks");
    assert!(
        compilation
            .plan
            .nodes
            .iter()
            .all(|owner| owner.loop_spec().is_none())
    );
    assert_eq!(
        compilation
            .plan
            .nodes
            .iter()
            .filter(|owner| owner.retry_spec().is_some())
            .count(),
        1,
        "one Retry policy, no authored retry state"
    );
    let runs = rastok::lab::run_program(
        &compilation.plan,
        compilation.hash,
        GraphVersion(1),
        &compilation.labs[0],
        None,
    );
    assert_eq!(runs.len(), 2);
    assert!(runs.iter().all(|run| run.report.passed()), "{runs:#?}");
    assert_eq!(
        runs,
        rastok::lab::run_program(
            &compilation.plan,
            compilation.hash,
            GraphVersion(1),
            &compilation.labs[0],
            None
        )
    );
    let calls: Vec<_> = runs[0]
        .records
        .iter()
        .filter_map(|record| match record {
            Record::Egress(EgressRecord::Call(call)) if matches!(call.status, CallStatus::Open) => {
                Some(&call.intent)
            }
            _ => None,
        })
        .collect();
    assert_eq!(calls.len(), 3, "Alice, Bob, then Alice alone");
    assert_eq!(
        calls
            .iter()
            .map(|call| call.logical_time)
            .collect::<Vec<_>>(),
        [0, 0, 2_000_000_000].map(rastok::time::LogicalTime::from_nanoseconds)
    );
    assert_eq!(
        calls[0].prepared, calls[2].prepared,
        "no attempt field or request reconstruction"
    );
    assert_eq!(calls[0].resolved_settings, calls[2].resolved_settings);
    assert_ne!(calls[0].call_id, calls[2].call_id);
    assert_ne!(calls[0].emitting_id, calls[2].emitting_id);
    assert_ne!(
        calls[0].prepared, calls[1].prepared,
        "Bob owns a different request"
    );
    let mut dispositions = Vec::new();
    for record in &runs[0].records {
        if let Record::Transaction(tx) = record {
            for fact in tx.retry_record.iter().flat_map(|packet| &packet.facts) {
                match fact {
                    Fact::Held(batch) => {
                        assert!(tx.routes.is_empty());
                        dispositions.push((batch.attempt.operation, batch.attempt.ordinal, "held"));
                    }
                    Fact::Published(batch) => dispositions.push((
                        batch.attempt.operation,
                        batch.attempt.ordinal,
                        "published",
                    )),
                    _ => {}
                }
            }
        }
    }
    assert_eq!(dispositions.len(), 3);
    assert_eq!(
        dispositions
            .iter()
            .map(|(_, ordinal, disposition)| (*ordinal, *disposition))
            .collect::<Vec<_>>(),
        [(1, "published"), (1, "held"), (2, "published")]
    );
    assert_ne!(dispositions[0].0, dispositions[1].0);
    assert_eq!(dispositions[1].0, dispositions[2].0);

    let simulation = &compilation.labs[0].simulations[0];
    for (scenario, expected) in simulation.scenarios.iter().zip(&runs) {
        let mut session = rastok::lab::Simulator::try_new_program(
            &compilation.plan,
            compilation.hash,
            GraphVersion(1),
            &simulation.name,
            &simulation.mocks,
            scenario,
            None,
        )
        .expect("interactive Lab");
        for _ in 0..500 {
            if session.is_complete() {
                break;
            }
            if session.is_quiescent() {
                session.next_action().expect("action");
            } else {
                session.step_transaction().expect("transition");
            }
        }
        assert!(session.is_complete());
        assert_eq!(
            &session.finish(),
            expected,
            "same journal in stepping and batch"
        );
    }
}
