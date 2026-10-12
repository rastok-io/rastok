//! Disposable explanations of a selected journal prefix. No execution command,
//! retained fact or guessed value is introduced by this read model.

use super::*;
use crate::envelope::{Envelope, Id};
use crate::instance;
use crate::journal::{
    ConsumedPosition, DeadlineKind, TransactionRecord, loop_protocol as protocol,
};
use crate::node::{HandlerActivation, Lifecycle, Trigger};
use crate::plan::{
    Dispatch, DispatchStep, Endpoint, EndpointOwner, RouteDestination, RouteSelection,
};
use crate::runtime::{DataSlot, LazyBindings, State};

pub(super) fn future(inspection: &VmInspection, records: &[Record]) -> JsonValue {
    let deadlines: BTreeMap<_, _> = records
        .iter()
        .filter_map(Record::deadlines)
        .flat_map(|schedule| schedule.items())
        .map(|deadline| ((deadline.kind, deadline.reference), deadline.due_at))
        .collect();
    let mut result = Vec::new();
    for mailbox in &inspection.mailboxes {
        for delivery in &mailbox.deliveries {
            let id = delivery.envelope.meta().sys().id();
            result.push(json!({ "kind": "delivery", "owner": mailbox.node.to_string(),
                "port": delivery.port.name(), "envelope_id": id.to_string(), "instance": instance_ref_json(&mailbox.instance) }));
            let reference = crate::journal::ids::delivery_id(&delivery.edge, id);
            for (kind, label) in [
                (DeadlineKind::CallTimeout, "l:timeout"),
                (DeadlineKind::JoinWithin, "join expiry"),
                (DeadlineKind::SessionHandshake, "handshake timeout"),
            ] {
                // A won Call closure keeps its head claimed until consumption;
                // its timeout is already cancelled, not a future obligation.
                let settled = records.iter().any(|record| {
                    matches!(record,
                    Record::Egress(EgressRecord::Call(call)) if call.intent.emitting_id == id
                        && call.intent.edge == delivery.edge && call.status != CallStatus::Open)
                });
                let stale_handshake = kind == DeadlineKind::SessionHandshake
                    && inspection.sessions.iter().any(|session| {
                        session.connector == mailbox.node && session.instance == mailbox.instance
                    });
                if let Some(due) = deadlines.get(&(kind, reference))
                    && !(kind == DeadlineKind::CallTimeout && settled)
                    && !stale_handshake
                {
                    result.push(json!({ "kind": label, "owner": mailbox.node.to_string(), "due_at_ns": due.to_string(), "envelope_id": id.to_string() }));
                }
            }
        }
    }
    for instance in &inspection.instances {
        let scheduled = match instance.phase {
            InstancePhaseInspection::Open => vec![(
                DeadlineKind::InstanceLifetime,
                crate::journal::ids::instance_lifetime_id(&instance.reference),
                "l:closing",
            )],
            InstancePhaseInspection::Draining => vec![
                (
                    DeadlineKind::InstanceDrain,
                    crate::journal::ids::instance_closing_id(
                        &instance.reference,
                        DeadlineKind::InstanceDrain,
                    ),
                    "drain deadline",
                ),
                (
                    DeadlineKind::InstanceSettlement,
                    crate::journal::ids::instance_closing_id(
                        &instance.reference,
                        DeadlineKind::InstanceSettlement,
                    ),
                    "settlement deadline",
                ),
            ],
            InstancePhaseInspection::Settling => vec![(
                DeadlineKind::InstanceSettlement,
                crate::journal::ids::instance_closing_id(
                    &instance.reference,
                    DeadlineKind::InstanceSettlement,
                ),
                "settlement deadline",
            )],
            InstancePhaseInspection::Stopping => Vec::new(),
        };
        for (kind, reference, label) in scheduled {
            if let Some(due) = deadlines.get(&(kind, reference)) {
                result.push(json!({"kind": label, "owner": instance.owner.to_string(), "due_at_ns": due.to_string(), "instance": instance_ref_json(&instance.reference)}));
            }
        }
    }
    if let Ok(fold) = protocol::LoopView::from_records(records) {
        for (reference, run) in fold.runs() {
            if run.phase == protocol::Phase::AwaitingAdmission
                && let Some(due) = run.due_at
            {
                let owner = inspection
                    .instances
                    .iter()
                    .find(|instance| instance.reference == run.instance)
                    .map(|instance| instance.owner.to_string());
                result.push(json!({ "kind": "l:admit", "owner": owner, "due_at_ns": due.to_string(), "run": reference.to_string(), "envelope_id": run.state.meta().sys().id().to_string() }));
            }
        }
    }
    json!(result)
}

