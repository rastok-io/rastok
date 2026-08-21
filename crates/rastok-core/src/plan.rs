//! Serializable compiled execution model.
//!
//! The `ExecutionPlan` is what ships to runtime hosts: tables plus
//! canonical expression trees — no closures, no host references. Attribute
//! constants are the boot-time reductions; the semantic identity was
//! taken earlier, over the canonical HIR (`identity.md` §1), and rides
//! beside the plan as [`SemanticHash`].

mod serialization;
mod type_resolution;
mod validation;

pub(crate) use serialization::type_tree;
pub use serialization::{DecodeError, FORMAT_VERSION, MAGIC, decode, encode};
pub use type_resolution::{ResolvedPort, ResolvedPortTable, TypeOrigin};
pub use validation::{PlanValidationError, ValidatedExecutionPlan};

use crate::datatype::DataType;
use crate::expr;
use crate::node;
use crate::value::Value;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// Compile-time resolved node reference. The claim is scoped: slots
/// STORED in a [`ValidatedExecutionPlan`] are proven in bounds, but
/// the type itself is freely constructible — so every public API
/// that takes a caller's `Slot` next to a plan VERIFIES it and
/// refuses typed, never indexes on faith.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Slot(u32);

impl Slot {
    /// Wraps a node index.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// The node index inside [`ExecutionPlan::nodes`].
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// What happens when an output has no outgoing edge.
///
/// This is plan data, not a convention inferred from the port's name.  The
/// validated plan rejects a root-sink policy on an input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnwiredPolicy {
    /// The compiler warns when the output is left unwired.
    RequireWire,
    /// The VM publishes an unclaimed emission at the root sink.
    RootSink,
}

/// One entry of a port table — a node's or the boundary's: the name,
/// direction, unwired policy, and OPTIONAL pinned type. A present type is a
/// public contract (identity material, transported in the package);
/// an absent one is a source-level inference variable, never an implicit
/// `variant`. The canonical order key is
/// `(port, direction)` — the type never decides an order.
#[derive(Debug, Clone, PartialEq)]
pub struct PortDecl {
    /// The port name.
    pub port: node::Port,
    /// The declared direction.
    pub direction: node::Direction,
    /// Routing policy when no edge claims this port. Inputs must use
    /// [`UnwiredPolicy::RequireWire`].
    pub unwired: UnwiredPolicy,
    /// The pinned type, when annotated.
    pub ty: Option<DataType>,
}

impl PortDecl {
    /// An untyped entry — the whole M0 subset.
    #[must_use]
    pub fn untyped(port: node::Port, direction: node::Direction) -> Self {
        Self {
            port,
            direction,
            unwired: UnwiredPolicy::RequireWire,
            ty: None,
        }
    }

    /// An output which publishes unclaimed emissions at the root sink.
    #[must_use]
    pub fn root_sink(port: node::Port, ty: Option<DataType>) -> Self {
        Self {
            port,
            direction: node::Direction::Output,
            unwired: UnwiredPolicy::RootSink,
            ty,
        }
    }

    /// The canonical order key.
    #[must_use]
    pub fn key(&self) -> (&node::Port, node::Direction) {
        (&self.port, self.direction)
    }
}

/// The compiled graph, in the database sense: tables of nodes, edges,
/// and handlers.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionPlan {
    /// Lexical lifecycle owners, in canonical preorder. Entry zero is the
    /// root App Graph. Graph composition may flatten the spatial DAG, but it
    /// never erases this table: instance addressing, immutable bindings and
    /// future placement all attach here.
    pub owners: Vec<Owner>,
    /// The nodes, in canonical order; [`Slot`]s index this table.
    pub nodes: Vec<Node>,
    /// Boundary ports declared on the graph, in canonical order.
    /// Conventional `in`, `out`, and `err` rows are omitted when
    /// unannotated, but an explicit type constraint may restate them.
    pub boundary: Vec<PortDecl>,
    /// The wiring, desugared to explicit pairs, in canonical order —
    /// the same order the semantic hash sees.
    pub edges: Vec<Edge>,
    /// Every handler, in canonical order — (slot, activation key).
    /// Declaration order between handlers carries no identity; only
    /// the statements INSIDE a handler are order-sensitive.
    pub handlers: Vec<Handler>,
    /// The resolved `meta.app` schema — inferred from the graph's
    /// writes, travelling so the validator answers without a
    /// Registry (D-2026-08-10-1): flat dotted field, unified type.
    /// Sorted by field; prefix-free.
    pub app_schema: Vec<(alloc::string::String, DataType)>,
    /// The resolved `meta.ext` vocabulary — the instantiated
    /// sheets' fields ONLY: the standard `content_type` is IMPLICIT
    /// (`text`), known to every consumer and never serialized
    /// (D-2026-08-10-4).
    pub ext_schema: Vec<(alloc::string::String, DataType)>,
    /// The build-time asset table (D-2026-08-10-5): content-addressed,
    /// DEDUPLICATED by digest; `expr::Expr::Asset` indexes it. Sorted
    /// by digest — one content, one order, one identity.
    pub assets: Vec<AssetBlob>,
}

