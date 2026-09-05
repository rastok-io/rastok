//! Serializable compiled execution model.
//!
//! The `ExecutionPlan` is what ships to runtime hosts: tables plus
//! canonical expression trees — no closures, no host references. Attribute
//! constants are the boot-time reductions; the semantic identity was
//! taken earlier, over the canonical HIR (`identity.md` §1), and rides
//! beside the plan as [`SemanticHash`].

mod annotation_resolution;
mod case_partition;
mod serialization;
mod type_resolution;
mod validation;

pub use case_partition::{CasePartitionError, closed_case_domain, validate_case_partition};
pub(crate) use serialization::type_tree;
pub use serialization::{DecodeError, FORMAT_VERSION, MAGIC, decode, encode};

/// Standard `meta.ext` leaves known to every graph without travelling in the
/// plan schema. They are protocol context, not fields owned by one Connector
/// sheet. The list is closed for one package format and sorted by spelling.
pub const STANDARD_EXT_FIELDS: &[&str] = &[
    "content_type",
    "http_headers",
    "http_method",
    "http_status",
    "http_uri",
    "http_version",
    "session_id",
    "websocket_origin",
    "websocket_subprotocol",
];

/// Logical type of one standard implicit `meta.ext` leaf.
#[must_use]
pub fn standard_ext_field_type(field: &str) -> Option<DataType> {
    match field {
        "content_type" => Some(DataType::MediaType),
        "http_method"
        | "http_uri"
        | "http_version"
        | "session_id"
        | "websocket_origin"
        | "websocket_subprotocol" => Some(DataType::Text),
        "http_headers" => Some(http_headers_type()),
        "http_status" => Some(DataType::INT),
        _ => None,
    }
}

/// Lossless logical projection of HTTP fields: lowercase names map to values
/// in wire order, and field bytes are never forced through UTF-8.
#[must_use]
pub fn http_headers_type() -> DataType {
    DataType::text_map(DataType::array(DataType::BYTES))
}

