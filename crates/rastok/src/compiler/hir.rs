//! Desugared, resolved, and typed graph — the checker's output and
//! the planner's input.
//!
//! The HIR carries only what survived every check: names are resolved,
//! the port table is complete — declared and inferred, defaults
//! implicit — every braced value is classified into its semantic
//! form (D-2026-08-09-1), and every expression is a compiled Rastok expression
//! tree. Spans
//! and other diagnostic material stay in side tables owned by later
//! stages; nothing here is hashed with positions in it.

use crate::datatype::DataType;
use crate::expr;
use crate::node;
use crate::schema_annotation::{AnnotatedSchema, AnnotationTable};

/// The checked unit: the single `: App` graph of the deployed unit,
/// and its lab corpus. Labs are never identity material — the
/// semantic hash reads `app` alone.
#[derive(Debug, Clone, PartialEq)]
pub struct Hir {
    /// The application graph.
    pub app: Graph,
    /// The checked simulations of the App's lab, when declared.
    pub labs: Vec<Lab>,
    /// The sealed `meta.app` schema — the graph's writes, unified
    /// (D-2026-08-10-1). DERIVED from the writes: the hash never
    /// seals it twice.
    pub app_schema: Vec<(String, DataType)>,
    /// The `meta.ext` vocabulary: the sheets' fields only — standard
    /// `content_type` and `session_id` are implicit, never carried.
    pub ext_schema: Vec<(String, DataType)>,
    /// The build-time assets the checker resolved (D-2026-08-10-5).
    pub assets: Vec<crate::plan::AssetBlob>,
    /// Resolved annotated aliases, keyed by source name. Aliases remain
    /// structural sugar for ordinary typing; this side table is the A1 proof
    /// that their policy metadata survived expansion into canonical paths.
    /// It cannot lower until categorical package v15 exists.
    pub named_schemas: Vec<(String, AnnotatedSchema)>,
}

pub use crate::node::Direction;

/// One checked graph.
#[derive(Debug, Clone, PartialEq)]
pub struct Graph {
    /// The graph's declared name; excluded from descendant identity.
    pub name: String,
    /// The graph's effective node type (`App` for the deployment root,
    /// otherwise the canonical `Graph`).
    pub ty: node::Type,
    /// Canonically sorted aspects. Recovery HIR records a recognized aspect
    /// even while its diagnostic refuses compilation; lowering independently
    /// rejects every aspect until its plan/runtime gate lands.
    pub aspects: Vec<node::Aspect>,
    /// Checked Loop source contract retained behind the M1.7 lowering gate.
    /// `None` for every Graph without `: Loop`, and for a malformed recovery
    /// declaration.
    pub loop_spec: Option<LoopSpec>,
    /// Root-relative lexical owner path. The App owns the empty path; nested
    /// Graphs append their declared name to their parent's path.
    pub path: crate::plan::OwnerPath,
    /// Boundary ports declared on the graph itself; the defaults `in`
    /// and `out` exist implicitly. Identity material.
    pub boundary: Vec<crate::plan::PortDecl>,
    /// Sparse annotation tables beside the graph ports that declare policy.
    pub boundary_annotations: Vec<(node::Port, AnnotationTable)>,
    /// Graph-level constant bindings, in declaration order.
    pub vals: Vec<Val>,
    /// Graph-owned handlers. They use the same checked statement algebra as
    /// executable Node handlers; the distinct owner is preserved at lowering.
    pub handlers: Vec<Handler>,
    /// The graph's nodes, in declaration order.
    pub nodes: Vec<Node>,
    /// Lexically nested Graph owners, in declaration order.
    pub graphs: Vec<Graph>,
    /// The wiring, desugared to explicit endpoint pairs.
    pub edges: Vec<Edge>,
    /// Graph-owned instance addressing and lifetime. Absence means the
    /// persistent singleton and is canonically identical to an explicitly
    /// written persistent lifetime.
    pub instance: Option<InstancePolicy>,
}

