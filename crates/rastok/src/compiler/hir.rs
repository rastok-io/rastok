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
    /// Uses of an alias lower their expanded table beside the concrete schema;
    /// the alias declaration itself never becomes a second runtime authority.
    pub named_schemas: Vec<(String, AnnotatedSchema)>,
}

pub use crate::node::PortDirection;

/// One checked graph.
#[derive(Debug, Clone, PartialEq)]
pub struct Graph {
    /// The graph's declared name; excluded from descendant identity.
    pub name: String,
    /// Canonically sorted Extensions. The root owns `App`; `Loop` and future
    /// temporal Extensions remain orthogonal to the Graph category.
    pub extensions: Vec<node::Extension>,
    /// Resolved Loop policy, absent on a Graph without Loop.
    pub loop_spec: Option<LoopSpec>,
    /// Root-relative lexical owner path. The App owns the empty path; nested
    /// Graphs append their declared name to their parent's path.
    pub path: crate::node::OwnerPath,
    /// Boundary ports declared on the graph itself; the defaults `in`
    /// and `out` exist implicitly. Identity material.
    pub boundary: Vec<crate::plan::PortDecl>,
    /// Graph-level constant bindings, in declaration order.
    pub vals: Vec<Val>,
    /// The graph's nodes, in declaration order.
    pub nodes: Vec<Node>,
    /// Lexically nested Graph owners, in declaration order.
    pub graphs: Vec<Graph>,
    /// The wiring, desugared to explicit endpoint pairs.
    pub edges: Vec<Edge>,
    /// Source-level routing groups and case selection. Every declaration is
    /// retained here; `edges` is only its deduplicated physical projection.
    pub routes: Vec<RouteRule>,
    /// Graph-owned instance addressing and lifetime. Absence means the
    /// persistent singleton and is canonically identical to an explicitly
    /// written persistent lifetime.
    pub instance: Option<InstancePolicy>,
}

/// Checked Loop policy. Reduced constants remain beside their expression
/// trees; neither parser strings nor a runtime default become an authority.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopSpec {
    /// Resolved recurrent admission port. The manager always re-enters this
    /// exact port; it is identity material in v16.
    pub entry: node::Port,
    /// Canonically sorted ordinary outputs whose emission supplies the natural
    /// `Done` verdict. Connector Calls use their Contract terminal instead and
    /// therefore keep this empty.
    pub terminals: Vec<node::Port>,
    /// Existing host terminal selected for productible exhaustion.
    pub expired: Option<crate::plan::LoopExpiredOutput>,
    /// Optional positive body-application budget. A checked measure can stand
    /// alone; when both are present, fuel remains the operational cap.
    pub fuel: Option<Const>,
    /// Optional re-entry timing policy.
    pub delay: Option<DelayPolicy>,
    /// Independently checkable termination proof, when declared.
    pub proof: Option<crate::loop_proof::Certificate>,
}

/// Recurrence delay policy shared by Loop applications and Retry attempts.
#[derive(Debug, Clone, PartialEq)]
pub enum DelayPolicy {
    /// Every recurrence waits the same positive duration.
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

/// Executor-specific facts attached to one checked node.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeSemantics {
    /// Pure graph-internal computation.
    Operator,
    /// External-effect Driver under a contract sheet.
    Connector {
        /// Exact versioned Contract owned by this Connector.
        contract: crate::connector::ContractId,
        /// Runtime roles declared by that contract.
        roles: crate::connector::ConnectorDirection,
        /// Interaction lifecycle declared by that contract.
        exchange: crate::connector::ConnectorExchange,
        /// Checked attributes, in declaration order.
        attributes: Vec<Attribute>,
    },
}

