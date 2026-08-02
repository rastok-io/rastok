//! Desugared, resolved, and typed graph — the checker's output and
//! the planner's input.
//!
//! The HIR carries only what survived every check: names are resolved,
//! default ports are explicit, envelope sugar is normalized into its
//! semantic form, and every expression is a compiled CEL tree. Spans
//! and other diagnostic material stay in side tables owned by later
//! stages; nothing here is hashed with positions in it.

use crate::cel;
use crate::node;

/// The checked unit: the single `: App` graph of the deployed unit.
#[derive(Debug, Clone, PartialEq)]
pub struct Hir {
    /// The application graph.
    pub app: Graph,
}

/// Semantic port direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Direction {
    /// The port receives envelopes.
    Input,
    /// The port emits envelopes.
    Output,
}

/// One checked graph.
#[derive(Debug, Clone, PartialEq)]
pub struct Graph {
    /// The graph's declared name; excluded from descendant identity.
    pub name: String,
    /// Boundary ports declared on the graph itself; the defaults `in`
    /// and `out` exist implicitly. Identity material.
    pub boundary: Vec<(node::Port, Direction)>,
    /// Graph-level constant bindings, in declaration order.
    pub vals: Vec<Val>,
    /// The graph's nodes, in declaration order.
    pub nodes: Vec<Node>,
    /// The wiring, desugared to explicit endpoint pairs.
    pub edges: Vec<Edge>,
}

/// A constant position: the expression it was written as — the
/// identity material, hashed before const-eval per `identity.md` §1 —
/// and its reduced value.
#[derive(Debug, Clone, PartialEq)]
pub struct Const {
    /// The expression as compiled; enters the semantic hash.
    pub tree: cel::Tree,
    /// The boot-time reduction; never identity material.
    pub value: cel::Value,
}

/// One constant binding.
#[derive(Debug, Clone, PartialEq)]
pub struct Val {
    /// The binding name.
    pub name: String,
    /// The bound constant.
    pub value: Const,
}

/// One checked node declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Root-relative address; single-segment in the flat M0 subset.
    pub path: node::Path,
    /// Operator or connector.
    pub kind: node::Kind,
    /// The connector's trait symbol (`Process`), when a connector.
    pub connector_trait: Option<String>,
    /// Checked attributes, in declaration order.
    pub attributes: Vec<Attribute>,
    /// Node-local constant bindings — identity material, substituted
    /// into shipped expressions at lowering.
    pub vals: Vec<Val>,
    /// Mutable state cells, in declaration order.
    pub vars: Vec<Var>,
    /// Ports declared explicitly on the node, with their direction.
    pub declared_ports: Vec<(node::Port, Direction)>,
    /// Checked handlers, in declaration order.
    pub handlers: Vec<Handler>,
}

/// One checked attribute.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    /// The attribute key, dotted segments joined.
    pub name: String,
    /// The checked value.
    pub value: AttrValue,
}

/// A checked attribute value.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrValue {
    /// A bare enum-like keyword (`job`, `lines`, `idempotent`).
    Word(String),
    /// A constant expression with its reduction.
    Const(Const),
}

/// One mutable state cell.
#[derive(Debug, Clone, PartialEq)]
pub struct Var {
    /// The cell name.
    pub name: String,
    /// The cell's CEL-facing type.
    pub ty: cel::Type,
    /// The initial value, when declared; identity material — the
    /// recovery fold reads defaults.
    pub init: Option<Const>,
}

/// One checked handler.
#[derive(Debug, Clone, PartialEq)]
pub struct Handler {
    /// What fires the handler, in the semantic vocabulary.
    pub trigger: node::Trigger,
    /// The compiled `where` guard, when present.
    pub guard: Option<cel::Tree>,
    /// The body, in text order — the order that fixes emission
    /// indices.
    pub body: Vec<Statement>,
}

/// One checked statement.
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    /// Handler-local constant binding.
    Let {
        /// The binding name.
        name: String,
        /// The compiled expression.
        value: cel::Tree,
    },
    /// Emission on a port.
    Emit {
        /// The emission port.
        port: node::Port,
        /// What is emitted.
        value: EmitValue,
    },
    /// Write to a declared state cell.
    SetVar {
        /// The cell name.
        name: String,
        /// The compiled expression.
        value: cel::Tree,
    },
}

/// What an emission builds.
#[derive(Debug, Clone, PartialEq)]
pub enum EmitValue {
    /// `envelope` bare — re-emit the input unchanged.
    Identity,
    /// `envelope { … }` — inherit the input and patch listed fields.
    Patch(Vec<FieldWrite>),
    /// `{ … }` sugar — full-data replacement, meta untouched.
    Replace(Vec<FieldWrite>),
    /// A computed value becoming the whole `data`.
    Data(cel::Tree),
    /// `nothing` — no emission on this port this transaction.
    Nothing,
}

/// One writable field assignment inside an envelope builder.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldWrite {
    /// The dotted writable path (`data.text`, `meta.app.label`).
    pub path: Vec<String>,
    /// The compiled expression.
    pub value: cel::Tree,
}

/// One desugared edge: a single source-to-target pair.
#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    /// The emitting end.
    pub from: Endpoint,
    /// The receiving end, or the explicit discard.
    pub to: EdgeTarget,
}

/// The receiving end of an edge.
#[derive(Debug, Clone, PartialEq)]
pub enum EdgeTarget {
    /// A node or boundary port.
    Port(Endpoint),
    /// `nothing` — non-blocking drain.
    Discard,
}

/// One end of an edge, default ports made explicit.
#[derive(Debug, Clone, PartialEq)]
pub struct Endpoint {
    /// The node, or `None` for the graph's own boundary.
    pub node: Option<node::Path>,
    /// The port.
    pub port: node::Port,
}
