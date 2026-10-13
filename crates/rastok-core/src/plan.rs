//! Serializable compiled execution model.
//!
//! The `ExecutionPlan` is what ships to runtime hosts: tables plus
//! canonical expression trees — no closures, no host references. Attribute
//! constants are the boot-time reductions; the semantic identity was
//! taken earlier, over the canonical HIR (`identity.md` §1), and rides
//! beside the plan as [`SemanticHash`].
//!
//! A `*Spec` describes an authored header declaration after checking and
//! resolution: for example a Connector, an Extension or an instance policy.
//! It is configuration for execution, not mutable state of a running instance.

use crate::instance;
mod annotation_resolution;
mod call_loop;
mod case_partition;
mod composition;
mod dispatch;
mod dispatch_resolution;
mod dispatch_validation;
mod emit_value;
mod graph_loop;
mod productibility;
mod retry;
mod routing;
mod routing_validation;
mod serialization;
mod type_resolution;
mod validation;

use alloc::boxed::Box;

pub use case_partition::{CasePartitionError, closed_case_domain, validate_case_partition};
pub use composition::{RouteCompositionError, compose_routing};
pub use dispatch::{
    ClosedArm, Dispatch, DispatchStep, LoopCommand, MAX_DISPATCH_DEPTH, MultiEmitValues, OpenArm,
};
pub use dispatch_validation::{DispatchErrorKind, DispatchValidationError, validate_dispatch};
pub use emit_value::{EmitValue, FieldWrite};
pub use retry::LoopRepeatWarning;
pub use routing::{
    Endpoint, EndpointOwner, InstanceStopTarget, LoopCommandKind, LoopCommandTarget, RouteArm,
    RouteDestination, RouteRule, RouteSelection,
};
pub use routing_validation::{
    RouteConflict, RouteConflictKind, RouteErrorKind, RouteValidationError, route_conflict,
    validate_routing,
};
pub use serialization::fields;
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
pub(crate) use type_resolution::expression_reads_root;
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

/// Index into an execution plan's Node table, not a persistent identity.
///
/// Validation proves that indices stored in a [`ValidatedExecutionPlan`] are
/// in bounds. Callers can construct other indices; public APIs check them
/// against the supplied plan before accessing a Node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeIndex(u32);

impl NodeIndex {
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

/// One entry of a port table — a node's or the boundary's: the name,
/// direction and OPTIONAL pinned type. A present type is a
/// public contract (identity material, transported in the package);
/// an absent one is a source-level inference variable, never an implicit
/// `variant`. The canonical order key is
/// `(port, direction)` — the type never decides an order.
#[derive(Debug, Clone, PartialEq)]
pub struct PortDecl {
    /// The port name.
    pub port: node::Port,
    /// The declared direction.
    pub direction: node::PortDirection,
    /// The pinned type, when annotated.
    pub ty: Option<DataType>,
    /// Sparse data-policy metadata beside the effective logical type.
    pub annotations: AnnotationTable,
}

impl PortDecl {
    /// An untyped entry — the whole M0 subset.
    #[must_use]
    pub fn untyped(port: node::Port, direction: node::PortDirection) -> Self {
        Self {
            port,
            direction,
            ty: None,
            annotations: AnnotationTable::empty(),
        }
    }

