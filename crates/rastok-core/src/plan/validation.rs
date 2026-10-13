//! Structural validation of an [`ExecutionPlan`] — every invariant
//! the machine dereferences without checking and every canonical
//! order the plan documents, held once, at a boundary, by a type.

use crate::instance;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use crate::datatype::{DataType, TypeError};
use crate::expr;
use crate::node;
use crate::plan::{
    DispatchStep, EdgeEnd, EmitValue, EvaluationSite, ExecutionPlan, Node, NodeExecution,
    NodeIndex, OwnerKind,
};
use crate::schema_annotation::{AnnotationTable, SchemaAnnotationError};

/// Why a plan is not executable — one named invariant violation. The
/// variants are semantic, not prose: a caller can match on the class
/// of defect; `Display` renders the human diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanValidationError {
    /// The independently checked routing inventory contradicts the plan.
    InvalidRouting(super::RouteValidationError),
    /// A possible output has no explicit spatial or manager destination.
    UnwiredOutput {
        /// The producing Executor.
        node: node::Path,
        /// The output requiring a destination.
        port: node::Port,
    },
    /// A handler decision contradicts its independently resolved environment.
    InvalidDispatch {
        /// Handler-table index.
        handler: usize,
        /// Exact path and violated decision rule.
        reason: super::DispatchValidationError,
    },
    /// A statement has no executable representation in the current package.
    UnsupportedStatement {
        /// Handler-table index.
        handler: usize,
        /// Named construct, never silently omitted from execution.
        statement: &'static str,
    },
    /// A DSL handler targets an Executor whose transitions are Contract-owned.
    NonOperatorHandler {
        /// The invalid handler target.
        node: node::Path,
    },
    /// The categorical owner table is absent or has no root App Graph.
    MissingRootOwner,
    /// An owner reference indexes past the owner table.
    OwnerOutOfBounds {
        /// The impossible reference.
        owner: node::OwnerSlot,
        /// The table it missed.
        owner_count: usize,
    },
    /// Two owners share one static lexical path.
    DuplicateOwnerPath {
        /// The repeated dotted path; empty denotes the root App Graph.
        path: String,
    },
    /// One owner does not form a valid parent-linked lexical tree.
    InvalidOwnerTree {
        /// The malformed row.
        owner: node::OwnerSlot,
    },
    /// A Node and its owner row do not point to one another.
    OwnerNodeMismatch {
        /// The Node slot.
        node: NodeIndex,
        /// The claimed owner row.
        owner: node::OwnerSlot,
    },
    /// An owner carries a malformed boundary or Extension contract.
    InvalidGraphContract {
        /// Static Graph path; `<root>` denotes the App.
        owner: String,
        /// Closed invariant that failed.
        detail: &'static str,
    },
    /// A Loop host does not prove one finite verdict per application.
    InvalidLoopControl {
        /// Static Graph path; `<root>` denotes the App.
        owner: String,
        /// Canonical activation or spatial path that exposes the defect.
        path: String,
        /// Closed proof obligation that failed.
        detail: &'static str,
    },
    /// A Retry policy contradicts the exact Call Contract or its constants.
    InvalidRetryPolicy {
        /// Static path of the Retry owner.
        owner: String,
        /// Failed independent obligation.
        detail: String,
    },
    /// An edge's lexical scope is absent or is not a Graph owner.
    InvalidEdgeScope {
        /// The claimed Graph owner.
        owner: node::OwnerSlot,
    },
    /// One owner declares an immutable binding twice.
    DuplicateOwnerValue {
        /// The static owner path.
        owner: String,
        /// The repeated binding.
        name: String,
    },
    /// An immutable value or Connector setting contradicts the phase derived
    /// from its expression and lexical dependencies.
    BindingSiteMismatch {
        /// The static owner path.
        owner: String,
        /// The malformed binding.
        name: String,
    },
    /// An immutable value or Connector setting reads outside its lexical
    /// environment.
    InvalidBinding {
        /// The static owner path.
        owner: String,
        /// The binding being checked.
        name: String,
        /// The unavailable resolved path.
        path: String,
    },
    /// Immutable bindings on one owner form a dependency cycle.
    CyclicOwnerValue {
        /// The static owner path.
        owner: String,
        /// One binding on the cycle.
        name: String,
    },
    /// A plan-phase binding's transported reduction is not the result of its
    /// transported expression.
    OwnerValueResolutionMismatch {
        /// The static owner path.
        owner: String,
        /// The lying binding.
        name: String,
    },
    /// A slot indexes past the node table.
    SlotOutOfBounds {
        /// The impossible reference.
        slot: NodeIndex,
        /// The table it missed.
        node_count: usize,
    },
    /// Two nodes share a path.
    DuplicateNodePath {
        /// The repeated path.
        path: node::Path,
    },
    /// Flattening lexical Graph boundaries exposed a cycle between executable
    /// Nodes. The shipped spatial graph is always a DAG, independently of how
    /// many composition seams the source crossed.
    WiringCycle {
        /// One Node on the cycle.
        node: node::Path,
    },
    /// One node declares a port twice.
    DuplicatePort {
        /// The declaring node.
        node: node::Path,
        /// The repeated port.
        port: node::Port,
    },
    /// One node declares a var twice.
    DuplicateVar {
        /// The declaring node.
        node: node::Path,
        /// The repeated name.
        name: String,
    },
    /// The boundary declares a port name twice.
    DuplicateBoundaryPort {
        /// The repeated port.
        port: node::Port,
    },
    /// A connector declares an attribute twice.
    DuplicateAttribute {
        /// The declaring node.
        node: node::Path,
        /// The repeated attribute.
        name: String,
    },
    /// A built-in Connector omits one effective sheet attribute.
    MissingContractAttribute {
        /// Connector node.
        node: node::Path,
        /// Required effective attribute.
        attribute: &'static str,
    },
    /// A built-in Connector attribute contradicts its Registry-free sheet
    /// mirror (shape, value, phase, or profile applicability).
    ContractAttributeMismatch {
        /// Connector node.
        node: node::Path,
        /// Contradicting attribute.
        attribute: String,
    },
    /// A default-lineage inbound Call response can be reached through a
    /// root-mixing join. The trigger root is not a stable request selector;
    /// the contract therefore requires an explicit correlator.
    AmbiguousInboundCallCorrelation {
        /// Inbound Call Connector receiving the response.
        node: node::Path,
    },
    /// A `Process` connector's `codec` is outside the runtime's
    /// dispatch domain — accepting it would silently run as
    /// `lines`.
    UnknownCodec {
        /// The declaring node.
        node: node::Path,
    },
    /// A `Process` connector carries no `codec` at all — the sheet
    /// requires one, so a plan without it never left the compiler.
    MissingCodec {
        /// The declaring node.
        node: node::Path,
    },
    /// A `Process` connector carries no effective `err_codec` even though
    /// the sheet defaults it before lowering.
    MissingErrCodec {
        /// The declaring node.
        node: node::Path,
    },
    /// A `Process` connector's stderr framing is outside `lines | bytes`.
    UnknownErrCodec {
        /// The declaring node.
        node: node::Path,
    },
    /// A connector some edge delivers to exposes neither the outbound role
    /// nor the inbound-Call response protocol required to consume it.
    CallableConnectorWithoutOutboundRole {
        /// The declaring node.
        node: node::Path,
    },
    /// One node declares two handlers for one activation — the
    /// second would silently shadow the first at dispatch.
    DuplicateHandler {
        /// The declaring node.
        node: node::Path,
        /// The doubled activation — carried whole, so a future join
        /// activation changes nothing about this variant's shape.
        activation: node::HandlerActivation,
    },
    /// A join activation has fewer than two distinct input ports.
    InvalidJoinActivation {
        /// The declaring node.
        node: node::Path,
    },
    /// One input port participates in more than one activation. Dispatch must
    /// never guess whether an arrival is a single-port firing or join input.
    AmbiguousInputActivation {
        /// The declaring node.
        node: node::Path,
        /// The multiply-owned input port.
        port: node::Port,
    },
    /// A join retention bound is zero or otherwise outside its closed domain.
    InvalidJoinBound {
        /// The declaring node.
        node: node::Path,
    },
    /// A bounded join lacks the conventional typed eviction output.
    MissingJoinExpiryPort {
        /// The declaring node.
        node: node::Path,
    },
    /// Identity emission has no unique input envelope under a join.
    AmbiguousJoinIdentity {
        /// The declaring node.
        node: node::Path,
    },
    /// A required metadata read can observe a field removed by a preceding
    /// join's CommonOrAbsent merge. An explicit write must select the value
    /// before the field becomes required again.
    AmbiguousJoinMetadataRead {
        /// The owner whose expression performs the required read.
        owner: String,
        /// The complete `meta.app.*` or `meta.ext.*` path.
        field: String,
    },
    /// A lifecycle activation belongs to a later executable slice. Format v10
    /// retains the closed registry without silently enabling it.
    UnsupportedLifecycleHandler {
        /// The declaring node.
        node: node::Path,
        /// The unsupported lifecycle name.
        lifecycle: node::Lifecycle,
    },
    /// A start/stop hook has no admission envelope from which an instance or
    /// causal parent could be created.
    LifecycleWithoutInput {
        /// The declaring node.
        node: node::Path,
        /// The impossible lifecycle hook.
        lifecycle: node::Lifecycle,
    },
    /// A Rastok expression map literal names one key twice.
    DuplicateMapKey {
        /// The repeated key.
        key: String,
    },
    /// A resolved built-in Connector table omits one promised port.
    MissingContractPort {
        /// Connector node.
        node: node::Path,
        /// Missing port.
        port: &'static str,
    },
    /// A built-in Connector claims runtime roles that its stable contract id
    /// does not define.
    ContractRoleMismatch {
        /// Connector node.
        node: node::Path,
    },
    /// A built-in Connector claims an exchange lifecycle that its stable
    /// contract id does not define.
    ContractExchangeMismatch {
        /// Connector node.
        node: node::Path,
    },
    /// The conventional terminal row contradicts the generic exchange fact.
    ExchangeTerminalMismatch {
        /// Connector node.
        node: node::Path,
        /// Exchange whose terminal cardinality was contradicted.
        exchange: crate::connector::ConnectorExchange,
    },
    /// The role set cannot expose this exchange's complete generic protocol.
    ExchangeRoleMismatch {
        /// Connector node.
        node: node::Path,
        /// Incompatible exchange.
        exchange: crate::connector::ConnectorExchange,
        /// Incompatible roles.
        roles: crate::connector::ConnectorDirection,
    },
    /// A resolved built-in Connector row disagrees with its self-contained
    /// contract rule (direction, routing policy, or effective type).
    ContractPortMismatch {
        /// Connector node.
        node: node::Path,
        /// Mismatching port.
        port: node::Port,
    },
    /// A built-in Connector table carries a row its resolved contract does
    /// not declare.
    UnexpectedContractPort {
        /// Connector node.
        node: node::Path,
        /// Unexpected port.
        port: node::Port,
    },
    /// A referenced port exists, but not in the referenced role —
    /// an edge from an input, an edge into an output, an activation
    /// on an output, an emission on an input.
    WrongPortDirection {
        /// The owning node; `None` is the boundary.
        node: Option<node::Path>,
        /// The port.
        port: node::Port,
        /// The role the reference required.
        expected: node::PortDirection,
    },
    /// An edge references a boundary port the plan does not know.
    UnknownBoundaryPort {
        /// The unknown port.
        port: node::Port,
    },
    /// A handler writes a var its node does not declare.
    UnknownVariable {
        /// The writing node.
        node: node::Path,
        /// The undeclared name.
        name: String,
    },
    /// A materialized constant or compiled expression tree contradicts its
    /// claimed resolved type or canonical constructor shape.
    UnrepresentableConstant,
    /// A metadata write outside the travelling zone schema — the
    /// checker's grid, re-verified without a Registry
    /// (D-2026-08-10-1).
    UnknownMetaField {
        /// `"app"` or `"ext"`.
        zone: &'static str,
        /// The flat dotted field.
        field: alloc::string::String,
    },
    /// A schema type no metadata value can satisfy.
    MetaSchemaType {
        /// The zone.
        zone: &'static str,
        /// The unsatisfiable field.
        field: alloc::string::String,
    },
    /// A flat key whose spelling breaks the dotted form — empty, a
    /// leading/trailing dot, or an empty segment; the nested
    /// projection could never reach it.
    MetaFieldMalformed {
        /// The zone.
        zone: &'static str,
        /// The malformed key.
        field: alloc::string::String,
    },
    /// Two asset blobs carrying one digest — the table is
    /// content-addressed, one content one blob.
    DuplicateAsset {
        /// The later duplicate's index.
        index: u32,
    },
    /// A blob whose declared digest is not `blake3(bytes)` — the
    /// identity would silently change across an encode/decode trip.
    AssetDigestMismatch {
        /// The lying blob.
        index: u32,
    },
    /// The asset table out of digest order — one content, one
    /// canonical layout (D-2026-08-10-5 as amended).
    AssetOrder {
        /// The out-of-place blob.
        index: u32,
    },
    /// An asset (or the table) beyond the fixed v1 budgets.
    AssetBudget {
        /// The offending blob, `u32::MAX` for the total.
        index: u32,
        /// The measured size.
        bytes: u64,
    },
    /// An `Expr::Asset` index past the asset table.
    AssetOutOfBounds {
        /// The impossible reference.
        index: u32,
        /// The table it missed.
        count: usize,
    },
    /// Two flat keys where one prefixes the other — the nested
    /// projection cannot hold both.
    MetaPrefixCollision {
        /// The shorter (leaf) key.
        leaf: alloc::string::String,
        /// The longer (branch-making) key.
        branch: alloc::string::String,
    },
    /// A reference to a connector port beyond the default trio. The
    /// RESOLVED table travels in the plan (the future needs it), but
    /// this machine's OutboundConnectorDriver seam does not carry the port yet —
    /// `CallOpen`, the response protocol, and the outcome trigger all
    /// speak `in`/`out`/`err`. A plan wiring such a port is valid in
    /// shape and NOT executable faithfully, so it refuses here; the
    /// restriction lifts when the port enters `CallOpen`.
    UnsupportedConnectorPort {
        /// The connector.
        node: node::Path,
        /// The not-yet-executable port.
        port: node::Port,
    },
    /// An edge whose two PINNED ends disagree — the checker refuses
    /// this at source; a foreign package must not smuggle it past
    /// the type into a machine that trusts its wiring.
    MismatchedEdgePins {
        /// The target port.
        port: node::Port,
        /// The target's pin.
        expected: DataType,
        /// The source's pin.
        found: DataType,
    },
    /// A port survived every inference constraint without one effective
    /// logical type. `variant` is never substituted for this state.
    UnresolvedPortType {
        /// The owning node; `None` is the graph boundary.
        node: Option<node::Path>,
        /// The unresolved port.
        port: node::Port,
    },
    /// Two constraints on one port cannot both hold.
    ConflictingPortTypes {
        /// The owning node; `None` is the graph boundary.
        node: Option<node::Path>,
        /// The constrained port.
        port: node::Port,
        /// The type already required there.
        expected: DataType,
        /// The incompatible producing type.
        found: DataType,
    },
    /// A logical type violates the canonical algebra's global budgets. Local
    /// parameter and uniqueness invariants are already held by constructors;
    /// this catches oversized expanded trees before any recursive consumer.
    InvalidDataType {
        /// Where the type occurs in the plan.
        location: String,
        /// The violated type invariant.
        reason: TypeError,
    },
    /// Sparse policy metadata does not hold against its adjacent resolved
    /// logical schema or a pinned semantic registry.
    InvalidSchemaAnnotations {
        /// Stable plan location of the annotated schema.
        location: String,
        /// Exact structural, target, carrier, or registry failure.
        reason: SchemaAnnotationError,
    },
    /// A canonical operation cannot derive the policy table transported at
    /// its result, or alternative producers disagree on exact accounting
    /// meaning.
    InvalidAnnotationPropagation {
        /// Stable plan location of the operation or result.
        location: String,
        /// Exact propagation invariant that failed.
        reason: crate::data_policy::PropagationError,
    },
    /// Lowering left an `asset()` call in an executable expression instead of
    /// binding it to the plan's content-addressed asset table.
    UnboundAssetCall,
    /// A transported instance-key expression does not yield a value in the
    /// canonical scalar/enum-or-struct key domain.
    InvalidInstanceKey {
        /// The keyed owner path; empty denotes the root App Graph.
        owner: String,
        /// The expression's resolved type.
        ty: DataType,
    },
    /// A transported instance expression reads outside the complete immutable
    /// entering envelope available before routing commits.
    InvalidInstanceBinding {
        /// The keyed owner path; empty denotes the root App Graph.
        owner: String,
        /// The forbidden resolved path.
        path: String,
    },
    /// Different input ports make one data-dependent instance expression
    /// resolve to different logical key types.
    ConflictingInstanceKeyTypes {
        /// The keyed owner path; empty denotes the root App Graph.
        owner: String,
        /// Type resolved for the first input family.
        expected: DataType,
        /// Incompatible type resolved for another input family.
        found: DataType,
    },
    /// A data-dependent key has no resolved admission signature against which
    /// the package validator can re-derive its type.
    UnresolvedInstanceAdmission {
        /// The keyed owner path; empty denotes the root App Graph.
        owner: String,
    },
    /// A transported key's claimed result type differs from the type
    /// re-derived from its expression and immutable admission signature.
    InstanceKeyTypeMismatch {
        /// The keyed owner path; empty denotes the root App Graph.
        owner: String,
        /// Type derived from the transported expression.
        expected: DataType,
        /// Type claimed by the transported expression tree.
        found: DataType,
    },
    /// A transported instance lifetime with a duration is not positive.
    InvalidInstanceLifetime {
        /// The owner path; empty denotes the root App Graph.
        owner: String,
    },
    /// A Closing budget is zero or negative.
    InvalidClosingPolicy {
        /// The owner path; empty denotes the root App Graph.
        owner: String,
    },
    /// A finite owner can close while its subtree contains external calls,
    /// but no explicit drain/settlement policy was transported.
    MissingClosingPolicy {
        /// The outbound Connector proving that cancellation may be required.
        connector: String,
        /// The finite owner whose close governs the subtree.
        owner: String,
    },
    /// An `on stop` handler recursively requests the same close.
    RecursiveStop {
        /// The offending node.
        node: node::Path,
    },
    /// A collection the plan documents as canonically ordered is not.
    NonCanonicalOrder {
        /// Which collection broke the order.
        collection: &'static str,
    },
    /// The plan's canonical body would nest beyond the format
    /// reader's budget — the one plan `encode` would write that
    /// `decode` could not read back, refused here so it cannot exist.
    NestingExceeded {
        /// The deepest level the body would reach (root = 0).
        depth: usize,
        /// The reader's budget ([`crate::canonical_cbor::MAX_NESTING`]).
        bound: usize,
    },
}

