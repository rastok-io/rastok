//! Proof-only projection of routed next-state products. No executable Graph
//! handler is built or retained in the plan.

use super::{Certificate, Error, derive_dispatch};
use crate::expr;
use crate::node;
use crate::plan::{
    Dispatch, DispatchStep, Endpoint, EndpointOwner, ExecutionPlan, LoopCommand, LoopCommandKind,
    Owner, RouteDestination, RouteSelection,
};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec;
use alloc::vec::Vec;

/// Re-derives a Graph's direct integer measure from its spatial producers.
/// The supported fragment passes the entry unchanged through passive scopes
/// to the deciding Operators. Opaque transformations require authored fuel.
///
/// # Errors
/// A recurrence cannot be traced to one direct-entry Dispatch, or one such
/// Dispatch does not expose a guarded literal decrease.
pub fn derive_graph(
    measure: expr::Tree,
    invariant: Option<expr::Tree>,
    plan: &ExecutionPlan,
    owner: &Owner,
) -> Result<Certificate, Error> {
    let spec = owner.loop_spec().ok_or(Error::UnsupportedActivation)?;
    let mut inputs = BTreeSet::new();
    let mut pending = vec![Endpoint {
        owner: EndpointOwner::Graph(owner.path.clone()),
        port: spec.entry.clone(),
    }];
    let mut visited = BTreeSet::new();
    while let Some(source) = pending.pop() {
        if !visited.insert(source.clone()) {
            return Err(Error::UnsupportedActivation);
        }
        for rule in plan
            .boundary_routes
            .iter()
            .filter(|rule| rule.source == source)
        {
            let RouteSelection::All(destinations) = &rule.selection else {
                return Err(Error::UnsupportedActivation);
            };
            for target in destinations {
                if let RouteDestination::Port(target) = target {
                    match &target.owner {
                        EndpointOwner::Node(_) => {
                            inputs.insert(target.clone());
                        }
                        EndpointOwner::Graph(path) if path != &owner.path => {
                            pending.push(target.clone())
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    let mut producers = BTreeMap::new();
    for node in &plan.nodes {
        if !node.path.segments().starts_with(owner.path.segments()) {
            continue;
        }
        for port in node
            .ports
            .iter()
            .filter(|port| port.direction == node::PortDirection::Output)
        {
            let source = Endpoint {
                owner: EndpointOwner::Node(node.path.clone()),
                port: port.port.clone(),
            };
            if recurs(plan, owner, &source)? {
                producers.insert(source, false);
            }
        }
    }
    let mut result = derive_dispatch(
        measure.clone(),
        invariant.clone(),
        &Dispatch::Linear(vec![]),
    )?;
    for handler in &plan.handlers {
        let Some(node) = plan.nodes.get(handler.owner.index()) else {
            return Err(Error::UnsupportedActivation);
        };
        let keys: BTreeSet<_> = handler
            .dispatch
            .steps()
            .flat_map(DispatchStep::emissions)
            .filter(|(_, value)| !matches!(value, crate::plan::EmitValue::Nothing))
            .map(|(port, _)| Endpoint {
                owner: EndpointOwner::Node(node.path.clone()),
                port: port.clone(),
            })
            .filter(|source| producers.contains_key(source))
            .collect();
        if keys.is_empty() {
            continue;
        }
        let node::HandlerActivation::Port(port) = &handler.activation else {
            return Err(Error::UnsupportedActivation);
        };
        if !inputs.contains(&Endpoint {
            owner: EndpointOwner::Node(node.path.clone()),
            port: port.clone(),
        }) {
            return Err(Error::UnsupportedActivation);
        }
        let input = Endpoint {
            owner: EndpointOwner::Node(node.path.clone()),
            port: port.clone(),
        };
        // A direct path is not enough: every possible producer of this
        // input must preserve the entry. Otherwise a second, transformed
        // input could make a local decrement increase the actual measure.
        for route in &plan.boundary_routes {
            let targets = match &route.selection {
                RouteSelection::All(targets) => vec![targets.as_slice()],
                RouteSelection::Cases(arms) => {
                    arms.iter().map(|arm| arm.destinations.as_slice()).collect()
                }
            };
            if targets
                .into_iter()
                .flatten()
                .any(|target| matches!(target, RouteDestination::Port(target) if *target == input))
                && !visited.contains(&route.source)
            {
                return Err(Error::UnsupportedActivation);
            }
        }
        let mut dispatch = handler.dispatch.clone();
        let ports = keys.iter().map(|source| source.port.clone()).collect();
        replace_products(&mut dispatch, &ports, 0)?;
        let proof = derive_dispatch(measure.clone(), invariant.clone(), &dispatch)?;
        result.decreases.extend(proof.decreases);
        for source in keys {
            producers.insert(source, true);
        }
    }
    if producers.values().any(|proved| !proved) {
        return Err(Error::UnsupportedActivation);
    }
    result.decreases.sort_unstable();
    Ok(result)
}

fn recurs(plan: &ExecutionPlan, owner: &Owner, source: &Endpoint) -> Result<bool, Error> {
    let mut pending = vec![source.clone()];
    let mut visited = BTreeSet::new();
    let mut found = false;
    while let Some(source) = pending.pop() {
        if !visited.insert(source.clone()) {
            return Err(Error::UnsupportedActivation);
        }
        for rule in plan
            .boundary_routes
            .iter()
            .filter(|rule| rule.source == source)
        {
            let selections: Vec<_> = match &rule.selection {
                RouteSelection::All(destinations) => vec![destinations.as_slice()],
                RouteSelection::Cases(arms) => {
                    arms.iter().map(|arm| arm.destinations.as_slice()).collect()
                }
            };
            for destinations in selections {
                for destination in destinations {
                    match destination {
                        RouteDestination::LoopCommand(command)
                            if command.owner == EndpointOwner::Graph(owner.path.clone()) =>
                        {
                            match command.kind {
                                LoopCommandKind::Again => {
                                    return Err(Error::MeasureDoesNotDecrease);
                                }
                                LoopCommandKind::Next => {
                                    if !matches!(rule.selection, RouteSelection::All(_)) || found {
                                        return Err(Error::UnsupportedActivation);
                                    }
                                    found = true;
                                }
                                LoopCommandKind::Done => {}
                            }
                        }
                        RouteDestination::Port(target)
                            if matches!(&target.owner, EndpointOwner::Graph(path)
                            if path != &owner.path && path.segments().starts_with(owner.path.segments())) =>
                        {
                            pending.push(target.clone())
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(found)
}

fn replace_products(
    dispatch: &mut Dispatch,
    ports: &BTreeSet<node::Port>,
    depth: usize,
) -> Result<(), Error> {
    if depth > crate::plan::MAX_DISPATCH_DEPTH {
        return Err(Error::UnsupportedActivation);
    }
    match dispatch {
        Dispatch::Linear(steps) => {
            let mut projected = vec![];
            for step in steps.iter() {
                if matches!(step, DispatchStep::LoopCommand(_)) {
                    return Err(Error::UnsupportedActivation);
                }
                projected.push(step.clone());
                for (port, value) in step.emissions() {
                    if ports.contains(port) && !matches!(value, crate::plan::EmitValue::Nothing) {
                        projected.push(DispatchStep::LoopCommand(LoopCommand::Next(value.clone())));
                    }
                }
            }
            *steps = projected;
        }
        Dispatch::OpenMatch { arms, fallback, .. } => {
            for arm in arms {
                replace_products(&mut arm.dispatch, ports, depth + 1)?;
            }
            replace_products(fallback, ports, depth + 1)?;
        }
        Dispatch::ClosedMatch { arms, .. } => {
            for arm in arms {
                replace_products(&mut arm.dispatch, ports, depth + 1)?;
            }
        }
    }
    Ok(())
}
