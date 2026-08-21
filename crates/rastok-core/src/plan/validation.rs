//! Structural validation of an [`ExecutionPlan`] — every invariant
//! the machine dereferences without checking and every canonical
//! order the plan documents, held once, at a boundary, by a type.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use crate::datatype::{DataType, TypeError};
use crate::expr;
use crate::node;
use crate::plan::{
    Emission, EndpointRef, EvaluationSite, ExecutionPlan, InstanceLifetime, Node, NodeKind,
    OwnerKind, OwnerSlot, Slot, Step,
};

/// Why a plan is not executable — one named invariant violation. The
/// variants are semantic, not prose: a caller can match on the class
/// of defect; `Display` renders the human diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanValidationError {
    /// The categorical owner table is absent or has no root App Graph.
    MissingRootOwner,
    /// An owner reference indexes past the owner table.
    OwnerOutOfBounds {
        /// The impossible reference.
        owner: OwnerSlot,
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
        owner: OwnerSlot,
    },
    /// A Node and its owner row do not point to one another.
    OwnerNodeMismatch {
        /// The Node slot.
        node: Slot,
        /// The claimed owner row.
        owner: OwnerSlot,
    },
    /// An edge's lexical scope is absent or is not a Graph owner.
    InvalidEdgeScope {
        /// The claimed Graph owner.
        owner: OwnerSlot,
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
        slot: Slot,
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
    /// A connector some edge DELIVERS TO declares `Effect::None` —
    /// a delivery opens a call, and a call needs an effect class.
    /// (A source connector with no incoming edge may honestly be
    /// effectless.)
    EffectlessCallee {
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
    /// A lifecycle activation belongs to a later executable slice. Format v9
    /// can represent the closed registry without silently enabling it.
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
    /// An input carries an output-only root-sink policy.
    InputHasRootSink {
        /// The declaring node.
        node: node::Path,
        /// The invalid input.
        port: node::Port,
    },
    /// Boundary ports cannot carry node-output routing policy. A boundary
    /// output is already the graph's externally observed endpoint.
    BoundaryHasRootSink {
        /// The invalid boundary port.
        port: node::Port,
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
        expected: node::Direction,
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
    /// this machine's Partner seam does not carry the port yet —
    /// `Intent`, the response protocol, and the outcome trigger all
    /// speak `in`/`out`/`err`. A plan wiring such a port is valid in
    /// shape and NOT executable faithfully, so it refuses here; the
    /// restriction lifts when the port enters `Intent`.
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
        /// The Process owner proving that cancellation may be required.
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
    NestingBudgetExceeded {
        /// The deepest level the body would reach (root = 0).
        depth: usize,
        /// The reader's budget ([`crate::canonical_cbor::MAX_NESTING`]).
        budget: usize,
    },
}

impl core::fmt::Display for PlanValidationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
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
            Self::EffectlessCallee { node } => {
                write!(
                    f,
                    "invalid plan: `{node}` receives deliveries but declares no effect class — a call cannot open without one"
                )
            }
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
            Self::InputHasRootSink { node, port } => write!(
                f,
                "invalid plan: input `{node}.{port}` carries an output root-sink policy"
            ),
            Self::BoundaryHasRootSink { port } => write!(
                f,
                "invalid plan: boundary port `{port}` carries node-output root-sink policy"
            ),
            Self::DuplicateMapKey { key } => {
                write!(f, "invalid plan: duplicate map literal key `{key}`")
            }
            Self::MissingContractPort { node, port } => write!(
                f,
                "invalid plan: `{node}` omits resolved contract port `{port}`"
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
                "invalid plan: finite owner `{owner}` contains Process `{connector}` but declares no bounded Closing policy"
            ),
            Self::RecursiveStop { node } => write!(
                f,
                "invalid plan: `on stop` for `{node}` cannot request closure again"
            ),
            Self::NonCanonicalOrder { collection } => {
                write!(f, "invalid plan: `{collection}` is not in canonical order")
            }
            Self::NestingBudgetExceeded { depth, budget } => write!(
                f,
                "invalid plan: the body nests {depth} levels deep; the format reads at most {budget}"
            ),
        }
    }
}

