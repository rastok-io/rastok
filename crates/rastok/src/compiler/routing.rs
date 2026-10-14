//! Preserve authored boundary crossings and derive flattened static selections.

use super::hir::{EdgeTarget, Graph, RouteDestination, RouteSelection};
use crate::plan::{RouteCompositionError, RouteRule};

/// Normalize only this Graph's declarations; the HIR used for semantic
/// identity remains unchanged. Additive routes become one selected set.
pub(super) fn wiring(graph: &Graph) -> Vec<RouteRule> {
    use std::collections::BTreeMap;
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
    let mut result: Vec<_> = sources
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
    result.sort_by_key(RouteRule::canonical_order_key);
    result
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
