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

mod expectation;
mod explanation;
pub use expectation::expectation_json;

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
            detail: format!("Scenario {action} · the authored action reached its effect"),
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
                String::from("a Connector exchange fact was retained; no Node fired"),
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
    /// Currently only `simulate` is admitted by the CLI.
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
    /// Interactive Lab projection.
    pub scenario: Option<JsonValue>,
    /// Fresh simulation session ordinal, incremented on reset or selection.
    pub session: u64,
    /// Atomic execution boundary within that session.
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
        session,
        boundary,
    } = input;
    let legal_commands = legal_commands(mode, state, fault, scenario.as_ref());
    let inspection = vm.inspection();
    let active_instances = active_instance_lifecycles(journal);
    let value = json!({
        "revision": revision,
        "mode": mode,
        "timeline": {
            "session": session,
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
        "loop_runs": loop_runs(plan, journal.records()),
        "future": explanation::future(&inspection, journal.records()),
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
    if mode == "replay" {
        return vec!["close"];
    }
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
                .map(|port| port_json(port.port(), port.direction(), Some(port.ty())))
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
                "loop": loop_spec_json(&plan.owners[node.owner.index()]),
                "profile": match &node.execution {
                    NodeExecution::Connector(spec) => Some(format!("{:?} · {:?}", spec.exchange, spec.roles)),
                    NodeExecution::Operator => None,
                },
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
                    .map(|port| port_json(port.port(), port.direction(), Some(port.ty())))
                    .collect::<Vec<_>>()
            } else {
                owner
                    .ports
                    .iter()
                    .map(|port| port_json(&port.port, port.direction, port.ty.as_ref()))
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
                "loop": loop_spec_json(owner),
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
                "capacity": inspection.mailbox_capacity,
                "cases": explanation::edge_cases(plan, edge),
                "mailbox": mailbox,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "nodes": nodes,
        "owners": owners,
        "edges": edges,
        "commands": manager_commands(plan),
        "routes": explanation::routes(plan),
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

fn manager_commands(plan: &crate::plan::ExecutionPlan) -> Vec<JsonValue> {
    use crate::plan::{EndpointOwner, LoopCommandKind, RouteDestination, RouteSelection};
    let owner_path = |owner: &EndpointOwner| match owner {
        EndpointOwner::Node(path) => path.to_string(),
        EndpointOwner::Graph(path) => path.to_string(),
    };
    let mut commands = Vec::new();
    for rule in &plan.routes {
        let mut project = |cases: &[String], destinations: &[RouteDestination]| {
            for destination in destinations {
                let RouteDestination::LoopCommand(target) = destination else {
                    continue;
                };
                let command = match target.kind {
                    LoopCommandKind::Next => "Loop.next",
                    LoopCommandKind::Again => "Loop.again",
                    LoopCommandKind::Done => "Loop.done",
                };
                commands.push(json!({
                    "source_owner": owner_path(&rule.source.owner),
                    "source_port": rule.source.port.name(),
                    "owner": owner_path(&target.owner),
                    "command": command,
                    "cases": cases,
                }));
            }
        };
        match &rule.selection {
            RouteSelection::All(destinations) => project(&[], destinations),
            RouteSelection::Cases(arms) => {
                for arm in arms {
                    project(&arm.cases, &arm.destinations);
                }
            }
        }
    }
    commands
}

fn loop_runs(plan: &ValidatedExecutionPlan, records: &[Record]) -> JsonValue {
    use crate::journal::loop_protocol::{CommandValue, Fact, Fold, Phase};
    let fold = match Fold::from_records(records) {
        Ok(fold) => fold,
        Err(error) => return json!({ "error": error.to_string() }),
    };
    let owners = records
        .iter()
        .flat_map(|record| {
            let Record::Transaction(transaction) = record else {
                return Vec::new();
            };
            let Some(packet) = transaction.loop_protocol() else {
                return Vec::new();
            };
            packet
                .facts
                .iter()
                .filter_map(|fact| match fact {
                    Fact::Open { run, fuel, .. } => {
                        Some((*run, (transaction.owner_path.to_string(), *fuel)))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .collect::<BTreeMap<_, _>>();
    let admitted = records
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction) => transaction.loop_protocol(),
            _ => None,
        })
        .flat_map(|packet| &packet.facts)
        .filter_map(|fact| match fact {
            Fact::Admit { run, iteration, .. } => Some((*run, iteration.saturating_add(1))),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    json!(
        fold.runs()
            .map(|(reference, run)| json!({
                "reference": reference.to_string(),
                "owner": owners.get(reference).map(|(owner, _)| owner),
                "instance": instance_ref_json(&run.instance),
                "entry": run.entry.name(),
                "application": admitted.get(reference).copied().unwrap_or(0).to_string(),
                "active": matches!(run.phase, Phase::Active | Phase::AwaitingAdmission),
                "waiting": run.phase == Phase::AwaitingAdmission && run.due_at.is_some(),
                "fuel": owners.get(reference).map(|(_, fuel)| fuel.to_string()),
                "expired_port": plan.owners.iter().find(|owner| owners.get(reference).is_some_and(|(path, _)| owner.path.to_string() == *path))
                    .and_then(|owner| owner.loop_spec()).and_then(|spec| spec.expired.as_ref())
                    .map(|expired| expired.port.name()),
                "remaining": run.remaining.to_string(),
                "phase": format!("{:?}", run.phase),
                "state_envelope_id": run.state.meta().sys().id().to_string(),
                "due_at_ns": run.due_at.map(|instant| instant.to_string()),
                "pending_command": run.pending.as_ref().map(|command| match &command.value {
                    CommandValue::Next(_) => "Loop.next",
                    CommandValue::Again(_) => "Loop.again",
                    CommandValue::Done { .. } => "Loop.done",
                }),
            }))
            .collect::<Vec<_>>()
    )
}

fn mailbox_contents(
    plan: &ValidatedExecutionPlan,
    inspection: &VmInspection,
) -> BTreeMap<String, Vec<JsonValue>> {
    let mut contents = BTreeMap::<String, Vec<JsonValue>>::new();
    for mailbox in &inspection.mailboxes {
        for (position, delivery) in mailbox.deliveries.iter().enumerate() {
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
                    "data": port_data_json(plan, Some(&mailbox.node), &delivery.port, delivery.envelope.data()),
                }));
        }
    }
    contents
}

fn source_definition_json(definition: &SourceDefinition) -> JsonValue {
    fn arms(values: &[crate::compiler::SourceArm]) -> JsonValue {
        json!(
            values
                .iter()
                .map(
                    |arm| json!({ "cases": arm.cases, "predicate": arm.predicate,
            "fallback": arm.fallback, "lines": arm.lines, "children": arms(&arm.children) })
                )
                .collect::<Vec<_>>()
        )
    }
    json!({
        "source_index": definition.source.index(),
        "first_line": definition.first_line,
        "last_line": definition.last_line,
        "code": definition.code,
        "decisions": definition.decisions.iter().map(|(trigger, values)| json!({"trigger": trigger, "arms": arms(values)})).collect::<Vec<_>>(),
    })
}

/// Authored scenario DSL, never a reconstruction from executed values.
/// Until source-level redaction exists, classified programs expose no raw
/// scenario text: literals and mock/expect bodies could contain protected DATA.
#[must_use]
pub fn scenario_definition_json(
    compilation: &crate::compiler::Compilation,
    simulation: &str,
    scenario: &str,
) -> JsonValue {
    let plan = compilation.plan.plan();
    let resolved = compilation.plan.resolved_ports();
    let classified = |table: &AnnotationTable| {
        table
            .entries()
            .iter()
            .any(|(path, _)| is_redacted_path(table, path))
    };
    let has_classified_data = resolved
        .boundary()
        .iter()
        .any(|port| classified(port.annotations()))
        || plan.nodes.iter().enumerate().any(|(index, node)| {
            resolved
                .node(Slot::new(u32::try_from(index).unwrap_or(u32::MAX)))
                .is_some_and(|ports| ports.iter().any(|port| classified(port.annotations())))
                || node.vars.iter().any(|var| classified(&var.annotations))
        })
        || plan.owners.iter().any(|owner| {
            owner.ports.iter().any(|port| classified(&port.annotations))
                || owner
                    .vals
                    .iter()
                    .any(|value| classified(&value.annotations))
        });
    if has_classified_data {
        return json!({ "redacted": true, "reason": "Scenario source is hidden because this program carries classified data; inspect the redacted input and history instead." });
    }
    compilation
        .scenario_definitions
        .iter()
        .find(|definition| definition.simulation == simulation && definition.scenario == scenario)
        .map_or(JsonValue::Null, |definition| {
            json!({
                "source_index": definition.source.index(),
                "first_line": definition.first_line,
                "code": definition.code,
                "action_lines": definition.action_lines,
            })
        })
}

fn record_traffic(traffic: &mut BTreeMap<String, Traffic>, edge: &str, record_index: usize) {
    let Some(traffic) = traffic.get_mut(edge) else {
        return;
    };
    traffic.count = traffic.count.saturating_add(1);
    traffic.last_record = Some(record_index);
}

fn port_json(
    port: &crate::node::Port,
    direction: crate::node::Direction,
    ty: Option<&crate::datatype::DataType>,
) -> JsonValue {
    json!({
        "name": port.name(),
        "type": ty.map(ToString::to_string),
        "direction": match direction {
            crate::node::Direction::Input => "input",
            crate::node::Direction::Output => "output",
        },
    })
}

fn loop_spec_json(owner: &crate::plan::Owner) -> JsonValue {
    owner.loop_spec().map_or(JsonValue::Null, |spec| json!({
        "entry": spec.entry.name(),
        "terminals": spec.terminals.iter().map(|port| port.name()).collect::<Vec<_>>(),
        "fuel": spec.fuel.as_ref().and_then(|constant| match constant.resolved { Value::Int(value) => Some(value.to_string()), _ => None }),
        "expired_port": spec.expired.as_ref().map(|target| target.port.name()),
    }))
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
    let explanation = explanation::history(plan, journal.records());
    debug_assert_eq!(observations.len(), journal.records().len());
    let records = observations
        .iter()
        .zip(journal.records())
        .enumerate()
        .map(|(index, (observation, record))| {
            let mut value = crate::observability::observation_json(observation, None);
            if let JsonValue::Object(fields) = &mut value {
                if let Some(JsonValue::Object(extra)) = explanation.records.get(index) {
                    fields.extend(extra.clone());
                }
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
            json!({
                "index": index,
                "node": observed.node.as_ref().map(ToString::to_string),
                "port": observed.port.name(),
                "envelope_id": observed.envelope.meta().sys().id().to_string(),
                "origin_id": observed.envelope.meta().sys().origin_id().to_string(),
                "meta": meta_json(observed.envelope.meta()),
                "data": port_data_json(plan, observed.node.as_ref(), &observed.port, observed.envelope.data()),
            })
        })
        .collect::<Vec<_>>();
    let sources = journal
        .records()
        .iter()
        .enumerate()
        .filter_map(|(index, record)| {
            let Record::Inbound(root) = record else {
                return None;
            };
            Some(json!({
                "record_index": index,
                "node": root.connector_path.as_ref().map(ToString::to_string),
                "port": root.port.name(),
                "envelope_id": root.envelope.meta().sys().id().to_string(),
                "origin_id": root.envelope.meta().sys().origin_id().to_string(),
                "meta": meta_json(root.envelope.meta()),
                "data": if explanation::classified(plan) {
                    json!({ "redacted": true })
                } else {
                    port_data_json(plan, root.connector_path.as_ref(), &root.port, root.envelope.data())
                },
                "instance_labels": root.routes.iter().map(|route| explanation::instance_label(&route.target, explanation::classified(plan))).collect::<BTreeSet<_>>(),
            }))
        })
        .collect::<Vec<_>>();
    let outputs = vm
        .outputs()
        .iter()
        .map(|output| {
            json!({
                "node": null,
                "port": output.port.name(),
                "envelope_id": output.envelope.meta().sys().id().to_string(),
                "origin_id": output.envelope.meta().sys().origin_id().to_string(),
                "meta": meta_json(output.envelope.meta()),
                "data": port_data_json(plan, None, &output.port, output.envelope.data()),
            })
        })
        .collect::<Vec<_>>();
    // Boundary crossings reference the original emission; they must not be
    // added to the source-observation count as synthetic deliveries.
    json!({ "records": records, "deliveries": deliveries, "sources": sources, "outputs": outputs, "envelopes": explanation.envelopes, "commands": explanation.commands })
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
            if let Some(fact) = transaction
                .loop_protocol()
                .and_then(|packet| packet.facts.first())
            {
                use crate::journal::loop_protocol::Fact;
                let (kind, trigger) = match fact {
                    Fact::Open { .. } => ("loop-open", "l:open"),
                    Fact::Admit { .. } => ("loop-admit", "l:admit"),
                    Fact::Advance { .. } => ("loop-advance", "l:advance"),
                    Fact::Close { .. } => ("loop-close", "l:close"),
                    Fact::Expired { .. } => ("loop-expired", "l:expired"),
                };
                return (kind, Some(trigger.into()), transaction_summary(transaction));
            }
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
    if let Some(fact) = transaction
        .loop_protocol()
        .and_then(|packet| packet.facts.last())
    {
        use crate::journal::loop_protocol::Fact;
        let verb = match fact {
            Fact::Open { .. } => "Loop run opened",
            Fact::Advance { .. } => "Loop recurrence scheduled",
            Fact::Admit { .. } => "Loop application admitted",
            Fact::Close { .. } => "Loop run closed",
            Fact::Expired { .. } => "Loop fuel exhausted",
        };
        return format!("{verb} · {owner} · gen {generation}");
    }
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

fn port_schema<'a>(
    plan: &'a ValidatedExecutionPlan,
    path: Option<&crate::node::Path>,
    port: &crate::node::Port,
) -> Option<&'a crate::plan::ResolvedPort> {
    let ports = match path {
        Some(path) => {
            let index = plan
                .plan()
                .nodes
                .iter()
                .position(|node| &node.path == path)?;
            plan.resolved_ports()
                .node(Slot::new(u32::try_from(index).ok()?))?
        }
        None => plan.resolved_ports().boundary(),
    };
    ports.iter().find(|resolved| resolved.port() == port)
}

fn port_data_json(
    plan: &ValidatedExecutionPlan,
    path: Option<&crate::node::Path>,
    port: &crate::node::Port,
    data: &Data,
) -> JsonValue {
    let schema = port_schema(plan, path, port);
    // A dynamic copy before typed validation has no field-policy proof. In a
    // classified graph, absence of that proof must not expose the raw request.
    if explanation::classified(plan)
        && schema.is_none_or(|schema| matches!(schema.ty(), crate::datatype::DataType::Variant))
    {
        json!({ "redacted": true })
    } else {
        data_json(
            data,
            schema
                .map(crate::plan::ResolvedPort::annotations)
                .unwrap_or(&EMPTY_ANNOTATIONS),
        )
    }
}

fn node_port_annotations<'a>(
    plan: &'a ValidatedExecutionPlan,
    path: &crate::node::Path,
    port: &crate::node::Port,
) -> &'a AnnotationTable {
    port_schema(plan, Some(path), port)
        .map(crate::plan::ResolvedPort::annotations)
        .unwrap_or(&EMPTY_ANNOTATIONS)
}

fn instances(
    plan: &ValidatedExecutionPlan,
    inspection: &VmInspection,
    active_instances: &BTreeSet<InstanceRef>,
) -> JsonValue {
    let classified = explanation::classified(plan);
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
                    "label": explanation::instance_label(&instance.reference, classified),
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
pub fn scenario_injection_json(
    plan: &ValidatedExecutionPlan,
    injection: &crate::lab::ScenarioInjectionDraft,
) -> JsonValue {
    // A generic source pin may precede the first classified schema. As with
    // scenario source text, hide that raw copy rather than imply backward taint.
    let data = if injection.connector.is_some() && explanation::classified(plan) {
        json!({ "redacted": true })
    } else {
        data_json(&injection.data, &injection.annotations)
    };
    json!({
        "position": if injection.connector.is_some() { "before-source-output" } else { "before-app-input" },
        "connector": injection.connector.as_ref().map(ToString::to_string),
        "port": injection.port.name(),
        "type": injection.ty.to_string(),
        "meta": draft_meta_json(&injection.ext),
        "data": data,
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
    #[test]
    fn scenario_source_never_bypasses_classified_data_redaction() {
        for annotation in ["@sensitive(level = restricted)", "@personal"] {
            let source = format!(
                r#"
type Protected = struct {{ name: text {annotation} }}
graph demo : App {{ input in: Protected; output out: Protected; in -> out }}
lab demo {{ simulation test on app {{ scenario classified {{
  app.in = {{ name = "private-scenario-literal" }}
  expect app.out count == 1
}} }} }}
"#
            );
            let compilation = crate::compiler::compile(
                &[&source],
                &crate::contract::ContractCatalog::try_m0().expect("catalogue"),
            )
            .expect("classified scenario checks before deployment");
            let value = super::scenario_definition_json(&compilation, "test", "classified");
            assert_eq!(value["redacted"], true);
            assert!(value.get("code").is_none());
            assert!(!value.to_string().contains("private-scenario-literal"));
        }
    }
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
    in -> relay -> out
  }
  in -> inner -> out
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
    fn loop_commands_and_run_dependencies_do_not_invent_ports_or_expose_state_data() {
        let source = include_str!("../../tests/fixtures/ui-two-retries.rkg");
        let compilation = crate::compiler::compile(
            &[source],
            &crate::contract::ContractCatalog::try_m0().unwrap(),
        )
        .unwrap();
        let simulation = &compilation.labs[0].simulations[0];
        let mut session = crate::lab::ScenarioSession::try_new(
            &compilation.plan,
            crate::plan::GraphVersion(1),
            &simulation.name,
            &simulation.mocks,
            &simulation.scenarios[0],
            None,
        )
        .unwrap();
        session.run_to_completion();
        assert!(session.report().passed());
        let value = topology(
            &compilation.plan,
            &compilation.source_definitions,
            &session.vm().inspection(),
            session.journal(),
            &active_instance_lifecycles(session.journal()),
            None,
        );
        let commands = value["commands"].as_array().unwrap();
        assert!(
            commands
                .iter()
                .any(|command| command["source_owner"] == "decide"
                    && command["source_port"] == "retry"
                    && command["owner"] == ""
                    && command["command"] == "Loop.next")
        );
        assert!(value["nodes"].as_array().unwrap().iter().all(|node| {
            node["ports"].as_array().unwrap().iter().all(|port| {
                !["Loop.next", "Loop.again", "Loop.done"].contains(&port["name"].as_str().unwrap())
            })
        }));
        let records = session.journal().records();
        let runs = loop_runs(&compilation.plan, records);
        let runs = runs.as_array().unwrap();
        assert_eq!(runs.len(), 2, "one run for each admitted conversation");
        assert!(runs.iter().all(|run| run["phase"] == "Closed"
            && run["application"] == "2"
            && run["active"] == false));
        assert!(
            runs.iter().all(|run| run["state_envelope_id"].is_string()
                && run.get("data").is_none()
                && run.get("state").is_none()),
            "Loop.state is an exact dependency, not a second unclassified payload view"
        );
        assert!(
            session
                .journal()
                .records()
                .iter()
                .any(|record| match record {
                    Record::Transaction(transaction) =>
                        transaction_summary(transaction).starts_with("Loop application admitted"),
                    _ => false,
                })
        );
        use crate::journal::loop_protocol::Fact;
        let mut saw_wait = false;
        let mut saw_two_active = false;
        for (index, record) in records.iter().enumerate() {
            let before = loop_runs(&compilation.plan, &records[..index]);
            let after = loop_runs(&compilation.plan, &records[..=index]);
            saw_two_active |= after
                .as_array()
                .unwrap()
                .iter()
                .filter(|run| run["active"] == true)
                .count()
                == 2;
            let Record::Transaction(transaction) = record else {
                continue;
            };
            let Some(packet) = transaction.loop_protocol() else {
                continue;
            };
            for fact in &packet.facts {
                let (reference, trigger) = match fact {
                    Fact::Open { run, .. } => (run, "l:open"),
                    Fact::Admit { run, .. } => (run, "l:admit"),
                    Fact::Advance { run, .. } => (run, "l:advance"),
                    Fact::Close { run, .. } => (run, "l:close"),
                    Fact::Expired { run, .. } => (run, "l:expired"),
                };
                assert_eq!(
                    history_record_description(record).1.as_deref(),
                    Some(trigger)
                );
                let run = after
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|run| run["reference"] == reference.to_string())
                    .unwrap();
                assert_eq!(run["expired_port"], "failed");
                match fact {
                    Fact::Open { .. } => assert_eq!(run["application"], "0"),
                    Fact::Admit { iteration, .. } => {
                        assert_eq!(run["application"], (iteration + 1).to_string())
                    }
                    Fact::Advance { .. } => {
                        let old = before
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|old| old["reference"] == run["reference"])
                            .unwrap();
                        assert_eq!(
                            run["application"], old["application"],
                            "Advance is not Admit"
                        );
                        assert_eq!(run["application"], "1");
                        assert_eq!(run["waiting"], true);
                        saw_wait = true;
                    }
                    Fact::Close { .. } | Fact::Expired { .. } => assert_eq!(run["active"], false),
                }
            }
        }
        assert!(saw_wait && saw_two_active);
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
        let compilation = crate::compiler::compile(
            &["graph app : App { input in: text; output out: text; in -> out }"],
            &crate::contract::ContractCatalog::try_m0().unwrap(),
        )
        .unwrap();
        let ty = crate::datatype::DataType::Struct(
            crate::datatype::StructType::new(vec![
                crate::datatype::TypeField::new("count", crate::datatype::DataType::INT),
                crate::datatype::TypeField::new("name", crate::datatype::DataType::Text),
            ])
            .expect("two fields"),
        );
        let mut draft = crate::lab::ScenarioInjectionDraft {
            connector: None,
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
        let rendered = scenario_injection_json(&compilation.plan, &draft);
        assert_eq!(rendered["position"], "before-app-input");
        assert_eq!(rendered["meta"]["sys"]["pending"], "minted on admission");
        assert_eq!(rendered["type"], "struct");
        assert_eq!(rendered["data"]["kind"], "struct");
        assert_eq!(rendered["data"]["content"]["count"], 3);
        assert_eq!(rendered["data"]["content"]["name"], "alice");
        assert!(rendered.get("editable").is_none());
        draft.connector = Some(crate::node::Path::new(["listener"]).expect("source path"));
        draft.port = crate::node::Port::new("out").expect("source port");
        let rendered = scenario_injection_json(&compilation.plan, &draft);
        assert_eq!(rendered["position"], "before-source-output");
        assert_eq!(rendered["connector"], "listener");
        assert_eq!(rendered["port"], "out");
        assert_eq!(rendered["meta"]["sys"]["pending"], "minted on admission");
    }

    #[test]
    fn a_classified_pending_scenario_value_is_redacted() {
        let compilation = crate::compiler::compile(
            &["graph app : App { input in: text; output out: text; in -> out }"],
            &crate::contract::ContractCatalog::try_m0().unwrap(),
        )
        .unwrap();
        let annotations = AnnotationTable::new(vec![(
            SchemaPath::root(),
            AnnotationSet::new(vec![DataAnnotation::Sensitive(
                SensitivityLevel::Confidential,
            )])
            .expect("one classification"),
        )])
        .expect("one canonical path");
        let draft = crate::lab::ScenarioInjectionDraft {
            connector: None,
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
        let rendered = scenario_injection_json(&compilation.plan, &draft);
        assert!(!rendered.to_string().contains("alice"));
        assert!(rendered.to_string().contains("redacted"));
        assert!(rendered.get("editable").is_none());
    }

    #[test]
    fn a_source_copy_cannot_bypass_classification_further_inside_the_graph() {
        let source = r#"
type Protected = struct { name: text @sensitive(level = restricted) }
graph webhook : App {
  output out: Protected
  connector listener : HttpServer { listen = "127.0.0.1:8080"; codec = text }
  operator classify {
    input in: text
    output out: Protected
    on in { out = { name = data } }
  }
  listener -> classify -> out
  listener.exit -> nothing
}
lab webhook { simulation requests on app { scenario classified {
  listener.out = "private-source-literal"
} } }
"#;
        let compilation = crate::compiler::compile(
            &[source],
            &crate::contract::ContractCatalog::try_m0().unwrap(),
        )
        .unwrap();
        let simulation = &compilation.labs[0].simulations[0];
        let mut session = crate::lab::ScenarioSession::try_new(
            &compilation.plan,
            crate::plan::GraphVersion(1),
            &simulation.name,
            &simulation.mocks,
            &simulation.scenarios[0],
            None,
        )
        .unwrap();
        let pending = session.next_injection_draft().unwrap().unwrap();
        let rendered = scenario_injection_json(&compilation.plan, &pending);
        assert!(!rendered.to_string().contains("private-source-literal"));
        assert_eq!(rendered["data"]["redacted"], true);

        session.next_action().unwrap();
        let rendered = history(&compilation.plan, session.vm(), session.journal());
        assert!(
            !rendered["sources"]
                .to_string()
                .contains("private-source-literal")
        );
        assert_eq!(rendered["sources"][0]["data"]["redacted"], true);
    }
}