/// One embedded build-time blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetBlob {
    /// blake3 of the bytes — the IDENTITY the hash seals.
    pub digest: [u8; 32],
    /// The raw content, unnormalized.
    pub bytes: alloc::vec::Vec<u8>,
}

impl ExecutionPlan {
    /// Resolves one owner reference without trusting a freely constructed
    /// [`OwnerSlot`].
    #[must_use]
    pub fn owner(&self, slot: OwnerSlot) -> Option<&Owner> {
        self.owners.get(slot.index())
    }

    /// The instance policy owned by one executable Node.
    #[must_use]
    pub fn node_instance(&self, slot: Slot) -> Option<&InstanceSpec> {
        let node = self.nodes.get(slot.index())?;
        self.owner(node.owner).map(|owner| &owner.instance)
    }

    /// The canonical text of one edge — the same rendering the
    /// consumed cursors and the budget obligations seal.
    #[must_use]
    pub fn edge_text(&self, edge: &Edge) -> String {
        let end = |endpoint: &EndpointRef| match endpoint.node {
            Some(slot) => format!("{}.{}", self.nodes[slot.index()].path, endpoint.port),
            None => format!(".{}", endpoint.port),
        };
        let to = match &edge.to {
            Some(endpoint) => end(endpoint),
            None => String::from("!discard"),
        };
        format!("{}>{to}", end(&edge.from))
    }
}

/// What a planned node is — the valid states, and only those: an
/// operator never carries a connector spec, a connector never lacks
/// one.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    /// Pure graph-internal computation.
    Operator,
    /// External-effect adapter under a contract sheet.
    Connector(ConnectorSpec),
}

/// One planned node.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Root-relative address — id-recipe material.
    pub path: node::Path,
    /// What the node is.
    pub kind: NodeKind,
    /// The node's materialized port table, in canonical order. Every resolved
    /// Connector contract row is present and typed; Operator rows record
    /// source/inferred constraints. The total effective types, including
    /// inference origin, live in [`ValidatedExecutionPlan::resolved_ports`].
    pub ports: Vec<PortDecl>,
    /// Declared state cells with their boot-time defaults, in
    /// canonical order.
    pub vars: Vec<VarCell>,
    /// The owner row for this Node. Validation proves that the row is a
    /// [`OwnerKind::Node`] carrying this exact Node slot.
    pub owner: OwnerSlot,
}

impl Node {
    /// The connector configuration, when the node is a connector.
    #[must_use]
    pub fn connector(&self) -> Option<&ConnectorSpec> {
        match &self.kind {
            NodeKind::Connector(spec) => Some(spec),
            NodeKind::Operator => None,
        }
    }
}

/// Compile-time resolved owner reference. Like [`Slot`], the freely
/// constructible value carries no validity claim until it sits in a
/// [`ValidatedExecutionPlan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OwnerSlot(u32);

impl OwnerSlot {
    /// Wraps an owner-table index.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// The index inside [`ExecutionPlan::owners`].
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Root-relative lexical owner path. The root App Graph is the empty path;
/// every nested Graph or Node appends one non-empty segment. This differs
/// deliberately from [`node::Path`], whose empty form is never a Node.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OwnerPath(Vec<String>);

impl OwnerPath {
    /// The root App Graph path.
    #[must_use]
    pub const fn root() -> Self {
        Self(Vec::new())
    }

    /// Builds a root-relative owner path.
    ///
    /// # Errors
    ///
    /// Returns [`node::InvalidName`] when a segment is empty or contains `.`.
    pub fn new<I, S>(segments: I) -> Result<Self, node::InvalidName>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let segments: Vec<String> = segments.into_iter().map(Into::into).collect();
        if segments.is_empty() {
            return Ok(Self::root());
        }
        node::Path::new(segments.clone())?;
        Ok(Self(segments))
    }