impl core::error::Error for PlanValidationError {}

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
        let resolved_ports = super::type_resolution::resolve(&plan)?;
        verify_instance_key_types(&mut plan, &resolved_ports, key_mode)?;
        let depth = super::serialization::deepest_level_with_resolved(&plan, &resolved_ports);
        if depth >= crate::canonical_cbor::MAX_NESTING {
            return Err(PlanValidationError::NestingBudgetExceeded {
                depth,
                budget: crate::canonical_cbor::MAX_NESTING,
            });
        }
        Ok(Self {
            plan,
            resolved_ports,
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
    /// variable and no recovery hole. Format v9 carries it and the decoder
    /// derives it again from the raw constraints before requiring equality.
    #[must_use]
    pub const fn resolved_ports(&self) -> &super::type_resolution::ResolvedPortTable {
        &self.resolved_ports
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
                    .filter(|port| port.direction() == node::Direction::Input)
                    .map(|port| port.ty().clone())
                    .collect(),
                OwnerKind::Graph if index == 0 => ports
                    .boundary()
                    .iter()
                    .filter(|port| port.direction() == node::Direction::Input)
                    .map(|port| port.ty().clone())
                    .collect(),
                // Nested Graph admission types are closed by the compiler from
                // the scoped crossing routes before transport. The owner tree
                // preserves the boundary even though the spatial DAG is flat.
                OwnerKind::Graph => {
                    let owner = OwnerSlot::new(u32::try_from(index).map_err(|_| {
                        PlanValidationError::OwnerOutOfBounds {
                            owner: OwnerSlot::new(u32::MAX),
                            owner_count: plan.owners.len(),
                        }
                    })?);
                    plan.edges
                        .iter()
                        .filter_map(|edge| {
                            let target = edge.to.as_ref()?;
                            let target_owner = plan.nodes.get(target.node?.index())?.owner;
                            (owner_contains(plan, owner, target_owner)
                                && edge.scope != owner
                                && owner_contains(plan, edge.scope, owner))
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
        .with("meta.sys.role", DataType::Text)
        .with("meta.sys.graph_version", DataType::INT)
        .with("meta.ext.content_type", DataType::Text);
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
/// table refuses instead of minting a second byte form of the same
/// graph.
fn kind_rank(kind: &NodeKind) -> u8 {
    match kind {
        NodeKind::Operator => 0,
        NodeKind::Connector(_) => 1,
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
            // The standard field is IMPLICIT — known to every
            // consumer, never serialized (D-2026-08-10-4).
            if field == "content_type" || plan.ext_schema.iter().any(|(known, _)| known == field) {
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
/// `content_type` pinned to text.
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
        // In EXT the standard field is implicit and reserved — a
        // serialized entry would create a second authority, and a
        // BRANCH under it would shadow the implicit leaf (a prefix
        // collision with the implicit leaf, named as such). In APP
        // it is an ordinary application field (D-2026-08-10-4).
        if zone == "ext" && field == "content_type" {
            return Err(PlanValidationError::UnknownMetaField {
                zone,
                field: field.clone(),
            });
        }
        if zone == "ext" && field.starts_with("content_type.") {
            return Err(PlanValidationError::MetaPrefixCollision {
                leaf: alloc::string::String::from("content_type"),
                branch: field.clone(),
            });
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
    ty.validate()
        .map_err(|reason| PlanValidationError::InvalidDataType { location, reason })
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

/// Every type-bearing plan position passes this audit before the infallible
/// v9 encoder can observe the plan.
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
        if let Some(key) = &owner.instance.key {
            tree_type_at(key, format!("instance key on `{}`", owner.path))?;
        }
        for value in &owner.vals {
            tree_type_at(
                &value.expr,
                format!("immutable value `{}.{}`", owner.path, value.name),
            )?;
        }
    }
    for node in &plan.nodes {
        if let NodeKind::Connector(spec) = &node.kind {
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
        for (step_index, step) in handler.body.iter().enumerate() {
            let location = |role: &str| format!("handler {handler_index} step {step_index} {role}");
            match step {
                Step::Let { expr, .. } => tree_type_at(expr, location("let expression"))?,
                Step::SetVar { expr, .. } => tree_type_at(expr, location("state expression"))?,
                Step::Stop => {}
                Step::Emit { value, .. } => match value {
                    Emission::Identity | Emission::Nothing => {}
                    Emission::Data(tree) => tree_type_at(tree, location("data emission"))?,
                    Emission::Patch(fields) | Emission::Replace(fields) => {
                        for (field_index, field) in fields.iter().enumerate() {
                            tree_type_at(
                                &field.expr,
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

        let local: BTreeMap<&str, usize> = owner
            .vals
            .iter()
            .enumerate()
            .map(|(index, value)| (value.name.as_str(), index))
            .collect();
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
            && let Some(NodeKind::Connector(spec)) =
                plan.nodes.get(slot.index()).map(|node| &node.kind)
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
        let slot = OwnerSlot::new(u32::try_from(index).unwrap_or(u32::MAX));
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
        }

        let duration = match owner.instance.lifetime {
            InstanceLifetime::Persistent | InstanceLifetime::SinglePass => None,
            InstanceLifetime::Idle(value)
            | InstanceLifetime::Fixed(value)
            | InstanceLifetime::Tumbling(value) => Some(value),
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
        if owner.instance.lifetime != InstanceLifetime::Persistent {
            for node in &plan.nodes {
                let Some(spec) = node.connector() else {
                    continue;
                };
                if spec.contract != "Process" || !owner_contains(plan, slot, node.owner) {
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
            .map(|index| Slot::new(u32::try_from(index).unwrap_or(u32::MAX)))
            .find(|slot| !owned_nodes.contains(slot))
            .unwrap_or_else(|| Slot::new(u32::MAX));
        let owner = plan
            .nodes
            .get(missing.index())
            .map_or(OwnerSlot::new(u32::MAX), |node| node.owner);
        return Err(PlanValidationError::OwnerNodeMismatch {
            node: missing,
            owner,
        });
    }
    let is_descendant = |mut owner: OwnerSlot, ancestor: OwnerSlot| {
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
            plan.owner(edge.scope).map(|owner| &owner.kind),
            Some(OwnerKind::Graph)
        ) {
            return Err(PlanValidationError::InvalidEdgeScope { owner: edge.scope });
        }
        let endpoint_is_scoped = |endpoint: &EndpointRef| {
            endpoint.node.is_none_or(|slot| {
                plan.nodes
                    .get(slot.index())
                    .is_some_and(|node| is_descendant(node.owner, edge.scope))
            })
        };
        if !endpoint_is_scoped(&edge.from)
            || edge.to.as_ref().is_some_and(|to| !endpoint_is_scoped(to))
        {
            return Err(PlanValidationError::InvalidEdgeScope { owner: edge.scope });
        }
    }
    owner_values_well_formed(plan)?;
    Ok(())
}

fn owner_contains(plan: &ExecutionPlan, ancestor: OwnerSlot, child: OwnerSlot) -> bool {
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
        if (kind_rank(&pair[0].kind), pair[0].path.to_string())
            >= (kind_rank(&pair[1].kind), pair[1].path.to_string())
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
        "in" => Some(node::Direction::Input),
        "out" | "err" => Some(node::Direction::Output),
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
    if let Some(decl) = plan
        .boundary
        .iter()
        .find(|decl| decl.unwired == crate::plan::UnwiredPolicy::RootSink)
    {
        return Err(PlanValidationError::BoundaryHasRootSink {
            port: decl.port.clone(),
        });
    }
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
        if let Some(decl) = node.ports.iter().find(|decl| {
            decl.direction == node::Direction::Input
                && decl.unwired == crate::plan::UnwiredPolicy::RootSink
        }) {
            return Err(PlanValidationError::InputHasRootSink {
                node: node.path.clone(),
                port: decl.port.clone(),
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
        if let NodeKind::Connector(spec) = &node.kind {
            for (name, setting) in &spec.attributes {
                match setting {
                    crate::plan::AttrSetting::Value(value) => value_expressible(value)?,
                    crate::plan::AttrSetting::Expr { site, expr } => {
                        let admitted =
                            crate::boundary::process_attribute_site(&spec.contract, name)
                                .is_some_and(|latest| {
                                    latest != EvaluationSite::Plan && latest.admits(*site)
                                });
                        if *site == EvaluationSite::Plan || !admitted {
                            return Err(PlanValidationError::BindingSiteMismatch {
                                owner: node.path.to_string(),
                                name: name.clone(),
                            });
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
            // The REAL effect invariant (D-2026-08-12-1, review
            // round): any connector a delivery can reach opens a
            // call, and the machine dispatches on its effect CLASS —
            // `Effect::None` there would be a machine fault, so it
            // never validates. A pure source (no incoming edge)
            // stays free to be effectless.
            if spec.effect == node::Effect::None {
                let slot = crate::plan::Slot::new(u32::try_from(node_index).unwrap_or(u32::MAX));
                let receives = plan
                    .edges
                    .iter()
                    .any(|edge| edge.to.as_ref().is_some_and(|to| to.node == Some(slot)));
                if receives {
                    return Err(PlanValidationError::EffectlessCallee {
                        node: node.path.clone(),
                    });
                }
            }
            if spec.contract == "Process" {
                let codec = match spec.attribute("codec") {
                    Some(crate::plan::AttrSetting::Word(word))
                        if matches!(word.as_str(), "bytes" | "jsonl" | "lines") =>
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
                        if matches!(word.as_str(), "bytes" | "lines") =>
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
                let expected = [
                    (
                        "in",
                        node::Direction::Input,
                        crate::plan::UnwiredPolicy::RequireWire,
                    ),
                    (
                        "out",
                        node::Direction::Output,
                        crate::plan::UnwiredPolicy::RequireWire,
                    ),
                    (
                        "err",
                        node::Direction::Output,
                        crate::plan::UnwiredPolicy::RootSink,
                    ),
                    (
                        "exit",
                        node::Direction::Output,
                        crate::plan::UnwiredPolicy::RootSink,
                    ),
                ];
                for (name, direction, unwired) in expected {
                    let Some(port) = node.ports.iter().find(|port| port.port.name() == name) else {
                        return Err(PlanValidationError::MissingContractPort {
                            node: node.path.clone(),
                            port: name,
                        });
                    };
                    let expected_ty = crate::boundary::process_port_type(
                        &spec.contract,
                        name,
                        Some(codec),
                        Some(err_codec),
                    );
                    if port.direction != direction
                        || port.unwired != unwired
                        || port.ty != expected_ty
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
                    .find(|port| !matches!(port.port.name(), "in" | "out" | "err" | "exit"))
                {
                    return Err(PlanValidationError::UnexpectedContractPort {
                        node: node.path.clone(),
                        port: port.port.clone(),
                    });
                }
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

    // The reader's nesting budget, enforced on the writing side: a
    // plan whose body would nest beyond what `decode` reads cannot
    // earn the type, so `encode ⇒ decode` is total. Measured without
    // recursion — nothing below (nor the codec, nor the hasher) may
    // recurse over the plan before this bound holds.
    let depth = super::serialization::deepest_level(plan);
    if depth >= crate::canonical_cbor::MAX_NESTING {
        return Err(PlanValidationError::NestingBudgetExceeded {
            depth,
            budget: crate::canonical_cbor::MAX_NESTING,
        });
    }

    // References: slots, ports in their roles, handlers, vars. Every port is
    // now a row; conventional source spelling confers no hidden existence.
    let has_port = |node: &Node, port: &node::Port, direction: node::Direction| {
        node.ports
            .iter()
            .any(|decl| &decl.port == port && decl.direction == direction)
    };
    let node_at = |slot: Slot| {
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
        |endpoint: &EndpointRef, position: node::Direction| -> Result<(), PlanValidationError> {
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
                    // Partner seam cannot execute it yet: refuse by
                    // capability, distinctly from ignorance.
                    if matches!(node.kind, NodeKind::Connector(_))
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
                        node::Direction::Output => node::Direction::Input,
                        node::Direction::Input => node::Direction::Output,
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
        endpoint_ok(&edge.from, node::Direction::Output)?;
        if let Some(to) = &edge.to {
            endpoint_ok(to, node::Direction::Input)?;
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
    let pin_of = |endpoint: &EndpointRef| -> Option<DataType> {
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
            && let Some(found) = pin_of(&edge.from)
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
    // Uniqueness precedes every local judgement: a repeated activation is
    // one trigger fired twice, whereas sharing one input between two
    // DIFFERENT activations is the distinct AmbiguousInputActivation defect.
    let handler_key =
        |handler: &crate::plan::Handler| (handler.node.index(), handler.activation.canonical_key());
    let mut activations = BTreeSet::new();
    for handler in &plan.handlers {
        if !activations.insert(handler_key(handler)) {
            return Err(PlanValidationError::DuplicateHandler {
                node: node_at(handler.node)?.path.clone(),
                activation: handler.activation.clone(),
            });
        }
    }

    let mut activated_inputs = BTreeSet::new();
    for handler in &plan.handlers {
        let node = node_at(handler.node)?;
        match &handler.activation {
            node::HandlerActivation::Port(port) => {
                if !has_port(node, port, node::Direction::Input) {
                    return Err(PlanValidationError::WrongPortDirection {
                        node: Some(node.path.clone()),
                        port: port.clone(),
                        expected: node::Direction::Input,
                    });
                }
                if !activated_inputs.insert((handler.node, port.clone())) {
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
                    if !has_port(node, port, node::Direction::Input) {
                        return Err(PlanValidationError::WrongPortDirection {
                            node: Some(node.path.clone()),
                            port: port.clone(),
                            expected: node::Direction::Input,
                        });
                    }
                    if !activated_inputs.insert((handler.node, port.clone())) {
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
                            && port.direction == node::Direction::Output
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
                    .any(|port| port.direction == node::Direction::Input)
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
        for step in &handler.body {
            match step {
                Step::Emit { port, value } => {
                    if !has_port(node, port, node::Direction::Output) {
                        return Err(PlanValidationError::WrongPortDirection {
                            node: Some(node.path.clone()),
                            port: port.clone(),
                            expected: node::Direction::Output,
                        });
                    }
                    match value {
                        Emission::Data(tree) => {
                            expr_canonical(&tree.root)?;
                            asset_indexes_bound(plan, &tree.root)?;
                        }
                        Emission::Patch(writes) | Emission::Replace(writes) => {
                            for write in writes {
                                expr_canonical(&write.expr.root)?;
                                asset_indexes_bound(plan, &write.expr.root)?;
                                meta_target_known(plan, &write.target)?;
                            }
                        }
                        Emission::Identity | Emission::Nothing => {}
                    }
                    if matches!(value, Emission::Identity)
                        && matches!(handler.activation, node::HandlerActivation::Join { .. })
                    {
                        return Err(PlanValidationError::AmbiguousJoinIdentity {
                            node: node.path.clone(),
                        });
                    }
                }
                Step::SetVar { name, expr } => {
                    if !node.vars.iter().any(|var| var.name == *name) {
                        return Err(PlanValidationError::UnknownVariable {
                            node: node.path.clone(),
                            name: name.clone(),
                        });
                    }
                    expr_canonical(&expr.root)?;
                }
                Step::Let { expr, .. } => expr_canonical(&expr.root)?,
                Step::Stop => {
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
    // Handler order is canonical — (slot, activation key), the same
    // key the lowering and the hash sort by, because declaration
    // order between handlers carries neither semantics nor identity.
    // Slot bounds and uniqueness are already proven above.
    for pair in plan.handlers.windows(2) {
        if handler_key(&pair[0]) >= handler_key(&pair[1]) {
            return Err(PlanValidationError::NonCanonicalOrder {
                collection: "handlers",
            });
        }
    }
    join_metadata_presence_holds(plan)?;
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
type EndpointPresence = BTreeMap<(Option<Slot>, node::Port), MetaPresence>;

/// Proves the static half of CommonOrAbsent. A join is allowed to discard a
/// conflicting field silently; only a later required read is illegal. The
/// analysis is a monotone flow over the flattened DAG and is re-derived from
/// plan facts, so a forged package cannot turn the runtime's absent-field
/// defense into ordinary execution.
fn join_metadata_presence_holds(plan: &ExecutionPlan) -> Result<(), PlanValidationError> {
    let all_fields: MetaPresence = plan
        .app_schema
        .iter()
        .map(|(field, _)| WritableMetaField::App(field.clone()))
        .chain(
            plan.ext_schema
                .iter()
                .map(|(field, _)| WritableMetaField::Ext(field.clone())),
        )
        .chain(core::iter::once(WritableMetaField::Ext(String::from(
            "content_type",
        ))))
        .collect();
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
            let base = activation_presence(handler, &inputs, &all_fields);
            for step in &handler.body {
                let Step::Emit { port, value } = step else {
                    continue;
                };
                if matches!(value, Emission::Nothing) {
                    continue;
                }
                let mut emitted = base.clone();
                for field in emission_meta_writes(value) {
                    emitted.remove(&field);
                }
                changed |= extend_presence(
                    outputs
                        .entry((Some(handler.node), port.clone()))
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
                let inherited = node_input_presence(plan, handler.node, &inputs);
                let expired = node::Port::new("expired").map_err(|_| {
                    PlanValidationError::InvalidJoinActivation {
                        node: plan.nodes[handler.node.index()].path.clone(),
                    }
                })?;
                changed |= extend_presence(
                    outputs.entry((Some(handler.node), expired)).or_default(),
                    &inherited,
                );
            }
        }

        for (slot_index, planned) in plan.nodes.iter().enumerate() {
            if !matches!(planned.kind, NodeKind::Connector(_)) {
                continue;
            }
            let slot = Slot::new(slot_index as u32);
            let inherited = node_input_presence(plan, slot, &inputs);
            for port in planned
                .ports
                .iter()
                .filter(|port| port.direction == node::Direction::Output)
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
        let base = activation_presence(handler, &inputs, &all_fields);
        let owner = &plan.nodes[handler.node.index()].path.to_string();
        if let Some(guard) = &handler.guard {
            required_meta_reads_hold(&guard.root, &base, owner)?;
        }
        for step in &handler.body {
            match step {
                Step::Let { expr, .. } | Step::SetVar { expr, .. } => {
                    required_meta_reads_hold(&expr.root, &base, owner)?;
                }
                Step::Emit { value, .. } => {
                    for expression in emission_expressions(value) {
                        required_meta_reads_hold(expression, &base, owner)?;
                    }
                }
                Step::Stop => {}
            }
        }
    }

    for (slot_index, planned) in plan.nodes.iter().enumerate() {
        let slot = Slot::new(slot_index as u32);
        let base = node_input_presence(plan, slot, &inputs);
        if base.is_empty() {
            continue;
        }
        let owner = planned.path.to_string();
        if let NodeKind::Connector(spec) = &planned.kind {
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
    slot: Slot,
    inputs: &EndpointPresence,
) -> MetaPresence {
    plan.nodes[slot.index()]
        .ports
        .iter()
        .filter(|port| port.direction == node::Direction::Input)
        .filter_map(|port| inputs.get(&(Some(slot), port.port.clone())))
        .flat_map(BTreeSet::iter)
        .cloned()
        .collect()
}

fn activation_presence(
    handler: &crate::plan::Handler,
    inputs: &EndpointPresence,
    all_fields: &MetaPresence,
) -> MetaPresence {
    match &handler.activation {
        node::HandlerActivation::Port(port) => inputs
            .get(&(Some(handler.node), port.clone()))
            .cloned()
            .unwrap_or_default(),
        node::HandlerActivation::Join { .. } => all_fields.clone(),
        node::HandlerActivation::Lifecycle(_) => inputs
            .iter()
            .filter(|((slot, _), _)| *slot == Some(handler.node))
            .flat_map(|(_, fields)| fields)
            .cloned()
            .collect(),
    }
}

fn emission_meta_writes(emission: &Emission) -> MetaPresence {
    match emission {
        Emission::Patch(writes) | Emission::Replace(writes) => writes
            .iter()
            .filter_map(|write| match &write.target {
                crate::envelope::WriteTarget::App(field) => {
                    Some(WritableMetaField::App(field.clone()))
                }
                crate::envelope::WriteTarget::Ext(field) => {
                    Some(WritableMetaField::Ext(field.clone()))
                }
                crate::envelope::WriteTarget::Data(_) => None,
            })
            .collect(),
        Emission::Identity | Emission::Data(_) | Emission::Nothing => MetaPresence::new(),
    }
}

fn emission_expressions(emission: &Emission) -> Vec<&expr::Expr> {
    match emission {
        Emission::Data(tree) => alloc::vec![&tree.root],
        Emission::Patch(writes) | Emission::Replace(writes) => {
            writes.iter().map(|write| &write.expr.root).collect()
        }
        Emission::Identity | Emission::Nothing => Vec::new(),
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
        .validate()
        .map_err(|_| PlanValidationError::UnrepresentableConstant)
}

/// Every map literal in a shipped expression tree is construction-sorted —
/// the parser sorts them, so an unsorted or duplicated literal is a
/// second byte form of the same expression, which the re-encoding
/// check cannot catch (it faithfully reproduces whatever order the
/// tree carries). Recursion is safe here: the nesting budget has
/// already bounded the tree.
fn expr_canonical(expr: &expr::Expr) -> Result<(), PlanValidationError> {
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
                .validate()
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
    ty.validate()
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
