//! Checked Operator decision algebra, shared by the source HIR and core proof.
//!
//! Source spans and compiler inference state never enter the tree.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use super::Emission;
use crate::schema_annotation::AnnotationTable;
use crate::{expr, node};

/// Maximum nested decisions on a Dispatch path, including a desugared `where`.
/// The parser and independent core proof enforce the same resource bound.
pub const MAX_DISPATCH_DEPTH: usize = 32;

/// Recursive, typed Operator decision tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Dispatch {
    /// Rectilinear statements in semantic source order.
    Linear(Vec<DispatchStep>),
    /// Closed selection over a tagged union, enum, or bool. Arms are sorted
    /// by their canonical case sets; their source order is erased.
    ClosedMatch {
        /// Pure local bindings evaluated before the scrutinee.
        prefix: Vec<DispatchStep>,
        /// Checked closed-domain expression.
        scrutinee: expr::Tree,
        /// Pairwise-disjoint, exhaustive arms.
        arms: Vec<ClosedArm>,
    },
    /// Ordered predicate selection; first true wins and fallback is required.
    OpenMatch {
        /// Pure local bindings evaluated before the predicates.
        prefix: Vec<DispatchStep>,
        /// Predicates and nested dispatches in source order.
        arms: Vec<OpenArm>,
        /// Exact `_` fallback.
        fallback: Box<Dispatch>,
    },
}

impl Dispatch {
    /// The statements of a linear leaf.
    #[must_use]
    pub fn as_linear(&self) -> Option<&[DispatchStep]> {
        match self {
            Self::Linear(statements) => Some(statements),
            Self::ClosedMatch { .. } | Self::OpenMatch { .. } => None,
        }
    }
}

/// One canonical closed-match arm.
#[derive(Debug, Clone, PartialEq)]
pub struct ClosedArm {
    /// Sorted, non-empty symbolic cases covered by this arm.
    pub cases: Vec<String>,
    /// Nested dispatch for those cases.
    pub dispatch: Box<Dispatch>,
}

/// One ordered open-match arm.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenArm {
    /// Boolean predicate.
    pub predicate: expr::Tree,
    /// Nested dispatch when it is the first true predicate.
    pub dispatch: Box<Dispatch>,
}

/// One checked statement.
#[derive(Debug, Clone, PartialEq)]
pub enum DispatchStep {
    /// Handler-local constant binding.
    Let {
        /// The binding name.
        name: String,
        /// The compiled expression.
        value: expr::Tree,
        /// Sparse policy table beside the local binding's explicit type.
        annotations: AnnotationTable,
    },
    /// Emission on a port.
    Emit {
        /// The emission port.
        port: node::Port,
        /// What is emitted.
        value: Emission,
    },
    /// Ordered atomic production of several distinct envelopes.
    MultiEmit {
        /// Output targets in authored order.
        ports: Vec<node::Port>,
        /// Broadcast or positionally paired values.
        values: MultiEmitValues,
    },
    /// Write to a declared state cell.
    SetVar {
        /// The cell name.
        name: String,
        /// The compiled expression.
        value: expr::Tree,
    },
    /// Bare `stop`: close this Node's current logical instance.
    Stop,
    /// A durable command produced for the owner's Loop manager.
    LoopCommand(LoopCommand),
}

/// Right-hand side shape of an atomic multi-emission.
#[derive(Debug, Clone, PartialEq)]
pub enum MultiEmitValues {
    /// Evaluate once, then build one envelope for every target.
    Broadcast(Emission),
    /// Evaluate and pair values with targets from left to right.
    Tuple(Vec<Emission>),
}

/// Loop manager commands. They are neither ports nor spatial emissions.
#[derive(Debug, Clone, PartialEq)]
pub enum LoopCommand {
    /// Supply the next entry envelope.
    Next(Emission),
    /// Retain the current `Loop.state`.
    Again,
    /// Close without a business envelope.
    Done,
}
