//! Structural validation of an [`ExecutionPlan`] — every invariant
//! the machine dereferences without checking and every canonical
//! order the plan documents, held once, at a boundary, by a type.

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::{String, ToString};

use crate::cel;
use crate::datatype::{DataType, TypeError};
use crate::node;
use crate::plan::{Emission, EndpointRef, ExecutionPlan, Node, NodeKind, Slot, Step};

/// Why a plan is not executable — one named invariant violation. The
/// variants are semantic, not prose: a caller can match on the class
/// of defect; `Display` renders the human diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanValidationError {
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
    /// A connector's error set names one kind twice.
    DuplicateErrorKind {
        /// The declaring node.
        node: node::Path,
        /// The repeated kind.
        kind: String,
    },
    /// A CEL map literal names one key twice.
    DuplicateMapKey {
        /// The repeated key.
        key: String,
    },
    /// A port table redundantly declares an untyped conventional port.
    /// `in`, `out`, and `err` already exist; only an explicit type adds
    /// semantic information worth transporting in the raw v6 table.
    ImplicitPortDeclared {
        /// The declaring node; `None` is the boundary.
        node: Option<node::Path>,
        /// The implicit port.
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
    /// A constant value no source can express — a struct constant:
    /// CEL cannot construct one, so the compiler never emits one,
    /// and the codec's tables do not carry one. Refused so
    /// `encode ⇒ decode` stays total.
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
    /// The logical type is valid in the M1.3 algebra but has no spelling in
    /// the currently published plan-package registry.
    DataTypeNotInFormat {
        /// Where the type occurs in the plan.
        location: String,
        /// The valid but not-yet-transportable type.
        ty: DataType,
        /// Published format that cannot carry it.
        format: u32,
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
            Self::SlotOutOfBounds { slot, node_count } => write!(
                f,
                "invalid plan: slot {} of {node_count} nodes",
                slot.index()
            ),
            Self::DuplicateNodePath { path } => {
                write!(f, "invalid plan: duplicate node path `{path}`")
            }
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
                    "invalid plan: `{node}` carries a codec outside the dispatch domain (lines | jsonl)"
                )
            }
            Self::MissingCodec { node } => {
                write!(
                    f,
                    "invalid plan: `{node}` is a Process connector without a codec — the sheet requires one"
                )
            }
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
            Self::DuplicateErrorKind { node, kind } => {
                write!(f, "invalid plan: duplicate error kind `{kind}` on `{node}`")
            }
            Self::DuplicateMapKey { key } => {
                write!(f, "invalid plan: duplicate map literal key `{key}`")
            }
            Self::ImplicitPortDeclared { node, port } => match node {
                Some(node) => write!(
                    f,
                    "invalid plan: `{node}` declares the implicit port `{port}`"
                ),
                None => write!(
                    f,
                    "invalid plan: the boundary declares the implicit port `{port}`"
                ),
            },
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
            Self::DataTypeNotInFormat {
                location,
                ty,
                format,
            } => write!(
                f,
                "invalid plan: type `{ty}` at {location} has no plan-package v{format} encoding"
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
        validate(&plan)?;
        let resolved_ports = super::type_resolution::resolve(&plan)?;
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
    /// Unlike the raw v6 declarations, this table contains no inference
    /// variable and no recovery hole. It is derived again when a package is
    /// decoded; B8 will make it part of the published v7 representation.
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
/// schema — the Registry-less half of the CEL grid.
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
fn asset_indexes_bound(plan: &ExecutionPlan, expr: &cel::Expr) -> Result<(), PlanValidationError> {
    match expr {
        cel::Expr::Asset(index) => {
            if (*index as usize) < plan.assets.len() {
                Ok(())
            } else {
                Err(PlanValidationError::AssetOutOfBounds {
                    index: *index,
                    count: plan.assets.len(),
                })
            }
        }
        cel::Expr::Literal(_) | cel::Expr::Path(_) => Ok(()),
        cel::Expr::Unary { expr, .. } => asset_indexes_bound(plan, expr),
        cel::Expr::Binary { left, right, .. } => {
            asset_indexes_bound(plan, left)?;
            asset_indexes_bound(plan, right)
        }
        cel::Expr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            asset_indexes_bound(plan, cond)?;
            asset_indexes_bound(plan, then)?;
            asset_indexes_bound(plan, otherwise)
        }
        cel::Expr::Index { base, index } => {
            asset_indexes_bound(plan, base)?;
            asset_indexes_bound(plan, index)
        }
        cel::Expr::Call { args, .. } => args
            .iter()
            .try_for_each(|arg| asset_indexes_bound(plan, arg)),
        cel::Expr::List(items) => items
            .iter()
            .try_for_each(|item| asset_indexes_bound(plan, item)),
        cel::Expr::MapLit(pairs) => pairs
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
        .map_err(|reason| PlanValidationError::InvalidDataType {
            location: location.clone(),
            reason,
        })?;
    if ty.is_v6_wire_type() {
        Ok(())
    } else {
        Err(PlanValidationError::DataTypeNotInFormat {
            location,
            ty: ty.clone(),
            format: super::serialization::FORMAT_VERSION,
        })
    }
}

fn tree_type_at(tree: &cel::Tree, location: String) -> Result<(), PlanValidationError> {
    data_type_at(&tree.ty, location)
}

/// Holds the package boundary while B1 grows the canonical algebra ahead of
/// B8's categorical wire bump. Every type-bearing plan position passes this
/// one audit before the infallible v6 encoder can observe the plan.
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
    for node in &plan.nodes {
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
    // Untyped conventional declarations are redundant byte forms. A
    // typed declaration is different: it carries a source constraint
    // that B2 must retain until the complete resolved table ships in v7.
    let conventional_direction = |port: &node::Port| match port.name() {
        "in" => Some(node::Direction::Input),
        "out" | "err" => Some(node::Direction::Output),
        _ => None,
    };
    let implicit = |port: &node::Port| conventional_direction(port).is_some();
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
        .find(|decl| implicit(&decl.port) && decl.ty.is_none())
    {
        return Err(PlanValidationError::ImplicitPortDeclared {
            node: None,
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
        if let Some(decl) = node
            .ports
            .iter()
            .find(|decl| implicit(&decl.port) && decl.ty.is_none())
        {
            return Err(PlanValidationError::ImplicitPortDeclared {
                node: Some(node.path.clone()),
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
            }
        }
        for pair in node.vars.windows(2) {
            if pair[0].name >= pair[1].name {
                return Err(PlanValidationError::NonCanonicalOrder { collection: "vars" });
            }
        }
        if let NodeKind::Connector(spec) = &node.kind {
            for (_, setting) in &spec.attributes {
                if let crate::plan::AttrSetting::Value(value) = setting {
                    value_expressible(value)?;
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
                match spec.attribute("codec") {
                    Some(crate::plan::AttrSetting::Word(word))
                        if word == "lines" || word == "jsonl" => {}
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
            // The error set is a SET — the hash reads it sorted, so
            // an unsorted table would be a second byte form of the
            // same contract.
            let mut kinds = BTreeSet::new();
            for kind in &spec.error_set {
                if !kinds.insert(kind.as_str()) {
                    return Err(PlanValidationError::DuplicateErrorKind {
                        node: node.path.clone(),
                        kind: kind.clone(),
                    });
                }
            }
            for pair in spec.error_set.windows(2) {
                if pair[0] >= pair[1] {
                    return Err(PlanValidationError::NonCanonicalOrder {
                        collection: "error_set",
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

    // References: slots, ports in their roles, handlers, vars. The
    // The conventional trio is always available in its fixed directions.
    // A typed restatement may additionally occur in the table; direction
    // remains conventional while the row supplies its source constraint.
    let has_port = |node: &Node, port: &node::Port, direction: node::Direction| match port.name() {
        "in" => direction == node::Direction::Input,
        "out" | "err" => direction == node::Direction::Output,
        _ => node
            .ports
            .iter()
            .any(|decl| &decl.port == port && decl.direction == direction),
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
                        && !matches!(endpoint.port.name(), "in" | "out" | "err")
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
                    // Conventional direction first — `in` is the graph's
                    // input, `out` and `err` its outputs — then extra rows.
                    let actual = match endpoint.port.name() {
                        "in" => Some(node::Direction::Input),
                        "out" | "err" => Some(node::Direction::Output),
                        _ => plan
                            .boundary
                            .iter()
                            .find(|decl| decl.port == endpoint.port)
                            .map(|decl| decl.direction),
                    };
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
                    .or_else(|| {
                        // The connector contract's effective conventional pin
                        // — the same authority the checker, the
                        // mock judge, and the machine consult
                        // (D-2026-08-11-1, amended).
                        match &node.kind {
                            NodeKind::Connector(spec) => {
                                crate::boundary::resolved_port_type(spec, &endpoint.port)
                            }
                            NodeKind::Operator => None,
                        }
                    })
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
    for handler in &plan.handlers {
        let node = node_at(handler.node)?;
        let node::HandlerActivation::Port(port) = &handler.activation;
        if !has_port(node, port, node::Direction::Input) {
            return Err(PlanValidationError::WrongPortDirection {
                node: Some(node.path.clone()),
                port: port.clone(),
                expected: node::Direction::Input,
            });
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
            }
        }
    }
    // Handler order is canonical — (slot, activation key), the same
    // key the lowering and the hash sort by, because declaration
    // order between handlers carries neither semantics nor identity.
    // A repeated activation is one trigger fired twice: dispatch
    // would silently shadow the second, so it refuses by name —
    // globally first, like every other uniqueness. Slot bounds are
    // already proven above.
    let key =
        |handler: &crate::plan::Handler| (handler.node.index(), handler.activation.canonical_key());
    let mut activations = BTreeSet::new();
    for handler in &plan.handlers {
        if !activations.insert(key(handler)) {
            return Err(PlanValidationError::DuplicateHandler {
                node: node_at(handler.node)?.path.clone(),
                activation: handler.activation.clone(),
            });
        }
    }
    for pair in plan.handlers.windows(2) {
        if key(&pair[0]) >= key(&pair[1]) {
            return Err(PlanValidationError::NonCanonicalOrder {
                collection: "handlers",
            });
        }
    }
    Ok(())
}

/// The expressible-constant gate. CEL cannot construct a struct, so
/// the one struct constant a compiled plan holds is the EMPTY record
/// literal (`parent = {}`, D-2026-08-09-2) — the v4 tables read it
/// back, and anything with fields still refuses: admitting more than the
/// compiler emits would make hand-forged plans indistinguishable from
/// compiled ones. A runtime laziness witness is not a core
/// [`Value`](crate::value::Value) and is therefore unrepresentable here by
/// construction.
fn value_expressible(value: &crate::value::Value) -> Result<(), PlanValidationError> {
    value
        .validate()
        .map_err(|_| PlanValidationError::UnrepresentableConstant)?;
    value_shape_expressible(value)
}

fn value_shape_expressible(value: &crate::value::Value) -> Result<(), PlanValidationError> {
    match value {
        // The current expression subset cannot CONSTRUCT a struct; the one
        // struct constant a compiler emits is the EMPTY record literal (`parent = {}`,
        // D-2026-08-09-1). Anything with fields still refuses.
        crate::value::Value::Struct(fields) if fields.is_empty() => Ok(()),
        // The decoder's exact rule (D-2026-08-13-1, review round):
        // a plan constant is a DECLARED-scale decimal — `decimal()`
        // preserves written scale, and a document-origin (minimal)
        // form satisfies the same gate. A name must not change a
        // program's validity: `val p = decimal("1.50")` folds to
        // this constant.
        crate::value::Value::Decimal { unscaled, scale } => {
            if crate::json::declared_decimal_well_formed(*unscaled, *scale) {
                Ok(())
            } else {
                Err(PlanValidationError::UnrepresentableConstant)
            }
        }
        crate::value::Value::Struct(_) => Err(PlanValidationError::UnrepresentableConstant),
        crate::value::Value::Array(items) => items.iter().try_for_each(value_shape_expressible),
        crate::value::Value::Map(entries) => entries.values().try_for_each(value_shape_expressible),
        _ => Ok(()),
    }
}

/// Every map literal in a shipped expression tree is construction-sorted —
/// the parser sorts them, so an unsorted or duplicated literal is a
/// second byte form of the same expression, which the re-encoding
/// check cannot catch (it faithfully reproduces whatever order the
/// tree carries). Recursion is safe here: the nesting budget has
/// already bounded the tree.
fn expr_canonical(expr: &cel::Expr) -> Result<(), PlanValidationError> {
    match expr {
        cel::Expr::Literal(value) => value_expressible(value),
        // The index is bounds-checked against the asset table in
        // the walk that owns the plan (`meta_target_known`'s
        // sibling); canonical here.
        cel::Expr::Asset(_) => Ok(()),
        cel::Expr::Path(_) => Ok(()),
        cel::Expr::Unary { expr, .. } => expr_canonical(expr),
        cel::Expr::Binary { left, right, .. } => {
            expr_canonical(left)?;
            expr_canonical(right)
        }
        cel::Expr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            expr_canonical(cond)?;
            expr_canonical(then)?;
            expr_canonical(otherwise)
        }
        cel::Expr::Index { base, index } => {
            expr_canonical(base)?;
            expr_canonical(index)
        }
        cel::Expr::Call { args, .. } => args.iter().try_for_each(expr_canonical),
        cel::Expr::List(items) => items.iter().try_for_each(expr_canonical),
        cel::Expr::MapLit(pairs) => {
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
    }
}
