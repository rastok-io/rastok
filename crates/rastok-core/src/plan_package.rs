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

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::{format, vec};

use crate::canon::{self, Cbor, cel_tagged};
use crate::cel;
use crate::envelope::WriteTarget;
use crate::node;
use crate::plan::{
    AttrSetting, ConnectorSpec, Edge, Emission, EndpointRef, ExecutionPlan, FieldWrite, Handler,
    Node, NodeKind, SemanticHash, Slot, Step, VarCell,
};

/// The package's leading magic.
pub const MAGIC: [u8; 4] = *b"RKPB";

/// The format version this module writes and reads.
pub const FORMAT_VERSION: u32 = 2;

/// The checksum domain — fixed-width framing, so the covered fields
/// cannot slide into one another.
const CHECKSUM_DOMAIN: &[u8] = b"rastok.plan-package\0";

/// A typed refusal from the package decoder. Total: nothing is
/// guessed, nothing is migrated silently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanPackageError {
    /// The bytes do not start with the package magic.
    BadMagic,
    /// A format version this build does not read.
    UnknownVersion(u32),
    /// The blake3 checksum does not match the covered fields.
    BrokenChecksum,
    /// The canonical reader refused, with its own category — a
    /// truncated input and a syntactically foreign one are different
    /// diagnoses and stay different.
    Decode(canon::DecodeError),
    /// Bytes inside the format's frame that its tables cannot read,
    /// or bytes that read but are not the canonical encoding.
    Malformed(String),
    /// A decoded plan that violates the ExecutionPlan invariants —
    /// well-shaped bytes naming impossible structure.
    InvalidPlan(String),
}

impl core::fmt::Display for PlanPackageError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadMagic => f.write_str("not a plan package (bad magic)"),
            Self::UnknownVersion(version) => {
                write!(
                    f,
                    "plan-package format v{version} is not readable by this build"
                )
            }
            Self::BrokenChecksum => f.write_str("the checksum does not match the covered fields"),
            Self::Decode(error) => write!(f, "unreadable canonical bytes: {error}"),
            Self::Malformed(detail) => write!(f, "malformed plan package: {detail}"),
            Self::InvalidPlan(detail) => write!(f, "invalid plan in package: {detail}"),
        }
    }
}

impl core::error::Error for PlanPackageError {}

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

/// Encodes a plan and its identity into a package.
#[must_use]
pub fn encode(plan: &ExecutionPlan, hash: &SemanticHash) -> Vec<u8> {
    let body = canon::encode(&plan_tree(plan));
    let digest = checksum(FORMAT_VERSION, hash.as_bytes(), &body);
    let envelope = Cbor::Map(vec![
        (1, Cbor::Uint(u64::from(FORMAT_VERSION))),
        (2, Cbor::Bytes(hash.as_bytes().to_vec())),
        (3, Cbor::Bytes(body)),
        (4, Cbor::Bytes(digest.to_vec())),
    ]);
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&canon::encode(&envelope));
    out
}

