//! Re-derived output obligations. Absence of a route is never a disposition.

use super::*;
use crate::connector::FrameProduction;
use crate::node;

pub(super) fn validate(
    plan: &ExecutionPlan,
    ports: &ResolvedPortTable,
) -> Result<(), PlanValidationError> {
    let live = live_owners(plan);
    for (index, node) in plan.nodes.iter().enumerate() {
        if !live[node.owner.index()] {
            continue;
        }
        let slot = NodeIndex::new(u32::try_from(index).unwrap_or(u32::MAX));
        for decl in ports.node(slot).into_iter().flatten() {
            if decl.direction() != node::PortDirection::Output
                || !may_produce(plan, slot, decl.port())
            {
                continue;
            }
            let source = EdgeEnd {
                node: Some(slot),
                port: decl.port().clone(),
            };
            if plan.edges.iter().any(|edge| edge.from == source)
                || plan
                    .routes
                    .iter()
                    .any(|rule| plan.endpoint_matches(&rule.source, &source))
            {
                // Selection validation proves every row nonempty and every
                // closed case covered. A command may have no physical edge.
                continue;
            }
            return Err(PlanValidationError::UnwiredOutput {
                node: node.path.clone(),
                port: decl.port().clone(),
            });
        }
    }
    Ok(())
}

pub(super) fn may_produce(plan: &ExecutionPlan, slot: NodeIndex, port: &node::Port) -> bool {
    let node = &plan.nodes[slot.index()];
    if plan.owners[node.owner.index()]
        .loop_spec()
        .and_then(|spec| spec.expired.as_ref())
        .is_some_and(|expired| expired.port == *port)
    {
        return true;
    }
    match &node.execution {
        NodeExecution::Operator { handlers, .. } => handlers.iter().any(|handler| {
            handler.dispatch.may_emit(port)
                    // Re-derive the implicit producer from the validated join bound.
                    || (port.name() == "expired"
                        && matches!(handler.activation,
                            node::HandlerActivation::Join { within_ns: Some(_), .. }
                            | node::HandlerActivation::Join { last: Some(_), .. }))
        }),
        NodeExecution::Connector(connector) => !connector
            .call_semantics()
            .is_some_and(|call| call.frame_production(port.name()) == Some(FrameProduction::Never)),
    }
}

/// Whole-Graph inaccessibility, not an exemption for an unused Node in a live
/// scope. Inbound Contracts are roots; a spatial crossing may activate every
/// enclosing scope. A least fixed point does not depend on slot order.
pub(super) fn live_owners(plan: &ExecutionPlan) -> Vec<bool> {
    let mut live = alloc::vec![false; plan.owners.len()];
    live[0] = true;
    let activate = |owner: node::OwnerSlot, live: &mut [bool]| {
        let mut current = Some(owner);
        let mut changed = false;
        while let Some(slot) = current {
            changed |= !live[slot.index()];
            live[slot.index()] = true;
            current = plan.owners[slot.index()].parent;
        }
        changed
    };
    for node in &plan.nodes {
        if node
            .connector()
            .is_some_and(|connector| connector.roles.has_inbound())
        {
            activate(node.owner, &mut live);
        }
    }
    loop {
        let mut changed = false;
        for (index, owner) in plan.owners.iter().enumerate() {
            if matches!(owner.kind, OwnerKind::Node(_))
                && owner.parent.is_some_and(|parent| live[parent.index()])
                && !live[index]
            {
                live[index] = true;
                changed = true;
            }
        }
        for edge in &plan.edges {
            if edge
                .from
                .node
                .is_some_and(|slot| !live[plan.nodes[slot.index()].owner.index()])
            {
                continue;
            }
            if let Some(slot) = edge.to.as_ref().and_then(|target| target.node) {
                changed |= activate(plan.nodes[slot.index()].owner, &mut live);
            }
        }
        for rule in &plan.boundary_routes {
            let source_path = match &rule.source.owner {
                EndpointOwner::Node(path) => node::OwnerPath::from(path),
                EndpointOwner::Graph(path) => path.clone(),
            };
            if !plan
                .owners
                .iter()
                .enumerate()
                .any(|(index, owner)| owner.path == source_path && live[index])
            {
                continue;
            }
            let sets: Vec<_> = match &rule.selection {
                RouteSelection::All(set) => alloc::vec![set],
                RouteSelection::Cases(arms) => arms.iter().map(|arm| &arm.destinations).collect(),
            };
            for target in sets.into_iter().flatten() {
                if let RouteDestination::Port(Endpoint {
                    owner: EndpointOwner::Graph(path),
                    ..
                }) = target
                    && let Some(index) = plan.owners.iter().position(|owner| owner.path == *path)
                {
                    changed |= activate(node::OwnerSlot::new(index as u32), &mut live);
                }
            }
        }
        if !changed {
            return live;
        }
    }
}
