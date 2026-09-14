//! Conservative control proof for spatial Executor applications. Counts are
//! saturated at two: the proof distinguishes silence, one verdict and conflict.

use super::*;
use crate::connector::{CallSemantics, FrameProduction};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::ToString;
use alloc::{format, vec};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Outcome {
    terminal: bool,
    other: u8,
    recurs: bool,
    unsafe_effect: bool,
    joins: BTreeMap<Endpoint, u8>,
    retries: BTreeSet<OwnerPath>,
}

type Outcomes = BTreeSet<Outcome>;

// Independent joins and nested retries can otherwise multiply proof states
// exponentially. Reaching this bound refuses the proof, never approximates it.
const MAX_GRAPH_LOOP_OUTCOMES: usize = 1024;

fn proof_limit(owner: &Owner) -> PlanValidationError {
    PlanValidationError::InvalidLoopControl {
        owner: owner.path.to_string(),
        path: "proof".into(),
        detail: "the Graph Loop control proof exceeds 1024 distinct outcomes",
    }
}

fn empty() -> Outcomes {
    BTreeSet::from([Outcome {
        terminal: false,
        other: 0,
        recurs: false,
        unsafe_effect: false,
        joins: BTreeMap::new(),
        retries: BTreeSet::new(),
    }])
}
fn combine(left: &Outcomes, right: &Outcomes) -> Option<Outcomes> {
    let mut combined = Outcomes::new();
    for left in left {
        for right in right {
            let mut joins = left.joins.clone();
            for (input, count) in &right.joins {
                let total = joins.entry(input.clone()).or_default();
                *total = total.saturating_add(*count).min(2);
            }
            combined.insert(Outcome {
                terminal: left.terminal || right.terminal,
                other: left.other.saturating_add(right.other).min(2),
                recurs: left.recurs || right.recurs,
                unsafe_effect: left.unsafe_effect || right.unsafe_effect,
                joins,
                retries: left.retries.union(&right.retries).cloned().collect(),
            });
            if combined.len() > MAX_GRAPH_LOOP_OUTCOMES {
                return None;
            }
        }
    }
    Some(combined)
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Work {
    Source(Endpoint, Option<String>),
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
        .handlers
        .iter()
        .filter(|handler| {
            plan.nodes[handler.owner.index()]
                .path
                .segments()
                .starts_with(owner.path.segments())
        })
        .any(|handler| matches!(handler.activation, node::HandlerActivation::Lifecycle(_)))
    {
        return Err(error(
            "application".into(),
            "a Graph Loop application cannot depend on a lifecycle handler",
        ));
    }
    for node in plan
        .nodes
        .iter()
        .filter(|node| node.path.segments().starts_with(owner.path.segments()))
    {
        if let Some(connector) = node.connector() {
            if connector.call_semantics().is_none() {
                return Err(error(
                    node.path.to_string(),
                    "a Graph Loop application requires a finite outbound Call profile",
                ));
            }
            if !matches!(connector.attribute("timeout"), Some(AttrSetting::Value(Value::Duration(ns))) if *ns > 0)
            {
                return Err(error(
                    format!("{}.timeout", node.path),
                    "an outbound Call in Loop needs a positive plan-constant timeout",
                ));
            }
        }
    }
    for handler in plan.handlers.iter().filter(|handler| {
        plan.nodes[handler.owner.index()]
            .path
            .segments()
            .starts_with(owner.path.segments())
    }) {
        if let node::HandlerActivation::Join {
            within_ns, last, ..
        } = &handler.activation
            && (handler.guard.is_some() || within_ns.is_some() || last.is_some())
        {
            return Err(error(
                "join".into(),
                "a retained or expiring join needs a finite per-run completion proof",
            ));
        }
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
    }) {
        return Err(error(
            "closing".into(),
            "Graph Loop descendant-local stopping requires obligation qualification",
        ));
    }
    let root = Work::Source(entry, None);
    let mut cache = BTreeMap::<Work, Outcomes>::new();
    let mut visiting = BTreeSet::new();
    let mut pending = vec![(root.clone(), false)];
    while let Some((work, expanded)) = pending.pop() {
        if cache.contains_key(&work) {
            continue;
        }
        let mut dependencies = BTreeSet::new();
        let nested = nested_host(plan, owner, &work);
        if let Some(nested) = nested
            && nested
                .loop_spec()
                .is_some_and(|spec| spec.proof.is_none() || spec.fuel.is_some())
        {
            dependencies.insert(expiry_work(nested)?);
        }
        match &work {
            Work::Source(source, case) => {
                for rule in plan
                    .boundary_routes
                    .iter()
                    .filter(|rule| rule.source == *source)
                {
                    for destination in sets(&rule.selection).into_iter().flatten() {
                        if let Some(next) = dependency(destination, &owner.path, case.as_deref()) {
                            dependencies.insert(next);
                        }
                    }
                }
            }
            Work::Input(input) => {
                if let Some((node, call)) = input_call(plan, input) {
                    for port in &node.ports {
                        if call
                            .frame_production(port.port.name())
                            .is_some_and(|production| production != FrameProduction::Never)
                        {
                            dependencies.insert(Work::Source(
                                Endpoint {
                                    owner: input.owner.clone(),
                                    port: port.port.clone(),
                                },
                                None,
                            ));
                        }
                    }
                    for (case, _) in call.terminal_cases() {
                        dependencies.insert(Work::Source(
                            Endpoint {
                                owner: input.owner.clone(),
                                port: node::Port::new(call.terminal()).map_err(|_| {
                                    error("terminal".into(), "a Call terminal must be a port")
                                })?,
                            },
                            Some((*case).into()),
                        ));
                    }
                } else {
                    let handler = input_handler(plan, input).ok_or_else(|| {
                        error(
                            format!("{:?}", input),
                            "an internal Loop input has no finite application handler",
                        )
                    })?;
                    if matches!(&handler.activation, node::HandlerActivation::Join { ports, .. } if ports.first() != Some(&input.port))
                    {
                        dependencies.clear();
                    } else {
                        for (port, expression) in
                            handler.dispatch.steps().flat_map(DispatchStep::emissions)
                        {
                            if !matches!(expression, Emission::Nothing) {
                                dependencies.insert(Work::Source(
                                    Endpoint {
                                        owner: input.owner.clone(),
                                        port: port.clone(),
                                    },
                                    None,
                                ));
                            }
                        }
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
            Work::Source(source, case) => {
                let mut total = empty();
                for rule in plan
                    .boundary_routes
                    .iter()
                    .filter(|rule| rule.source == *source)
                {
                    let mut alternatives = Outcomes::new();
                    let selections = match (&rule.selection, case) {
                        (RouteSelection::Cases(arms), Some(case)) => arms
                            .iter()
                            .filter(|arm| arm.cases.contains(case))
                            .map(|arm| arm.destinations.as_slice())
                            .collect(),
                        _ => sets(&rule.selection),
                    };
                    for destinations in selections {
                        let mut arm = empty();
                        for destination in destinations {
                            let next = match destination {
                                RouteDestination::InstanceStop(target)
                                    if target.owner == EndpointOwner::Graph(owner.path.clone()) => {
                                    BTreeSet::from([Outcome { terminal: false, other: 1, recurs: false,
                                        unsafe_effect: false, joins: BTreeMap::new(), retries: BTreeSet::new() }])
                                }
                                RouteDestination::InstanceStop(_) => return Err(error(
                                    "routing".into(),
                                    "a child-only closure inside a Loop application requires the deferred lifecycle-dependent control proof; close the Loop scope or route closure from its enclosing Graph",
                                )),
                                RouteDestination::Discard => empty(),
                                RouteDestination::LoopCommand(command)
                                    if command.owner == EndpointOwner::Graph(owner.path.clone()) =>
                                {
                                    BTreeSet::from([Outcome {
                                        terminal: false, other: 1,
                                        recurs: command.kind != LoopCommandKind::Done,
                                        unsafe_effect: false,
                                        joins: BTreeMap::new(),
                                        retries: BTreeSet::new(),
                                    }])
                                }
                                RouteDestination::LoopCommand(command) => {
                                    let target = match &command.owner {
                                        EndpointOwner::Graph(path) => path.clone(),
                                        EndpointOwner::Node(path) => OwnerPath::from(path),
                                    };
                                    if !target.segments().starts_with(owner.path.segments()) {
                                        return Err(error("command".into(), "a command escapes its active Loop context"));
                                    }
                                    empty().into_iter().map(|mut outcome| {
                                        if command.kind != LoopCommandKind::Done { outcome.retries.insert(target.clone()); }
                                        outcome
                                    }).collect()
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
                                        recurs: false,
                                        unsafe_effect: false,
                                        joins: BTreeMap::new(),
                                        retries: BTreeSet::new(),
                                    }])
                                }
                                RouteDestination::Port(_) => dependency(destination, &owner.path, case.as_deref())
                                    .and_then(|work| cache.get(&work))
                                    .cloned()
                                    .ok_or_else(|| {
                                        error(
                                            "routing".into(),
                                            "a Graph route leaves its causal application without a boundary",
                                        )
                                    })?,
                            };
                            arm = combine(&arm, &next).ok_or_else(|| proof_limit(owner))?;
                        }
                        alternatives.extend(arm);
                        if alternatives.len() > MAX_GRAPH_LOOP_OUTCOMES {
                            return Err(proof_limit(owner));
                        }
                    }
                    total = combine(&total, &alternatives).ok_or_else(|| proof_limit(owner))?;
                }
                total
            }
            Work::Input(input) => {
                if let Some((node, call)) = input_call(plan, input) {
                    call_outcomes(node, input, call, &cache).ok_or_else(|| {
                        error(
                            node.path.to_string(),
                            "a Call output lost its derived routes",
                        )
                    })?
                } else {
                    let handler = input_handler(plan, input).ok_or_else(|| {
                        error("input".into(), "an internal Loop input lost its handler")
                    })?;
                    let joined = matches!(handler.activation, node::HandlerActivation::Join { .. });
                    let secondary = matches!(&handler.activation, node::HandlerActivation::Join { ports, .. } if ports.first() != Some(&input.port));
                    let mut alternatives = Outcomes::new();
                    if secondary {
                        alternatives = empty();
                    } else {
                        for leaf in handler.dispatch.decisions() {
                            let Dispatch::Linear(steps) = leaf else {
                                continue;
                            };
                            let mut total = empty();
                            for step in steps {
                                if let DispatchStep::LoopCommand(command) = step {
                                    let host = &plan.owners
                                        [plan.nodes[handler.owner.index()].owner.index()];
                                    if host.loop_spec().is_none() {
                                        return Err(error(
                                            "command".into(),
                                            "a direct command requires an owning Loop Executor",
                                        ));
                                    }
                                    if !matches!(command, LoopCommand::Done) {
                                        total = total
                                            .into_iter()
                                            .map(|mut outcome| {
                                                outcome.retries.insert(host.path.clone());
                                                outcome
                                            })
                                            .collect();
                                    }
                                }
                                for (port, _) in step.emissions().filter(|(_, expression)| {
                                    !matches!(expression, Emission::Nothing)
                                }) {
                                    let source = Work::Source(
                                        Endpoint {
                                            owner: input.owner.clone(),
                                            port: port.clone(),
                                        },
                                        None,
                                    );
                                    let next = cache.get(&source).ok_or_else(|| {
                                        error(
                                            "output".into(),
                                            "a produced output has no derived route",
                                        )
                                    })?;
                                    total =
                                        combine(&total, next).ok_or_else(|| proof_limit(owner))?;
                                }
                            }
                            // A child transaction coalesces its own terminal crossings;
                            // two different child transactions never coalesce verdicts.
                            alternatives.extend(total.into_iter().map(|outcome| Outcome {
                                terminal: false,
                                other: (outcome.other + u8::from(outcome.terminal)).min(2),
                                ..outcome
                            }));
                            if alternatives.len() > MAX_GRAPH_LOOP_OUTCOMES {
                                return Err(proof_limit(owner));
                            }
                        }
                    }
                    if joined {
                        alternatives = alternatives
                            .into_iter()
                            .map(|mut outcome| {
                                outcome
                                    .joins
                                    .entry(input.clone())
                                    .and_modify(|count| *count = count.saturating_add(1).min(2))
                                    .or_insert(1);
                                outcome
                            })
                            .collect();
                    }
                    alternatives
                }
            }
        };
        let outcomes = if let Some(nested) = nested {
            nested_outcomes(nested, &outcomes, &cache)?
        } else {
            outcomes
        };
        visiting.remove(&work);
        cache.insert(work, outcomes);
    }
    if cache.get(&root).is_some_and(|outcomes| {
        outcomes
            .iter()
            .any(|outcome| outcome.recurs && outcome.unsafe_effect)
    }) {
        return Err(PlanValidationError::UnsafeLoopEffect {
            owner: owner.path.to_string(),
        });
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
    for outcome in cache.get(&root).into_iter().flatten() {
        for input in outcome.joins.keys() {
            let Some(Handler {
                activation: node::HandlerActivation::Join { ports, .. },
                ..
            }) = input_handler(plan, input)
            else {
                continue;
            };
            if ports.iter().any(|port| {
                outcome.joins.get(&Endpoint {
                    owner: input.owner.clone(),
                    port: port.clone(),
                }) != Some(&1)
            }) {
                return Err(error(
                    "join".into(),
                    "each Loop application must supply exactly one candidate to every join input",
                ));
            }
        }
    }
    Ok(())
}

fn sets(selection: &RouteSelection) -> Vec<&[RouteDestination]> {
    match selection {
        RouteSelection::All(set) => vec![set],
        RouteSelection::Cases(arms) => arms.iter().map(|arm| arm.destinations.as_slice()).collect(),
    }
}

fn dependency(
    destination: &RouteDestination,
    scope: &OwnerPath,
    case: Option<&str>,
) -> Option<Work> {
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
            Some(Work::Source(target.clone(), case.map(String::from)))
        }
        _ => None,
    }
}

