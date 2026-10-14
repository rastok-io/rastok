//! Preserve authored boundary crossings and derive flattened static selections.

use super::hir::{
    self, EdgeTarget, EndpointOwner, Graph, RouteDestination, RouteRule, RouteSelection,
};
use crate::{node, plan};
use std::collections::{BTreeMap, BTreeSet};

/// Normalize only this Graph's declarations; the HIR used for semantic
/// identity remains unchanged. Additive routes become one selected set.
pub(super) fn wiring(
    graph: &Graph,
    nodes: &BTreeMap<node::OwnerPath, node::NodeIndex>,
) -> Result<Vec<plan::RouteRule>, super::lower::LowerError> {
    let mut sources = BTreeMap::new();
    for rule in &graph.routes {
        let (all, arms) = sources
            .entry(rule.source.clone())
            .or_insert_with(|| (Vec::new(), Vec::new()));
        match &rule.selection {
            RouteSelection::All(destinations) => all.extend(destinations.iter().cloned()),
            RouteSelection::Cases(cases) => arms.extend(cases.iter().cloned()),
        }
    }
    for edge in &graph.edges {
        if graph.routes.iter().any(|rule| rule.source == edge.from) {
            continue;
        }
        let (all, _) = sources
            .entry(edge.from.clone())
            .or_insert_with(|| (Vec::new(), Vec::new()));
        all.push(match &edge.to {
            EdgeTarget::Port(endpoint) => RouteDestination::Port(endpoint.clone()),
            EdgeTarget::Discard => RouteDestination::Discard,
        });
    }
    let result: Vec<_> = sources
        .into_iter()
        .map(|(source, (mut all, mut arms))| {
            let selection = if arms.is_empty() {
                all.sort();
                RouteSelection::All(all)
            } else {
                for arm in &mut arms {
                    arm.destinations.extend(all.iter().cloned());
                    arm.destinations.sort();
                }
                arms.sort_by(|left, right| left.cases.cmp(&right.cases));
                RouteSelection::Cases(arms)
            };
            RouteRule { source, selection }
        })
        .collect();
    // Every referenced path must be in the final Node table, not only among
    // the Nodes already emitted by lowering.
    for owner in owners(&result) {
        let path =
            node::OwnerPath::new(owner.segments()).map_err(|_| super::lower::LowerError {
                owner: owner.segments().join("."),
                detail: "a checked route has no valid owner path",
            })?;
        if !nodes.contains_key(&path) {
            return Err(super::lower::LowerError {
                owner: path.to_string(),
                detail: "a checked route lost its declared Node",
            });
        }
    }
    let indices: BTreeMap<_, _> = nodes
        .iter()
        .map(|(path, index)| (path.segments(), *index))
        .collect();
    let mut result: Vec<_> = result
        .iter()
        .map(|rule| resolve(rule, &|owner| indices[owner.segments()]))
        .collect();
    for rule in &mut result {
        match &mut rule.selection {
            plan::RouteSelection::All(set) => set.sort(),
            plan::RouteSelection::Cases(arms) => {
                for arm in arms {
                    arm.destinations.sort();
                }
            }
        }
    }
    result.sort_by_key(plan::RouteRule::canonical_order_key);
    Ok(result)
}

pub(super) fn inventory(graph: &Graph) -> Vec<RouteRule> {
    fn collect(graph: &Graph, rules: &mut Vec<RouteRule>) {
        rules.extend(graph.routes.iter().cloned());
        for edge in &graph.edges {
            if graph.routes.iter().any(|rule| rule.source == edge.from) {
                continue;
            }
            rules.push(RouteRule {
                source: edge.from.clone(),
                selection: RouteSelection::All(vec![match &edge.to {
                    EdgeTarget::Port(endpoint) => RouteDestination::Port(endpoint.clone()),
                    EdgeTarget::Discard => RouteDestination::Discard,
                }]),
            });
        }
        for child in &graph.graphs {
            collect(child, rules);
        }
    }
    let mut rules = Vec::new();
    collect(graph, &mut rules);
    rules
}

/// Run the same indexed composition proof as package validation, using a
/// local symbol table solely to render source diagnostics.
pub(super) fn flatten(
    graph: &Graph,
) -> Result<(), (Vec<EndpointOwner>, Box<plan::RouteCompositionError>)> {
    let (owners, rules) = indexed(&inventory(graph));
    plan::compose_routing(
        &rules,
        |index| matches!(&owners[index.index()], EndpointOwner::Graph(path) if !path.is_root()),
        |source, scope| {
            owners[source.index()]
                .segments()
                .starts_with(owners[scope.index()].segments())
        },
    )
    .map(|_| ())
    .map_err(|error| (owners, error))
}