/// Decodes a package back into the plan and its declared identity.
/// Magic, version, and checksum are verified before a body byte is
/// parsed; the parsed plan is then validated structurally, and the
/// whole input must be the canonical encoding of what was read —
/// re-encoding reproduces it byte for byte, or the package refuses.
///
/// # Errors
///
/// A [`PlanPackageError`] naming exactly what refused.
pub fn decode(bytes: &[u8]) -> Result<(ExecutionPlan, SemanticHash), PlanPackageError> {
    let Some(rest) = bytes.strip_prefix(&MAGIC) else {
        return Err(PlanPackageError::BadMagic);
    };
    let (envelope, used) = canon::decode(rest).map_err(PlanPackageError::Decode)?;
    if used != rest.len() {
        return Err(PlanPackageError::Malformed("trailing bytes".into()));
    }
    let Cbor::Map(pairs) = envelope else {
        return Err(PlanPackageError::Malformed(
            "the envelope is not a map".into(),
        ));
    };
    let field = |key: u64| pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| v);
    let Some(Cbor::Uint(version)) = field(1) else {
        return Err(PlanPackageError::Malformed("no format version".into()));
    };
    if *version != u64::from(FORMAT_VERSION) {
        return Err(PlanPackageError::UnknownVersion(
            u32::try_from(*version).unwrap_or(u32::MAX),
        ));
    }
    let Some(Cbor::Bytes(hash)) = field(2) else {
        return Err(PlanPackageError::Malformed("no semantic hash".into()));
    };
    let hash: [u8; 32] = hash
        .as_slice()
        .try_into()
        .map_err(|_| PlanPackageError::Malformed("the hash is not 32 bytes".into()))?;
    let Some(Cbor::Bytes(body)) = field(3) else {
        return Err(PlanPackageError::Malformed("no body".into()));
    };
    let Some(Cbor::Bytes(declared)) = field(4) else {
        return Err(PlanPackageError::Malformed("no checksum".into()));
    };
    let version =
        u32::try_from(*version).unwrap_or_else(|_| unreachable!("equal to FORMAT_VERSION"));
    if declared.as_slice() != checksum(version, &hash, body) {
        return Err(PlanPackageError::BrokenChecksum);
    }
    let (tree, used) = canon::decode(body).map_err(PlanPackageError::Decode)?;
    if used != body.len() {
        return Err(PlanPackageError::Malformed("trailing body bytes".into()));
    }
    let plan = plan_from(&tree)?;
    validate(&plan)?;
    // Canonicity, as one total check: the only accepted bytes for
    // this plan are the ones this module would write. Re-encoding
    // catches non-minimal integers, reordered or duplicated keys,
    // and every other non-canonical variant at once — and the outer
    // envelope is held to the same rule.
    if canon::encode(&plan_tree(&plan)) != *body {
        return Err(PlanPackageError::Malformed(
            "the body is not the canonical encoding of its plan".into(),
        ));
    }
    let canonical_envelope = Cbor::Map(vec![
        (1, Cbor::Uint(u64::from(FORMAT_VERSION))),
        (2, Cbor::Bytes(hash.to_vec())),
        (3, Cbor::Bytes(body.clone())),
        (4, Cbor::Bytes(declared.clone())),
    ]);
    if canon::encode(&canonical_envelope) != rest {
        return Err(PlanPackageError::Malformed(
            "the envelope is not the canonical encoding of its fields".into(),
        ));
    }
    Ok((plan, SemanticHash::from_bytes(hash)))
}

// ===================== validation: parsed → valid =====================

/// Holds a decoded plan to the `ExecutionPlan` invariants the
/// machine assumes: every slot in bounds, every referenced port
/// declared with the right direction, every handler owned, names
/// unique, every state write resolvable. Well-shaped bytes naming
/// impossible structure refuse here, typed — the machine never sees
/// them.
fn validate(plan: &ExecutionPlan) -> Result<(), PlanPackageError> {
    let invalid = |detail: String| Err(PlanPackageError::InvalidPlan(detail));
    // M0 lowers no explicit port table — a node's ports are the
    // implicit trio. A declared table, once present, is the contract.
    let has_port = |node: &Node, port: &node::Port, direction: node::Direction| {
        if node.ports.is_empty() {
            match direction {
                node::Direction::Input => port.name() == "in",
                node::Direction::Output => matches!(port.name(), "out" | "err"),
            }
        } else {
            node.ports
                .iter()
                .any(|(candidate, dir)| candidate == port && *dir == direction)
        }
    };
    let mut paths = alloc::collections::BTreeSet::new();
    for node in &plan.nodes {
        if !paths.insert(node.path.to_string()) {
            return invalid(format!("duplicate node path `{}`", node.path));
        }
        let mut ports = alloc::collections::BTreeSet::new();
        for (port, _) in &node.ports {
            if !ports.insert(port.to_string()) {
                return invalid(format!("duplicate port `{port}` on `{}`", node.path));
            }
        }
        let mut vars = alloc::collections::BTreeSet::new();
        for var in &node.vars {
            if !vars.insert(var.name.as_str()) {
                return invalid(format!("duplicate var `{}` on `{}`", var.name, node.path));
            }
        }
    }
    let node_at = |slot: Slot| plan.nodes.get(slot.index());
    let endpoint_ok =
        |endpoint: &EndpointRef, direction: node::Direction| -> Result<(), PlanPackageError> {
            match endpoint.node {
                Some(slot) => {
                    let Some(node) = node_at(slot) else {
                        return Err(PlanPackageError::InvalidPlan(format!(
                            "edge endpoint names slot {} of {} nodes",
                            slot.index(),
                            plan.nodes.len()
                        )));
                    };
                    if !has_port(node, &endpoint.port, direction) {
                        return Err(PlanPackageError::InvalidPlan(format!(
                            "edge endpoint names `{}.{}` which is not a {direction:?} port",
                            node.path, endpoint.port
                        )));
                    }
                    Ok(())
                }
                None => {
                    // M0 leaves the boundary table empty — its ports are
                    // the implicit trio. A declared table, once present,
                    // is the contract instead.
                    let known = if plan.boundary.is_empty() {
                        matches!(endpoint.port.name(), "in" | "out" | "err")
                    } else {
                        plan.boundary.iter().any(|(port, _)| port == &endpoint.port)
                    };
                    if !known {
                        return Err(PlanPackageError::InvalidPlan(format!(
                            "edge endpoint names undeclared boundary port `{}`",
                            endpoint.port
                        )));
                    }
                    Ok(())
                }
            }
        };
    for edge in &plan.edges {
        endpoint_ok(&edge.from, node::Direction::Output)?;
        if let Some(to) = &edge.to {
            endpoint_ok(to, node::Direction::Input)?;
        }
    }
    for handler in &plan.handlers {
        let Some(node) = node_at(handler.node) else {
            return invalid(format!(
                "handler names slot {} of {} nodes",
                handler.node.index(),
                plan.nodes.len()
            ));
        };
        let node::HandlerActivation::Port(port) = &handler.activation;
        if !has_port(node, port, node::Direction::Input) {
            return invalid(format!(
                "handler on `{}` activates on `{port}` which is not an input port",
                node.path
            ));
        }
        for step in &handler.body {
            match step {
                Step::Emit { port, .. } => {
                    if !has_port(node, port, node::Direction::Output) {
                        return invalid(format!(
                            "handler on `{}` emits on `{port}` which is not an output port",
                            node.path
                        ));
                    }
                }
                Step::SetVar { name, .. } => {
                    if !node.vars.iter().any(|var| var.name == *name) {
                        return invalid(format!(
                            "handler on `{}` writes undeclared var `{name}`",
                            node.path
                        ));
                    }
                }
                Step::Let { .. } => {}
            }
        }
    }
    Ok(())
}