    /// The path segments, outermost first.
    #[must_use]
    pub fn segments(&self) -> &[String] {
        &self.0
    }

    /// Whether this is the root App Graph.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for OwnerPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.join("."))
    }
}

/// The declaration represented by one owner row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerKind {
    /// A lexical Graph scope. The root App Graph is the unique parentless row.
    Graph,
    /// An executable Operator or Connector.
    Node(Slot),
}

/// One lexical lifecycle and instance owner.
#[derive(Debug, Clone, PartialEq)]
pub struct Owner {
    /// Root-relative static owner path.
    pub path: OwnerPath,
    /// Enclosing Graph owner. Only the root App Graph has no parent.
    pub parent: Option<OwnerSlot>,
    /// The declaration this row preserves after DAG flattening.
    pub kind: OwnerKind,
    /// Orthogonal logical addressing and lifetime policy.
    pub instance: InstanceSpec,
    /// Immutable bindings owned by this scope, in canonical name order.
    pub vals: Vec<OwnerValue>,
}

/// One owner's logical instance policy. A missing key selects the singleton
/// address inside the parent instance; persistent remains an explicit value
/// in the categorical plan even when its source spelling was omitted.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceSpec {
    /// Expression and inferred type of the selected key, when keyed.
    pub key: Option<expr::Tree>,
    /// When the selected instance closes automatically.
    pub lifetime: InstanceLifetime,
}

/// Closed transport vocabulary for lifetime policies. The scheduler admits
/// each profile only when its corresponding runtime gate has landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceLifetime {
    /// No automatic expiry.
    Persistent,
    /// Expire after this many nanoseconds without activity.
    Idle(i64),
    /// Expire this many nanoseconds after `l:start`.
    Fixed(i64),
    /// Admit one root delivery, then close after its work quiesces.
    SinglePass,
    /// Epoch-aligned processing-time boxes of this many nanoseconds.
    Tumbling(i64),
}

/// The point at which an immutable expression becomes a value. Ordering is
/// semantic: later sites may depend on every earlier one.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EvaluationSite {
    /// Reduced by the compiler.
    Plan,
    /// Bound once from deployment inputs.
    Boot,
    /// Bound once for each logical owner instance.
    Instance,
    /// Evaluated for each admitted invocation.
    Invocation,
}

impl EvaluationSite {
    /// Whether this ceiling admits the actual site.
    #[must_use]
    pub const fn admits(self, actual: Self) -> bool {
        actual as u8 <= self as u8
    }
}

/// One immutable binding preserved with its typed expression. Plan-reduced
/// values still carry the source expression because that is semantic identity;
/// their transported reduction lets later construction reuse the proven
/// constant without evaluating the expression again.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnerValue {
    /// Lexical binding name.
    pub name: String,
    /// Earliest site at which the expression closes.
    pub site: EvaluationSite,
    /// Checked expression and result type.
    pub expr: expr::Tree,
    /// Present exactly for [`EvaluationSite::Plan`].
    pub resolved: Option<Value>,
}

/// A connector's planned configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectorSpec {
    /// The contract sheet name (`Process`).
    pub contract: String,
    /// The declared effect — typed at the HIR/plan frontier, where
    /// distinctions become irrevocable.
    pub effect: node::Effect,
    /// The declared behavior, likewise typed.
    pub behavior: node::Behavior,
    /// The remaining effective attributes, either reduced or carrying their
    /// checked evaluation phase; the typed axes above are not repeated here.
    pub attributes: Vec<(String, AttrSetting)>,
}

impl ConnectorSpec {
    /// The setting of one attribute, if declared.
    #[must_use]
    pub fn attribute(&self, name: &str) -> Option<&AttrSetting> {
        self.attributes
            .iter()
            .find(|(declared, _)| declared == name)
            .map(|(_, setting)| setting)
    }
}

/// One effective attribute setting.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrSetting {
    /// A bare enum-like keyword.
    Word(String),
    /// A reduced constant.
    Value(Value),
    /// A checked expression evaluated at the transported site. Plan-shaping
    /// rows can never use this form; validation has enough information to
    /// reject it without the compiler Registry.
    Expr {
        /// Earliest actual evaluation site.
        site: EvaluationSite,
        /// Checked expression and result type.
        expr: expr::Tree,
    },
}

/// One planned state cell.
#[derive(Debug, Clone, PartialEq)]
pub struct VarCell {
    /// The cell name.
    pub name: String,
    /// The cell's logical data type.
    pub ty: DataType,
    /// The boot-time default, when declared.
    pub init: Option<Value>,
}