/// A checked Retry policy, disjoint from Loop's authored state protocol.
#[derive(Debug, Clone, PartialEq)]
pub struct RetrySpec {
    /// Total attempt bound, including the first invocation.
    pub attempts: Const,
    /// Canonically sorted terminal names, checked against the Contract.
    pub cases: Vec<String>,
    /// Backoff starts at the winning retained Call closure.
    pub delay: Option<DelayPolicy>,
    /// Resolved Call request port.
    pub entry: node::Port,
    /// Resolved Call terminal port; Retry does not add one.
    pub terminal: node::Port,
}

/// One checked node declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Root-relative address; single-segment in the flat M0 subset.
    pub path: node::Path,
    /// Canonically sorted Extensions attached to this Executor.
    pub extensions: Vec<node::Extension>,
    /// Checked Loop source contract when this Executor owns `+ Loop`.
    pub loop_spec: Option<LoopSpec>,
    /// A pinned-operation policy, without a port or authored command.
    pub retry_spec: Option<RetrySpec>,
    /// Operator or Connector semantics.
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
        match self.semantics {
            NodeSemantics::Operator => node::Kind::Operator,
            NodeSemantics::Connector { .. } => node::Kind::Connector,
        }
    }

    /// The connector's stable contract identity, when a connector.
    #[must_use]
    pub fn connector_contract_id(&self) -> Option<&crate::connector::ContractId> {
        match &self.semantics {
            NodeSemantics::Connector { contract, .. } => Some(contract),
            NodeSemantics::Operator => None,
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
    /// A join's pre-consumption `where`. A one-port `where` is normalized
    /// into [`Dispatch::OpenMatch`] and never appears here.
    pub admission_guard: Option<expr::Tree>,
    /// The checked decision tree. Exactly one linear leaf executes.
    pub dispatch: Dispatch,
}

impl Handler {
    /// Projects the subset that the categorical v15 plan can still carry.
    ///
    /// A one-port `where` is represented in the HIR as an open match with an
    /// empty fallback. Recognising that exact normal form here keeps the R3
    /// source/HIR work identity-neutral until the single v16 bump in R4.
    #[must_use]
    pub(crate) fn v15_linear_parts(&self) -> Option<(Option<&expr::Tree>, &[Statement])> {
        if let Some(body) = self.dispatch.as_linear() {
            return Some((self.admission_guard.as_ref(), body));
        }
        if self.admission_guard.is_some() {
            return None;
        }
        let (predicate, branch) = self.dispatch.filter_parts()?;
        Some((Some(predicate), branch.as_linear()?))
    }
}

// Core owns the checked decision and envelope-construction algebra.
pub use crate::plan::{
    ClosedArm, Dispatch, DispatchStep as Statement, EmitValue, FieldWrite, LoopCommand,
    MultiEmitValues, OpenArm,
};

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

mod routing;
pub use routing::{
    Endpoint, EndpointOwner, InstanceStopTarget, LoopCommandKind, LoopCommandTarget, RouteArm,
    RouteDestination, RouteRule, RouteSelection,
};

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
    /// The world sends through an App input or a modeled inbound Connector.
    Inject {
        /// Resolved, direction-checked ingress; never an arbitrary Node port.
        target: InjectionTarget,
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

/// The two root-producing seams admitted by a checked scenario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InjectionTarget {
    /// An input of the App's public boundary.
    Boundary(node::Port),
    /// A decoded arrival at a source Connector, admitted by its Lab Driver.
    Connector {
        /// Exact source Node; also the root's Connector provenance.
        path: node::Path,
        /// Source output selected by the resolved Contract.
        port: node::Port,
    },
}

impl InjectionTarget {
    /// Port at which the world-authored value enters the graph.
    #[must_use]
    pub fn port(&self) -> &node::Port {
        match self {
            Self::Boundary(port) | Self::Connector { port, .. } => port,
        }
    }

    /// Source Connector, absent for the separate App-input API.
    #[must_use]
    pub fn connector(&self) -> Option<&node::Path> {
        match self {
            Self::Boundary(_) => None,
            Self::Connector { path, .. } => Some(path),
        }
    }
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
