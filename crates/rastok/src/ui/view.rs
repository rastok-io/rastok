use super::UiSnapshot;
use crate::accounting::{CostEvidence, CostReport};
use crate::data_policy::is_redacted_path;
use crate::envelope::{Data, Value};
use crate::identity::InstanceRef;
use crate::journal::{Journal, MemJournal, Record};
use crate::observation::ExchangeCorrelation;
use crate::plan::{NodeExecution, Slot, ValidatedExecutionPlan};
use crate::runtime::vm::{
    CallDirectionInspection, CallReferenceInspection, InstancePhaseInspection,
    SessionDirectionInspection, Vm,
};
use crate::schema_annotation::{AnnotationTable, MeasureUnit, SchemaPath, SchemaPathSegment};
use serde_json::{Map, Value as JsonValue, json};
use std::collections::BTreeMap;

static EMPTY_ANNOTATIONS: AnnotationTable = AnnotationTable::empty();

/// Physical controller state, outside every semantic identity recipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionState {
    /// No semantic transaction may start.
    Paused,
    /// The controller advances ready work normally.
    Running,
    /// Open outbound Calls may settle, but their outcomes are not consumed.
    PauseRequested,
    /// Execution stopped on a transaction or physical boundary fault.
    Faulted,
    /// The selected run or scenario reached its terminal condition.
    Completed,
}

impl ExecutionState {
    /// Stable spelling used by the embedded page protocol.
    pub const fn spelling(self) -> &'static str {
        match self {
            Self::Paused => "paused",
            Self::Running => "running",
            Self::PauseRequested => "pause-requested",
            Self::Faulted => "faulted",
            Self::Completed => "completed",
        }
    }
}

/// Helpful, redaction-safe description of one faulting transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaultView {
    /// Typed engine or boundary error rendered without payload secrets.
    pub message: String,
    /// Concrete next-action guidance.
    pub help: String,
    /// Exact commit status of the failed operation.
    pub commit: String,
    /// Whether the reserved transaction may be attempted again.
    pub retryable: bool,
}

/// Physical description of the most recently advanced UI boundary.
///
/// This value only selects a slice of already-retained causal records for
/// presentation. It is never written to the plan or journal and therefore
/// carries no semantic authority of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityView {
    kind: &'static str,
    detail: String,
    node: Option<String>,
    records: Option<(usize, usize)>,
}

impl ActivityView {
    /// Describes one authored Lab action, which is explicitly not a Node
    /// transaction even when it admits an envelope onto a graph edge.
    #[must_use]
    pub fn scenario_action(
        action: &'static str,
        journal: &MemJournal,
        records_before: usize,
    ) -> Self {
        let records = committed_range(journal, records_before);
        Self {
            kind: "scenario-action",
            detail: format!("scenario action `{action}`; no Node fired"),
            node: None,
            records,
        }
    }

    /// Describes records retained while advancing one execution boundary.
    ///
    /// The last committed transaction owns the highlight. A pure inbound or
    /// Connector-boundary change remains visible without inventing a Node
    /// firing.
    #[must_use]
    pub fn journal_change(journal: &MemJournal, records_before: usize) -> Option<Self> {
        let records = committed_range(journal, records_before)?;
        let changed = &journal.records()[records.0..=records.1];
        if let Some(transaction) = changed.iter().rev().find_map(|record| match record {
            Record::Transaction(transaction) => Some(transaction),
            Record::Inbound(_) | Record::Outbound(_) => None,
        }) {
            let owner = transaction.owner_path.to_string();
            let label = if owner.is_empty() { "app" } else { &owner };
            return Some(Self {
                kind: "transaction",
                detail: format!(
                    "Node `{label}` committed one `{}` transaction",
                    transaction.trigger
                ),
                node: Some(owner),
                records: Some(records),
            });
        }
        let (kind, detail) = if changed
            .iter()
            .any(|record| matches!(record, Record::Inbound(_)))
        {
            (
                "inbound",
                String::from("an inbound envelope was admitted; no Node fired"),
            )
        } else {
            (
                "connector-boundary",
                String::from("a Connector boundary fact was retained; no Node fired"),
            )
        };
        Some(Self {
            kind,
            detail,
            node: None,
            records: Some(records),
        })
    }
}

