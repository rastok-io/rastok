//! Retry inspection borrows the accepted protocol fold and links its original
//! records. It never exposes pinned request/settings or synthesizes deliveries.

use super::*;
use crate::journal::retry_protocol::{Fact, Phase};

pub(super) fn spec(owner: &crate::plan::Node) -> JsonValue {
    owner.retry_spec().map_or(JsonValue::Null, |spec| json!({
        "entry": spec.entry.name(),
        "terminal": spec.terminal.name(),
        "attempts": match spec.attempts.resolved { Value::Int(n) => Some(n.to_string()), _ => None },
        "cases": spec.cases,
        "delay": spec.delay.as_ref().map(|delay| format!("{delay:?}")),
    }))
}

pub(super) fn operations(vm: &Execution<'_>, records: &[Record]) -> JsonValue {
    let operations = vm
        .retry_operations()
        .map(|(reference, operation)| {
            let mut owner = None;
            let mut attempts = BTreeMap::<u64, JsonValue>::new();
            for (index, record) in records.iter().enumerate() {
                match record {
                    Record::Transaction(tx) => {
                        let Some(packet) = &tx.retry_record else {
                            continue;
                        };
                        for fact in packet
                            .facts
                            .iter()
                            .filter(|fact| fact.operation() == *reference)
                        {
                            match fact {
                                Fact::Opened { .. } => owner = Some(tx.owner_path.to_string()),
                                Fact::Admitted {
                                    attempt, envelope, ..
                                } => {
                                    let row = attempts.entry(attempt.ordinal).or_insert_with(
                                        || json!({"ordinal": attempt.ordinal.to_string()}),
                                    );
                                    row["admitted_record"] = json!(index);
                                    row["request_envelope_id"] =
                                        json!(envelope.meta().sys().id().to_string());
                                }
                                Fact::Held(batch) | Fact::Published(batch) => {
                                    let row = attempts.entry(batch.attempt.ordinal).or_insert_with(
                                        || json!({"ordinal": batch.attempt.ordinal.to_string()}),
                                    );
                                    row["produced_record"] = json!(index);
                                    row["disposition"] = json!(if matches!(fact, Fact::Held(_)) {
                                        "manager"
                                    } else {
                                        "graph"
                                    });
                                    row["emissions"] = json!(
                                        batch
                                            .emissions
                                            .iter()
                                            .map(ToString::to_string)
                                            .collect::<Vec<_>>()
                                    );
                                    // Only the tag is needed here. DATA remains in the common,
                                    // annotation-aware envelope inspector, including held frames.
                                    row["terminal"] = tx
                                        .emissions
                                        .last()
                                        .and_then(|emission| {
                                            emission.envelope.data().materialized_value()
                                        })
                                        .and_then(|value| match value {
                                            Value::TaggedUnion(value) => Some(json!(value.case)),
                                            _ => None,
                                        })
                                        .unwrap_or(JsonValue::Null);
                                }
                                Fact::Consumed(batch) => {
                                    if let Some(row) = attempts.get_mut(&batch.attempt.ordinal) {
                                        row["consumed_record"] = json!(index);
                                    }
                                }
                                Fact::Advance { .. } | Fact::Close { .. } => {}
                            }
                        }
                    }
                    Record::Egress(EgressRecord::Call(call)) => {
                        let Some(context) = call
                            .retry_record
                            .as_ref()
                            .and_then(|packet| packet.context)
                            .filter(|context| context.operation == *reference)
                        else {
                            continue;
                        };
                        let row = attempts
                            .entry(context.ordinal)
                            .or_insert_with(|| json!({"ordinal": context.ordinal.to_string()}));
                        row["call"] = json!(call.intent.call_id.to_string());
                        match call.status {
                            CallStatus::Open => row["opened_record"] = json!(index),
                            _ => {
                                row["closed_record"] = json!(index);
                                row["closed_at_ns"] =
                                    json!(call.closed_at.map(|at| at.to_string()));
                            }
                        }
                    }
                    Record::Ingress(_) | Record::Egress(_) => {}
                }
            }
            json!({
                "reference": reference.id().to_string(),
                "owner": owner,
                "instance": instance_ref_json(&operation.instance),
                "entry": operation.entry.name(),
                "attempt": operation.ordinal.to_string(),
                "limit": operation.policy().attempts().to_string(),
                "active": operation.phase != Phase::Closed,
                "phase": format!("{:?}", operation.phase),
                "closed": operation.closed.map(|reason| format!("{reason:?}")),
                "due_at_ns": operation.due_at.map(|at| at.to_string()),
                "attempts": attempts.into_values().collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    json!(operations)
}

pub(super) fn append_future(future: &mut JsonValue, operations: &JsonValue) {
    let (Some(future), Some(operations)) = (future.as_array_mut(), operations.as_array()) else {
        return;
    };
    for operation in operations {
        if operation["active"] == true && operation["due_at_ns"].is_string() {
            future.push(json!({
                "kind": "Retry attempt",
                "owner": operation["owner"],
                "instance": operation["instance"],
                "operation": operation["reference"],
                "due_at_ns": operation["due_at_ns"],
            }));
        }
    }
}

pub(super) fn description(
    tx: &crate::journal::TransactionRecord,
) -> Option<(&'static str, String)> {
    let fact = tx.retry_record.as_ref()?.facts.last()?;
    let (kind, summary, ordinal) = match fact {
        Fact::Opened { .. } => ("retry-opened", "Retry operation opened", None),
        Fact::Admitted { attempt, .. } => (
            "retry-admitted",
            "Retry attempt admitted",
            Some(attempt.ordinal),
        ),
        Fact::Held(batch) => (
            "retry-held",
            "Retry result held by manager",
            Some(batch.attempt.ordinal),
        ),
        Fact::Consumed(batch) => (
            "retry-consumed",
            "Retry result consumed",
            Some(batch.attempt.ordinal),
        ),
        Fact::Advance { from, .. } => ("retry-advance", "Retry scheduled", Some(from.ordinal)),
        Fact::Published(batch) => (
            "retry-published",
            "Retry result published",
            Some(batch.attempt.ordinal),
        ),
        Fact::Close { attempt, reason } => match reason {
            crate::journal::retry_protocol::CloseReason::Final => (
                "retry-published",
                "Retry final result published",
                Some(attempt.ordinal),
            ),
            crate::journal::retry_protocol::CloseReason::Stopped => (
                "retry-stopped",
                "Retry stopped without publication",
                Some(attempt.ordinal),
            ),
        },
    };
    Some((
        kind,
        format!(
            "{summary} · {}{}",
            tx.owner_path,
            ordinal.map_or_else(String::new, |ordinal| format!(" · attempt {ordinal}"))
        ),
    ))
}

pub(super) fn held(tx: &crate::journal::TransactionRecord) -> bool {
    tx.retry_record.as_ref().is_some_and(|packet| {
        packet
            .facts
            .iter()
            .any(|fact| matches!(fact, Fact::Held(_)))
    })
}