fn owners(rules: &[RouteRule]) -> BTreeSet<EndpointOwner> {
    let mut owners = BTreeSet::new();
    for rule in rules {
        owners.insert(rule.source.owner.clone());
        let sets: Vec<_> = match &rule.selection {
            RouteSelection::All(set) => vec![set.as_slice()],
            RouteSelection::Cases(arms) => {
                arms.iter().map(|arm| arm.destinations.as_slice()).collect()
            }
        };
        for destination in sets.into_iter().flatten() {
            match destination {
                RouteDestination::Port(endpoint) => {
                    owners.insert(endpoint.owner.clone());
                }
                RouteDestination::LoopCommand(command) => {
                    owners.insert(command.owner.clone());
                }
                RouteDestination::InstanceStop(target) => {
                    owners.insert(EndpointOwner::Graph(target.scope.clone()));
                    owners.insert(target.owner.clone());
                }
                RouteDestination::Discard => {}
            }
        }
    }
    owners
}

pub(super) fn indexed(rules: &[RouteRule]) -> (Vec<EndpointOwner>, Vec<plan::RouteRule>) {
    let owners: Vec<_> = owners(rules).into_iter().collect();
    let indices: BTreeMap<_, _> = owners
        .iter()
        .cloned()
        .zip(0..)
        .map(|(owner, index)| (owner, node::NodeIndex::new(index)))
        .collect();
    // The map is built from these exact rules, including every command scope.
    let rules = rules
        .iter()
        .map(|rule| resolve(rule, &|owner| indices[owner]))
        .collect();
    (owners, rules)
}

fn resolve(
    rule: &RouteRule,
    index: &impl Fn(&EndpointOwner) -> node::NodeIndex,
) -> plan::RouteRule {
    let endpoint = |endpoint: &hir::Endpoint| plan::Endpoint {
        node: index(&endpoint.owner),
        port: endpoint.port.clone(),
    };
    let destinations = |set: &[RouteDestination]| {
        set.iter()
            .map(|destination| match destination {
                RouteDestination::Port(target) => plan::Destination::Port(endpoint(target)),
                RouteDestination::Discard => plan::Destination::Discard,
                RouteDestination::LoopCommand(command) => {
                    plan::Destination::LoopCommand(plan::LoopCommandTarget {
                        owner: index(&command.owner),
                        kind: command.kind,
                    })
                }
                RouteDestination::InstanceStop(target) => {
                    plan::Destination::InstanceStop(plan::InstanceStopTarget {
                        scope: index(&EndpointOwner::Graph(target.scope.clone())),
                        owner: index(&target.owner),
                    })
                }
            })
            .collect()
    };
    plan::RouteRule {
        source: endpoint(&rule.source),
        selection: match &rule.selection {
            RouteSelection::All(set) => plan::RouteSelection::All(destinations(set)),
            RouteSelection::Cases(arms) => plan::RouteSelection::Cases(
                arms.iter()
                    .map(|arm| plan::RouteArm {
                        cases: arm.cases.clone(),
                        destinations: destinations(&arm.destinations),
                    })
                    .collect(),
            ),
        },
    }
}

pub(super) fn named_endpoint(owners: &[EndpointOwner], endpoint: &plan::Endpoint) -> hir::Endpoint {
    hir::Endpoint {
        owner: owners[endpoint.node.index()].clone(),
        port: endpoint.port.clone(),
    }
}

pub(super) fn named_destination(
    owners: &[EndpointOwner],
    destination: &plan::Destination,
) -> RouteDestination {
    match destination {
        plan::Destination::Port(endpoint) => {
            RouteDestination::Port(named_endpoint(owners, endpoint))
        }
        plan::Destination::Discard => RouteDestination::Discard,
        plan::Destination::LoopCommand(command) => {
            RouteDestination::LoopCommand(hir::LoopCommandTarget {
                owner: owners[command.owner.index()].clone(),
                kind: command.kind,
            })
        }
        plan::Destination::InstanceStop(target) => {
            let scope = match &owners[target.scope.index()] {
                EndpointOwner::Graph(scope) => scope.clone(),
                EndpointOwner::Node(path) => node::OwnerPath::from(path),
            };
            RouteDestination::InstanceStop(Box::new(hir::InstanceStopTarget {
                scope,
                owner: owners[target.owner.index()].clone(),
            }))
        }
    }
}