fn committed_range(journal: &MemJournal, records_before: usize) -> Option<(usize, usize)> {
    let records_after = journal.records().len();
    (records_after > records_before).then_some((records_before, records_after - 1))
}

/// Inputs for one complete disposable UI projection.
#[derive(Debug)]
pub struct SnapshotInput<'a> {
    /// Monotonic physical view revision.
    pub revision: u64,
    /// `run` or `simulate`.
    pub mode: &'static str,
    /// Current physical controller state.
    pub state: ExecutionState,
    /// Validated static topology and resolved annotations.
    pub plan: &'a ValidatedExecutionPlan,
    /// Read-only mutable-machine inspection source.
    pub vm: &'a Vm<'a>,
    /// Complete in-memory causal authority.
    pub journal: &'a MemJournal,
    /// Optional deployment price evidence for the public accounting fold.
    pub cost_profile: Option<&'a crate::accounting::CostProfile>,
    /// Current redaction-safe fault, when execution refused.
    pub fault: Option<&'a FaultView>,
    /// Most recently advanced physical UI boundary.
    pub activity: Option<&'a ActivityView>,
    /// Interactive Lab projection, absent in real-run mode.
    pub scenario: Option<JsonValue>,
}

/// Builds one complete disposable execution projection.
///
/// # Errors
///
/// A JSON serialization failure refuses publication but never changes the VM
/// or journal.
pub fn snapshot(input: SnapshotInput<'_>) -> Result<UiSnapshot, serde_json::Error> {
    let SnapshotInput {
        revision,
        mode,
        state,
        plan,
        vm,
        journal,
        cost_profile,
        fault,
        activity,
        scenario,
    } = input;
    let legal_commands = legal_commands(mode, state, fault, scenario.as_ref());
    let value = json!({
        "revision": revision,
        "mode": mode,
        "execution": {
            "state": state.spelling(),
            "logical_time_ns": vm.now().to_string(),
            "awaiting_outbound_calls": vm.awaiting_outbound_calls(),
            "legal_commands": legal_commands,
        },
        "topology": topology(plan, journal, activity),
        "history": history(plan, vm, journal),
        "instances": instances(plan, vm),
        "exchanges": exchanges(vm),
        "cost_report": cost_report(plan, journal, cost_profile),
        "scenario": scenario,
        "fault": fault.map(|fault| json!({
            "message": fault.message,
            "help": fault.help,
            "retryable": fault.retryable,
            "commit": fault.commit,
        })),
    });
    UiSnapshot::new(revision, &value)
}

fn legal_commands(
    mode: &str,
    state: ExecutionState,
    fault: Option<&FaultView>,
    scenario: Option<&JsonValue>,
) -> Vec<&'static str> {
    let mut commands = match state {
        ExecutionState::Running => vec!["pause"],
        ExecutionState::PauseRequested => vec!["resume"],
        ExecutionState::Faulted => {
            if fault.is_some_and(|fault| fault.retryable) {
                vec!["retry-step"]
            } else {
                Vec::new()
            }
        }
        ExecutionState::Completed => Vec::new(),
        ExecutionState::Paused if mode == "run" => {
            vec!["resume", "step", "run-to-completion"]
        }
        ExecutionState::Paused => {
            let current = scenario.and_then(|scenario| scenario.get("current"));
            let complete = current
                .and_then(|current| current.get("complete"))
                .and_then(JsonValue::as_bool)
                .unwrap_or(false);
            let quiescent = current
                .and_then(|current| current.get("quiescent"))
                .and_then(JsonValue::as_bool)
                .unwrap_or(true);
            let mut commands = Vec::new();
            if !complete {
                commands.extend(["resume", "step", "run-until-assertion", "run-to-completion"]);
                if quiescent {
                    commands.push("next-scenario-action");
                }
            }
            commands
        }
    };
    if mode == "simulate" {
        commands.extend(["reset-scenario", "select-scenario"]);
    }
    commands.push("close");
    commands
}

#[derive(Debug, Clone, Copy, Default)]
struct Traffic {
    count: u64,
    last_record: Option<usize>,
}