/// The first checked Loop contract. The categorical plan remains the next
/// gate; keeping reduced constants here prevents parser strings from becoming
/// a second semantic authority.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopSpec {
    /// Optional positive body-application budget. A checked measure can stand
    /// alone; when both are present, fuel remains the operational cap.
    pub fuel: Option<Const>,
    /// Optional re-entry timing policy.
    pub delay: Option<LoopDelay>,
    /// Independently checkable termination proof, when declared.
    pub proof: Option<crate::loop_proof::Certificate>,
}

/// Closed Loop re-entry delay policy.
#[derive(Debug, Clone, PartialEq)]
pub enum LoopDelay {
    /// Every `next` waits the same positive duration.
    Fixed {
        /// Fixed delay.
        after: Const,
    },
    /// Delay doubles from `first`, saturating at `ceiling`.
    Exponential {
        /// Delay for the first re-entry.
        first: Const,
        /// Inclusive saturation ceiling, no smaller than `first`.
        ceiling: Const,
    },
}

/// A constant position: the expression it was written as — the
/// identity material, hashed before const-eval per `identity.md` §1 —
/// and its reduced value.
#[derive(Debug, Clone, PartialEq)]
pub struct Const {
    /// The expression as compiled; enters the semantic hash.
    pub tree: expr::Tree,
    /// The boot-time reduction; never identity material.
    pub value: crate::value::Value,
}

/// One constant binding.
#[derive(Debug, Clone, PartialEq)]
pub struct Val {
    /// The binding name.
    pub name: String,
    /// The bound constant.
    pub value: Const,
    /// Policy attached to the binding's explicit resolved type, empty when
    /// the binding was inferred or unannotated.
    pub annotations: AnnotationTable,
}

/// Family-specific facts attached to one checked node.
///
/// The structural family itself is [`node::Kind`], derived from the node's
/// effective [`node::Type`]. This enum preserves the extra contract facts a
/// Connector needs without calling that family distinction a "role".
#[derive(Debug, Clone, PartialEq)]
pub enum NodeSemantics {
    /// Pure graph-internal computation.
    Operator,
    /// External-effect Driver under a contract sheet.
    Connector {
        /// Runtime roles declared by that contract.
        roles: crate::connector::ConnectorRoles,
        /// Interaction lifecycle declared by that contract.
        exchange: crate::connector::ConnectorExchange,
        /// Checked attributes, in declaration order.
        attributes: Vec<Attribute>,
    },
}

/// One checked node declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Root-relative address; single-segment in the flat M0 subset.
    pub path: node::Path,
    /// Exactly one effective node type.
    pub ty: node::Type,
    /// Canonically sorted aspects. Recovery HIR records a recognized aspect;
    /// lowering independently refuses it until its plan/runtime gate lands.
    pub aspects: Vec<node::Aspect>,
    /// Family-specific checked facts.
    pub semantics: NodeSemantics,
    /// Node-local constant bindings — identity material, substituted
    /// into shipped expressions at lowering.
    pub vals: Vec<Val>,
    /// Mutable state cells, in declaration order.
    pub vars: Vec<Var>,
    /// Complete source/resolved port rows. Connector contracts materialize
    /// every conventional port with its active logical type and routing
    /// policy; operators materialize every port their body can use.
    pub ports: Vec<crate::plan::PortDecl>,
    /// Sparse annotation tables beside explicitly typed Node ports.
    pub port_annotations: Vec<(node::Port, AnnotationTable)>,
    /// Checked handlers, in declaration order.
    pub handlers: Vec<Handler>,
    /// Node-owned instance addressing and lifetime. Absence means the
    /// persistent singleton in the enclosing Graph instance.
    pub instance: Option<InstancePolicy>,
}

impl Node {
    /// The node's structural kind.
    #[must_use]
    pub fn kind(&self) -> node::Kind {
        self.ty.kind()
    }

    /// The connector's stable contract identity, when a connector.
    #[must_use]
    pub fn connector_contract_id(&self) -> Option<&crate::connector::ConnectorId> {
        match &self.ty {
            node::Type::Connector(contract) => Some(contract),
            node::Type::Graph | node::Type::App | node::Type::Operator => None,
        }
    }

    /// The checked attributes; an operator has none.
    #[must_use]
    pub fn attributes(&self) -> &[Attribute] {
        match &self.semantics {
            NodeSemantics::Connector { attributes, .. } => attributes,
            NodeSemantics::Operator => &[],
        }
    }
}

