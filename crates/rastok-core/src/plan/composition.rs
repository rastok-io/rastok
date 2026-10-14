//! Compose static routing through transparent Graph boundaries without
//! erasing the inventory that proves each boundary's dispositions.

use crate::node;
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::{
    EdgeEnd, Endpoint, EndpointOwner, ExecutionPlan, NodeIndex, ResolvedPortTable, RouteArm,
    RouteDestination, RouteErrorKind, RouteRule, RouteSelection, RouteValidationError,
};
use crate::datatype::DataType;
use crate::node::PortDirection;

/// A composition refusal retains the exact source and selected case; source
/// diagnostics do not have to recover semantic fields from formatted text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteCompositionError {
    /// Source whose disposition paths contradict each other.
    pub source: Endpoint,
    /// Selected closed case, or every value for unconditional routing.
    pub case: Option<String>,
    /// The violated composition invariant.
    pub kind: RouteErrorKind,
}

impl From<RouteCompositionError> for RouteValidationError {
    fn from(error: RouteCompositionError) -> Self {
        Self {
            path: format!(
                "composition {:?}.{} case {:?}",
                error.source.owner, error.source.port, error.case
            ),
            kind: error.kind,
        }
    }
}

impl From<Box<RouteCompositionError>> for RouteValidationError {
    fn from(error: Box<RouteCompositionError>) -> Self {
        Self::from(*error)
    }
}

impl core::fmt::Display for RouteCompositionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        RouteValidationError::from(self.clone()).fmt(f)
    }
}

impl core::error::Error for RouteCompositionError {}

/// Build runtime indexes from the sole authored wiring. No supplied edge or
/// composed selection is trusted, even when validating a clone of an old plan.
pub(super) fn derive(plan: &mut ExecutionPlan) -> Result<(), super::PlanValidationError> {
    let inventory: Vec<_> = plan.reference_routes().cloned().collect();
    let mut routes = compose_routing(&inventory).map_err(RouteValidationError::from)?;
    let endpoint = |value: &Endpoint| -> Result<EdgeEnd, super::PlanValidationError> {
        let index = plan
            .nodes
            .iter()
            .position(|node| match &value.owner {
                EndpointOwner::Node(path) => node.path == *path && node.kind() != node::Kind::Graph,
                EndpointOwner::Graph(path) => {
                    node.path == *path && node.kind() == node::Kind::Graph
                }
            })
            .ok_or_else(|| failure("Graph wiring", RouteErrorKind::UnknownEndpoint))?;
        Ok(EdgeEnd {
            node: NodeIndex::new(index as u32),
            port: value.port.clone(),
        })
    };
    let scope = |end: &EdgeEnd| {
        let node = &plan.nodes[end.node.index()];
        if node.kind() == node::Kind::Graph {
            node.path.clone()
        } else {
            node.path.parent().unwrap_or_else(node::OwnerPath::root)
        }
    };
    let mut edges = Vec::new();
    for rule in &routes {
        let from = endpoint(&rule.source)?;
        for destination in destinations(&rule.selection) {
            let to = match destination {
                RouteDestination::Port(value) => Some(endpoint(value)?),
                RouteDestination::Discard => None,
                RouteDestination::LoopCommand(_) | RouteDestination::InstanceStop(_) => continue,
            };
            let owner_path = to.as_ref().map_or_else(
                || scope(&from),
                |to| scope(&from).common_ancestor(&scope(to)),
            );
            let owner = plan
                .nodes
                .iter()
                .position(|node| node.path == owner_path && node.kind() == node::Kind::Graph)
                .ok_or_else(|| failure("Graph wiring scope", RouteErrorKind::UnknownEndpoint))?;
            edges.push(super::Edge {
                owner: NodeIndex::new(owner as u32),
                from: from.clone(),
                to,
            });
        }
    }
    edges.sort_by_key(|edge| plan.edge_text(edge));
    edges.dedup();
    routes.retain(|rule| match &rule.selection {
        RouteSelection::All(set) => set.iter().any(|destination| {
            matches!(
                destination,
                RouteDestination::LoopCommand(_) | RouteDestination::InstanceStop(_)
            )
        }),
        RouteSelection::Cases(_) => true,
    });
    plan.edges = edges;
    plan.routes = routes;
    Ok(())
}

