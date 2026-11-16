//! Source explanations for an independently refused Operator Loop leaf.
//! The core proof remains authoritative; this pass only describes its result.

use super::{Unit, hir, syntax};
use crate::diagnostics::{Diagnostic, Phase, Span};
use crate::node;
use crate::plan::{Dispatch, DispatchStep, EmitValue, LoopCommand};

pub(super) fn explain(owner: &str, path: &str, hir: &hir::Hir, unit: &Unit) -> Option<Diagnostic> {
    let (entry, ordinal) = path.rsplit_once(".leaf[")?;
    let ordinal: usize = ordinal.strip_suffix(']')?.parse().ok()?;
    let (graph, node) = find_node(&hir.app, owner)?;
    let spec = node.loop_spec.as_ref()?;
    let activation = node::HandlerActivation::Port(spec.entry.clone());
    let handler = node
        .handlers
        .iter()
        .find(|handler| handler.activation == activation)?;
    let Dispatch::Linear(steps) = handler.dispatch.decisions().nth(ordinal)? else {
        return None;
    };
    let emissions: Vec<_> = steps
        .iter()
        .flat_map(DispatchStep::emissions)
        .filter(|(_, value)| !matches!(value, EmitValue::Nothing))
        .map(|(port, _)| port)
        .collect();
    let terminals: Vec<_> = emissions
        .iter()
        .filter(|port| spec.terminals.contains(port))
        .copied()
        .collect();
    let mut verdicts = Vec::new();
    if let Some(port) = terminals.first() {
        // Several terminal emissions coalesce into one Done, as in the proof.
        verdicts.push(if terminals.len() == 1 {
            format!("the terminal output `{port}`")
        } else {
            format!(
                "the terminal outputs {} (one Done)",
                terminals
                    .iter()
                    .map(|p| format!("`{p}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        });
    }
    for step in steps {
        if let DispatchStep::LoopCommand(command) = step {
            verdicts.push(
                match command {
                    LoopCommand::Next(_) => "`Loop.next`",
                    LoopCommand::Again => "`Loop.again`",
                    LoopCommand::Done => "`Loop.done`",
                }
                .to_owned(),
            );
        }
    }
    if steps.iter().any(|step| matches!(step, DispatchStep::Stop)) {
        verdicts.push("`stop`".into());
    }
    let detail = match verdicts.len() {
        1 => return None,
        0 => match emissions.first() {
            Some(port) if discarded(graph, node, port) => format!(
                "selects no verdict: `{port}` is routed to `nothing`, which disposes an output and does not finish a Loop application"
            ),
            Some(port) => {
                let mut terminals = spec.terminals.clone();
                terminals.extend(emissions.iter().map(|port| (*port).clone()));
                terminals.sort();
                terminals.dedup();
                format!("selects no verdict: it writes `{port}`, which is not a terminal output; add `Loop.terminal = [{}]`, or select Next, Again, Done or stop",
                    terminals.iter().map(node::Port::name).collect::<Vec<_>>().join(", "))
            }
            None => "selects no verdict: select Next, Again, Done or stop; `nothing` only disposes an output".into(),
        },
        count => format!("selects {} verdicts: {}", if count == 2 { "two".into() } else { count.to_string() }, verdicts.join(" and ")),
    };
    let declaration = unit.files.iter().find_map(|(_, tree)| {
        tree.decls.iter().find_map(|decl| {
            let syntax::TopDecl::Graph(graph) = decl else {
                return None;
            };
            find_operator(graph, &owner.split('.').collect::<Vec<_>>())
        })
    })?;
    let source = declaration.body.iter().find_map(|item| {
        let syntax::OperatorItem::Handler(handler) = item else {
            return None;
        };
        matches!(&handler.trigger, syntax::Trigger::Port(port) if port.text == entry)
            .then_some(handler)
    })?;
    let dispatch = source.dispatch.as_ref()?;
    let span = if let Some(guard) = source.guards.iter().find_map(|guard| match guard {
        syntax::Guard::Where(expr) => Some(expr.span),
        _ => None,
    }) {
        let Dispatch::OpenMatch { arms, .. } = &handler.dispatch else {
            return None;
        };
        let mut cursor = 1; // the generated one-port `where` decision
        locate(
            &arms.first()?.dispatch,
            dispatch,
            source.span,
            ordinal,
            &mut cursor,
        )
        .or_else(|| (cursor == ordinal).then_some(guard))?
    } else {
        locate(&handler.dispatch, dispatch, source.span, ordinal, &mut 0)?
    };
    Some(
        Diagnostic::new(
            super::check::E_LOOP_CONTROL,
            Phase::Check,
            format!("this arm of Loop `{owner}` {detail}"),
        )
        .with_span(span)
        .with_field("owner", owner)
        .with_field("path", path)
        .with_field("detail", detail),
    )
}

fn find_node<'a>(graph: &'a hir::Graph, owner: &str) -> Option<(&'a hir::Graph, &'a hir::Node)> {
    graph
        .nodes
        .iter()
        .find(|node| node.path.to_string() == owner)
        .map(|node| (graph, node))
        .or_else(|| {
            graph
                .graphs
                .iter()
                .find_map(|child| find_node(child, owner))
        })
}

fn find_operator<'a>(
    graph: &'a syntax::GraphDecl,
    path: &[&str],
) -> Option<&'a syntax::OperatorDecl> {
    let (name, rest) = path.split_first()?;
    graph.body.iter().find_map(|item| match item {
        syntax::GraphItem::Operator(operator) if rest.is_empty() && operator.name.text == *name => {
            Some(operator)
        }
        syntax::GraphItem::Graph(child) if child.name.text == *name => find_operator(child, rest),
        _ => None,
    })
}

fn discarded(graph: &hir::Graph, node: &hir::Node, port: &node::Port) -> bool {
    graph.routes.iter().any(|rule| {
        if rule.source.owner != hir::EndpointOwner::Node(node.path.clone())
            || rule.source.port != *port
        {
            return false;
        }
        let discards = |destinations: &[hir::RouteDestination]| {
            matches!(destinations, [hir::RouteDestination::Discard])
        };
        match &rule.selection {
            hir::RouteSelection::All(destinations) => discards(destinations),
            hir::RouteSelection::Cases(arms) => {
                !arms.is_empty() && arms.iter().all(|arm| discards(&arm.destinations))
            }
        }
    })
}

/// Pair canonical decisions with authored arms. Closed arms may have been
/// reordered and their fallback expanded into a case set; source order cannot
/// be used as the plan's leaf ordinal.
fn locate(
    typed: &Dispatch,
    source: &syntax::Dispatch,
    span: Span,
    wanted: usize,
    cursor: &mut usize,
) -> Option<Span> {
    let current = *cursor;
    *cursor += 1;
    if current == wanted {
        return Some(span);
    }
    let syntax::Dispatch::Match {
        arms: source_arms, ..
    } = source
    else {
        return None;
    };
    match typed {
        Dispatch::Linear(_) => None,
        Dispatch::ClosedMatch { arms, .. } => {
            for arm in arms {
                let source = source_arms
                    .iter()
                    .find(|source| match &source.selector {
                        syntax::MatchSelector::Cases(cases) => {
                            cases.iter().any(|case| arm.cases.contains(&case.text))
                        }
                        _ => false,
                    })
                    .or_else(|| {
                        source_arms.iter().find(|source| {
                            matches!(source.selector, syntax::MatchSelector::Fallback(_))
                        })
                    })?;
                if let Some(span) =
                    locate(&arm.dispatch, &source.dispatch, source.span, wanted, cursor)
                {
                    return Some(span);
                }
            }
            None
        }
        Dispatch::OpenMatch { arms, fallback, .. } => {
            for (arm, source) in
                arms.iter().zip(source_arms.iter().filter(|source| {
                    !matches!(source.selector, syntax::MatchSelector::Fallback(_))
                }))
            {
                if let Some(span) =
                    locate(&arm.dispatch, &source.dispatch, source.span, wanted, cursor)
                {
                    return Some(span);
                }
            }
            let source = source_arms
                .iter()
                .find(|source| matches!(source.selector, syntax::MatchSelector::Fallback(_)))?;
            locate(fallback, &source.dispatch, source.span, wanted, cursor)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refusal(source: &str) -> Diagnostic {
        let registry = crate::connector::contract::ContractCatalog::try_m0().unwrap();
        let bag = crate::compiler::compile(&[source], &registry).unwrap_err();
        bag.diagnostics()
            .iter()
            .find(|d| d.code() == super::super::check::E_LOOP_CONTROL)
            .unwrap_or_else(|| panic!("{bag:?}"))
            .clone()
    }

    #[test]
    fn canonical_closed_arms_and_nested_decisions_keep_authored_spans() {
        for (body, selected) in [
            (
                "on in match data { true => out = data\n false => note = data }",
                "false => note = data",
            ),
            (
                "on in match data { true => match { data => note = data\n _ => out = data }\n false => out = data }",
                "data => note = data",
            ),
            ("on in where data { out = data }", "data"),
        ] {
            let source = format!(
                "graph app : App {{ input in: bool\n output out: bool\n operator count : Loop {{ input in: bool\n output out: bool\n output note: bool\n output err: union {{ LoopExpired {{ last_state: bool }} }}\n Loop.fuel = 2\n {body}\n }}\n in -> count\n count.out -> out\n count.note -> nothing\n count.err -> nothing\n }}"
            );
            let diagnostic = refusal(&source);
            assert!(
                diagnostic.message().contains("selects no verdict"),
                "{diagnostic:?}"
            );
            assert_eq!(source[diagnostic.spans()[0].range()].trim(), selected);
        }
    }

    #[test]
    fn several_terminal_emissions_still_count_as_one_done_verdict() {
        let source = "graph app : App { input in: int\n output out: int\n operator count : Loop { input in: int\n output out: int\n output status: int\n Loop.measure = Loop.state.data\n Loop.terminal = [out, status]\n on in match { data > 0 => { (out, status) = data; Loop.next = data - 1 }\n _ => out = data } }\n in -> count\n count.out -> out\n count.status -> nothing\n }";
        let diagnostic = refusal(source);
        assert!(
            diagnostic.message().contains("selects two verdicts"),
            "{diagnostic:?}"
        );
        assert!(diagnostic.message().contains("(one Done)"));
        assert!(diagnostic.message().contains("Loop.next"));
        let registry = crate::connector::contract::ContractCatalog::try_m0().unwrap();
        let valid = source.replace("; Loop.next = data - 1", "");
        crate::compiler::compile(&[&valid], &registry).unwrap();
    }

    #[test]
    fn an_arm_in_a_nested_graph_keeps_its_source_position() {
        let source = "graph app : App { input in: int\n output out: int\n graph inner { input in: int\n output out: int\n operator count : Loop { input in: int\n output out: int\n output note: int\n Loop.measure = Loop.state.data\n on in match { data > 0 => note = data\n _ => out = data } }\n in -> count\n count.out -> out\n count.note -> nothing\n }\n in -> inner\n inner.out -> out\n }";
        let diagnostic = refusal(source);
        assert!(diagnostic.message().contains("Loop `inner.count`"));
        assert_eq!(
            source[diagnostic.spans()[0].range()].trim(),
            "data > 0 => note = data"
        );
    }
}
