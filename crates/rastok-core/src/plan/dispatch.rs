//! Checked Operator decision algebra, shared by the source HIR and core proof.
//!
//! This is not an additional v15 execution path. The categorical v16 transport
//! will carry this tree; v15 lowering continues to admit only its exact linear
//! projection. Source spans and compiler inference state never enter the tree.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

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
        value: DispatchEmission,
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
    Broadcast(DispatchEmission),
    /// Evaluate and pair values with targets from left to right.
    Tuple(Vec<DispatchEmission>),
}

/// Loop manager commands. They are neither ports nor spatial emissions.
#[derive(Debug, Clone, PartialEq)]
pub enum LoopCommand {
    /// Supply the next entry envelope.
    Next(DispatchEmission),
    /// Retain the current `Loop.state`.
    Again,
    /// Close without a business envelope.
    Done,
}

/// What an emission builds.
#[derive(Debug, Clone, PartialEq)]
pub enum DispatchEmission {
    /// `.` — re-emit the input unchanged.
    Identity,
    /// The envelope form without a whole-`data` write — inherit the
    /// input and patch the listed fields.
    Patch(Vec<DispatchFieldWrite>),
    /// A braced build whose data is FRESH — a pure record, or the
    /// envelope form with a whole-`data` assignment (which may end
    /// scalar: `{data = 1}`); heritable metadata stays the input's.
    /// Lowered to the plan's `Replace`.
    FreshData(Vec<DispatchFieldWrite>),
    /// A computed value becoming the whole `data`.
    Data(expr::Tree),
    /// `nothing` — no emission on this port this transaction.
    Nothing,
}

/// One writable field assignment inside an envelope builder. The
/// RESOLVED target is what travels: the hash seals it canonically
/// (D-2026-08-04-2 — `meta.flag` and `meta.app.flag` are one
/// identity), and the as-written spelling lives only in the
/// diagnostics' spans.
#[derive(Debug, Clone, PartialEq)]
pub struct DispatchFieldWrite {
    /// The checked landing site — what the runtime executes.
    pub target: crate::envelope::WriteTarget,
    /// The compiled expression.
    pub value: expr::Tree,
}
