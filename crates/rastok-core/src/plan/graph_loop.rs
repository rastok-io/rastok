//! Conservative control proof for spatial Operator applications. Counts are
//! saturated at two: the proof distinguishes silence, one verdict and conflict.

use super::*;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::ToString;
use alloc::{format, vec};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Outcome {
    terminal: bool,
    other: u8,
}

type Outcomes = BTreeSet<Outcome>;

fn empty() -> Outcomes {
    BTreeSet::from([Outcome {
        terminal: false,
        other: 0,
    }])
}
fn combine(left: &Outcomes, right: &Outcomes) -> Outcomes {
    left.iter()
        .flat_map(|left| {
            right.iter().map(move |right| Outcome {
                terminal: left.terminal || right.terminal,
                other: left.other.saturating_add(right.other).min(2),
            })
        })
        .collect()
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Work {
    Source(Endpoint),
    Input(Endpoint),
}

pub(super) fn validate(plan: &ExecutionPlan, owner: &Owner) -> Result<(), PlanValidationError> {
    let error = |path, detail| PlanValidationError::InvalidLoopControl {
        owner: owner.path.to_string(),
        path,
        detail,
    };
    let Some(spec) = owner.loop_spec() else {
        return Ok(());
    };
    if plan
        .boundary
        .iter()
        .chain(plan.nodes.iter().flat_map(|node| &node.ports))
        .chain(plan.owners.iter().flat_map(|owner| &owner.ports))
        .any(|port| !port.annotations.is_empty())
        || plan
            .nodes
            .iter()
            .flat_map(|node| &node.vars)
            .any(|var| !var.annotations.is_empty())
        || plan
            .owners
            .iter()
            .flat_map(|owner| &owner.vals)
            .any(|value| !value.annotations.is_empty())
        || plan
            .handlers
            .iter()
            .flat_map(|handler| handler.dispatch.steps())
            .any(|step| {
                matches!(step, DispatchStep::Let { annotations, .. } if !annotations.is_empty())
            })
    {
        return Err(error(
            "data-policy".into(),
            "Graph Loop annotated state and recurrence require policy qualification",
        ));
    }
    if spec.fuel.is_none() || spec.proof.is_some() {
        return Err(error(
            "measure".into(),
            "Graph measure proofs require whole-body progress qualification",
        ));
    }
    if plan
        .owners
        .iter()
        .filter(|owner| owner.loop_spec().is_some())
        .count()
        != 1
    {
        return Err(error(
            "context".into(),
            "nested or adjacent Graph Loop contexts require their qualification",
        ));
    }
    if plan
        .nodes
        .iter()
        .any(|node| !matches!(node.execution, NodeExecution::Operator))
        || plan
            .handlers
            .iter()
            .any(|handler| !matches!(handler.activation, node::HandlerActivation::Port(_)))
    {
        return Err(error(
            "application".into(),
            "Graph Loop Calls, joins and lifecycle handlers require obligation qualification",
        ));
    }
    let entry = Endpoint {
        owner: EndpointOwner::Graph(owner.path.clone()),
        port: spec.entry.clone(),
    };
    let in_scope = |endpoint: &Endpoint| match &endpoint.owner {
        EndpointOwner::Node(path) => path.segments().starts_with(owner.path.segments()),
        EndpointOwner::Graph(path) => path.segments().starts_with(owner.path.segments()),
    };
    for route in &plan.boundary_routes {
        for destination in sets(&route.selection).into_iter().flatten() {
            if let RouteDestination::Port(target) = destination
                && in_scope(target)
                && *target != entry
                && !in_scope(&route.source)
            {
                return Err(error(
                    "context".into(),
                    "a Graph Loop descendant must receive its context through the recurrent entry",
                ));
            }
        }
    }
    if plan.owners.iter().any(|child| {
        child.path != owner.path
            && child.path.segments().starts_with(owner.path.segments())
            && child.instance.lifetime != InstanceLifetime::Persistent
    }) || plan
        .handlers
        .iter()
        .flat_map(|handler| handler.dispatch.steps())
        .any(|step| matches!(step, DispatchStep::Stop))
    {
        return Err(error(
            "closing".into(),
            "Graph Loop descendant-local stopping requires obligation qualification",
        ));
    }
    let root = Work::Source(entry);
    let mut immediate = vec![root.clone()];
    let mut inspected = BTreeSet::new();
    while let Some(Work::Source(source)) = immediate.pop() {
        if !inspected.insert(source.clone()) {
            continue;
        }
        for rule in plan
            .boundary_routes
            .iter()
            .filter(|rule| rule.source == source)
        {
            for destination in sets(&rule.selection).into_iter().flatten() {
                if matches!(destination, RouteDestination::LoopCommand(_))
                    || matches!(destination, RouteDestination::Port(target) if target.owner == EndpointOwner::Graph(owner.path.clone()))
                {
                    return Err(error(
                        "entry".into(),
                        "a Graph Loop boundary-only verdict requires manager-entry qualification",
                    ));
                }
                if let Some(Work::Source(source)) = dependency(destination, &owner.path) {
                    immediate.push(Work::Source(source));
                }
            }
        }
    }
    let mut cache = BTreeMap::<Work, Outcomes>::new();
    let mut visiting = BTreeSet::new();
    let mut pending = vec![(root.clone(), false)];
    while let Some((work, expanded)) = pending.pop() {
        if cache.contains_key(&work) {
            continue;
        }
        let mut dependencies = BTreeSet::new();
        match &work {
            Work::Source(source) => {
                for rule in plan
                    .boundary_routes
                    .iter()
                    .filter(|rule| rule.source == *source)
                {
                    for destination in sets(&rule.selection).into_iter().flatten() {
                        if let Some(next) = dependency(destination, &owner.path) {
                            dependencies.insert(next);
                        }
                    }
                }
            }
            Work::Input(input) => {
                let handler = input_handler(plan, input).ok_or_else(|| {
                    error(
                        format!("{:?}", input),
                        "an internal Loop input has no finite application handler",
                    )
                })?;
                for (port, expression) in handler.dispatch.steps().flat_map(DispatchStep::emissions)
                {
                    if !matches!(expression, Emission::Nothing) {
                        dependencies.insert(Work::Source(Endpoint {
                            owner: input.owner.clone(),
                            port: port.clone(),
                        }));
                    }
                }
            }
        }
        if !expanded {
            if !visiting.insert(work.clone()) {
                return Err(error(
                    "routing".into(),
                    "Loop application routing contains a spatial cycle",
                ));
            }
            pending.push((work.clone(), true));
            pending.extend(
                dependencies
                    .into_iter()
                    .filter(|dependency| !cache.contains_key(dependency))
                    .map(|dependency| (dependency, false)),
            );
            continue;
        }
        let outcomes = match &work {
            Work::Source(source) => {
                let mut total = empty();
                for rule in plan
                    .boundary_routes
                    .iter()
                    .filter(|rule| rule.source == *source)
                {
                    let mut alternatives = Outcomes::new();
                    for destinations in sets(&rule.selection) {
                        let mut arm = empty();
                        for destination in destinations {
                            let next = match destination {
                                RouteDestination::Discard => empty(),
                                RouteDestination::LoopCommand(command)
                                    if command.owner == EndpointOwner::Graph(owner.path.clone()) =>
                                {
                                    BTreeSet::from([Outcome { terminal: false, other: 1 }])
                                }
                                RouteDestination::LoopCommand(_) => {
                                    return Err(error(
                                        "command".into(),
                                        "a Graph command must address the current Loop scope",
                                    ));
                                }
                                RouteDestination::Port(target)
                                    if target.owner == EndpointOwner::Graph(owner.path.clone()) =>
                                {
                                    if target.port == spec.entry {
                                        return Err(error(
                                            "entry".into(),
                                            "a Loop recurrence cannot be a spatial entry edge",
                                        ));
                                    }
                                    BTreeSet::from([Outcome {
                                        terminal: spec.terminals.contains(&target.port),
                                        other: 0,
                                    }])
                                }
                                RouteDestination::Port(_) => dependency(destination, &owner.path)
                                    .and_then(|work| cache.get(&work))
                                    .cloned()
                                    .ok_or_else(|| {
                                        error(
                                            "routing".into(),
                                            "a Graph route leaves its causal application without a boundary",
                                        )
                                    })?,
                            };
                            arm = combine(&arm, &next);
                        }
                        alternatives.extend(arm);
                    }
                    total = combine(&total, &alternatives);
                }
                total
            }
            Work::Input(input) => {
                let handler = input_handler(plan, input).ok_or_else(|| {
                    error("input".into(), "an internal Loop input lost its handler")
                })?;
                let mut alternatives = Outcomes::new();
                for leaf in handler.dispatch.decisions() {
                    let Dispatch::Linear(steps) = leaf else {
                        continue;
                    };
                    let mut total = empty();
                    for step in steps {
                        if matches!(step, DispatchStep::LoopCommand(_)) {
                            return Err(error(
                                "command".into(),
                                "descendant direct commands require their context qualification; route an output to Loop.next/again/done",
                            ));
                        }
                        for (port, _) in step
                            .emissions()
                            .filter(|(_, expression)| !matches!(expression, Emission::Nothing))
                        {
                            let source = Work::Source(Endpoint {
                                owner: input.owner.clone(),
                                port: port.clone(),
                            });
                            let next = cache.get(&source).ok_or_else(|| {
                                error("output".into(), "a produced output has no derived route")
                            })?;
                            total = combine(&total, next);
                        }
                    }
                    // A child transaction coalesces its own terminal crossings;
                    // two different child transactions never coalesce verdicts.
                    alternatives.extend(total.into_iter().map(|outcome| Outcome {
                        terminal: false,
                        other: (outcome.other + u8::from(outcome.terminal)).min(2),
                    }));
                }
                alternatives
            }
        };
        visiting.remove(&work);
        cache.insert(work, outcomes);
    }
    if cache.get(&root).is_none_or(|outcomes| {
        outcomes.is_empty()
            || outcomes
                .iter()
                .any(|outcome| outcome.other + u8::from(outcome.terminal) != 1)
    }) {
        return Err(error(
            "entry".into(),
            "each spatial Loop application must select exactly one verdict on every Dispatch path",
        ));
    }
    Ok(())
}

fn sets(selection: &RouteSelection) -> Vec<&[RouteDestination]> {
    match selection {
        RouteSelection::All(set) => vec![set],
        RouteSelection::Cases(arms) => arms.iter().map(|arm| arm.destinations.as_slice()).collect(),
    }
}

fn dependency(destination: &RouteDestination, scope: &OwnerPath) -> Option<Work> {
    let RouteDestination::Port(target) = destination else {
        return None;
    };
    match &target.owner {
        EndpointOwner::Node(path) if path.segments().starts_with(scope.segments()) => {
            Some(Work::Input(target.clone()))
        }
        EndpointOwner::Graph(path)
            if path != scope && path.segments().starts_with(scope.segments()) =>
        {
            Some(Work::Source(target.clone()))
        }
        _ => None,
    }
}

fn input_handler<'a>(plan: &'a ExecutionPlan, input: &Endpoint) -> Option<&'a Handler> {
    let EndpointOwner::Node(path) = &input.owner else {
        return None;
    };
    let slot = Slot::new(plan.nodes.iter().position(|node| node.path == *path)? as u32);
    plan.handlers.iter().find(|handler| {
        handler.owner == slot
            && handler.activation == node::HandlerActivation::Port(input.port.clone())
    })
}
