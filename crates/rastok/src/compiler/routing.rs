//! Preserve authored boundary crossings and derive flattened static selections.

use super::hir::{EdgeTarget, Graph, RouteDestination, RouteSelection};
use crate::plan::{RouteCompositionError, RouteRule};

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
    rules.sort_by_key(RouteRule::canonical_order_key);
    rules
}

pub(super) fn flatten(graph: &Graph) -> Result<Vec<RouteRule>, Box<RouteCompositionError>> {
    let mut rules = crate::plan::compose_routing(&inventory(graph))?;
    rules.retain(|rule| match &rule.selection {
        RouteSelection::All(destinations) => destinations.iter().any(|destination| {
            matches!(
                destination,
                RouteDestination::LoopCommand(_) | RouteDestination::InstanceStop(_)
            )
        }),
        RouteSelection::Cases(_) => true,
    });
    Ok(rules)
}

pub(super) fn selects(rule: &RouteRule, target: &EdgeTarget) -> bool {
    let target = match target {
        EdgeTarget::Port(endpoint) => RouteDestination::Port(endpoint.clone()),
        EdgeTarget::Discard => RouteDestination::Discard,
    };
    match &rule.selection {
        RouteSelection::All(destinations) => destinations.contains(&target),
        RouteSelection::Cases(arms) => arms.iter().any(|arm| arm.destinations.contains(&target)),
    }
}
