use super::UiSnapshot;
use crate::accounting::{CostEvidence, CostReport};
use crate::compiler::SourceDefinition;
use crate::data_policy::is_redacted_path;
use crate::envelope::{Data, MetaValue, Namespace, Scalar, Value};
use crate::identity::InstanceRef;
use crate::journal::{Journal, MemJournal, OutboundRecord, OutboundStatus, Record, SessionStatus};
use crate::observation::ExchangeCorrelation;
use crate::plan::{NodeExecution, Slot, ValidatedExecutionPlan};
use crate::runtime::vm::{
    CallDirectionInspection, CallReferenceInspection, InstancePhaseInspection,
    SessionDirectionInspection, Vm, VmInspection,
};
use crate::schema_annotation::{AnnotationTable, MeasureUnit, SchemaPath, SchemaPathSegment};
use serde_json::{Map, Value as JsonValue, json};
use std::collections::{BTreeMap, BTreeSet};

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
    /// Typed engine or boundary error rendered without DATA secrets.
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
    ordinal: Option<u64>,
    kind: &'static str,
    detail: String,
    node: Option<String>,
    motion: Option<&'static str>,
    records: Option<(usize, usize)>,
    instance: Option<InstanceRef>,
}

impl ActivityView {
    /// Describes one physical boundary observation which retained no Node
    /// transaction of its own.
    #[must_use]
    pub fn external(ordinal: u64, kind: &'static str, detail: impl Into<String>) -> Self {
        Self {
            ordinal: Some(ordinal),
            kind,
            detail: detail.into(),
            node: None,
            motion: None,
            records: None,
            instance: None,
        }
    }

    /// Describes one authored Lab action, which is explicitly not a Node
    /// transaction even when it admits an envelope onto a graph edge.
    #[must_use]
    pub fn scenario_action(
        action: &'static str,
        journal: &MemJournal,
        records_before: usize,
        ordinal: u64,
    ) -> Self {
        let records = committed_range(journal, records_before);
        Self {
            ordinal: Some(ordinal),
            kind: "scenario-action",
            detail: format!("Scenario {action} · the authored action reached its boundary"),
            node: None,
            motion: None,
            records,
            instance: None,
        }
    }