fn topology(
    plan: &ValidatedExecutionPlan,
    journal: &MemJournal,
    activity: Option<&ActivityView>,
) -> JsonValue {
    let plan = plan.plan();
    let mut traffic = plan
        .edges
        .iter()
        .map(|edge| (plan.edge_text(edge), Traffic::default()))
        .collect::<BTreeMap<_, _>>();
    let mut owner_transactions = plan
        .owners
        .iter()
        .map(|owner| (owner.path.to_string(), Traffic::default()))
        .collect::<BTreeMap<_, _>>();
    let node_slots = plan
        .nodes
        .iter()
        .enumerate()
        .map(|(slot, node)| (node.path.to_string(), slot))
        .collect::<BTreeMap<_, _>>();

    for (record_index, record) in journal.records().iter().enumerate() {
        match record {
            Record::Inbound(inbound) => {
                for route in &inbound.routes {
                    record_traffic(&mut traffic, &route.edge, record_index);
                }
            }
            Record::Transaction(transaction) => {
                let owner = transaction.owner_path.to_string();
                record_traffic(&mut owner_transactions, &owner, record_index);
                let firing_slot = node_slots.get(&owner).copied();
                for route in &transaction.routes {
                    record_traffic(&mut traffic, &route.edge, record_index);
                }
                if let Some(slot) = firing_slot {
                    for emission in &transaction.emissions {
                        for edge in &plan.edges {
                            let leaves_graph = edge.to.as_ref().is_none_or(|to| to.node.is_none());
                            if edge.from.node.is_some_and(|source| source.index() == slot)
                                && edge.from.port == emission.port
                                && leaves_graph
                            {
                                record_traffic(&mut traffic, &plan.edge_text(edge), record_index);
                            }
                        }
                    }
                }
            }
            Record::Outbound(_) => {}
        }
    }

    let nodes = plan
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let (kind, ty) = match &node.execution {
                NodeExecution::Operator => ("operator", String::from("Operator")),
                NodeExecution::Connector(spec) => ("connector", spec.contract.to_string()),
            };
            let firing = owner_transactions
                .get(&node.path.to_string())
                .copied()
                .unwrap_or_default();
            json!({
                "slot": index,
                "id": format!("node:{index}"),
                "path": node.path.to_string(),
                "kind": kind,
                "type": ty,
                "firing_count": firing.count.to_string(),
                "last_firing_record": firing.last_record,
            })
        })
        .collect::<Vec<_>>();
    let owners = plan
        .owners
        .iter()
        .enumerate()
        .filter(|(_, owner)| owner.kind == crate::plan::OwnerKind::Graph)
        .map(|(index, owner)| {
            let activity = owner_transactions
                .get(&owner.path.to_string())
                .copied()
                .unwrap_or_default();
            json!({
                "id": format!("owner:{index}"),
                "path": owner.path.to_string(),
                "label": if owner.path.is_root() {
                    String::from("App")
                } else {
                    owner.path.to_string()
                },
                "parent": owner.parent.map(|parent| format!("owner:{}", parent.index())),
                "transaction_count": activity.count.to_string(),
                "last_transaction_record": activity.last_record,
            })
        })
        .collect::<Vec<_>>();
    let edges = plan
        .edges
        .iter()
        .enumerate()
        .map(|(index, edge)| {
            let route = plan.edge_text(edge);
            let traffic = traffic.get(&route).copied().unwrap_or_default();
            json!({
                "index": index,
                "route": route,
                "scope": plan.owners[edge.scope.index()].path.to_string(),
                "source": endpoint_json(&edge.from, true),
                "target": edge.to.as_ref().map_or_else(
                    || json!({
                        "id": "discard",
                        "kind": "discard",
                        "label": "Discard",
                        "port": JsonValue::Null,
                    }),
                    |target| endpoint_json(target, false),
                ),
                "envelope_count": traffic.count.to_string(),
                "last_envelope_record": traffic.last_record,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "nodes": nodes,
        "owners": owners,
        "edges": edges,
        "activity": activity.map(|activity| json!({
            "kind": activity.kind,
            "detail": activity.detail,
            "node": activity.node,
            "records": activity.records.map(|(from, through)| json!({
                "from": from,
                "through": through,
            })),
        })),
    })
}

fn record_traffic(traffic: &mut BTreeMap<String, Traffic>, edge: &str, record_index: usize) {
    let Some(traffic) = traffic.get_mut(edge) else {
        return;
    };
    traffic.count = traffic.count.saturating_add(1);
    traffic.last_record = Some(record_index);
}

fn endpoint_json(endpoint: &crate::plan::EndpointRef, source: bool) -> JsonValue {
    match endpoint.node {
        Some(slot) => json!({
            "id": format!("node:{}", slot.index()),
            "kind": "node",
            "label": JsonValue::Null,
            "port": endpoint.port.name(),
        }),
        None => json!({
            "id": format!(
                "boundary:{}:{}",
                if source { "input" } else { "output" },
                endpoint.port.name(),
            ),
            "kind": if source { "input" } else { "output" },
            "label": if source { "App input" } else { "App output" },
            "port": endpoint.port.name(),
        }),
    }
}

fn history(plan: &ValidatedExecutionPlan, vm: &Vm<'_>, journal: &MemJournal) -> JsonValue {
    let records = journal
        .retained_observations()
        .iter()
        .enumerate()
        .map(|(index, observation)| {
            let mut value = crate::observability::observation_json(observation, None);
            if let JsonValue::Object(fields) = &mut value {
                fields.insert(String::from("record_index"), JsonValue::from(index));
            }
            value
        })
        .collect::<Vec<_>>();
    let deliveries = vm
        .observations()
        .iter()
        .enumerate()
        .map(|(index, observed)| {
            let annotations = observation_annotations(plan, observed);
            json!({
                "index": index,
                "node": observed.node.as_ref().map(ToString::to_string),
                "port": observed.port.name(),
                "envelope_id": observed.envelope.meta().sys().id().to_string(),
                "origin_id": observed.envelope.meta().sys().origin_id().to_string(),
                "payload": data_json(observed.envelope.data(), annotations),
            })
        })
        .collect::<Vec<_>>();
    json!({ "records": records, "deliveries": deliveries })
}

fn observation_annotations<'a>(
    plan: &'a ValidatedExecutionPlan,
    observed: &crate::runtime::vm::Observed,
) -> &'a AnnotationTable {
    match &observed.node {
        Some(path) => plan
            .plan()
            .nodes
            .iter()
            .position(|node| &node.path == path)
            .and_then(|index| {
                plan.resolved_ports()
                    .node(Slot::new(u32::try_from(index).unwrap_or(u32::MAX)))
            })
            .and_then(|ports| ports.iter().find(|port| port.port() == &observed.port))
            .map(crate::plan::ResolvedPort::annotations)
            .unwrap_or(&EMPTY_ANNOTATIONS),
        None => plan
            .resolved_ports()
            .boundary()
            .iter()
            .find(|port| port.port() == &observed.port)
            .map(crate::plan::ResolvedPort::annotations)
            .unwrap_or(&EMPTY_ANNOTATIONS),
    }
}