impl core::fmt::Display for PlanValidationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidRouting(error) => error.fmt(f),
            Self::UnwiredOutput { node, port } => write!(
                f,
                "invalid plan: `{node}.{port}` can be produced but has no destination; route it or write `{node}.{port} -> nothing`"
            ),
            Self::InvalidDispatch { handler, reason } => {
                write!(f, "invalid plan: handler {handler} {reason}")
            }
            Self::UnsupportedStatement { handler, statement } => {
                write!(f, "invalid plan: handler {handler} uses {statement}")
            }
            Self::NonOperatorHandler { node } => write!(
                f,
                "invalid plan: Connector `{node}` cannot own DSL handlers; its Contract defines its transitions"
            ),
            Self::MissingRootOwner => {
                f.write_str("invalid plan: the owner table has no root App Graph")
            }
            Self::OwnerOutOfBounds { owner, owner_count } => write!(
                f,
                "invalid plan: owner {} of {owner_count} owners",
                owner.index()
            ),
            Self::DuplicateOwnerPath { path } => write!(
                f,
                "invalid plan: duplicate owner path `{}`",
                if path.is_empty() { "<root>" } else { path }
            ),
            Self::InvalidOwnerTree { owner } => write!(
                f,
                "invalid plan: owner {} does not form a canonical lexical tree",
                owner.index()
            ),
            Self::OwnerNodeMismatch { node, owner } => write!(
                f,
                "invalid plan: node {} and owner {} do not refer to each other",
                node.index(),
                owner.index()
            ),
            Self::InvalidGraphContract { owner, detail } => write!(
                f,
                "invalid plan: owner `{owner}` carries an invalid boundary or Extension contract: {detail}"
            ),
            Self::InvalidLoopControl {
                owner,
                path,
                detail,
            } => write!(
                f,
                "invalid plan: cannot prove that Loop `{owner}` always terminates at `{path}`: {detail}"
            ),
            Self::InvalidRetryPolicy { owner, detail } => {
                write!(f, "invalid Retry `{owner}`: {detail}")
            }
            Self::InvalidEdgeScope { owner } => write!(
                f,
                "invalid plan: edge scope {} is not a Graph owner shared by both endpoints",
                owner.index()
            ),
            Self::DuplicateOwnerValue { owner, name } => write!(
                f,
                "invalid plan: owner `{owner}` declares immutable value `{name}` twice"
            ),
            Self::BindingSiteMismatch { owner, name } => write!(
                f,
                "invalid plan: immutable binding `{owner}.{name}` contradicts its evaluation site"
            ),
            Self::InvalidBinding { owner, name, path } => write!(
                f,
                "invalid plan: immutable binding `{owner}.{name}` cannot read `{path}` at its evaluation site"
            ),
            Self::CyclicOwnerValue { owner, name } => write!(
                f,
                "invalid plan: immutable value `{owner}.{name}` participates in a dependency cycle"
            ),
            Self::OwnerValueResolutionMismatch { owner, name } => write!(
                f,
                "invalid plan: immutable value `{owner}.{name}` does not reduce to its transported value"
            ),
            Self::SlotOutOfBounds { slot, node_count } => write!(
                f,
                "invalid plan: slot {} of {node_count} nodes",
                slot.index()
            ),
            Self::DuplicateNodePath { path } => {
                write!(f, "invalid plan: duplicate node path `{path}`")
            }
            Self::WiringCycle { node } => write!(
                f,
                "invalid plan: the wiring cycles through `{node}`; executable edges form a DAG"
            ),
            Self::UnknownMetaField { zone, field } => write!(
                f,
                "invalid plan: a `meta.{zone}.{field}` write outside the travelling schema"
            ),
            Self::MetaSchemaType { zone, field } => write!(
                f,
                "invalid plan: `meta.{zone}.{field}` declares a type no value can satisfy"
            ),
            Self::MetaFieldMalformed { zone, field } => write!(
                f,
                "invalid plan: `meta.{zone}.{field}` is not a well-formed dotted key"
            ),
            Self::DuplicateAsset { index } => {
                write!(f, "invalid plan: asset {index} repeats an earlier digest")
            }
            Self::AssetDigestMismatch { index } => {
                write!(f, "invalid plan: asset {index} lies about its digest")
            }
            Self::AssetOrder { index } => {
                write!(f, "invalid plan: asset {index} is out of digest order")
            }
            Self::AssetBudget { index, bytes } => {
                write!(
                    f,
                    "invalid plan: asset budget breached at {index} ({bytes} bytes)"
                )
            }
            Self::AssetOutOfBounds { index, count } => {
                write!(f, "invalid plan: asset {index} of a {count}-blob table")
            }
            Self::MetaPrefixCollision { leaf, branch } => write!(
                f,
                "invalid plan: `{leaf}` is a leaf and `{branch}` makes it a branch — flat keys are prefix-free"
            ),
            Self::DuplicatePort { node, port } => {
                write!(f, "invalid plan: duplicate port `{port}` on `{node}`")
            }
            Self::DuplicateVar { node, name } => {
                write!(f, "invalid plan: duplicate var `{name}` on `{node}`")
            }
            Self::DuplicateBoundaryPort { port } => {
                write!(f, "invalid plan: duplicate boundary port `{port}`")
            }
            Self::DuplicateAttribute { node, name } => {
                write!(f, "invalid plan: duplicate attribute `{name}` on `{node}`")
            }
            Self::MissingContractAttribute { node, attribute } => write!(
                f,
                "connector `{node}` omits required effective attribute `{attribute}`"
            ),
            Self::ContractAttributeMismatch { node, attribute } => write!(
                f,
                "connector `{node}` carries an invalid effective `{attribute}` attribute"
            ),
            Self::AmbiguousInboundCallCorrelation { node } => write!(
                f,
                "connector `{node}` receives a response through a root-mixing join and therefore requires an explicit `correlate` attribute"
            ),
            Self::UnknownCodec { node } => {
                write!(
                    f,
                    "invalid plan: `{node}` carries a codec outside the contract domain (bytes | jsonl | lines)"
                )
            }
            Self::MissingCodec { node } => {
                write!(
                    f,
                    "invalid plan: `{node}` is a Process connector without a codec — the sheet requires one"
                )
            }
            Self::MissingErrCodec { node } => write!(
                f,
                "invalid plan: `{node}` is a Process connector without effective `err_codec`"
            ),
            Self::UnknownErrCodec { node } => write!(
                f,
                "invalid plan: `{node}` carries `err_codec` outside lines | bytes"
            ),
            Self::CallableConnectorWithoutOutboundRole { node } => write!(
                f,
                "invalid plan: `{node}` receives deliveries but its contract exposes neither an outbound call nor an inbound-Call response sink"
            ),
            Self::DuplicateHandler { node, activation } => {
                write!(
                    f,
                    "invalid plan: `{node}` declares two handlers on `{activation}`"
                )
            }
            Self::InvalidJoinActivation { node } => {
                write!(
                    f,
                    "`{node}` declares a join without two distinct input ports"
                )
            }
            Self::AmbiguousInputActivation { node, port } => write!(
                f,
                "input `{node}.{port}` participates in more than one handler activation"
            ),
            Self::InvalidJoinBound { node } => {
                write!(f, "`{node}` declares a non-positive join retention bound")
            }
            Self::MissingJoinExpiryPort { node } => write!(
                f,
                "bounded join `{node}` requires `expired` with the canonical JoinExpired type"
            ),
            Self::AmbiguousJoinIdentity { node } => write!(
                f,
                "join `{node}` cannot use identity emission because it has no single input"
            ),
            Self::AmbiguousJoinMetadataRead { owner, field } => write!(
                f,
                "required read of `{field}` on `{owner}` may be absent after a join; select one named input with an explicit metadata write first"
            ),
            Self::UnsupportedLifecycleHandler { node, lifecycle } => write!(
                f,
                "invalid plan: `{node}` declares the unsupported lifecycle handler `l:{lifecycle}`"
            ),
            Self::LifecycleWithoutInput { node, lifecycle } => write!(
                f,
                "invalid plan: `{node}` declares `l:{lifecycle}` but has no input envelope to cause an instance"
            ),
            Self::DuplicateMapKey { key } => {
                write!(f, "invalid plan: duplicate map literal key `{key}`")
            }
            Self::MissingContractPort { node, port } => write!(
                f,
                "invalid plan: `{node}` omits resolved contract port `{port}`"
            ),
            Self::ContractRoleMismatch { node } => write!(
                f,
                "invalid plan: `{node}` carries roles outside its stable connector contract"
            ),
            Self::ContractExchangeMismatch { node } => write!(
                f,
                "invalid plan: `{node}` carries an exchange outside its stable connector contract"
            ),
            Self::ExchangeTerminalMismatch { node, exchange } => write!(
                f,
                "invalid plan: `{node}` carries a terminal `exit` table incompatible with {exchange}"
            ),
            Self::ExchangeRoleMismatch {
                node,
                exchange,
                roles,
            } => write!(
                f,
                "invalid plan: `{node}` carries {roles} roles incompatible with its {exchange} exchange"
            ),
            Self::ContractPortMismatch { node, port } => write!(
                f,
                "invalid plan: `{node}.{port}` disagrees with the resolved connector contract"
            ),
            Self::UnexpectedContractPort { node, port } => write!(
                f,
                "invalid plan: `{node}` carries undeclared contract port `{port}`"
            ),
            Self::WrongPortDirection {
                node,
                port,
                expected,
            } => match node {
                Some(node) => write!(
                    f,
                    "invalid plan: `{node}.{port}` is not usable as {expected:?}"
                ),
                None => write!(
                    f,
                    "invalid plan: boundary port `{port}` is not usable as graph {expected:?}"
                ),
            },
            Self::UnknownBoundaryPort { port } => {
                write!(f, "invalid plan: undeclared boundary port `{port}`")
            }
            Self::UnknownVariable { node, name } => {
                write!(f, "invalid plan: `{node}` writes undeclared var `{name}`")
            }
            Self::UnrepresentableConstant => f.write_str(
                "invalid plan: a constant outside the source-expressible forms \
                 (a non-empty struct, a laziness witness, or a decimal the \
                 written-scale rules refuse)",
            ),
            Self::UnsupportedConnectorPort { node, port } => write!(
                f,
                "invalid plan: `{node}.{port}` is a contract port this machine cannot execute yet"
            ),
            Self::MismatchedEdgePins {
                port,
                expected,
                found,
            } => write!(
                f,
                "invalid plan: `{port}` is pinned to {expected:?}, its source to {found:?}"
            ),
            Self::UnresolvedPortType { node, port } => match node {
                Some(node) => write!(
                    f,
                    "invalid plan: `{node}.{port}` has no effective type after inference"
                ),
                None => write!(
                    f,
                    "invalid plan: boundary port `{port}` has no effective type after inference"
                ),
            },
            Self::ConflictingPortTypes {
                node,
                port,
                expected,
                found,
            } => match node {
                Some(node) => write!(
                    f,
                    "invalid plan: `{node}.{port}` requires {expected}, but also receives {found}"
                ),
                None => write!(
                    f,
                    "invalid plan: boundary port `{port}` requires {expected}, but also receives {found}"
                ),
            },
            Self::InvalidDataType { location, reason } => {
                write!(f, "invalid plan: type at {location} is invalid: {reason}")
            }
            Self::InvalidSchemaAnnotations { location, reason } => {
                write!(
                    f,
                    "invalid plan: annotations at {location} are invalid: {reason}"
                )
            }
            Self::InvalidAnnotationPropagation { location, reason } => write!(
                f,
                "invalid plan: annotation propagation at {location} failed: {reason}"
            ),
            Self::UnboundAssetCall => write!(
                f,
                "invalid plan: an asset() call was not bound to the plan asset table"
            ),
            Self::InvalidInstanceKey { owner, ty } => write!(
                f,
                "invalid plan: the instance key on `{owner}` has non-canonical key type {ty}"
            ),
            Self::InvalidInstanceBinding { owner, path } => write!(
                f,
                "invalid plan: the instance key on `{owner}` reads `{path}` outside its immutable admission context"
            ),
            Self::ConflictingInstanceKeyTypes {
                owner,
                expected,
                found,
            } => write!(
                f,
                "invalid plan: the instance key on `{owner}` resolves to both {expected} and {found} across its input ports"
            ),
            Self::UnresolvedInstanceAdmission { owner } => write!(
                f,
                "invalid plan: the data-dependent instance key on `{owner}` has no resolved admission signature"
            ),
            Self::InstanceKeyTypeMismatch {
                owner,
                expected,
                found,
            } => write!(
                f,
                "invalid plan: the instance key on `{owner}` claims {found}, but its admission expression resolves to {expected}"
            ),
            Self::InvalidInstanceLifetime { owner } => write!(
                f,
                "invalid plan: the instance lifetime on `{owner}` is not positive"
            ),
            Self::InvalidClosingPolicy { owner } => write!(
                f,
                "invalid plan: the Closing budgets on `{owner}` must both be positive"
            ),
            Self::MissingClosingPolicy { connector, owner } => write!(
                f,
                "invalid plan: stoppable owner `{owner}` contains outbound Connector `{connector}` but declares no bounded Closing policy"
            ),
            Self::RecursiveStop { node } => write!(
                f,
                "invalid plan: `on stop` for `{node}` cannot request closure again"
            ),
            Self::NonCanonicalOrder { collection } => {
                write!(f, "invalid plan: `{collection}` is not in canonical order")
            }
            Self::NestingExceeded {
                depth,
                bound: budget,
            } => write!(
                f,
                "invalid plan: the body nests {depth} levels deep; the format reads at most {budget}"
            ),
        }
    }
}

impl core::error::Error for PlanValidationError {}

impl From<super::RouteValidationError> for PlanValidationError {
    fn from(error: super::RouteValidationError) -> Self {
        Self::InvalidRouting(error)
    }
}

/// An [`ExecutionPlan`] whose structural invariants have been
/// verified: every slot in bounds, every referenced port declared in
/// the referenced role — boundary conventions inverted, as the graph
/// sees them — every handler owned by its node, every state write
/// resolvable, names unique, and every collection the plan documents
/// as canonical actually in canonical order. The field is private
/// and there is no unverified constructor and no mutable access —
/// holding the type IS holding the property. The machine and the
/// codec accept only this type.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedExecutionPlan {
    plan: ExecutionPlan,
    resolved_ports: super::type_resolution::ResolvedPortTable,
    handler_context: Vec<HandlerContextUse>,
}

/// Derived context demand for one handler. It never travels in a package:
/// validation computes it from the already canonical expression trees once,
/// so a context-insensitive firing does no AST walk or metadata construction
/// on the hot path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct HandlerContextUse {
    /// Guard selection can observe the firing instant.
    pub(crate) guard: bool,
    /// The selected body can observe the firing instant.
    pub(crate) body: bool,
    /// A chosen expression may read the full metadata value.
    pub(crate) meta: bool,
}

fn handler_context_use(handler: &crate::plan::Handler) -> HandlerContextUse {
    let reads =
        |tree: &expr::Tree| super::type_resolution::expression_reads_root(&tree.root, "now");
    HandlerContextUse {
        guard: handler.guard.as_ref().is_some_and(reads),
        body: handler
            .dispatch
            .predicates()
            .chain(handler.dispatch.steps().flat_map(DispatchStep::expressions))
            .any(reads),
        meta: handler
            .guard
            .iter()
            .chain(handler.dispatch.predicates())
            .chain(handler.dispatch.steps().flat_map(DispatchStep::expressions))
            .any(|tree| super::type_resolution::expression_reads_root(&tree.root, "meta")),
    }
}

impl ValidatedExecutionPlan {
    /// Validates a raw plan.
    ///
    /// Raw-plan contract: the checks themselves never recurse before
    /// the nesting budget holds, but the plan's own `Drop` does — a
    /// hand-built plan nesting beyond the machine stack cannot even
    /// be destroyed safely, and is outside this function's domain.
    /// Both real sources (the compiler, gated at parse; the decoder,
    /// bounded by its reader) sit far below that.
    ///
    /// # Errors
    ///
    /// The first violated invariant, named.
    pub fn try_new(plan: ExecutionPlan) -> Result<Self, PlanValidationError> {
        Self::try_new_with_key_mode(plan, InstanceKeyTypeMode::Verify)
    }

    /// Closes the one inference variable intentionally left by source
    /// lowering: a data-dependent instance key whose input port type is
    /// solved by the core port resolver. Alternative frontends may use this
    /// boundary; package decoding and freely constructed plans must use
    /// [`Self::try_new`] so transported type claims are never normalized.
    pub fn try_from_lowered(plan: ExecutionPlan) -> Result<Self, PlanValidationError> {
        Self::try_new_with_key_mode(plan, InstanceKeyTypeMode::Materialize)
    }

    fn try_new_with_key_mode(
        mut plan: ExecutionPlan,
        key_mode: InstanceKeyTypeMode,
    ) -> Result<Self, PlanValidationError> {
        validate(&plan)?;
        super::retry::validate(&plan)?;
        let declared_ports = super::type_resolution::resolve(&plan)?;
        super::routing_validation::validate_plan_routing(&plan, &declared_ports)?;
        super::composition::validate(&plan, &declared_ports)?;
        verify_instance_key_types(&mut plan, &declared_ports, key_mode)?;
        if key_mode == InstanceKeyTypeMode::Materialize {
            let signatures: Vec<_> = plan
                .handlers
                .iter()
                .map(|handler| handler_signature(&plan, &declared_ports, handler))
                .collect();
            for (handler, signature) in plan.handlers.iter_mut().zip(signatures) {
                if let Some(signature) = signature {
                    super::dispatch_resolution::resolve(&mut handler.dispatch, signature);
                }
            }
        }
        handler_decisions_hold(&plan, &declared_ports)?;
        super::productibility::validate(&plan, &declared_ports)?;
        annotations_well_formed(&plan, &declared_ports)?;
        let resolved_ports = super::annotation_resolution::resolve(&plan, &declared_ports)
            .map_err(|error| PlanValidationError::InvalidAnnotationPropagation {
                location: error.location,
                reason: error.reason,
            })?;
        // Raw tables remain declarations; the resolved table is the separately
        // derived effective policy transported as categorical evidence.
        annotations_well_formed(&plan, &resolved_ports)?;
        let depth = super::serialization::deepest_level_with_resolved(&plan, &resolved_ports);
        if depth >= crate::canonical_cbor::MAX_NESTING {
            return Err(PlanValidationError::NestingExceeded {
                depth,
                bound: crate::canonical_cbor::MAX_NESTING,
            });
        }
        let handler_context = plan.handlers.iter().map(handler_context_use).collect();
        Ok(Self {
            plan,
            resolved_ports,
            handler_context,
        })
    }

    /// Read access to the validated plan.
    #[must_use]
    pub fn plan(&self) -> &ExecutionPlan {
        &self.plan
    }

    /// Effective logical types of every graph-boundary and node port.
    ///
    /// Unlike raw source declarations, this table contains no inference
    /// variable or recovery hole and includes effective data policy derived
    /// through canonical operations. Categorical transport carries it and the
    /// decoder reconstructs it from raw constraints before requiring equality.
    #[must_use]
    pub const fn resolved_ports(&self) -> &super::type_resolution::ResolvedPortTable {
        &self.resolved_ports
    }

    /// Context demands in exact handler-table order. This is derived from the
    /// validated trees and deliberately absent from categorical transport.
    #[must_use]
    pub(crate) fn handler_context(&self) -> &[HandlerContextUse] {
        &self.handler_context
    }

