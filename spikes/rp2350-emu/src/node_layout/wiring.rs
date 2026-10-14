//! Measure indexed wiring and the existing composition algorithm together.
//! This experiment converts validated declarations; it is not a new loader.

use super::NodeIndex;
use alloc::{string::String, vec::Vec};
use rastok_core::{
    node::{OwnerPath, Port, PortDirection},
    plan::{self, ExecutionPlan},
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
    scope_chains: Vec<Vec<NodeIndex>>,
}

fn owner(plan: &ExecutionPlan, segments: &[String]) -> NodeIndex {
    NodeIndex(
        u32::try_from(
            plan.nodes
                .iter()
                .position(|owner| owner.path.segments() == segments)
                .unwrap(),
        )
        .unwrap(),
    )
}

fn endpoint(_plan: &ExecutionPlan, value: &plan::Endpoint) -> Endpoint {
    Endpoint {
        node: NodeIndex::from(value.node),
        port: value.port.clone(),
    }
}

fn physical_endpoint(_plan: &ExecutionPlan, value: &plan::Endpoint) -> Endpoint {
    Endpoint {
        node: NodeIndex::from(value.node),
        port: value.port.clone(),
    }
}

fn destination(plan: &ExecutionPlan, value: &plan::Destination) -> Destination {
    match value {
        plan::Destination::Port(value) => Destination::Port(endpoint(plan, value)),
        plan::Destination::Discard => Destination::Discard,
        plan::Destination::LoopCommand(command) => Destination::LoopCommand {
            owner: NodeIndex::from(command.owner),
            kind: command.kind,
        },
        plan::Destination::InstanceStop(command) => Destination::InstanceStop {
            scope: NodeIndex::from(command.scope),
            target: NodeIndex::from(command.owner),
        },
    }
}

fn rule(plan: &ExecutionPlan, value: &plan::RouteRule) -> RouteRule {
    let destinations =
        |set: &[plan::Destination]| set.iter().map(|value| destination(plan, value)).collect();
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

/// Copy the per-Graph reference wiring for the layout experiment.
pub(super) fn inventory(plan: &ExecutionPlan) -> Vec<plan::RouteRule> {
    plan.reference_routes().cloned().collect()
}

fn graph(plan: &ExecutionPlan, rule: &plan::RouteRule) -> usize {
    let source = rule.source.node.index();
    let declaration = &plan.nodes[source];
    // A validated root source is always an input, including implicit ports
    // absent from the declaration table. Nested boundaries are explicit.
    let inner_face = source == 0
        || declaration.kind() == rastok_core::node::Kind::Graph
            && plan
                .graph_ports(rastok_core::node::NodeIndex::new(source as u32))
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

pub(super) fn scope_chains(plan: &ExecutionPlan) -> Vec<Vec<NodeIndex>> {
    // Mirror the validated plan's index for every candidate, including
    // the control. These copies do not measure the production loader's peak.
    let mut chains: Vec<Vec<NodeIndex>> = Vec::with_capacity(plan.nodes.len());
    for (owner, index) in plan.nodes.iter().zip(0..) {
        let mut chain = Vec::with_capacity(owner.path.segments().len() + 1);
        if let Some(parent) = owner.parent {
            chain.extend_from_slice(&chains[parent.index()]);
        }
        chain.push(NodeIndex(index));
        chains.push(chain);
    }
    chains
}

fn edge_scope<'a>(plan: &'a ExecutionPlan, endpoint: &Endpoint) -> &'a OwnerPath {
    let owner = &plan.nodes[endpoint.node.0 as usize];
    if owner.kind() == rastok_core::node::Kind::Graph {
        &owner.path
    } else {
        &plan.nodes[owner.parent.unwrap().index()].path
    }
}

impl Indexes {
    pub(super) fn derive(plan: &ExecutionPlan, inventory: &[plan::RouteRule]) -> Self {
        // Exercise the existing, checked composition algorithm; do not write
        // a second one just for measurement. Its temporary allocations count.
        let composed = plan::compose_routing(
            inventory,
            |index| {
                let node = &plan.nodes[index.index()];
                node.kind() == rastok_core::node::Kind::Graph && !node.path.is_root()
            },
            |source, scope| {
                plan.nodes[source.index()]
                    .path
                    .segments()
                    .starts_with(plan.nodes[scope.index()].path.segments())
            },
        )
        .unwrap();
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
                .enumerate()
                .filter(|(_, node)| node.kind() != rastok_core::node::Kind::Graph)
                .map(|(index, _)| NodeIndex(u32::try_from(index).unwrap()))
                .collect(),
            scope_chains: scope_chains(plan),
        }
    }

    pub(super) fn check(&self, plan: &ExecutionPlan, inventory: &[plan::RouteRule]) {
        let expected = plan::compose_routing(
            inventory,
            |index| {
                let node = &plan.nodes[index.index()];
                node.kind() == rastok_core::node::Kind::Graph && !node.path.is_root()
            },
            |source, scope| {
                plan.nodes[source.index()]
                    .path
                    .segments()
                    .starts_with(plan.nodes[scope.index()].path.segments())
            },
        )
        .unwrap();
        assert_eq!(
            self.routes,
            expected
                .iter()
                .map(|value| rule(plan, value))
                .collect::<Vec<_>>()
        );
        let mut expected: Vec<_> = plan
            .edges()
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
                .enumerate()
                .filter(|(_, node)| node.kind() != rastok_core::node::Kind::Graph)
                .map(|(index, _)| NodeIndex(u32::try_from(index).unwrap()))
                .collect::<Vec<_>>()
        );
        assert_eq!(self.scope_chains, scope_chains(plan));
    }
}
