//! The plan package — the transportable, verifiable serialization
//! of an [`ExecutionPlan`], per `docs/architecture/plan-package.md`.
//!
//! Format v2: the 4-byte magic `RKPB`, then one canonical CBOR map —
//! format version, the plan's semantic hash, the body as an embedded
//! byte string, and a blake3 checksum over the domain-framed
//! association `domain ‖ version ‖ semantic_hash ‖ body`, so a
//! tampered header refuses like a tampered body. The decoder
//! verifies magic, version, and checksum before parsing a single
//! body byte, then holds what it parsed to the rest of the contract:
//! the plan must validate structurally, and re-encoding it must
//! reproduce the exact input bytes — canonicity as one total check.
//! Every refusal is typed; an unknown version is a refusal by name,
//! never a silent migration.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::{format, vec};

use crate::canonical_cbor::{self, Value as Cbor};
use crate::cel;
use crate::envelope::WriteTarget;
use crate::identity::encoding::cel_tagged;
use crate::node;
use crate::plan::{
    AttrSetting, ConnectorSpec, Edge, Emission, EndpointRef, ExecutionPlan, FieldWrite, Handler,
    Node, NodeKind, PlanValidationError, SemanticHash, Slot, Step, ValidatedExecutionPlan, VarCell,
};

/// The package's leading magic.
pub const MAGIC: [u8; 4] = *b"RKPB";

/// The format version this module writes and reads.
pub const FORMAT_VERSION: u32 = 2;

/// The checksum domain — fixed-width framing, so the covered fields
/// cannot slide into one another.
const CHECKSUM_DOMAIN: &[u8] = b"rastok.plan-package\0";