    /// Gives the raw plan back — the validity claim does not travel
    /// with it.
    #[must_use]
    pub fn into_inner(self) -> ExecutionPlan {
        self.plan
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstanceKeyTypeMode {
    Verify,
    Materialize,
}

impl TryFrom<ExecutionPlan> for ValidatedExecutionPlan {
    type Error = PlanValidationError;

    fn try_from(plan: ExecutionPlan) -> Result<Self, Self::Error> {
        Self::try_new(plan)
    }
}

// Read-only ergonomics: the plan's tables stay reachable through the
// wrapper. There is deliberately NO DerefMut — mutation would void
// the property the type asserts.
impl core::ops::Deref for ValidatedExecutionPlan {
    type Target = ExecutionPlan;

    fn deref(&self) -> &Self::Target {
        &self.plan
    }
}

/// Re-derives every instance-key type from the immutable admission signature.
/// Strict validation compares that fact with the transported claim. The
/// source-lowering mode may additionally close its exact data-input
/// placeholder, including inside a structured key; the general port solver
/// remains the sole inference authority.
fn verify_instance_key_types(
    plan: &mut ExecutionPlan,
    ports: &super::type_resolution::ResolvedPortTable,
    mode: InstanceKeyTypeMode,
) -> Result<(), PlanValidationError> {
    for index in 0..plan.owners.len() {
        let Some(key) = plan.owners[index].instance.key.as_ref() else {
            continue;
        };
        let owner_name = plan.owners[index].path.to_string();
        let kind = plan.owners[index].kind.clone();
        let reads_data = super::type_resolution::expression_reads_data(&key.root);
        let resolved = if reads_data {
            let mut resolved: Option<DataType> = None;
            let inputs: alloc::vec::Vec<DataType> = match kind {
                OwnerKind::Node(slot) => ports
                    .node(slot)
                    .unwrap_or(&[])
                    .iter()
                    .filter(|port| port.direction() == node::PortDirection::Input)
                    .map(|port| port.ty().clone())
                    .collect(),
                OwnerKind::Graph if index == 0 => ports
                    .boundary()
                    .iter()
                    .filter(|port| port.direction() == node::PortDirection::Input)
                    .map(|port| port.ty().clone())
                    .collect(),
                OwnerKind::Graph if plan.owners[index].loop_spec().is_some() => {
                    // The manager admits the typed Graph entry before any
                    // fan-out. A descendant Call may accept variant without
                    // weakening that opening's type or reselecting its key.
                    let owner = &plan.owners[index];
                    owner
                        .ports
                        .iter()
                        .filter(|port| {
                            owner
                                .loop_spec()
                                .is_some_and(|spec| spec.entry == port.port)
                        })
                        .filter_map(|port| port.ty.clone())
                        .collect()
                }
                // Nested Graph admission types are closed by the compiler from
                // the scoped crossing routes before transport. The owner tree
                // preserves the boundary even though the spatial DAG is flat.
                OwnerKind::Graph => {
                    let owner = node::OwnerSlot::new(u32::try_from(index).map_err(|_| {
                        PlanValidationError::OwnerOutOfBounds {
                            owner: node::OwnerSlot::new(u32::MAX),
                            owner_count: plan.owners.len(),
                        }
                    })?);
                    plan.edges
                        .iter()
                        .filter_map(|edge| {
                            let target = edge.to.as_ref()?;
                            let target_owner = plan.nodes.get(target.node?.index())?.owner;
                            (owner_contains(plan, owner, target_owner)
                                && edge.owner != owner
                                && owner_contains(plan, edge.owner, owner))
                            .then(|| ports.endpoint_type(target).cloned())
                            .flatten()
                        })
                        .collect()
                }
            };
            if inputs.is_empty() {
                return Err(PlanValidationError::UnresolvedInstanceAdmission { owner: owner_name });
            }
            for input in inputs {
                let signature = instance_key_signature(plan, index, input);
                let Some(candidate) = expr::rederive_type(&key.root, &signature) else {
                    return Err(PlanValidationError::InvalidInstanceKey {
                        owner: owner_name,
                        ty: DataType::Variant,
                    });
                };
                match &resolved {
                    Some(previous) if previous != &candidate => {
                        return Err(PlanValidationError::ConflictingInstanceKeyTypes {
                            owner: owner_name,
                            expected: previous.clone(),
                            found: candidate,
                        });
                    }
                    Some(_) => {}
                    None => resolved = Some(candidate),
                }
            }
            resolved.ok_or_else(|| PlanValidationError::UnresolvedInstanceAdmission {
                owner: owner_name.clone(),
            })?
        } else {
            let signature = instance_key_signature(plan, index, DataType::Variant);
            expr::rederive_type(&key.root, &signature).ok_or_else(|| {
                PlanValidationError::InvalidInstanceKey {
                    owner: owner_name.clone(),
                    ty: DataType::Variant,
                }
            })?
        };
        if !resolved.is_instance_key() {
            return Err(PlanValidationError::InvalidInstanceKey {
                owner: owner_name,
                ty: resolved,
            });
        }
        if key.ty != resolved {
            match mode {
                InstanceKeyTypeMode::Verify => {
                    return Err(PlanValidationError::InstanceKeyTypeMismatch {
                        owner: owner_name,
                        expected: resolved,
                        found: key.ty.clone(),
                    });
                }
                InstanceKeyTypeMode::Materialize if reads_data => {
                    let unresolved = expr::rederive_type(
                        &key.root,
                        &instance_key_signature(plan, index, DataType::Variant),
                    )
                    .unwrap_or(DataType::Variant);
                    if key.ty != unresolved {
                        return Err(PlanValidationError::InstanceKeyTypeMismatch {
                            owner: owner_name,
                            expected: resolved,
                            found: key.ty.clone(),
                        });
                    }
                    if let Some(key) = plan.owners[index].instance.key.as_mut() {
                        key.ty = resolved;
                    }
                }
                InstanceKeyTypeMode::Materialize => {
                    return Err(PlanValidationError::InstanceKeyTypeMismatch {
                        owner: owner_name,
                        expected: resolved,
                        found: key.ty.clone(),
                    });
                }
            }
        }
    }
    Ok(())
}

fn instance_key_signature(
    plan: &ExecutionPlan,
    owner_index: usize,
    input: DataType,
) -> expr::Signature {
    let mut signature = expr::Signature::new()
        .with("data", input)
        .with("meta.sys.id", DataType::Text)
        .with("meta.sys.parent_id", DataType::Variant)
        .with("meta.sys.origin_id", DataType::Text)
        .with("meta.sys.graph_version", DataType::INT);
    for field in crate::plan::STANDARD_EXT_FIELDS {
        if let Some(ty) = crate::plan::standard_ext_field_type(field) {
            signature = signature.with(alloc::format!("meta.ext.{field}"), ty);
        }
    }
    for (field, ty) in &plan.app_schema {
        signature = signature.with(format!("meta.app.{field}"), ty.clone());
    }
    for (field, ty) in &plan.ext_schema {
        signature = signature.with(format!("meta.ext.{field}"), ty.clone());
    }

    let mut lineage = Vec::new();
    let mut cursor = plan.owners.get(owner_index).and_then(|owner| owner.parent);
    while let Some(parent) = cursor {
        let Some(owner) = plan.owners.get(parent.index()) else {
            break;
        };
        lineage.push(owner);
        cursor = owner.parent;
    }
    for owner in lineage.into_iter().rev() {
        for value in &owner.vals {
            signature = signature.with(value.name.clone(), value.expr.ty.clone());
        }
    }
    signature
}

/// The node ordering the lowering emits: operators before
/// connectors, then by path — mirrored here so a reordered node
/// table refuses instead of producing a second byte form of the same
/// graph.
fn handler_decisions_hold(
    plan: &ExecutionPlan,
    ports: &super::ResolvedPortTable,
) -> Result<(), PlanValidationError> {
    for (index, handler) in plan.handlers.iter().enumerate() {
        let Some(signature) = handler_signature(plan, ports, handler) else {
            continue;
        };
        super::validate_dispatch(&handler.dispatch, &signature).map_err(|reason| {
            PlanValidationError::InvalidDispatch {
                handler: index,
                reason,
            }
        })?;
    }
    Ok(())
}

fn handler_signature(
    plan: &ExecutionPlan,
    ports: &super::ResolvedPortTable,
    handler: &super::Handler,
) -> Option<expr::Signature> {
    let slot = handler.owner;
    let node = &plan.nodes[slot.index()];
    let data = match &handler.activation {
        node::HandlerActivation::Port(port) => ports.node_port_type(slot, port).cloned(),
        node::HandlerActivation::Lifecycle(_) => ports.node(slot).and_then(|ports| {
            ports
                .iter()
                .find(|port| port.direction() == node::PortDirection::Input)
                .map(|port| port.ty().clone())
        }),
        _ => None,
    }
    .unwrap_or(DataType::Variant);
    let mut signature = instance_key_signature(plan, node.owner.index(), data)
        .with_metadata("meta")
        .with("now", DataType::TIMESTAMP);
    if let Some(spec) = plan.owners[node.owner.index()].loop_spec()
        && handler.activation == node::HandlerActivation::Port(spec.entry.clone())
        && let Some(state) = ports.node_port_type(slot, &spec.entry)
    {
        signature = signature
            .with("Loop.state", DataType::Variant)
            .with("Loop.state.data", state.clone())
            .with_metadata("Loop.state.meta");
    } else if !matches!(handler.activation, node::HandlerActivation::Lifecycle(_)) {
        let mut ancestor = plan.owners[node.owner.index()].parent;
        while let Some(owner) = ancestor {
            if let Some(spec) = plan.owners[owner.index()].loop_spec() {
                if let Some(state) = plan
                    .graph_ports(owner)
                    .and_then(|ports| ports.iter().find(|port| port.port == spec.entry))
                    .and_then(|port| port.ty.clone())
                {
                    signature = signature
                        .with("Loop.state", DataType::Variant)
                        .with("Loop.state.data", state)
                        .with_metadata("Loop.state.meta");
                }
                break;
            }
            ancestor = plan.owners[owner.index()].parent;
        }
    }
    for value in &plan.owners[node.owner.index()].vals {
        signature = signature.with(&value.name, value.expr.ty.clone());
    }
    let mut owner = Some(node.owner);
    while let Some(slot) = owner {
        if let Some(key) = &plan.owners[slot.index()].instance.key {
            signature = signature.with("instance.key", key.ty.clone());
            break;
        }
        owner = plan.owners[slot.index()].parent;
    }
    for var in &node.vars {
        signature = signature.with(&var.name, var.ty.clone());
    }
    if let node::HandlerActivation::Join { ports: inputs, .. } = &handler.activation {
        for input in inputs {
            if let Some(ty) = ports.node_port_type(slot, input) {
                signature = signature
                    .with(format!("{}.data", input.name()), ty.clone())
                    .with_metadata(format!("{}.meta", input.name()));
            }
        }
    }
    if let Some(guard) = &handler.guard {
        signature = signature.narrowed_by(&guard.root);
    }
    Some(signature)
}

fn execution_rank(execution: &NodeExecution) -> u8 {
    match execution {
        NodeExecution::Operator => 0,
        NodeExecution::Connector(_) => 1,
    }
}

/// A metadata write target must sit inside the travelling zone
/// schema — the Registry-less half of the Rastok expression grid.
fn meta_target_known(
    plan: &ExecutionPlan,
    target: &crate::envelope::WriteTarget,
) -> Result<(), PlanValidationError> {
    match target {
        crate::envelope::WriteTarget::Data(_) => Ok(()),
        crate::envelope::WriteTarget::App(field) => {
            if plan.app_schema.iter().any(|(known, _)| known == field) {
                Ok(())
            } else {
                Err(PlanValidationError::UnknownMetaField {
                    zone: "app",
                    field: field.clone(),
                })
            }
        }
        crate::envelope::WriteTarget::Ext(field) => {
            // Standard leaves are IMPLICIT — known to every consumer and
            // never serialized beside sheet-owned fields.
            if crate::plan::standard_ext_field_type(field).is_some()
                || plan.ext_schema.iter().any(|(known, _)| known == field)
            {
                Ok(())
            } else {
                Err(PlanValidationError::UnknownMetaField {
                    zone: "ext",
                    field: field.clone(),
                })
            }
        }
    }
}
/// One zone schema, held whole (D-2026-08-10-1): SORTED and unique
/// by field, flat keys prefix-free, every type inside the metadata
/// leaf rule (`scalar | array<scalar>` | `variant`), and the standard
/// standard leaves pinned to text.
fn schema_well_formed(
    zone: &'static str,
    schema: &[(alloc::string::String, DataType)],
) -> Result<(), PlanValidationError> {
    fn leaf_scalar(ty: &DataType) -> bool {
        // EXACTLY the checker's static list and `meta_conforms`'s
        // vocabulary — a schema no value can satisfy must not
        // validate (uint has no leaf representation).
        matches!(ty, DataType::Bool | DataType::Text | DataType::Variant)
            || ty == &DataType::INT
            || ty == &DataType::DOUBLE
            || ty == &DataType::BYTES
    }
    for (index, (field, ty)) in schema.iter().enumerate() {
        // The dotted-key FORM first: the nested projection splits on
        // dots, so an empty key or segment is unreachable state.
        if field.is_empty() || field.split('.').any(str::is_empty) {
            return Err(PlanValidationError::MetaFieldMalformed {
                zone,
                field: field.clone(),
            });
        }
        let leaf_ok = leaf_scalar(ty)
            || matches!(ty, DataType::Array(array) if array.length().is_none() && leaf_scalar(array.element()));
        if !leaf_ok {
            return Err(PlanValidationError::MetaSchemaType {
                zone,
                field: field.clone(),
            });
        }
        // In EXT standard fields are implicit and reserved — a serialized
        // entry would create a second authority, and a BRANCH under one would
        // shadow its implicit leaf. In APP the spellings remain ordinary.
        if zone == "ext" && crate::plan::standard_ext_field_type(field).is_some() {
            return Err(PlanValidationError::UnknownMetaField {
                zone,
                field: field.clone(),
            });
        }
        if zone == "ext" {
            for standard in crate::plan::STANDARD_EXT_FIELDS {
                if field.starts_with(standard)
                    && field.as_bytes().get(standard.len()) == Some(&b'.')
                {
                    return Err(PlanValidationError::MetaPrefixCollision {
                        leaf: alloc::string::String::from(*standard),
                        branch: field.clone(),
                    });
                }
            }
        }
        if let Some((previous, _)) = index.checked_sub(1).and_then(|i| schema.get(i))
            && previous >= field
        {
            // Sorted strictly — uniqueness for free.
            return Err(PlanValidationError::UnknownMetaField {
                zone: "schema-order",
                field: field.clone(),
            });
        }
    }
    let keys: alloc::vec::Vec<&str> = schema.iter().map(|(field, _)| field.as_str()).collect();
    for pair in keys.windows(2) {
        if pair[1].starts_with(pair[0]) && pair[1].as_bytes().get(pair[0].len()) == Some(&b'.') {
            return Err(PlanValidationError::MetaPrefixCollision {
                leaf: alloc::string::String::from(pair[0]),
                branch: alloc::string::String::from(pair[1]),
            });
        }
    }
    Ok(())
}

/// Every `Expr::Asset` index must sit inside the asset table — the
/// VM dereferences without checking.
fn asset_indexes_bound(plan: &ExecutionPlan, expr: &expr::Expr) -> Result<(), PlanValidationError> {
    match expr {
        expr::Expr::Asset(index) => {
            if (*index as usize) < plan.assets.len() {
                Ok(())
            } else {
                Err(PlanValidationError::AssetOutOfBounds {
                    index: *index,
                    count: plan.assets.len(),
                })
            }
        }
        expr::Expr::Literal(_) | expr::Expr::Path(_) => Ok(()),
        expr::Expr::Unary { expr, .. }
        | expr::Expr::Is { expr, .. }
        | expr::Expr::As { expr, .. } => asset_indexes_bound(plan, expr),
        expr::Expr::Binary { left, right, .. } => {
            asset_indexes_bound(plan, left)?;
            asset_indexes_bound(plan, right)
        }
        expr::Expr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            asset_indexes_bound(plan, cond)?;
            asset_indexes_bound(plan, then)?;
            asset_indexes_bound(plan, otherwise)
        }
        expr::Expr::Index { base, index } => {
            asset_indexes_bound(plan, base)?;
            asset_indexes_bound(plan, index)
        }
        expr::Expr::Call { args, .. } => args
            .iter()
            .try_for_each(|arg| asset_indexes_bound(plan, arg)),
        expr::Expr::List(items) => items
            .iter()
            .try_for_each(|item| asset_indexes_bound(plan, item)),
        expr::Expr::MapLit(pairs) => pairs
            .iter()
            .try_for_each(|(_, value)| asset_indexes_bound(plan, value)),
        expr::Expr::Struct(fields) | expr::Expr::Tagged { fields, .. } => fields
            .iter()
            .try_for_each(|(_, value)| asset_indexes_bound(plan, value)),
    }
}

/// The asset table's own invariants (D-2026-08-10-5): digests
/// unique (one content, one blob) and the fixed v1 budgets held —
/// 8 MiB per asset, 32 MiB total. Digests were DERIVED at decode,
/// so verifying them here is verifying the bytes.
fn assets_well_formed(plan: &ExecutionPlan) -> Result<(), PlanValidationError> {
    const PER_ASSET: u64 = 8 * 1024 * 1024;
    const TOTAL: u64 = 32 * 1024 * 1024;
    let mut total: u64 = 0;
    for (index, blob) in plan.assets.iter().enumerate() {
        // The digest IS the identity: a blob whose declared digest
        // is not blake3(bytes) would encode (bytes only), then
        // decode to a DIFFERENT identity — refused here, so
        // encode ⇒ decode holds.
        if blob.digest != *blake3::hash(&blob.bytes).as_bytes() {
            return Err(PlanValidationError::AssetDigestMismatch {
                index: index as u32,
            });
        }
        if let Some(previous) = index.checked_sub(1).and_then(|i| plan.assets.get(i))
            && previous.digest >= blob.digest
        {
            // Strictly increasing — canonical order AND uniqueness.
            return Err(PlanValidationError::AssetOrder {
                index: index as u32,
            });
        }
        if blob.bytes.len() as u64 > PER_ASSET {
            return Err(PlanValidationError::AssetBudget {
                index: index as u32,
                bytes: blob.bytes.len() as u64,
            });
        }
        total += blob.bytes.len() as u64;
        if plan.assets[..index]
            .iter()
            .any(|earlier| earlier.digest == blob.digest)
        {
            return Err(PlanValidationError::DuplicateAsset {
                index: index as u32,
            });
        }
    }
    if total > TOTAL {
        return Err(PlanValidationError::AssetBudget {
            index: u32::MAX,
            bytes: total,
        });
    }
    Ok(())
}

fn data_type_at(ty: &DataType, location: String) -> Result<(), PlanValidationError> {
    ty.check_bounds()
        .map_err(|reason| PlanValidationError::InvalidDataType { location, reason })
}

fn annotated_schema_at(
    ty: &DataType,
    annotations: &AnnotationTable,
    location: String,
) -> Result<(), PlanValidationError> {
    if annotations.is_empty() {
        return Ok(());
    }
    annotations
        .validate_for(ty)
        .map_err(|reason| PlanValidationError::InvalidSchemaAnnotations { location, reason })
}

fn annotations_well_formed(
    plan: &ExecutionPlan,
    resolved: &super::type_resolution::ResolvedPortTable,
) -> Result<(), PlanValidationError> {
    for port in resolved.boundary() {
        annotated_schema_at(
            port.ty(),
            port.annotations(),
            format!("boundary port `{}`", port.port()),
        )?;
    }
    for (index, node) in plan.nodes.iter().enumerate() {
        let slot = NodeIndex::new(u32::try_from(index).unwrap_or(u32::MAX));
        if let Some(ports) = resolved.node(slot) {
            for port in ports {
                annotated_schema_at(
                    port.ty(),
                    port.annotations(),
                    format!("port `{}.{}`", node.path, port.port()),
                )?;
            }
        }
        for var in &node.vars {
            annotated_schema_at(
                &var.ty,
                &var.annotations,
                format!("var `{}.{}`", node.path, var.name),
            )?;
        }
    }
    for owner in &plan.owners {
        for port in &owner.ports {
            let location = format!("Graph port `{}.{}`", owner.path, port.port);
            match &port.ty {
                Some(ty) => annotated_schema_at(ty, &port.annotations, location)?,
                None if !port.annotations.is_empty() => {
                    return Err(PlanValidationError::InvalidSchemaAnnotations {
                        location,
                        reason: SchemaAnnotationError::MissingLogicalType,
                    });
                }
                None => {}
            }
        }
        for value in &owner.vals {
            annotated_schema_at(
                &value.expr.ty,
                &value.annotations,
                format!("value `{}.{}`", owner.path, value.name),
            )?;
        }
    }
    for (handler_index, handler) in plan.handlers.iter().enumerate() {
        for step in handler.dispatch.steps() {
            if let DispatchStep::Let {
                name,
                value: expr,
                annotations,
            } = step
            {
                annotated_schema_at(
                    &expr.ty,
                    annotations,
                    format!("handler {handler_index} let `{name}`"),
                )?;
            }
        }
    }
    Ok(())
}

fn tree_type_at(tree: &expr::Tree, location: String) -> Result<(), PlanValidationError> {
    data_type_at(&tree.ty, location)?;
    let holds = match &tree.root {
        expr::Expr::Literal(value) => tree.ty.accepts(value),
        expr::Expr::Struct(fields) => match &tree.ty {
            DataType::Struct(structure) => {
                structure.fields().len() == fields.len()
                    && structure
                        .fields()
                        .iter()
                        .zip(fields)
                        .all(|(expected, (found, _))| expected.name() == found)
            }
            _ => false,
        },
        expr::Expr::Tagged { union, .. } => tree.ty == DataType::TaggedUnion((**union).clone()),
        expr::Expr::Is { .. } => tree.ty == DataType::Bool,
        expr::Expr::As { target, .. } => match target.as_ref() {
            expr::NarrowTarget::Type(target) => &tree.ty == target.ty(),
            expr::NarrowTarget::Case { payload, .. } => &tree.ty == payload.as_ref(),
        },
        expr::Expr::Call {
            function: expr::Function::Decimal,
            ..
        } => tree.ty == DataType::Number,
        _ => true,
    };
    if holds {
        Ok(())
    } else {
        Err(PlanValidationError::UnrepresentableConstant)
    }
}

fn setting_has_type(setting: &crate::plan::AttrSetting, ty: &DataType) -> bool {
    match setting {
        crate::plan::AttrSetting::Value(value) => ty.accepts(value),
        crate::plan::AttrSetting::Expr { expr, .. } => &expr.ty == ty,
        crate::plan::AttrSetting::Word(_) => false,
    }
}

fn network_setting_holds(name: &str, setting: &crate::plan::AttrSetting) -> bool {
    match name {
        "json_profile" => {
            matches!(setting, crate::plan::AttrSetting::Word(word) if word == "projected")
        }
        "idempotent" => matches!(
            setting,
            crate::plan::AttrSetting::Value(crate::value::Value::Bool(_))
        ),
        "codec" => matches!(
            setting,
            crate::plan::AttrSetting::Word(word)
                if matches!(word.as_str(), "bytes" | "json" | "text")
        ),
        "mode" => matches!(
            setting,
            crate::plan::AttrSetting::Word(word) if matches!(word.as_str(), "ack" | "respond")
        ),
        "zero_rtt" => matches!(
            setting,
            crate::plan::AttrSetting::Word(word) if word == "disabled"
        ),
        "versions" => match setting {
            crate::plan::AttrSetting::Value(crate::value::Value::Array(items))
                if !items.is_empty() =>
            {
                let mut seen = BTreeSet::new();
                items.iter().all(|item| match item {
                    crate::value::Value::Text(version) => {
                        matches!(version.as_str(), "http/1.1" | "http/2" | "http/3")
                            && seen.insert(version.as_str())
                    }
                    _ => false,
                })
            }
            _ => false,
        },
        "max_body" | "max_message" => matches!(
            setting,
            crate::plan::AttrSetting::Value(crate::value::Value::Text(value))
                if crate::connector::byte_size(value).is_some()
        ),
        "timeout" | "handshake_timeout" => match setting {
            crate::plan::AttrSetting::Value(crate::value::Value::Duration(value)) => *value > 0,
            crate::plan::AttrSetting::Expr { expr, .. } => expr.ty == DataType::DURATION,
            _ => false,
        },
        "auth_bearer" | "correlate" | "idempotency_key" | "listen" | "method" | "path" | "url" => {
            setting_has_type(setting, &DataType::Text)
        }
        "tls_ca" | "tls_certificate" | "tls_private_key" => {
            setting_has_type(setting, &DataType::BYTES)
        }
        "allowed_origins" | "subprotocols" => {
            setting_has_type(setting, &DataType::array(DataType::Text))
        }
        "headers" => match setting {
            crate::plan::AttrSetting::Value(value) => {
                crate::plan::typed_http_headers_value_holds(value)
            }
            crate::plan::AttrSetting::Expr { expr, .. } => {
                crate::plan::typed_http_headers_type_holds(&expr.ty)
            }
            crate::plan::AttrSetting::Word(_) => false,
        },
        "custom_headers" => match setting {
            crate::plan::AttrSetting::Value(value) => {
                crate::plan::custom_http_headers_value_holds(value)
            }
            crate::plan::AttrSetting::Expr { expr, .. } => {
                expr.ty == crate::plan::custom_http_headers_type()
            }
            crate::plan::AttrSetting::Word(_) => false,
        },
        _ => false,
    }
}

fn effective_contract_port(
    node: &Node,
    loop_spec: Option<&crate::plan::LoopSpec>,
    name: &str,
    base: Option<DataType>,
) -> Option<DataType> {
    let Some(spec) = loop_spec.filter(|spec| {
        spec.expired
            .as_ref()
            .is_some_and(|expiry| expiry.port.name() == name)
    }) else {
        return base;
    };
    let DataType::TaggedUnion(union) = base? else {
        return None;
    };
    let state = node
        .ports
        .iter()
        .find(|port| port.port == spec.entry)?
        .ty
        .clone()?;
    node::loop_call_exit_type(&union, state).ok()
}

fn validate_network_connector(
    node: &Node,
    spec: &crate::plan::ConnectorSpec,
    loop_spec: Option<&crate::plan::LoopSpec>,
) -> Result<bool, PlanValidationError> {
    use crate::connector::{ConnectorDirection, ConnectorExchange};

    struct ContractShape {
        roles: ConnectorDirection,
        exchange: ConnectorExchange,
        required: &'static [&'static str],
        allowed: &'static [&'static str],
        ports: &'static [(&'static str, node::PortDirection)],
    }

    let contract = spec.contract.as_str();
    let shape = match contract {
        crate::connector::HTTP_CLIENT_CONNECTOR_ID => ContractShape {
            roles: ConnectorDirection::Outbound,
            exchange: ConnectorExchange::Call,
            required: &[
                "codec", "max_body", "method", "timeout", "url", "versions", "zero_rtt",
            ],
            allowed: &[
                "auth_bearer",
                "codec",
                "custom_headers",
                "headers",
                "idempotent",
                "idempotency_key",
                "json_profile",
                "max_body",
                "method",
                "timeout",
                "tls_ca",
                "tls_certificate",
                "tls_private_key",
                "url",
                "versions",
                "zero_rtt",
            ],
            ports: &[
                ("in", node::PortDirection::Input),
                ("out", node::PortDirection::Output),
                ("exit", node::PortDirection::Output),
            ],
        },
        crate::connector::HTTP_SERVER_CONNECTOR_ID => ContractShape {
            roles: ConnectorDirection::Inbound,
            exchange: ConnectorExchange::Call,
            required: &[
                "codec", "listen", "max_body", "mode", "path", "versions", "zero_rtt",
            ],
            allowed: &[
                "auth_bearer",
                "codec",
                "correlate",
                "custom_headers",
                "headers",
                "listen",
                "max_body",
                "mode",
                "path",
                "timeout",
                "tls_certificate",
                "tls_private_key",
                "versions",
                "zero_rtt",
            ],
            ports: &[
                ("in", node::PortDirection::Input),
                ("out", node::PortDirection::Output),
                ("exit", node::PortDirection::Output),
            ],
        },
        crate::connector::WEBSOCKET_CLIENT_CONNECTOR_ID => ContractShape {
            roles: ConnectorDirection::InboundAndOutbound,
            exchange: ConnectorExchange::Session,
            required: &[
                "codec",
                "handshake_timeout",
                "max_message",
                "url",
                "versions",
                "zero_rtt",
            ],
            allowed: &[
                "auth_bearer",
                "codec",
                "custom_headers",
                "handshake_timeout",
                "headers",
                "max_message",
                "subprotocols",
                "tls_ca",
                "tls_certificate",
                "tls_private_key",
                "url",
                "versions",
                "zero_rtt",
            ],
            ports: &[
                ("in", node::PortDirection::Input),
                ("out", node::PortDirection::Output),
                ("exit", node::PortDirection::Output),
            ],
        },
        crate::connector::WEBSOCKET_SERVER_CONNECTOR_ID => ContractShape {
            roles: ConnectorDirection::InboundAndOutbound,
            exchange: ConnectorExchange::Session,
            required: &[
                "codec",
                "handshake_timeout",
                "listen",
                "max_message",
                "path",
                "versions",
                "zero_rtt",
            ],
            allowed: &[
                "allowed_origins",
                "auth_bearer",
                "codec",
                "correlate",
                "custom_headers",
                "handshake_timeout",
                "headers",
                "listen",
                "max_message",
                "path",
                "subprotocols",
                "tls_certificate",
                "tls_private_key",
                "versions",
                "zero_rtt",
            ],
            ports: &[
                ("in", node::PortDirection::Input),
                ("out", node::PortDirection::Output),
                ("exit", node::PortDirection::Output),
            ],
        },
        _ => return Ok(false),
    };

    if spec.roles != shape.roles {
        return Err(PlanValidationError::ContractRoleMismatch {
            node: node.path.clone(),
        });
    }
    if spec.exchange != shape.exchange {
        return Err(PlanValidationError::ContractExchangeMismatch {
            node: node.path.clone(),
        });
    }
    let axes_hold = match contract {
        crate::connector::HTTP_SERVER_CONNECTOR_ID
        | crate::connector::WEBSOCKET_SERVER_CONNECTOR_ID => {
            spec.behavior == node::Behavior::Deterministic
        }
        _ => true,
    };
    if !axes_hold {
        return Err(PlanValidationError::ContractAttributeMismatch {
            node: node.path.clone(),
            attribute: String::from("behavior"),
        });
    }

    for required in shape.required {
        if spec.attribute(required).is_none() {
            return Err(PlanValidationError::MissingContractAttribute {
                node: node.path.clone(),
                attribute: required,
            });
        }
    }
    for (name, setting) in &spec.attributes {
        if !shape.allowed.contains(&name.as_str()) || !network_setting_holds(name, setting) {
            return Err(PlanValidationError::ContractAttributeMismatch {
                node: node.path.clone(),
                attribute: name.clone(),
            });
        }
    }
    if contract == crate::connector::HTTP_SERVER_CONNECTOR_ID {
        match spec.attribute("mode") {
            Some(crate::plan::AttrSetting::Word(mode)) if mode == "ack" => {
                if spec.attribute("correlate").is_some() || spec.attribute("timeout").is_some() {
                    return Err(PlanValidationError::ContractAttributeMismatch {
                        node: node.path.clone(),
                        attribute: String::from("mode/correlate/timeout"),
                    });
                }
            }
            Some(crate::plan::AttrSetting::Word(mode)) if mode == "respond" => {
                if spec.attribute("timeout").is_none() {
                    return Err(PlanValidationError::MissingContractAttribute {
                        node: node.path.clone(),
                        attribute: "timeout",
                    });
                }
                if let Some(crate::plan::AttrSetting::Expr { site, expr }) =
                    spec.attribute("correlate")
                    && (*site != EvaluationSite::Invocation || expr.ty != DataType::Text)
                {
                    return Err(PlanValidationError::ContractAttributeMismatch {
                        node: node.path.clone(),
                        attribute: String::from("correlate"),
                    });
                }
            }
            Some(_) => {
                return Err(PlanValidationError::ContractAttributeMismatch {
                    node: node.path.clone(),
                    attribute: String::from("mode"),
                });
            }
            None => {
                return Err(PlanValidationError::MissingContractAttribute {
                    node: node.path.clone(),
                    attribute: "mode",
                });
            }
        }
    }
    if spec.attribute("tls_certificate").is_some() != spec.attribute("tls_private_key").is_some() {
        return Err(PlanValidationError::ContractAttributeMismatch {
            node: node.path.clone(),
            attribute: String::from("tls_certificate/tls_private_key"),
        });
    }

    let codec = match spec.attribute("codec") {
        Some(crate::plan::AttrSetting::Word(codec)) => Some(codec.as_str()),
        _ => None,
    };
    for (name, direction) in shape.ports {
        let Some(port) = node.ports.iter().find(|port| port.port.name() == *name) else {
            return Err(PlanValidationError::MissingContractPort {
                node: node.path.clone(),
                port: name,
            });
        };
        let json_output = contract == crate::connector::HTTP_CLIENT_CONNECTOR_ID
            && *name == "out"
            && codec == Some("json");
        if *name == "out"
            && spec.attribute("json_profile").is_some()
            && (!json_output || !matches!(port.ty, Some(DataType::Struct(_))))
        {
            return Err(PlanValidationError::ContractAttributeMismatch {
                node: node.path.clone(),
                attribute: String::from("json_profile"),
            });
        }
        let type_holds = if json_output {
            port.ty.as_ref().is_some_and(crate::json::supports_type)
        } else {
            port.ty
                == effective_contract_port(
                    node,
                    loop_spec,
                    name,
                    crate::boundary::network_port_type(&spec.contract, name, codec),
                )
        };
        if port.direction != *direction || !type_holds {
            return Err(PlanValidationError::ContractPortMismatch {
                node: node.path.clone(),
                port: port.port.clone(),
            });
        }
    }
    if let Some(port) = node.ports.iter().find(|port| {
        !shape
            .ports
            .iter()
            .any(|(name, _)| *name == port.port.name())
    }) {
        return Err(PlanValidationError::UnexpectedContractPort {
            node: node.path.clone(),
            port: port.port.clone(),
        });
    }
    Ok(true)
}

