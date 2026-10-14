//! Registry-free proof of static route selections and additive dispositions.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use super::{
    CasePartitionError, Destination, Endpoint, RouteRule, RouteSelection, validate_case_partition,
};
use crate::datatype::DataType;

#[cfg(test)]
mod tests;

/// Incompatibility between declarations for the same source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteConflictKind {
    /// A source has more than one case match.
    MultipleMatches,
    /// Explicit discard and a real destination select the same value.
    MixedDiscard,
    /// One destination is selected twice for the same value.
    DuplicateDestination(Destination),
}

/// An additive conflict, with the affected cases rather than a source span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteConflict {
    /// `None` means all source values; otherwise the exact overlapping cases.
    pub cases: Option<Vec<String>>,
    /// The violated disposition rule.
    pub kind: RouteConflictKind,
}

/// A malformed selection or conflict in the complete routing inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteErrorKind {
    /// Transparent boundary composition contains a spatial cycle.
    BoundaryCycle,
    /// A flattened delivery violates a crossed Graph boundary's type pin.
    BoundaryTypeMismatch,
    /// A selection names an endpoint outside the flattened executable table.
    UnknownEndpoint,
    /// The command has no compatible owner or causal scope.
    InvalidManagerCommand,
    /// A closure target is not the wiring Graph or one of its direct children.
    InvalidInstanceCommand,
    /// The actual emitter cannot supply the lexical Graph's instance ancestry.
    InstanceCommandScope,
    /// There is no independently resolved type for a case-routed source.
    UnresolvedCaseSource,
    /// A case table contradicts the resolved source type or canonical order.
    CasePartition(CasePartitionError),
    /// A selected value has no disposition; discard must be explicit.
    EmptyDestinations,
    /// The destination set is not in strictly canonical order.
    DestinationOrder,
    /// A destination occurs twice within one selected set.
    DuplicateDestination(Destination),
    /// A selected set contains both discard and a real destination.
    MixedDiscard,
    /// Two declarations for one source conflict.
    Conflict(RouteConflict),
}

/// Exact location of a failed routing proof, independent of source spans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteValidationError {
    /// Structural position in the supplied routing inventory.
    pub path: String,
    /// The contradicted invariant.
    pub kind: RouteErrorKind,
}

impl RouteValidationError {
    fn at(path: &str, kind: RouteErrorKind) -> Self {
        Self {
            path: path.to_string(),
            kind,
        }
    }
}

impl fmt::Display for RouteValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", self.path)?;
        match &self.kind {
            RouteErrorKind::BoundaryCycle => f.write_str("Graph boundary routing contains a cycle"),
            RouteErrorKind::BoundaryTypeMismatch => {
                f.write_str("routed value does not conform to a crossed Graph boundary")
            }
            RouteErrorKind::UnresolvedCaseSource => {
                f.write_str("case-routed source has no resolved type")
            }
            RouteErrorKind::UnknownEndpoint => {
                f.write_str("route endpoint is not a declared executable endpoint")
            }
            RouteErrorKind::InvalidManagerCommand => {
                f.write_str("Loop manager command has no compatible owner or causal scope")
            }
            RouteErrorKind::InvalidInstanceCommand => {
                f.write_str("instance closure must target its lexical Graph or an exact direct child of that Graph")
            }
            RouteErrorKind::InstanceCommandScope => {
                f.write_str("instance closure needs a source already inside its lexical Graph instance; emit the request from an Operator inside that Graph")
            }
            RouteErrorKind::CasePartition(error) => fmt::Display::fmt(error, f),
            RouteErrorKind::EmptyDestinations => {
                f.write_str("a selected value needs a destination or explicit `nothing`")
            }
            RouteErrorKind::DestinationOrder => {
                f.write_str("route destinations are not in canonical order")
            }
            RouteErrorKind::DuplicateDestination(_) => {
                f.write_str("a destination occurs twice in one selected set")
            }
            RouteErrorKind::MixedDiscard => {
                f.write_str("`nothing` cannot coexist with a real destination")
            }
            RouteErrorKind::Conflict(conflict) => {
                match &conflict.kind {
                    RouteConflictKind::MultipleMatches => {
                        f.write_str("a source has more than one case match")?;
                    }
                    RouteConflictKind::MixedDiscard => {
                        f.write_str("additive routes mix `nothing` with a real destination")?;
                    }
                    RouteConflictKind::DuplicateDestination(_) => {
                        f.write_str("additive routes select the same destination twice")?;
                    }
                }
                if let Some(cases) = &conflict.cases {
                    write!(f, " for case(s) {}", cases.join("|"))?;
                }
                Ok(())
            }
        }
    }
}