/// Whether one spelling is the canonical lowercase HTTP field-name used by
/// the portable header projection. HTTP pseudo-fields are protocol state and
/// never enter the application header map.
#[must_use]
pub fn http_header_name_holds(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

/// Whether a materialized value is a canonical lossless HTTP header map.
#[must_use]
pub fn http_headers_value_holds(value: &crate::value::Value) -> bool {
    http_headers_type().accepts(value)
        && matches!(value, crate::value::Value::Map(fields)
        if fields.iter().all(|(name, values)| {
            http_header_name_holds(name)
                && matches!(values, crate::value::Value::Array(values) if !values.is_empty())
        }))
}

/// Logical request-header fields with dedicated contract spelling.
///
/// The struct is intentionally width-subtyped: each written field has a
/// precise type, while an absent standard field is absent rather than
/// fabricated as `null`. Extending this table adds completion without turning
/// the open HTTP field namespace into a closed enum.
#[must_use]
pub fn typed_http_headers_type_holds(ty: &DataType) -> bool {
    let DataType::Struct(structure) = ty else {
        return false;
    };
    structure.fields().iter().all(|field| match field.name() {
        "content_type" => field.ty() == &DataType::MediaType,
        _ => false,
    })
}

/// Whether a typed request-header struct carries only canonical known fields.
#[must_use]
pub fn typed_http_headers_value_holds(value: &crate::value::Value) -> bool {
    let crate::value::Value::Struct(fields) = value else {
        return false;
    };
    fields.iter().all(|(name, value)| match name.as_str() {
        "content_type" => DataType::MediaType.accepts(value),
        _ => false,
    })
}

/// User-facing map for extension request fields. Names retain ordinary HTTP
/// casing in source, while values are explicit ordered UTF-8 field lines.
#[must_use]
pub fn custom_http_headers_type() -> DataType {
    DataType::text_map(DataType::array(DataType::Text))
}

fn http_header_token_holds(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn managed_http_header(name: &str) -> bool {
    matches!(
        name,
        "authorization"
            | "connection"
            | "content-length"
            | "content-type"
            | "host"
            | "idempotency-key"
            | "keep-alive"
            | "proxy-connection"
            | "sec-websocket-extensions"
            | "sec-websocket-key"
            | "sec-websocket-protocol"
            | "sec-websocket-version"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn http_header_text_holds(value: &str) -> bool {
    value
        .as_bytes()
        .iter()
        .all(|byte| *byte == b'\t' || (0x20..=0x7e).contains(byte) || *byte >= 0x80)
}

/// Whether a materialized custom request-header map is portable and cannot
/// override a field owned by a typed attribute or the HTTP engine.
#[must_use]
pub fn custom_http_headers_value_holds(value: &crate::value::Value) -> bool {
    if !custom_http_headers_type().accepts(value) {
        return false;
    }
    let crate::value::Value::Map(fields) = value else {
        return false;
    };
    let mut canonical_names = alloc::collections::BTreeSet::new();
    fields.iter().all(|(name, values)| {
        let canonical = name.to_ascii_lowercase();
        http_header_token_holds(name)
            && !managed_http_header(&canonical)
            && canonical_names.insert(canonical)
            && matches!(values, crate::value::Value::Array(values)
                if !values.is_empty() && values.iter().all(|value|
                    matches!(value, crate::value::Value::Text(text)
                        if http_header_text_holds(text))))
    })
}

/// Merges typed and custom request fields into the one canonical lossless
/// descriptor sealed before an external call opens.
///
/// Names become lowercase because HTTP field names are case-insensitive;
/// collisions created by that normalization refuse instead of choosing a
/// winner. The result uses [`http_headers_type`] and is suitable for the
/// existing request digest and all HTTP versions.
#[must_use]
pub fn canonical_request_headers(
    typed: &crate::value::Value,
    custom: &crate::value::Value,
) -> Option<crate::value::Value> {
    if !typed_http_headers_value_holds(typed) || !custom_http_headers_value_holds(custom) {
        return None;
    }
    let mut result = alloc::collections::BTreeMap::new();
    let crate::value::Value::Struct(fields) = typed else {
        return None;
    };
    for (name, value) in fields {
        match (name.as_str(), value) {
            ("content_type", crate::value::Value::Text(value)) => {
                result.insert(
                    String::from("content-type"),
                    crate::value::Value::Array(alloc::vec![crate::value::Value::Bytes(
                        value.as_bytes().to_vec(),
                    )]),
                );
            }
            _ => return None,
        }
    }
    let crate::value::Value::Map(fields) = custom else {
        return None;
    };
    for (name, values) in fields {
        let name = name.to_ascii_lowercase();
        let crate::value::Value::Array(values) = values else {
            return None;
        };
        let values = values
            .iter()
            .map(|value| match value {
                crate::value::Value::Text(value) => {
                    Some(crate::value::Value::Bytes(value.as_bytes().to_vec()))
                }
                _ => None,
            })
            .collect::<Option<alloc::vec::Vec<_>>>()?;
        if result
            .insert(name, crate::value::Value::Array(values))
            .is_some()
        {
            return None;
        }
    }
    let value = crate::value::Value::Map(result);
    http_headers_value_holds(&value).then_some(value)
}
pub use type_resolution::{ResolvedPort, ResolvedPortTable, TypeOrigin};
pub use validation::{PlanValidationError, ValidatedExecutionPlan};

use crate::datatype::DataType;
use crate::expr;
use crate::node;
use crate::schema_annotation::AnnotationTable;
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
    /// Sparse data-policy metadata beside the effective logical type.
    pub annotations: AnnotationTable,
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
            annotations: AnnotationTable::empty(),
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
            annotations: AnnotationTable::empty(),
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
    /// Every handler, in complete canonical transport order. Declaration
    /// order between mutually exclusive guarded handlers carries no identity;
    /// only statements INSIDE a handler are order-sensitive.
    pub handlers: Vec<Handler>,
    /// The resolved `meta.app` schema — inferred from the graph's
    /// writes, travelling so the validator answers without a
    /// Registry (D-2026-08-10-1): flat dotted field, unified type.
    /// Sorted by field; prefix-free.
    pub app_schema: Vec<(alloc::string::String, DataType)>,
    /// The resolved `meta.ext` vocabulary — the instantiated
    /// sheets' fields ONLY: the standard `content_type` and `session_id`
    /// leaves are IMPLICIT (`text`), known to every consumer and never
    /// serialized (D-2026-08-10-4, D-2026-08-25-1).
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

    /// Complete boundary contract of one Graph owner.
    ///
    /// The root Graph already owns [`Self::boundary`], so its owner row keeps
    /// no duplicate claim. Nested Graphs carry their contract on the owner
    /// row because composition removes their boundary endpoints from the
    /// flattened edge table.
    #[must_use]
    pub fn graph_ports(&self, slot: OwnerSlot) -> Option<&[PortDecl]> {
        let owner = self.owner(slot)?;
        if owner.kind != OwnerKind::Graph {
            return None;
        }
        if owner.path.is_root() {
            Some(&self.boundary)
        } else {
            Some(&owner.ports)
        }
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

/// Executable payload carried by one planned node.
///
/// This is deliberately not the semantic [`node::Kind`]. Graphs live in the
/// owner table rather than the executable-node table; an executable Operator
/// carries no extra payload, while an executable Connector must carry its
/// exact contract specification.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeExecution {
    /// Pure graph-internal computation.
    Operator,
    /// External-effect Driver under a contract sheet.
    Connector(ConnectorSpec),
}

/// One planned node.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Root-relative address — id-recipe material.
    pub path: node::Path,
    /// Kind-specific executable payload.
    pub execution: NodeExecution,
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
    /// The node's structural semantic kind.
    #[must_use]
    pub const fn kind(&self) -> node::Kind {
        match self.execution {
            NodeExecution::Operator => node::Kind::Operator,
            NodeExecution::Connector(_) => node::Kind::Connector,
        }
    }

    /// The connector configuration, when the node is a connector.
    #[must_use]
    pub fn connector(&self) -> Option<&ConnectorSpec> {
        match &self.execution {
            NodeExecution::Connector(spec) => Some(spec),
            NodeExecution::Operator => None,
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

    /// Enclosing lexical Graph path. The root has no parent.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        (!self.0.is_empty()).then(|| Self(self.0[..self.0.len() - 1].to_vec()))
    }

    /// Deepest lexical path shared by both owners.
    #[must_use]
    pub fn common_ancestor(&self, other: &Self) -> Self {
        let length = self
            .0
            .iter()
            .zip(&other.0)
            .take_while(|(left, right)| left == right)
            .count();
        Self(self.0[..length].to_vec())
    }
}

impl From<&node::Path> for OwnerPath {
    fn from(path: &node::Path) -> Self {
        Self(path.segments().to_vec())
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
    /// A nested Graph's complete boundary contract in canonical order. Node
    /// owners and the root Graph carry an empty table: Node ports remain on
    /// [`Node`], while the root contract has its existing single authority in
    /// [`ExecutionPlan::boundary`].
    pub ports: Vec<PortDecl>,
    /// Closed aspect specifications in canonical aspect order. Keeping the
    /// specification inside the enum makes an aspect-without-configuration
    /// impossible in a categorical plan.
    pub aspects: Vec<AspectSpec>,
    /// Orthogonal logical addressing and lifetime policy.
    pub instance: InstanceSpec,
    /// Immutable bindings owned by this scope, in canonical name order.
    pub vals: Vec<OwnerValue>,
}

/// One fully configured NodeAspect carried by a categorical plan.
///
/// The enum is deliberately closed. Recognizing a source name does not grant
/// it a transport tag: a new aspect lands only together with its complete
/// Registry-free validation contract.
#[derive(Debug, Clone, PartialEq)]
pub enum AspectSpec {
    /// Finite temporal recursion owned by a Graph boundary.
    Loop(LoopSpec),
}

impl AspectSpec {
    /// Semantic aspect selected by this complete specification.
    #[must_use]
    pub const fn aspect(&self) -> node::Extension {
        match self {
            Self::Loop(_) => node::Extension::Loop,
        }
    }
}

/// One plan-reduced constant whose checked expression remains identity and
/// validation material.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanConstant {
    /// Canonical checked expression after lexical constant substitution.
    pub expr: expr::Tree,
    /// Claimed reduction, re-evaluated Registry-free after decode.
    pub resolved: Value,
}

/// Complete first-gate Loop transport.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopSpec {
    /// Optional positive maximum number of body applications. A verified
    /// measure may stand alone; when both exist this remains the runtime cap.
    pub fuel: Option<PlanConstant>,
    /// Optional re-admission delay.
    pub delay: Option<LoopDelay>,
    /// Optional small termination certificate, re-verified after decode.
    pub proof: Option<crate::loop_proof::Certificate>,
}

/// Closed Loop delay policy.
#[derive(Debug, Clone, PartialEq)]
pub enum LoopDelay {
    /// Every recurrence waits the same duration.
    Fixed {
        /// Positive delay.
        after: PlanConstant,
    },
    /// Delay doubles and saturates at the inclusive ceiling.
    Exponential {
        /// Positive first delay.
        first: PlanConstant,
        /// Positive ceiling, no smaller than `first`.
        ceiling: PlanConstant,
    },
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
    /// Finite drain and settlement budgets. Absence is valid only when this
    /// owner's closure can never hold an external invocation open.
    pub closing: Option<ClosingPolicy>,
}

/// The one bounded Closing protocol. Actions are fixed by the runtime:
/// drain admitted work, cancel calls, settle observed exits, then abandon
/// every still-unknown outcome. Only the two explicit budgets are policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClosingPolicy {
    /// Nanoseconds available to drain already-admitted work.
    pub drain_for: i64,
    /// Nanoseconds available for cancelled calls to publish an observed exit.
    pub settle_for: i64,
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
    /// Sparse policy beside the expression's resolved result type.
    pub annotations: AnnotationTable,
}

/// A connector's planned configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectorSpec {
    /// Stable semantic contract identity (`rastok/process@1`).
    pub contract: crate::connector::ContractId,
    /// Inbound/outbound runtime roles required by the contract.
    pub roles: crate::connector::ConnectorRoles,
    /// Cardinality and terminal lifecycle required by the contract.
    pub exchange: crate::connector::ConnectorExchange,
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
    /// reject it without the compiler ContractCatalog.
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
    /// Sparse policy beside the cell's logical type.
    pub annotations: AnnotationTable,
}

/// One planned handler: a triggered, ordered statement body.
#[derive(Debug, Clone, PartialEq)]
pub struct Handler {
    /// Lexical declaration that owns the handler.
    pub owner: HandlerOwner,
    /// The declared activation pattern — not the causal trigger a
    /// transaction records; see [`node::HandlerActivation`].
    pub activation: node::HandlerActivation,
    /// The compiled guard, when present.
    pub guard: Option<expr::Tree>,
    /// The steps, in text order — the order fixing emission indices.
    pub body: Vec<Step>,
}

impl Handler {
    /// Complete canonical ordering key for the shipped handler table.
    ///
    /// Graph aspects may own several mutually exclusive guarded handlers for
    /// one activation. The activation alone is therefore no longer a total
    /// order; the exact transported handler supplies the deterministic
    /// tie-break without making declaration order semantic.
    #[must_use]
    pub fn canonical_order_key(&self) -> Vec<u8> {
        serialization::handler_order_key(self)
    }
}

/// Owner of one ordinary handler body.
///
/// Graph handlers reuse the exact guard/statement representation of Node
/// handlers. This is a transport distinction, not a Loop-only execution
/// family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HandlerOwner {
    /// Executable Operator or Connector slot.
    Node(Slot),
    /// Lexical Graph owner slot.
    Graph(OwnerSlot),
}

impl HandlerOwner {
    /// Node slot when this is an executable-Node handler.
    #[must_use]
    pub const fn node(self) -> Option<Slot> {
        match self {
            Self::Node(slot) => Some(slot),
            Self::Graph(_) => None,
        }
    }

    /// Graph owner slot when this is a Graph handler.
    #[must_use]
    pub const fn graph(self) -> Option<OwnerSlot> {
        match self {
            Self::Graph(slot) => Some(slot),
            Self::Node(_) => None,
        }
    }
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
        /// Sparse policy beside the binding's resolved result type.
        annotations: AnnotationTable,
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
    /// Request closure of the current logical Node instance. The request is
    /// committed atomically with this handler transaction.
    Stop,
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