/// Every type-bearing plan position passes this audit before the infallible
/// encoder can observe the plan.
fn data_types_well_formed(plan: &ExecutionPlan) -> Result<(), PlanValidationError> {
    for (field, ty) in &plan.app_schema {
        data_type_at(ty, format!("meta.app.{field}"))?;
    }
    for (field, ty) in &plan.ext_schema {
        data_type_at(ty, format!("meta.ext.{field}"))?;
    }
    for decl in &plan.boundary {
        if let Some(ty) = &decl.ty {
            data_type_at(ty, format!("boundary port `{}`", decl.port))?;
        }
    }
    for owner in &plan.owners {
        for decl in &owner.ports {
            if let Some(ty) = &decl.ty {
                data_type_at(ty, format!("Graph port `{}.{}`", owner.path, decl.port))?;
            }
        }
        if let Some(key) = &owner.instance.key {
            tree_type_at(key, format!("instance key on `{}`", owner.path))?;
        }
        for value in &owner.vals {
            tree_type_at(
                &value.expr,
                format!("immutable value `{}.{}`", owner.path, value.name),
            )?;
        }
        for aspect in &owner.extensions {
            let crate::plan::ExtensionSpec::Loop(spec) = aspect else {
                continue;
            };
            if let Some(expired) = &spec.expired {
                data_type_at(&expired.ty, format!("Loop exhaustion on `{}`", owner.path))?;
            }
            if let Some(fuel) = &spec.fuel {
                tree_type_at(&fuel.expr, format!("Loop fuel on `{}`", owner.path))?;
            }
            match &spec.delay {
                None => {}
                Some(crate::plan::DelayPolicy::Fixed { after }) => {
                    tree_type_at(&after.expr, format!("Loop delay on `{}`", owner.path))?;
                }
                Some(crate::plan::DelayPolicy::Exponential { first, ceiling }) => {
                    tree_type_at(&first.expr, format!("Loop delay on `{}`", owner.path))?;
                    tree_type_at(
                        &ceiling.expr,
                        format!("Loop delay ceiling on `{}`", owner.path),
                    )?;
                }
            }
            if let Some(proof) = &spec.proof {
                tree_type_at(&proof.measure, format!("Loop measure on `{}`", owner.path))?;
                if let Some(invariant) = &proof.invariant {
                    tree_type_at(invariant, format!("Loop invariant on `{}`", owner.path))?;
                }
            }
        }
    }
    for node in &plan.nodes {
        if let NodeExecution::Connector(spec) = &node.execution {
            for (name, setting) in &spec.attributes {
                if let crate::plan::AttrSetting::Expr { expr, .. } = setting {
                    tree_type_at(expr, format!("attribute `{}.{name}`", node.path))?;
                }
            }
        }
        for decl in &node.ports {
            if let Some(ty) = &decl.ty {
                data_type_at(ty, format!("port `{}.{}`", node.path, decl.port))?;
            }
        }
        for var in &node.vars {
            data_type_at(&var.ty, format!("state `{}.{}`", node.path, var.name))?;
        }
    }
    for (handler_index, handler) in plan.handlers.iter().enumerate() {
        if let Some(guard) = &handler.guard {
            tree_type_at(guard, format!("handler {handler_index} guard"))?;
        }
        for (index, predicate) in handler.dispatch.predicates().enumerate() {
            tree_type_at(
                predicate,
                format!("handler {handler_index} decision {index}"),
            )?;
        }
        for (step_index, step) in handler.dispatch.steps().enumerate() {
            let location = |role: &str| format!("handler {handler_index} step {step_index} {role}");
            match step {
                DispatchStep::MultiEmit { .. } | DispatchStep::LoopCommand(_) => {
                    for (index, expression) in step.expressions().enumerate() {
                        tree_type_at(expression, location(&format!("expression {index}")))?;
                    }
                }
                DispatchStep::Let { value: expr, .. } => {
                    tree_type_at(expr, location("let expression"))?
                }
                DispatchStep::SetVar { value: expr, .. } => {
                    tree_type_at(expr, location("state expression"))?
                }
                DispatchStep::Stop => {}
                DispatchStep::Emit { value, .. } => match value {
                    EmitValue::Identity | EmitValue::Nothing => {}
                    EmitValue::Data(tree) => tree_type_at(tree, location("data emission"))?,
                    EmitValue::Patch(fields) | EmitValue::FreshData(fields) => {
                        for (field_index, field) in fields.iter().enumerate() {
                            tree_type_at(
                                &field.value,
                                format!(
                                    "handler {handler_index} step {step_index} field {field_index}"
                                ),
                            )?;
                        }
                    }
                },
            }
        }
    }
    Ok(())
}

/// Default inbound-Call lineage is sound only while every upstream handler
/// observes one root. A join deliberately mixes roots and inherits the
/// trigger's origin, so walking the finite DAG backwards is the exact static
/// proof that an explicit correlator is required.
fn response_path_crosses_join(plan: &ExecutionPlan, target: NodeIndex) -> bool {
    let mut pending = vec![target];
    let mut visited = BTreeSet::new();
    while let Some(slot) = pending.pop() {
        if !visited.insert(slot) {
            continue;
        }
        if plan.handlers.iter().any(|handler| {
            handler.owner == slot
                && matches!(handler.activation, node::HandlerActivation::Join { .. })
        }) {
            return true;
        }
        for source in plan.edges.iter().filter_map(|edge| {
            edge.to
                .as_ref()
                .filter(|endpoint| endpoint.node == Some(slot))
                .and(edge.from.node)
        }) {
            pending.push(source);
        }
    }
    false
}

#[derive(Debug, Clone)]
struct ImmutableBinding {
    site: EvaluationSite,
    resolved: Option<crate::value::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindingVisit {
    New,
    Visiting,
    Done,
}

struct PlanValueBindings<'a> {
    values: &'a BTreeMap<String, ImmutableBinding>,
    assets: &'a [crate::plan::AssetBlob],
}

impl expr::Bindings for PlanValueBindings<'_> {
    fn get(&self, name: &str) -> Option<expr::EvalValue> {
        self.values
            .get(name)
            .and_then(|binding| binding.resolved.clone())
            .map(expr::EvalValue::from)
    }

    fn asset(&self, index: u32) -> Result<expr::EvalValue, expr::Error> {
        self.assets
            .get(index as usize)
            .map(|blob| expr::EvalValue::Bytes(blob.bytes.clone()))
            .ok_or_else(|| expr::Error {
                kind: expr::ErrorKind::Evaluation,
                message: format!("asset {index} is outside the validated table"),
            })
    }
}

fn expression_site(
    expression: &expr::Expr,
    owner: &crate::plan::Owner,
    inherited: &BTreeMap<String, ImmutableBinding>,
    local: &BTreeMap<&str, usize>,
    visits: &mut [BindingVisit],
    sites: &mut [Option<EvaluationSite>],
) -> Result<EvaluationSite, PlanValidationError> {
    let owner_name = owner.path.to_string();
    let combine = |left: EvaluationSite, right: EvaluationSite| left.max(right);
    let child_site = |child, visits: &mut [BindingVisit], sites: &mut [Option<EvaluationSite>]| {
        expression_site(child, owner, inherited, local, visits, sites)
    };
    match expression {
        expr::Expr::Literal(_) | expr::Expr::Asset(_) => Ok(EvaluationSite::Plan),
        expr::Expr::Path(segments) => {
            match expr::boot_reference(expression) {
                Ok(Some(_)) => return Ok(EvaluationSite::Boot),
                Err(_) => {
                    return Err(PlanValidationError::InvalidBinding {
                        owner: owner_name,
                        name: String::new(),
                        path: segments.join("."),
                    });
                }
                Ok(None) => {}
            }
            let Some(root) = segments.first() else {
                return Err(PlanValidationError::UnrepresentableConstant);
            };
            if let Some(index) = local.get(root.as_str()) {
                return owner_value_site(*index, owner, inherited, local, visits, sites);
            }
            if let Some(binding) = inherited.get(root) {
                return Ok(binding.site);
            }
            if matches!(segments.as_slice(), [instance, key, ..]
                if instance == "instance" && key == "key")
                && owner.instance.key.is_some()
            {
                return Ok(EvaluationSite::Instance);
            }
            if matches!(root.as_str(), "data" | "meta") {
                return Ok(EvaluationSite::Invocation);
            }
            Err(PlanValidationError::InvalidBinding {
                owner: owner_name,
                name: String::new(),
                path: segments.join("."),
            })
        }
        expr::Expr::Unary { expr, .. }
        | expr::Expr::Is { expr, .. }
        | expr::Expr::As { expr, .. } => child_site(expr, visits, sites),
        expr::Expr::Binary { left, right, .. }
        | expr::Expr::Index {
            base: left,
            index: right,
        } => {
            let left = child_site(left, visits, sites)?;
            let right = child_site(right, visits, sites)?;
            Ok(combine(left, right))
        }
        expr::Expr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            let cond = child_site(cond, visits, sites)?;
            let then = child_site(then, visits, sites)?;
            let otherwise = child_site(otherwise, visits, sites)?;
            Ok(combine(combine(cond, then), otherwise))
        }
        expr::Expr::Call { args, .. } | expr::Expr::List(args) => {
            let mut site = EvaluationSite::Plan;
            for child in args {
                site = combine(site, child_site(child, visits, sites)?);
            }
            Ok(site)
        }
        expr::Expr::MapLit(fields)
        | expr::Expr::Struct(fields)
        | expr::Expr::Tagged { fields, .. } => {
            let mut site = EvaluationSite::Plan;
            for (_, child) in fields {
                site = combine(site, child_site(child, visits, sites)?);
            }
            Ok(site)
        }
    }
}

