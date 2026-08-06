//! The plan blob — the sealed, versioned serialization of an
//! [`ExecutionPlan`], per `docs/architecture/plan-blob.md`.
//!
//! Format v1: the 4-byte magic `RKPB`, then one canonical CBOR map —
//! format version, the plan's semantic hash, the body as an embedded
//! byte string, and a blake3 seal over exactly those body bytes. The
//! decoder verifies magic, version, and seal before parsing a single
//! body byte; every refusal is typed, and an unknown version is a
//! refusal by name, never a silent migration.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::{format, vec};

use crate::canon::{Cbor, cel_tagged, decode, encode};
use crate::cel;
use crate::envelope::WriteTarget;
use crate::node;
use crate::plan::{
    AttrSetting, ConnectorSpec, Edge, Emission, EndpointRef, ExecutionPlan, FieldWrite, Handler,
    Node, NodeKind, SemanticHash, Slot, Step, VarCell,
};

/// The blob's leading magic.
pub const MAGIC: [u8; 4] = *b"RKPB";

/// The format version this module writes and reads.
pub const FORMAT_VERSION: u32 = 1;

/// A typed refusal from the blob decoder. Total: nothing is guessed,
/// nothing is migrated silently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobError {
    /// The bytes do not start with the blob magic.
    BadMagic,
    /// A format version this build does not read.
    UnknownVersion(u32),
    /// The blake3 seal does not match the body bytes.
    BrokenSeal,
    /// The bytes end before the structure does.
    Truncated,
    /// Bytes inside the format's frame that its tables cannot read.
    Malformed(String),
}

impl core::fmt::Display for BlobError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadMagic => f.write_str("not a plan blob (bad magic)"),
            Self::UnknownVersion(version) => {
                write!(
                    f,
                    "plan-blob format v{version} is not readable by this build"
                )
            }
            Self::BrokenSeal => f.write_str("the body does not match its seal"),
            Self::Truncated => f.write_str("the blob ends before its structure does"),
            Self::Malformed(detail) => write!(f, "malformed plan blob: {detail}"),
        }
    }
}

impl core::error::Error for BlobError {}

/// Encodes a plan and its identity into a sealed blob.
#[must_use]
pub fn seal(plan: &ExecutionPlan, hash: &SemanticHash) -> Vec<u8> {
    let body = encode(&plan_tree(plan));
    let mut digest = [0u8; 32];
    let mut hasher = blake3::Hasher::new();
    hasher.update(&body);
    hasher.finalize_xof().fill(&mut digest);
    let envelope = Cbor::Map(vec![
        (1, Cbor::Uint(u64::from(FORMAT_VERSION))),
        (2, Cbor::Bytes(hash.as_bytes().to_vec())),
        (3, Cbor::Bytes(body)),
        (4, Cbor::Bytes(digest.to_vec())),
    ]);
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&encode(&envelope));
    out
}

