//! Desugared, resolved, and typed graph — the checker's output and
//! the planner's input.
//!
//! The HIR carries only what survived every check: names are resolved,
//! the port table is complete — declared and inferred, defaults
//! implicit — envelope sugar is normalized into its
//! semantic form, and every expression is a compiled CEL tree. Spans
//! and other diagnostic material stay in side tables owned by later
//! stages; nothing here is hashed with positions in it.

use crate::cel;
use crate::node;

/// The checked unit: the single `: App` graph of the deployed unit,
/// and its lab corpus. Labs are never identity material — the
/// semantic hash reads `app` alone.
#[derive(Debug, Clone, PartialEq)]
pub struct Hir {
    /// The application graph.
    pub app: Graph,
    /// The checked simulations of the App's lab, when declared.
    pub labs: Vec<Lab>,
}

pub use crate::node::Direction;

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

/// What a checked node is — one source of truth, no duplicated
/// discriminant: an operator never carries a trait or attributes, a
/// connector always carries both.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeRole {
    /// Pure graph-internal computation.
    Operator,
    /// External-effect adapter under a contract sheet.
    Connector {
        /// The trait symbol (`Process`).
        trait_name: String,
        /// Checked attributes, in declaration order.
        attributes: Vec<Attribute>,
    },
}

/// One checked node declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Root-relative address; single-segment in the flat M0 subset.
    pub path: node::Path,
    /// What the node is.
    pub role: NodeRole,
    /// Node-local constant bindings — identity material, substituted
    /// into shipped expressions at lowering.
    pub vals: Vec<Val>,
    /// Mutable state cells, in declaration order.
    pub vars: Vec<Var>,
    /// The semantic port table — declared and inferred, with
    /// directions; the defaults `in`, `out`, `err` stay implicit.
    pub ports: Vec<(node::Port, Direction)>,
    /// Checked handlers, in declaration order.
    pub handlers: Vec<Handler>,
}

impl Node {
    /// The node vocabulary category of this role.
    #[must_use]
    pub fn category(&self) -> node::Kind {
        match self.role {
            NodeRole::Operator => node::Kind::Operator,
            NodeRole::Connector { .. } => node::Kind::Connector,
        }
    }

    /// The connector's trait symbol, when a connector.
    #[must_use]
    pub fn connector_trait(&self) -> Option<&str> {
        match &self.role {
            NodeRole::Connector { trait_name, .. } => Some(trait_name),
            NodeRole::Operator => None,
        }
    }

    /// The checked attributes; an operator has none.
    #[must_use]
    pub fn attributes(&self) -> &[Attribute] {
        match &self.role {
            NodeRole::Connector { attributes, .. } => attributes,
            NodeRole::Operator => &[],
        }
    }
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
    /// The declared activation pattern — not the causal trigger a
    /// transaction records; see [`node::HandlerActivation`].
    pub activation: node::HandlerActivation,
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

/// One checked lab: the simulation corpus bound to the App sharing
/// its name.
#[derive(Debug, Clone, PartialEq)]
pub struct Lab {
    /// The lab name — the App's name.
    pub name: String,
    /// The checked simulations.
    pub simulations: Vec<Simulation>,
}

/// One checked simulation: a named set of scenarios sharing mocks.
#[derive(Debug, Clone, PartialEq)]
pub struct Simulation {
    /// The simulation name.
    pub name: String,
    /// Simulation-level mocks, shared by every scenario.
    pub mocks: Vec<Mock>,
    /// The scenarios, in source order.
    pub scenarios: Vec<Scenario>,
}

/// A scripted stand-in for one connector: the partner replaced, the
/// bookkeeping untouched.
#[derive(Debug, Clone, PartialEq)]
pub struct Mock {
    /// The mocked connector.
    pub connector: node::Path,
    /// The reply branches — ordinary checked handlers, first match
    /// in text order.
    pub handlers: Vec<Handler>,
}

/// One checked scenario.
#[derive(Debug, Clone, PartialEq)]
pub struct Scenario {
    /// The scenario name.
    pub name: String,
    /// The optional human label.
    pub label: Option<String>,
    /// Scenario-level mock overrides — whole-mock replacement,
    /// nearest wins.
    pub mocks: Vec<Mock>,
    /// The action and observation steps, in source order.
    pub steps: Vec<ScenarioStep>,
}

/// One checked scenario step.
#[derive(Debug, Clone, PartialEq)]
pub enum ScenarioStep {
    /// The world sends: an injection at the App boundary.
    Inject {
        /// The draft content, in emission vocabulary.
        value: EmitValue,
        /// The explicit inbound sequence, when written.
        seq: Option<u64>,
    },
    /// The discrete sweep to `now + nanos`.
    Advance {
        /// The constant duration, reduced to nanoseconds.
        nanos: i64,
    },
    /// A pure observation of the current window.
    Expect(Expectation),
}

/// One checked expectation.
#[derive(Debug, Clone, PartialEq)]
pub struct Expectation {
    /// The optional report label.
    pub label: Option<String>,
    /// How many matches satisfy it.
    pub quantifier: Quantifier,
    /// The observed target.
    pub target: ExpectTarget,
    /// The compiled predicate over each delivery.
    pub predicate: Option<cel::Tree>,
}

/// The three quantifiers, nothing more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Quantifier {
    /// At least one match.
    Exists,
    /// Zero deliveries in the window.
    Nothing,
    /// An exact-count comparison.
    Count(Cmp, i64),
}

/// A count comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    Le,
    /// `>=`
    Ge,
}

/// Where an expectation looks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpectTarget {
    /// `app.err` — the root error sink.
    RootErr,
    /// `app.<port>` — envelopes leaving the App boundary.
    Boundary(node::Port),
    /// `<node>.<port>` — deliveries reaching a node port.
    Node {
        /// The receiving node.
        path: node::Path,
        /// The receiving port.
        port: node::Port,
    },
}