pub(super) fn validate_plan_routing(
    plan: &super::ExecutionPlan,
    ports: &super::ResolvedPortTable,
) -> Result<(), super::PlanValidationError> {
    let physical = |endpoint: &Endpoint| -> Option<&DataType> {
        let node = plan.nodes.get(endpoint.node.index())?;
        if node.kind() == crate::node::Kind::Graph && endpoint.node != super::NodeIndex::ROOT {
            return None;
        }
        ports.endpoint_type(endpoint)
    };
    for pair in plan.routes.windows(2) {
        if pair[0].canonical_order_key() >= pair[1].canonical_order_key() {
            return Err(super::PlanValidationError::NonCanonicalOrder {
                collection: "routes",
            });
        }
    }
    for (index, rule) in plan.routes.iter().enumerate() {
        let path = format!("routes[{index}]");
        physical(&rule.source)
            .ok_or_else(|| RouteValidationError::at(&path, RouteErrorKind::UnknownEndpoint))?;
        let source = &rule.source;
        let sets = match &rule.selection {
            RouteSelection::All(destinations) => alloc::vec![(None, destinations.as_slice())],
            RouteSelection::Cases(arms) => arms
                .iter()
                .map(|arm| (Some(arm.cases.as_slice()), arm.destinations.as_slice()))
                .collect(),
        };
        for (cases, destination) in sets.into_iter().flat_map(|(cases, destinations)| {
            destinations
                .iter()
                .map(move |destination| (cases, destination))
        }) {
            if let Destination::InstanceStop(target) = destination {
                if !valid_instance_stop_target(plan, target) {
                    return Err(RouteValidationError::at(
                        &path,
                        RouteErrorKind::InvalidInstanceCommand,
                    )
                    .into());
                }
                continue;
            }
            if let Destination::LoopCommand(command) = destination {
                // Boundary composition validates the real command source.
                // Its flattened source can precede the Graph run admission.
                let graph_command = plan.nodes.get(command.owner.index()).is_some_and(|node| {
                    node.kind() == crate::node::Kind::Graph && node.loop_spec().is_some()
                });
                let admitted = graph_command
                    || command.owner == rule.source.node && {
                        let node = &plan.nodes[source.node.index()];
                        node.loop_spec().is_some()
                            && node
                                .connector()
                                .and_then(super::ConnectorSpec::call_semantics)
                                .is_some()
                    };
                if !admitted {
                    return Err(RouteValidationError::at(
                        &path,
                        RouteErrorKind::InvalidManagerCommand,
                    )
                    .into());
                }
                if graph_command && command.kind == super::LoopCommandKind::Next {
                    let selected = ports.endpoint_type(source).and_then(|ty| match cases {
                        Some(cases) => super::routing::selected_case_type(ty, cases),
                        None => Some(ty.clone()),
                    });
                    let entry = plan.nodes.get(command.owner.index()).and_then(|owner| {
                        let spec = owner.loop_spec()?;
                        owner
                            .ports
                            .iter()
                            .find(|port| port.port == spec.entry)?
                            .ty
                            .as_ref()
                    });
                    if !matches!((selected, entry), (Some(from), Some(to)) if from.is_assignable_to(to))
                    {
                        return Err(super::PlanValidationError::InvalidLoopControl {
                            owner: format!("{:?}", command.owner),
                            path: path.clone(),
                            detail: "Loop.next must preserve the recurrent entry type",
                        });
                    }
                }
                continue;
            }
            if matches!(destination, Destination::Port(endpoint) if physical(endpoint).is_none()) {
                return Err(
                    RouteValidationError::at(&path, RouteErrorKind::UnknownEndpoint).into(),
                );
            }
        }
    }
    validate_routing(&plan.routes, |endpoint| physical(endpoint).cloned())?;
    for (index, _) in plan.nodes.iter().enumerate() {
        super::call_loop::validate(
            plan,
            super::NodeIndex::new(u32::try_from(index).unwrap_or(u32::MAX)),
            ports,
        )?;
    }
    Ok(())
}

pub(super) fn valid_instance_stop_target(
    plan: &super::ExecutionPlan,
    target: &super::InstanceStopTarget,
) -> bool {
    plan.nodes
        .get(target.scope.index())
        .is_some_and(|scope| scope.kind() == crate::node::Kind::Graph)
        && plan
            .nodes
            .get(target.owner.index())
            .is_some_and(|owner| target.owner == target.scope || owner.parent == Some(target.scope))
}

impl core::error::Error for RouteValidationError {}