pub(super) fn validate(
    plan: &ExecutionPlan,
    ports: &ResolvedPortTable,
) -> Result<(), super::PlanValidationError> {
    for owner in &plan.nodes {
        let Some(graph) = owner.graph() else { continue };
        let mut sources = BTreeSet::new();
        for pair in graph.wiring.windows(2) {
            if pair[0].canonical_order_key() >= pair[1].canonical_order_key() {
                return Err(super::PlanValidationError::NonCanonicalOrder {
                    collection: "Graph wiring",
                });
            }
        }
        super::validate_routing(&graph.wiring, |endpoint| {
            port(plan, ports, endpoint).and_then(|(_, ty)| ty.cloned())
        })?;
        for rule in &graph.wiring {
            let path = format!("Graph {} wiring", owner.path);
            if !sources.insert(&rule.source) {
                return Err(super::PlanValidationError::NonCanonicalOrder {
                    collection: "Graph wiring sources",
                });
            }
            let scoped = |endpoint: &Endpoint, source: bool| -> bool {
                let Some((direction, _)) = port(plan, ports, endpoint) else {
                    return false;
                };
                let endpoint_path = match &endpoint.owner {
                    EndpointOwner::Node(path) => path.segments(),
                    EndpointOwner::Graph(path) => path.segments(),
                };
                let own_boundary =
                    matches!(&endpoint.owner, EndpointOwner::Graph(path) if path == &owner.path);
                endpoint_path.starts_with(owner.path.segments())
                    && direction
                        == if source == own_boundary {
                            PortDirection::Input
                        } else {
                            PortDirection::Output
                        }
            };
            if !scoped(&rule.source, true) {
                return Err(failure(&path, RouteErrorKind::UnknownEndpoint).into());
            }
            for destination in destinations(&rule.selection) {
                match destination {
                    RouteDestination::InstanceStop(target) => {
                        if target.scope != owner.path
                            || !super::routing_validation::valid_instance_stop_target(plan, target)
                        {
                            return Err(
                                failure(&path, RouteErrorKind::InvalidInstanceCommand).into()
                            );
                        }
                    }
                    RouteDestination::LoopCommand(command) => {
                        let valid = match (&command.owner, &rule.source.owner) {
                            (EndpointOwner::Graph(target), source) => {
                                let source = match source {
                                    EndpointOwner::Graph(path) => path.segments(),
                                    EndpointOwner::Node(path) => path.segments(),
                                };
                                source.starts_with(target.segments())
                                    && plan.nodes.iter().any(|candidate| {
                                        candidate.path == *target && candidate.loop_spec().is_some()
                                    })
                            }
                            (EndpointOwner::Node(target), EndpointOwner::Node(source))
                                if target == source =>
                            {
                                plan.nodes
                                    .iter()
                                    .find(|node| node.path == *target)
                                    .is_some_and(|node| {
                                        node.loop_spec().is_some()
                                            && node
                                                .connector()
                                                .and_then(super::ConnectorSpec::call_semantics)
                                                .is_some()
                                    })
                            }
                            _ => false,
                        };
                        if !valid {
                            return Err(
                                failure(&path, RouteErrorKind::InvalidManagerCommand).into()
                            );
                        }
                    }
                    RouteDestination::Port(endpoint) if !scoped(endpoint, false) => {
                        return Err(failure(&path, RouteErrorKind::UnknownEndpoint).into());
                    }
                    RouteDestination::Port(_) | RouteDestination::Discard => {}
                }
            }
        }
    }

    let live = super::productibility::live_owners(plan);
    for source in plan
        .reference_routes()
        .map(|rule| &rule.source)
        .filter(|source| concrete(source))
    {
        if let EndpointOwner::Node(path) = &source.owner {
            let Some((index, _)) = plan
                .nodes
                .iter()
                .enumerate()
                .find(|(_, node)| node.path == *path)
            else {
                return Err(failure("Graph wiring", RouteErrorKind::UnknownEndpoint).into());
            };
            if !live[index]
                || !super::productibility::may_produce(
                    plan,
                    NodeIndex::new(index as u32),
                    &source.port,
                )
            {
                continue;
            }
        }
        let (_, Some(ty)) = port(plan, ports, source)
            .ok_or_else(|| failure("Graph wiring", RouteErrorKind::UnknownEndpoint))?
        else {
            return Err(failure("Graph wiring", RouteErrorKind::UnresolvedCaseSource).into());
        };
        let cases = super::closed_case_domain(ty)
            .map_or_else(|| vec![None], |cases| cases.into_iter().map(Some).collect());
        for case in cases {
            let selected = case.as_ref().and_then(|case| {
                super::routing::selected_case_type(ty, core::slice::from_ref(case))
            });
            check_crossings(
                plan,
                ports,
                source,
                case.as_deref(),
                selected.as_ref().unwrap_or(ty),
            )?;
        }
    }
    Ok(())
}