fn instances(plan: &ValidatedExecutionPlan, vm: &Vm<'_>) -> JsonValue {
    JsonValue::Array(
        vm.inspection()
            .instances
            .into_iter()
            .map(|instance| {
                let state = instance.node.as_ref().map_or_else(Map::new, |path| {
                    let annotations = plan
                        .plan()
                        .nodes
                        .iter()
                        .find(|node| &node.path == path)
                        .map(|node| &node.vars);
                    instance
                        .state
                        .iter()
                        .map(|(name, value)| {
                            let table = annotations
                                .and_then(|vars| vars.iter().find(|var| &var.name == name))
                                .map(|var| &var.annotations)
                                .unwrap_or(&EMPTY_ANNOTATIONS);
                            (name.clone(), materialized_value_json(value, table))
                        })
                        .collect()
                });
                json!({
                    "reference": instance_ref_json(&instance.reference),
                    "owner": instance.owner.to_string(),
                    "phase": phase(instance.phase),
                    "node": instance.node.map(|path| path.to_string()),
                    "state": state,
                })
            })
            .collect(),
    )
}

fn exchanges(vm: &Vm<'_>) -> JsonValue {
    let inspection = vm.inspection();
    let calls = inspection
        .calls
        .into_iter()
        .map(|call| {
            let (reference, direction) = match call.reference {
                CallReferenceInspection::Outbound(reference) => (reference.to_string(), "outbound"),
                CallReferenceInspection::Inbound(reference) => (reference.to_string(), "inbound"),
            };
            debug_assert_eq!(
                direction,
                match call.direction {
                    CallDirectionInspection::Outbound => "outbound",
                    CallDirectionInspection::Inbound => "inbound",
                }
            );
            json!({
                "reference": reference,
                "direction": direction,
                "connector": call.connector.to_string(),
                "instance": instance_ref_json(&call.instance),
                "completion_retained": call.completion_retained,
            })
        })
        .collect::<Vec<_>>();
    let sessions = inspection
        .sessions
        .into_iter()
        .map(|session| {
            json!({
                "reference": session.reference.to_string(),
                "direction": match session.direction {
                    SessionDirectionInspection::Outbound => "outbound",
                    SessionDirectionInspection::Inbound => "inbound",
                },
                "connector": session.connector.to_string(),
                "instance": instance_ref_json(&session.instance),
                "opened": session.opened,
                "pending_writes": session.pending_writes,
                "completion_retained": session.completion_retained,
            })
        })
        .collect::<Vec<_>>();
    json!({ "calls": calls, "sessions": sessions })
}

