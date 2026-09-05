//! Static routing decisions, distinct from physical delivery edges.

use alloc::string::String;
use alloc::vec::Vec;

use crate::node;

/// One checked routing decision owned by a single source port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteRule {
    /// The source whose emitted envelope selects this rule.
    pub source: Endpoint,
    /// Unconditional or closed-case routing.
    pub selection: RouteSelection,
}

/// Static route selection. Closed cases are stored canonically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteSelection {
    /// The same destination set applies to every productible source case.
    All(Vec<RouteDestination>),
    /// Exactly one exhaustive case arm applies.
    Cases(Vec<RouteArm>),
}

/// One canonical route arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteArm {
    /// Sorted, non-empty source cases.
    pub cases: Vec<String>,
    /// Canonically sorted destinations selected for those cases.
    pub destinations: Vec<RouteDestination>,
}

/// One destination selected before a producer transaction commits.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum RouteDestination {
    /// A spatial delivery endpoint.
    Port(Endpoint),
    /// Explicitly consume the source without a delivery.
    Discard,
    /// A durable command to a temporal Extension manager. This is never a
    /// spatial edge and therefore cannot make a cycle in the DAG.
    LoopCommand(LoopCommandTarget),
}

/// Addressed Loop-manager command used as a wiring destination.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LoopCommandTarget {
    /// Owner whose attached Loop manager receives the command.
    pub owner: EndpointOwner,
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

/// One end of an edge, default ports made explicit.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Endpoint {
    /// The executable Node or transparent Graph boundary named by this end.
    pub owner: EndpointOwner,
    /// The port.
    pub port: node::Port,
}

/// What owns one checked edge endpoint. Graph boundaries survive checking so
/// lowering can compose nested wiring without inventing a runtime pseudo-node.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum EndpointOwner {
    /// One executable Operator or Connector.
    Node(node::Path),
    /// One Graph's input/output boundary, including the root App boundary.
    Graph(crate::plan::OwnerPath),
}