/// A transported plan constant must be canonical, closed over the plan's
/// assets, and equal to its independent reduction. No invocation environment
/// may turn a repetition guarantee or delay into a runtime policy.
pub(super) fn plan_constant_holds(
    plan: &ExecutionPlan,
    constant: &crate::plan::PlanConstant,
    expected: &DataType,
) -> Result<bool, PlanValidationError> {
    expr_canonical(&constant.expr.root)?;
    asset_indexes_bound(plan, &constant.expr.root)?;
    value_expressible(&constant.resolved)?;
    let Some(derived) = expr::rederive_type(&constant.expr.root, &expr::Signature::new()) else {
        return Ok(false);
    };
    if derived != constant.expr.ty || !derived.is_assignable_to(expected) {
        return Ok(false);
    }
    let bindings = PlanValueBindings {
        values: &BTreeMap::new(),
        assets: &plan.assets,
    };
    Ok(expr::eval(&constant.expr, &bindings)
        .and_then(expr::EvalValue::into_value)
        .is_ok_and(|actual| actual == constant.resolved && expected.accepts(&actual)))
}

fn owner_value_site(
    index: usize,
    owner: &crate::plan::Owner,
    inherited: &BTreeMap<String, ImmutableBinding>,
    local: &BTreeMap<&str, usize>,
    visits: &mut [BindingVisit],
    sites: &mut [Option<EvaluationSite>],
) -> Result<EvaluationSite, PlanValidationError> {
    if let Some(site) = sites[index] {
        return Ok(site);
    }
    match visits[index] {
        BindingVisit::Visiting => {
            return Err(PlanValidationError::CyclicOwnerValue {
                owner: owner.path.to_string(),
                name: owner.vals[index].name.clone(),
            });
        }
        BindingVisit::Done => {
            return sites[index].ok_or_else(|| PlanValidationError::CyclicOwnerValue {
                owner: owner.path.to_string(),
                name: owner.vals[index].name.clone(),
            });
        }
        BindingVisit::New => {}
    }
    visits[index] = BindingVisit::Visiting;
    let site = expression_site(
        &owner.vals[index].expr.root,
        owner,
        inherited,
        local,
        visits,
        sites,
    )
    .map_err(|error| match error {
        PlanValidationError::InvalidBinding {
            owner: owner_path,
            name: _,
            path,
        } => PlanValidationError::InvalidBinding {
            owner: owner_path,
            name: owner.vals[index].name.clone(),
            path,
        },
        other => other,
    })?;
    visits[index] = BindingVisit::Done;
    sites[index] = Some(site);
    Ok(site)
}

/// Proves that the phase and reduction transported beside every immutable
/// owner expression are derived facts, not claims. This walk also rejects
/// lexical shadowing and cycles, so evaluation never depends on declaration
/// order that canonical transport deliberately erases.
fn owner_values_well_formed(plan: &ExecutionPlan) -> Result<(), PlanValidationError> {
    let mut environments: Vec<BTreeMap<String, ImmutableBinding>> = Vec::new();
    for owner in &plan.owners {
        let inherited = owner
            .parent
            .and_then(|parent| environments.get(parent.index()))
            .cloned()
            .unwrap_or_default();
        for value in &owner.vals {
            if inherited.contains_key(&value.name) {
                return Err(PlanValidationError::DuplicateOwnerValue {
                    owner: owner.path.to_string(),
                    name: value.name.clone(),
                });
            }
        }

        let mut local = BTreeMap::new();
        for (index, value) in owner.vals.iter().enumerate() {
            local.insert(value.name.as_str(), index);
        }
        let mut visits = alloc::vec![BindingVisit::New; owner.vals.len()];
        let mut sites = alloc::vec![None; owner.vals.len()];
        for index in 0..owner.vals.len() {
            let actual =
                owner_value_site(index, owner, &inherited, &local, &mut visits, &mut sites)?;
            let value = &owner.vals[index];
            if value.site != actual
                || (actual == EvaluationSite::Plan) != value.resolved.is_some()
                || actual == EvaluationSite::Invocation
            {
                return Err(PlanValidationError::BindingSiteMismatch {
                    owner: owner.path.to_string(),
                    name: value.name.clone(),
                });
            }
        }

        let mut environment = inherited;
        for value in &owner.vals {
            environment.insert(
                value.name.clone(),
                ImmutableBinding {
                    site: value.site,
                    resolved: value.resolved.clone(),
                },
            );
        }
        let bindings = PlanValueBindings {
            values: &environment,
            assets: &plan.assets,
        };
        for value in &owner.vals {
            let Some(claimed) = &value.resolved else {
                continue;
            };
            let actual = expr::eval(&value.expr, &bindings)
                .and_then(expr::EvalValue::into_value)
                .map_err(|_| PlanValidationError::OwnerValueResolutionMismatch {
                    owner: owner.path.to_string(),
                    name: value.name.clone(),
                })?;
            if &actual != claimed {
                return Err(PlanValidationError::OwnerValueResolutionMismatch {
                    owner: owner.path.to_string(),
                    name: value.name.clone(),
                });
            }
        }
        if let OwnerKind::Node(slot) = owner.kind
            && let Some(NodeExecution::Connector(spec)) =
                plan.nodes.get(slot.index()).map(|node| &node.execution)
        {
            for (name, setting) in &spec.attributes {
                let crate::plan::AttrSetting::Expr { site, expr } = setting else {
                    continue;
                };
                let actual = expression_site(
                    &expr.root,
                    owner,
                    &environment,
                    &local,
                    &mut visits,
                    &mut sites,
                )
                .map_err(|error| match error {
                    PlanValidationError::InvalidBinding {
                        owner,
                        name: _,
                        path,
                    } => PlanValidationError::InvalidBinding {
                        owner,
                        name: name.clone(),
                        path,
                    },
                    other => other,
                })?;
                if actual != *site || actual == EvaluationSite::Plan {
                    return Err(PlanValidationError::BindingSiteMismatch {
                        owner: owner.path.to_string(),
                        name: name.clone(),
                    });
                }
            }
        }
        environments.push(environment);
    }
    Ok(())
}

fn loop_specs_well_formed(plan: &ExecutionPlan) -> Result<(), PlanValidationError> {
    for (owner_index, owner) in plan.owners.iter().enumerate() {
        let owner_slot = node::OwnerSlot::new(u32::try_from(owner_index).unwrap_or(u32::MAX));
        let Some(spec) = owner.loop_spec() else {
            continue;
        };
        let owner_name = if owner.path.is_root() {
            String::from("<root>")
        } else {
            owner.path.to_string()
        };
        let ports = match owner.kind {
            OwnerKind::Graph => plan.graph_ports(owner_slot),
            OwnerKind::Node(slot) => plan
                .nodes
                .get(slot.index())
                .map(|node| node.ports.as_slice()),
        }
        .ok_or_else(|| PlanValidationError::InvalidGraphContract {
            owner: owner_name.clone(),
            detail: "the Loop owner has no port table",
        })?;
        let state = ports
            .iter()
            .find(|port| port.port == spec.entry && port.direction == node::PortDirection::Input)
            .and_then(|port| port.ty.clone())
            .ok_or_else(|| PlanValidationError::InvalidGraphContract {
                owner: owner_name.clone(),
                detail: "Loop.entry must name a typed input",
            })?;
        if !spec.terminals.windows(2).all(|pair| pair[0] < pair[1]) {
            return Err(PlanValidationError::InvalidGraphContract {
                owner: owner_name.clone(),
                detail: "Loop terminal outputs must be unique and canonically ordered",
            });
        }
        let connector = matches!(owner.kind, OwnerKind::Node(slot)
            if matches!(plan.nodes[slot.index()].execution, NodeExecution::Connector(_)));
        for terminal in &spec.terminals {
            if !ports
                .iter()
                .any(|port| port.port == *terminal && port.direction == node::PortDirection::Output)
            {
                return Err(PlanValidationError::InvalidGraphContract {
                    owner: owner_name.clone(),
                    detail: "Loop.terminal must name an ordinary output",
                });
            }
        }
        let expired = node::loop_expired_type(state.clone()).map_err(|_| {
            PlanValidationError::InvalidGraphContract {
                owner: owner_name.clone(),
                detail: "LoopExpired cannot be represented",
            }
        })?;
        match &spec.expired {
            Some(target) => {
                let terminal = if let OwnerKind::Node(slot) = owner.kind {
                    plan.nodes[slot.index()]
                        .connector()
                        .and_then(crate::plan::ConnectorSpec::call_semantics)
                        .map(|call| call.terminal())
                } else {
                    None
                };
                if !matches!(target.ty, DataType::TaggedUnion(_))
                    || !expired.is_assignable_to(&target.ty)
                    || !ports.iter().any(|port| {
                        port.port == target.port
                            && port.direction == node::PortDirection::Output
                            && port.ty.as_ref() == Some(&target.ty)
                    })
                    || if connector {
                        terminal != Some(target.port.name())
                    } else {
                        !spec.terminals.contains(&target.port)
                    }
                    || (spec.proof.is_some() && spec.fuel.is_none())
                {
                    return Err(PlanValidationError::InvalidGraphContract {
                        owner: owner_name.clone(),
                        detail: "Loop exhaustion must name the resolved compatible host terminal",
                    });
                }
            }
            None if spec.proof.is_some() && spec.fuel.is_none() => {}
            None => {
                return Err(PlanValidationError::InvalidGraphContract {
                    owner: owner_name.clone(),
                    detail: "productible Loop exhaustion needs a resolved host terminal",
                });
            }
        }
        if ports.iter().any(|port| {
            matches!(
                port.port.name(),
                "Loop.next" | "Loop.again" | "Loop.done" | "Loop.expired"
            )
        }) {
            return Err(PlanValidationError::InvalidGraphContract {
                owner: owner_name.clone(),
                detail: "Loop contributes no port",
            });
        }

        let validate_constant =
            |constant: &crate::plan::PlanConstant, expected: &DataType, detail: &'static str| {
                if !plan_constant_holds(plan, constant, expected)? {
                    return Err(PlanValidationError::InvalidGraphContract {
                        owner: owner_name.clone(),
                        detail,
                    });
                }
                Ok(())
            };
        if spec.fuel.is_none() && spec.proof.is_none() {
            return Err(PlanValidationError::InvalidGraphContract {
                owner: owner_name.clone(),
                detail: "Loop requires positive fuel or a verified measure",
            });
        }
        if let Some(fuel) = &spec.fuel {
            let fuel_type = match fuel.resolved {
                crate::value::Value::Int(value) if value > 0 => {
                    if i32::try_from(value).is_ok() {
                        DataType::INT
                    } else {
                        DataType::LONG
                    }
                }
                crate::value::Value::Uint(value) if value > 0 => DataType::UINT,
                _ => {
                    return Err(PlanValidationError::InvalidGraphContract {
                        owner: owner_name.clone(),
                        detail: "Loop.fuel must reduce to a positive integer",
                    });
                }
            };
            validate_constant(
                fuel,
                &fuel_type,
                "Loop.fuel is not its claimed plan constant",
            )?;
        }
        if let Some(proof) = &spec.proof {
            expr_canonical(&proof.measure.root)?;
            asset_indexes_bound(plan, &proof.measure.root)?;
            let theorem_signature = expr::Signature::new()
                .with("data", state.clone())
                .with("Loop.state", DataType::Variant)
                .with("Loop.state.data", state.clone())
                .with_metadata("Loop.state.meta");
            let measure_type = expr::rederive_type(&proof.measure.root, &theorem_signature)
                .ok_or_else(|| PlanValidationError::InvalidLoopControl {
                    owner: owner_name.clone(),
                    path: String::from("measure"),
                    detail: "the Loop measure type cannot be re-derived",
                })?;
            if measure_type != proof.measure.ty
                || !matches!(
                    measure_type,
                    DataType::Integer(crate::datatype::IntegerType::Signed(_))
                )
            {
                return Err(PlanValidationError::InvalidLoopControl {
                    owner: owner_name.clone(),
                    path: String::from("measure"),
                    detail: "the Loop measure is not a signed integer expression",
                });
            }
            if let Some(invariant) = &proof.invariant {
                expr_canonical(&invariant.root)?;
                asset_indexes_bound(plan, &invariant.root)?;
                let invariant_type = expr::rederive_type(&invariant.root, &theorem_signature)
                    .ok_or_else(|| PlanValidationError::InvalidLoopControl {
                        owner: owner_name.clone(),
                        path: String::from("invariant"),
                        detail: "the Loop invariant type cannot be re-derived",
                    })?;
                if invariant_type != invariant.ty || invariant_type != DataType::Bool {
                    return Err(PlanValidationError::InvalidLoopControl {
                        owner: owner_name.clone(),
                        path: String::from("invariant"),
                        detail: "the Loop invariant is not boolean",
                    });
                }
            }
            let verification = match owner.kind {
                OwnerKind::Graph => crate::loop_proof::derive_graph(
                    proof.measure.clone(),
                    proof.invariant.clone(),
                    plan,
                    owner,
                )
                .and_then(|derived| {
                    if derived.decreases == proof.decreases {
                        Ok(())
                    } else {
                        Err(crate::loop_proof::Error::CertificateMismatch)
                    }
                }),
                OwnerKind::Node(slot) => plan
                    .handlers
                    .iter()
                    .find(|handler| {
                        handler.owner == slot
                            && handler.activation
                                == node::HandlerActivation::Port(spec.entry.clone())
                    })
                    .ok_or(crate::loop_proof::Error::UnsupportedActivation)
                    .and_then(|handler| {
                        crate::loop_proof::verify_dispatch(proof, &handler.dispatch)
                    }),
            };
            verification.map_err(|error| {
                let detail = match error {
                    crate::loop_proof::Error::UnsupportedMeasure => {
                        "the Loop measure is outside the supported direct integer-path fragment"
                    }
                    crate::loop_proof::Error::UnsupportedInvariant => {
                        "the Loop invariant cannot be established at entry by this proof gate"
                    }
                    crate::loop_proof::Error::UnsupportedActivation => {
                        "a recurrent path is not a supported entry application"
                    }
                    crate::loop_proof::Error::AmbiguousRecurrence => {
                        "a recurrent handler does not select exactly one next state"
                    }
                    crate::loop_proof::Error::MeasureNotPositive => {
                        "a recurrent path does not prove that the Loop measure is positive"
                    }
                    crate::loop_proof::Error::MeasureDoesNotDecrease => {
                        "a recurrent path does not strictly decrease the Loop measure"
                    }
                    crate::loop_proof::Error::CertificateMismatch => {
                        "the Loop proof certificate disagrees with the shipped handlers"
                    }
                };
                PlanValidationError::InvalidLoopControl {
                    owner: owner_name.clone(),
                    path: String::from("measure"),
                    detail,
                }
            })?;
        }
        match &spec.delay {
            None => {}
            Some(crate::plan::DelayPolicy::Fixed { after }) => {
                validate_constant(
                    after,
                    &DataType::DURATION,
                    "Loop.delay.after is not a positive duration constant",
                )?;
                if !matches!(after.resolved, crate::value::Value::Duration(value) if value > 0) {
                    return Err(PlanValidationError::InvalidGraphContract {
                        owner: owner_name,
                        detail: "Loop.delay.after is not positive",
                    });
                }
            }
            Some(crate::plan::DelayPolicy::Exponential { first, ceiling }) => {
                validate_constant(
                    first,
                    &DataType::DURATION,
                    "Loop.delay.first is not a positive duration constant",
                )?;
                validate_constant(
                    ceiling,
                    &DataType::DURATION,
                    "Loop.delay.ceiling is not a positive duration constant",
                )?;
                let (crate::value::Value::Duration(first), crate::value::Value::Duration(ceiling)) =
                    (&first.resolved, &ceiling.resolved)
                else {
                    return Err(PlanValidationError::InvalidGraphContract {
                        owner: owner_name,
                        detail: "Loop.delay exponential fields are not durations",
                    });
                };
                if *first <= 0 || ceiling < first {
                    return Err(PlanValidationError::InvalidGraphContract {
                        owner: owner_name,
                        detail: "Loop.delay.ceiling must be no smaller than positive first",
                    });
                }
            }
        }
    }
    Ok(())
}

fn owners_well_formed(plan: &ExecutionPlan) -> Result<(), PlanValidationError> {
    let Some(root) = plan.owners.first() else {
        return Err(PlanValidationError::MissingRootOwner);
    };
    if !root.path.is_root() || root.parent.is_some() || root.kind != OwnerKind::Graph {
        return Err(PlanValidationError::MissingRootOwner);
    }

    let mut paths = BTreeSet::new();
    let mut owned_nodes = BTreeSet::new();
    for (index, owner) in plan.owners.iter().enumerate() {
        let slot = node::OwnerSlot::new(u32::try_from(index).unwrap_or(u32::MAX));
        let path = owner.path.to_string();
        if !paths.insert(path.clone()) {
            return Err(PlanValidationError::DuplicateOwnerPath { path });
        }
        if index > 0 {
            let Some(parent_slot) = owner.parent else {
                return Err(PlanValidationError::InvalidOwnerTree { owner: slot });
            };
            if parent_slot.index() >= index {
                return Err(PlanValidationError::InvalidOwnerTree { owner: slot });
            }
            let Some(parent) = plan.owners.get(parent_slot.index()) else {
                return Err(PlanValidationError::OwnerOutOfBounds {
                    owner: parent_slot,
                    owner_count: plan.owners.len(),
                });
            };
            if parent.kind != OwnerKind::Graph
                || owner.path.segments().len() != parent.path.segments().len() + 1
                || !owner.path.segments().starts_with(parent.path.segments())
            {
                return Err(PlanValidationError::InvalidOwnerTree { owner: slot });
            }
        }

        if let OwnerKind::Node(node_slot) = owner.kind {
            if !owner.ports.is_empty() {
                return Err(PlanValidationError::InvalidGraphContract {
                    owner: path.clone(),
                    detail: "Node owner ports belong only to the Node table",
                });
            }
            let Some(node) = plan.nodes.get(node_slot.index()) else {
                return Err(PlanValidationError::SlotOutOfBounds {
                    slot: node_slot,
                    node_count: plan.nodes.len(),
                });
            };
            if node.owner != slot || node.path.segments() != owner.path.segments() {
                return Err(PlanValidationError::OwnerNodeMismatch {
                    node: node_slot,
                    owner: slot,
                });
            }
            if !owned_nodes.insert(node_slot) {
                return Err(PlanValidationError::OwnerNodeMismatch {
                    node: node_slot,
                    owner: slot,
                });
            }
        } else {
            if owner.path.is_root() && !owner.ports.is_empty() {
                return Err(PlanValidationError::InvalidGraphContract {
                    owner: path.clone(),
                    detail: "the root Graph boundary belongs only to the plan boundary table",
                });
            }
            // `ExecutionPlan::boundary` remains the root Graph's one port
            // table. Nested Graph owners have no other table and are
            // validated here directly.
            if !owner.path.is_root() {
                let mut ports = BTreeSet::new();
                for port in &owner.ports {
                    if !ports.insert(port.port.clone()) {
                        return Err(PlanValidationError::InvalidGraphContract {
                            owner: path.clone(),
                            detail: "a Graph boundary port is declared twice",
                        });
                    }
                }
                if !owner
                    .ports
                    .windows(2)
                    .all(|pair| pair[0].key() < pair[1].key())
                {
                    return Err(PlanValidationError::NonCanonicalOrder {
                        collection: "Graph owner ports",
                    });
                }
            }
            if !owner
                .extensions
                .windows(2)
                .all(|pair| pair[0].extension() < pair[1].extension())
            {
                return Err(PlanValidationError::NonCanonicalOrder {
                    collection: "Graph extensions",
                });
            }
        }
        let is_app = owner
            .extensions
            .iter()
            .any(|extension| matches!(extension, crate::plan::ExtensionSpec::App));
        if !owner
            .extensions
            .windows(2)
            .all(|pair| pair[0].extension() < pair[1].extension())
        {
            return Err(PlanValidationError::NonCanonicalOrder {
                collection: "owner extensions",
            });
        }
        if is_app != owner.path.is_root() {
            return Err(PlanValidationError::InvalidGraphContract {
                owner: path.clone(),
                detail: "App must occur exactly once, on the root Graph",
            });
        }

        let duration = match owner.instance.lifetime {
            instance::Lifetime::Persistent | instance::Lifetime::SinglePass => None,
            instance::Lifetime::Idle(value)
            | instance::Lifetime::Fixed(value)
            | instance::Lifetime::Tumbling(value) => Some(value),
        };
        if duration.is_some_and(|value| value <= 0) {
            return Err(PlanValidationError::InvalidInstanceLifetime { owner: path });
        }
        if owner
            .instance
            .closing
            .is_some_and(|closing| closing.drain_for <= 0 || closing.settle_for <= 0)
        {
            return Err(PlanValidationError::InvalidClosingPolicy { owner: path });
        }
        let explicitly_stoppable = plan.handlers.iter().any(|handler| {
            let belongs_to_owner = plan
                .nodes
                .get(handler.owner.index())
                .is_some_and(|node| node.owner == slot);
            belongs_to_owner
                && handler
                    .dispatch
                    .steps()
                    .any(|step| matches!(step, DispatchStep::Stop))
        }) || plan.routes.iter().any(|rule| {
            let matches_target = |destination: &super::RouteDestination| {
                let super::RouteDestination::InstanceStop(target) = destination else {
                    return false;
                };
                match &target.owner {
                    super::EndpointOwner::Graph(path) => path == &owner.path,
                    super::EndpointOwner::Node(path) => path.segments() == owner.path.segments(),
                }
            };
            match &rule.selection {
                super::RouteSelection::All(set) => set.iter().any(matches_target),
                super::RouteSelection::Cases(arms) => arms
                    .iter()
                    .flat_map(|arm| &arm.destinations)
                    .any(matches_target),
            }
        });
        if owner.instance.lifetime != instance::Lifetime::Persistent || explicitly_stoppable {
            for node in &plan.nodes {
                let Some(spec) = node.connector() else {
                    continue;
                };
                if !spec.roles.has_outbound() || !owner_contains(plan, slot, node.owner) {
                    continue;
                }
                if owner.instance.closing.is_none() {
                    return Err(PlanValidationError::MissingClosingPolicy {
                        connector: node.path.to_string(),
                        owner: path,
                    });
                }
            }
        }
        if let Some(key) = &owner.instance.key {
            if !key.ty.is_instance_key()
                && !super::type_resolution::expression_reads_data(&key.root)
            {
                return Err(PlanValidationError::InvalidInstanceKey {
                    owner: path.clone(),
                    ty: key.ty.clone(),
                });
            }
            expr_canonical(&key.root)?;
            asset_indexes_bound(plan, &key.root)?;
            let mut enclosing = BTreeSet::new();
            let mut cursor = owner.parent;
            while let Some(parent) = cursor {
                let Some(ancestor) = plan.owners.get(parent.index()) else {
                    return Err(PlanValidationError::OwnerOutOfBounds {
                        owner: parent,
                        owner_count: plan.owners.len(),
                    });
                };
                enclosing.extend(ancestor.vals.iter().map(|value| value.name.as_str()));
                cursor = ancestor.parent;
            }
            instance_bindings_available(&key.root, &path, &enclosing)?;
        }

        let mut names = BTreeSet::new();
        for value in &owner.vals {
            if !names.insert(value.name.as_str()) {
                return Err(PlanValidationError::DuplicateOwnerValue {
                    owner: path.clone(),
                    name: value.name.clone(),
                });
            }
            if (value.site == EvaluationSite::Plan) != value.resolved.is_some() {
                return Err(PlanValidationError::BindingSiteMismatch {
                    owner: path.clone(),
                    name: value.name.clone(),
                });
            }
            if value.site == EvaluationSite::Invocation {
                return Err(PlanValidationError::BindingSiteMismatch {
                    owner: path.clone(),
                    name: value.name.clone(),
                });
            }
            if let Some(resolved) = &value.resolved {
                value_expressible(resolved)?;
                if !value.expr.ty.accepts(resolved) {
                    return Err(PlanValidationError::UnrepresentableConstant);
                }
            }
            expr_canonical(&value.expr.root)?;
            asset_indexes_bound(plan, &value.expr.root)?;
        }
        for pair in owner.vals.windows(2) {
            if pair[0].name >= pair[1].name {
                return Err(PlanValidationError::NonCanonicalOrder {
                    collection: "owner values",
                });
            }
        }
    }
    for pair in plan.owners.windows(2) {
        if pair[0].path >= pair[1].path {
            return Err(PlanValidationError::NonCanonicalOrder {
                collection: "owners",
            });
        }
    }
    if owned_nodes.len() != plan.nodes.len() {
        let missing = (0..plan.nodes.len())
            .map(|index| NodeIndex::new(u32::try_from(index).unwrap_or(u32::MAX)))
            .find(|slot| !owned_nodes.contains(slot))
            .unwrap_or_else(|| NodeIndex::new(u32::MAX));
        let owner = plan
            .nodes
            .get(missing.index())
            .map_or(node::OwnerSlot::new(u32::MAX), |node| node.owner);
        return Err(PlanValidationError::OwnerNodeMismatch {
            node: missing,
            owner,
        });
    }
    let is_descendant = |mut owner: node::OwnerSlot, ancestor: node::OwnerSlot| {
        loop {
            if owner == ancestor {
                return true;
            }
            let Some(row) = plan.owner(owner) else {
                return false;
            };
            let Some(parent) = row.parent else {
                return false;
            };
            owner = parent;
        }
    };
    for edge in &plan.edges {
        if !matches!(
            plan.owner(edge.owner).map(|owner| &owner.kind),
            Some(OwnerKind::Graph)
        ) {
            return Err(PlanValidationError::InvalidEdgeScope { owner: edge.owner });
        }
        let endpoint_is_scoped = |endpoint: &EdgeEnd| {
            endpoint.node.is_none_or(|slot| {
                plan.nodes
                    .get(slot.index())
                    .is_some_and(|node| is_descendant(node.owner, edge.owner))
            })
        };
        if !endpoint_is_scoped(&edge.from)
            || edge.to.as_ref().is_some_and(|to| !endpoint_is_scoped(to))
        {
            return Err(PlanValidationError::InvalidEdgeScope { owner: edge.owner });
        }
    }
    owner_values_well_formed(plan)?;
    Ok(())
}