/// Checks canonical sets, closed case partitions and cross-declaration rules.
///
/// The supplied lookup must derive case-source types independently of route
/// claims. It is not consulted for unconditional selections, which also work
/// while source port types are being inferred. A case match always needs a
/// resolved closed type. No selection is sorted, deduplicated or repaired.
///
/// This proves the selection algebra, not a whole execution plan. Callers must
/// also establish endpoint existence/direction, destination type compatibility,
/// lexical scope, capacities, productibility, and manager ownership/context.
/// A manager destination is a command, not a physical delivery edge.
///
/// # Errors
///
/// Returns the first invalid selection or additive conflict, with its exact
/// inventory position. The input inventory remains unchanged.
pub fn validate_routing(
    rules: &[RouteRule],
    mut source_type: impl FnMut(&Endpoint) -> Option<DataType>,
) -> Result<(), RouteValidationError> {
    let mut by_source = BTreeMap::<&Endpoint, Vec<&RouteRule>>::new();
    for (index, rule) in rules.iter().enumerate() {
        let path = format!("routes[{index}].selection");
        match &rule.selection {
            RouteSelection::All(destinations) => {
                validate_destinations(destinations, &format!("{path}.destinations"))?;
            }
            RouteSelection::Cases(arms) => {
                let ty = source_type(&rule.source).ok_or_else(|| {
                    RouteValidationError::at(
                        &format!("routes[{index}].source"),
                        RouteErrorKind::UnresolvedCaseSource,
                    )
                })?;
                validate_case_partition(&ty, arms.iter().map(|arm| arm.cases.as_slice())).map_err(
                    |error| {
                        RouteValidationError::at(
                            &format!("{path}.arms"),
                            RouteErrorKind::CasePartition(error),
                        )
                    },
                )?;
                for (arm, selection) in arms.iter().enumerate() {
                    validate_destinations(
                        &selection.destinations,
                        &format!("{path}.arms[{arm}].destinations"),
                    )?;
                }
            }
        }
        let preceding = by_source.entry(&rule.source).or_default();
        for existing in preceding.iter() {
            if let Some(conflict) = route_conflict(existing, rule) {
                return Err(RouteValidationError::at(
                    &path,
                    RouteErrorKind::Conflict(conflict),
                ));
            }
        }
        preceding.push(rule);
    }
    Ok(())
}

fn validate_destinations(
    destinations: &[Destination],
    path: &str,
) -> Result<(), RouteValidationError> {
    if destinations.is_empty() {
        return Err(RouteValidationError::at(
            path,
            RouteErrorKind::EmptyDestinations,
        ));
    }
    if destinations.contains(&Destination::Discard)
        && destinations
            .iter()
            .any(|destination| *destination != Destination::Discard)
    {
        return Err(RouteValidationError::at(path, RouteErrorKind::MixedDiscard));
    }
    for pair in destinations.windows(2) {
        if pair[0] == pair[1] {
            return Err(RouteValidationError::at(
                path,
                RouteErrorKind::DuplicateDestination(pair[1].clone()),
            ));
        }
        if pair[0] > pair[1] {
            return Err(RouteValidationError::at(
                path,
                RouteErrorKind::DestinationOrder,
            ));
        }
    }
    Ok(())
}

/// Detects a conflict between two declarations without merging away evidence.
///
/// This also serves source diagnostics before the complete inventory is
/// verified. It does not replace the canonical-set and partition proof.
#[must_use]
pub fn route_conflict(existing: &RouteRule, candidate: &RouteRule) -> Option<RouteConflict> {
    if existing.source != candidate.source {
        return None;
    }
    match (&existing.selection, &candidate.selection) {
        (RouteSelection::Cases(_), RouteSelection::Cases(_)) => Some(RouteConflict {
            cases: None,
            kind: RouteConflictKind::MultipleMatches,
        }),
        (RouteSelection::All(left), RouteSelection::All(right)) => {
            destination_conflict(left, right, None)
        }
        (RouteSelection::All(left), RouteSelection::Cases(arms)) => arms
            .iter()
            .find_map(|arm| destination_conflict(left, &arm.destinations, Some(&arm.cases))),
        (RouteSelection::Cases(arms), RouteSelection::All(right)) => arms
            .iter()
            .find_map(|arm| destination_conflict(&arm.destinations, right, Some(&arm.cases))),
    }
}

fn destination_conflict(
    left: &[Destination],
    right: &[Destination],
    cases: Option<&[String]>,
) -> Option<RouteConflict> {
    let discards = |set: &[Destination]| set.contains(&Destination::Discard);
    let delivers = |set: &[Destination]| {
        set.iter()
            .any(|destination| *destination != Destination::Discard)
    };
    let kind = if (discards(left) && delivers(right)) || (discards(right) && delivers(left)) {
        RouteConflictKind::MixedDiscard
    } else {
        let duplicate = left
            .iter()
            .find(|destination| right.contains(destination))?;
        RouteConflictKind::DuplicateDestination(duplicate.clone())
    };
    Some(RouteConflict {
        cases: cases.map(<[String]>::to_vec),
        kind,
    })
}