pub(super) fn endpoint_owner(owner: &EndpointOwner) -> String {
    match owner {
        EndpointOwner::Node(path) => path.to_string(),
        EndpointOwner::Graph(path) => path.to_string(),
    }
}

fn destination(value: &RouteDestination) -> String {
    match value {
        RouteDestination::Discard => "nothing".into(),
        RouteDestination::Port(endpoint) => format!(
            "{}.{}",
            display_owner(&endpoint_owner(&endpoint.owner)),
            endpoint.port
        ),
        RouteDestination::InstanceStop(target) => format!(
            "⇢ {}.instance.stop",
            display_owner(&endpoint_owner(&target.owner)),
        ),
        RouteDestination::LoopCommand(command) => format!(
            "⇢ {}.Loop.{}",
            display_owner(&endpoint_owner(&command.owner)),
            match command.kind {
                crate::plan::LoopCommandKind::Next => "next",
                crate::plan::LoopCommandKind::Again => "again",
                crate::plan::LoopCommandKind::Done => "done",
            }
        ),
    }
}

fn display_owner(owner: &str) -> &str {
    if owner.is_empty() { "App" } else { owner }
}

pub(super) fn routes(plan: &crate::plan::ExecutionPlan) -> JsonValue {
    let inventory = if plan.boundary_routes.is_empty() {
        &plan.routes
    } else {
        &plan.boundary_routes
    };
    json!(inventory.iter().map(|rule| {
        let arms = match &rule.selection {
            RouteSelection::All(destinations) => vec![json!({ "cases": [], "destinations": destinations.iter().map(destination).collect::<Vec<_>>() })],
            RouteSelection::Cases(arms) => arms.iter().map(|arm| json!({ "cases": arm.cases, "destinations": arm.destinations.iter().map(destination).collect::<Vec<_>>() })).collect(),
        };
        json!({ "owner": endpoint_owner(&rule.source.owner), "port": rule.source.port.name(), "arms": arms })
    }).collect::<Vec<_>>())
}

fn physical_matches(
    plan: &crate::plan::ExecutionPlan,
    logical: &Endpoint,
    physical: &crate::plan::EdgeEnd,
) -> bool {
    logical.port == physical.port
        && match (&logical.owner, physical.node) {
            (EndpointOwner::Node(path), Some(slot)) => plan.nodes[slot.index()].path == *path,
            (EndpointOwner::Graph(path), None) => path.is_root(),
            _ => false,
        }
}

pub(super) fn edge_cases(
    plan: &crate::plan::ExecutionPlan,
    edge: &crate::plan::Edge,
) -> Vec<String> {
    let mut cases = BTreeSet::new();
    for rule in plan
        .routes
        .iter()
        .filter(|rule| physical_matches(plan, &rule.source, &edge.from))
    {
        let selected = |destinations: &[RouteDestination]| {
            destinations
                .iter()
                .any(|destination| match (destination, &edge.to) {
                    (RouteDestination::Discard, None) => true,
                    (RouteDestination::Port(logical), Some(physical)) => {
                        physical_matches(plan, logical, physical)
                    }
                    _ => false,
                })
        };
        match &rule.selection {
            RouteSelection::All(destinations) if selected(destinations) => return Vec::new(),
            RouteSelection::Cases(arms) => {
                for arm in arms {
                    if selected(&arm.destinations) {
                        cases.extend(arm.cases.iter().cloned());
                    }
                }
            }
            _ => {}
        }
    }
    cases.into_iter().collect()
}

