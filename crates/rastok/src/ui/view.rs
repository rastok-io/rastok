use super::UiSnapshot;
use crate::accounting::{CostEvidence, CostReport};
use crate::compiler::SourceDefinition;
use crate::data_policy::is_redacted_path;
use crate::datatype::{DataType, IntegerType, TemporalType};
use crate::envelope::{Data, MetaValue, Namespace, Scalar, Value};
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
        "topology": topology(plan, definitions, journal, activity),
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
            let mut commands = Vec::new();
            if fault.is_some_and(|fault| fault.retryable) {
                commands.push("retry-step");
            }
            let editable_draft = scenario
                .and_then(|scenario| scenario.get("current"))
                .and_then(|current| current.get("draft"))
                .and_then(|draft| draft.get("editable"))
                .and_then(JsonValue::as_bool)
                .unwrap_or(false);
            if editable_draft {
                commands.push("edit-next-injection");
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
                    let editable_draft = current
                        .and_then(|current| current.get("draft"))
                        .and_then(|draft| draft.get("editable"))
                        .and_then(JsonValue::as_bool)
                        .unwrap_or(false);
                    if editable_draft {
                        commands.push("edit-next-injection");
                    }
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
    definitions: &[SourceDefinition],
    journal: &MemJournal,
    activity: Option<&ActivityView>,
) -> JsonValue {
    let resolved_ports = plan.resolved_ports();
    let plan = plan.plan();
    let definitions_by_owner = definitions
        .iter()
        .map(|definition| (definition.owner.to_string(), definition))
        .collect::<BTreeMap<_, _>>();
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
            let definition = definitions_by_owner
                .get(&node.path.to_string())
                .copied()
                .filter(|definition| definition.kind == node.kind());
            let aspects = plan.owners[node.owner.index()]
                .aspects
                .iter()
                .map(|aspect| aspect.aspect().source_name())
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
                "aspects": aspects,
                "ports": ports,
                "definition": definition.map(source_definition_json),
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
                "aspects": owner.aspects.iter().map(|aspect| aspect.aspect().source_name()).collect::<Vec<_>>(),
                "ports": ports,
                "definition": definition.map(source_definition_json),
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
                        "id": format!("discard:{index}"),
                        "kind": "discard",
                        "label": "Discard",
                        "owner": format!("owner:{}", edge.scope.index()),
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
                "meta": meta_json(observed.envelope.meta()),
                "data": data_json(observed.envelope.data(), annotations),
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

/// Projects one pending Lab injection into the typed, pre-admission editor
/// model. Classified or dynamic trees remain visible under the ordinary
/// redaction rules but are not editable in M1.
#[must_use]
pub fn scenario_injection_json(draft: &crate::lab::ScenarioInjectionDraft) -> JsonValue {
    let value = draft.data.materialized_value();
    let editable = value.is_some()
        && schema_is_editable(&draft.ty)
        && !draft
            .annotations
            .entries()
            .keys()
            .any(|path| is_redacted_path(&draft.annotations, path));
    json!({
        "position": "before-app-input",
        "port": draft.port.name(),
        "edited": draft.edited,
        "editable": editable,
        "type": type_json(&draft.ty),
        "meta": draft_meta_json(&draft.ext),
        "data": if editable {
            value.and_then(|value| editable_value_json(&draft.ty, value).ok())
                .unwrap_or(JsonValue::Null)
        } else {
            data_json(&draft.data, &draft.annotations)
        },
    })
}

/// Decodes the typed editor wire shape against one resolved App input type.
/// The caller still admits the resulting value through the ordinary Lab/VM
/// boundary; this function cannot mint or mutate an envelope.
///
/// # Errors
///
/// Malformed JSON, an unsupported dynamic tree, or any exact conformance
/// violation refuses before the scenario draft changes.
pub fn parse_edited_data(text: &str, ty: &DataType) -> Result<Value, String> {
    if text.len() > super::MAX_EDIT_BODY_BYTES {
        return Err(String::from("the edited DATA tree exceeds 256 KiB"));
    }
    let json = serde_json::from_str::<JsonValue>(text)
        .map_err(|error| format!("the edited DATA tree is not valid JSON: {error}"))?;
    let value = editable_json_value(ty, &json, "DATA")?;
    if let Some(violation) = ty.violation("edited input data", &value) {
        return Err(format!(
            "the edited DATA tree does not conform: {violation}"
        ));
    }
    Ok(value)
}

fn schema_is_editable(ty: &DataType) -> bool {
    match ty {
        DataType::Variant => false,
        DataType::Struct(structure) => structure
            .fields()
            .iter()
            .all(|field| schema_is_editable(field.ty())),
        DataType::Map(map) => schema_is_editable(map.key()) && schema_is_editable(map.value()),
        DataType::Array(array) => schema_is_editable(array.element()),
        DataType::StructuralUnion(union) => union.branches().iter().all(schema_is_editable),
        DataType::TaggedUnion(union) => union.cases().iter().all(|case| {
            case.payload()
                .fields()
                .iter()
                .all(|field| schema_is_editable(field.ty()))
        }),
        _ => true,
    }
}

fn type_json(ty: &DataType) -> JsonValue {
    let label = ty.to_string();
    match ty {
        DataType::Null => json!({ "kind": "null", "label": label }),
        DataType::Bool => json!({ "kind": "bool", "label": label }),
        DataType::Integer(integer) => json!({
            "kind": "integer",
            "label": label,
            "signed": matches!(integer, IntegerType::Signed(_)),
        }),
        DataType::Float(_) => json!({ "kind": "float", "label": label }),
        DataType::Decimal(decimal) => json!({
            "kind": "decimal",
            "label": label,
            "precision": decimal.precision(),
            "scale": decimal.scale(),
        }),
        DataType::Number => json!({ "kind": "number", "label": label }),
        DataType::Text => json!({ "kind": "text", "label": label }),
        DataType::MediaType => json!({ "kind": "media-type", "label": label }),
        DataType::Bytes(binary) => json!({
            "kind": "bytes",
            "label": label,
            "length": match binary {
                crate::datatype::BinaryType::Variable => None,
                crate::datatype::BinaryType::Fixed(length) => Some(length.get()),
            },
        }),
        DataType::Temporal(temporal) => json!({
            "kind": "temporal",
            "label": label,
            "family": format!("{temporal:?}"),
        }),
        DataType::Uuid => json!({ "kind": "uuid", "label": label }),
        DataType::Struct(structure) => json!({
            "kind": "struct",
            "label": label,
            "fields": structure.fields().iter().map(|field| json!({
                "name": field.name(),
                "type": type_json(field.ty()),
            })).collect::<Vec<_>>(),
        }),
        DataType::Map(map) => json!({
            "kind": "map",
            "label": label,
            "key": type_json(map.key()),
            "value": type_json(map.value()),
        }),
        DataType::Array(array) => json!({
            "kind": "array",
            "label": label,
            "element": type_json(array.element()),
            "length": array.length().map(crate::datatype::ArrayLength::get),
        }),
        DataType::Enum(enumeration) => json!({
            "kind": "enum",
            "label": label,
            "symbols": enumeration.symbols(),
        }),
        DataType::StructuralUnion(union) => json!({
            "kind": "structural-union",
            "label": label,
            "branches": union.branches().iter().map(type_json).collect::<Vec<_>>(),
        }),
        DataType::TaggedUnion(union) => json!({
            "kind": "tagged-union",
            "label": label,
            "cases": union.cases().iter().map(|case| json!({
                "name": case.name(),
                "type": type_json(&DataType::Struct(case.payload().clone())),
            })).collect::<Vec<_>>(),
        }),
        DataType::Variant => json!({
            "kind": "variant",
            "label": label,
            "editable": false,
        }),
    }
}

fn editable_value_json(ty: &DataType, value: &Value) -> Result<JsonValue, String> {
    match (ty, value) {
        (DataType::Null, Value::Null) => Ok(JsonValue::Null),
        (DataType::Bool, Value::Bool(value)) => Ok(JsonValue::Bool(*value)),
        (DataType::Integer(IntegerType::Signed(_)), Value::Int(value)) => {
            Ok(JsonValue::String(value.to_string()))
        }
        (DataType::Integer(IntegerType::Unsigned(_)), Value::Uint(value)) => {
            Ok(JsonValue::String(value.to_string()))
        }
        (DataType::Float(_), Value::Double(value)) => Ok(JsonValue::String(value.to_string())),
        (DataType::Decimal(_), Value::Decimal { unscaled, scale })
        | (DataType::Number, Value::Decimal { unscaled, scale }) => Ok(JsonValue::String(format!(
            "{unscaled}e{}",
            scale.saturating_neg()
        ))),
        (DataType::Number, Value::Int(value)) => Ok(JsonValue::String(value.to_string())),
        (DataType::Number, Value::Uint(value)) => Ok(JsonValue::String(value.to_string())),
        (DataType::Text | DataType::MediaType, Value::Text(value))
        | (DataType::Enum(_), Value::Enum(value)) => Ok(JsonValue::String(value.clone())),
        (DataType::Bytes(_), Value::Bytes(value)) => Ok(JsonValue::String(hex(value))),
        (DataType::Temporal(TemporalType::Date(_)), Value::Date(value))
        | (DataType::Temporal(TemporalType::Time(_)), Value::Time(value))
        | (DataType::Temporal(TemporalType::Timestamp { .. }), Value::Timestamp(value))
        | (DataType::Temporal(TemporalType::TimestampNtz(_)), Value::TimestampNtz(value))
        | (DataType::Temporal(TemporalType::Duration(_)), Value::Duration(value)) => {
            Ok(JsonValue::String(value.to_string()))
        }
        (
            DataType::Temporal(TemporalType::Interval(_)),
            Value::Interval {
                months,
                days,
                nanoseconds,
            },
        ) => Ok(json!({
            "months": months.to_string(),
            "days": days.to_string(),
            "nanoseconds": nanoseconds.to_string(),
        })),
        (DataType::Uuid, Value::Uuid(value)) => Ok(JsonValue::String(hex(value))),
        (DataType::Struct(structure), Value::Struct(fields)) => {
            let fields = fields
                .iter()
                .map(|(name, value)| (name.as_str(), value))
                .collect::<BTreeMap<_, _>>();
            let mut json = Map::new();
            for field in structure.fields() {
                let value = fields
                    .get(field.name())
                    .ok_or_else(|| format!("the value has no field `{}`", field.name()))?;
                json.insert(
                    field.name().to_string(),
                    editable_value_json(field.ty(), value)?,
                );
            }
            Ok(JsonValue::Object(json))
        }
        (DataType::Map(map), Value::Map(entries)) if map.key() == &DataType::Text => {
            Ok(JsonValue::Array(
                entries
                    .iter()
                    .map(|(key, value)| {
                        Ok(json!({
                            "key": key,
                            "value": editable_value_json(map.value(), value)?,
                        }))
                    })
                    .collect::<Result<Vec<_>, String>>()?,
            ))
        }
        (DataType::Map(map), Value::KeyedMap(entries)) => Ok(JsonValue::Array(
            entries
                .iter()
                .map(|(key, value)| {
                    Ok(json!({
                        "key": editable_value_json(map.key(), key)?,
                        "value": editable_value_json(map.value(), value)?,
                    }))
                })
                .collect::<Result<Vec<_>, String>>()?,
        )),
        (DataType::Array(array), Value::Array(items)) => Ok(JsonValue::Array(
            items
                .iter()
                .map(|item| editable_value_json(array.element(), item))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        (DataType::StructuralUnion(union), Value::StructuralUnion { branch, value }) => {
            let branch_type = union
                .branches()
                .get(usize::from(*branch))
                .ok_or_else(|| String::from("the structural-union branch is out of range"))?;
            Ok(json!({
                "branch": branch,
                "value": editable_value_json(branch_type, value)?,
            }))
        }
        (DataType::TaggedUnion(union), Value::TaggedUnion(tagged)) => {
            let case = union
                .cases()
                .iter()
                .find(|case| case.name() == tagged.case)
                .ok_or_else(|| format!("unknown tagged case `{}`", tagged.case))?;
            Ok(json!({
                "case": tagged.case,
                "value": editable_value_json(
                    &DataType::Struct(case.payload().clone()),
                    &Value::Struct(tagged.payload.clone()),
                )?,
            }))
        }
        _ => Err(format!("the value does not inhabit `{ty}`")),
    }
}

fn editable_json_value(ty: &DataType, json: &JsonValue, path: &str) -> Result<Value, String> {
    let scalar = |expected: &str| {
        json.as_str()
            .map(String::from)
            .or_else(|| json.as_i64().map(|value| value.to_string()))
            .or_else(|| json.as_u64().map(|value| value.to_string()))
            .or_else(|| json.as_f64().map(|value| value.to_string()))
            .ok_or_else(|| format!("{path} must be edited as {expected}"))
    };
    match ty {
        DataType::Null if json.is_null() => Ok(Value::Null),
        DataType::Null => Err(format!("{path} must be null")),
        DataType::Bool => json
            .as_bool()
            .map(Value::Bool)
            .ok_or_else(|| format!("{path} must be a boolean")),
        DataType::Integer(IntegerType::Signed(_)) => scalar("a signed integer")?
            .parse::<i64>()
            .map(Value::Int)
            .map_err(|_| format!("{path} is not a signed 64-bit integer")),
        DataType::Integer(IntegerType::Unsigned(_)) => scalar("an unsigned integer")?
            .parse::<u64>()
            .map(Value::Uint)
            .map_err(|_| format!("{path} is not an unsigned 64-bit integer")),
        DataType::Float(_) => scalar("a floating-point number")?
            .parse::<f64>()
            .map(Value::Double)
            .map_err(|_| format!("{path} is not a floating-point number")),
        DataType::Decimal(_) | DataType::Number => {
            let text = scalar("an exact number")?;
            let value = match rastok_core::json::parse_declared_number(&text)
                .map_err(|error| format!("{path} is not an exact number: {error}"))?
            {
                rastok_core::json::DeclaredNumber::Int(value) => Value::Int(value),
                rastok_core::json::DeclaredNumber::Decimal { unscaled, scale } => {
                    Value::Decimal { unscaled, scale }
                }
            };
            ty.narrow(value, None)
                .map_err(|error| format!("{path} does not fit `{ty}`: {error}"))
        }
        DataType::Text | DataType::MediaType => json
            .as_str()
            .map(|value| Value::Text(value.to_string()))
            .ok_or_else(|| format!("{path} must be text")),
        DataType::Bytes(_) => parse_hex(&scalar("hexadecimal bytes")?)
            .map(Value::Bytes)
            .map_err(|error| format!("{path}: {error}")),
        DataType::Temporal(temporal) => match temporal {
            TemporalType::Date(_) => parse_i64(json, path).map(Value::Date),
            TemporalType::Time(_) => parse_i64(json, path).map(Value::Time),
            TemporalType::Timestamp { .. } => parse_i64(json, path).map(Value::Timestamp),
            TemporalType::TimestampNtz(_) => parse_i64(json, path).map(Value::TimestampNtz),
            TemporalType::Duration(_) => parse_i64(json, path).map(Value::Duration),
            TemporalType::Interval(_) => {
                let object = json
                    .as_object()
                    .ok_or_else(|| format!("{path} must be an interval object"))?;
                Ok(Value::Interval {
                    months: parse_i64(
                        required(object, "months", path)?,
                        &format!("{path}.months"),
                    )?,
                    days: parse_i64(required(object, "days", path)?, &format!("{path}.days"))?,
                    nanoseconds: parse_i64(
                        required(object, "nanoseconds", path)?,
                        &format!("{path}.nanoseconds"),
                    )?,
                })
            }
        },
        DataType::Uuid => {
            let bytes = parse_hex(&scalar("a 16-byte UUID in hexadecimal")?)
                .map_err(|error| format!("{path}: {error}"))?;
            let bytes: [u8; 16] = bytes
                .try_into()
                .map_err(|_| format!("{path} must contain exactly 16 bytes"))?;
            Ok(Value::Uuid(bytes))
        }
        DataType::Struct(structure) => {
            let object = json
                .as_object()
                .ok_or_else(|| format!("{path} must be a struct"))?;
            let expected = structure
                .fields()
                .iter()
                .map(|field| field.name())
                .collect::<BTreeSet<_>>();
            if let Some(extra) = object.keys().find(|name| !expected.contains(name.as_str())) {
                return Err(format!("{path} has undeclared field `{extra}`"));
            }
            structure
                .fields()
                .iter()
                .map(|field| {
                    let child = format!("{path}.{}", field.name());
                    Ok((
                        field.name().to_string(),
                        editable_json_value(
                            field.ty(),
                            required(object, field.name(), path)?,
                            &child,
                        )?,
                    ))
                })
                .collect::<Result<Vec<_>, String>>()
                .map(Value::Struct)
        }
        DataType::Map(map) => {
            let entries = json
                .as_array()
                .ok_or_else(|| format!("{path} must be a list of map entries"))?;
            if map.key() == &DataType::Text {
                let mut values = BTreeMap::new();
                for (index, entry) in entries.iter().enumerate() {
                    let entry = entry
                        .as_object()
                        .ok_or_else(|| format!("{path}[{index}] must be a map entry"))?;
                    let key = required(entry, "key", path)?
                        .as_str()
                        .ok_or_else(|| format!("{path}[{index}].key must be text"))?
                        .to_string();
                    let value = editable_json_value(
                        map.value(),
                        required(entry, "value", path)?,
                        &format!("{path}[{index}].value"),
                    )?;
                    if values.insert(key.clone(), value).is_some() {
                        return Err(format!("{path} contains duplicate key `{key}`"));
                    }
                }
                Ok(Value::Map(values))
            } else {
                entries
                    .iter()
                    .enumerate()
                    .map(|(index, entry)| {
                        let entry = entry
                            .as_object()
                            .ok_or_else(|| format!("{path}[{index}] must be a map entry"))?;
                        Ok((
                            editable_json_value(
                                map.key(),
                                required(entry, "key", path)?,
                                &format!("{path}[{index}].key"),
                            )?,
                            editable_json_value(
                                map.value(),
                                required(entry, "value", path)?,
                                &format!("{path}[{index}].value"),
                            )?,
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>()
                    .map(Value::KeyedMap)
            }
        }
        DataType::Array(array) => json
            .as_array()
            .ok_or_else(|| format!("{path} must be an array"))?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                editable_json_value(array.element(), value, &format!("{path}[{index}]"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        DataType::Enum(enumeration) => {
            let value = json
                .as_str()
                .ok_or_else(|| format!("{path} must be an enum symbol"))?;
            if !enumeration.symbols().iter().any(|symbol| symbol == value) {
                return Err(format!("{path} is not a declared enum symbol"));
            }
            Ok(Value::Enum(value.to_string()))
        }
        DataType::StructuralUnion(union) => {
            let object = json
                .as_object()
                .ok_or_else(|| format!("{path} must select one union branch"))?;
            let branch = required(object, "branch", path)?
                .as_u64()
                .and_then(|branch| u16::try_from(branch).ok())
                .ok_or_else(|| format!("{path}.branch must be an unsigned branch index"))?;
            let branch_type = union
                .branches()
                .get(usize::from(branch))
                .ok_or_else(|| format!("{path}.branch is out of range"))?;
            Ok(Value::StructuralUnion {
                branch,
                value: Box::new(editable_json_value(
                    branch_type,
                    required(object, "value", path)?,
                    &format!("{path}.value"),
                )?),
            })
        }
        DataType::TaggedUnion(union) => {
            let object = json
                .as_object()
                .ok_or_else(|| format!("{path} must select one tagged case"))?;
            let case_name = required(object, "case", path)?
                .as_str()
                .ok_or_else(|| format!("{path}.case must be text"))?;
            let case = union
                .cases()
                .iter()
                .find(|case| case.name() == case_name)
                .ok_or_else(|| format!("{path} selects undeclared case `{case_name}`"))?;
            let payload = editable_json_value(
                &DataType::Struct(case.payload().clone()),
                required(object, "value", path)?,
                &format!("{path}.{case_name}"),
            )?;
            let Value::Struct(payload) = payload else {
                return Err(format!("{path}.{case_name} must be a struct"));
            };
            Ok(Value::TaggedUnion(Box::new(
                crate::value::TaggedUnionValue {
                    case: case_name.to_string(),
                    payload,
                },
            )))
        }
        DataType::Variant => Err(format!(
            "{path} has type `variant`; M1 edits only statically typed DATA trees"
        )),
    }
}

fn required<'a>(
    object: &'a Map<String, JsonValue>,
    field: &str,
    path: &str,
) -> Result<&'a JsonValue, String> {
    object
        .get(field)
        .ok_or_else(|| format!("{path} has no field `{field}`"))
}

fn parse_i64(json: &JsonValue, path: &str) -> Result<i64, String> {
    json.as_str()
        .and_then(|value| value.parse().ok())
        .or_else(|| json.as_i64())
        .ok_or_else(|| format!("{path} must be a signed 64-bit integer"))
}

fn parse_hex(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(String::from(
            "hexadecimal bytes need an even number of hex digits",
        ));
    }
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digits = core::str::from_utf8(pair)
                .map_err(|_| String::from("hexadecimal bytes must be ASCII"))?;
            u8::from_str_radix(digits, 16).map_err(|_| String::from("invalid hexadecimal byte"))
        })
        .collect()
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
    use crate::datatype::{StructType, TypeField};
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

        let value = topology(
            &compilation.plan,
            &compilation.source_definitions,
            &journal,
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
    fn a_pending_injection_exposes_one_typed_edit_tree_before_meta_sys_exists() {
        let ty = DataType::Struct(
            StructType::new(vec![
                TypeField::new("count", DataType::INT),
                TypeField::new("name", DataType::Text),
            ])
            .expect("two fields"),
        );
        let draft = crate::lab::ScenarioInjectionDraft {
            port: crate::node::Port::new("in").expect("conventional port"),
            ty: ty.clone(),
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
        assert_eq!(rendered["type"]["kind"], "struct");
        assert_eq!(rendered["data"]["count"], "3");
        assert_eq!(rendered["data"]["name"], "alice");

        let edited = parse_edited_data(r#"{"count":"9","name":"bob"}"#, &ty)
            .expect("the schema-guided tree decodes exactly");
        assert_eq!(
            edited,
            Value::Struct(vec![
                (String::from("count"), Value::Int(9)),
                (String::from("name"), Value::Text(String::from("bob"))),
            ])
        );
        assert!(parse_edited_data(r#"{"count":"wide","name":"bob"}"#, &ty).is_err());

        let unsigned_number = crate::lab::ScenarioInjectionDraft {
            port: crate::node::Port::new("in").expect("conventional port"),
            ty: DataType::Number,
            annotations: AnnotationTable::empty(),
            data: Data::materialized(Value::Uint(u64::MAX)),
            ext: Namespace::new(),
            edited: false,
        };
        let rendered = scenario_injection_json(&unsigned_number);
        assert_eq!(rendered["data"], u64::MAX.to_string());
        assert!(parse_edited_data(&rendered["data"].to_string(), &DataType::Number).is_ok());
    }

    #[test]
    fn a_classified_pending_tree_is_redacted_and_not_editable() {
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
            ty: DataType::Struct(
                StructType::new(vec![TypeField::new("name", DataType::Text)]).expect("one field"),
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
        assert_eq!(rendered["editable"], false);
        assert!(!rendered.to_string().contains("alice"));
    }
}
