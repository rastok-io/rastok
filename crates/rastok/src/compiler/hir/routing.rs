//! Checked source routing, before paths are resolved to execution-plan indices.
//! These paths and their ordering are semantic-hash inputs; plan layout is not.

use crate::node;
pub use crate::plan::LoopCommandKind;

/// One source-level routing declaration. Additive declarations are preserved
/// until lowering normalizes the Graph's wiring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteRule {
    /// Named source port before lowering assigns Node indices.
    pub source: Endpoint,
    /// Unconditional destinations or a closed case table.
    pub selection: RouteSelection,
}

/// The same destinations for every value, or disjoint closed cases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteSelection {
    /// The destination set applies to every source value.
    All(Vec<RouteDestination>),
    /// Disjoint cases choose one destination set.
    Cases(Vec<RouteArm>),
}

/// A canonical set of cases and their explicit destinations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteArm {
    /// Sorted, nonempty case names.
    pub cases: Vec<String>,
    /// Sorted explicit dispositions selected by these cases.
    pub destinations: Vec<RouteDestination>,
}

/// A checked source disposition. Commands are not spatial delivery edges.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum RouteDestination {
    /// Deliver to an ordinary port.
    Port(Endpoint),
    /// Explicitly discard the value with no delivery.
    Discard,
    /// Produce a temporal manager command, not a spatial edge.
    LoopCommand(LoopCommandTarget),
    /// Request closure within the lexical Graph instance.
    InstanceStop(Box<InstanceStopTarget>),
}

/// The lexical authority and target of an instance-closure command.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct InstanceStopTarget {
    /// Graph in which the command was authored.
    pub scope: node::OwnerPath,
    /// Declared owner addressed by this endpoint or command.
    pub owner: EndpointOwner,
}

/// The owner and kind of a checked Loop command.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LoopCommandTarget {
    /// Declared owner addressed by this endpoint or command.
    pub owner: EndpointOwner,
    /// Next, Again or Done, independently of its syntax.
    pub kind: LoopCommandKind,
}

/// A checked named endpoint; default ports are already explicit.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Endpoint {
    /// Declared owner addressed by this endpoint or command.
    pub owner: EndpointOwner,
    /// Explicit port name, including a resolved conventional default.
    pub port: node::Port,
}

/// The source-level distinction between an Executor and a Graph boundary.
/// The plan instead resolves both to its one Node table.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum EndpointOwner {
    /// Operator or Connector path, which cannot be empty.
    Node(node::Path),
    /// Graph boundary path; the App root has the empty path.
    Graph(node::OwnerPath),
}

impl EndpointOwner {
    /// Root-relative segments, independent of the declared Node's kind.
    pub fn segments(&self) -> &[String] {
        match self {
            Self::Node(path) => path.segments(),
            Self::Graph(path) => path.segments(),
        }
    }
}