fn owner_contains(plan: &ExecutionPlan, ancestor: node::OwnerSlot, child: node::OwnerSlot) -> bool {
    let mut cursor = Some(child);
    while let Some(owner) = cursor {
        if owner == ancestor {
            return true;
        }
        cursor = plan
            .owners
            .get(owner.index())
            .and_then(|candidate| candidate.parent);
    }
    false
}

fn validate(plan: &ExecutionPlan) -> Result<(), PlanValidationError> {
    let depth = super::serialization::deepest_level(plan);
    if depth >= crate::canonical_cbor::MAX_NESTING {
        return Err(PlanValidationError::NestingExceeded {
            depth,
            bound: crate::canonical_cbor::MAX_NESTING,
        });
    }
    for (handler, definition) in plan.handlers.iter().enumerate() {
        let handler_owner = plan.nodes.get(definition.owner.index()).ok_or(
            PlanValidationError::SlotOutOfBounds {
                slot: definition.owner,
                node_count: plan.nodes.len(),
            },
        )?;
        if definition.guard.is_some()
            && !matches!(definition.activation, node::HandlerActivation::Join { .. })
        {
            return Err(PlanValidationError::UnsupportedStatement {
                handler,
                statement: "a one-port filter must be represented by an OpenMatch Dispatch",
            });
        }
        if definition
            .dispatch
            .steps()
            .any(|step| matches!(step, DispatchStep::LoopCommand(_)))
            && !plan
                .owners
                .get(handler_owner.owner.index())
                .is_some_and(|owner| owner.loop_spec().is_some())
        {
            return Err(PlanValidationError::UnsupportedStatement {
                handler,
                statement: "a Loop command without an owning Loop application",
            });
        }
    }
    data_types_well_formed(plan)?;
    schema_well_formed("app", &plan.app_schema)?;
    schema_well_formed("ext", &plan.ext_schema)?;
    assets_well_formed(plan)?;
    // Uniqueness first, GLOBALLY, then canonical order: a collection
    // that is both duplicated and disordered refuses as the
    // DUPLICATE — the dedicated variant names the defect wherever
    // the copies sit, adjacent or not. (Edges are the stated
    // exception: a doubled edge has no dedicated variant — the
    // checker refuses it at the surface as E-DUP-EDGE — so a raw
    // plan carrying one refuses as the non-canonical order it also
    // is.)
    let mut paths = BTreeSet::new();
    for node in &plan.nodes {
        if !paths.insert(&node.path) {
            return Err(PlanValidationError::DuplicateNodePath {
                path: node.path.clone(),
            });
        }
    }
    for pair in plan.nodes.windows(2) {
        if (execution_rank(&pair[0].execution), pair[0].path.to_string())
            >= (execution_rank(&pair[1].execution), pair[1].path.to_string())
        {
            return Err(PlanValidationError::NonCanonicalOrder {
                collection: "nodes",
            });
        }
    }
    owners_well_formed(plan)?;
    let mut boundary_ports = BTreeSet::new();
    for decl in &plan.boundary {
        if !boundary_ports.insert(&decl.port) {
            return Err(PlanValidationError::DuplicateBoundaryPort {
                port: decl.port.clone(),
            });
        }
    }
    for pair in plan.boundary.windows(2) {
        if pair[0].key() >= pair[1].key() {
            return Err(PlanValidationError::NonCanonicalOrder {
                collection: "boundary",
            });
        }
    }
    let conventional_direction = |port: &node::Port| match port.name() {
        "in" => Some(node::PortDirection::Input),
        "out" | "err" => Some(node::PortDirection::Output),
        _ => None,
    };
    if let Some((decl, expected)) = plan.boundary.iter().find_map(|decl| {
        conventional_direction(&decl.port)
            .filter(|expected| decl.direction != *expected)
            .map(|expected| (decl, expected))
    }) {
        return Err(PlanValidationError::WrongPortDirection {
            node: None,
            port: decl.port.clone(),
            expected,
        });
    }
    // The root boundary has passed its own uniqueness and canonicality gates;
    // only now may an aspect derive a contract from it. This preserves the
    // validation rule that the table's structural defect speaks before a
    // dependent semantic claim.
    loop_specs_well_formed(plan)?;
    for (node_index, node) in plan.nodes.iter().enumerate() {
        if let Some((decl, expected)) = node.ports.iter().find_map(|decl| {
            conventional_direction(&decl.port)
                .filter(|expected| decl.direction != *expected)
                .map(|expected| (decl, expected))
        }) {
            return Err(PlanValidationError::WrongPortDirection {
                node: Some(node.path.clone()),
                port: decl.port.clone(),
                expected,
            });
        }
        let mut ports = BTreeSet::new();
        for decl in &node.ports {
            if !ports.insert(&decl.port) {
                return Err(PlanValidationError::DuplicatePort {
                    node: node.path.clone(),
                    port: decl.port.clone(),
                });
            }
        }
        for pair in node.ports.windows(2) {
            if pair[0].key() >= pair[1].key() {
                return Err(PlanValidationError::NonCanonicalOrder {
                    collection: "ports",
                });
            }
        }
        let mut vars = BTreeSet::new();
        for var in &node.vars {
            if !vars.insert(var.name.as_str()) {
                return Err(PlanValidationError::DuplicateVar {
                    node: node.path.clone(),
                    name: var.name.clone(),
                });
            }
            if let Some(init) = &var.init {
                value_expressible(init)?;
                if !var.ty.accepts(init) {
                    return Err(PlanValidationError::UnrepresentableConstant);
                }
            }
        }
        for pair in node.vars.windows(2) {
            if pair[0].name >= pair[1].name {
                return Err(PlanValidationError::NonCanonicalOrder { collection: "vars" });
            }
        }
        if let NodeExecution::Connector(spec) = &node.execution {
            for (name, setting) in &spec.attributes {
                match setting {
                    crate::plan::AttrSetting::Value(value) => value_expressible(value)?,
                    crate::plan::AttrSetting::Expr { site, expr } => {
                        let admitted =
                            crate::boundary::connector_attribute_site(&spec.contract, name)
                                .is_some_and(|latest| {
                                    latest != EvaluationSite::Plan && latest.admits(*site)
                                });
                        if *site == EvaluationSite::Plan || !admitted {
                            return Err(PlanValidationError::BindingSiteMismatch {
                                owner: node.path.to_string(),
                                name: name.clone(),
                            });
                        }
                        match expr::boot_reference(&expr.root) {
                            Ok(Some(reference)) => {
                                let admitted = crate::boundary::connector_boot_reference_policy(
                                    &spec.contract,
                                    name,
                                )
                                .is_some_and(|policy| policy.admits(&reference));
                                if *site != EvaluationSite::Boot || !admitted {
                                    return Err(PlanValidationError::BindingSiteMismatch {
                                        owner: node.path.to_string(),
                                        name: name.clone(),
                                    });
                                }
                            }
                            Ok(None) if *site == EvaluationSite::Boot => {
                                return Err(PlanValidationError::BindingSiteMismatch {
                                    owner: node.path.to_string(),
                                    name: name.clone(),
                                });
                            }
                            Err(_) => {
                                return Err(PlanValidationError::BindingSiteMismatch {
                                    owner: node.path.to_string(),
                                    name: name.clone(),
                                });
                            }
                            Ok(None) => {}
                        }
                        expr_canonical(&expr.root)?;
                        asset_indexes_bound(plan, &expr.root)?;
                    }
                    crate::plan::AttrSetting::Word(_) => {}
                }
            }
            // The codec rule is the PROCESS contract's, not the
            // attribute name's: the runtime dispatches on it, so a
            // `Process` plan must carry `codec` (the sheet requires
            // it) and carry it inside the dispatch domain — an
            // unknown or absent one must never earn a
            // ValidatedExecutionPlan and quietly run as `lines`
            // (D-2026-08-11-1, second amendment). Another sheet
            // spelling its own `codec` vocabulary is untouched.
            // The Contract roles distinguish an outgoing request from a
            // retained inbound Call response delivered on an input port.
            let slot = crate::plan::NodeIndex::new(u32::try_from(node_index).unwrap_or(u32::MAX));
            let receives = plan
                .edges
                .iter()
                .any(|edge| edge.to.as_ref().is_some_and(|to| to.node == Some(slot)));
            let receives_inbound_call_response = receives
                && spec.roles.has_inbound()
                && !spec.roles.has_outbound()
                && spec.exchange == crate::connector::ConnectorExchange::Call;
            if receives && !spec.roles.has_outbound() && !receives_inbound_call_response {
                return Err(PlanValidationError::CallableConnectorWithoutOutboundRole {
                    node: node.path.clone(),
                });
            }
            if receives_inbound_call_response
                && spec.contract.as_str() == crate::connector::HTTP_SERVER_CONNECTOR_ID
                && !matches!(spec.attribute("mode"), Some(crate::plan::AttrSetting::Word(mode)) if mode == "respond")
            {
                return Err(PlanValidationError::ContractAttributeMismatch {
                    node: node.path.clone(),
                    attribute: String::from("mode"),
                });
            }
            if receives_inbound_call_response
                && spec.contract.as_str() == crate::connector::HTTP_SERVER_CONNECTOR_ID
                && spec.attribute("correlate").is_none()
                && response_path_crosses_join(plan, slot)
            {
                return Err(PlanValidationError::AmbiguousInboundCallCorrelation {
                    node: node.path.clone(),
                });
            }
            if matches!(
                spec.contract.as_str(),
                crate::connector::PROCESS_CONNECTOR_ID
                    | crate::connector::PROCESS_SERVICE_CONNECTOR_ID
            ) {
                let service =
                    spec.contract.as_str() == crate::connector::PROCESS_SERVICE_CONNECTOR_ID;
                let roles = if service {
                    crate::connector::ConnectorDirection::InboundAndOutbound
                } else {
                    crate::connector::ConnectorDirection::Outbound
                };
                let exchange = if service {
                    crate::connector::ConnectorExchange::Session
                } else {
                    crate::connector::ConnectorExchange::Call
                };
                if spec.roles != roles {
                    return Err(PlanValidationError::ContractRoleMismatch {
                        node: node.path.clone(),
                    });
                }
                if spec.exchange != exchange {
                    return Err(PlanValidationError::ContractExchangeMismatch {
                        node: node.path.clone(),
                    });
                }
                match spec.attribute("mode") {
                    Some(crate::plan::AttrSetting::Word(word))
                        if word == if service { "service" } else { "job" } => {}
                    Some(_) => {
                        return Err(PlanValidationError::ContractAttributeMismatch {
                            node: node.path.clone(),
                            attribute: String::from("mode"),
                        });
                    }
                    None => {
                        return Err(PlanValidationError::MissingContractAttribute {
                            node: node.path.clone(),
                            attribute: "mode",
                        });
                    }
                }
                match spec.attribute("on_timeout") {
                    None if service => {}
                    Some(crate::plan::AttrSetting::Word(word)) if !service && word == "kill" => {}
                    Some(_) => {
                        return Err(PlanValidationError::ContractAttributeMismatch {
                            node: node.path.clone(),
                            attribute: String::from("on_timeout"),
                        });
                    }
                    None => {
                        return Err(PlanValidationError::MissingContractAttribute {
                            node: node.path.clone(),
                            attribute: "on_timeout",
                        });
                    }
                }
                match spec.attribute("command") {
                    Some(crate::plan::AttrSetting::Value(crate::value::Value::Text(_))) => {}
                    Some(crate::plan::AttrSetting::Expr { site, expr })
                        if *site == EvaluationSite::Instance && expr.ty == DataType::Text => {}
                    Some(_) => {
                        return Err(PlanValidationError::ContractAttributeMismatch {
                            node: node.path.clone(),
                            attribute: String::from("command"),
                        });
                    }
                    None => {
                        return Err(PlanValidationError::MissingContractAttribute {
                            node: node.path.clone(),
                            attribute: "command",
                        });
                    }
                }
                match spec.attribute("args") {
                    Some(crate::plan::AttrSetting::Value(crate::value::Value::Array(items)))
                        if items
                            .iter()
                            .all(|item| matches!(item, crate::value::Value::Text(_))) => {}
                    Some(crate::plan::AttrSetting::Expr { site, expr })
                        if *site == EvaluationSite::Instance
                            && expr.ty == DataType::array(DataType::Text) => {}
                    Some(_) => {
                        return Err(PlanValidationError::ContractAttributeMismatch {
                            node: node.path.clone(),
                            attribute: String::from("args"),
                        });
                    }
                    None => {
                        return Err(PlanValidationError::MissingContractAttribute {
                            node: node.path.clone(),
                            attribute: "args",
                        });
                    }
                }
                if let Some(cwd) = spec.attribute("cwd") {
                    let valid = match cwd {
                        crate::plan::AttrSetting::Value(crate::value::Value::Text(_)) => true,
                        crate::plan::AttrSetting::Expr { site, expr } => {
                            *site == EvaluationSite::Instance && expr.ty == DataType::Text
                        }
                        _ => false,
                    };
                    if !valid {
                        return Err(PlanValidationError::ContractAttributeMismatch {
                            node: node.path.clone(),
                            attribute: String::from("cwd"),
                        });
                    }
                }
                if let Some(env) = spec.attribute("env") {
                    let valid = match env {
                        crate::plan::AttrSetting::Value(crate::value::Value::Map(entries)) => {
                            entries
                                .values()
                                .all(|value| matches!(value, crate::value::Value::Text(_)))
                        }
                        crate::plan::AttrSetting::Expr { site, expr } => {
                            *site == EvaluationSite::Instance
                                && expr.ty == DataType::text_map(DataType::Text)
                        }
                        _ => false,
                    };
                    if !valid {
                        return Err(PlanValidationError::ContractAttributeMismatch {
                            node: node.path.clone(),
                            attribute: String::from("env"),
                        });
                    }
                }
                if let Some(timeout) = spec.attribute("timeout") {
                    let valid = match timeout {
                        crate::plan::AttrSetting::Value(crate::value::Value::Duration(_)) => true,
                        crate::plan::AttrSetting::Expr { site, expr } => {
                            *site == EvaluationSite::Instance && expr.ty == DataType::DURATION
                        }
                        _ => false,
                    };
                    if !valid || service {
                        return Err(PlanValidationError::ContractAttributeMismatch {
                            node: node.path.clone(),
                            attribute: String::from("timeout"),
                        });
                    }
                }
                let size_attribute = if service { "max_frame" } else { "max_output" };
                match spec.attribute(size_attribute) {
                    Some(crate::plan::AttrSetting::Value(crate::value::Value::Text(value)))
                        if crate::connector::byte_size(value)
                            .is_some_and(|size| !service || size > 0) => {}
                    Some(_) => {
                        return Err(PlanValidationError::ContractAttributeMismatch {
                            node: node.path.clone(),
                            attribute: String::from(size_attribute),
                        });
                    }
                    None => {
                        return Err(PlanValidationError::MissingContractAttribute {
                            node: node.path.clone(),
                            attribute: size_attribute,
                        });
                    }
                }
                let codec = match spec.attribute("codec") {
                    Some(crate::plan::AttrSetting::Word(word))
                        if matches!(word.as_str(), "bytes" | "jsonl" | "lines")
                            && !(service && word == "bytes") =>
                    {
                        word.as_str()
                    }
                    Some(_) => {
                        return Err(PlanValidationError::UnknownCodec {
                            node: node.path.clone(),
                        });
                    }
                    None => {
                        return Err(PlanValidationError::MissingCodec {
                            node: node.path.clone(),
                        });
                    }
                };
                let err_codec = match spec.attribute("err_codec") {
                    Some(crate::plan::AttrSetting::Word(word))
                        if matches!(word.as_str(), "bytes" | "lines")
                            && !(service && word == "bytes") =>
                    {
                        word.as_str()
                    }
                    Some(_) => {
                        return Err(PlanValidationError::UnknownErrCodec {
                            node: node.path.clone(),
                        });
                    }
                    None => {
                        return Err(PlanValidationError::MissingErrCodec {
                            node: node.path.clone(),
                        });
                    }
                };
                for (name, _) in &spec.attributes {
                    if !matches!(
                        name.as_str(),
                        "args"
                            | "codec"
                            | "err_codec"
                            | "command"
                            | "cwd"
                            | "timeout"
                            | "env"
                            | "mode"
                            | "max_output"
                            | "max_frame"
                            | "on_timeout"
                            | "idempotent"
                    ) || (service
                        && matches!(
                            name.as_str(),
                            "max_output" | "on_timeout" | "timeout" | "idempotent"
                        ))
                        || (!service && name == "max_frame")
                    {
                        return Err(PlanValidationError::ContractAttributeMismatch {
                            node: node.path.clone(),
                            attribute: name.clone(),
                        });
                    }
                }
                let expected = [
                    ("in", node::PortDirection::Input),
                    ("out", node::PortDirection::Output),
                    ("err", node::PortDirection::Output),
                    ("exit", node::PortDirection::Output),
                ];
                for (name, direction) in expected {
                    let Some(port) = node.ports.iter().find(|port| port.port.name() == name) else {
                        return Err(PlanValidationError::MissingContractPort {
                            node: node.path.clone(),
                            port: name,
                        });
                    };
                    let expected_ty = effective_contract_port(
                        node,
                        plan.owners[node.owner.index()].loop_spec(),
                        name,
                        crate::boundary::process_port_type(
                            &spec.contract,
                            name,
                            Some(codec),
                            Some(err_codec),
                        ),
                    );
                    if port.direction != direction || port.ty != expected_ty {
                        return Err(PlanValidationError::ContractPortMismatch {
                            node: node.path.clone(),
                            port: port.port.clone(),
                        });
                    }
                }
                if let Some(port) = node
                    .ports
                    .iter()
                    .find(|port| !matches!(port.port.name(), "in" | "out" | "err" | "exit"))
                {
                    return Err(PlanValidationError::UnexpectedContractPort {
                        node: node.path.clone(),
                        port: port.port.clone(),
                    });
                }
            }
            if spec.contract.as_str() == crate::connector::FILE_CONNECTOR_ID {
                if spec.roles != crate::connector::ConnectorDirection::Outbound {
                    return Err(PlanValidationError::ContractRoleMismatch {
                        node: node.path.clone(),
                    });
                }
                if spec.exchange != crate::connector::ConnectorExchange::Call {
                    return Err(PlanValidationError::ContractExchangeMismatch {
                        node: node.path.clone(),
                    });
                }
                if spec.behavior != node::Behavior::Deterministic {
                    return Err(PlanValidationError::ContractAttributeMismatch {
                        node: node.path.clone(),
                        attribute: String::from("behavior"),
                    });
                }
                let mode = match spec.attribute("mode") {
                    Some(crate::plan::AttrSetting::Word(word))
                        if matches!(word.as_str(), "read" | "replace") =>
                    {
                        word.as_str()
                    }
                    Some(_) => {
                        return Err(PlanValidationError::ContractAttributeMismatch {
                            node: node.path.clone(),
                            attribute: String::from("mode"),
                        });
                    }
                    None => {
                        return Err(PlanValidationError::MissingContractAttribute {
                            node: node.path.clone(),
                            attribute: "mode",
                        });
                    }
                };
                match spec.attribute("path") {
                    Some(crate::plan::AttrSetting::Value(crate::value::Value::Text(_))) => {}
                    Some(crate::plan::AttrSetting::Expr { site, expr })
                        if matches!(
                            site,
                            EvaluationSite::Instance | EvaluationSite::Invocation
                        ) && expr.ty == DataType::Text => {}
                    Some(_) => {
                        return Err(PlanValidationError::ContractAttributeMismatch {
                            node: node.path.clone(),
                            attribute: String::from("path"),
                        });
                    }
                    None => {
                        return Err(PlanValidationError::MissingContractAttribute {
                            node: node.path.clone(),
                            attribute: "path",
                        });
                    }
                }
                if let Some(timeout) = spec.attribute("timeout") {
                    let valid = match timeout {
                        crate::plan::AttrSetting::Value(crate::value::Value::Duration(_)) => true,
                        crate::plan::AttrSetting::Expr { site, expr } => {
                            *site == EvaluationSite::Instance && expr.ty == DataType::DURATION
                        }
                        _ => false,
                    };
                    if !valid {
                        return Err(PlanValidationError::ContractAttributeMismatch {
                            node: node.path.clone(),
                            attribute: String::from("timeout"),
                        });
                    }
                }
                match (mode, spec.attribute("max_read")) {
                    (
                        "read",
                        Some(crate::plan::AttrSetting::Value(crate::value::Value::Text(value))),
                    ) if crate::connector::byte_size(value).is_some() => {}
                    ("read", None) => {
                        return Err(PlanValidationError::MissingContractAttribute {
                            node: node.path.clone(),
                            attribute: "max_read",
                        });
                    }
                    ("replace", None) => {}
                    _ => {
                        return Err(PlanValidationError::ContractAttributeMismatch {
                            node: node.path.clone(),
                            attribute: String::from("max_read"),
                        });
                    }
                }
                for (name, _) in &spec.attributes {
                    let allowed =
                        matches!(name.as_str(), "mode" | "path" | "timeout" | "idempotent")
                            || (mode == "read" && name == "max_read");
                    if !allowed {
                        return Err(PlanValidationError::ContractAttributeMismatch {
                            node: node.path.clone(),
                            attribute: name.clone(),
                        });
                    }
                }
                let expected = [
                    ("in", node::PortDirection::Input),
                    ("out", node::PortDirection::Output),
                    ("exit", node::PortDirection::Output),
                ];
                for (name, direction) in expected {
                    let Some(port) = node.ports.iter().find(|port| port.port.name() == name) else {
                        return Err(PlanValidationError::MissingContractPort {
                            node: node.path.clone(),
                            port: name,
                        });
                    };
                    if port.direction != direction
                        || port.ty
                            != effective_contract_port(
                                node,
                                plan.owners[node.owner.index()].loop_spec(),
                                name,
                                crate::boundary::file_port_type(&spec.contract, name, Some(mode)),
                            )
                    {
                        return Err(PlanValidationError::ContractPortMismatch {
                            node: node.path.clone(),
                            port: port.port.clone(),
                        });
                    }
                }
                if let Some(port) = node
                    .ports
                    .iter()
                    .find(|port| !matches!(port.port.name(), "in" | "out" | "exit"))
                {
                    return Err(PlanValidationError::UnexpectedContractPort {
                        node: node.path.clone(),
                        port: port.port.clone(),
                    });
                }
            }
            validate_network_connector(node, spec, plan.owners[node.owner.index()].loop_spec())?;
            // Built-in sheets above retain their more specific missing-row
            // diagnostics. Foreign families still cannot bypass the generic
            // exchange lifecycle.
            let exits = node
                .ports
                .iter()
                .filter(|port| {
                    port.port.name() == "exit" && port.direction == node::PortDirection::Output
                })
                .count();
            let terminal_shape_holds = match spec.exchange {
                crate::connector::ConnectorExchange::Event => exits == 0,
                crate::connector::ConnectorExchange::Call
                | crate::connector::ConnectorExchange::Session => exits == 1,
            };
            if !terminal_shape_holds {
                return Err(PlanValidationError::ExchangeTerminalMismatch {
                    node: node.path.clone(),
                    exchange: spec.exchange,
                });
            }
            let roles_hold = match spec.exchange {
                crate::connector::ConnectorExchange::Event => {
                    spec.roles == crate::connector::ConnectorDirection::Inbound
                }
                crate::connector::ConnectorExchange::Call => true,
                crate::connector::ConnectorExchange::Session => spec.roles.has_outbound(),
            };
            if !roles_hold {
                return Err(PlanValidationError::ExchangeRoleMismatch {
                    node: node.path.clone(),
                    exchange: spec.exchange,
                    roles: spec.roles,
                });
            }
            let mut attributes = BTreeSet::new();
            for (name, _) in &spec.attributes {
                if !attributes.insert(name.as_str()) {
                    return Err(PlanValidationError::DuplicateAttribute {
                        node: node.path.clone(),
                        name: name.clone(),
                    });
                }
            }
            for pair in spec.attributes.windows(2) {
                if pair[0].0 >= pair[1].0 {
                    return Err(PlanValidationError::NonCanonicalOrder {
                        collection: "attributes",
                    });
                }
            }
        }
    }

    // References: slots, ports in their roles, handlers, vars. Every port is
    // now a row; conventional source spelling confers no hidden existence.
    let has_port = |node: &Node, port: &node::Port, direction: node::PortDirection| {
        node.ports
            .iter()
            .any(|decl| &decl.port == port && decl.direction == direction)
    };
    let node_at = |slot: NodeIndex| {
        plan.nodes
            .get(slot.index())
            .ok_or(PlanValidationError::SlotOutOfBounds {
                slot,
                node_count: plan.nodes.len(),
            })
    };
    // `position` is the node-side role of the endpoint: Output for a
    // source, Input for a target. The boundary convention INVERTS:
    // an edge sourced at the boundary consumes a graph INPUT, an
    // edge targeting the boundary feeds a graph OUTPUT — the same
    // rule the checker applies to the surface language.
    let endpoint_ok =
        |endpoint: &EdgeEnd, position: node::PortDirection| -> Result<(), PlanValidationError> {
            match endpoint.node {
                Some(slot) => {
                    let node = node_at(slot)?;
                    if !has_port(node, &endpoint.port, position) {
                        return Err(PlanValidationError::WrongPortDirection {
                            node: Some(node.path.clone()),
                            port: endpoint.port.clone(),
                            expected: position,
                        });
                    }
                    // The table ANSWERED for the port — but the
                    // OutboundConnectorDriver seam cannot execute it yet: refuse by
                    // capability, distinctly from ignorance.
                    if matches!(node.execution, NodeExecution::Connector(_))
                        && !matches!(endpoint.port.name(), "in" | "out" | "err" | "exit")
                    {
                        return Err(PlanValidationError::UnsupportedConnectorPort {
                            node: node.path.clone(),
                            port: endpoint.port.clone(),
                        });
                    }
                    Ok(())
                }
                None => {
                    let expected = match position {
                        node::PortDirection::Output => node::PortDirection::Input,
                        node::PortDirection::Input => node::PortDirection::Output,
                    };
                    let actual = plan
                        .boundary
                        .iter()
                        .find(|decl| decl.port == endpoint.port)
                        .map(|decl| decl.direction);
                    match actual {
                        None => Err(PlanValidationError::UnknownBoundaryPort {
                            port: endpoint.port.clone(),
                        }),
                        Some(declared) if declared != expected => {
                            Err(PlanValidationError::WrongPortDirection {
                                node: None,
                                port: endpoint.port.clone(),
                                expected,
                            })
                        }
                        Some(_) => Ok(()),
                    }
                }
            }
        };
    for edge in &plan.edges {
        endpoint_ok(&edge.from, node::PortDirection::Output)?;
        if let Some(to) = &edge.to {
            endpoint_ok(to, node::PortDirection::Input)?;
        }
    }
    // Graph boundaries are compile-time composition seams. A cycle may cross
    // one seam without appearing in either source Graph's local edge list, so
    // the Registry-free plan boundary proves the invariant again on the
    // flattened executable Nodes. Iterative three-state DFS keeps the check
    // bounded by the already-validated plan size and avoids recursion.
    let mut successors = vec![Vec::new(); plan.nodes.len()];
    for edge in &plan.edges {
        if let (Some(from), Some(to)) = (edge.from.node, edge.to.as_ref().and_then(|to| to.node)) {
            if !plan.nodes[to.index()].input_drives_outputs() {
                // An inbound Call or duplex Session node has two boundary
                // halves, not an implicit `in -> out` execution arc. A
                // response/message terminates at `in`; a later world message
                // is a fresh external root at `out`.
                // Treating the node as one opaque vertex would report a
                // spatial cycle where the executable dependency graph has
                // none.
                continue;
            }
            successors[from.index()].push(to.index());
        }
    }
    let mut state = vec![0u8; plan.nodes.len()];
    for start in 0..plan.nodes.len() {
        if state[start] != 0 {
            continue;
        }
        let mut stack = vec![(start, 0usize)];
        state[start] = 1;
        while let Some((current, next_child)) = stack.pop() {
            if let Some(&child) = successors[current].get(next_child) {
                stack.push((current, next_child + 1));
                match state[child] {
                    0 => {
                        state[child] = 1;
                        stack.push((child, 0));
                    }
                    1 => {
                        return Err(PlanValidationError::WiringCycle {
                            node: plan.nodes[child].path.clone(),
                        });
                    }
                    _ => {}
                }
            } else {
                state[current] = 2;
            }
        }
    }
    // Two PINNED edge ends must agree — the same rule the checker
    // holds the surface to; endpoint slots are proven above, so the
    // lookups dereference safely.
    let pin_of = |endpoint: &EdgeEnd| -> Option<DataType> {
        match endpoint.node {
            Some(slot) => {
                let node = &plan.nodes[slot.index()];
                node.ports
                    .iter()
                    .find(|decl| decl.port == endpoint.port)
                    .and_then(|decl| decl.ty.clone())
            }
            None => plan
                .boundary
                .iter()
                .find(|decl| decl.port == endpoint.port)
                .and_then(|decl| decl.ty.clone()),
        }
    };
    for edge in &plan.edges {
        if let Some(to) = &edge.to
            && let Some(found) = pin_of(&edge.from).and_then(|ty| match plan.edge_cases(edge) {
                None => Some(ty),
                Some(cases) => super::routing::selected_case_type(&ty, &cases),
            })
            && let Some(expected) = pin_of(to)
            && !found.is_assignable_to(&expected)
        {
            return Err(PlanValidationError::MismatchedEdgePins {
                port: to.port.clone(),
                expected,
                found,
            });
        }
    }

    // The edge order renders through `edge_text`, which dereferences
    // slots — bounds first, order after.
    if !plan
        .edges
        .windows(2)
        .all(|w| plan.edge_text(&w[0]) < plan.edge_text(&w[1]))
    {
        return Err(PlanValidationError::NonCanonicalOrder {
            collection: "edges",
        });
    }
    // Decisions belong inside a Dispatch; no activation can select two handlers.
    let mut node_activations = BTreeSet::new();
    for handler in &plan.handlers {
        if !node_activations.insert((handler.owner, handler.activation.canonical_key())) {
            return Err(PlanValidationError::DuplicateHandler {
                node: node_at(handler.owner)?.path.clone(),
                activation: handler.activation.clone(),
            });
        }
    }

    let mut activated_inputs = BTreeSet::new();
    for handler in &plan.handlers {
        let handler_node = handler.owner;
        let node = node_at(handler_node)?;
        if node.connector().is_some() {
            return Err(PlanValidationError::NonOperatorHandler {
                node: node.path.clone(),
            });
        }
        match &handler.activation {
            node::HandlerActivation::Port(port) => {
                if !has_port(node, port, node::PortDirection::Input) {
                    return Err(PlanValidationError::WrongPortDirection {
                        node: Some(node.path.clone()),
                        port: port.clone(),
                        expected: node::PortDirection::Input,
                    });
                }
                if !activated_inputs.insert((handler_node, port.clone())) {
                    return Err(PlanValidationError::AmbiguousInputActivation {
                        node: node.path.clone(),
                        port: port.clone(),
                    });
                }
            }
            node::HandlerActivation::Join {
                ports,
                within_ns,
                last,
            } => {
                let distinct: BTreeSet<_> = ports.iter().cloned().collect();
                if ports.len() < 2 || distinct.len() != ports.len() {
                    return Err(PlanValidationError::InvalidJoinActivation {
                        node: node.path.clone(),
                    });
                }
                if within_ns.is_some_and(|duration| duration <= 0)
                    || last.is_some_and(|count| count == 0)
                {
                    return Err(PlanValidationError::InvalidJoinBound {
                        node: node.path.clone(),
                    });
                }
                for port in ports {
                    if !has_port(node, port, node::PortDirection::Input) {
                        return Err(PlanValidationError::WrongPortDirection {
                            node: Some(node.path.clone()),
                            port: port.clone(),
                            expected: node::PortDirection::Input,
                        });
                    }
                    if !activated_inputs.insert((handler_node, port.clone())) {
                        return Err(PlanValidationError::AmbiguousInputActivation {
                            node: node.path.clone(),
                            port: port.clone(),
                        });
                    }
                }
                if within_ns.is_some() || last.is_some() {
                    let expected = node::join_expired_type().map_err(|_| {
                        PlanValidationError::InvalidJoinActivation {
                            node: node.path.clone(),
                        }
                    })?;
                    let valid_expiry = node.ports.iter().any(|port| {
                        port.port.name() == "expired"
                            && port.direction == node::PortDirection::Output
                            && port.ty.as_ref() == Some(&expected)
                    });
                    if !valid_expiry {
                        return Err(PlanValidationError::MissingJoinExpiryPort {
                            node: node.path.clone(),
                        });
                    }
                }
            }
            node::HandlerActivation::Lifecycle(
                lifecycle @ (node::Lifecycle::Start | node::Lifecycle::Stop),
            ) => {
                if !node
                    .ports
                    .iter()
                    .any(|port| port.direction == node::PortDirection::Input)
                {
                    return Err(PlanValidationError::LifecycleWithoutInput {
                        node: node.path.clone(),
                        lifecycle: *lifecycle,
                    });
                }
            }
            node::HandlerActivation::Lifecycle(lifecycle) => {
                return Err(PlanValidationError::UnsupportedLifecycleHandler {
                    node: node.path.clone(),
                    lifecycle: *lifecycle,
                });
            }
        }
        if let Some(guard) = &handler.guard {
            expr_canonical(&guard.root)?;
        }
        for expression in handler
            .dispatch
            .predicates()
            .chain(handler.dispatch.steps().flat_map(DispatchStep::expressions))
        {
            expr_canonical(&expression.root)?;
            asset_indexes_bound(plan, &expression.root)?;
        }
        for step in handler.dispatch.steps() {
            match step {
                DispatchStep::Emit { .. } | DispatchStep::MultiEmit { .. } => {
                    for (port, value) in step.emissions() {
                        if !has_port(node, port, node::PortDirection::Output) {
                            return Err(PlanValidationError::WrongPortDirection {
                                node: Some(node.path.clone()),
                                port: port.clone(),
                                expected: node::PortDirection::Output,
                            });
                        }
                        match value {
                            EmitValue::Data(tree) => {
                                expr_canonical(&tree.root)?;
                                asset_indexes_bound(plan, &tree.root)?;
                            }
                            EmitValue::Patch(writes) | EmitValue::FreshData(writes) => {
                                for write in writes {
                                    expr_canonical(&write.value.root)?;
                                    asset_indexes_bound(plan, &write.value.root)?;
                                    meta_target_known(plan, &write.target)?;
                                }
                            }
                            EmitValue::Identity | EmitValue::Nothing => {}
                        }
                        if matches!(value, EmitValue::Identity)
                            && matches!(handler.activation, node::HandlerActivation::Join { .. })
                        {
                            return Err(PlanValidationError::AmbiguousJoinIdentity {
                                node: node.path.clone(),
                            });
                        }
                    }
                }
                DispatchStep::SetVar { name, value: expr } => {
                    if !node.vars.iter().any(|var| var.name == *name) {
                        return Err(PlanValidationError::UnknownVariable {
                            node: node.path.clone(),
                            name: name.clone(),
                        });
                    }
                    expr_canonical(&expr.root)?;
                }
                DispatchStep::Let { value: expr, .. } => expr_canonical(&expr.root)?,
                DispatchStep::LoopCommand(_) => {
                    for expression in step.expressions() {
                        expr_canonical(&expression.root)?;
                        asset_indexes_bound(plan, &expression.root)?;
                    }
                }
                DispatchStep::Stop => {
                    if handler.activation
                        == node::HandlerActivation::Lifecycle(node::Lifecycle::Stop)
                    {
                        return Err(PlanValidationError::RecursiveStop {
                            node: node.path.clone(),
                        });
                    }
                }
            }
        }
    }
    // Each owner/activation is unique; declaration order is not semantic.
    for pair in plan.handlers.windows(2) {
        let order = pair[0].owner.cmp(&pair[1].owner).then_with(|| {
            pair[0]
                .activation
                .canonical_key()
                .cmp(&pair[1].activation.canonical_key())
        });
        if order != core::cmp::Ordering::Less {
            return Err(PlanValidationError::NonCanonicalOrder {
                collection: "handlers",
            });
        }
    }
    loop_control_holds(plan)?;
    join_metadata_presence_holds(plan)?;
    Ok(())
}

