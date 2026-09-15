//! Re-derive one outbound Call application's terminal and command guarantees.

use alloc::format;
use alloc::string::ToString;

use super::*;
use crate::connector::{CallCommandError, FrameProduction};

pub(super) fn validate(
    plan: &ExecutionPlan,
    slot: Slot,
    ports: &ResolvedPortTable,
) -> Result<(), PlanValidationError> {
    let node = &plan.nodes[slot.index()];
    let (NodeExecution::Connector(connector), Some(spec)) =
        (&node.execution, plan.owners[node.owner.index()].loop_spec())
    else {
        return Ok(());
    };
    let error = |path: &str, detail| PlanValidationError::InvalidLoopControl {
        owner: node.path.to_string(),
        path: path.to_string(),
        detail,
    };
    let call = connector.call_semantics().ok_or_else(|| {
        error(
            "host",
            "Loop requires a resolved finite outbound Call profile",
        )
    })?;
    if spec.entry.name() != call.request() || !spec.terminals.is_empty() {
        return Err(error(
            "entry",
            "a Call Loop uses its Contract request and terminal, not authored terminal ports",
        ));
    }
    if !matches!(connector.attribute("timeout"), Some(AttrSetting::Value(Value::Duration(ns))) if *ns > 0)
    {
        return Err(error(
            "timeout",
            "an outbound Call in Loop needs a positive plan-constant timeout",
        ));
    }
    if spec.fuel.is_none() || spec.proof.is_some() {
        return Err(error(
            "fuel",
            "a Connector-dependent recurrence requires explicit fuel, not a measure proof",
        ));
    }
    let owner = EndpointOwner::Node(node.path.clone());
    let rules = || plan.routes.iter().filter(|rule| rule.source.owner == owner);
    for rule in rules() {
        let sets: Vec<_> = match &rule.selection {
            RouteSelection::All(destinations) => alloc::vec![(None, destinations.as_slice())],
            RouteSelection::Cases(arms) => arms
                .iter()
                .map(|arm| (Some(arm.cases.as_slice()), arm.destinations.as_slice()))
                .collect(),
        };
        for (cases, destinations) in sets {
            for destination in destinations {
                let RouteDestination::LoopCommand(command) = destination else {
                    continue;
                };
                let path = format!("{} -> {:?}", rule.source.port, command.kind);
                if spec
                    .expired
                    .as_ref()
                    .is_some_and(|expiry| expiry.port == rule.source.port)
                    && cases.is_none_or(|cases| cases.iter().any(|case| case == "LoopExpired"))
                {
                    return Err(error(
                        &path,
                        "LoopExpired closes the run and cannot command it again",
                    ));
                }
                if command.owner != owner {
                    return Err(error(
                        &path,
                        "a Call command must target its own Loop context",
                    ));
                }
                if let Err(refused) = call.validate_command(
                    rule.source.port.name(),
                    cases,
                    command.kind,
                    connector.effect,
                ) {
                    let detail = match refused {
                        CallCommandError::RepeatedFrames => {
                            "a streamed port cannot issue a single Loop verdict"
                        }
                        CallCommandError::UnsafeRecurrence { .. } => {
                            "recurrence after possible visibility requires effect = none or idempotent"
                        }
                        CallCommandError::UnknownPort | CallCommandError::UnknownCase(_) => {
                            "the command source is outside the Contract's Call guarantees"
                        }
                    };
                    return Err(error(&path, detail));
                }
                if command.kind == LoopCommandKind::Next {
                    let source = ports.node_port_type(slot, &rule.source.port);
                    let selected = source.and_then(|ty| match cases {
                        Some(cases) => routing::selected_case_type(ty, cases),
                        None => Some(ty.clone()),
                    });
                    let entry = ports.node_port_type(slot, &spec.entry);
                    if !matches!((selected, entry), (Some(from), Some(to)) if from.is_assignable_to(to))
                    {
                        return Err(error(
                            &path,
                            "Loop.next must preserve the recurrent entry type",
                        ));
                    }
                }
            }
        }
    }
    // Frame production and terminal cases are correlated by the Contract.
    // Unknown payload choices conservatively contribute their maximum command
    // count; a second verdict cannot hide on another port or route declaration.
    for (case, _) in call.terminal_cases() {
        let mut commands = 0;
        for rule in rules() {
            let terminal = rule.source.port.name() == call.terminal();
            let produces = terminal
                || matches!(call.frame_production(rule.source.port.name()),
                Some(FrameProduction::ExactlyOneOn(cases)) if cases.contains(case));
            if !produces {
                continue;
            }
            let count = |destinations: &[RouteDestination]| {
                destinations
                    .iter()
                    .filter(|destination| matches!(destination, RouteDestination::LoopCommand(_)))
                    .count()
            };
            commands += match &rule.selection {
                RouteSelection::All(destinations) => count(destinations),
                RouteSelection::Cases(arms) if terminal => arms
                    .iter()
                    .find(|arm| arm.cases.iter().any(|candidate| candidate == case))
                    .map_or(0, |arm| count(&arm.destinations)),
                RouteSelection::Cases(arms) => arms
                    .iter()
                    .map(|arm| count(&arm.destinations))
                    .max()
                    .unwrap_or(0),
            };
        }
        if commands > 1 {
            return Err(error(
                case,
                "a Call application could issue more than one Loop command",
            ));
        }
    }
    Ok(())
}