    /// Describes records retained while advancing one execution boundary.
    ///
    /// The last committed transaction owns the highlight. A pure inbound or
    /// Connector-boundary change remains visible without inventing a Node
    /// firing.
    #[must_use]
    pub fn journal_change(
        journal: &MemJournal,
        records_before: usize,
        ordinal: Option<u64>,
    ) -> Option<Self> {
        let records = committed_range(journal, records_before)?;
        let changed = &journal.records()[records.0..=records.1];
        if let Some(transaction) = changed.iter().rev().find_map(|record| match record {
            Record::Transaction(transaction) => Some(transaction),
            Record::Inbound(_) | Record::Outbound(_) => None,
        }) {
            let owner = transaction.owner_path.to_string();
            let kind = match &transaction.trigger {
                crate::node::Trigger::Port(_) => "transaction",
                crate::node::Trigger::Lifecycle(_) => "lifecycle",
            };
            return Some(Self {
                ordinal,
                kind,
                detail: transaction_summary(transaction),
                node: Some(owner),
                motion: None,
                records: Some(records),
                instance: Some(transaction.instance.clone()),
            });
        }
        if let Some((kind, node, motion, detail)) =
            changed.iter().rev().find_map(|record| match record {
                Record::Outbound(OutboundRecord::Call(call)) => {
                    let motion = match &call.status {
                        OutboundStatus::Open => "outbound",
                        OutboundStatus::Closed(_) | OutboundStatus::Abandoned => "inbound",
                    };
                    let (_, _, detail) = history_record_description(record);
                    Some((
                        "call-boundary",
                        call.intent.connector_path.to_string(),
                        motion,
                        detail,
                    ))
                }
                Record::Outbound(OutboundRecord::Session(session)) => {
                    let motion = match &session.status {
                        SessionStatus::Opening(_) | SessionStatus::WriteOpen(_) => "outbound",
                        SessionStatus::Opened
                        | SessionStatus::WriteSettled(_)
                        | SessionStatus::WriteAbandoned(_) => "inbound",
                    };
                    let (_, _, detail) = history_record_description(record);
                    Some((
                        "session-boundary",
                        session.connector_path.to_string(),
                        motion,
                        detail,
                    ))
                }
                Record::Inbound(_) | Record::Transaction(_) => None,
            })
        {
            return Some(Self {
                ordinal,
                kind,
                detail,
                node: Some(node),
                motion: Some(motion),
                records: Some(records),
                instance: None,
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
            ordinal,
            kind,
            detail,
            node: None,
            motion: None,
            records: Some(records),
            instance: None,
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
    /// Source-faithful physical definitions retained by the compiler. They
    /// enrich inspection only and never become execution authority.
    pub definitions: &'a [SourceDefinition],
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
    /// Append-only presentation branch. Always zero for a real run.
    pub branch: u64,
    /// Atomic execution boundary within that branch.
    pub boundary: u64,
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
        definitions,
        vm,
        journal,
        cost_profile,
        fault,
        activity,
        scenario,
        branch,
        boundary,
    } = input;
    let legal_commands = legal_commands(mode, state, fault, scenario.as_ref());
    let inspection = vm.inspection();
    let active_instances = active_instance_lifecycles(journal);
    let value = json!({
        "revision": revision,
        "mode": mode,
        "timeline": {
            "branch": branch,
            "boundary": boundary,
            "record_through": journal.records().len().checked_sub(1),
        },
        "execution": {
            "state": state.spelling(),
            "logical_time_ns": vm.now().to_string(),
            "awaiting_outbound_calls": vm.awaiting_outbound_calls(),
            "legal_commands": legal_commands,
        },
        "topology": topology(
            plan,
            definitions,
            &inspection,
            journal,
            &active_instances,
            activity,
        ),
        "history": history(plan, vm, journal),
        "instances": instances(plan, &inspection, &active_instances),
        "exchanges": exchanges(&inspection),
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
            let mut commands = Vec::new();
            if fault.is_some_and(|fault| fault.retryable) {
                commands.push("retry-step");
            }
            commands
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
        if matches!(
            state,
            ExecutionState::Paused | ExecutionState::Faulted | ExecutionState::Completed
        ) {
            commands.push("branch-from");
        }
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

fn active_instance_lifecycles(journal: &MemJournal) -> BTreeSet<InstanceRef> {
    let mut active = BTreeSet::new();
    for record in journal.records() {
        let Record::Transaction(transaction) = record else {
            continue;
        };
        match &transaction.trigger {
            crate::node::Trigger::Lifecycle(crate::node::Lifecycle::Start) => {
                active.insert(transaction.instance.clone());
            }
            crate::node::Trigger::Lifecycle(crate::node::Lifecycle::Stop) => {
                active.remove(&transaction.instance);
            }
            crate::node::Trigger::Port(_) | crate::node::Trigger::Lifecycle(_) => {}
        }
    }
    active
}

fn record_activity_route(
    activity: &mut BTreeMap<String, &'static str>,
    edge: &str,
    phase: &'static str,
) {
    activity
        .entry(String::from(edge))
        .and_modify(|existing| {
            if *existing != phase {
                *existing = "transferred";
            }
        })
        .or_insert(phase);
}

fn topology(
    plan: &ValidatedExecutionPlan,
    definitions: &[SourceDefinition],
    inspection: &VmInspection,
    journal: &MemJournal,
    active_instances: &BTreeSet<InstanceRef>,
    activity: Option<&ActivityView>,
) -> JsonValue {
    let mailbox_contents = mailbox_contents(plan, inspection);
    let resolved_ports = plan.resolved_ports();
    let plan = plan.plan();
    let definitions_by_owner = definitions
        .iter()
        .map(|definition| (definition.owner.to_string(), definition))
        .collect::<BTreeMap<_, _>>();
    let mut owner_transactions = plan
        .owners
        .iter()
        .map(|owner| (owner.path.to_string(), Traffic::default()))
        .collect::<BTreeMap<_, _>>();
    let mut activity_routes = BTreeMap::new();
    let instance_badges = inspection
        .instances
        .iter()
        .filter_map(|instance| {
            instance_badge_json(instance, active_instances)
                .map(|badge| (instance.reference.clone(), badge))
        })
        .collect::<BTreeMap<_, _>>();
    let node_slots = plan
        .nodes
        .iter()
        .enumerate()
        .map(|(slot, node)| (node.path.to_string(), slot))
        .collect::<BTreeMap<_, _>>();

    for (record_index, record) in journal.records().iter().enumerate() {
        let belongs_to_activity = activity
            .and_then(|activity| activity.records)
            .is_some_and(|(from, through)| (from..=through).contains(&record_index));
        match record {
            Record::Inbound(inbound) => {
                for route in &inbound.routes {
                    if belongs_to_activity {
                        record_activity_route(&mut activity_routes, &route.edge, "admitted");
                    }
                }
            }
            Record::Transaction(transaction) => {
                let owner = transaction.owner_path.to_string();
                record_traffic(&mut owner_transactions, &owner, record_index);
                let firing_slot = node_slots.get(&owner).copied();
                if belongs_to_activity {
                    for consumed in &transaction.consumed {
                        record_activity_route(&mut activity_routes, &consumed.edge, "consumed");
                    }
                }
                for route in &transaction.routes {
                    if belongs_to_activity {
                        record_activity_route(&mut activity_routes, &route.edge, "emitted");
                    }
                }
                if let Some(slot) = firing_slot {
                    for emission in &transaction.emissions {
                        for edge in &plan.edges {
                            let leaves_graph = edge.to.as_ref().is_none_or(|to| to.node.is_none());
                            if edge.from.node.is_some_and(|source| source.index() == slot)
                                && edge.from.port == emission.port
                                && leaves_graph
                            {
                                let route = plan.edge_text(edge);
                                if belongs_to_activity {
                                    record_activity_route(&mut activity_routes, &route, "emitted");
                                }
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
            let definition = definitions_by_owner
                .get(&node.path.to_string())
                .copied()
                .filter(|definition| definition.kind == node.kind());
            let extensions = plan.owners[node.owner.index()]
                .extensions
                .iter()
                .map(|aspect| aspect.extension().source_name())
                .collect::<Vec<_>>();
            let containing_owner = plan.owners[node.owner.index()]
                .parent
                .map(|owner| format!("owner:{}", owner.index()));
            let ports = resolved_ports
                .node(crate::plan::Slot::new(
                    u32::try_from(index).unwrap_or(u32::MAX),
                ))
                .unwrap_or_default()
                .iter()
                .map(|port| port_json(port.port(), port.direction()))
                .collect::<Vec<_>>();
            json!({
                "slot": index,
                "id": format!("node:{index}"),
                "path": node.path.to_string(),
                "label": node.path.segments().last(),
                "owner": containing_owner,
                "kind": kind,
                "type": ty,
                "extensions": extensions,
                "ports": ports,
                "definition": definition.map(source_definition_json),
                "firing_count": firing.count.to_string(),
                "last_firing_record": firing.last_record,
                "instances": inspection.instances.iter().filter(|instance| {
                    instance.node.as_ref() == Some(&node.path)
                }).filter_map(|instance| {
                    instance_badges.get(&instance.reference).cloned()
                }).collect::<Vec<_>>(),
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
            let definition = definitions_by_owner
                .get(&owner.path.to_string())
                .copied()
                .filter(|definition| definition.kind == crate::node::Kind::Graph);
            let ports = if owner.path.is_root() {
                resolved_ports
                    .boundary()
                    .iter()
                    .map(|port| port_json(port.port(), port.direction()))
                    .collect::<Vec<_>>()
            } else {
                owner
                    .ports
                    .iter()
                    .map(|port| port_json(&port.port, port.direction))
                    .collect::<Vec<_>>()
            };
            json!({
                "id": format!("owner:{index}"),
                "path": owner.path.to_string(),
                "label": if owner.path.is_root() {
                    String::from("App")
                } else {
                    owner
                        .path
                        .segments()
                        .last()
                        .cloned()
                        .unwrap_or_else(|| String::from("Graph"))
                },
                "parent": owner.parent.map(|parent| format!("owner:{}", parent.index())),
                "type": if owner.path.is_root() { "App" } else { "Graph" },
                "extensions": owner.extensions.iter().map(|aspect| aspect.extension().source_name()).collect::<Vec<_>>(),
                "ports": ports,
                "definition": definition.map(source_definition_json),
                "transaction_count": activity.count.to_string(),
                "last_transaction_record": activity.last_record,
                "instances": inspection.instances.iter().filter(|instance| {
                    instance.owner == owner.path
                }).filter_map(|instance| {
                    instance_badges.get(&instance.reference).cloned()
                }).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    let edges = plan
        .edges
        .iter()
        .enumerate()
        .map(|(index, edge)| {
            let route = plan.edge_text(edge);
            let mailbox = mailbox_contents.get(&route).cloned().unwrap_or_default();
            json!({
                "index": index,
                "route": route,
                "scope": plan.owners[edge.scope.index()].path.to_string(),
                "source": endpoint_json(&edge.from, true),
                "target": edge.to.as_ref().map_or_else(
                    || json!({
                        "id": format!("discard:{index}"),
                        "kind": "discard",
                        "label": format!(
                            "Discard · {}.{}",
                            edge.from.node.map_or_else(
                                || String::from("App"),
                                |slot| plan.nodes[slot.index()].path.to_string(),
                            ),
                            edge.from.port,
                        ),
                        "owner": format!("owner:{}", edge.scope.index()),
                        "port": JsonValue::Null,
                    }),
                    |target| endpoint_json(target, false),
                ),
                "queued_count": mailbox.len().to_string(),
                "mailbox": mailbox,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "nodes": nodes,
        "owners": owners,
        "edges": edges,
        "activity": activity.map(|activity| json!({
            "ordinal": activity.ordinal,
            "kind": activity.kind,
            "detail": activity.detail,
            "node": activity.node,
            "motion": activity.motion,
            "records": activity.records.map(|(from, through)| json!({
                "from": from,
                "through": through,
            })),
            "instance": activity.instance.as_ref().map(instance_ref_json),
            "routes": activity_routes.iter().map(|(route, phase)| json!({
                "route": route,
                "phase": phase,
            })).collect::<Vec<_>>(),
        })),
    })
}

fn mailbox_contents(
    plan: &ValidatedExecutionPlan,
    inspection: &VmInspection,
) -> BTreeMap<String, Vec<JsonValue>> {
    let mut contents = BTreeMap::<String, Vec<JsonValue>>::new();
    for mailbox in &inspection.mailboxes {
        for (position, delivery) in mailbox.deliveries.iter().enumerate() {
            let annotations = node_port_annotations(plan, &mailbox.node, &delivery.port);
            contents
                .entry(delivery.edge.clone())
                .or_default()
                .push(json!({
                    "position": position,
                    "node": mailbox.node.to_string(),
                    "port": delivery.port.name(),
                    "claimed": mailbox.claimed && position == 0,
                    "instance": instance_ref_json(&mailbox.instance),
                    "source_instance": instance_ref_json(&delivery.source_instance),
                    "sequence": delivery.seq.to_string(),
                    "envelope_id": delivery.envelope.meta().sys().id().to_string(),
                    "origin_id": delivery.envelope.meta().sys().origin_id().to_string(),
                    "meta": meta_json(delivery.envelope.meta()),
                    "data": data_json(delivery.envelope.data(), annotations),
                }));
        }
    }
    contents
}

fn source_definition_json(definition: &SourceDefinition) -> JsonValue {
    json!({
        "source_index": definition.source.index(),
        "first_line": definition.first_line,
        "last_line": definition.last_line,
        "code": definition.code,
    })
}

fn record_traffic(traffic: &mut BTreeMap<String, Traffic>, edge: &str, record_index: usize) {
    let Some(traffic) = traffic.get_mut(edge) else {
        return;
    };
    traffic.count = traffic.count.saturating_add(1);
    traffic.last_record = Some(record_index);
}

fn port_json(port: &crate::node::Port, direction: crate::node::Direction) -> JsonValue {
    json!({
        "name": port.name(),
        "direction": match direction {
            crate::node::Direction::Input => "input",
            crate::node::Direction::Output => "output",
        },
    })
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
            "owner": "owner:0",
            "port": endpoint.port.name(),
        }),
    }
}

fn history(plan: &ValidatedExecutionPlan, vm: &Vm<'_>, journal: &MemJournal) -> JsonValue {
    let observations = journal.retained_observations();
    debug_assert_eq!(observations.len(), journal.records().len());
    let records = observations
        .iter()
        .zip(journal.records())
        .enumerate()
        .map(|(index, (observation, record))| {
            let mut value = crate::observability::observation_json(observation, None);
            if let JsonValue::Object(fields) = &mut value {
                fields.insert(String::from("record_index"), JsonValue::from(index));
                let (kind, trigger, summary) = history_record_description(record);
                fields.insert(
                    String::from("record_kind"),
                    JsonValue::String(String::from(kind)),
                );
                fields.insert(String::from("summary"), JsonValue::String(summary));
                if let Some(trigger) = trigger {
                    fields.insert(String::from("trigger"), JsonValue::String(trigger));
                }
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
                "meta": meta_json(observed.envelope.meta()),
                "data": data_json(observed.envelope.data(), annotations),
            })
        })
        .collect::<Vec<_>>();
    json!({ "records": records, "deliveries": deliveries })
}

fn history_record_description(record: &Record) -> (&'static str, Option<String>, String) {
    match record {
        Record::Inbound(inbound) => {
            let source = inbound.connector_path.as_ref().map_or_else(
                || String::from("App input"),
                |connector| format!("Inbound · {connector}"),
            );
            ("inbound", None, format!("Input admitted · {source}"))
        }
        Record::Transaction(transaction) => {
            let kind = match &transaction.trigger {
                crate::node::Trigger::Port(_) => "port-transaction",
                crate::node::Trigger::Lifecycle(crate::node::Lifecycle::Start) => "instance-opened",
                crate::node::Trigger::Lifecycle(crate::node::Lifecycle::Stop) => "instance-stopped",
                crate::node::Trigger::Lifecycle(crate::node::Lifecycle::Closing) => {
                    "instance-closing"
                }
                crate::node::Trigger::Lifecycle(crate::node::Lifecycle::Next) => "loop-next",
                crate::node::Trigger::Lifecycle(crate::node::Lifecycle::Reply) => "call-reply",
                crate::node::Trigger::Lifecycle(crate::node::Lifecycle::Exit) => "session-exit",
                crate::node::Trigger::Lifecycle(crate::node::Lifecycle::Timeout) => "timeout",
                crate::node::Trigger::Lifecycle(crate::node::Lifecycle::Evict) => "eviction",
                crate::node::Trigger::Lifecycle(crate::node::Lifecycle::Recover) => "recovery",
                crate::node::Trigger::Lifecycle(crate::node::Lifecycle::Compensate) => {
                    "compensation"
                }
            };
            (
                kind,
                Some(transaction.trigger.to_string()),
                transaction_summary(transaction),
            )
        }
        Record::Outbound(OutboundRecord::Call(call)) => {
            let (kind, verb) = match &call.status {
                OutboundStatus::Open => ("call-opened", "Call opened"),
                OutboundStatus::Closed(_) => ("call-closed", "Call closed"),
                OutboundStatus::Abandoned => ("call-abandoned", "Call abandoned"),
            };
            (
                kind,
                None,
                format!("{verb} · {}", call.intent.connector_path),
            )
        }
        Record::Outbound(OutboundRecord::Session(session)) => {
            let (kind, verb) = match &session.status {
                SessionStatus::Opening(_) => ("session-opening", "Session opening"),
                SessionStatus::Opened => ("session-opened", "Session opened"),
                SessionStatus::WriteOpen(_) => ("session-write-opened", "Session write opened"),
                SessionStatus::WriteSettled(_) => {
                    ("session-write-settled", "Session write settled")
                }
                SessionStatus::WriteAbandoned(_) => {
                    ("session-write-abandoned", "Session write abandoned")
                }
            };
            (kind, None, format!("{verb} · {}", session.connector_path))
        }
    }
}

fn transaction_summary(transaction: &crate::journal::TransactionRecord) -> String {
    let owner = if transaction.owner_path.segments().is_empty() {
        String::from("App")
    } else {
        transaction.owner_path.to_string()
    };
    let generation = transaction.instance.generation();
    match &transaction.trigger {
        crate::node::Trigger::Port(port) => format!("Port transaction · {owner} · {port}"),
        crate::node::Trigger::Lifecycle(lifecycle) => {
            let verb = match lifecycle {
                crate::node::Lifecycle::Start => "Instance opened",
                crate::node::Lifecycle::Stop => "Instance stopped",
                crate::node::Lifecycle::Closing => "Instance closing",
                crate::node::Lifecycle::Next => "Loop continued",
                crate::node::Lifecycle::Reply => "Call reply",
                crate::node::Lifecycle::Exit => "Session exit",
                crate::node::Lifecycle::Timeout => "Timeout",
                crate::node::Lifecycle::Evict => "State evicted",
                crate::node::Lifecycle::Recover => "Recovery",
                crate::node::Lifecycle::Compensate => "Compensation",
            };
            format!("{verb} · {owner} · gen {generation}")
        }
    }
}

fn observation_annotations<'a>(
    plan: &'a ValidatedExecutionPlan,
    observed: &crate::runtime::vm::Observed,
) -> &'a AnnotationTable {
    match &observed.node {
        Some(path) => node_port_annotations(plan, path, &observed.port),
        None => plan
            .resolved_ports()
            .boundary()
            .iter()
            .find(|port| port.port() == &observed.port)
            .map(crate::plan::ResolvedPort::annotations)
            .unwrap_or(&EMPTY_ANNOTATIONS),
    }
}

fn node_port_annotations<'a>(
    plan: &'a ValidatedExecutionPlan,
    path: &crate::node::Path,
    port: &crate::node::Port,
) -> &'a AnnotationTable {
    plan.plan()
        .nodes
        .iter()
        .position(|node| &node.path == path)
        .and_then(|index| {
            plan.resolved_ports()
                .node(Slot::new(u32::try_from(index).unwrap_or(u32::MAX)))
        })
        .and_then(|ports| ports.iter().find(|resolved| resolved.port() == port))
        .map(crate::plan::ResolvedPort::annotations)
        .unwrap_or(&EMPTY_ANNOTATIONS)
}

fn instances(
    plan: &ValidatedExecutionPlan,
    inspection: &VmInspection,
    active_instances: &BTreeSet<InstanceRef>,
) -> JsonValue {
    JsonValue::Array(
        inspection
            .instances
            .iter()
            .filter(|instance| active_instances.contains(&instance.reference))
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
                    "node": instance.node.as_ref().map(|path| path.to_string()),
                    "state": state,
                })
            })
            .collect(),
    )
}

fn exchanges(inspection: &VmInspection) -> JsonValue {
    let calls = inspection
        .calls
        .iter()
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
        .iter()
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

fn instance_badge_json(
    instance: &crate::runtime::vm::InstanceInspection,
    active_instances: &BTreeSet<InstanceRef>,
) -> Option<JsonValue> {
    active_instances
        .contains(&instance.reference)
        .then_some(())?;
    Some(json!({
        "generation": instance.reference.generation(),
        "phase": phase(instance.phase),
    }))
}

fn meta_json(meta: &crate::envelope::Meta) -> JsonValue {
    json!({
        "sys": {
            "id": meta.sys().id().to_string(),
            "parent_id": meta.sys().parent_id().map(|id| id.to_string()),
            "origin_id": meta.sys().origin_id().to_string(),
            "role": meta.sys().role().to_string(),
            "graph_version": meta.sys().graph_version(),
        },
        "app": namespace_json(meta.app()),
        "ext": namespace_json(meta.ext()),
    })
}

fn draft_meta_json(ext: &Namespace) -> JsonValue {
    json!({
        "sys": { "pending": "minted on admission" },
        "app": {},
        "ext": namespace_json(ext),
    })
}

fn namespace_json(namespace: &Namespace) -> JsonValue {
    JsonValue::Object(
        namespace
            .iter()
            .map(|(name, value)| (name.clone(), meta_value_json(value)))
            .collect(),
    )
}

fn meta_value_json(value: &MetaValue) -> JsonValue {
    match value {
        MetaValue::Scalar(value) => scalar_json(value),
        MetaValue::Array(values) => JsonValue::Array(values.iter().map(scalar_json).collect()),
        MetaValue::Map(values) => JsonValue::Object(
            values
                .iter()
                .map(|(name, values)| {
                    (
                        name.clone(),
                        JsonValue::Array(values.iter().map(scalar_json).collect()),
                    )
                })
                .collect(),
        ),
    }
}

fn scalar_json(value: &Scalar) -> JsonValue {
    match value {
        Scalar::Bool(value) => JsonValue::Bool(*value),
        Scalar::Int(value) => json_integer(i128::from(*value), "int"),
        Scalar::Double(value) if value.is_finite() => JsonValue::from(*value),
        Scalar::Double(value) => json!({ "kind": "double", "exact": value.to_string() }),
        Scalar::Text(value) => JsonValue::String(value.clone()),
        Scalar::Bytes(value) => json!({ "kind": "bytes", "hex": hex(value) }),
        Scalar::Decimal { unscaled, scale } => json!({
            "kind": "decimal",
            "unscaled": unscaled.to_string(),
            "scale": scale,
        }),
    }
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

/// Projects the next authored Lab injection for read-only inspection.
///
/// The value still comes from the checked scenario on disk. The UI has no
/// command that can replace it.
#[must_use]
pub fn scenario_injection_json(injection: &crate::lab::ScenarioInjectionDraft) -> JsonValue {
    json!({
        "position": "before-app-input",
        "port": injection.port.name(),
        "type": injection.ty.to_string(),
        "meta": draft_meta_json(&injection.ext),
        "data": data_json(&injection.data, &injection.annotations),
    })
}

fn materialized_value_json(value: &Value, annotations: &AnnotationTable) -> JsonValue {
    json!({
        "kind": crate::value::ValueKind::of(value).to_string(),
        "materialized": true,
        "content": value_json(value, annotations, &SchemaPath::root(), !annotations.is_empty()),
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
    fn topology_exposes_exact_source_for_selectable_nodes_and_graphs() {
        let source = r#"
graph inspected : App {
  input in: text
  output out: text

  graph inner {
    input in: text
    output out: text

    operator relay {
      on in where size(data) > 0 { out = . }
    }
    . -> relay -> .
  }
  . -> inner -> .
}
"#;
        let compilation = crate::compiler::compile(
            &[source],
            &crate::contract::ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("the inspection fixture compiles");
        let journal = MemJournal::new(crate::plan::GraphVersion(1));
        let vm = Vm::boot(&compilation.plan, crate::plan::GraphVersion(1));
        let inspection = vm.inspection();
        let active_instances = active_instance_lifecycles(&journal);

        let value = topology(
            &compilation.plan,
            &compilation.source_definitions,
            &inspection,
            &journal,
            &active_instances,
            None,
        );
        assert_eq!(value["owners"][0]["definition"]["first_line"], 2);
        assert!(
            value["owners"][0]["definition"]["code"]
                .as_str()
                .is_some_and(|code| code.starts_with("graph inspected : App"))
        );
        assert_eq!(value["owners"][1]["label"], "inner");
        assert_eq!(value["owners"][1]["parent"], "owner:0");
        for owner in [&value["owners"][0]["ports"], &value["owners"][1]["ports"]] {
            let ports = owner.as_array().expect("Graph ports are explicit");
            assert!(
                ports
                    .iter()
                    .any(|port| { port["name"] == "in" && port["direction"] == "input" })
            );
            assert!(
                ports
                    .iter()
                    .any(|port| { port["name"] == "out" && port["direction"] == "output" })
            );
        }
        assert_eq!(value["nodes"][0]["path"], "inner.relay");
        assert_eq!(value["nodes"][0]["label"], "relay");
        assert_eq!(value["nodes"][0]["owner"], "owner:1");
        let ports = value["nodes"][0]["ports"]
            .as_array()
            .expect("the resolved port row is exposed");
        assert!(
            ports
                .iter()
                .any(|port| { port["name"] == "in" && port["direction"] == "input" })
        );
        assert!(
            ports
                .iter()
                .any(|port| { port["name"] == "out" && port["direction"] == "output" })
        );
        assert!(
            value["nodes"][0]["definition"]["code"]
                .as_str()
                .is_some_and(|code| code.contains("where size(data) > 0"))
        );
    }

    #[test]
    fn topology_projects_scope_wiring_without_synthetic_handler_edges() {
        let source = r#"
          graph app : App {
            input in: text
            output out: text
            graph inner {
              input in: text
              output out: text
              operator relay {
                input in: text
                output out: text
                on in { out = data }
              }
              in -> relay -> out
            }
            in -> inner -> out
          }"#;
        let compilation = crate::compiler::compile(
            &[source],
            &crate::contract::ContractCatalog::try_m0().unwrap(),
        )
        .unwrap();
        let journal = MemJournal::new(crate::plan::GraphVersion(1));
        let vm = Vm::boot(&compilation.plan, crate::plan::GraphVersion(1));
        let value = topology(
            &compilation.plan,
            &compilation.source_definitions,
            &vm.inspection(),
            &journal,
            &active_instance_lifecycles(&journal),
            None,
        );
        assert!(value.get("handler_relations").is_none());
        assert!(value.get("control_nodes").is_none());
        let edges = value["edges"].as_array().unwrap();
        assert_eq!(edges.len(), compilation.plan.edges.len());
        assert_eq!(edges.len(), 2);
        assert!(edges.iter().any(|edge| edge["target"]["kind"] == "output"));
        assert_eq!(value["nodes"][0]["path"], "inner.relay");
    }

    #[test]
    fn materialized_data_exposes_unclassified_scalars_and_redacts_before_projection() {
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
        assert_eq!(rendered["kind"], "struct");
        assert_eq!(rendered["materialized"], true);
        assert_eq!(rendered["content"]["public"], JsonValue::from(7));
        assert_eq!(rendered["content"]["private"], json!({"redacted": true}));
        assert!(!rendered.to_string().contains("secret"));

        let scalar_value = Value::Text(String::from("visible"));
        let scalar = materialized_value_json(&scalar_value, &EMPTY_ANNOTATIONS);
        assert_eq!(scalar["kind"], "text");
        assert_eq!(scalar["materialized"], true);
        assert_eq!(scalar["content"], "visible");

        let classified = AnnotationTable::new(vec![(
            SchemaPath::root(),
            AnnotationSet::new(vec![DataAnnotation::Sensitive(
                SensitivityLevel::Confidential,
            )])
            .expect("one root classification"),
        )])
        .expect("one canonical root");
        let scalar = materialized_value_json(&scalar_value, &classified);
        assert_eq!(scalar["content"], json!({"redacted": true}));
        assert!(!scalar.to_string().contains("visible"));
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

    #[test]
    fn a_pending_injection_is_a_read_only_projection_of_the_authored_scenario() {
        let ty = crate::datatype::DataType::Struct(
            crate::datatype::StructType::new(vec![
                crate::datatype::TypeField::new("count", crate::datatype::DataType::INT),
                crate::datatype::TypeField::new("name", crate::datatype::DataType::Text),
            ])
            .expect("two fields"),
        );
        let draft = crate::lab::ScenarioInjectionDraft {
            port: crate::node::Port::new("in").expect("conventional port"),
            ty,
            annotations: AnnotationTable::empty(),
            data: Data::materialized(Value::Struct(vec![
                (String::from("count"), Value::Int(3)),
                (String::from("name"), Value::Text(String::from("alice"))),
            ])),
            ext: Namespace::new(),
            edited: false,
        };
        let rendered = scenario_injection_json(&draft);
        assert_eq!(rendered["position"], "before-app-input");
        assert_eq!(rendered["meta"]["sys"]["pending"], "minted on admission");
        assert_eq!(rendered["type"], "struct");
        assert_eq!(rendered["data"]["kind"], "struct");
        assert_eq!(rendered["data"]["content"]["count"], 3);
        assert_eq!(rendered["data"]["content"]["name"], "alice");
        assert!(rendered.get("editable").is_none());
    }

    #[test]
    fn a_classified_pending_scenario_value_is_redacted() {
        let annotations = AnnotationTable::new(vec![(
            SchemaPath::root(),
            AnnotationSet::new(vec![DataAnnotation::Sensitive(
                SensitivityLevel::Confidential,
            )])
            .expect("one classification"),
        )])
        .expect("one canonical path");
        let draft = crate::lab::ScenarioInjectionDraft {
            port: crate::node::Port::new("in").expect("conventional port"),
            ty: crate::datatype::DataType::Struct(
                crate::datatype::StructType::new(vec![crate::datatype::TypeField::new(
                    "name",
                    crate::datatype::DataType::Text,
                )])
                .expect("one field"),
            ),
            annotations,
            data: Data::materialized(Value::Struct(vec![(
                String::from("name"),
                Value::Text(String::from("alice")),
            )])),
            ext: Namespace::new(),
            edited: false,
        };
        let rendered = scenario_injection_json(&draft);
        assert!(!rendered.to_string().contains("alice"));
        assert!(rendered.to_string().contains("redacted"));
        assert!(rendered.get("editable").is_none());
    }
}