/// A display key is decoded from the address's retained frames, never guessed
/// from a convenient DATA field such as `tenant`. Classified keys stay hidden.
pub(super) fn instance_label(instance: &instance::Generation, classified: bool) -> String {
    if classified {
        return "classified instance".into();
    }
    use crate::canonical_cbor::Value as Cbor;
    let Ok((Cbor::Array(address), _)) =
        crate::canonical_cbor::decode(instance.address().as_bytes())
    else {
        return "instance".into();
    };
    let Some(Cbor::Array(frames)) = address.get(1) else {
        return "instance".into();
    };
    let labels = frames
        .iter()
        .filter_map(|frame| {
            let Cbor::Array(frame) = frame else {
                return None;
            };
            let [_, Cbor::Uint(1), Cbor::Array(typed)] = frame.as_slice() else {
                return None;
            };
            let Some(Cbor::Array(value)) = typed.get(1) else {
                return None;
            };
            match value.get(1)? {
                Cbor::Text(value) => Some(value.clone()),
                Cbor::Uint(value) => Some(value.to_string()),
                Cbor::Negative(value) => Some(format!("-{}", u128::from(*value) + 1)),
                Cbor::Bool(value) => Some(value.to_string()),
                _ => Some("compound key".into()),
            }
        })
        .collect::<Vec<_>>();
    if labels.is_empty() {
        "singleton".into()
    } else {
        labels.join(" / ")
    }
}

pub(super) fn classified(plan: &ValidatedExecutionPlan) -> bool {
    let sensitive = |annotations: &AnnotationTable| {
        annotations
            .entries()
            .iter()
            .any(|(path, _)| is_redacted_path(annotations, path))
    };
    plan.resolved_ports()
        .boundary()
        .iter()
        .any(|port| sensitive(port.annotations()))
        || plan.nodes.iter().enumerate().any(|(index, node)| {
            plan.resolved_ports()
                .node(Slot::new(index as u32))
                .is_some_and(|ports| ports.iter().any(|port| sensitive(port.annotations())))
                || node.vars.iter().any(|var| sensitive(&var.annotations))
        })
        || plan.owners.iter().any(|owner| {
            owner.vals.iter().any(|value| sensitive(&value.annotations))
                || owner.ports.iter().any(|port| sensitive(&port.annotations))
        })
}

pub(super) struct HistoryExplanation {
    pub records: Vec<JsonValue>,
    pub envelopes: Vec<JsonValue>,
    pub commands: Vec<JsonValue>,
}