fn input_call<'a>(plan: &'a ExecutionPlan, input: &Endpoint) -> Option<(&'a Node, CallSemantics)> {
    let EndpointOwner::Node(path) = &input.owner else {
        return None;
    };
    let node = plan.nodes.iter().find(|node| node.path == *path)?;
    let call = node.connector()?.call_semantics()?;
    (input.port.name() == call.request()).then_some((node, call))
}

fn call_outcomes(
    node: &Node,
    input: &Endpoint,
    call: CallSemantics,
    cache: &BTreeMap<Work, Outcomes>,
) -> Option<Outcomes> {
    let connector = node.connector()?;
    let mut alternatives = Outcomes::new();
    for (case, visibility) in call.terminal_cases() {
        let terminal = Work::Source(
            Endpoint {
                owner: input.owner.clone(),
                port: node::Port::new(call.terminal()).ok()?,
            },
            Some((*case).into()),
        );
        let mut total = cache.get(&terminal)?.clone();
        for port in &node.ports {
            let source = Work::Source(
                Endpoint {
                    owner: input.owner.clone(),
                    port: port.port.clone(),
                },
                None,
            );
            let frames = match call.frame_production(port.port.name()) {
                Some(FrameProduction::ExactlyOneOnExit(on)) if on == *case => {
                    cache.get(&source)?.clone()
                }
                Some(FrameProduction::ZeroOrMore) => {
                    // The finite saturated algebra reaches a fixed point for
                    // any stream length, including zero. Frames never imply
                    // an at-most-one verdict merely because a mock sent one.
                    let one = cache.get(&source)?;
                    let mut repeated = empty();
                    loop {
                        let mut next = combine(&repeated, one)?;
                        next.extend(repeated.iter().cloned());
                        if next.len() > MAX_GRAPH_LOOP_OUTCOMES {
                            return None;
                        }
                        if next == repeated {
                            break repeated;
                        }
                        repeated = next;
                    }
                }
                _ => continue,
            };
            total = combine(&total, &frames)?;
        }
        // These Contracts publish the frames and terminal in one atomic
        // Connector transaction. Only its own boundary crossings coalesce.
        alternatives.extend(total.into_iter().map(|outcome| Outcome {
            terminal: false,
            other: (outcome.other + u8::from(outcome.terminal)).min(2),
            unsafe_effect: outcome.unsafe_effect || !visibility.allows_recurrence(connector.effect),
            ..outcome
        }));
        if alternatives.len() > MAX_GRAPH_LOOP_OUTCOMES {
            return None;
        }
    }
    Some(alternatives)
}