fn failure(path: &str, kind: RouteErrorKind) -> RouteValidationError {
    RouteValidationError {
        path: path.into(),
        kind,
    }
}

fn destinations(selection: &RouteSelection) -> Vec<&RouteDestination> {
    match selection {
        RouteSelection::All(set) => set.iter().collect(),
        RouteSelection::Cases(arms) => arms.iter().flat_map(|arm| &arm.destinations).collect(),
    }
}

fn port<'a>(
    plan: &'a ExecutionPlan,
    ports: &'a ResolvedPortTable,
    endpoint: &Endpoint,
) -> Option<(PortDirection, Option<&'a DataType>)> {
    match &endpoint.owner {
        EndpointOwner::Node(path) => {
            let index = plan.nodes.iter().position(|node| node.path == *path)?;
            ports
                .node(NodeIndex::new(u32::try_from(index).ok()?))?
                .iter()
                .find(|port| port.port() == &endpoint.port)
                .map(|port| (port.direction(), Some(port.ty())))
        }
        EndpointOwner::Graph(path) if path.is_root() => ports
            .boundary()
            .iter()
            .find(|port| port.port() == &endpoint.port)
            .map(|port| (port.direction(), Some(port.ty()))),
        EndpointOwner::Graph(path) => {
            let index = plan
                .nodes
                .iter()
                .position(|owner| owner.path == *path && owner.kind() == node::Kind::Graph)?;
            plan.graph_ports(node::NodeIndex::new(u32::try_from(index).ok()?))?
                .iter()
                .find(|port| port.port == endpoint.port)
                .map(|port| (port.direction, port.ty.as_ref()))
        }
    }
}

fn check_crossings(
    plan: &ExecutionPlan,
    ports: &ResolvedPortTable,
    source: &Endpoint,
    case: Option<&str>,
    ty: &DataType,
) -> Result<(), RouteValidationError> {
    let mut pending = vec![source];
    let mut visited = BTreeSet::new();
    while let Some(source) = pending.pop() {
        if !visited.insert(source) {
            continue;
        }
        let mut has_disposition = false;
        for rule in plan
            .reference_routes()
            .filter(|rule| rule.source == *source)
        {
            let selected = match &rule.selection {
                RouteSelection::All(set) => Some(set),
                RouteSelection::Cases(arms) => arms
                    .iter()
                    .find(|arm| arm.cases.iter().any(|member| Some(member.as_str()) == case))
                    .map(|arm| &arm.destinations),
            };
            for destination in selected.into_iter().flatten() {
                has_disposition = true;
                if let RouteDestination::Port(target) = destination {
                    if let Some((_, Some(pin))) = port(plan, ports, target)
                        && !ty.is_assignable_to(pin)
                    {
                        return Err(failure(
                            &format!("boundary {:?}.{}", target.owner, target.port),
                            RouteErrorKind::BoundaryTypeMismatch,
                        ));
                    }
                    if !concrete(target) {
                        pending.push(target);
                    }
                }
            }
        }
        if !has_disposition
            && matches!(&source.owner, EndpointOwner::Graph(path) if !path.is_root())
            && port(plan, ports, source)
                .is_some_and(|(direction, _)| direction == PortDirection::Output)
        {
            return Err(failure(
                &format!(
                    "boundary {:?}.{} case {:?}",
                    source.owner, source.port, case
                ),
                RouteErrorKind::EmptyDestinations,
            ));
        }
    }
    Ok(())
}