/// A typed refusal from `plan::decode`. Total: nothing is guessed,
/// nothing is migrated silently, and the canonical reader's own
/// categories are preserved, never collapsed.
#[derive(Debug, Clone, PartialEq)]
pub enum DecodeError {
    /// The bytes do not start with the package magic.
    BadMagic,
    /// A format version this build does not read — carrying the
    /// value actually read, however large.
    UnsupportedVersion(u64),
    /// The blake3 checksum does not match the covered fields.
    BrokenChecksum,
    /// The canonical reader refused, with its own category — a
    /// truncated input and a syntactically foreign one are different
    /// diagnoses and stay different.
    Read(canonical_cbor::DecodeError),
    /// Bytes inside the format's frame that its tables cannot read.
    Malformed(String),
    /// Bytes that read, but are not the canonical encoding of what
    /// they say.
    NonCanonical(&'static str),
    /// A decoded plan that violates the ExecutionPlan invariants —
    /// well-shaped bytes naming impossible structure.
    Invalid(PlanValidationError),
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadMagic => f.write_str("not a plan package (bad magic)"),
            Self::UnsupportedVersion(version) => {
                write!(
                    f,
                    "plan-package format v{version} is not readable by this build"
                )
            }
            Self::BrokenChecksum => f.write_str("the checksum does not match the covered fields"),
            Self::Read(error) => write!(f, "unreadable canonical bytes: {error}"),
            Self::Malformed(detail) => write!(f, "malformed plan package: {detail}"),
            Self::NonCanonical(detail) => write!(f, "not the canonical encoding: {detail}"),
            Self::Invalid(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for DecodeError {}

/// The v2 checksum: blake3 over the domain-framed association.
/// Every covered field is fixed-width or last, so fields cannot
/// slide into one another.
fn checksum(version: u32, hash: &[u8; 32], body: &[u8]) -> [u8; 32] {
    let mut digest = [0u8; 32];
    let mut hasher = blake3::Hasher::new();
    hasher.update(CHECKSUM_DOMAIN);
    hasher.update(&version.to_le_bytes());
    hasher.update(hash);
    hasher.update(body);
    hasher.finalize_xof().fill(&mut digest);
    digest
}

/// Encodes a validated plan and its declared identity into a
/// package. Infallible: structural validity is carried by the type,
/// so what this function accepts, [`decode`] reads back.
#[must_use]
pub fn encode(plan: &ValidatedExecutionPlan, declared_hash: &SemanticHash) -> Vec<u8> {
    let body = canonical_cbor::encode(&plan_tree(plan.plan()));
    let digest = checksum(FORMAT_VERSION, declared_hash.as_bytes(), &body);
    let envelope = Cbor::Map(BTreeMap::from([
        (1, Cbor::Uint(u64::from(FORMAT_VERSION))),
        (2, Cbor::Bytes(declared_hash.as_bytes().to_vec())),
        (3, Cbor::Bytes(body)),
        (4, Cbor::Bytes(digest.to_vec())),
    ]));
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&canonical_cbor::encode(&envelope));
    out
}

/// Decodes a package back into a validated plan and its declared
/// identity. Magic, version, and checksum are verified before a
/// body byte is parsed; the parsed plan passes through
/// [`ValidatedExecutionPlan`] — the one validation, at the type —
/// and the whole input must be the canonical encoding of what was
/// read, byte for byte.
///
/// # Errors
///
/// A [`DecodeError`] naming exactly what refused.
pub fn decode(bytes: &[u8]) -> Result<(ValidatedExecutionPlan, SemanticHash), DecodeError> {
    let Some(rest) = bytes.strip_prefix(&MAGIC) else {
        return Err(DecodeError::BadMagic);
    };
    let (envelope, used) = canonical_cbor::decode(rest).map_err(DecodeError::Read)?;
    if used != rest.len() {
        return Err(DecodeError::Malformed("trailing bytes".into()));
    }
    let Cbor::Map(pairs) = envelope else {
        return Err(DecodeError::Malformed("the envelope is not a map".into()));
    };
    let field = |key: u64| pairs.get(&key);
    let Some(Cbor::Uint(version)) = field(1) else {
        return Err(DecodeError::Malformed("no format version".into()));
    };
    // The version read is reported as read — a value beyond u32 is
    // simply an unsupported version carrying itself, never saturated
    // into a number that was not in the bytes.
    if *version != u64::from(FORMAT_VERSION) {
        return Err(DecodeError::UnsupportedVersion(*version));
    }
    let Some(Cbor::Bytes(hash)) = field(2) else {
        return Err(DecodeError::Malformed("no declared semantic hash".into()));
    };
    let hash: [u8; 32] = hash
        .as_slice()
        .try_into()
        .map_err(|_| DecodeError::Malformed("the declared hash is not 32 bytes".into()))?;
    let Some(Cbor::Bytes(body)) = field(3) else {
        return Err(DecodeError::Malformed("no body".into()));
    };
    let Some(Cbor::Bytes(declared)) = field(4) else {
        return Err(DecodeError::Malformed("no checksum".into()));
    };
    if declared.as_slice() != checksum(FORMAT_VERSION, &hash, body) {
        return Err(DecodeError::BrokenChecksum);
    }
    let (tree, used) = canonical_cbor::decode(body).map_err(DecodeError::Read)?;
    if used != body.len() {
        return Err(DecodeError::Malformed("trailing body bytes".into()));
    }
    let plan = plan_from(&tree)?;
    let validated = ValidatedExecutionPlan::try_new(plan).map_err(DecodeError::Invalid)?;
    // Canonicity, as one total check: the only accepted bytes for
    // this plan are the ones this module would write. Re-encoding
    // catches non-minimal integers, reordered or duplicated keys,
    // and every other non-canonical variant at once — and the outer
    // envelope is held to the same rule.
    if canonical_cbor::encode(&plan_tree(validated.plan())) != *body {
        return Err(DecodeError::NonCanonical(
            "the body is not the canonical encoding of its plan",
        ));
    }
    let canonical_envelope = Cbor::Map(BTreeMap::from([
        (1, Cbor::Uint(u64::from(FORMAT_VERSION))),
        (2, Cbor::Bytes(hash.to_vec())),
        (3, Cbor::Bytes(body.clone())),
        (4, Cbor::Bytes(declared.clone())),
    ]));
    if canonical_cbor::encode(&canonical_envelope) != rest {
        return Err(DecodeError::NonCanonical(
            "the envelope is not the canonical encoding of its fields",
        ));
    }
    Ok((validated, SemanticHash::from_bytes(hash)))
}

// ===================== encoding: plan → tree =====================

fn plan_tree(plan: &ExecutionPlan) -> Cbor {
    Cbor::Map(BTreeMap::from([
        (1, Cbor::Array(plan.nodes.iter().map(node_tree).collect())),
        (
            2,
            Cbor::Array(
                plan.boundary
                    .iter()
                    .map(|(port, direction)| port_decl_tree(port, *direction))
                    .collect(),
            ),
        ),
        (3, Cbor::Array(plan.edges.iter().map(edge_tree).collect())),
        (
            4,
            Cbor::Array(plan.handlers.iter().map(handler_tree).collect()),
        ),
    ]))
}

fn node_tree(node: &Node) -> Cbor {
    let kind = match &node.kind {
        NodeKind::Operator => Cbor::Uint(1),
        NodeKind::Connector(spec) => Cbor::Array(vec![Cbor::Uint(2), spec_tree(spec)]),
    };
    Cbor::Map(BTreeMap::from([
        (1, Cbor::Text(node.path.to_string())),
        (2, kind),
        (
            3,
            Cbor::Array(
                node.ports
                    .iter()
                    .map(|(port, direction)| port_decl_tree(port, *direction))
                    .collect(),
            ),
        ),
        (4, Cbor::Array(node.vars.iter().map(var_tree).collect())),
    ]))
}

fn spec_tree(spec: &ConnectorSpec) -> Cbor {
    let effect = match spec.effect {
        node::Effect::None => Cbor::Uint(0),
        node::Effect::Class(class) => Cbor::Array(vec![
            Cbor::Uint(1),
            Cbor::Uint(match class {
                node::EffectClass::Idempotent => 1,
                node::EffectClass::Compensable => 2,
                node::EffectClass::Irreversible => 3,
            }),
        ]),
    };
    let behavior = Cbor::Uint(match spec.behavior {
        node::Behavior::Deterministic => 1,
        node::Behavior::Probabilistic => 2,
    });
    Cbor::Map(BTreeMap::from([
        (1, Cbor::Text(spec.contract.clone())),
        (2, effect),
        (3, behavior),
        (
            4,
            Cbor::Array(
                spec.attributes
                    .iter()
                    .map(|(name, setting)| {
                        Cbor::Array(vec![Cbor::Text(name.clone()), attr_tree(setting)])
                    })
                    .collect(),
            ),
        ),
        (
            5,
            Cbor::Array(
                spec.error_set
                    .iter()
                    .map(|kind| Cbor::Text(kind.clone()))
                    .collect(),
            ),
        ),
    ]))
}

fn attr_tree(setting: &AttrSetting) -> Cbor {
    match setting {
        AttrSetting::Word(word) => Cbor::Array(vec![Cbor::Uint(1), Cbor::Text(word.clone())]),
        AttrSetting::Value(value) => Cbor::Array(vec![Cbor::Uint(2), cel_tagged(value)]),
    }
}

fn port_decl_tree(port: &node::Port, direction: node::Direction) -> Cbor {
    Cbor::Array(vec![
        Cbor::Text(port.name().to_string()),
        Cbor::Uint(match direction {
            node::Direction::Input => 1,
            node::Direction::Output => 2,
        }),
    ])
}

fn var_tree(var: &VarCell) -> Cbor {
    let mut pairs = BTreeMap::from([(1, Cbor::Text(var.name.clone())), (2, type_tree(&var.ty))]);
    if let Some(init) = &var.init {
        pairs.insert(3, cel_tagged(init));
    }
    Cbor::Map(pairs)
}

fn edge_tree(edge: &Edge) -> Cbor {
    let to = match &edge.to {
        Some(endpoint) => endpoint_tree(endpoint),
        None => Cbor::Null,
    };
    Cbor::Array(vec![endpoint_tree(&edge.from), to])
}

fn endpoint_tree(endpoint: &EndpointRef) -> Cbor {
    let slot = match endpoint.node {
        Some(slot) => Cbor::Uint(slot.index() as u64),
        None => Cbor::Null,
    };
    Cbor::Array(vec![slot, Cbor::Text(endpoint.port.name().to_string())])
}

fn handler_tree(handler: &Handler) -> Cbor {
    let node::HandlerActivation::Port(port) = &handler.activation;
    let activation = Cbor::Array(vec![Cbor::Uint(1), Cbor::Text(port.name().to_string())]);
    let mut pairs = BTreeMap::from([
        (1, Cbor::Uint(handler.node.index() as u64)),
        (2, activation),
    ]);
    if let Some(guard) = &handler.guard {
        pairs.insert(3, tree_tree(guard));
    }
    pairs.insert(4, Cbor::Array(handler.body.iter().map(step_tree).collect()));
    Cbor::Map(pairs)
}

fn step_tree(step: &Step) -> Cbor {
    match step {
        Step::Let { name, expr } => Cbor::Array(vec![
            Cbor::Uint(1),
            Cbor::Text(name.clone()),
            tree_tree(expr),
        ]),
        Step::Emit { port, value } => Cbor::Array(vec![
            Cbor::Uint(2),
            Cbor::Text(port.name().to_string()),
            emission_tree(value),
        ]),
        Step::SetVar { name, expr } => Cbor::Array(vec![
            Cbor::Uint(3),
            Cbor::Text(name.clone()),
            tree_tree(expr),
        ]),
    }
}

fn emission_tree(emission: &Emission) -> Cbor {
    match emission {
        Emission::Identity => Cbor::Array(vec![Cbor::Uint(1)]),
        Emission::Patch(writes) => Cbor::Array(vec![Cbor::Uint(2), writes_tree(writes)]),
        Emission::Replace(writes) => Cbor::Array(vec![Cbor::Uint(3), writes_tree(writes)]),
        Emission::Data(tree) => Cbor::Array(vec![Cbor::Uint(4), tree_tree(tree)]),
        Emission::Nothing => Cbor::Array(vec![Cbor::Uint(5)]),
    }
}

fn writes_tree(writes: &[FieldWrite]) -> Cbor {
    Cbor::Array(
        writes
            .iter()
            .map(|write| Cbor::Array(vec![target_tree(&write.target), tree_tree(&write.expr)]))
            .collect(),
    )
}

fn target_tree(target: &WriteTarget) -> Cbor {
    match target {
        WriteTarget::Data(path) => Cbor::Array(vec![
            Cbor::Uint(1),
            Cbor::Array(path.iter().map(|seg| Cbor::Text(seg.clone())).collect()),
        ]),
        WriteTarget::AppMeta(field) => Cbor::Array(vec![Cbor::Uint(2), Cbor::Text(field.clone())]),
        WriteTarget::CustomMeta(namespace, field) => Cbor::Array(vec![
            Cbor::Uint(3),
            Cbor::Text(namespace.clone()),
            Cbor::Text(field.clone()),
        ]),
        WriteTarget::MimeType => Cbor::Array(vec![Cbor::Uint(4)]),
    }
}

fn tree_tree(tree: &cel::Tree) -> Cbor {
    Cbor::Array(vec![type_tree(&tree.ty), expr_tree(&tree.root)])
}

fn type_tree(ty: &cel::Type) -> Cbor {
    match ty {
        cel::Type::Bool => Cbor::Uint(1),
        cel::Type::Int => Cbor::Uint(2),
        cel::Type::Uint => Cbor::Uint(3),
        cel::Type::Double => Cbor::Uint(4),
        cel::Type::Text => Cbor::Uint(5),
        cel::Type::Bytes => Cbor::Uint(6),
        cel::Type::Duration => Cbor::Uint(7),
        cel::Type::Timestamp => Cbor::Uint(8),
        cel::Type::Null => Cbor::Uint(9),
        cel::Type::Dyn => Cbor::Uint(10),
        cel::Type::List(inner) => Cbor::Array(vec![Cbor::Uint(11), type_tree(inner)]),
        cel::Type::Map(inner) => Cbor::Array(vec![Cbor::Uint(12), type_tree(inner)]),
    }
}

fn expr_tree(expr: &cel::Expr) -> Cbor {
    match expr {
        cel::Expr::Literal(value) => Cbor::Array(vec![Cbor::Uint(1), cel_tagged(value)]),
        cel::Expr::Path(segments) => Cbor::Array(vec![
            Cbor::Uint(2),
            Cbor::Array(segments.iter().map(|seg| Cbor::Text(seg.clone())).collect()),
        ]),
        cel::Expr::Unary { op, expr } => Cbor::Array(vec![
            Cbor::Uint(3),
            Cbor::Uint(match op {
                cel::UnaryOp::Not => 1,
                cel::UnaryOp::Neg => 2,
            }),
            expr_tree(expr),
        ]),
        cel::Expr::Binary { op, left, right } => Cbor::Array(vec![
            Cbor::Uint(4),
            Cbor::Uint(binop_code(*op)),
            expr_tree(left),
            expr_tree(right),
        ]),
        cel::Expr::Ternary {
            cond,
            then,
            otherwise,
        } => Cbor::Array(vec![
            Cbor::Uint(5),
            expr_tree(cond),
            expr_tree(then),
            expr_tree(otherwise),
        ]),
        cel::Expr::Index { base, index } => {
            Cbor::Array(vec![Cbor::Uint(6), expr_tree(base), expr_tree(index)])
        }
        cel::Expr::Call { function, args } => Cbor::Array(vec![
            Cbor::Uint(7),
            Cbor::Uint(match function {
                cel::Function::Duration => 1,
                cel::Function::Timestamp => 2,
                cel::Function::Size => 3,
            }),
            Cbor::Array(args.iter().map(expr_tree).collect()),
        ]),
        cel::Expr::List(items) => Cbor::Array(vec![
            Cbor::Uint(8),
            Cbor::Array(items.iter().map(expr_tree).collect()),
        ]),
        cel::Expr::MapLit(pairs) => Cbor::Array(vec![
            Cbor::Uint(9),
            Cbor::Array(
                pairs
                    .iter()
                    .map(|(key, value)| {
                        Cbor::Array(vec![Cbor::Text(key.clone()), expr_tree(value)])
                    })
                    .collect(),
            ),
        ]),
    }
}

/// The 0-based level of the deepest CBOR node the body encoding of
/// this plan will contain — computed WITHOUT building the tree and
/// without recursion, so the measurement itself never rides the
/// machine stack. Both real plan sources arrive pre-bounded: the
/// compiler gates expression depth at parse, and the decoder's
/// reader is budgeted — so everything that recurses after this check
/// (the tree builders above, the encoder, the expression walks) is
/// bounded too. A hand-BUILT plan nesting beyond the machine stack
/// is outside the raw-plan contract: its mere destruction would
/// overflow in its builder's hands before validation could answer.
/// The per-construct arithmetic mirrors the builders line by line;
/// `the_level_walker_matches_the_builders` pins the equality.
pub(crate) fn deepest_level(plan: &ExecutionPlan) -> usize {
    enum Item<'a> {
        Expr(&'a cel::Expr, usize),
        Val(&'a cel::Value, usize),
        Ty(&'a cel::Type, usize),
    }
    /// A `tree_tree`: its type and expression roots sit at `level`.
    fn tree<'a>(tree: &'a cel::Tree, level: usize, stack: &mut Vec<Item<'a>>) {
        stack.push(Item::Ty(&tree.ty, level));
        stack.push(Item::Expr(&tree.root, level));
    }
    // The root map's four table arrays sit at level 1 always.
    let mut max = 1usize;
    let mut stack: Vec<Item<'_>> = Vec::new();
    for node in &plan.nodes {
        // node map at 2: path, kind, and the two table arrays at 3.
        max = max.max(3);
        if let NodeKind::Connector(spec) = &node.kind {
            // kind array at 3, spec map at 4, its fields at 5.
            max = max.max(5);
            if matches!(spec.effect, node::Effect::Class(_)) {
                max = max.max(6);
            }
            for (_, setting) in &spec.attributes {
                // entry array at 6, name and attr_tree at 7, the
                // attr_tree's tag and payload at 8.
                max = max.max(8);
                if let AttrSetting::Value(value) = setting {
                    stack.push(Item::Val(value, 8));
                }
            }
            if !spec.error_set.is_empty() {
                max = max.max(6);
            }
        }
        if !node.ports.is_empty() {
            // decl array at 4, name and direction at 5.
            max = max.max(5);
        }
        for var in &node.vars {
            // var map at 4, its fields at 5.
            max = max.max(5);
            stack.push(Item::Ty(&var.ty, 5));
            if let Some(init) = &var.init {
                stack.push(Item::Val(init, 5));
            }
        }
    }
    if !plan.boundary.is_empty() {
        // decl array at 2, name and direction at 3.
        max = max.max(3);
    }
    if !plan.edges.is_empty() {
        // edge array at 2, endpoint arrays at 3, their fields at 4.
        max = max.max(4);
    }
    for handler in &plan.handlers {
        // handler map at 2, activation array at 3, its fields at 4.
        max = max.max(4);
        if let Some(guard) = &handler.guard {
            // tree_tree at 3, type and expression roots at 4.
            tree(guard, 4, &mut stack);
        }
        for step in &handler.body {
            // step array at 4, tag and name at 5.
            max = max.max(5);
            match step {
                Step::Let { expr, .. } | Step::SetVar { expr, .. } => tree(expr, 6, &mut stack),
                Step::Emit { value, .. } => match value {
                    // emission array at 5, its tag at 6.
                    Emission::Identity | Emission::Nothing => max = max.max(6),
                    Emission::Data(data) => tree(data, 7, &mut stack),
                    Emission::Patch(writes) | Emission::Replace(writes) => {
                        max = max.max(6);
                        for write in writes {
                            // write pair at 7, target array and
                            // tree_tree at 8, their fields at 9 — a
                            // data path's segments text at 10.
                            max = max.max(match &write.target {
                                WriteTarget::Data(segments) if !segments.is_empty() => 10,
                                _ => 9,
                            });
                            tree(&write.expr, 9, &mut stack);
                        }
                    }
                },
            }
        }
    }
    while let Some(item) = stack.pop() {
        match item {
            Item::Expr(expr, level) => {
                // Every variant is an array whose tag sits one down.
                max = max.max(level + 1);
                match expr {
                    cel::Expr::Literal(value) => stack.push(Item::Val(value, level + 1)),
                    cel::Expr::Path(segments) => {
                        if !segments.is_empty() {
                            max = max.max(level + 2);
                        }
                    }
                    cel::Expr::Unary { expr, .. } => stack.push(Item::Expr(expr, level + 1)),
                    cel::Expr::Binary { left, right, .. } => {
                        stack.push(Item::Expr(left, level + 1));
                        stack.push(Item::Expr(right, level + 1));
                    }
                    cel::Expr::Ternary {
                        cond,
                        then,
                        otherwise,
                    } => {
                        stack.push(Item::Expr(cond, level + 1));
                        stack.push(Item::Expr(then, level + 1));
                        stack.push(Item::Expr(otherwise, level + 1));
                    }
                    cel::Expr::Index { base, index } => {
                        stack.push(Item::Expr(base, level + 1));
                        stack.push(Item::Expr(index, level + 1));
                    }
                    cel::Expr::Call { args, .. } => {
                        for arg in args {
                            stack.push(Item::Expr(arg, level + 2));
                        }
                    }
                    cel::Expr::List(items) => {
                        for item in items {
                            stack.push(Item::Expr(item, level + 2));
                        }
                    }
                    cel::Expr::MapLit(pairs) => {
                        for (_, value) in pairs {
                            // pair array at +2, key text and value
                            // at +3.
                            max = max.max(level + 3);
                            stack.push(Item::Expr(value, level + 3));
                        }
                    }
                }
            }
            Item::Val(value, level) => {
                // `cel_tagged`: an array whose tag and scalar
                // payloads sit one down.
                max = max.max(level + 1);
                match value {
                    cel::Value::List(items) => {
                        for item in items {
                            stack.push(Item::Val(item, level + 2));
                        }
                    }
                    cel::Value::Map(entries) => {
                        for entry in entries.values() {
                            // pair array at +2, the tagged key's own
                            // tag and text at +4.
                            max = max.max(level + 4);
                            stack.push(Item::Val(entry, level + 3));
                        }
                    }
                    _ => {}
                }
            }
            Item::Ty(ty, level) => match ty {
                cel::Type::List(inner) | cel::Type::Map(inner) => {
                    max = max.max(level + 1);
                    stack.push(Item::Ty(inner, level + 1));
                }
                _ => max = max.max(level),
            },
        }
    }
    max
}

fn binop_code(op: cel::BinaryOp) -> u64 {
    match op {
        cel::BinaryOp::Mul => 1,
        cel::BinaryOp::Div => 2,
        cel::BinaryOp::Rem => 3,
        cel::BinaryOp::Add => 4,
        cel::BinaryOp::Sub => 5,
        cel::BinaryOp::Eq => 6,
        cel::BinaryOp::Ne => 7,
        cel::BinaryOp::Lt => 8,
        cel::BinaryOp::Le => 9,
        cel::BinaryOp::Gt => 10,
        cel::BinaryOp::Ge => 11,
        cel::BinaryOp::And => 12,
        cel::BinaryOp::Or => 13,
    }
}

// ===================== decoding: tree → plan =====================

fn err(detail: &str) -> DecodeError {
    DecodeError::Malformed(detail.into())
}

fn plan_from(tree: &Cbor) -> Result<ExecutionPlan, DecodeError> {
    let pairs = map_of(tree, "plan")?;
    let nodes = array_of(get(pairs, 1, "plan nodes")?, "plan nodes")?
        .iter()
        .map(node_from)
        .collect::<Result<Vec<_>, _>>()?;
    let boundary = array_of(get(pairs, 2, "plan boundary")?, "plan boundary")?
        .iter()
        .map(port_decl_from)
        .collect::<Result<Vec<_>, _>>()?;
    let edges = array_of(get(pairs, 3, "plan edges")?, "plan edges")?
        .iter()
        .map(edge_from)
        .collect::<Result<Vec<_>, _>>()?;
    let handlers = array_of(get(pairs, 4, "plan handlers")?, "plan handlers")?
        .iter()
        .map(handler_from)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ExecutionPlan {
        nodes,
        boundary,
        edges,
        handlers,
    })
}

fn node_from(tree: &Cbor) -> Result<Node, DecodeError> {
    let pairs = map_of(tree, "node")?;
    let path = node::Path::new(
        text_of(get(pairs, 1, "node path")?, "node path")?
            .split('.')
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
    )
    .map_err(|_| err("node path"))?;
    let kind = match get(pairs, 2, "node kind")? {
        Cbor::Uint(1) => NodeKind::Operator,
        Cbor::Array(parts) => match parts.as_slice() {
            [Cbor::Uint(2), spec] => NodeKind::Connector(spec_from(spec)?),
            _ => return Err(err("node kind")),
        },
        _ => return Err(err("node kind")),
    };
    let ports = array_of(get(pairs, 3, "node ports")?, "node ports")?
        .iter()
        .map(port_decl_from)
        .collect::<Result<Vec<_>, _>>()?;
    let vars = array_of(get(pairs, 4, "node vars")?, "node vars")?
        .iter()
        .map(var_from)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Node {
        path,
        kind,
        ports,
        vars,
    })
}

fn spec_from(tree: &Cbor) -> Result<ConnectorSpec, DecodeError> {
    let pairs = map_of(tree, "connector spec")?;
    let contract = text_of(get(pairs, 1, "contract")?, "contract")?.to_string();
    let effect = match get(pairs, 2, "effect")? {
        Cbor::Uint(0) => node::Effect::None,
        Cbor::Array(parts) => match parts.as_slice() {
            [Cbor::Uint(1), Cbor::Uint(class)] => node::Effect::Class(match class {
                1 => node::EffectClass::Idempotent,
                2 => node::EffectClass::Compensable,
                3 => node::EffectClass::Irreversible,
                _ => return Err(err("effect class")),
            }),
            _ => return Err(err("effect")),
        },
        _ => return Err(err("effect")),
    };
    let behavior = match get(pairs, 3, "behavior")? {
        Cbor::Uint(1) => node::Behavior::Deterministic,
        Cbor::Uint(2) => node::Behavior::Probabilistic,
        _ => return Err(err("behavior")),
    };
    let mut attributes = Vec::new();
    for entry in array_of(get(pairs, 4, "attributes")?, "attributes")? {
        let parts = array_of(entry, "attribute")?;
        match parts {
            [name, setting] => {
                attributes.push((
                    text_of(name, "attribute name")?.to_string(),
                    attr_from(setting)?,
                ));
            }
            _ => return Err(err("attribute")),
        }
    }
    let mut error_set = Vec::new();
    for kind in array_of(get(pairs, 5, "error set")?, "error set")? {
        error_set.push(text_of(kind, "error kind")?.to_string());
    }
    Ok(ConnectorSpec {
        contract,
        effect,
        behavior,
        attributes,
        error_set,
    })
}

fn attr_from(tree: &Cbor) -> Result<AttrSetting, DecodeError> {
    match array_of(tree, "attr setting")? {
        [Cbor::Uint(1), Cbor::Text(word)] => Ok(AttrSetting::Word(word.clone())),
        [Cbor::Uint(2), value] => Ok(AttrSetting::Value(cel_value_from(value)?)),
        _ => Err(err("attr setting")),
    }
}

fn port_decl_from(tree: &Cbor) -> Result<(node::Port, node::Direction), DecodeError> {
    match array_of(tree, "port declaration")? {
        [Cbor::Text(name), Cbor::Uint(direction)] => Ok((
            node::Port::new(name.clone()).map_err(|_| err("port name"))?,
            match direction {
                1 => node::Direction::Input,
                2 => node::Direction::Output,
                _ => return Err(err("port direction")),
            },
        )),
        _ => Err(err("port declaration")),
    }
}

fn var_from(tree: &Cbor) -> Result<VarCell, DecodeError> {
    let pairs = map_of(tree, "var")?;
    let name = text_of(get(pairs, 1, "var name")?, "var name")?.to_string();
    let ty = type_from(get(pairs, 2, "var type")?)?;
    let init = match find(pairs, 3) {
        Some(value) => Some(cel_value_from(value)?),
        None => None,
    };
    Ok(VarCell { name, ty, init })
}

fn edge_from(tree: &Cbor) -> Result<Edge, DecodeError> {
    match array_of(tree, "edge")? {
        [from, Cbor::Null] => Ok(Edge {
            from: endpoint_from(from)?,
            to: None,
        }),
        [from, to] => Ok(Edge {
            from: endpoint_from(from)?,
            to: Some(endpoint_from(to)?),
        }),
        _ => Err(err("edge")),
    }
}

fn endpoint_from(tree: &Cbor) -> Result<EndpointRef, DecodeError> {
    match array_of(tree, "endpoint")? {
        [slot, Cbor::Text(port)] => Ok(EndpointRef {
            node: match slot {
                Cbor::Null => None,
                Cbor::Uint(index) => {
                    Some(Slot::new(u32::try_from(*index).map_err(|_| err("slot"))?))
                }
                _ => return Err(err("endpoint slot")),
            },
            port: node::Port::new(port.clone()).map_err(|_| err("endpoint port"))?,
        }),
        _ => Err(err("endpoint")),
    }
}

fn handler_from(tree: &Cbor) -> Result<Handler, DecodeError> {
    let pairs = map_of(tree, "handler")?;
    let slot = match get(pairs, 1, "handler slot")? {
        Cbor::Uint(index) => Slot::new(u32::try_from(*index).map_err(|_| err("handler slot"))?),
        _ => return Err(err("handler slot")),
    };
    let activation = match array_of(get(pairs, 2, "activation")?, "activation")? {
        [Cbor::Uint(1), Cbor::Text(port)] => node::HandlerActivation::Port(
            node::Port::new(port.clone()).map_err(|_| err("activation port"))?,
        ),
        _ => return Err(err("activation")),
    };
    let guard = match find(pairs, 3) {
        Some(tree) => Some(tree_from(tree)?),
        None => None,
    };
    let body = array_of(get(pairs, 4, "handler body")?, "handler body")?
        .iter()
        .map(step_from)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Handler {
        node: slot,
        activation,
        guard,
        body,
    })
}