/// One planned handler: a triggered, ordered statement body.
#[derive(Debug, Clone, PartialEq)]
pub struct Handler {
    /// The owning node.
    pub node: Slot,
    /// The declared activation pattern — not the causal trigger a
    /// transaction records; see [`node::HandlerActivation`].
    pub activation: node::HandlerActivation,
    /// The compiled guard, when present.
    pub guard: Option<expr::Tree>,
    /// The steps, in text order — the order fixing emission indices.
    pub body: Vec<Step>,
}

/// One executable step.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Transaction-local binding.
    Let {
        /// The binding name.
        name: String,
        /// The expression.
        expr: expr::Tree,
    },
    /// Emission on a port.
    Emit {
        /// The port.
        port: node::Port,
        /// What is emitted.
        value: Emission,
    },
    /// State-cell write.
    SetVar {
        /// The cell name.
        name: String,
        /// The expression.
        expr: expr::Tree,
    },
}

/// What an emission builds.
#[derive(Debug, Clone, PartialEq)]
pub enum Emission {
    /// Re-emit the input unchanged.
    Identity,
    /// Inherit the input; patch the listed fields.
    Patch(Vec<FieldWrite>),
    /// Full-data replacement; meta untouched.
    Replace(Vec<FieldWrite>),
    /// A computed value becoming the whole `data`.
    Data(expr::Tree),
    /// No emission on this port this transaction.
    Nothing,
}

/// One writable field assignment inside an emission.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldWrite {
    /// The checked landing site.
    pub target: crate::envelope::WriteTarget,
    /// The expression.
    pub expr: expr::Tree,
}

/// One planned edge.
#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    /// Lexical Graph owner in which this route was declared.
    pub scope: OwnerSlot,
    /// The emitting end.
    pub from: EndpointRef,
    /// The receiving end; `None` is the explicit discard.
    pub to: Option<EndpointRef>,
}

/// One resolved edge end; a boundary end has no node.
#[derive(Debug, Clone, PartialEq)]
pub struct EndpointRef {
    /// The node, or `None` for the graph's boundary.
    pub node: Option<Slot>,
    /// The port.
    pub port: node::Port,
}

/// Diagnostic and trace symbol information — a side table, never
/// identity material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolTable {
    /// Slot index to node path text.
    pub nodes: Vec<String>,
}

/// `blake3(canonical HIR)` — the full 256-bit semantic identity of a
/// graph (`identity.md` §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SemanticHash([u8; 32]);

impl SemanticHash {
    /// Wraps a digest.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for SemanticHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Sequential build/deploy number — a flyweight alias of the semantic
/// hash; the manifest is the mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GraphVersion(pub u64);

/// The append-only (version ↔ hash) ledger. In embedded mode — every
/// simulation included — it is re-seeded at each boot, so the first
/// allocation is deterministically version 1 and golden traces stay
/// stable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    rows: Vec<(GraphVersion, SemanticHash)>,
}

impl Manifest {
    /// A fresh embedded-mode manifest.
    #[must_use]
    pub fn embedded() -> Self {
        Self::default()
    }

    /// Allocates the next version for a deployment of `hash`: always
    /// `max + 1`, append-only — a rollback receives a new number.
    /// `None` when the range is exhausted; monotonicity never wraps.
    pub fn allocate(&mut self, hash: SemanticHash) -> Option<GraphVersion> {
        let next = GraphVersion(match self.rows.last() {
            None => 1,
            Some((version, _)) => version.0.checked_add(1)?,
        });
        self.rows.push((next, hash));
        Some(next)
    }

    /// The hash a version points at, if allocated.
    #[must_use]
    pub fn hash_of(&self, version: GraphVersion) -> Option<SemanticHash> {
        self.rows
            .iter()
            .find(|(allocated, _)| *allocated == version)
            .map(|(_, hash)| *hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_allocates_monotonically() {
        let mut manifest = Manifest::embedded();
        let first = manifest.allocate(SemanticHash::from_bytes([1; 32]));
        let second = manifest.allocate(SemanticHash::from_bytes([2; 32]));
        let rollback = manifest.allocate(SemanticHash::from_bytes([1; 32]));
        assert_eq!(first, Some(GraphVersion(1)));
        assert_eq!(second, Some(GraphVersion(2)));
        assert_eq!(rollback, Some(GraphVersion(3)));
        assert_eq!(
            manifest.hash_of(rollback.unwrap()),
            Some(SemanticHash::from_bytes([1; 32]))
        );
    }
}