// ===================== encoding: plan → tree =====================

fn plan_tree(plan: &ExecutionPlan) -> Cbor {
    Cbor::Map(vec![
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
    ])
}

fn node_tree(node: &Node) -> Cbor {
    let kind = match &node.kind {
        NodeKind::Operator => Cbor::Uint(1),
        NodeKind::Connector(spec) => Cbor::Array(vec![Cbor::Uint(2), spec_tree(spec)]),
    };
    Cbor::Map(vec![
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
    ])
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
    Cbor::Map(vec![
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
    ])
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
    let mut pairs = vec![(1, Cbor::Text(var.name.clone())), (2, type_tree(&var.ty))];
    if let Some(init) = &var.init {
        pairs.push((3, cel_tagged(init)));
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
    let mut pairs = vec![
        (1, Cbor::Uint(handler.node.index() as u64)),
        (2, activation),
    ];
    if let Some(guard) = &handler.guard {
        pairs.push((3, tree_tree(guard)));
    }
    pairs.push((4, Cbor::Array(handler.body.iter().map(step_tree).collect())));
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

fn err(detail: &str) -> PlanPackageError {
    PlanPackageError::Malformed(detail.into())
}

fn plan_from(tree: &Cbor) -> Result<ExecutionPlan, PlanPackageError> {
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

fn node_from(tree: &Cbor) -> Result<Node, PlanPackageError> {
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

fn spec_from(tree: &Cbor) -> Result<ConnectorSpec, PlanPackageError> {
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

fn attr_from(tree: &Cbor) -> Result<AttrSetting, PlanPackageError> {
    match array_of(tree, "attr setting")? {
        [Cbor::Uint(1), Cbor::Text(word)] => Ok(AttrSetting::Word(word.clone())),
        [Cbor::Uint(2), value] => Ok(AttrSetting::Value(cel_value_from(value)?)),
        _ => Err(err("attr setting")),
    }
}

fn port_decl_from(tree: &Cbor) -> Result<(node::Port, node::Direction), PlanPackageError> {
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

fn var_from(tree: &Cbor) -> Result<VarCell, PlanPackageError> {
    let pairs = map_of(tree, "var")?;
    let name = text_of(get(pairs, 1, "var name")?, "var name")?.to_string();
    let ty = type_from(get(pairs, 2, "var type")?)?;
    let init = match find(pairs, 3) {
        Some(value) => Some(cel_value_from(value)?),
        None => None,
    };
    Ok(VarCell { name, ty, init })
}

fn edge_from(tree: &Cbor) -> Result<Edge, PlanPackageError> {
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

fn endpoint_from(tree: &Cbor) -> Result<EndpointRef, PlanPackageError> {
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

fn handler_from(tree: &Cbor) -> Result<Handler, PlanPackageError> {
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

fn step_from(tree: &Cbor) -> Result<Step, PlanPackageError> {
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

fn emission_from(tree: &Cbor) -> Result<Emission, PlanPackageError> {
    match array_of(tree, "emission")? {
        [Cbor::Uint(1)] => Ok(Emission::Identity),
        [Cbor::Uint(2), writes] => Ok(Emission::Patch(writes_from(writes)?)),
        [Cbor::Uint(3), writes] => Ok(Emission::Replace(writes_from(writes)?)),
        [Cbor::Uint(4), tree] => Ok(Emission::Data(tree_from(tree)?)),
        [Cbor::Uint(5)] => Ok(Emission::Nothing),
        _ => Err(err("emission")),
    }
}

fn writes_from(tree: &Cbor) -> Result<Vec<FieldWrite>, PlanPackageError> {
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

fn target_from(tree: &Cbor) -> Result<WriteTarget, PlanPackageError> {
    match array_of(tree, "write target")? {
        [Cbor::Uint(1), Cbor::Array(segments)] => Ok(WriteTarget::Data(
            segments
                .iter()
                .map(|seg| Ok(text_of(seg, "path segment")?.to_string()))
                .collect::<Result<Vec<_>, PlanPackageError>>()?,
        )),
        [Cbor::Uint(2), Cbor::Text(field)] => Ok(WriteTarget::AppMeta(field.clone())),
        [Cbor::Uint(3), Cbor::Text(namespace), Cbor::Text(field)] => {
            Ok(WriteTarget::CustomMeta(namespace.clone(), field.clone()))
        }
        [Cbor::Uint(4)] => Ok(WriteTarget::MimeType),
        _ => Err(err("write target")),
    }
}

fn tree_from(tree: &Cbor) -> Result<cel::Tree, PlanPackageError> {
    match array_of(tree, "cel tree")? {
        [ty, expr] => Ok(cel::Tree {
            ty: type_from(ty)?,
            root: expr_from(expr)?,
        }),
        _ => Err(err("cel tree")),
    }
}

fn type_from(tree: &Cbor) -> Result<cel::Type, PlanPackageError> {
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

fn expr_from(tree: &Cbor) -> Result<cel::Expr, PlanPackageError> {
    let parts = array_of(tree, "cel expr")?;
    match parts {
        [Cbor::Uint(1), value] => Ok(cel::Expr::Literal(cel_value_from(value)?)),
        [Cbor::Uint(2), Cbor::Array(segments)] => Ok(cel::Expr::Path(
            segments
                .iter()
                .map(|seg| Ok(text_of(seg, "path segment")?.to_string()))
                .collect::<Result<Vec<_>, PlanPackageError>>()?,
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

fn binop_from(code: u64) -> Result<cel::BinaryOp, PlanPackageError> {
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
fn cel_value_from(tree: &Cbor) -> Result<cel::Value, PlanPackageError> {
    match array_of(tree, "tagged value")? {
        [Cbor::Uint(1), Cbor::Bool(v)] => Ok(cel::Value::Bool(*v)),
        [Cbor::Uint(2), value] | [Cbor::Uint(3), value] => {
            // Tags 2 (int) and 3 (long) are both the signed domain —
            // the registry keeps them distinct so the seal shows the
            // honest width; the decoded value is one i64 either way.
            Ok(cel::Value::Int(int_from(value)?))
        }
        [Cbor::Uint(5), Cbor::Double(v)] => Ok(cel::Value::Double(*v)),
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

fn int_from(tree: &Cbor) -> Result<i64, PlanPackageError> {
    match tree {
        Cbor::Uint(v) => i64::try_from(*v).map_err(|_| err("integer range")),
        Cbor::Int(v) => Ok(*v),
        _ => Err(err("integer")),
    }
}

// ===================== small readers =====================

fn map_of<'a>(tree: &'a Cbor, what: &str) -> Result<&'a [(u64, Cbor)], PlanPackageError> {
    match tree {
        Cbor::Map(pairs) => Ok(pairs),
        _ => Err(PlanPackageError::Malformed(format!("{what}: not a map"))),
    }
}

fn array_of<'a>(tree: &'a Cbor, what: &str) -> Result<&'a [Cbor], PlanPackageError> {
    match tree {
        Cbor::Array(items) => Ok(items),
        _ => Err(PlanPackageError::Malformed(format!("{what}: not an array"))),
    }
}

fn get<'a>(pairs: &'a [(u64, Cbor)], key: u64, what: &str) -> Result<&'a Cbor, PlanPackageError> {
    find(pairs, key).ok_or_else(|| PlanPackageError::Malformed(format!("{what}: missing")))
}

fn find(pairs: &[(u64, Cbor)], key: u64) -> Option<&Cbor> {
    pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
}

fn text_of<'a>(tree: &'a Cbor, what: &str) -> Result<&'a str, PlanPackageError> {
    match tree {
        Cbor::Text(text) => Ok(text),
        _ => Err(PlanPackageError::Malformed(format!("{what}: not text"))),
    }
}
