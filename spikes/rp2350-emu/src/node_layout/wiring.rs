//! Measure indexed wiring and the existing composition algorithm together.
//! This experiment converts validated declarations; it is not a new loader.

use super::NodeIndex;
use alloc::{collections::BTreeMap, string::String, vec::Vec};
use rastok_core::{
    node::{OwnerPath, Port, PortDirection},
    plan::{self, ExecutionPlan, OwnerKind},
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Endpoint {
    node: NodeIndex,
    port: Port,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Destination {
    Port(Endpoint),
    Discard,
    LoopCommand {
        owner: NodeIndex,
        kind: plan::LoopCommandKind,
    },
    InstanceStop {
        scope: NodeIndex,
        target: NodeIndex,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Selection {
    All(Vec<Destination>),
    Cases(Vec<RouteArm>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RouteArm {
    cases: Vec<String>,
    destinations: Vec<Destination>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RouteRule {
    source: Endpoint,
    selection: Selection,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Edge {
    owner: NodeIndex,
    from: Endpoint,
    to: Endpoint,
}

/// Derived data is kept alive with the declarations while sampling. Keeping
/// a flattened rule and an edge table is deliberately conservative: it does
/// not assume a future optimized lookup representation saves any bytes.
pub(super) struct Indexes {
    routes: Vec<RouteRule>,
    edges: Vec<Edge>,
    execution_order: Vec<NodeIndex>,
    scope_chains: Vec<Option<Vec<NodeIndex>>>,
}

fn owner(plan: &ExecutionPlan, segments: &[String]) -> NodeIndex {
    NodeIndex(
        u32::try_from(
            plan.owners
                .iter()
                .position(|owner| owner.path.segments() == segments)
                .unwrap(),
        )
        .unwrap(),
    )
}

fn endpoint_owner(plan: &ExecutionPlan, value: &plan::EndpointOwner) -> NodeIndex {
    owner(
        plan,
        match value {
            plan::EndpointOwner::Node(path) => path.segments(),
            plan::EndpointOwner::Graph(path) => path.segments(),
        },
    )
}

fn endpoint(plan: &ExecutionPlan, value: &plan::Endpoint) -> Endpoint {
    Endpoint {
        node: endpoint_owner(plan, &value.owner),
        port: value.port.clone(),
    }
}

fn physical_endpoint(plan: &ExecutionPlan, value: &plan::EdgeEnd) -> Endpoint {
    Endpoint {
        node: value.node.map_or(NodeIndex(0), |slot| {
            NodeIndex::from(plan.nodes[slot.index()].owner)
        }),
        port: value.port.clone(),
    }
}

fn destination(plan: &ExecutionPlan, value: &plan::RouteDestination) -> Destination {
    match value {
        plan::RouteDestination::Port(value) => Destination::Port(endpoint(plan, value)),
        plan::RouteDestination::Discard => Destination::Discard,
        plan::RouteDestination::LoopCommand(command) => Destination::LoopCommand {
            owner: endpoint_owner(plan, &command.owner),
            kind: command.kind,
        },
        plan::RouteDestination::InstanceStop(command) => Destination::InstanceStop {
            scope: owner(plan, command.scope.segments()),
            target: endpoint_owner(plan, &command.owner),
        },
    }
}

fn rule(plan: &ExecutionPlan, value: &plan::RouteRule) -> RouteRule {
    let destinations =
        |set: &[plan::RouteDestination]| set.iter().map(|value| destination(plan, value)).collect();
    RouteRule {
        source: endpoint(plan, &value.source),
        selection: match &value.selection {
            plan::RouteSelection::All(set) => Selection::All(destinations(set)),
            plan::RouteSelection::Cases(arms) => Selection::Cases(
                arms.iter()
                    .map(|arm| RouteArm {
                        cases: arm.cases.clone(),
                        destinations: destinations(&arm.destinations),
                    })
                    .collect(),
            ),
        },
    }
}

/// Recover the current reference wiring for a flat plan, where no separate
/// boundary inventory is transported. Case/command rules take precedence over
/// the physical pairs that are their already-validated redundant projection.
pub(super) fn inventory(plan: &ExecutionPlan) -> Vec<plan::RouteRule> {
    if !plan.boundary_routes.is_empty() {
        return plan.boundary_routes.clone();
    }
    let endpoint = |end: &plan::EdgeEnd| plan::Endpoint {
        owner: end.node.map_or_else(
            || plan::EndpointOwner::Graph(OwnerPath::root()),
            |slot| plan::EndpointOwner::Node(plan.nodes[slot.index()].path.clone()),
        ),
        port: end.port.clone(),
    };
    let mut rules = plan.routes.clone();
    let mut ordinary = BTreeMap::<plan::Endpoint, Vec<plan::RouteDestination>>::new();
    for edge in &plan.edges {
        let source = endpoint(&edge.from);
        if !plan.routes.iter().any(|rule| rule.source == source) {
            ordinary.entry(source).or_default().push(
                edge.to
                    .as_ref()
                    .map_or(plan::RouteDestination::Discard, |to| {
                        plan::RouteDestination::Port(endpoint(to))
                    }),
            );
        }
    }
    rules.extend(ordinary.into_iter().map(|(source, mut set)| {
        set.sort();
        plan::RouteRule {
            source,
            selection: plan::RouteSelection::All(set),
        }
    }));
    rules.sort_by_key(plan::RouteRule::canonical_order_key);
    rules
}

fn graph(plan: &ExecutionPlan, rule: &plan::RouteRule) -> usize {
    let source = endpoint_owner(plan, &rule.source.owner).0 as usize;
    let declaration = &plan.owners[source];
    // A validated root source is always an input, including implicit ports
    // absent from the declaration table. Nested boundaries are explicit.
    let inner_face = source == 0
        || declaration.kind == OwnerKind::Graph
            && plan
                .graph_ports(rastok_core::node::OwnerSlot::new(source as u32))
                .unwrap()
                .iter()
                .any(|port| {
                    port.port == rule.source.port && port.direction == PortDirection::Input
                });
    if inner_face {
        source
    } else {
        declaration.parent.unwrap().index()
    }
}

pub(super) fn reference(
    plan: &ExecutionPlan,
    graph_index: usize,
    inventory: &[plan::RouteRule],
) -> Vec<RouteRule> {
    let count = inventory
        .iter()
        .filter(|rule| graph(plan, rule) == graph_index)
        .count();
    let mut rules = Vec::with_capacity(count);
    rules.extend(
        inventory
            .iter()
            .filter(|rule| graph(plan, rule) == graph_index)
            .map(|value| rule(plan, value)),
    );
    rules
}

pub(super) fn scope_chains(plan: &ExecutionPlan) -> Vec<Option<Vec<NodeIndex>>> {
    // Reproduce the current VM allocation policy, including spare capacity.
    // This experiment does not claim a saving from a different chain cache.
    let mut chains: Vec<Option<Vec<NodeIndex>>> = Vec::with_capacity(plan.owners.len());
    for (index, owner) in plan.owners.iter().enumerate() {
        let mut chain = match owner.parent {
            None => Some(Vec::new()),
            Some(parent) => chains[parent.index()].clone(),
        };
        if let Some(chain) = &mut chain {
            chain.push(NodeIndex(u32::try_from(index).unwrap()));
        }
        chains.push(chain);
    }
    chains
}

fn edge_scope<'a>(plan: &'a ExecutionPlan, endpoint: &Endpoint) -> &'a OwnerPath {
    let owner = &plan.owners[endpoint.node.0 as usize];
    match owner.kind {
        OwnerKind::Graph => &owner.path,
        OwnerKind::Node(_) => &plan.owners[owner.parent.unwrap().index()].path,
    }
}

impl Indexes {
    pub(super) fn derive(plan: &ExecutionPlan, inventory: &[plan::RouteRule]) -> Self {
        // Exercise the existing, checked composition algorithm; do not write
        // a second one just for measurement. Its temporary allocations count.
        let composed = plan::compose_routing(inventory).unwrap();
        let routes: Vec<_> = composed.iter().map(|value| rule(plan, value)).collect();
        drop(composed);
        let mut edges = Vec::new();
        for route in &routes {
            let destinations: Vec<_> = match &route.selection {
                Selection::All(set) => set.iter().collect(),
                Selection::Cases(arms) => arms.iter().flat_map(|arm| &arm.destinations).collect(),
            };
            for destination in destinations {
                if let Destination::Port(to) = destination {
                    let scope =
                        edge_scope(plan, &route.source).common_ancestor(edge_scope(plan, to));
                    let edge = Edge {
                        owner: owner(plan, scope.segments()),
                        from: route.source.clone(),
                        to: to.clone(),
                    };
                    if !edges.contains(&edge) {
                        edges.push(edge);
                    }
                }
            }
        }
        edges.sort();
        Self {
            routes,
            edges,
            execution_order: plan
                .nodes
                .iter()
                .map(|node| NodeIndex::from(node.owner))
                .collect(),
            scope_chains: scope_chains(plan),
        }
    }

    pub(super) fn check(&self, plan: &ExecutionPlan, inventory: &[plan::RouteRule]) {
        let expected = plan::compose_routing(inventory).unwrap();
        assert_eq!(
            self.routes,
            expected
                .iter()
                .map(|value| rule(plan, value))
                .collect::<Vec<_>>()
        );
        let mut expected: Vec<_> = plan
            .edges
            .iter()
            .filter_map(|edge| {
                Some(Edge {
                    owner: NodeIndex::from(edge.owner),
                    from: physical_endpoint(plan, &edge.from),
                    to: physical_endpoint(plan, edge.to.as_ref()?),
                })
            })
            .collect();
        expected.sort();
        assert_eq!(
            self.edges, expected,
            "composition changed physical deliveries"
        );
        assert_eq!(
            self.execution_order,
            plan.nodes
                .iter()
                .map(|node| NodeIndex::from(node.owner))
                .collect::<Vec<_>>()
        );
        assert_eq!(self.scope_chains, scope_chains(plan));
    }
}