fn cost_report(
    plan: &ValidatedExecutionPlan,
    journal: &MemJournal,
    profile: Option<&crate::accounting::CostProfile>,
) -> JsonValue {
    match CostReport::from_journal(plan, journal, profile) {
        Ok(report) => json!({
            "measures": report.measures().iter().map(|row| json!({
                "scope": scope_json(row.scope()),
                "id": row.id().as_str(),
                "quantity": row.quantity().magnitude().to_string(),
                "unit": measure_unit(row.unit()),
                "subset_of": row.subset_of().map(|id| id.as_str()),
            })).collect::<Vec<_>>(),
            "costs": report.costs().iter().map(|row| json!({
                "scope": scope_json(row.scope()),
                "money": money_json(row.money()),
                "basis": cost_evidence(row.evidence()),
                "calculated_from": row.calculated_from().map(|id| id.as_str()),
            })).collect::<Vec<_>>(),
            "unpriced": report.unpriced().iter().map(|row| json!({
                "scope": scope_json(row.scope()),
                "id": row.id().as_str(),
                "quantity": row.quantity().magnitude().to_string(),
            })).collect::<Vec<_>>(),
        }),
        Err(error) => json!({ "error": error.to_string() }),
    }
}

fn scope_json(scope: &crate::accounting::ReportScope) -> JsonValue {
    json!({
        "origin_id": scope.origin_id().map(|id| id.to_string()),
        "instance": scope.instance().map(instance_ref_json),
        "owner": scope.owner().map(ToString::to_string),
        "node": scope.node().map(ToString::to_string),
        "connector": scope.connector().map(ToString::to_string),
        "port": scope.port().name(),
        "exchange": scope.exchange().map(exchange_json),
        "provider": scope.provider(),
        "model": scope.model(),
    })
}

fn exchange_json(exchange: &ExchangeCorrelation) -> JsonValue {
    match exchange {
        ExchangeCorrelation::OutboundCall(reference) => {
            json!({ "kind": "outbound-call", "reference": reference.to_string() })
        }
        ExchangeCorrelation::InboundCall(reference) => {
            json!({ "kind": "inbound-call", "reference": reference.to_string() })
        }
        ExchangeCorrelation::Session { session, write } => json!({
            "kind": "session",
            "reference": session.to_string(),
            "write": write.map(|reference| reference.to_string()),
        }),
    }
}

fn money_json(money: &crate::accounting::Money) -> JsonValue {
    json!({
        "unscaled": money.amount().unscaled().to_string(),
        "scale": money.amount().scale(),
        "currency": money.currency().as_str(),
    })
}

fn cost_evidence(evidence: &CostEvidence) -> JsonValue {
    match evidence {
        CostEvidence::ProviderReported => json!({ "kind": "provider-reported" }),
        CostEvidence::PricebookCalculated { profile, digest } => json!({
            "kind": "pricebook-calculated",
            "profile": profile.as_str(),
            "digest": hex(digest),
        }),
    }
}

fn measure_unit(unit: MeasureUnit) -> &'static str {
    match unit {
        MeasureUnit::Token => "token",
        MeasureUnit::Byte => "byte",
        MeasureUnit::Nanosecond => "nanosecond",
    }
}

fn phase(phase: InstancePhaseInspection) -> &'static str {
    match phase {
        InstancePhaseInspection::Open => "open",
        InstancePhaseInspection::Draining => "draining",
        InstancePhaseInspection::Settling => "settling",
        InstancePhaseInspection::Stopping => "stopping",
    }
}