/// Decodes a sealed blob back into the plan and its declared
/// identity, verifying magic, version, and seal before the body.
///
/// # Errors
///
/// A [`BlobError`] naming exactly what refused.
pub fn open(bytes: &[u8]) -> Result<(ExecutionPlan, SemanticHash), BlobError> {
    let Some(rest) = bytes.strip_prefix(&MAGIC) else {
        return Err(BlobError::BadMagic);
    };
    let Some((envelope, used)) = decode(rest) else {
        return Err(BlobError::Truncated);
    };
    if used != rest.len() {
        return Err(BlobError::Malformed("trailing bytes".into()));
    }
    let Cbor::Map(pairs) = envelope else {
        return Err(BlobError::Malformed("the envelope is not a map".into()));
    };
    let field = |key: u64| pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| v);
    let Some(Cbor::Uint(version)) = field(1) else {
        return Err(BlobError::Malformed("no format version".into()));
    };
    if *version != u64::from(FORMAT_VERSION) {
        return Err(BlobError::UnknownVersion(
            u32::try_from(*version).unwrap_or(u32::MAX),
        ));
    }
    let Some(Cbor::Bytes(hash)) = field(2) else {
        return Err(BlobError::Malformed("no semantic hash".into()));
    };
    let hash: [u8; 32] = hash
        .as_slice()
        .try_into()
        .map_err(|_| BlobError::Malformed("the hash is not 32 bytes".into()))?;
    let Some(Cbor::Bytes(body)) = field(3) else {
        return Err(BlobError::Malformed("no body".into()));
    };
    let Some(Cbor::Bytes(declared)) = field(4) else {
        return Err(BlobError::Malformed("no seal".into()));
    };
    let mut digest = [0u8; 32];
    let mut hasher = blake3::Hasher::new();
    hasher.update(body);
    hasher.finalize_xof().fill(&mut digest);
    if declared.as_slice() != digest {
        return Err(BlobError::BrokenSeal);
    }
    let Some((tree, used)) = decode(body) else {
        return Err(BlobError::Truncated);
    };
    if used != body.len() {
        return Err(BlobError::Malformed("trailing body bytes".into()));
    }
    let plan = plan_from(&tree)?;
    Ok((plan, SemanticHash::from_bytes(hash)))
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

fn err(detail: &str) -> BlobError {
    BlobError::Malformed(detail.into())
}