pub(super) fn history(plan: &ValidatedExecutionPlan, records: &[Record]) -> HistoryExplanation {
    let protected = classified(plan);
    let mut envelopes = BTreeMap::<Id, (&Envelope, usize)>::new();
    let mut positions = BTreeMap::<(String, instance::Generation), Vec<Id>>::new();
    let mut state = State::new();
    let mut fold = protocol::LoopView::default();
    let mut context_valid = true;
    let mut output = Vec::new();
    let mut commands = BTreeMap::<String, JsonValue>::new();
    let mut readmissions = BTreeMap::new();
    let parents: BTreeMap<_, _> = records
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(tx) if tx.trigger == Trigger::Lifecycle(Lifecycle::Start) => {
                Some((
                    tx.instance.clone(),
                    (tx.owner_path.clone(), tx.parent_instance.clone()),
                ))
            }
            _ => None,
        })
        .collect();
    for (index, record) in records.iter().enumerate() {
        let mut extra = json!({ "record_type": match record { Record::Ingress(_) => "INGRESS", Record::Transaction(_) => "TRANSACTION", Record::Egress(_) => "EGRESS" } });
        match record {
            Record::Ingress(inbound) => {
                let id = inbound.envelope.meta().sys().id();
                envelopes.entry(id).or_insert((&inbound.envelope, index));
                extra["envelope_ids"] = json!([id.to_string()]);
                for route in &inbound.routes {
                    let Some(edge) = plan
                        .edges
                        .iter()
                        .find(|edge| plan.edge_text(edge) == route.edge)
                    else {
                        continue;
                    };
                    let scope = &plan.owners[edge.scope.index()].path;
                    let mut source = Some(&route.target);
                    while let Some(reference) = source {
                        let Some((owner, parent)) = parents.get(reference) else {
                            break;
                        };
                        if owner == scope {
                            positions
                                .entry((route.edge.clone(), reference.clone()))
                                .or_default()
                                .push(id);
                            break;
                        }
                        source = parent.as_ref();
                    }
                }
            }
            Record::Transaction(tx) => {
                let input_ids: Vec<_> = tx
                    .consumed
                    .iter()
                    .filter_map(|position| consumed_id(&positions, position))
                    .collect();
                let inputs: Vec<_> = input_ids
                    .iter()
                    .filter_map(|id| envelopes.get(id).map(|(envelope, _)| *envelope))
                    .collect();
                let slot = plan
                    .nodes
                    .iter()
                    .position(|node| crate::node::OwnerPath::from(&node.path) == tx.owner_path)
                    .map(|slot| Slot::new(slot as u32));
                if let Some(slot) = slot {
                    extra["decision"] = if context_valid {
                        decision(plan, tx, slot, &inputs, &state, &fold, protected)
                    } else {
                        json!({"unavailable": "The preceding journal context failed validation."})
                    };
                    // Apply after explaining: a match reads the pre-transaction state.
                    context_valid &= state
                        .apply(plan, slot, &tx.instance, &tx.state_delta)
                        .is_ok();
                }
                let mut ids: Vec<_> = input_ids;
                ids.extend(tx.cause);
                for emission in &tx.emissions {
                    envelopes
                        .entry(emission.envelope.meta().sys().id())
                        .or_insert((&emission.envelope, index));
                    ids.push(emission.envelope.meta().sys().id());
                }
                if let Some(packet) = tx.loop_protocol() {
                    for command in &packet.produced {
                        let kind = match &command.value {
                            protocol::CommandValue::Next(_) => "Next",
                            protocol::CommandValue::Again(_) => "Again",
                            protocol::CommandValue::Done { .. } => "Done",
                        };
                        commands.insert(command.reference.id().to_string(), json!({"reference": command.reference.id().to_string(), "kind": kind, "run": command.run.to_string(), "cause": command.value.cause().to_string(), "produced": index}));
                    }
                    for command in &packet.consumed {
                        if let Some(value) = commands.get_mut(&command.id().to_string()) {
                            value["consumed"] = json!(index);
                        }
                    }
                    for fact in &packet.facts {
                        match fact {
                            protocol::Fact::Advance {
                                run,
                                to_iteration,
                                command,
                                ..
                            } => {
                                readmissions
                                    .insert((*run, *to_iteration), command.id().to_string());
                            }
                            protocol::Fact::Admit { run, iteration, .. } => {
                                if let Some(reference) = readmissions.get(&(*run, *iteration))
                                    && let Some(command) = commands.get_mut(reference)
                                {
                                    command["admitted"] = json!(index);
                                }
                            }
                            _ => {}
                        }
                    }
                    for envelope in packet.envelopes() {
                        envelopes
                            .entry(envelope.meta().sys().id())
                            .or_insert((envelope, index));
                        ids.push(envelope.meta().sys().id());
                    }
                    ids.extend(packet.produced.iter().map(|command| command.value.cause()));
                    extra["commands_produced"] = json!(packet.produced.iter().map(|command| json!({"reference": command.reference.id().to_string(), "run": command.run.to_string(), "cause": command.value.cause().to_string()})).collect::<Vec<_>>());
                    extra["commands_consumed"] = json!(
                        packet
                            .consumed
                            .iter()
                            .map(|command| command.id().to_string())
                            .collect::<Vec<_>>()
                    );
                    extra["loop_runs"] = json!(
                        packet
                            .context
                            .iter()
                            .copied()
                            .chain(packet.facts.iter().map(protocol::Fact::run))
                            .map(|run| run.to_string())
                            .collect::<BTreeSet<_>>()
                    );
                }
                context_valid &= fold.apply_transaction(tx).is_ok();
                if let Some(packet) = &tx.retry_record {
                    for envelope in packet.envelopes() {
                        let id = envelope.meta().sys().id();
                        envelopes.entry(id).or_insert((envelope, index));
                        ids.push(id);
                    }
                    extra["retry_operations"] = json!(
                        packet
                            .context
                            .iter()
                            .map(|context| context.operation)
                            .chain(
                                packet
                                    .facts
                                    .iter()
                                    .map(crate::journal::retry_protocol::Fact::operation)
                            )
                            .map(|reference| reference.id().to_string())
                            .collect::<BTreeSet<_>>()
                    );
                }
                ids.sort();
                ids.dedup();
                extra["envelope_ids"] =
                    json!(ids.iter().map(ToString::to_string).collect::<Vec<_>>());
                for route in &tx.routes {
                    positions
                        .entry((route.edge.clone(), tx.instance.clone()))
                        .or_default()
                        .push(route.delivery);
                }
            }
            Record::Egress(EgressRecord::Call(call)) => {
                extra["envelope_ids"] = json!([call.intent.emitting_id.to_string()])
            }
            Record::Egress(_) => {}
        }
        output.push(extra);
    }
    HistoryExplanation { records: output, commands: commands.into_values().collect(), envelopes: envelopes.iter().map(|(id, (envelope, index))| json!({
        "envelope_id": id.to_string(), "origin_id": envelope.meta().sys().origin_id().to_string(), "parent_id": envelope.meta().sys().parent_id().map(|id| id.to_string()), "record_index": index,
    })).collect() }
}