fn instance_ref_json(reference: &InstanceRef) -> JsonValue {
    json!({
        "address": hex(reference.address().as_bytes()),
        "generation": reference.generation(),
    })
}

fn data_json(data: &Data, annotations: &AnnotationTable) -> JsonValue {
    let Some(value) = data.materialized_value() else {
        return match data {
            Data::Encoded {
                encoding, bytes, ..
            } => json!({
                "kind": "encoded",
                "encoding": encoding.to_string(),
                "byte_size": bytes.len(),
                "materialized": false,
            }),
            Data::Materialized(_) => unreachable!("materialized data exposes its value"),
        };
    };
    materialized_value_json(value, annotations)
}

fn materialized_value_json(value: &Value, annotations: &AnnotationTable) -> JsonValue {
    if !is_composite(value) {
        return json!({
            "kind": crate::value::ValueKind::of(value).to_string(),
            "materialized": true,
            "content": "hidden for scalar roots",
        });
    }
    json!({
        "kind": crate::value::ValueKind::of(value).to_string(),
        "materialized": true,
        "value": value_json(value, annotations, &SchemaPath::root(), !annotations.is_empty()),
    })
}

fn value_json(
    value: &Value,
    annotations: &AnnotationTable,
    path: &SchemaPath,
    classified: bool,
) -> JsonValue {
    if classified && is_redacted_path(annotations, path) {
        return json!({ "redacted": true });
    }
    match value {
        Value::Bool(value) => JsonValue::Bool(*value),
        Value::Int(value) => json_integer(i128::from(*value), "int"),
        Value::Uint(value) => json_unsigned(u128::from(*value), "uint"),
        Value::Double(value) if value.is_finite() => JsonValue::from(*value),
        Value::Double(value) => json!({ "kind": "double", "exact": value.to_string() }),
        Value::Text(value) | Value::Enum(value) => JsonValue::String(value.clone()),
        Value::Bytes(value) => json!({ "kind": "bytes", "hex": hex(value) }),
        Value::Duration(value) => json!({ "kind": "duration", "nanoseconds": value.to_string() }),
        Value::Timestamp(value) => {
            json!({ "kind": "timestamp", "nanoseconds_since_epoch": value.to_string() })
        }
        Value::Date(value) => json!({ "kind": "date", "days_since_epoch": value.to_string() }),
        Value::Time(value) => {
            json!({ "kind": "time", "nanoseconds_since_midnight": value.to_string() })
        }
        Value::TimestampNtz(value) => {
            json!({ "kind": "timestamp_ntz", "nanoseconds_since_epoch": value.to_string() })
        }
        Value::Interval {
            months,
            days,
            nanoseconds,
        } => json!({
            "kind": "interval",
            "months": months,
            "days": days,
            "nanoseconds": nanoseconds.to_string(),
        }),
        Value::Uuid(value) => json!({ "kind": "uuid", "hex": hex(value) }),
        Value::Null => JsonValue::Null,
        Value::Array(items) => {
            let child = path.child(SchemaPathSegment::ArrayElement);
            JsonValue::Array(
                items
                    .iter()
                    .map(|item| value_json(item, annotations, &child, classified))
                    .collect(),
            )
        }
        Value::Map(entries) => {
            let key_path = path.child(SchemaPathSegment::MapKey);
            let value_path = path.child(SchemaPathSegment::MapValue);
            JsonValue::Array(
                entries
                    .iter()
                    .map(|(key, value)| {
                        json!({
                            "key": if classified && is_redacted_path(annotations, &key_path) {
                                JsonValue::String(String::from("[REDACTED]"))
                            } else {
                                JsonValue::String(key.clone())
                            },
                            "value": value_json(value, annotations, &value_path, classified),
                        })
                    })
                    .collect(),
            )
        }
        Value::KeyedMap(entries) => {
            let key_path = path.child(SchemaPathSegment::MapKey);
            let value_path = path.child(SchemaPathSegment::MapValue);
            JsonValue::Array(
                entries
                    .iter()
                    .map(|(key, value)| {
                        json!({
                            "key": value_json(key, annotations, &key_path, classified),
                            "value": value_json(value, annotations, &value_path, classified),
                        })
                    })
                    .collect(),
            )
        }
        Value::Struct(fields) => JsonValue::Object(
            fields
                .iter()
                .map(|(name, value)| {
                    let child = path.child(SchemaPathSegment::Field(name.clone()));
                    (
                        name.clone(),
                        value_json(value, annotations, &child, classified),
                    )
                })
                .collect(),
        ),
        Value::Decimal { unscaled, scale } => json!({
            "kind": "decimal",
            "unscaled": unscaled.to_string(),
            "scale": scale,
        }),
        Value::StructuralUnion { branch, value } => {
            let child = path.child(SchemaPathSegment::StructuralUnionBranch(*branch));
            json!({
                "kind": "structural-union",
                "branch": branch,
                "value": value_json(value, annotations, &child, classified),
            })
        }
        Value::TaggedUnion(tagged) => {
            let case = path.child(SchemaPathSegment::TaggedUnionCase(tagged.case.clone()));
            let payload = tagged
                .payload
                .iter()
                .map(|(name, value)| {
                    let child = case.child(SchemaPathSegment::Field(name.clone()));
                    (
                        name.clone(),
                        value_json(value, annotations, &child, classified),
                    )
                })
                .collect::<Map<_, _>>();
            json!({
                "kind": "tagged-union",
                "case": tagged.case,
                "payload": payload,
            })
        }
    }
}

