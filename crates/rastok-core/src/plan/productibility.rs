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
        if !live[index] || node.kind() == node::Kind::Graph {
            continue;
        }
        let slot = NodeIndex::new(u32::try_from(index).unwrap_or(u32::MAX));
        for decl in ports.node(slot).into_iter().flatten() {
            if decl.direction() != node::PortDirection::Output
                || !may_produce(plan, slot, decl.port())
            {
                continue;
            }
            let source = Endpoint {
                node: slot,
                port: decl.port().clone(),
            };
            if plan.edges.iter().any(|edge| edge.from == source)
                || plan.routes.iter().any(|rule| rule.source == source)
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
    if node
        .loop_spec()
        .and_then(|spec| spec.expired.as_ref())
        .is_some_and(|expired| expired.port == *port)
    {
        return true;
    }
    match &node.body {
        NodeKind::Graph(_) => false,
        NodeKind::Operator(operator) => operator.handlers.iter().any(|handler| {
            handler.dispatch.may_emit(port)
                    // Re-derive the implicit producer from the validated join bound.
                    || (port.name() == "expired"
                        && matches!(handler.activation,
                            node::HandlerActivation::Join { within_ns: Some(_), .. }
                            | node::HandlerActivation::Join { last: Some(_), .. }))
        }),
        NodeKind::Connector(connector) => !connector
            .call_semantics()
            .is_some_and(|call| call.frame_production(port.name()) == Some(FrameProduction::Never)),
    }
}

/// Whole-Graph inaccessibility, not an exemption for an unused Node in a live
/// scope. Inbound Contracts are roots; a spatial crossing may activate every
/// enclosing scope. A least fixed point does not depend on slot order.
pub(super) fn live_owners(plan: &ExecutionPlan) -> Vec<bool> {
    let mut live = alloc::vec![false; plan.nodes.len()];
    live[0] = true;
    let activate = |owner: node::NodeIndex, live: &mut [bool]| {
        let mut current = Some(owner);
        let mut changed = false;
        while let Some(slot) = current {
            changed |= !live[slot.index()];
            live[slot.index()] = true;
            current = plan.nodes[slot.index()].parent;
        }
        changed
    };
    for (index, node) in plan.nodes.iter().enumerate() {
        if node
            .connector()
            .is_some_and(|connector| connector.roles.has_inbound())
        {
            activate(NodeIndex::new(index as u32), &mut live);
        }
    }
    loop {
        let mut changed = false;
        for (index, owner) in plan.nodes.iter().enumerate() {
            if owner.kind() != node::Kind::Graph
                && owner.parent.is_some_and(|parent| live[parent.index()])
                && !live[index]
            {
                live[index] = true;
                changed = true;
            }
        }
        for edge in &plan.edges {
            if !live[edge.from.node.index()] {
                continue;
            }
            if let Some(target) = &edge.to {
                changed |= activate(target.node, &mut live);
            }
        }
        for rule in plan.reference_routes() {
            if !live[rule.source.node.index()] {
                continue;
            }
            let sets: Vec<_> = match &rule.selection {
                RouteSelection::All(set) => alloc::vec![set],
                RouteSelection::Cases(arms) => arms.iter().map(|arm| &arm.destinations).collect(),
            };
            for target in sets.into_iter().flatten() {
                if let Destination::Port(endpoint) = target
                    && plan.nodes[endpoint.node.index()].kind() == node::Kind::Graph
                {
                    changed |= activate(endpoint.node, &mut live);
                }
            }
        }
        if !changed {
            return live;
        }
    }
}