fn consumed_id(
    positions: &BTreeMap<(String, instance::Generation), Vec<Id>>,
    position: &ConsumedPosition,
) -> Option<Id> {
    positions
        .get(&(position.edge.clone(), position.source.clone()))?
        .get(usize::try_from(position.seq.checked_sub(1)?).ok()?)
        .copied()
}

fn decision(
    plan: &ValidatedExecutionPlan,
    tx: &TransactionRecord,
    slot: Slot,
    inputs: &[&Envelope],
    state: &State,
    fold: &protocol::LoopView,
    protected: bool,
) -> JsonValue {
    let input_ports: Vec<_> = tx
        .consumed
        .iter()
        .filter_map(|position| {
            plan.edges
                .iter()
                .find(|edge| plan.edge_text(edge) == position.edge)?
                .to
                .as_ref()
                .map(|endpoint| endpoint.port.clone())
        })
        .collect();
    let handler = plan.handlers.iter().find(|handler| {
        handler.owner == slot
            && match &handler.activation {
                HandlerActivation::Port(port) => {
                    tx.join_inputs.is_empty() && tx.trigger == Trigger::Port(port.clone())
                }
                HandlerActivation::Lifecycle(lifecycle) => {
                    tx.trigger == Trigger::Lifecycle(*lifecycle)
                }
                HandlerActivation::Join { ports, .. } => {
                    !tx.join_inputs.is_empty() && ports == &input_ports
                }
            }
    });
    let Some(handler) = handler else {
        return JsonValue::Null;
    };
    if matches!(handler.dispatch, Dispatch::Linear(_)) {
        return JsonValue::Null;
    }
    if protected {
        return json!({ "unavailable": "Decision details are redacted because this program carries classified data." });
    }
    let Some(input) = inputs
        .iter()
        .find(|input| Some(input.meta().sys().id()) == tx.cause)
        .or_else(|| {
            // A join retains its completing port and all contribution ids,
            // not a single `cause`. The trigger's DATA is that exact input.
            input_ports
                .iter()
                .position(|port| tx.trigger == Trigger::Port(port.clone()))
                .filter(|_| !tx.join_inputs.is_empty())
                .and_then(|index| inputs.get(index))
        })
        .or_else(|| inputs.first().filter(|_| inputs.len() == 1))
    else {
        return json!({"unavailable": "No exact consumed envelope is available for this transaction."});
    };
    if input.data().materialized_value().is_none() {
        return json!({"unavailable": "The recorded input is opaque; inspection does not decode DATA."});
    }
    let Ok(prestate) = state.cell(plan, slot, &tx.instance) else {
        return JsonValue::Null;
    };
    let mut bindings: BTreeMap<_, _> = prestate
        .into_iter()
        .map(|(name, value)| (name, crate::expr::EvalValue::from(value)))
        .collect();
    bindings.insert(
        "meta".into(),
        crate::expr::EvalValue::from(crate::runtime::meta_value(input.meta())),
    );
    if let HandlerActivation::Join { ports, .. } = &handler.activation {
        if inputs.len() != ports.len()
            || inputs
                .iter()
                .map(|input| input.meta().sys().id())
                .collect::<Vec<_>>()
                != tx.join_inputs
        {
            return json!({"unavailable": "The complete named-input environment is not recorded."});
        }
        for (port, input) in ports.iter().zip(inputs) {
            let Some(value) = input.data().materialized_value() else {
                return json!({"unavailable": "A joined input is opaque; inspection does not decode DATA."});
            };
            let data = crate::expr::EvalValue::from(value.clone());
            let meta = crate::expr::EvalValue::from(crate::runtime::meta_value(input.meta()));
            bindings.insert(
                port.name().into(),
                crate::expr::EvalValue::Struct(vec![
                    ("data".into(), data.clone()),
                    ("meta".into(), meta.clone()),
                ]),
            );
            bindings.insert(format!("{}.data", port.name()), data);
            bindings.insert(format!("{}.meta", port.name()), meta);
        }
        let common = |zone: fn(&crate::envelope::Meta) -> &Namespace,
                      fields: &[(String, crate::datatype::DataType)]| {
            let mut values = Namespace::new();
            for (field, _) in fields {
                let mut present = inputs
                    .iter()
                    .filter_map(|input| zone(input.meta()).get(field));
                if let Some(value) = present.next()
                    && present.all(|candidate| candidate == value)
                {
                    values.insert(field.clone(), value.clone());
                }
            }
            values
        };
        let meta = crate::envelope::Meta::new(input.meta().sys().clone())
            .with_app(common(crate::envelope::Meta::app, &plan.app_schema))
            .with_ext(common(crate::envelope::Meta::ext, &plan.ext_schema));
        bindings.insert(
            String::from("meta"),
            crate::expr::EvalValue::from(crate::runtime::meta_value(&meta)),
        );
    }
    if let Some(now) = tx.logical_time {
        bindings.insert("now".into(), crate::expr::EvalValue::Timestamp(now));
    }
    // Only plan-reduced immutable values are available without reconstructing
    // instance bindings. Missing context is an explicit explanation limit.
    let mut owners = Vec::new();
    let mut owner = Some(plan.nodes[slot.index()].owner);
    while let Some(current) = owner {
        owners.push(current);
        owner = plan.owners[current.index()].parent;
    }
    for owner in owners.into_iter().rev() {
        for declaration in &plan.owners[owner.index()].vals {
            if let Some(value) = &declaration.resolved {
                bindings.insert(
                    declaration.name.clone(),
                    crate::expr::EvalValue::from(value.clone()),
                );
            }
        }
    }
    if let Some(run) = tx
        .loop_protocol()
        .and_then(|packet| packet.context.last())
        .and_then(|reference| fold.run(*reference))
        && let Some(value) = run.state.data().materialized_value()
    {
        let data = crate::expr::EvalValue::from(value.clone());
        let meta = crate::expr::EvalValue::from(crate::runtime::meta_value(run.state.meta()));
        let state = crate::expr::EvalValue::Struct(vec![
            ("data".into(), data.clone()),
            ("meta".into(), meta.clone()),
        ]);
        bindings.extend([
            ("Loop.state.data".into(), data),
            ("Loop.state.meta".into(), meta),
            ("Loop.state".into(), state.clone()),
            (
                "Loop".into(),
                crate::expr::EvalValue::Struct(vec![("state".into(), state)]),
            ),
        ]);
    }
    let input_type = match &tx.trigger {
        Trigger::Port(port) => plan.resolved_ports().node_port_type(slot, port),
        Trigger::Lifecycle(_) => None,
    };
    let data = input_type.map_or_else(
        || DataSlot::new(input.data()),
        |ty| DataSlot::typed(input.data(), ty),
    );
    let scope = LazyBindings {
        base: &bindings,
        data: &data,
        assets: &plan.assets,
    };
    if crate::runtime::select_dispatch(&handler.dispatch, &scope).is_err() {
        return json!({"unavailable": "The exact decision environment could not be reconstructed; no branch is guessed."});
    }
    match dispatch_decisions(&handler.dispatch, &mut bindings, &data, &plan.assets) {
        Ok(decisions) => {
            json!({ "trigger": match &handler.activation { HandlerActivation::Join { ports, .. } => format!("join:{}", ports.iter().map(|port| port.name()).collect::<Vec<_>>().join(",")), _ => tx.trigger.to_string() }, "input": input.meta().sys().id().to_string(), "steps": decisions })
        }
        Err(()) => json!({"unavailable": "Decision details could not be reconstructed."}),
    }
}