fn input_handler<'a>(plan: &'a ExecutionPlan, input: &Endpoint) -> Option<&'a Handler> {
    let EndpointOwner::Node(path) = &input.owner else {
        return None;
    };
    let slot = Slot::new(plan.nodes.iter().position(|node| node.path == *path)? as u32);
    plan.handlers.iter().find(|handler| {
        handler.owner == slot
            && match &handler.activation {
                node::HandlerActivation::Port(port) => *port == input.port,
                node::HandlerActivation::Join { ports, .. } => ports.contains(&input.port),
                _ => false,
            }
    })
}

fn nested_host<'a>(plan: &'a ExecutionPlan, scope: &Owner, work: &Work) -> Option<&'a Owner> {
    let endpoint = match work {
        Work::Source(endpoint, _) if matches!(endpoint.owner, EndpointOwner::Graph(_)) => endpoint,
        Work::Input(endpoint) => endpoint,
        _ => return None,
    };
    let path = match &endpoint.owner {
        EndpointOwner::Graph(path) => path.clone(),
        EndpointOwner::Node(path) => OwnerPath::from(path),
    };
    plan.owners.iter().find(|owner| {
        owner.path == path
            && owner.path != scope.path
            && owner.path.segments().starts_with(scope.path.segments())
            && owner
                .loop_spec()
                .is_some_and(|spec| spec.entry == endpoint.port)
    })
}