fn plan_from(tree: &Cbor) -> Result<ExecutionPlan, BlobError> {
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

fn node_from(tree: &Cbor) -> Result<Node, BlobError> {
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

fn spec_from(tree: &Cbor) -> Result<ConnectorSpec, BlobError> {
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

fn attr_from(tree: &Cbor) -> Result<AttrSetting, BlobError> {
    match array_of(tree, "attr setting")? {
        [Cbor::Uint(1), Cbor::Text(word)] => Ok(AttrSetting::Word(word.clone())),
        [Cbor::Uint(2), value] => Ok(AttrSetting::Value(cel_value_from(value)?)),
        _ => Err(err("attr setting")),
    }
}

fn port_decl_from(tree: &Cbor) -> Result<(node::Port, node::Direction), BlobError> {
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

fn var_from(tree: &Cbor) -> Result<VarCell, BlobError> {
    let pairs = map_of(tree, "var")?;
    let name = text_of(get(pairs, 1, "var name")?, "var name")?.to_string();
    let ty = type_from(get(pairs, 2, "var type")?)?;
    let init = match find(pairs, 3) {
        Some(value) => Some(cel_value_from(value)?),
        None => None,
    };
    Ok(VarCell { name, ty, init })
}

fn edge_from(tree: &Cbor) -> Result<Edge, BlobError> {
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

fn endpoint_from(tree: &Cbor) -> Result<EndpointRef, BlobError> {
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

fn handler_from(tree: &Cbor) -> Result<Handler, BlobError> {
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

fn step_from(tree: &Cbor) -> Result<Step, BlobError> {
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

fn emission_from(tree: &Cbor) -> Result<Emission, BlobError> {
    match array_of(tree, "emission")? {
        [Cbor::Uint(1)] => Ok(Emission::Identity),
        [Cbor::Uint(2), writes] => Ok(Emission::Patch(writes_from(writes)?)),
        [Cbor::Uint(3), writes] => Ok(Emission::Replace(writes_from(writes)?)),
        [Cbor::Uint(4), tree] => Ok(Emission::Data(tree_from(tree)?)),
        [Cbor::Uint(5)] => Ok(Emission::Nothing),
        _ => Err(err("emission")),
    }
}

fn writes_from(tree: &Cbor) -> Result<Vec<FieldWrite>, BlobError> {
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

fn target_from(tree: &Cbor) -> Result<WriteTarget, BlobError> {
    match array_of(tree, "write target")? {
        [Cbor::Uint(1), Cbor::Array(segments)] => Ok(WriteTarget::Data(
            segments
                .iter()
                .map(|seg| Ok(text_of(seg, "path segment")?.to_string()))
                .collect::<Result<Vec<_>, BlobError>>()?,
        )),
        [Cbor::Uint(2), Cbor::Text(field)] => Ok(WriteTarget::AppMeta(field.clone())),
        [Cbor::Uint(3), Cbor::Text(namespace), Cbor::Text(field)] => {
            Ok(WriteTarget::CustomMeta(namespace.clone(), field.clone()))
        }
        [Cbor::Uint(4)] => Ok(WriteTarget::MimeType),
        _ => Err(err("write target")),
    }
}

fn tree_from(tree: &Cbor) -> Result<cel::Tree, BlobError> {
    match array_of(tree, "cel tree")? {
        [ty, expr] => Ok(cel::Tree {
            ty: type_from(ty)?,
            root: expr_from(expr)?,
        }),
        _ => Err(err("cel tree")),
    }
}

fn type_from(tree: &Cbor) -> Result<cel::Type, BlobError> {
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

fn expr_from(tree: &Cbor) -> Result<cel::Expr, BlobError> {
    let parts = array_of(tree, "cel expr")?;
    match parts {
        [Cbor::Uint(1), value] => Ok(cel::Expr::Literal(cel_value_from(value)?)),
        [Cbor::Uint(2), Cbor::Array(segments)] => Ok(cel::Expr::Path(
            segments
                .iter()
                .map(|seg| Ok(text_of(seg, "path segment")?.to_string()))
                .collect::<Result<Vec<_>, BlobError>>()?,
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

fn binop_from(code: u64) -> Result<cel::BinaryOp, BlobError> {
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
fn cel_value_from(tree: &Cbor) -> Result<cel::Value, BlobError> {
    match array_of(tree, "tagged value")? {
        [Cbor::Uint(1), Cbor::Bool(v)] => Ok(cel::Value::Bool(*v)),
        [Cbor::Uint(2), value] => Ok(cel::Value::Int(int_from(value)?)),
        [Cbor::Uint(3), value] => match value {
            Cbor::Uint(v) => Ok(cel::Value::Uint(*v)),
            _ => Ok(cel::Value::Int(int_from(value)?)),
        },
        [Cbor::Uint(5), Cbor::Double(v)] => Ok(cel::Value::Double(*v)),
        [Cbor::Uint(7), Cbor::Bytes(v)] => Ok(cel::Value::Bytes(v.clone())),
        [Cbor::Uint(8), Cbor::Text(v)] => Ok(cel::Value::Text(v.clone())),
        [Cbor::Uint(9), Cbor::Null] => Ok(cel::Value::Null),
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

fn int_from(tree: &Cbor) -> Result<i64, BlobError> {
    match tree {
        Cbor::Uint(v) => i64::try_from(*v).map_err(|_| err("integer range")),
        Cbor::Int(v) => Ok(*v),
        _ => Err(err("integer")),
    }
}

// ===================== small readers =====================

fn map_of<'a>(tree: &'a Cbor, what: &str) -> Result<&'a [(u64, Cbor)], BlobError> {
    match tree {
        Cbor::Map(pairs) => Ok(pairs),
        _ => Err(BlobError::Malformed(format!("{what}: not a map"))),
    }
}

fn array_of<'a>(tree: &'a Cbor, what: &str) -> Result<&'a [Cbor], BlobError> {
    match tree {
        Cbor::Array(items) => Ok(items),
        _ => Err(BlobError::Malformed(format!("{what}: not an array"))),
    }
}

fn get<'a>(pairs: &'a [(u64, Cbor)], key: u64, what: &str) -> Result<&'a Cbor, BlobError> {
    find(pairs, key).ok_or_else(|| BlobError::Malformed(format!("{what}: missing")))
}

fn find(pairs: &[(u64, Cbor)], key: u64) -> Option<&Cbor> {
    pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
}

fn text_of<'a>(tree: &'a Cbor, what: &str) -> Result<&'a str, BlobError> {
    match tree {
        Cbor::Text(text) => Ok(text),
        _ => Err(BlobError::Malformed(format!("{what}: not text"))),
    }
}