    /// The canonical order key.
    #[must_use]
    pub fn key(&self) -> (&node::Port, node::PortDirection) {
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
    /// The nodes, in canonical order; [`NodeIndex`]s index this table.
    pub nodes: Vec<Node>,
    /// Boundary ports declared on the graph, in canonical order.
    /// Conventional `in`, `out`, and `err` rows are omitted when
    /// unannotated, but an explicit type constraint may restate them.
    pub boundary: Vec<PortDecl>,
    /// The wiring, desugared to explicit pairs, in canonical order —
    /// the same order the semantic hash sees.
    pub edges: Vec<Edge>,
    /// Case selections and manager destinations beside the spatial edges.
    /// A source absent from this sparse table selects its ordinary edges.
    pub routes: Vec<RouteRule>,
    /// Unflattened routing when the plan contains nested Graph boundaries.
    /// The validator derives the physical routes again from this inventory;
    /// boundaries remain structural seams, never executable pseudo-nodes.
    /// Empty for plans with no nested boundary or Graph Loop admission.
    pub boundary_routes: Vec<RouteRule>,
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
    pub(crate) fn endpoint_matches(&self, endpoint: &Endpoint, physical: &EdgeEnd) -> bool {
        endpoint.port == physical.port
            && match (&endpoint.owner, physical.node) {
                (EndpointOwner::Node(path), Some(slot)) => self
                    .nodes
                    .get(slot.index())
                    .is_some_and(|node| node.path == *path),
                (EndpointOwner::Graph(path), None) => path.is_root(),
                _ => false,
            }
    }

    fn route_destination_matches(&self, destination: &RouteDestination, edge: &Edge) -> bool {
        match (destination, &edge.to) {
            (RouteDestination::Port(endpoint), Some(physical)) => {
                self.endpoint_matches(endpoint, physical)
            }
            (RouteDestination::Discard, None) => true,
            _ => false,
        }
    }

    pub(crate) fn has_case_routes(&self, source: &EdgeEnd) -> bool {
        self.routes.iter().any(|route| {
            self.endpoint_matches(&route.source, source)
                && matches!(route.selection, RouteSelection::Cases(_))
        })
    }

    pub(crate) fn edge_is_selected(&self, edge: &Edge, value: &Value) -> bool {
        let mut constrained = false;
        for route in self
            .routes
            .iter()
            .filter(|route| self.endpoint_matches(&route.source, &edge.from))
        {
            constrained = true;
            let selected = route.selection.selected(value);
            if selected.is_some_and(|destinations| {
                destinations
                    .iter()
                    .any(|destination| self.route_destination_matches(destination, edge))
            }) {
                return true;
            }
        }
        !constrained
    }

    pub(crate) fn edge_cases(&self, edge: &Edge) -> Option<Vec<String>> {
        let mut cases = None;
        for route in self
            .routes
            .iter()
            .filter(|route| self.endpoint_matches(&route.source, &edge.from))
        {
            match &route.selection {
                RouteSelection::All(destinations) => {
                    if destinations
                        .iter()
                        .any(|destination| self.route_destination_matches(destination, edge))
                    {
                        return None;
                    }
                }
                RouteSelection::Cases(arms) => {
                    let selected = cases.get_or_insert_with(Vec::new);
                    for arm in arms {
                        if arm
                            .destinations
                            .iter()
                            .any(|destination| self.route_destination_matches(destination, edge))
                        {
                            selected.extend(arm.cases.iter().cloned());
                        }
                    }
                }
            }
        }
        if let Some(cases) = &mut cases {
            cases.sort();
            cases.dedup();
        }
        cases
    }

    /// Resolves one owner reference without trusting a freely constructed
    /// [`node::OwnerSlot`].
    #[must_use]
    pub fn owner(&self, slot: node::OwnerSlot) -> Option<&Owner> {
        self.owners.get(slot.index())
    }

    /// Complete boundary contract of one Graph owner.
    ///
    /// The root Graph already owns [`Self::boundary`], so its owner row keeps
    /// no duplicate claim. Nested Graphs carry their contract on the owner
    /// row because composition removes their boundary endpoints from the
    /// flattened edge table.
    #[must_use]
    pub fn graph_ports(&self, slot: node::OwnerSlot) -> Option<&[PortDecl]> {
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
    pub fn node_instance(&self, slot: NodeIndex) -> Option<&instance::InstanceSpec> {
        let node = self.nodes.get(slot.index())?;
        self.owner(node.owner).map(|owner| &owner.instance)
    }

    /// The canonical text of one edge — the same rendering the
    /// consumed cursors and the budget obligations seal.
    #[must_use]
    pub fn edge_text(&self, edge: &Edge) -> String {
        let end = |endpoint: &EdgeEnd| match endpoint.node {
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
    pub owner: node::OwnerSlot,
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

    /// An inbound Call or duplex Session has separate world-facing halves:
    /// delivering a response/message to `in` cannot cause a new root on `out`.
    pub(crate) fn input_drives_outputs(&self) -> bool {
        !self.connector().is_some_and(|spec| {
            spec.roles.has_inbound()
                && (spec.exchange == crate::connector::ConnectorExchange::Session
                    || (!spec.roles.has_outbound()
                        && spec.exchange == crate::connector::ConnectorExchange::Call))
        })
    }
}

/// The declaration represented by one owner row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerKind {
    /// A lexical Graph scope. The root App Graph is the unique parentless row.
    Graph,
    /// An executable Operator or Connector.
    Node(NodeIndex),
}

/// One lexical lifecycle and instance owner.
#[derive(Debug, Clone, PartialEq)]
pub struct Owner {
    /// Root-relative static owner path.
    pub path: node::OwnerPath,
    /// Enclosing Graph owner. Only the root App Graph has no parent.
    pub parent: Option<node::OwnerSlot>,
    /// The declaration this row preserves after DAG flattening.
    pub kind: OwnerKind,
    /// A nested Graph's complete boundary contract in canonical order. Node
    /// owners and the root Graph carry an empty table: Node ports remain on
    /// [`Node`], while the root contract has its existing single authority in
    /// [`ExecutionPlan::boundary`].
    pub ports: Vec<PortDecl>,
    /// Complete Extension specifications in canonical order.
    pub extensions: Vec<ExtensionSpec>,
    /// Orthogonal logical addressing and lifetime policy.
    pub instance: instance::InstanceSpec,
    /// Immutable bindings owned by this scope, in canonical name order.
    pub vals: Vec<OwnerValue>,
}

/// One fully configured Extension carried by a categorical plan.
///
/// The enum is deliberately closed. Recognizing a source name does not grant
/// it a transport tag: an Extension lands only together with its complete
/// Registry-free validation contract.
#[derive(Debug, Clone, PartialEq)]
pub enum ExtensionSpec {
    /// The unique root Graph's public deployment boundary.
    App,
    /// Finite temporal recursion over the owner's entry interface.
    Loop(Box<LoopSpec>),
    /// Bounded repetition of one pinned outbound Call operation.
    Retry(Box<RetrySpec>),
}

impl ExtensionSpec {
    /// Extension selected by this complete specification.
    #[must_use]
    pub const fn extension(&self) -> node::Extension {
        match self {
            Self::App => node::Extension::App,
            Self::Loop(_) => node::Extension::Loop,
            Self::Retry(_) => node::Extension::Retry,
        }
    }
}

impl Owner {
    /// The configured temporal recursion, independent of Extension order.
    #[must_use]
    pub fn loop_spec(&self) -> Option<&LoopSpec> {
        self.extensions
            .iter()
            .find_map(|extension| match extension {
                ExtensionSpec::Loop(spec) => Some(spec.as_ref()),
                ExtensionSpec::App | ExtensionSpec::Retry(_) => None,
            })
    }

    /// The configured pinned-operation policy, when this owner is a Call.
    #[must_use]
    pub fn retry_spec(&self) -> Option<&RetrySpec> {
        self.extensions
            .iter()
            .find_map(|extension| match extension {
                ExtensionSpec::Retry(spec) => Some(spec.as_ref()),
                ExtensionSpec::App | ExtensionSpec::Loop(_) => None,
            })
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

/// Complete Contract-checked policy for one pinned outbound Call operation.
#[derive(Debug, Clone, PartialEq)]
pub struct RetrySpec {
    /// Positive total bound, first attempt included.
    pub attempts: PlanConstant,
    /// Nonduplicated canonical terminal names, never runtime predicates.
    pub cases: Vec<String>,
    /// Original closure-based delay between attempts.
    pub delay: Option<DelayPolicy>,
    /// Resolved Contract request input.
    pub entry: node::Port,
    /// Resolved Contract terminal, with no contributed output or wrapper.
    pub terminal: node::Port,
}

/// Complete first-gate Loop transport.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopSpec {
    /// Resolved recurrent input; every admission of this run returns here.
    pub entry: node::Port,
    /// Canonical terminal business outputs. Calls use their Contract exit.
    pub terminals: Vec<node::Port>,
    /// Resolved host output for manager exhaustion; absent when impossible.
    pub expired: Option<LoopExpiredOutput>,
    /// Optional positive maximum number of body applications. A verified
    /// measure may stand alone; when both exist this remains the runtime cap.
    pub fuel: Option<PlanConstant>,
    /// Optional re-admission delay.
    pub delay: Option<DelayPolicy>,
    /// Optional small termination certificate, re-verified after decode.
    pub proof: Option<crate::loop_proof::Certificate>,
}

/// The existing host output chosen to publish `LoopExpired`, with its complete
/// effective union. This is not a new port or the set of business terminals.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopExpiredOutput {
    /// Existing output selected during checking.
    pub port: node::Port,
    /// Complete effective pin, independently re-derived after package decode.
    pub ty: DataType,
}

/// Closed delay policy shared by Loop applications and Retry attempts.
#[derive(Debug, Clone, PartialEq)]
pub enum DelayPolicy {
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

impl DelayPolicy {
    /// Positive delay after a one-based application/attempt ordinal, shared
    /// by recurrence policies. Saturation precedes multiplication overflow.
    /// `None` denotes malformed resolved bounds, never a zero-delay policy.
    #[must_use]
    pub fn duration_after(&self, ordinal: u64) -> Option<i64> {
        if ordinal == 0 {
            return None;
        }
        let duration = |constant: &PlanConstant| match constant.resolved {
            Value::Duration(value) if value > 0 => Some(value),
            _ => None,
        };
        match self {
            Self::Fixed { after } => duration(after),
            Self::Exponential { first, ceiling } => {
                let first = duration(first)?;
                let ceiling = duration(ceiling)?;
                if first > ceiling {
                    return None;
                }
                let scaled = u32::try_from(ordinal - 1)
                    .ok()
                    .filter(|shift| *shift < i64::BITS - 1)
                    .and_then(|shift| 1_i64.checked_shl(shift))
                    .and_then(|factor| first.checked_mul(factor))
                    .unwrap_or(ceiling);
                Some(scaled.min(ceiling))
            }
        }
    }
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
    /// Stable semantic contract identity (`std/process@4`).
    pub contract: crate::connector::ContractId,
    /// Inbound/outbound runtime roles required by the contract.
    pub roles: crate::connector::ConnectorDirection,
    /// Cardinality and terminal lifecycle required by the contract.
    pub exchange: crate::connector::ConnectorExchange,
    /// Original plan-owned Call guarantee. Non-Call profiles have none.
    /// It is never copied into a CallOpen or inferred from a partner key.
    pub idempotent: Option<PlanConstant>,
    /// The declared behavior, likewise typed.
    pub behavior: node::Behavior,
    /// The remaining effective attributes, either reduced or carrying their
    /// checked evaluation phase; the typed axes above are not repeated here.
    pub attributes: Vec<(String, AttrSetting)>,
}

impl ConnectorSpec {
    /// Only a validated true constant grants repeat-safety.
    #[must_use]
    pub fn is_idempotent(&self) -> bool {
        self.idempotent
            .as_ref()
            .is_some_and(|constant| constant.resolved == Value::Bool(true))
    }

    /// Closed Call guarantees derived from the exact Contract and profile.
    #[must_use]
    pub(crate) fn call_semantics(&self) -> Option<crate::connector::CallSemantics> {
        let mode = match self.attribute("mode") {
            Some(AttrSetting::Word(mode)) => Some(mode.as_str()),
            _ => None,
        };
        crate::connector::outbound_call_semantics(&self.contract, self.roles, self.exchange, mode)
    }

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

/// One Operator transition, with join admission separate from consumption.
#[derive(Debug, Clone, PartialEq)]
pub struct Handler {
    /// Operator index that owns this application transition. Graph scopes
    /// have no application handlers; Connector transitions follow a Contract.
    pub owner: NodeIndex,
    /// The declared activation pattern — not the causal trigger a
    /// transaction records; see [`node::HandlerActivation`].
    pub activation: node::HandlerActivation,
    /// Pre-consumption join admission predicate. A port filter belongs in
    /// `dispatch`, where its false branch still consumes the delivery.
    pub guard: Option<expr::Tree>,
    /// Typed decision and ordered atomic emissions.
    pub dispatch: Dispatch,
}

/// One planned edge.
#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    /// Lexical Graph owner in which this route was declared.
    pub owner: node::OwnerSlot,
    /// The emitting end.
    pub from: EdgeEnd,
    /// The receiving end; `None` is the explicit discard.
    pub to: Option<EdgeEnd>,
}

/// One resolved edge end; a boundary end has no node.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeEnd {
    /// The node, or `None` for the graph's boundary.
    pub node: Option<NodeIndex>,
    /// The port.
    pub port: node::Port,
}

/// Diagnostic and trace symbol information — a side table, never
/// identity material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolTable {
    /// Node index to node path text.
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