fn expiry_work(owner: &Owner) -> Result<Work, PlanValidationError> {
    let port = owner
        .loop_spec()
        .and_then(|spec| spec.expired.as_ref())
        .map(|expired| expired.port.clone())
        .ok_or_else(|| PlanValidationError::InvalidLoopControl {
            owner: owner.path.to_string(),
            path: "Loop.expired".into(),
            detail: "invalid expiry port",
        })?;
    let endpoint = match owner.kind {
        OwnerKind::Graph => EndpointOwner::Graph(owner.path.clone()),
        OwnerKind::Node(_) => EndpointOwner::Node(
            node::Path::new(owner.path.segments().iter().cloned()).map_err(|_| {
                PlanValidationError::InvalidLoopControl {
                    owner: owner.path.to_string(),
                    path: "owner".into(),
                    detail: "an Executor cannot be the root scope",
                }
            })?,
        ),
    };
    Ok(Work::Source(
        Endpoint {
            owner: endpoint,
            port,
        },
        Some(String::from("LoopExpired")),
    ))
}

fn nested_outcomes(
    owner: &Owner,
    application: &Outcomes,
    cache: &BTreeMap<Work, Outcomes>,
) -> Result<Outcomes, PlanValidationError> {
    let Some(spec) = owner.loop_spec() else {
        return Ok(application.clone());
    };
    let fuel = spec.fuel.as_ref().and_then(|fuel| match fuel.resolved {
        Value::Int(n) if n > 0 => Some(n as u64),
        Value::Uint(n) if n > 0 => Some(n),
        _ => None,
    });
    let mut terminal = Outcomes::new();
    let mut recurrent = Outcomes::new();
    for mut outcome in application.iter().cloned() {
        // Inner join cardinality is proved in its own application, not summed
        // over that run when proving the enclosing application.
        outcome.joins.retain(|input, _| {
            let path = match &input.owner {
                EndpointOwner::Graph(path) => path.segments(),
                EndpointOwner::Node(path) => path.segments(),
            };
            !path.starts_with(owner.path.segments())
        });
        if outcome.retries.remove(&owner.path) {
            recurrent.insert(outcome);
        } else {
            terminal.insert(outcome);
        }
    }
    let mut accumulated = empty();
    let mut result = Outcomes::new();
    let mut iteration = 0;
    loop {
        result.extend(combine(&accumulated, &terminal).ok_or_else(|| proof_limit(owner))?);
        if result.len() > MAX_GRAPH_LOOP_OUTCOMES {
            return Err(proof_limit(owner));
        }
        let next = combine(&accumulated, &recurrent).ok_or_else(|| proof_limit(owner))?;
        iteration += 1;
        if fuel == Some(iteration) || next == accumulated || next.is_empty() {
            if fuel.is_some()
                && !next.is_empty()
                && let Some(expired) = cache.get(&expiry_work(owner)?)
            {
                result.extend(combine(&next, expired).ok_or_else(|| proof_limit(owner))?);
                if result.len() > MAX_GRAPH_LOOP_OUTCOMES {
                    return Err(proof_limit(owner));
                }
            }
            break;
        }
        accumulated = next;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_choices_refuse_before_the_proof_state_explodes() {
        let mut accumulated = empty();
        for index in 0..10 {
            let mut choice = empty();
            let mut selected = empty().pop_first().unwrap();
            selected
                .retries
                .insert(OwnerPath::new([format!("child{index}")]).unwrap());
            choice.insert(selected);
            accumulated = combine(&accumulated, &choice).unwrap();
            assert_eq!(accumulated.len(), 1 << (index + 1));
        }
        let mut choice = empty();
        let mut selected = empty().pop_first().unwrap();
        selected.retries.insert(OwnerPath::new(["extra"]).unwrap());
        choice.insert(selected);
        assert!(combine(&accumulated, &choice).is_none());
        assert_eq!(accumulated.len(), MAX_GRAPH_LOOP_OUTCOMES);
    }
}
