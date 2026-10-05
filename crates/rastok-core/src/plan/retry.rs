//! Re-derive repetition policies from the original Contract and closed plan
//! expressions. No Registry, source spelling or Driver claim is authority.

use super::*;
use crate::connector::{ConnectorExchange, FILE_CONNECTOR_ID};
use crate::node;
use alloc::string::ToString;

pub(super) fn validate(plan: &ExecutionPlan) -> Result<(), PlanValidationError> {
    for node in &plan.nodes {
        let Some(connector) = node.connector() else {
            continue;
        };
        let error = || PlanValidationError::ContractAttributeMismatch {
            node: node.path.clone(),
            attribute: "idempotent".into(),
        };
        if connector.attribute("effect").is_some() {
            return Err(PlanValidationError::ContractAttributeMismatch {
                node: node.path.clone(),
                attribute: "retired effect".into(),
            });
        }
        match (connector.call_semantics(), connector.idempotent.as_ref()) {
            (Some(_), Some(guarantee)) => {
                if !validation::plan_constant_holds(plan, guarantee, &DataType::Bool)?
                    || connector.attribute("idempotent")
                        != Some(&AttrSetting::Value(guarantee.resolved.clone()))
                    || (connector.contract.as_str() == FILE_CONNECTOR_ID
                        && !connector.is_idempotent())
                {
                    return Err(error());
                }
            }
            (None, None) if connector.attribute("idempotent").is_none() => {}
            _ => return Err(error()),
        }
    }
    for owner in &plan.owners {
        let Some(spec) = owner.retry_spec() else {
            continue;
        };
        let error = |detail: &str| PlanValidationError::InvalidRetryPolicy {
            owner: owner.path.to_string(),
            detail: detail.into(),
        };
        if owner.loop_spec().is_some() {
            return Err(error("Loop and Retry cannot share an owner"));
        }
        let OwnerKind::Node(slot) = owner.kind else {
            return Err(error("Retry requires an outbound Call Connector"));
        };
        let node = plan
            .nodes
            .get(slot.index())
            .ok_or_else(|| error("Retry has no executable owner"))?;
        let connector = node
            .connector()
            .ok_or_else(|| error("Retry requires an outbound Call Connector"))?;
        let call = connector
            .call_semantics()
            .ok_or_else(|| error("this Contract profile is not a finite outbound Call"))?;
        if connector.exchange != ConnectorExchange::Call
            || !connector.roles.has_outbound()
            || spec.entry.name() != call.request()
            || spec.terminal.name() != call.terminal()
        {
            return Err(error(
                "Retry entry and terminal must be the resolved Call ports",
            ));
        }
        let attempts_type = match spec.attempts.resolved {
            Value::Int(value) if value > 0 => {
                if i32::try_from(value).is_ok() {
                    DataType::INT
                } else {
                    DataType::LONG
                }
            }
            Value::Uint(value) if value > 0 => DataType::UINT,
            _ => return Err(error("Retry.attempts must be a positive total count")),
        };
        if !validation::plan_constant_holds(plan, &spec.attempts, &attempts_type)? {
            return Err(error("Retry.attempts is not its claimed plan constant"));
        }
        call.validate_retry_cases(&spec.cases, connector.is_idempotent())
            .map_err(|refused| error(&refused.to_string()))?;
        if !matches!(connector.attribute("timeout"), Some(AttrSetting::Value(Value::Duration(timeout))) if *timeout > 0)
        {
            return Err(error(
                "every Retry attempt requires a positive plan-constant Call timeout",
            ));
        }
        let constants = match &spec.delay {
            None => &[][..],
            Some(LoopDelay::Fixed { after }) => core::slice::from_ref(after),
            Some(LoopDelay::Exponential { first, ceiling }) => {
                for constant in [first, ceiling] {
                    if !validation::plan_constant_holds(plan, constant, &DataType::DURATION)? {
                        return Err(error("Retry.delay is not its claimed plan constant"));
                    }
                }
                &[][..]
            }
        };
        for constant in constants {
            if !validation::plan_constant_holds(plan, constant, &DataType::DURATION)? {
                return Err(error("Retry.delay is not its claimed plan constant"));
            }
        }
        if spec
            .delay
            .as_ref()
            .is_some_and(|delay| delay.duration_after(1).is_none())
        {
            return Err(error(
                "Retry.delay needs positive bounds with first no greater than ceiling",
            ));
        }
    }
    Ok(())
}

/// One authored Again site and the visible Call its application may repeat.
/// This is diagnostic evidence, not a serialized guarantee or rejection.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LoopRepeatWarning {
    /// Loop whose application contains both the decision and the Call.
    pub owner: node::OwnerPath,
    /// Exact source routed to that Loop's Again command.
    pub source: Endpoint,
    /// Call without a sufficient original-plan idempotence guarantee.
    pub connector: node::Path,
}

impl ValidatedExecutionPlan {
    /// Advisory only: includes visible sibling Calls and preserves terminal
    /// case correlation. An authored Next is not treated as a Retry.
    pub fn loop_repeat_warnings(&self) -> Result<Vec<LoopRepeatWarning>, PlanValidationError> {
        let plan = self.plan();
        let mut warnings = alloc::collections::BTreeSet::new();
        for owner in &plan.owners {
            if owner.loop_spec().is_none() {
                continue;
            }
            match owner.kind {
                OwnerKind::Graph => warnings.extend(super::graph_loop::validate(plan, owner)?),
                OwnerKind::Node(slot) => {
                    let node = &plan.nodes[slot.index()];
                    let Some(connector) = node.connector() else {
                        continue;
                    };
                    let Some(call) = connector.call_semantics() else {
                        continue;
                    };
                    let target = EndpointOwner::Node(node.path.clone());
                    for rule in plan
                        .boundary_routes
                        .iter()
                        .filter(|rule| rule.source.owner == target)
                    {
                        let repeats =
                            |cases: Option<&[String]>, destinations: &[RouteDestination]| {
                                destinations.iter().any(|destination| matches!(destination,
                                RouteDestination::LoopCommand(command)
                                if command.owner == target && command.kind == LoopCommandKind::Again))
                                && if rule.source.port.name() == call.terminal() {
                                    call.terminal_cases().iter().any(|(case, visibility)|
                                        cases.is_none_or(|cases| cases.iter().any(|selected| selected == case))
                                            && !visibility.allows_repetition(connector.is_idempotent()))
                                } else { !connector.is_idempotent() }
                            };
                        let repeats = match &rule.selection {
                            RouteSelection::All(destinations) => repeats(None, destinations),
                            RouteSelection::Cases(arms) => arms
                                .iter()
                                .any(|arm| repeats(Some(&arm.cases), &arm.destinations)),
                        };
                        if repeats {
                            warnings.insert(LoopRepeatWarning {
                                owner: owner.path.clone(),
                                source: rule.source.clone(),
                                connector: node.path.clone(),
                            });
                        }
                    }
                }
            }
        }
        Ok(warnings.into_iter().collect())
    }
}