/// One checked Node-instance policy.
#[derive(Debug, Clone, PartialEq)]
pub struct InstancePolicy {
    /// The canonical-key expression; absence means one singleton address in
    /// the enclosing Graph instance.
    pub key: Option<expr::Tree>,
    /// Checked lifetime with source expressions retained for identity.
    pub lifetime: InstanceLifetime,
    /// Explicit finite Closing budgets, when this owner can hold calls.
    pub closing: Option<ClosingPolicy>,
}

/// Checked bounded Closing policy. Constants retain their source expression
/// because exact spelling is semantic identity at this frontier.
#[derive(Debug, Clone, PartialEq)]
pub struct ClosingPolicy {
    /// Drain budget.
    pub drain_for: Const,
    /// Settlement budget after cancellation.
    pub settle_for: Const,
}

/// Checked Node-instance lifetime.
#[derive(Debug, Clone, PartialEq)]
pub enum InstanceLifetime {
    /// No automatic expiry.
    Persistent,
    /// Expire after this admission-silence duration.
    Idle(Const),
    /// Expire this duration after the instance starts.
    Fixed(Const),
    /// Admit one root delivery, then close after its work quiesces.
    SinglePass,
    /// One epoch-aligned processing-time box of this duration.
    Tumbling(Const),
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
    /// A checked expression whose value closes after plan construction.
    Expr {
        /// Earliest site at which every binding is available.
        site: crate::plan::EvaluationSite,
        /// Canonical typed expression transported to the plan.
        tree: expr::Tree,
    },
}

/// One mutable state cell.
#[derive(Debug, Clone, PartialEq)]
pub struct Var {
    /// The cell name.
    pub name: String,
    /// The cell's logical data type.
    pub ty: DataType,
    /// The initial value, when declared; identity material — the
    /// recovery fold reads defaults.
    pub init: Option<Const>,
    /// Sparse policy table beside the state cell's logical type.
    pub annotations: AnnotationTable,
}

/// One checked handler.
#[derive(Debug, Clone, PartialEq)]
pub struct Handler {
    /// The declared activation pattern — not the causal trigger a
    /// transaction records; see [`node::HandlerActivation`].
    pub activation: node::HandlerActivation,
    /// Exact source expression of a join's `within` bound. The reduced
    /// nanoseconds travel in the activation; this tree preserves semantic
    /// identity just like every other constant position.
    pub join_within: Option<Const>,
    /// The compiled `where` guard, when present.
    pub guard: Option<expr::Tree>,
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
        value: expr::Tree,
        /// Sparse policy table beside the local binding's explicit type.
        annotations: AnnotationTable,
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
        value: expr::Tree,
    },
    /// Bare `stop`: close this Node's current logical instance.
    Stop,
}

/// What an emission builds.
#[derive(Debug, Clone, PartialEq)]
pub enum EmitValue {
    /// `.` — re-emit the input unchanged.
    Identity,
    /// The envelope form without a whole-`data` write — inherit the
    /// input and patch the listed fields.
    Patch(Vec<FieldWrite>),
    /// A braced build whose data is FRESH — a pure record, or the
    /// envelope form with a whole-`data` assignment (which may end
    /// scalar: `{data = 1}`); heritable metadata stays the input's.
    /// Lowered to the plan's `Replace`.
    FreshData(Vec<FieldWrite>),
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
pub struct FieldWrite {
    /// The checked landing site — what the runtime executes.
    pub target: crate::envelope::WriteTarget,
    /// The compiled expression.
    pub value: expr::Tree,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeTarget {
    /// A node or boundary port.
    Port(Endpoint),
    /// `nothing` — non-blocking drain.
    Discard,
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

/// A scripted stand-in for one connector: the driver replaced, the
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
        /// The graph input the draft enters — `in` or a declared
        /// boundary input, resolved and direction-checked.
        port: node::Port,
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
    pub predicate: Option<expr::Tree>,
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
    /// `app.<port>` — an ordinary output whose resolved unwired policy is
    /// `RootSink`.
    RootSink(node::Port),
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
