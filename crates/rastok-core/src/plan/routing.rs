//! Static routing decisions, distinct from physical delivery edges.

use alloc::string::String;
use alloc::vec::Vec;

use super::Endpoint;
use crate::node::NodeIndex;

/// Restrict a closed source type without unwrapping its values. A bool keeps
/// its logical type: the type algebra has no singleton-boolean family.
pub(crate) fn selected_case_type(
    ty: &crate::datatype::DataType,
    cases: &[String],
) -> Option<crate::datatype::DataType> {
    use crate::datatype::{DataType, EnumType, TaggedUnionType};
    if cases.is_empty() {
        return None;
    }
    match ty {
        DataType::Bool
            if cases
                .iter()
                .all(|case| matches!(case.as_str(), "true" | "false")) =>
        {
            Some(DataType::Bool)
        }
        DataType::Enum(domain) if cases.iter().all(|case| domain.symbols().contains(case)) => {
            EnumType::new(cases.to_vec()).ok().map(DataType::Enum)
        }
        DataType::TaggedUnion(domain)
            if cases
                .iter()
                .all(|case| domain.cases().iter().any(|member| member.name() == case)) =>
        {
            TaggedUnionType::new(
                domain
                    .cases()
                    .iter()
                    .filter(|member| cases.iter().any(|case| case == member.name()))
                    .cloned()
                    .collect(),
            )
            .ok()
            .map(DataType::TaggedUnion)
        }
        _ => None,
    }
}

/// One checked routing decision owned by a single source port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteRule {
    /// The source whose emitted envelope selects this rule.
    pub source: Endpoint,
    /// Unconditional or closed-case routing.
    pub selection: RouteSelection,
}

impl RouteRule {
    /// Canonical transport order; unlike open Dispatch arms, route declaration
    /// order never influences selection.
    #[must_use]
    pub fn canonical_order_key(&self) -> Vec<u8> {
        super::serialization::route_order_key(self)
    }
}

/// Static route selection. Closed cases are stored canonically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteSelection {
    /// The same destination set applies to every productible source case.
    All(Vec<Destination>),
    /// Exactly one exhaustive case arm applies.
    Cases(Vec<RouteArm>),
}

impl RouteSelection {
    pub(crate) fn selected(&self, value: &crate::value::Value) -> Option<&[Destination]> {
        use crate::value::Value;
        match self {
            Self::All(destinations) => Some(destinations),
            Self::Cases(arms) => {
                let case = match value {
                    Value::Bool(true) => "true",
                    Value::Bool(false) => "false",
                    Value::Enum(case) => case.as_str(),
                    Value::TaggedUnion(tagged) => tagged.case.as_str(),
                    _ => return None,
                };
                arms.iter()
                    .find(|arm| arm.cases.iter().any(|member| member == case))
                    .map(|arm| arm.destinations.as_slice())
            }
        }
    }
}

/// One canonical route arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteArm {
    /// Sorted, non-empty source cases.
    pub cases: Vec<String>,
    /// Canonically sorted destinations selected for those cases.
    pub destinations: Vec<Destination>,
}

/// One destination selected before a producer transaction commits.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Destination {
    /// A spatial delivery endpoint.
    Port(Endpoint),
    /// Explicitly consume the source without a delivery.
    Discard,
    /// A durable command to a temporal Extension manager. This is never a
    /// spatial edge and therefore cannot make a cycle in the DAG.
    LoopCommand(LoopCommandTarget),
    /// Request closure of an exact instance in the lexical wiring scope.
    /// This disposition has no spatial edge and never creates its target.
    InstanceStop(InstanceStopTarget),
}

/// Static authority for an instance closure, preserved through flattening.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct InstanceStopTarget {
    /// Graph in which the command was authored.
    pub scope: NodeIndex,
    /// That Graph or one of its direct children; never an arbitrary path.
    pub owner: NodeIndex,
}

/// Addressed Loop-manager command used as a wiring destination.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LoopCommandTarget {
    /// Owner whose attached Loop manager receives the command.
    pub owner: NodeIndex,
    /// Command kind; the routed envelope is the state/cause carrier.
    pub kind: LoopCommandKind,
}

/// Command kind independent of its statement or routing carrier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LoopCommandKind {
    /// Use the routed envelope as the next state.
    Next,
    /// Retain `Loop.state`; the routed envelope is only the cause.
    Again,
    /// Close the run without a business envelope.
    Done,
}

impl LoopCommandKind {
    /// Canonical command discriminant, shared by identity and transport.
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Next => 1,
            Self::Again => 2,
            Self::Done => 3,
        }
    }

    /// Read a canonical command discriminant without a default interpretation.
    #[must_use]
    pub const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            1 => Some(Self::Next),
            2 => Some(Self::Again),
            3 => Some(Self::Done),
            _ => None,
        }
    }
}