fn step_from(tree: &Cbor) -> Result<Step, DecodeError> {
    match array_of(tree, "step")? {
        [Cbor::Uint(1), Cbor::Text(name), expr] => Ok(Step::Let {
            name: name.clone(),
            expr: tree_from(expr)?,
        }),
        [Cbor::Uint(2), Cbor::Text(port), emission] => Ok(Step::Emit {
            port: node::Port::new(port.clone()).map_err(|_| err("emit port"))?,
            value: emission_from(emission)?,
        }),
        [Cbor::Uint(3), Cbor::Text(name), expr] => Ok(Step::SetVar {
            name: name.clone(),
            expr: tree_from(expr)?,
        }),
        _ => Err(err("step")),
    }
}

fn emission_from(tree: &Cbor) -> Result<Emission, DecodeError> {
    match array_of(tree, "emission")? {
        [Cbor::Uint(1)] => Ok(Emission::Identity),
        [Cbor::Uint(2), writes] => Ok(Emission::Patch(writes_from(writes)?)),
        [Cbor::Uint(3), writes] => Ok(Emission::Replace(writes_from(writes)?)),
        [Cbor::Uint(4), tree] => Ok(Emission::Data(tree_from(tree)?)),
        [Cbor::Uint(5)] => Ok(Emission::Nothing),
        _ => Err(err("emission")),
    }
}