fn dispatch_decisions(
    dispatch: &Dispatch,
    base: &mut BTreeMap<String, crate::expr::EvalValue>,
    data: &DataSlot<'_>,
    assets: &[crate::plan::AssetBlob],
) -> Result<Vec<JsonValue>, ()> {
    let mut current = dispatch;
    let mut result = Vec::new();
    for _ in 0..=crate::plan::MAX_DISPATCH_DEPTH {
        let prefix = match current {
            Dispatch::Linear(_) => return Ok(result),
            Dispatch::ClosedMatch { prefix, .. } | Dispatch::OpenMatch { prefix, .. } => prefix,
        };
        for statement in prefix {
            if let DispatchStep::Let { name, value, .. } = statement {
                let value = crate::runtime::eval_lazy(value, base, data, assets).map_err(|_| ())?;
                base.insert(name.clone(), value);
            } else {
                return Err(());
            }
        }
        let evaluate = |tree| {
            crate::runtime::eval_lazy(tree, base, data, assets)
                .and_then(|value| crate::runtime::resolve_payload(value, data))
                .map_err(|_| ())
        };
        match current {
            Dispatch::ClosedMatch {
                scrutinee, arms, ..
            } => {
                let case = match evaluate(scrutinee)? {
                    Value::TaggedUnion(value) => value.case,
                    Value::Enum(case) => case,
                    Value::Bool(value) => value.to_string(),
                    _ => return Err(()),
                };
                let arm = arms
                    .iter()
                    .find(|arm| arm.cases.contains(&case))
                    .ok_or(())?;
                result.push(json!({"kind": "closed", "case": case, "cases": arm.cases}));
                current = &arm.dispatch;
            }
            Dispatch::OpenMatch { arms, fallback, .. } => {
                let mut selected = fallback.as_ref();
                let mut predicates = Vec::new();
                let mut selected_index = None;
                for (index, arm) in arms.iter().enumerate() {
                    let Value::Bool(value) = evaluate(&arm.predicate)? else {
                        return Err(());
                    };
                    predicates.push(json!({"index": index, "value": value}));
                    if value {
                        selected = &arm.dispatch;
                        selected_index = Some(index);
                        break;
                    }
                }
                result.push(
                    json!({"kind": "open", "predicates": predicates, "selected": selected_index}),
                );
                current = selected;
            }
            Dispatch::Linear(_) => return Ok(result),
        }
    }
    Err(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decisions_use_consumed_envelopes_and_the_selected_prefix() {
        let compilation = crate::compiler::compile(
            &[include_str!("../../../tests/fixtures/ui-two-retries.rkg")],
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
        let records = session.journal().records();
        let explained = history(&compilation.plan, records);
        let decisions: Vec<_> = explained
            .records
            .iter()
            .filter_map(|record| record.get("decision"))
            .filter(|decision| !decision.is_null())
            .collect();
        assert_eq!(decisions.len(), 4);
        assert_eq!(
            decisions
                .iter()
                .map(|decision| decision["steps"][0]["case"]
                    .as_str()
                    .unwrap_or_else(|| panic!("{decision}")))
                .collect::<Vec<_>>(),
            ["TimeoutExpired", "TimeoutExpired", "Exited", "Exited"]
        );
        for end in 0..=records.len() {
            let prefix = history(&compilation.plan, &records[..end]);
            assert_eq!(prefix.records, explained.records[..end]);
            assert!(
                prefix
                    .envelopes
                    .iter()
                    .all(|envelope| envelope["record_index"].as_u64().unwrap() < end as u64)
            );
            for command in &prefix.commands {
                for field in ["produced", "consumed", "admitted"] {
                    if let Some(index) = command[field].as_u64() {
                        assert!(
                            index < end as u64,
                            "a causal link must not read past its prefix"
                        );
                    }
                }
            }
        }
        assert_eq!(explained.commands.len(), 4);
        assert_eq!(
            explained
                .commands
                .iter()
                .filter(|command| command.get("admitted").is_some())
                .count(),
            2
        );
        for command in &explained.commands {
            assert!(command["produced"].as_u64().unwrap() < command["consumed"].as_u64().unwrap());
            if command["kind"] == "Next" {
                assert!(
                    command["consumed"].as_u64().unwrap() < command["admitted"].as_u64().unwrap()
                );
            } else {
                assert!(command.get("admitted").is_none());
            }
        }
        let inspection = session.vm().inspection();
        let labels: BTreeSet<_> = inspection
            .instances
            .iter()
            .map(|instance| instance_label(&instance.reference, false))
            .collect();
        assert_eq!(
            labels,
            BTreeSet::from([String::from("alice"), String::from("bob")])
        );
        assert!(
            inspection
                .instances
                .iter()
                .all(|instance| instance_label(&instance.reference, true) == "classified instance")
        );
    }

    #[test]
    fn open_match_explains_only_evaluated_predicates_using_prestate() {
        let compilation = crate::compiler::compile(
            &[include_str!("../../../tests/fixtures/ui-explanation.rkg")],
            &crate::contract::ContractCatalog::try_m0().unwrap(),
        )
        .unwrap();
        let simulation = &compilation.labs[0].simulations[0];
        let mut session = crate::lab::ScenarioSession::try_new(
            &compilation.plan,
            crate::plan::GraphVersion(1),
            &simulation.name,
            &simulation.mocks,
            &simulation.scenarios[2],
            None,
        )
        .unwrap();
        session.run_to_completion();
        assert!(session.report().passed(), "{:?}", session.failures());
        let explained = history(&compilation.plan, session.journal().records());
        let decisions: Vec<_> = explained
            .records
            .iter()
            .filter_map(|record| record.pointer("/decision/steps/0"))
            .filter(|step| step["kind"] == "open")
            .collect();
        assert_eq!(decisions.len(), 2);
        assert_eq!(
            decisions[0]["predicates"],
            json!([{"index":0,"value":false},{"index":1,"value":true}])
        );
        assert_eq!(decisions[0]["selected"], 1);
        assert_eq!(
            decisions[1]["predicates"],
            json!([{"index":0,"value":false},{"index":1,"value":false}])
        );
        assert_eq!(decisions[1]["selected"], JsonValue::Null);
        let edge = compilation
            .plan
            .edges
            .iter()
            .find(|edge| {
                edge.from.node.is_some_and(|slot| {
                    compilation.plan.nodes[slot.index()].path.to_string() == "classify"
                }) && edge.to.is_some()
            })
            .unwrap();
        assert_eq!(edge_cases(compilation.plan.plan(), edge), ["Kept"]);
    }

    #[test]
    fn join_decisions_use_named_inputs_and_the_retained_completing_port() {
        let source = r#"
graph joined : App {
  input left: int
  input right: int
  output out: int
  operator pair {
    input left: int
    input right: int
    output out: int
    on (left, right) match {
      left.data < right.data => out = right.data
      _ => out = left.data
    }
  }
  left -> pair.left
  right -> pair.right
  pair.out -> out
}
lab joined {
  simulation inspect on app {
    scenario pair {
      app.left = 2
      app.right = 7
      expect app.out count == 1 where data == 7
    }
  }
}
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
        session.run_to_completion();
        assert!(session.report().passed());
        let explained = history(&compilation.plan, session.journal().records());
        let decision = explained
            .records
            .iter()
            .filter_map(|record| record.get("decision"))
            .find(|decision| !decision.is_null())
            .unwrap();
        assert_eq!(decision["trigger"], "join:left,right");
        assert_eq!(
            decision["steps"][0]["predicates"],
            json!([{"index":0,"value":true}])
        );
    }
}