fn loop_control_holds(plan: &ExecutionPlan) -> Result<(), PlanValidationError> {
    for owner in &plan.owners {
        if owner.loop_spec().is_none() {
            continue;
        }
        let owner_name = if owner.path.is_root() {
            String::from("<root>")
        } else {
            owner.path.to_string()
        };
        match owner.kind {
            OwnerKind::Node(slot) => operator_loop_control(plan, slot, &owner_name)?,
            OwnerKind::Graph => {
                super::graph_loop::validate(plan, owner)?;
            }
        }
    }
    Ok(())
}

fn operator_loop_control(
    plan: &ExecutionPlan,
    slot: NodeIndex,
    owner_name: &str,
) -> Result<(), PlanValidationError> {
    let node = &plan.nodes[slot.index()];
    let Some(spec) = plan.owners[node.owner.index()].loop_spec() else {
        return Ok(());
    };
    let refusal = |path: String, detail| PlanValidationError::InvalidLoopControl {
        owner: String::from(owner_name),
        path,
        detail,
    };
    if !matches!(node.execution, NodeExecution::Operator) {
        // Call control is independently proved with its resolved routing and
        // Contract frame/terminal guarantees in call_loop::validate.
        return Ok(());
    }
    let activation = node::HandlerActivation::Port(spec.entry.clone());
    let Some(handler) = plan
        .handlers
        .iter()
        .find(|handler| handler.owner == slot && handler.activation == activation)
    else {
        return Err(refusal(
            String::from("entry"),
            "the Loop entry requires one application handler",
        ));
    };
    for other in plan
        .handlers
        .iter()
        .filter(|handler| handler.owner == slot && handler.activation != activation)
    {
        if other
            .dispatch
            .steps()
            .any(|step| matches!(step, DispatchStep::LoopCommand(_)))
        {
            return Err(refusal(
                other.activation.canonical_key(),
                "only an entry application can issue a Loop command",
            ));
        }
    }
    for (index, decision) in handler.dispatch.decisions().enumerate() {
        let super::Dispatch::Linear(steps) = decision else {
            continue;
        };
        let path = format!("{}.leaf[{index}]", spec.entry);
        let commands = steps
            .iter()
            .filter(|step| matches!(step, DispatchStep::LoopCommand(_)))
            .count();
        let terminal = steps
            .iter()
            .flat_map(DispatchStep::emissions)
            .any(|(port, value)| {
                spec.terminals.contains(port) && !matches!(value, EmitValue::Nothing)
            });
        let stopped = steps.iter().any(|step| matches!(step, DispatchStep::Stop));
        if commands + usize::from(terminal) + usize::from(stopped) != 1 {
            return Err(refusal(
                path,
                "each Loop application must select exactly one of Next, Again, Done, or stop",
            ));
        }
        for step in steps {
            if let DispatchStep::LoopCommand(super::LoopCommand::Next(value)) = step {
                let entry = node
                    .ports
                    .iter()
                    .find(|port| port.port == spec.entry)
                    .and_then(|port| port.ty.as_ref());
                let valid = match entry {
                    Some(entry) => {
                        super::type_resolution::resolved_emission_type(value, entry, entry)?
                            .is_some_and(|produced| produced.is_assignable_to(entry))
                    }
                    None => false,
                };
                if !valid {
                    return Err(refusal(
                        path.clone(),
                        "Loop.next must produce a value assignable to the entry type",
                    ));
                }
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum WritableMetaField {
    App(String),
    Ext(String),
}

impl WritableMetaField {
    fn path(&self) -> String {
        match self {
            Self::App(field) => format!("meta.app.{field}"),
            Self::Ext(field) => format!("meta.ext.{field}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum WritableMetaRead {
    Field(WritableMetaField),
    AppZone,
    ExtZone,
    All,
}

type MetaPresence = BTreeSet<WritableMetaField>;
type EndpointPresence = BTreeMap<(Option<NodeIndex>, node::Port), MetaPresence>;

/// Proves the static half of CommonOrAbsent. A join is allowed to discard a
/// conflicting field silently; only a later required read is illegal. The
/// analysis is a monotone flow over the flattened DAG and is re-derived from
/// plan facts, so a forged package cannot turn the runtime's absent-field
/// defense into ordinary execution.
fn join_metadata_presence_holds(plan: &ExecutionPlan) -> Result<(), PlanValidationError> {
    let mut all_fields = MetaPresence::new();
    for (field, _) in &plan.app_schema {
        all_fields.insert(WritableMetaField::App(field.clone()));
    }
    for (field, _) in &plan.ext_schema {
        all_fields.insert(WritableMetaField::Ext(field.clone()));
    }
    all_fields.insert(WritableMetaField::Ext(String::from("content_type")));
    if all_fields.is_empty() {
        return Ok(());
    }

    let mut inputs = EndpointPresence::new();
    let mut outputs = EndpointPresence::new();
    loop {
        let mut changed = false;
        for edge in &plan.edges {
            let Some(to) = &edge.to else { continue };
            let propagated = outputs
                .get(&(edge.from.node, edge.from.port.clone()))
                .cloned()
                .unwrap_or_default();
            changed |= extend_presence(
                inputs.entry((to.node, to.port.clone())).or_default(),
                &propagated,
            );
        }

        for handler in &plan.handlers {
            let handler_node = handler.owner;
            let base = activation_presence(handler, &inputs, &all_fields);
            for step in handler.dispatch.steps() {
                let DispatchStep::Emit { port, value } = step else {
                    continue;
                };
                if matches!(value, EmitValue::Nothing) {
                    continue;
                }
                let mut emitted = base.clone();
                for field in emission_meta_writes(value) {
                    emitted.remove(&field);
                }
                changed |= extend_presence(
                    outputs
                        .entry((Some(handler_node), port.clone()))
                        .or_default(),
                    &emitted,
                );
            }

            // Join eviction carries one retained candidate, not the merged
            // join context. It therefore propagates the union of the input
            // risks without manufacturing a fresh ambiguity.
            if matches!(
                handler.activation,
                node::HandlerActivation::Join {
                    within_ns: Some(_),
                    ..
                } | node::HandlerActivation::Join { last: Some(_), .. }
            ) {
                let inherited = node_input_presence(plan, handler_node, &inputs);
                let expired = node::Port::new("expired").map_err(|_| {
                    PlanValidationError::InvalidJoinActivation {
                        node: plan.nodes[handler_node.index()].path.clone(),
                    }
                })?;
                changed |= extend_presence(
                    outputs.entry((Some(handler_node), expired)).or_default(),
                    &inherited,
                );
            }
        }

        for (slot_index, planned) in plan.nodes.iter().enumerate() {
            if !matches!(planned.execution, NodeExecution::Connector(_)) {
                continue;
            }
            let slot = NodeIndex::new(slot_index as u32);
            let inherited = node_input_presence(plan, slot, &inputs);
            for port in planned
                .ports
                .iter()
                .filter(|port| port.direction == node::PortDirection::Output)
            {
                changed |= extend_presence(
                    outputs.entry((Some(slot), port.port.clone())).or_default(),
                    &inherited,
                );
            }
        }

        if !changed {
            break;
        }
    }

    for handler in &plan.handlers {
        let handler_node = handler.owner;
        let base = activation_presence(handler, &inputs, &all_fields);
        let owner = &plan.nodes[handler_node.index()].path.to_string();
        if let Some(guard) = &handler.guard {
            required_meta_reads_hold(&guard.root, &base, owner)?;
        }
        for predicate in handler.dispatch.predicates() {
            required_meta_reads_hold(&predicate.root, &base, owner)?;
        }
        for step in handler.dispatch.steps() {
            for expression in step.expressions() {
                required_meta_reads_hold(&expression.root, &base, owner)?;
            }
        }
    }

    for (slot_index, planned) in plan.nodes.iter().enumerate() {
        let slot = NodeIndex::new(slot_index as u32);
        let base = node_input_presence(plan, slot, &inputs);
        if base.is_empty() {
            continue;
        }
        let owner = planned.path.to_string();
        if let NodeExecution::Connector(spec) = &planned.execution {
            for (_, setting) in &spec.attributes {
                if let crate::plan::AttrSetting::Expr { expr, .. } = setting {
                    required_meta_reads_hold(&expr.root, &base, &owner)?;
                }
            }
        }
        let lexical_owner = &plan.owners[planned.owner.index()];
        if let Some(key) = &lexical_owner.instance.key {
            required_meta_reads_hold(&key.root, &base, &owner)?;
        }
        for value in &lexical_owner.vals {
            required_meta_reads_hold(&value.expr.root, &base, &owner)?;
        }
    }

    Ok(())
}

fn extend_presence(target: &mut MetaPresence, source: &MetaPresence) -> bool {
    let before = target.len();
    target.extend(source.iter().cloned());
    target.len() != before
}

fn node_input_presence(
    plan: &ExecutionPlan,
    slot: NodeIndex,
    inputs: &EndpointPresence,
) -> MetaPresence {
    let mut presence = MetaPresence::new();
    for port in &plan.nodes[slot.index()].ports {
        if port.direction == node::PortDirection::Input
            && let Some(fields) = inputs.get(&(Some(slot), port.port.clone()))
        {
            presence.extend(fields.iter().cloned());
        }
    }
    presence
}

fn activation_presence(
    handler: &crate::plan::Handler,
    inputs: &EndpointPresence,
    all_fields: &MetaPresence,
) -> MetaPresence {
    let handler_node = handler.owner;
    match &handler.activation {
        node::HandlerActivation::Port(port) => inputs
            .get(&(Some(handler_node), port.clone()))
            .cloned()
            .unwrap_or_default(),
        node::HandlerActivation::Join { .. } => all_fields.clone(),
        node::HandlerActivation::Lifecycle(_) => {
            let mut presence = MetaPresence::new();
            for ((slot, _), fields) in inputs {
                if *slot == Some(handler_node) {
                    presence.extend(fields.iter().cloned());
                }
            }
            presence
        }
    }
}

fn emission_meta_writes(emission: &EmitValue) -> MetaPresence {
    match emission {
        EmitValue::Patch(writes) | EmitValue::FreshData(writes) => {
            let mut presence = MetaPresence::new();
            for write in writes {
                match &write.target {
                    crate::envelope::WriteTarget::App(field) => {
                        presence.insert(WritableMetaField::App(field.clone()));
                    }
                    crate::envelope::WriteTarget::Ext(field) => {
                        presence.insert(WritableMetaField::Ext(field.clone()));
                    }
                    crate::envelope::WriteTarget::Data(_) => {}
                }
            }
            presence
        }
        EmitValue::Identity | EmitValue::Data(_) | EmitValue::Nothing => MetaPresence::new(),
    }
}

fn required_meta_reads_hold(
    expression: &expr::Expr,
    ambiguous: &MetaPresence,
    owner: &str,
) -> Result<(), PlanValidationError> {
    let mut reads = BTreeSet::new();
    collect_writable_meta_reads(expression, &mut reads);
    let offending = reads.into_iter().find_map(|read| match read {
        WritableMetaRead::Field(field) if ambiguous.contains(&field) => Some(field),
        WritableMetaRead::AppZone => ambiguous
            .iter()
            .find(|field| matches!(field, WritableMetaField::App(_)))
            .cloned(),
        WritableMetaRead::ExtZone => ambiguous
            .iter()
            .find(|field| matches!(field, WritableMetaField::Ext(_)))
            .cloned(),
        WritableMetaRead::All => ambiguous.iter().next().cloned(),
        WritableMetaRead::Field(_) => None,
    });
    if let Some(field) = offending {
        Err(PlanValidationError::AmbiguousJoinMetadataRead {
            owner: owner.to_string(),
            field: field.path(),
        })
    } else {
        Ok(())
    }
}

fn collect_writable_meta_reads(expression: &expr::Expr, reads: &mut BTreeSet<WritableMetaRead>) {
    let inspect = |child, reads: &mut BTreeSet<WritableMetaRead>| {
        collect_writable_meta_reads(child, reads);
    };
    match expression {
        expr::Expr::Literal(_) | expr::Expr::Asset(_) => {}
        expr::Expr::Path(segments) => match segments.as_slice() {
            [meta] if meta == "meta" => {
                reads.insert(WritableMetaRead::All);
            }
            [meta, zone] if meta == "meta" && zone == "app" => {
                reads.insert(WritableMetaRead::AppZone);
            }
            [meta, zone] if meta == "meta" && zone == "ext" => {
                reads.insert(WritableMetaRead::ExtZone);
            }
            [meta, zone, field @ ..] if meta == "meta" && zone == "app" => {
                reads.insert(WritableMetaRead::Field(WritableMetaField::App(
                    field.join("."),
                )));
            }
            [meta, zone, field @ ..] if meta == "meta" && zone == "ext" => {
                reads.insert(WritableMetaRead::Field(WritableMetaField::Ext(
                    field.join("."),
                )));
            }
            _ => {}
        },
        expr::Expr::Unary { expr, .. }
        | expr::Expr::Is { expr, .. }
        | expr::Expr::As { expr, .. } => inspect(expr, reads),
        expr::Expr::Binary { left, right, .. } => {
            inspect(left, reads);
            inspect(right, reads);
        }
        expr::Expr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            inspect(cond, reads);
            inspect(then, reads);
            inspect(otherwise, reads);
        }
        expr::Expr::Index { base, index } => {
            inspect(base, reads);
            inspect(index, reads);
        }
        expr::Expr::Call { args, .. } | expr::Expr::List(args) => {
            for argument in args {
                inspect(argument, reads);
            }
        }
        expr::Expr::MapLit(fields)
        | expr::Expr::Struct(fields)
        | expr::Expr::Tagged { fields, .. } => {
            for (_, value) in fields {
                inspect(value, reads);
            }
        }
    }
}

/// The complete materialized-value gate. Runtime-only laziness witnesses are
/// not core [`Value`](crate::value::Value)s and remain unrepresentable by
/// construction.
fn value_expressible(value: &crate::value::Value) -> Result<(), PlanValidationError> {
    value
        .check_consistency()
        .map_err(|_| PlanValidationError::UnrepresentableConstant)
}

/// Every map literal in a shipped expression tree is construction-sorted —
/// the parser sorts them, so an unsorted or duplicated literal is a
/// second byte form of the same expression, which the re-encoding
/// check cannot catch (it faithfully reproduces whatever order the
/// tree carries). Recursion is safe here: the nesting budget has
/// already bounded the tree.
pub(super) fn expr_canonical(expr: &expr::Expr) -> Result<(), PlanValidationError> {
    match expr {
        expr::Expr::Literal(value) => value_expressible(value),
        // The index is bounds-checked against the asset table in
        // the walk that owns the plan (`meta_target_known`'s
        // sibling); canonical here.
        expr::Expr::Asset(_) => Ok(()),
        expr::Expr::Path(segments) if !segments.is_empty() => Ok(()),
        expr::Expr::Path(_) => Err(PlanValidationError::UnrepresentableConstant),
        expr::Expr::Unary { expr, .. } => expr_canonical(expr),
        expr::Expr::Binary { left, right, .. } => {
            expr_canonical(left)?;
            expr_canonical(right)
        }
        expr::Expr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            expr_canonical(cond)?;
            expr_canonical(then)?;
            expr_canonical(otherwise)
        }
        expr::Expr::Index { base, index } => {
            expr_canonical(base)?;
            expr_canonical(index)
        }
        expr::Expr::Call {
            function: expr::Function::Asset,
            ..
        } => Err(PlanValidationError::UnboundAssetCall),
        expr::Expr::Call { args, .. } => args.iter().try_for_each(expr_canonical),
        expr::Expr::List(items) => items.iter().try_for_each(expr_canonical),
        expr::Expr::MapLit(pairs) => {
            let mut keys = alloc::collections::BTreeSet::new();
            for (key, _) in pairs {
                if !keys.insert(key.as_str()) {
                    return Err(PlanValidationError::DuplicateMapKey { key: key.clone() });
                }
            }
            for pair in pairs.windows(2) {
                if pair[0].0 >= pair[1].0 {
                    return Err(PlanValidationError::NonCanonicalOrder {
                        collection: "map literal",
                    });
                }
            }
            pairs
                .iter()
                .map(|(_, value)| value)
                .try_for_each(expr_canonical)
        }
        expr::Expr::Struct(fields) => expression_fields_canonical(fields, "struct constructor"),
        expr::Expr::Tagged {
            union,
            case,
            fields,
        } => {
            DataType::TaggedUnion((**union).clone())
                .check_bounds()
                .map_err(|_| PlanValidationError::UnrepresentableConstant)?;
            let Some(declared) = union
                .cases()
                .iter()
                .find(|declared| declared.name() == case)
            else {
                return Err(PlanValidationError::UnrepresentableConstant);
            };
            if declared.payload().fields().len() != fields.len()
                || !declared
                    .payload()
                    .fields()
                    .iter()
                    .zip(fields)
                    .all(|(expected, (found, _))| expected.name() == found)
            {
                return Err(PlanValidationError::UnrepresentableConstant);
            }
            expression_fields_canonical(fields, "tagged-union constructor")
        }
        expr::Expr::Is { expr, target } | expr::Expr::As { expr, target } => {
            narrow_target_canonical(target)?;
            expr_canonical(expr)
        }
    }
}

/// Proves the pre-commit routing boundary independently of the source
/// checker. A key sees the entering envelope and immutable values of lexical
/// ancestors, never its own not-yet-created instance or destination facts.
fn instance_bindings_available(
    expr: &expr::Expr,
    owner: &str,
    enclosing: &BTreeSet<&str>,
) -> Result<(), PlanValidationError> {
    let inspect = |child| instance_bindings_available(child, owner, enclosing);
    match expr {
        expr::Expr::Literal(_) | expr::Expr::Asset(_) => Ok(()),
        expr::Expr::Path(segments) => {
            let available = matches!(segments.as_slice(), [root, ..] if root == "data")
                || matches!(segments.as_slice(), [meta, zone, _, ..]
                    if meta == "meta" && matches!(zone.as_str(), "sys" | "app" | "ext"))
                || segments
                    .first()
                    .is_some_and(|root| enclosing.contains(root.as_str()));
            if available {
                Ok(())
            } else {
                Err(PlanValidationError::InvalidInstanceBinding {
                    owner: owner.to_string(),
                    path: segments.join("."),
                })
            }
        }
        expr::Expr::Unary { expr, .. }
        | expr::Expr::Is { expr, .. }
        | expr::Expr::As { expr, .. } => inspect(expr),
        expr::Expr::Binary { left, right, .. } => {
            inspect(left)?;
            inspect(right)
        }
        expr::Expr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            inspect(cond)?;
            inspect(then)?;
            inspect(otherwise)
        }
        expr::Expr::Index { base, index } => {
            inspect(base)?;
            inspect(index)
        }
        expr::Expr::Call { args, .. } | expr::Expr::List(args) => args.iter().try_for_each(inspect),
        expr::Expr::MapLit(fields)
        | expr::Expr::Struct(fields)
        | expr::Expr::Tagged { fields, .. } => {
            fields.iter().map(|(_, value)| value).try_for_each(inspect)
        }
    }
}

fn expression_fields_canonical(
    fields: &[(String, expr::Expr)],
    collection: &'static str,
) -> Result<(), PlanValidationError> {
    for pair in fields.windows(2) {
        if pair[0].0 >= pair[1].0 {
            return Err(PlanValidationError::NonCanonicalOrder { collection });
        }
    }
    fields
        .iter()
        .map(|(_, value)| value)
        .try_for_each(expr_canonical)
}

fn narrow_target_canonical(target: &expr::NarrowTarget) -> Result<(), PlanValidationError> {
    let ty = match target {
        expr::NarrowTarget::Type(target) => target.ty(),
        expr::NarrowTarget::Case { name, payload } => {
            if name.is_empty() || !matches!(payload.as_ref(), DataType::Struct(_)) {
                return Err(PlanValidationError::UnrepresentableConstant);
            }
            payload
        }
    };
    ty.check_bounds()
        .map_err(|_| PlanValidationError::UnrepresentableConstant)
}

#[cfg(test)]
mod value_format_tests {
    use super::*;
    use crate::value::Value;

    #[test]
    fn v7_admits_every_well_formed_value_family() {
        let values = alloc::vec![
            Value::Date(0),
            Value::Time(0),
            Value::TimestampNtz(0),
            Value::Interval {
                months: 0,
                days: 0,
                nanoseconds: 0,
            },
            Value::Uuid([0; 16]),
            Value::KeyedMap(alloc::vec![(Value::Int(1), Value::Bool(true))]),
        ];
        for value in values {
            assert_eq!(value_expressible(&value), Ok(()), "v7 refused {value:?}");
        }
    }
}