fn writes_from(tree: &Cbor) -> Result<Vec<FieldWrite>, DecodeError> {
    array_of(tree, "writes")?
        .iter()
        .map(|entry| match array_of(entry, "write")? {
            [target, expr] => Ok(FieldWrite {
                target: target_from(target)?,
                expr: tree_from(expr)?,
            }),
            _ => Err(err("write")),
        })
        .collect()
}

fn target_from(tree: &Cbor) -> Result<WriteTarget, DecodeError> {
    match array_of(tree, "write target")? {
        [Cbor::Uint(1), Cbor::Array(segments)] => Ok(WriteTarget::Data(
            segments
                .iter()
                .map(|seg| Ok(text_of(seg, "path segment")?.to_string()))
                .collect::<Result<Vec<_>, DecodeError>>()?,
        )),
        [Cbor::Uint(2), Cbor::Text(field)] => Ok(WriteTarget::AppMeta(field.clone())),
        [Cbor::Uint(3), Cbor::Text(namespace), Cbor::Text(field)] => {
            Ok(WriteTarget::CustomMeta(namespace.clone(), field.clone()))
        }
        [Cbor::Uint(4)] => Ok(WriteTarget::MimeType),
        _ => Err(err("write target")),
    }
}

fn tree_from(tree: &Cbor) -> Result<cel::Tree, DecodeError> {
    match array_of(tree, "cel tree")? {
        [ty, expr] => Ok(cel::Tree {
            ty: type_from(ty)?,
            root: expr_from(expr)?,
        }),
        _ => Err(err("cel tree")),
    }
}