fn is_composite(value: &Value) -> bool {
    matches!(
        value,
        Value::Array(_)
            | Value::Map(_)
            | Value::KeyedMap(_)
            | Value::Struct(_)
            | Value::StructuralUnion { .. }
            | Value::TaggedUnion(_)
    )
}

fn json_integer(value: i128, kind: &'static str) -> JsonValue {
    const JS_EXACT_MAX: i128 = 9_007_199_254_740_991;
    if (-JS_EXACT_MAX..=JS_EXACT_MAX).contains(&value) {
        JsonValue::from(i64::try_from(value).unwrap_or_default())
    } else {
        json!({ "kind": kind, "exact": value.to_string() })
    }
}

fn json_unsigned(value: u128, kind: &'static str) -> JsonValue {
    const JS_EXACT_MAX: u128 = 9_007_199_254_740_991;
    if value <= JS_EXACT_MAX {
        JsonValue::from(u64::try_from(value).unwrap_or_default())
    } else {
        json!({ "kind": kind, "exact": value.to_string() })
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut value = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        let _ = write!(value, "{byte:02x}");
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema_annotation::{
        AnnotationSet, DataAnnotation, PersonalDataClass, SensitivityLevel,
    };

    #[test]
    fn structured_rendering_redacts_before_json_and_hides_scalar_roots() {
        let annotations = AnnotationTable::new(vec![(
            SchemaPath::new(vec![SchemaPathSegment::Field(String::from("private"))]),
            AnnotationSet::new(vec![DataAnnotation::Sensitive(
                SensitivityLevel::Restricted,
            )])
            .expect("one classification"),
        )])
        .expect("one canonical path");
        let value = Value::Struct(vec![
            (String::from("public"), Value::Int(7)),
            (String::from("private"), Value::Text(String::from("secret"))),
        ]);
        let rendered = materialized_value_json(&value, &annotations);
        assert_eq!(rendered["value"]["public"], JsonValue::from(7));
        assert_eq!(rendered["value"]["private"], json!({"redacted": true}));
        assert!(!rendered.to_string().contains("secret"));

        let scalar_value = Value::Text(String::from("hidden"));
        let scalar = materialized_value_json(&scalar_value, &EMPTY_ANNOTATIONS);
        assert_eq!(scalar["content"], "hidden for scalar roots");
        assert!(!scalar.to_string().contains("hidden\""));
    }

    #[test]
    fn personal_paths_use_the_same_non_reveal_rule() {
        let annotations = AnnotationTable::new(vec![(
            SchemaPath::new(vec![SchemaPathSegment::Field(String::from("email"))]),
            AnnotationSet::new(vec![DataAnnotation::Personal(PersonalDataClass::new(
                false, false, true,
            ))])
            .expect("one classification"),
        )])
        .expect("one canonical path");
        let value = Value::Struct(vec![(
            String::from("email"),
            Value::Text(String::from("alice@example.test")),
        )]);
        let rendered = materialized_value_json(&value, &annotations).to_string();
        assert!(!rendered.contains("alice@example.test"));
        assert!(rendered.contains("redacted"));
    }
}