/// Derive ordinary source dispositions through an unflattened inventory.
/// Values are not transformed: the same closed case selects every crossing.
/// The caller establishes types, endpoint directions and source productivity;
/// this function proves composition, uniqueness and explicit-discard isolation.
/// Missing boundary continuations remain empty, so a separate productivity
/// proof can exempt an entirely unreachable Graph without inventing a sink.
///
/// # Errors
/// A boundary cycle, duplicate selected destination, or a composed selection
/// mixing `nothing` with another destination. Instance closure also requires
/// the actual emitter to belong to its lexical scope. No declarations are repaired.
pub fn compose_routing(rules: &[RouteRule]) -> Result<Vec<RouteRule>, Box<RouteCompositionError>> {
    let mut inventory = BTreeMap::<&Endpoint, Vec<&RouteSelection>>::new();
    for rule in rules {
        inventory
            .entry(&rule.source)
            .or_default()
            .push(&rule.selection);
    }
    // The spatial DAG is a property of the whole inventory, including a
    // currently unreachable component and the union of its case routes.
    let mut finished = BTreeSet::new();
    let mut active = BTreeSet::new();
    for source in inventory.keys().copied() {
        let mut pending = vec![(source, false)];
        while let Some((endpoint, leaving)) = pending.pop() {
            if leaving {
                active.remove(endpoint);
                finished.insert(endpoint);
            } else if !finished.contains(endpoint) {
                if !active.insert(endpoint) {
                    return Err(Box::new(RouteCompositionError {
                        source: endpoint.clone(),
                        case: None,
                        kind: RouteErrorKind::BoundaryCycle,
                    }));
                }
                pending.push((endpoint, true));
                for selection in inventory.get(endpoint).into_iter().flatten() {
                    pending.extend(
                        destinations(selection)
                            .into_iter()
                            .filter_map(boundary)
                            .map(|target| (target, false)),
                    );
                }
            }
        }
    }
    let mut result = Vec::new();
    for source in inventory.keys().copied().filter(|source| concrete(source)) {
        let mut cases = BTreeSet::new();
        let mut pending = vec![source];
        let mut visited = BTreeSet::new();
        while let Some(endpoint) = pending.pop() {
            if !visited.insert(endpoint) {
                continue;
            }
            for selection in inventory.get(endpoint).into_iter().flatten() {
                match selection {
                    RouteSelection::All(destinations) => {
                        pending.extend(destinations.iter().filter_map(boundary));
                    }
                    RouteSelection::Cases(arms) => {
                        for arm in arms {
                            cases.extend(arm.cases.iter().cloned());
                            pending.extend(arm.destinations.iter().filter_map(boundary));
                        }
                    }
                }
            }
        }
        let selected_cases = if cases.is_empty() {
            vec![None]
        } else {
            cases.into_iter().map(Some).collect()
        };
        let mut arms = Vec::new();
        for case in selected_cases {
            let mut destinations = BTreeSet::new();
            resolve(source, case.as_deref(), &inventory, &mut destinations).map_err(|kind| {
                Box::new(RouteCompositionError {
                    source: source.clone(),
                    case: case.clone(),
                    kind,
                })
            })?;
            // Transparent inputs do not establish an emitting instance in the
            // crossed Graph. Closure can only follow the emitter's retained
            // ancestry, never select a descendant from this envelope's key.
            let source_path = match &source.owner {
                EndpointOwner::Graph(path) => path.segments(),
                EndpointOwner::Node(path) => path.segments(),
            };
            if destinations.iter().any(|destination| {
                matches!(destination,
                RouteDestination::InstanceStop(target)
                    if !source_path.starts_with(target.scope.segments()))
            }) {
                return Err(Box::new(RouteCompositionError {
                    source: source.clone(),
                    case,
                    kind: RouteErrorKind::InstanceCommandScope,
                }));
            }
            if destinations.len() > 1 && destinations.contains(&RouteDestination::Discard) {
                return Err(Box::new(RouteCompositionError {
                    source: source.clone(),
                    case,
                    kind: RouteErrorKind::MixedDiscard,
                }));
            }
            let destinations = destinations.into_iter().collect();
            if let Some(case) = case {
                arms.push(RouteArm {
                    cases: vec![case],
                    destinations,
                });
            } else {
                result.push(RouteRule {
                    source: source.clone(),
                    selection: RouteSelection::All(destinations),
                });
            }
        }
        if !arms.is_empty() {
            result.push(RouteRule {
                source: source.clone(),
                selection: RouteSelection::Cases(arms),
            });
        }
    }
    result.sort_by_key(RouteRule::canonical_order_key);
    Ok(result)
}