fn type_from(tree: &Cbor) -> Result<cel::Type, DecodeError> {
    match tree {
        Cbor::Uint(1) => Ok(cel::Type::Bool),
        Cbor::Uint(2) => Ok(cel::Type::Int),
        Cbor::Uint(3) => Ok(cel::Type::Uint),
        Cbor::Uint(4) => Ok(cel::Type::Double),
        Cbor::Uint(5) => Ok(cel::Type::Text),
        Cbor::Uint(6) => Ok(cel::Type::Bytes),
        Cbor::Uint(7) => Ok(cel::Type::Duration),
        Cbor::Uint(8) => Ok(cel::Type::Timestamp),
        Cbor::Uint(9) => Ok(cel::Type::Null),
        Cbor::Uint(10) => Ok(cel::Type::Dyn),
        Cbor::Array(parts) => match parts.as_slice() {
            [Cbor::Uint(11), inner] => {
                Ok(cel::Type::List(alloc::boxed::Box::new(type_from(inner)?)))
            }
            [Cbor::Uint(12), inner] => {
                Ok(cel::Type::Map(alloc::boxed::Box::new(type_from(inner)?)))
            }
            _ => Err(err("cel type")),
        },
        _ => Err(err("cel type")),
    }
}

fn expr_from(tree: &Cbor) -> Result<cel::Expr, DecodeError> {
    let parts = array_of(tree, "cel expr")?;
    match parts {
        [Cbor::Uint(1), value] => Ok(cel::Expr::Literal(cel_value_from(value)?)),
        [Cbor::Uint(2), Cbor::Array(segments)] => Ok(cel::Expr::Path(
            segments
                .iter()
                .map(|seg| Ok(text_of(seg, "path segment")?.to_string()))
                .collect::<Result<Vec<_>, DecodeError>>()?,
        )),
        [Cbor::Uint(3), Cbor::Uint(op), expr] => Ok(cel::Expr::Unary {
            op: match op {
                1 => cel::UnaryOp::Not,
                2 => cel::UnaryOp::Neg,
                _ => return Err(err("unary op")),
            },
            expr: alloc::boxed::Box::new(expr_from(expr)?),
        }),
        [Cbor::Uint(4), Cbor::Uint(op), left, right] => Ok(cel::Expr::Binary {
            op: binop_from(*op)?,
            left: alloc::boxed::Box::new(expr_from(left)?),
            right: alloc::boxed::Box::new(expr_from(right)?),
        }),
        [Cbor::Uint(5), cond, then, otherwise] => Ok(cel::Expr::Ternary {
            cond: alloc::boxed::Box::new(expr_from(cond)?),
            then: alloc::boxed::Box::new(expr_from(then)?),
            otherwise: alloc::boxed::Box::new(expr_from(otherwise)?),
        }),
        [Cbor::Uint(6), base, index] => Ok(cel::Expr::Index {
            base: alloc::boxed::Box::new(expr_from(base)?),
            index: alloc::boxed::Box::new(expr_from(index)?),
        }),
        [Cbor::Uint(7), Cbor::Uint(function), Cbor::Array(args)] => Ok(cel::Expr::Call {
            function: match function {
                1 => cel::Function::Duration,
                2 => cel::Function::Timestamp,
                3 => cel::Function::Size,
                _ => return Err(err("function")),
            },
            args: args.iter().map(expr_from).collect::<Result<Vec<_>, _>>()?,
        }),
        [Cbor::Uint(8), Cbor::Array(items)] => Ok(cel::Expr::List(
            items.iter().map(expr_from).collect::<Result<Vec<_>, _>>()?,
        )),
        [Cbor::Uint(9), Cbor::Array(pairs)] => Ok(cel::Expr::MapLit(
            pairs
                .iter()
                .map(|pair| match array_of(pair, "map literal entry")? {
                    [Cbor::Text(key), value] => Ok((key.clone(), expr_from(value)?)),
                    _ => Err(err("map literal entry")),
                })
                .collect::<Result<Vec<_>, _>>()?,
        )),
        _ => Err(err("cel expr")),
    }
}

