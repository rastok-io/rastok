//! Presence-join pairing follows the Call Contract and composed routing,
//! not the order in which different Calls happen to complete.

use std::collections::{BTreeMap, BTreeSet};

use crate::connector::{CallSemantics, FrameProduction, outbound_call_semantics};
use crate::diagnostics::{Bag, Diagnostic, Phase};
use crate::node::{HandlerActivation, NodeIndex};
use crate::plan::{AttrSetting, Edge, Endpoint, ValidatedExecutionPlan, closed_case_domain};

pub(super) fn check(plan: &ValidatedExecutionPlan, bag: &mut Bag) {
    for (owner, handler) in plan.handlers() {
        let HandlerActivation::Join {
            ports,
            within_ns,
            last,
        } = &handler.activation
        else {
            continue;
        };
        if plan.nodes[owner.index()].instance.key.is_some()
            && (within_ns.is_some() || last.is_some())
        {
            continue;
        }
        let mut incoming: BTreeMap<NodeIndex, Vec<(&Edge, &Endpoint)>> = BTreeMap::new();
        for edge in plan.edges() {
            if let Some(target) = &edge.to
                && target.node == owner
                && ports.contains(&target.port)
            {
                incoming
                    .entry(edge.from.node)
                    .or_default()
                    .push((edge, target));
            }
        }
        for (source, routes) in incoming {
            let Some(connector) = plan.nodes[source.index()].connector() else {
                continue;
            };
            let mode = match connector.attribute("mode") {
                Some(AttrSetting::Word(word)) => Some(word.as_str()),
                _ => None,
            };
            let Some(call) = outbound_call_semantics(
                &connector.contract,
                connector.roles,
                connector.exchange,
                mode,
            ) else {
                continue;
            };
            if let Some(warning) = unpaired_call(plan, owner, source, call, &routes) {
                bag.push(warning);
            }
        }
    }
}

fn unpaired_call(
    plan: &ValidatedExecutionPlan,
    owner: NodeIndex,
    source: NodeIndex,
    call: CallSemantics,
    routes: &[(&Edge, &Endpoint)],
) -> Option<Diagnostic> {
    let inputs: BTreeSet<_> = routes.iter().map(|(_, target)| &target.port).collect();
    if inputs.len() < 2
        || !routes
            .iter()
            .any(|(edge, _)| edge.from.port.name() == call.terminal())
    {
        return None;
    }
    let (frame, _) = routes
        .iter()
        .find(|(edge, _)| call.frame_production(edge.from.port.name()).is_some())?;
    let node = &plan.nodes[owner.index()].path;
    let connector = &plan.nodes[source.index()].path;
    let diagnostic = |port: &str, case: &str, detail: String| {
        Diagnostic::new(
            super::check::W_JOIN_UNPAIRED,
            Phase::Check,
            format!(
                "`{node}` joins `{connector}.{port}` with `{connector}.{}`, but {detail}. \
                 Key `{node}` by the Call and bound the join",
                call.terminal()
            ),
        )
        .with_field("node", node.to_string())
        .with_field("connector", connector.to_string())
        .with_field("port", port)
        .with_field("case", case)
    };
    if let Some((edge, _)) = routes.iter().find(|(edge, _)| {
        call.frame_production(edge.from.port.name()) == Some(FrameProduction::ZeroOrMore)
    }) {
        let port = edge.from.port.name();
        return Some(diagnostic(
            port,
            "all",
            format!(
                "`{connector}.{port}` carries zero or more frames per Call; \
                 a presence join cannot pair them with one `{}`",
                call.terminal()
            ),
        ));
    }
    // Counts belong to the joined input, not the source port: fan-out and
    // several source ports converging on one input must both be counted.
    for (case, _) in call.terminal_cases() {
        let mut counts: BTreeMap<_, _> = inputs.iter().map(|port| (*port, Some(0))).collect();
        for (edge, target) in routes {
            let count = deliveries_in_case(plan, edge, call, case);
            if let Some(total) = counts.get_mut(&target.port) {
                *total = match (*total, count) {
                    (Some(total), Some(count)) => Some(total + count),
                    _ => None,
                };
            }
        }
        if counts.values().all(|count| *count == Some(0))
            || counts.values().all(|count| *count == Some(1))
        {
            continue;
        }
        let received = counts
            .iter()
            .map(|(port, count)| match count {
                Some(count) => format!("{count} to `{node}.{port}`"),
                None => format!("an unproven number to `{node}.{port}`"),
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Some(diagnostic(
            frame.from.port.name(),
            case,
            format!("`{case}` delivers {received}; a lone delivery can pair with another Call"),
        ));
    }
    None
}

/// `None` means the Contract and route do not prove a fixed count. Filtering
/// a frame by its payload is independent of filtering the Call's terminal.
fn deliveries_in_case(
    plan: &ValidatedExecutionPlan,
    edge: &Edge,
    call: CallSemantics,
    case: &str,
) -> Option<usize> {
    let selected = plan.edge_cases(edge);
    if edge.from.port.name() == call.terminal() {
        return Some(usize::from(
            selected
                .as_ref()
                .is_none_or(|cases| cases.iter().any(|known| known == case)),
        ));
    }
    match call.frame_production(edge.from.port.name())? {
        FrameProduction::Never => Some(0),
        FrameProduction::ZeroOrMore => None,
        FrameProduction::ExactlyOneOn(cases) if !cases.contains(&case) => Some(0),
        FrameProduction::ExactlyOneOn(_) => {
            let covers_all = selected.is_none()
                || plan
                    .resolved_ports()
                    .endpoint_type(&edge.from)
                    .and_then(closed_case_domain)
                    .is_some_and(|domain| selected.as_ref() == Some(&domain));
            covers_all.then_some(1)
        }
    }
}