fn concrete(endpoint: &Endpoint) -> bool {
    matches!(&endpoint.owner, EndpointOwner::Node(_))
        || matches!(&endpoint.owner, EndpointOwner::Graph(path) if path.is_root())
}

fn boundary(destination: &RouteDestination) -> Option<&Endpoint> {
    match destination {
        RouteDestination::Port(endpoint) if !concrete(endpoint) => Some(endpoint),
        _ => None,
    }
}

fn resolve(
    source: &Endpoint,
    case: Option<&str>,
    inventory: &BTreeMap<&Endpoint, Vec<&RouteSelection>>,
    output: &mut BTreeSet<RouteDestination>,
) -> Result<(), RouteErrorKind> {
    // Heap work, not call-stack depth, bounds an untrusted boundary chain.
    // Exit frames retain path-local visitation: a shared seam is revisited
    // through a sibling so duplicate final deliveries are still diagnosed.
    let mut pending = vec![(source, false)];
    let mut visiting = BTreeSet::new();
    while let Some((source, leaving)) = pending.pop() {
        if leaving {
            visiting.remove(source);
            continue;
        }
        if !visiting.insert(source) {
            return Err(RouteErrorKind::BoundaryCycle);
        }
        pending.push((source, true));
        for selection in inventory.get(source).into_iter().flatten() {
            let destinations = match selection {
                RouteSelection::All(destinations) => Some(destinations),
                RouteSelection::Cases(arms) => arms
                    .iter()
                    .find(|arm| arm.cases.iter().any(|name| Some(name.as_str()) == case))
                    .map(|arm| &arm.destinations),
            };
            for destination in destinations.into_iter().flatten() {
                if let Some(endpoint) = boundary(destination) {
                    pending.push((endpoint, false));
                } else if !output.insert(destination.clone()) {
                    return Err(RouteErrorKind::DuplicateDestination(destination.clone()));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::Port;

    fn graph(name: &str, port: &str) -> Endpoint {
        Endpoint {
            owner: EndpointOwner::Graph(if name.is_empty() {
                node::OwnerPath::root()
            } else {
                node::OwnerPath::new([name]).unwrap()
            }),
            port: Port::new(port).unwrap(),
        }
    }

    fn link(source: Endpoint, target: Endpoint) -> RouteRule {
        RouteRule {
            source,
            selection: RouteSelection::All(vec![RouteDestination::Port(target)]),
        }
    }

    #[test]
    fn deep_boundary_chains_use_explicit_work_not_recursive_stack_frames() {
        let input = graph("", "in");
        let output = graph("", "out");
        let mut rules = vec![link(input.clone(), graph("g0", "in"))];
        for index in 0..2048 {
            rules.push(link(
                graph(&format!("g{index}"), "in"),
                graph(&format!("g{}", index + 1), "in"),
            ));
        }
        rules.push(link(graph("g2048", "in"), output.clone()));
        assert_eq!(compose_routing(&rules).unwrap(), vec![link(input, output)]);
    }

    #[test]
    fn even_an_unreachable_boundary_cycle_refuses() {
        let rules = vec![
            link(graph("left", "in"), graph("right", "in")),
            link(graph("right", "in"), graph("left", "in")),
        ];
        assert_eq!(
            compose_routing(&rules).unwrap_err().kind,
            RouteErrorKind::BoundaryCycle
        );
    }
}