fn binop_from(code: u64) -> Result<cel::BinaryOp, DecodeError> {
    Ok(match code {
        1 => cel::BinaryOp::Mul,
        2 => cel::BinaryOp::Div,
        3 => cel::BinaryOp::Rem,
        4 => cel::BinaryOp::Add,
        5 => cel::BinaryOp::Sub,
        6 => cel::BinaryOp::Eq,
        7 => cel::BinaryOp::Ne,
        8 => cel::BinaryOp::Lt,
        9 => cel::BinaryOp::Le,
        10 => cel::BinaryOp::Gt,
        11 => cel::BinaryOp::Ge,
        12 => cel::BinaryOp::And,
        13 => cel::BinaryOp::Or,
        _ => return Err(err("binary op")),
    })
}

/// Reads a `tagged_value` back into a CEL value — the inverse of the
/// canon's `cel_tagged`, over the tags the plan can carry.
fn cel_value_from(tree: &Cbor) -> Result<cel::Value, DecodeError> {
    match array_of(tree, "tagged value")? {
        [Cbor::Uint(1), Cbor::Bool(v)] => Ok(cel::Value::Bool(*v)),
        [Cbor::Uint(2), value] | [Cbor::Uint(3), value] => {
            // Tags 2 (int) and 3 (long) are both the signed domain —
            // the registry keeps them distinct so the seal shows the
            // honest width; the decoded value is one i64 either way.
            Ok(cel::Value::Int(int_from(value)?))
        }
        [Cbor::Uint(5), Cbor::Double(v)] => Ok(cel::Value::Double(v.value())),
        [Cbor::Uint(7), Cbor::Bytes(v)] => Ok(cel::Value::Bytes(v.clone())),
        [Cbor::Uint(8), Cbor::Text(v)] => Ok(cel::Value::Text(v.clone())),
        [Cbor::Uint(9), Cbor::Null] => Ok(cel::Value::Null),
        [Cbor::Uint(23), Cbor::Uint(v)] => Ok(cel::Value::Uint(*v)),
        [Cbor::Uint(12), value] => Ok(cel::Value::Timestamp(int_from(value)?)),
        [Cbor::Uint(16), value] => Ok(cel::Value::Duration(int_from(value)?)),
        [Cbor::Uint(18), Cbor::Array(pairs)] => {
            let mut map = alloc::collections::BTreeMap::new();
            for pair in pairs {
                match array_of(pair, "map entry")? {
                    [key, value] => {
                        let key = match array_of(key, "map key")? {
                            [Cbor::Uint(8), Cbor::Text(text)] => text.clone(),
                            _ => return Err(err("map key")),
                        };
                        map.insert(key, cel_value_from(value)?);
                    }
                    _ => return Err(err("map entry")),
                }
            }
            Ok(cel::Value::Map(map))
        }
        [Cbor::Uint(19), Cbor::Array(items)] => Ok(cel::Value::List(
            items
                .iter()
                .map(cel_value_from)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        _ => Err(err("tagged value")),
    }
}

fn int_from(tree: &Cbor) -> Result<i64, DecodeError> {
    match tree {
        Cbor::Uint(v) => i64::try_from(*v).map_err(|_| err("integer range")),
        // Major type 1 carries −1−n; the tables bound their integers
        // to i64, so an argument beyond it refuses HERE — a table
        // policy, not a byte-discipline rule.
        Cbor::Negative(n) => i64::try_from(*n)
            .map(|magnitude| -1 - magnitude)
            .map_err(|_| err("integer range")),
        _ => Err(err("integer")),
    }
}

// ===================== small readers =====================

fn map_of<'a>(tree: &'a Cbor, what: &str) -> Result<&'a BTreeMap<u64, Cbor>, DecodeError> {
    match tree {
        Cbor::Map(pairs) => Ok(pairs),
        _ => Err(DecodeError::Malformed(format!("{what}: not a map"))),
    }
}

fn array_of<'a>(tree: &'a Cbor, what: &str) -> Result<&'a [Cbor], DecodeError> {
    match tree {
        Cbor::Array(items) => Ok(items),
        _ => Err(DecodeError::Malformed(format!("{what}: not an array"))),
    }
}

fn get<'a>(pairs: &'a BTreeMap<u64, Cbor>, key: u64, what: &str) -> Result<&'a Cbor, DecodeError> {
    find(pairs, key).ok_or_else(|| DecodeError::Malformed(format!("{what}: missing")))
}

fn find(pairs: &BTreeMap<u64, Cbor>, key: u64) -> Option<&Cbor> {
    pairs.get(&key)
}

fn text_of<'a>(tree: &'a Cbor, what: &str) -> Result<&'a str, DecodeError> {
    match tree {
        Cbor::Text(text) => Ok(text),
        _ => Err(DecodeError::Malformed(format!("{what}: not text"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;

    /// The actual deepest level of a built tree — the ground truth
    /// the walker's arithmetic is held to.
    fn levels(value: &Cbor) -> usize {
        let mut max = 0;
        let mut stack = vec![(value, 0usize)];
        while let Some((value, level)) = stack.pop() {
            max = max.max(level);
            match value {
                Cbor::Array(items) => {
                    for item in items {
                        stack.push((item, level + 1));
                    }
                }
                Cbor::Map(pairs) => {
                    for entry in pairs.values() {
                        // The keys are nodes one down too.
                        max = max.max(level + 1);
                        stack.push((entry, level + 1));
                    }
                }
                _ => {}
            }
        }
        max
    }

    fn text(name: &str) -> String {
        String::from(name)
    }

    fn port(name: &str) -> node::Port {
        node::Port::new(name).unwrap()
    }

    fn tree(root: cel::Expr) -> cel::Tree {
        cel::Tree {
            ty: cel::Type::Dyn,
            root,
        }
    }

    /// One plan exercising every construct the builders encode.
    fn rich_plan() -> ExecutionPlan {
        let map_value = {
            let mut entries = BTreeMap::new();
            entries.insert(
                text("k"),
                cel::Value::List(vec![
                    cel::Value::Int(-3),
                    cel::Value::Bytes(vec![1]),
                    cel::Value::Double(1.5),
                    cel::Value::Timestamp(7),
                ]),
            );
            cel::Value::Map(entries)
        };
        let expr = cel::Expr::Ternary {
            cond: Box::new(cel::Expr::Binary {
                op: cel::BinaryOp::Lt,
                left: Box::new(cel::Expr::Path(vec![text("seen")])),
                right: Box::new(cel::Expr::Literal(cel::Value::Int(3))),
            }),
            then: Box::new(cel::Expr::Call {
                function: cel::Function::Size,
                args: vec![cel::Expr::List(vec![cel::Expr::MapLit(vec![(
                    text("k"),
                    cel::Expr::Unary {
                        op: cel::UnaryOp::Neg,
                        expr: Box::new(cel::Expr::Literal(cel::Value::Int(1))),
                    },
                )])])],
            }),
            otherwise: Box::new(cel::Expr::Index {
                base: Box::new(cel::Expr::Path(vec![text("data"), text("f")])),
                index: Box::new(cel::Expr::Literal(cel::Value::Uint(0))),
            }),
        };
        ExecutionPlan {
            nodes: vec![
                Node {
                    path: node::Path::new(["relay"]).unwrap(),
                    kind: NodeKind::Operator,
                    ports: vec![(port("extra"), node::Direction::Output)],
                    vars: vec![VarCell {
                        name: text("seen"),
                        ty: cel::Type::Map(Box::new(cel::Type::List(Box::new(cel::Type::Int)))),
                        init: Some(map_value.clone()),
                    }],
                },
                Node {
                    path: node::Path::new(["shell"]).unwrap(),
                    kind: NodeKind::Connector(ConnectorSpec {
                        contract: text("Process"),
                        effect: node::Effect::Class(node::EffectClass::Idempotent),
                        behavior: node::Behavior::Deterministic,
                        attributes: vec![
                            (text("cmd"), AttrSetting::Value(map_value)),
                            (text("codec"), AttrSetting::Word(text("lines"))),
                        ],
                        error_set: vec![text("A"), text("B")],
                    }),
                    ports: vec![],
                    vars: vec![],
                },
            ],
            boundary: vec![(port("feed"), node::Direction::Output)],
            edges: vec![
                Edge {
                    from: EndpointRef {
                        node: None,
                        port: port("in"),
                    },
                    to: Some(EndpointRef {
                        node: Some(Slot::new(0)),
                        port: port("in"),
                    }),
                },
                Edge {
                    from: EndpointRef {
                        node: Some(Slot::new(0)),
                        port: port("extra"),
                    },
                    to: None,
                },
            ],
            handlers: vec![Handler {
                node: Slot::new(0),
                activation: node::HandlerActivation::Port(port("in")),
                guard: Some(tree(expr.clone())),
                body: vec![
                    Step::Let {
                        name: text("x"),
                        expr: tree(expr.clone()),
                    },
                    Step::SetVar {
                        name: text("seen"),
                        expr: tree(expr.clone()),
                    },
                    Step::Emit {
                        port: port("out"),
                        value: Emission::Identity,
                    },
                    Step::Emit {
                        port: port("out"),
                        value: Emission::Nothing,
                    },
                    Step::Emit {
                        port: port("out"),
                        value: Emission::Data(tree(expr.clone())),
                    },
                    Step::Emit {
                        port: port("out"),
                        value: Emission::Patch(vec![
                            FieldWrite {
                                target: WriteTarget::Data(vec![text("a"), text("b")]),
                                expr: tree(expr),
                            },
                            FieldWrite {
                                target: WriteTarget::AppMeta(text("m")),
                                expr: tree(cel::Expr::Literal(cel::Value::Null)),
                            },
                            FieldWrite {
                                target: WriteTarget::CustomMeta(text("ns"), text("f")),
                                expr: tree(cel::Expr::Literal(cel::Value::Bool(true))),
                            },
                            FieldWrite {
                                target: WriteTarget::MimeType,
                                expr: tree(cel::Expr::Literal(cel::Value::Text(text("t")))),
                            },
                        ]),
                    },
                    Step::Emit {
                        port: port("out"),
                        value: Emission::Replace(vec![FieldWrite {
                            target: WriteTarget::Data(vec![]),
                            expr: tree(cel::Expr::Literal(cel::Value::Duration(5))),
                        }]),
                    },
                ],
            }],
        }
    }

    /// A one-step plan carrying one expression — the chassis for the
    /// nested chains.
    fn one_step_plan(root: cel::Expr) -> ExecutionPlan {
        ExecutionPlan {
            nodes: vec![Node {
                path: node::Path::new(["o"]).unwrap(),
                kind: NodeKind::Operator,
                ports: vec![],
                vars: vec![],
            }],
            boundary: vec![],
            edges: vec![],
            handlers: vec![Handler {
                node: Slot::new(0),
                activation: node::HandlerActivation::Port(port("in")),
                guard: None,
                body: vec![Step::Let {
                    name: text("x"),
                    expr: tree(root),
                }],
            }],
        }
    }

    #[test]
    fn the_level_walker_matches_the_builders() {
        let mut nested_binary = cel::Expr::Literal(cel::Value::Int(1));
        let mut nested_maplit = cel::Expr::Literal(cel::Value::Int(1));
        for _ in 0..60 {
            nested_binary = cel::Expr::Binary {
                op: cel::BinaryOp::Add,
                left: Box::new(nested_binary),
                right: Box::new(cel::Expr::Literal(cel::Value::Int(0))),
            };
            nested_maplit = cel::Expr::MapLit(vec![(text("k"), nested_maplit)]);
        }
        let mut nested_value = cel::Value::Int(1);
        let mut nested_type = cel::Type::Int;
        for _ in 0..60 {
            nested_value = cel::Value::List(vec![nested_value]);
            nested_type = cel::Type::Map(Box::new(nested_type));
        }
        let mut deep_type_plan = one_step_plan(cel::Expr::Literal(cel::Value::Null));
        deep_type_plan.handlers[0].body = vec![Step::Let {
            name: text("x"),
            expr: cel::Tree {
                ty: nested_type,
                root: cel::Expr::Literal(cel::Value::Null),
            },
        }];
        let plans = [
            ExecutionPlan {
                nodes: vec![],
                boundary: vec![],
                edges: vec![],
                handlers: vec![],
            },
            rich_plan(),
            one_step_plan(nested_binary),
            one_step_plan(nested_maplit),
            one_step_plan(cel::Expr::Literal(nested_value)),
            deep_type_plan,
        ];
        for plan in &plans {
            assert_eq!(
                deepest_level(plan),
                levels(&plan_tree(plan)),
                "the walker's arithmetic drifted from the builders"
            );
        }
    }
}
