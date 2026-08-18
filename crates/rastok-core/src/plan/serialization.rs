//! The plan package — the transportable, verifiable serialization
//! of an [`ExecutionPlan`], per `docs/architecture/plan-package.md`.
//!
//! Format v8: the 4-byte magic `RKPB`, then one canonical CBOR map —
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
use crate::datatype::{
    ArrayType, BinaryType, ByteLength, DataType, DateUnit, DecimalType, Discriminator, EnumType,
    FloatType, IntegerType, IntegerWidth, IntervalUnit, MapType, StructType, TaggedCase,
    TaggedUnionType, TemporalType, TimeUnit, TimeZone, TypeField,
};
use crate::envelope::WriteTarget;
use crate::expr;
use crate::identity::encoding::value_tagged;
use crate::node;
use crate::plan::{
    AttrSetting, ConnectorSpec, Edge, Emission, EndpointRef, ExecutionPlan, FieldWrite, Handler,
    Node, NodeKind, PartitionLifetime, PartitionSpec, PlanValidationError, PortDecl, SemanticHash,
    Slot, Step, ValidatedExecutionPlan, VarCell,
};
use crate::value::Value;

/// The package's leading magic.
pub const MAGIC: [u8; 4] = *b"RKPB";

/// The format version this module writes and reads. V7 publishes the complete
/// logical type, materialized-value and expression registries together with
/// the mandatory resolved-port table. A new format is a new version, and an
/// old decoder refuses it by name, exactly as the format note promises.
pub const FORMAT_VERSION: u32 = 8;

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
    /// The transported effective-port table disagrees with the table derived
    /// from the plan's own constraints.
    ResolvedPortTableMismatch,
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
            Self::ResolvedPortTableMismatch => {
                f.write_str("the resolved port table disagrees with the plan")
            }
        }
    }
}

impl core::error::Error for DecodeError {}

/// The checksum: blake3 over the domain-framed association.
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
    let body = canonical_cbor::encode(&validated_plan_tree(plan));
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
    let body_fields = map_of(&tree, "plan")?;
    let resolved_claim = resolved_table_from(get(body_fields, 8, "resolved port table")?)?;
    let plan = plan_from(&tree)?;
    let validated = ValidatedExecutionPlan::try_new(plan).map_err(DecodeError::Invalid)?;
    if resolved_claim != resolved_table_tree(validated.resolved_ports()) {
        return Err(DecodeError::ResolvedPortTableMismatch);
    }
    // Canonicity, as one total check: the only accepted bytes for
    // this plan are the ones this module would write. Re-encoding
    // catches non-minimal integers, reordered or duplicated keys,
    // and every other non-canonical variant at once — and the outer
    // envelope is held to the same rule.
    if canonical_cbor::encode(&validated_plan_tree(&validated)) != *body {
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

fn validated_plan_tree(plan: &ValidatedExecutionPlan) -> Cbor {
    plan_tree_with_resolved(plan.plan(), Some(plan.resolved_ports()))
}

#[cfg(test)]
fn plan_tree(plan: &ExecutionPlan) -> Cbor {
    plan_tree_with_resolved(plan, None)
}

fn plan_tree_with_resolved(
    plan: &ExecutionPlan,
    resolved: Option<&super::ResolvedPortTable>,
) -> Cbor {
    let mut pairs = BTreeMap::from([
        (1, Cbor::Array(plan.nodes.iter().map(node_tree).collect())),
        (
            2,
            Cbor::Array(plan.boundary.iter().map(port_decl_tree).collect()),
        ),
        (3, Cbor::Array(plan.edges.iter().map(edge_tree).collect())),
        (
            4,
            Cbor::Array(plan.handlers.iter().map(handler_tree).collect()),
        ),
    ]);
    // The travelling zone schemas (v5, D-2026-08-10-1): the
    // validator answers without a Registry. Absent = omitted key.
    if !plan.app_schema.is_empty() {
        pairs.insert(5, schema_tree(&plan.app_schema));
    }
    if !plan.ext_schema.is_empty() {
        pairs.insert(6, schema_tree(&plan.ext_schema));
    }
    if !plan.assets.is_empty() {
        // Content only: the digest is DERIVED and re-verified at
        // decode — a tampered blob cannot keep its identity.
        pairs.insert(
            7,
            Cbor::Array(
                plan.assets
                    .iter()
                    .map(|blob| Cbor::Bytes(blob.bytes.clone()))
                    .collect(),
            ),
        );
    }
    if let Some(resolved) = resolved {
        pairs.insert(8, resolved_table_tree(resolved));
    }
    Cbor::Map(pairs)
}

fn resolved_table_tree(table: &super::ResolvedPortTable) -> Cbor {
    let row = |ports: &[super::ResolvedPort]| {
        Cbor::Array(
            ports
                .iter()
                .map(|port| {
                    Cbor::Array(vec![
                        Cbor::Text(port.port().name().to_string()),
                        Cbor::Uint(match port.direction() {
                            node::Direction::Input => 1,
                            node::Direction::Output => 2,
                        }),
                        type_tree(port.ty()),
                        Cbor::Uint(match port.origin() {
                            super::TypeOrigin::Declared => 1,
                            super::TypeOrigin::Inferred => 2,
                        }),
                    ])
                })
                .collect(),
        )
    };
    Cbor::Array(vec![
        row(table.boundary()),
        Cbor::Array(
            (0..)
                .map(Slot::new)
                .map_while(|slot| table.node(slot))
                .map(row)
                .collect(),
        ),
    ])
}

fn resolved_table_from(tree: &Cbor) -> Result<Cbor, DecodeError> {
    fn row(tree: &Cbor) -> Result<Cbor, DecodeError> {
        let mut decoded = Vec::new();
        for port in array_of(tree, "resolved port row")? {
            let [
                Cbor::Text(name),
                Cbor::Uint(direction),
                ty,
                Cbor::Uint(origin),
            ] = array_of(port, "resolved port")?
            else {
                return Err(err("resolved port"));
            };
            node::Port::new(name.clone()).map_err(|_| err("resolved port name"))?;
            if !matches!(direction, 1 | 2) {
                return Err(err("resolved port direction"));
            }
            if !matches!(origin, 1 | 2) {
                return Err(err("resolved port origin"));
            }
            decoded.push(Cbor::Array(vec![
                Cbor::Text(name.clone()),
                Cbor::Uint(*direction),
                type_tree(&type_from(ty)?),
                Cbor::Uint(*origin),
            ]));
        }
        Ok(Cbor::Array(decoded))
    }

    let [boundary, nodes] = array_of(tree, "resolved port table")? else {
        return Err(err("resolved port table"));
    };
    Ok(Cbor::Array(vec![
        row(boundary)?,
        Cbor::Array(
            array_of(nodes, "resolved node port tables")?
                .iter()
                .map(row)
                .collect::<Result<Vec<_>, _>>()?,
        ),
    ]))
}

fn schema_tree(schema: &[(String, DataType)]) -> Cbor {
    Cbor::Array(
        schema
            .iter()
            .map(|(field, ty)| Cbor::Array(vec![Cbor::Text(field.clone()), type_tree(ty)]))
            .collect(),
    )
}

fn node_tree(node: &Node) -> Cbor {
    let kind = match &node.kind {
        NodeKind::Operator => Cbor::Uint(1),
        NodeKind::Connector(spec) => Cbor::Array(vec![Cbor::Uint(2), spec_tree(spec)]),
    };
    let mut fields = BTreeMap::from([
        (1, Cbor::Text(node.path.to_string())),
        (2, kind),
        (
            3,
            Cbor::Array(node.ports.iter().map(port_decl_tree).collect()),
        ),
        (4, Cbor::Array(node.vars.iter().map(var_tree).collect())),
    ]);
    if let Some(partition) = &node.partition {
        fields.insert(5, partition_tree(partition));
    }
    Cbor::Map(fields)
}

fn partition_tree(partition: &PartitionSpec) -> Cbor {
    Cbor::Array(vec![
        partition
            .name
            .as_ref()
            .map_or(Cbor::Null, |name| Cbor::Text(name.clone())),
        tree_tree(&partition.key),
        match partition.lifetime {
            PartitionLifetime::Persistent => Cbor::Array(vec![Cbor::Uint(1)]),
            PartitionLifetime::IdleTtl(nanos) => {
                Cbor::Array(vec![Cbor::Uint(2), signed_tree(nanos)])
            }
        },
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
    ]))
}

fn attr_tree(setting: &AttrSetting) -> Cbor {
    match setting {
        AttrSetting::Word(word) => Cbor::Array(vec![Cbor::Uint(1), Cbor::Text(word.clone())]),
        AttrSetting::Value(value) => Cbor::Array(vec![Cbor::Uint(2), value_tagged(value)]),
    }
}

fn port_decl_tree(decl: &PortDecl) -> Cbor {
    let mut parts = vec![
        Cbor::Text(decl.port.name().to_string()),
        Cbor::Uint(match (decl.direction, decl.unwired) {
            (node::Direction::Input, super::UnwiredPolicy::RequireWire) => 1,
            (node::Direction::Output, super::UnwiredPolicy::RequireWire) => 2,
            (node::Direction::Output, super::UnwiredPolicy::RootSink) => 3,
            (node::Direction::Input, super::UnwiredPolicy::RootSink) => 4,
        }),
    ];
    // The optional third element is the pinned type — ABSENT when
    // unannotated, so every pre-typed-ports package is byte-identical
    // and canonical form stays one form per table.
    if let Some(ty) = &decl.ty {
        parts.push(type_tree(ty));
    }
    Cbor::Array(parts)
}

fn var_tree(var: &VarCell) -> Cbor {
    let mut pairs = BTreeMap::from([(1, Cbor::Text(var.name.clone())), (2, type_tree(&var.ty))]);
    if let Some(init) = &var.init {
        pairs.insert(3, value_tagged(init));
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
    let activation = match &handler.activation {
        node::HandlerActivation::Port(port) => {
            Cbor::Array(vec![Cbor::Uint(1), Cbor::Text(port.name().to_string())])
        }
        node::HandlerActivation::Lifecycle(lifecycle) => {
            Cbor::Array(vec![Cbor::Uint(2), Cbor::Uint(lifecycle_tag(*lifecycle))])
        }
    };
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

const fn lifecycle_tag(lifecycle: node::Lifecycle) -> u64 {
    match lifecycle {
        node::Lifecycle::Start => 1,
        node::Lifecycle::Stop => 2,
        node::Lifecycle::Compensate => 3,
        node::Lifecycle::Next => 4,
        node::Lifecycle::Timeout => 5,
        node::Lifecycle::Evict => 6,
        node::Lifecycle::Recover => 7,
    }
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
        // Tags 3 (custom-namespace) and 4 (mime) are RETIRED with
        // D-2026-08-10-1/-4 — extend-only: never reused.
        WriteTarget::App(field) => Cbor::Array(vec![Cbor::Uint(2), Cbor::Text(field.clone())]),
        WriteTarget::Ext(field) => Cbor::Array(vec![Cbor::Uint(5), Cbor::Text(field.clone())]),
    }
}

fn tree_tree(tree: &expr::Tree) -> Cbor {
    Cbor::Array(vec![type_tree(&tree.ty), expr_tree(&tree.root)])
}

fn type_tree(ty: &DataType) -> Cbor {
    match ty {
        DataType::Null => Cbor::Uint(1),
        DataType::Bool => Cbor::Uint(2),
        DataType::Integer(integer) => {
            let (signedness, width) = match integer {
                IntegerType::Signed(width) => (1, width.bits()),
                IntegerType::Unsigned(width) => (2, width.bits()),
            };
            Cbor::Array(vec![
                Cbor::Uint(3),
                Cbor::Uint(signedness),
                Cbor::Uint(u64::from(width)),
            ])
        }
        DataType::Float(float) => {
            Cbor::Array(vec![Cbor::Uint(4), Cbor::Uint(u64::from(float.bits()))])
        }
        DataType::Decimal(decimal) => Cbor::Array(vec![
            Cbor::Uint(5),
            Cbor::Uint(u64::from(decimal.precision())),
            signed_tree(i64::from(decimal.scale())),
        ]),
        DataType::Number => Cbor::Uint(6),
        DataType::Text => Cbor::Uint(7),
        DataType::Bytes(binary) => Cbor::Array(vec![
            Cbor::Uint(8),
            match binary {
                BinaryType::Variable => Cbor::Null,
                BinaryType::Fixed(length) => Cbor::Uint(u64::from(length.get())),
            },
        ]),
        DataType::Temporal(temporal) => temporal_type_tree(temporal),
        DataType::Uuid => Cbor::Uint(10),
        DataType::Struct(structure) => {
            Cbor::Array(vec![Cbor::Uint(11), struct_fields_tree(structure)])
        }
        DataType::Map(map) => Cbor::Array(vec![
            Cbor::Uint(12),
            type_tree(map.key()),
            type_tree(map.value()),
        ]),
        DataType::Array(array) => Cbor::Array(vec![
            Cbor::Uint(13),
            type_tree(array.element()),
            array
                .length()
                .map_or(Cbor::Null, |length| Cbor::Uint(u64::from(length.get()))),
        ]),
        DataType::Enum(enumeration) => Cbor::Array(vec![
            Cbor::Uint(14),
            Cbor::Array(
                enumeration
                    .symbols()
                    .iter()
                    .map(|symbol| Cbor::Text(symbol.clone()))
                    .collect(),
            ),
        ]),
        DataType::StructuralUnion(union) => Cbor::Array(vec![
            Cbor::Uint(15),
            Cbor::Array(union.branches().iter().map(type_tree).collect()),
        ]),
        DataType::TaggedUnion(union) => Cbor::Array(vec![
            Cbor::Uint(16),
            Cbor::Array(
                union
                    .cases()
                    .iter()
                    .map(|case| {
                        Cbor::Array(vec![
                            Cbor::Text(case.name().to_string()),
                            struct_fields_tree(case.payload()),
                        ])
                    })
                    .collect(),
            ),
        ]),
        DataType::Variant => Cbor::Uint(17),
    }
}

fn struct_fields_tree(structure: &StructType) -> Cbor {
    Cbor::Array(
        structure
            .fields()
            .iter()
            .map(|field| {
                Cbor::Array(vec![
                    Cbor::Text(field.name().to_string()),
                    type_tree(field.ty()),
                ])
            })
            .collect(),
    )
}

fn temporal_type_tree(temporal: &TemporalType) -> Cbor {
    let time_unit = |unit: TimeUnit| {
        Cbor::Uint(match unit {
            TimeUnit::Second => 1,
            TimeUnit::Millisecond => 2,
            TimeUnit::Microsecond => 3,
            TimeUnit::Nanosecond => 4,
        })
    };
    let mut parts = vec![Cbor::Uint(9)];
    match temporal {
        TemporalType::Date(unit) => {
            parts.push(Cbor::Uint(1));
            parts.push(Cbor::Uint(match unit {
                DateUnit::Day => 1,
                DateUnit::Millisecond => 2,
            }));
        }
        TemporalType::Time(unit) => {
            parts.push(Cbor::Uint(2));
            parts.push(time_unit(*unit));
        }
        TemporalType::Timestamp { unit, zone } => {
            parts.push(Cbor::Uint(3));
            parts.push(time_unit(*unit));
            parts.push(match zone {
                TimeZone::Utc => Cbor::Null,
                TimeZone::Named(name) => Cbor::Text(name.as_str().to_string()),
            });
        }
        TemporalType::TimestampNtz(unit) => {
            parts.push(Cbor::Uint(4));
            parts.push(time_unit(*unit));
        }
        TemporalType::Duration(unit) => {
            parts.push(Cbor::Uint(5));
            parts.push(time_unit(*unit));
        }
        TemporalType::Interval(unit) => {
            parts.push(Cbor::Uint(6));
            parts.push(Cbor::Uint(match unit {
                IntervalUnit::YearMonth => 1,
                IntervalUnit::DayTime => 2,
                IntervalUnit::MonthDayNano => 3,
                IntervalUnit::MonthDayMillisUnsigned => 4,
            }));
        }
    }
    Cbor::Array(parts)
}

fn expr_tree(expr: &expr::Expr) -> Cbor {
    match expr {
        expr::Expr::Literal(value) => Cbor::Array(vec![Cbor::Uint(1), value_tagged(value)]),
        expr::Expr::Asset(index) => {
            Cbor::Array(vec![Cbor::Uint(10), Cbor::Uint(u64::from(*index))])
        }
        expr::Expr::Path(segments) => Cbor::Array(vec![
            Cbor::Uint(2),
            Cbor::Array(segments.iter().map(|seg| Cbor::Text(seg.clone())).collect()),
        ]),
        expr::Expr::Unary { op, expr } => Cbor::Array(vec![
            Cbor::Uint(3),
            Cbor::Uint(match op {
                expr::UnaryOp::Not => 1,
                expr::UnaryOp::Neg => 2,
            }),
            expr_tree(expr),
        ]),
        expr::Expr::Binary { op, left, right } => Cbor::Array(vec![
            Cbor::Uint(4),
            Cbor::Uint(binop_code(*op)),
            expr_tree(left),
            expr_tree(right),
        ]),
        expr::Expr::Ternary {
            cond,
            then,
            otherwise,
        } => Cbor::Array(vec![
            Cbor::Uint(5),
            expr_tree(cond),
            expr_tree(then),
            expr_tree(otherwise),
        ]),
        expr::Expr::Index { base, index } => {
            Cbor::Array(vec![Cbor::Uint(6), expr_tree(base), expr_tree(index)])
        }
        expr::Expr::Call { function, args } => Cbor::Array(vec![
            Cbor::Uint(7),
            Cbor::Uint(match function {
                // `asset(…)` must be rebound to `Expr::Asset` before plan
                // validation. This code is deliberately decodable so a forged
                // tree reaches the categorical validator refusal.
                expr::Function::Asset => 5,
                expr::Function::Duration => 1,
                expr::Function::Timestamp => 2,
                expr::Function::Size => 3,
                // D-2026-08-13-1 allocated call id 4 to the exact-number
                // constructor; v7 preserves that append-only spelling.
                expr::Function::Decimal => 4,
            }),
            Cbor::Array(args.iter().map(expr_tree).collect()),
        ]),
        expr::Expr::List(items) => Cbor::Array(vec![
            Cbor::Uint(8),
            Cbor::Array(items.iter().map(expr_tree).collect()),
        ]),
        expr::Expr::MapLit(pairs) => Cbor::Array(vec![
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
        expr::Expr::Struct(fields) => {
            Cbor::Array(vec![Cbor::Uint(11), expression_fields_tree(fields)])
        }
        expr::Expr::Tagged {
            union,
            case,
            fields,
        } => Cbor::Array(vec![
            Cbor::Uint(12),
            type_tree(&DataType::TaggedUnion((**union).clone())),
            Cbor::Text(case.clone()),
            expression_fields_tree(fields),
        ]),
        expr::Expr::Is { expr, target } => Cbor::Array(vec![
            Cbor::Uint(13),
            expr_tree(expr),
            narrow_target_tree(target),
        ]),
        expr::Expr::As { expr, target } => Cbor::Array(vec![
            Cbor::Uint(14),
            expr_tree(expr),
            narrow_target_tree(target),
        ]),
    }
}

fn expression_fields_tree(fields: &[(String, expr::Expr)]) -> Cbor {
    Cbor::Array(
        fields
            .iter()
            .map(|(name, value)| Cbor::Array(vec![Cbor::Text(name.clone()), expr_tree(value)]))
            .collect(),
    )
}

fn narrow_target_tree(target: &expr::NarrowTarget) -> Cbor {
    match target {
        expr::NarrowTarget::Type(target) => {
            let mut parts = vec![Cbor::Uint(1), type_tree(target.ty())];
            if let Some(discriminator) = target.discriminator() {
                parts.push(Cbor::Array(vec![
                    Cbor::Text(discriminator.field().to_string()),
                    Cbor::Array(
                        discriminator
                            .mapping()
                            .iter()
                            .map(|(external, case)| {
                                Cbor::Array(vec![
                                    Cbor::Text(external.clone()),
                                    Cbor::Text(case.clone()),
                                ])
                            })
                            .collect(),
                    ),
                ]));
            }
            Cbor::Array(parts)
        }
        expr::NarrowTarget::Case { name, payload } => Cbor::Array(vec![
            Cbor::Uint(2),
            Cbor::Text(name.clone()),
            type_tree(payload),
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
        Expr(&'a expr::Expr, usize),
        Val(&'a Value, usize),
        Ty(&'a DataType, usize),
    }
    /// A `tree_tree`: its type and expression roots sit at `level`.
    fn tree<'a>(tree: &'a expr::Tree, level: usize, stack: &mut Vec<Item<'a>>) {
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
        }
        for decl in &node.ports {
            // decl array at 4, name and direction at 5 — and the
            // optional pinned type's root at 5 too.
            max = max.max(5);
            if let Some(ty) = &decl.ty {
                stack.push(Item::Ty(ty, 5));
            }
        }
        for var in &node.vars {
            // var map at 4, its fields at 5.
            max = max.max(5);
            stack.push(Item::Ty(&var.ty, 5));
            if let Some(init) = &var.init {
                stack.push(Item::Val(init, 5));
            }
        }
        if let Some(partition) = &node.partition {
            // partition array at 3, name/key/lifetime roots at 4;
            // tree_tree's expression and type roots sit at 5.
            max = max.max(5);
            tree(&partition.key, 5, &mut stack);
        }
    }
    for decl in &plan.boundary {
        // decl array at 2, name and direction at 3 — and the
        // optional pinned type's root at 3.
        max = max.max(3);
        if let Some(ty) = &decl.ty {
            stack.push(Item::Ty(ty, 3));
        }
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
                    expr::Expr::Asset(_) => {}
                    expr::Expr::Literal(value) => stack.push(Item::Val(value, level + 1)),
                    expr::Expr::Path(segments) => {
                        if !segments.is_empty() {
                            max = max.max(level + 2);
                        }
                    }
                    expr::Expr::Unary { expr, .. } => {
                        stack.push(Item::Expr(expr, level + 1));
                    }
                    expr::Expr::Binary { left, right, .. } => {
                        stack.push(Item::Expr(left, level + 1));
                        stack.push(Item::Expr(right, level + 1));
                    }
                    expr::Expr::Ternary {
                        cond,
                        then,
                        otherwise,
                    } => {
                        stack.push(Item::Expr(cond, level + 1));
                        stack.push(Item::Expr(then, level + 1));
                        stack.push(Item::Expr(otherwise, level + 1));
                    }
                    expr::Expr::Index { base, index } => {
                        stack.push(Item::Expr(base, level + 1));
                        stack.push(Item::Expr(index, level + 1));
                    }
                    expr::Expr::Call { args, .. } => {
                        for arg in args {
                            stack.push(Item::Expr(arg, level + 2));
                        }
                    }
                    expr::Expr::List(items) => {
                        for item in items {
                            stack.push(Item::Expr(item, level + 2));
                        }
                    }
                    expr::Expr::MapLit(pairs) => {
                        for (_, value) in pairs {
                            // pair array at +2, key text and value
                            // at +3.
                            max = max.max(level + 3);
                            stack.push(Item::Expr(value, level + 3));
                        }
                    }
                    expr::Expr::Struct(fields) => {
                        max = max.max(level + 3);
                        for (_, value) in fields {
                            stack.push(Item::Expr(value, level + 3));
                        }
                    }
                    expr::Expr::Tagged { union, fields, .. } => {
                        max = max.max(level + 3);
                        for (_, value) in fields {
                            stack.push(Item::Expr(value, level + 3));
                        }
                        max = max.max(
                            level + 1 + data_type_level(&DataType::TaggedUnion((**union).clone())),
                        );
                    }
                    expr::Expr::Is { expr, target } | expr::Expr::As { expr, target } => {
                        stack.push(Item::Expr(expr, level + 1));
                        max = max.max(level + 1 + narrow_target_level(target));
                    }
                }
            }
            Item::Val(value, level) => {
                // `value_tagged`: an array whose tag and scalar
                // payloads sit one down.
                max = max.max(level + 1);
                match value {
                    Value::Array(items) => {
                        for item in items {
                            stack.push(Item::Val(item, level + 2));
                        }
                    }
                    Value::Map(entries) => {
                        for entry in entries.values() {
                            // pair array at +2, the tagged key's own
                            // tag and text at +4.
                            max = max.max(level + 4);
                            stack.push(Item::Val(entry, level + 3));
                        }
                    }
                    Value::KeyedMap(entries) => {
                        for (key, value) in entries {
                            // pair array at +2; both key and value are tagged
                            // values whose own payload starts at +3.
                            stack.push(Item::Val(key, level + 3));
                            stack.push(Item::Val(value, level + 3));
                        }
                    }
                    Value::Struct(fields) => {
                        for (_, value) in fields {
                            // pair array at +2, its BARE field text
                            // at +3 (tag 17 keys carry no tag).
                            max = max.max(level + 3);
                            stack.push(Item::Val(value, level + 3));
                        }
                    }
                    Value::Decimal { .. } => {
                        // `[10, [bytes, scale]]` — the payload pair
                        // sits one deeper than the tag.
                        max = max.max(level + 2);
                    }
                    Value::StructuralUnion { value, .. } => {
                        stack.push(Item::Val(value, level + 2));
                    }
                    Value::TaggedUnion(tagged) => {
                        // `[21, [case, [17, [field-pair*]]]]`: the tagged
                        // struct root sits at +2, its fields array at +3, a
                        // field pair at +4, and its name/value roots at +5.
                        max = max.max(level + 3);
                        for (_, value) in &tagged.payload {
                            max = max.max(level + 5);
                            stack.push(Item::Val(value, level + 5));
                        }
                    }
                    _ => {}
                }
            }
            Item::Ty(ty, level) => match ty {
                DataType::Array(array) => {
                    max = max.max(level + 1);
                    stack.push(Item::Ty(array.element(), level + 1));
                }
                DataType::Map(map) => {
                    max = max.max(level + 1);
                    stack.push(Item::Ty(map.key(), level + 1));
                    stack.push(Item::Ty(map.value(), level + 1));
                }
                DataType::Struct(structure) => {
                    max = max.max(level + 3);
                    for field in structure.fields() {
                        stack.push(Item::Ty(field.ty(), level + 3));
                    }
                }
                DataType::StructuralUnion(union) => {
                    max = max.max(level + 2);
                    for branch in union.branches() {
                        stack.push(Item::Ty(branch, level + 2));
                    }
                }
                DataType::TaggedUnion(union) => {
                    max = max.max(level + 5);
                    for case in union.cases() {
                        for field in case.payload().fields() {
                            stack.push(Item::Ty(field.ty(), level + 5));
                        }
                    }
                }
                DataType::Enum(enumeration) => {
                    max = max.max(if enumeration.symbols().is_empty() {
                        level + 1
                    } else {
                        level + 2
                    });
                }
                DataType::Integer(_)
                | DataType::Float(_)
                | DataType::Decimal(_)
                | DataType::Bytes(_)
                | DataType::Temporal(_) => max = max.max(level + 1),
                DataType::Null
                | DataType::Bool
                | DataType::Number
                | DataType::Text
                | DataType::Uuid
                | DataType::Variant => max = max.max(level),
            },
        }
    }
    max
}

pub(crate) fn deepest_level_with_resolved(
    plan: &ExecutionPlan,
    resolved: &super::ResolvedPortTable,
) -> usize {
    let mut max = deepest_level(plan).max(2);
    for port in resolved.boundary() {
        max = max.max(4 + data_type_level(port.ty()));
    }
    let mut slot = 0_u32;
    while let Some(row) = resolved.node(Slot::new(slot)) {
        max = max.max(3);
        for port in row {
            max = max.max(5 + data_type_level(port.ty()));
        }
        let Some(next) = slot.checked_add(1) else {
            break;
        };
        slot = next;
    }
    max
}

fn data_type_level(ty: &DataType) -> usize {
    let mut max = 0;
    let mut stack = vec![(ty, 0usize)];
    while let Some((ty, level)) = stack.pop() {
        match ty {
            DataType::Integer(_)
            | DataType::Float(_)
            | DataType::Decimal(_)
            | DataType::Bytes(_)
            | DataType::Temporal(_) => max = max.max(level + 1),
            DataType::Struct(structure) => {
                max = max.max(level + 3);
                stack.extend(
                    structure
                        .fields()
                        .iter()
                        .map(|field| (field.ty(), level + 3)),
                );
            }
            DataType::Map(map) => {
                max = max.max(level + 1);
                stack.push((map.key(), level + 1));
                stack.push((map.value(), level + 1));
            }
            DataType::Array(array) => {
                max = max.max(level + 1);
                stack.push((array.element(), level + 1));
            }
            DataType::Enum(enumeration) => {
                max = max.max(
                    level
                        + if enumeration.symbols().is_empty() {
                            1
                        } else {
                            2
                        },
                );
            }
            DataType::StructuralUnion(union) => {
                max = max.max(level + 2);
                stack.extend(union.branches().iter().map(|branch| (branch, level + 2)));
            }
            DataType::TaggedUnion(union) => {
                max = max.max(level + 5);
                for case in union.cases() {
                    stack.extend(
                        case.payload()
                            .fields()
                            .iter()
                            .map(|field| (field.ty(), level + 5)),
                    );
                }
            }
            DataType::Null
            | DataType::Bool
            | DataType::Number
            | DataType::Text
            | DataType::Uuid
            | DataType::Variant => max = max.max(level),
        }
    }
    max
}

fn narrow_target_level(target: &expr::NarrowTarget) -> usize {
    match target {
        expr::NarrowTarget::Type(target) => {
            let base = 1 + data_type_level(target.ty());
            if let Some(discriminator) = target.discriminator() {
                base.max(if discriminator.mapping().is_empty() {
                    2
                } else {
                    4
                })
            } else {
                base
            }
        }
        expr::NarrowTarget::Case { payload, .. } => 1 + data_type_level(payload),
    }
}

fn binop_code(op: expr::BinaryOp) -> u64 {
    match op {
        expr::BinaryOp::Mul => 1,
        expr::BinaryOp::Div => 2,
        expr::BinaryOp::Rem => 3,
        expr::BinaryOp::Add => 4,
        expr::BinaryOp::Sub => 5,
        expr::BinaryOp::Eq => 6,
        expr::BinaryOp::Ne => 7,
        expr::BinaryOp::Lt => 8,
        expr::BinaryOp::Le => 9,
        expr::BinaryOp::Gt => 10,
        expr::BinaryOp::Ge => 11,
        expr::BinaryOp::And => 12,
        expr::BinaryOp::Or => 13,
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
    let app_schema = match pairs.get(&5) {
        Some(tree) => schema_from(tree)?,
        None => Vec::new(),
    };
    let ext_schema = match pairs.get(&6) {
        Some(tree) => schema_from(tree)?,
        None => Vec::new(),
    };
    let assets = match pairs.get(&7) {
        Some(tree) => array_of(tree, "assets")?
            .iter()
            .map(|blob| match blob {
                Cbor::Bytes(bytes) => Ok(crate::plan::AssetBlob {
                    digest: *blake3::hash(bytes).as_bytes(),
                    bytes: bytes.clone(),
                }),
                _ => Err(err("asset blob")),
            })
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };
    Ok(ExecutionPlan {
        nodes,
        boundary,
        edges,
        handlers,
        app_schema,
        ext_schema,
        assets,
    })
}

fn schema_from(tree: &Cbor) -> Result<Vec<(String, DataType)>, DecodeError> {
    array_of(tree, "zone schema")?
        .iter()
        .map(|entry| match array_of(entry, "schema field")? {
            [Cbor::Text(field), ty] => Ok((field.clone(), type_from(ty)?)),
            _ => Err(err("schema field")),
        })
        .collect()
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
    let partition = find(pairs, 5).map(partition_from).transpose()?;
    Ok(Node {
        path,
        kind,
        ports,
        vars,
        partition,
    })
}

fn partition_from(tree: &Cbor) -> Result<PartitionSpec, DecodeError> {
    let [name, key, lifetime] = array_of(tree, "partition")? else {
        return Err(err("partition"));
    };
    let name = match name {
        Cbor::Null => None,
        Cbor::Text(name) if !name.is_empty() => Some(name.clone()),
        _ => return Err(err("partition name")),
    };
    let lifetime = match array_of(lifetime, "partition lifetime")? {
        [Cbor::Uint(1)] => PartitionLifetime::Persistent,
        [Cbor::Uint(2), nanos] => PartitionLifetime::IdleTtl(signed_from(nanos)?),
        _ => return Err(err("partition lifetime")),
    };
    Ok(PartitionSpec {
        name,
        key: tree_from(key)?,
        lifetime,
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
    Ok(ConnectorSpec {
        contract,
        effect,
        behavior,
        attributes,
    })
}

fn attr_from(tree: &Cbor) -> Result<AttrSetting, DecodeError> {
    match array_of(tree, "attr setting")? {
        [Cbor::Uint(1), Cbor::Text(word)] => Ok(AttrSetting::Word(word.clone())),
        [Cbor::Uint(2), value] => Ok(AttrSetting::Value(value_from(value)?)),
        _ => Err(err("attr setting")),
    }
}

fn port_decl_from(tree: &Cbor) -> Result<PortDecl, DecodeError> {
    let (name, direction, ty) = match array_of(tree, "port declaration")? {
        [Cbor::Text(name), Cbor::Uint(direction)] => (name, direction, None),
        [Cbor::Text(name), Cbor::Uint(direction), ty] => (name, direction, Some(type_from(ty)?)),
        _ => return Err(err("port declaration")),
    };
    let (direction, unwired) = match direction {
        1 => (node::Direction::Input, super::UnwiredPolicy::RequireWire),
        2 => (node::Direction::Output, super::UnwiredPolicy::RequireWire),
        3 => (node::Direction::Output, super::UnwiredPolicy::RootSink),
        4 => (node::Direction::Input, super::UnwiredPolicy::RootSink),
        _ => return Err(err("port direction/policy")),
    };
    Ok(PortDecl {
        port: node::Port::new(name.clone()).map_err(|_| err("port name"))?,
        direction,
        unwired,
        ty,
    })
}

fn var_from(tree: &Cbor) -> Result<VarCell, DecodeError> {
    let pairs = map_of(tree, "var")?;
    let name = text_of(get(pairs, 1, "var name")?, "var name")?.to_string();
    let ty = type_from(get(pairs, 2, "var type")?)?;
    let init = match find(pairs, 3) {
        Some(value) => Some(value_from(value)?),
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
        [Cbor::Uint(2), Cbor::Uint(lifecycle)] => {
            node::HandlerActivation::Lifecycle(match lifecycle {
                1 => node::Lifecycle::Start,
                2 => node::Lifecycle::Stop,
                3 => node::Lifecycle::Compensate,
                4 => node::Lifecycle::Next,
                5 => node::Lifecycle::Timeout,
                6 => node::Lifecycle::Evict,
                7 => node::Lifecycle::Recover,
                _ => return Err(err("activation lifecycle")),
            })
        }
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
        [Cbor::Uint(2), Cbor::Text(field)] => Ok(WriteTarget::App(field.clone())),
        [Cbor::Uint(5), Cbor::Text(field)] => Ok(WriteTarget::Ext(field.clone())),
        // 3 (custom) and 4 (mime) are retired draft tags — refused
        // like any unknown discriminant.
        _ => Err(err("write target")),
    }
}

fn tree_from(tree: &Cbor) -> Result<expr::Tree, DecodeError> {
    match array_of(tree, "expression tree")? {
        [ty, expr] => Ok(expr::Tree {
            ty: type_from(ty)?,
            root: expr_from(expr)?,
        }),
        _ => Err(err("expression tree")),
    }
}

fn type_from(tree: &Cbor) -> Result<DataType, DecodeError> {
    match tree {
        Cbor::Uint(1) => Ok(DataType::Null),
        Cbor::Uint(2) => Ok(DataType::Bool),
        Cbor::Uint(6) => Ok(DataType::Number),
        Cbor::Uint(7) => Ok(DataType::Text),
        Cbor::Uint(10) => Ok(DataType::Uuid),
        Cbor::Uint(17) => Ok(DataType::Variant),
        Cbor::Array(parts) => type_parts_from(parts),
        _ => Err(err("data type")),
    }
}

fn type_parts_from(parts: &[Cbor]) -> Result<DataType, DecodeError> {
    let built = match parts {
        [Cbor::Uint(3), Cbor::Uint(signedness), Cbor::Uint(width)] => {
            let width = integer_width_from(*width)?;
            DataType::Integer(match signedness {
                1 => IntegerType::Signed(width),
                2 => IntegerType::Unsigned(width),
                _ => return Err(err("integer signedness")),
            })
        }
        [Cbor::Uint(4), Cbor::Uint(width)] => DataType::Float(match width {
            16 => FloatType::Binary16,
            32 => FloatType::Binary32,
            64 => FloatType::Binary64,
            _ => return Err(err("float width")),
        }),
        [Cbor::Uint(5), Cbor::Uint(precision), scale] => {
            let precision = u32::try_from(*precision).map_err(|_| err("decimal precision"))?;
            let scale = i32::try_from(int_from(scale)?).map_err(|_| err("decimal scale"))?;
            DataType::Decimal(DecimalType::new(precision, scale).map_err(|_| err("decimal type"))?)
        }
        [Cbor::Uint(8), length] => DataType::Bytes(match length {
            Cbor::Null => BinaryType::Variable,
            Cbor::Uint(length) => BinaryType::Fixed(
                ByteLength::new(u32::try_from(*length).map_err(|_| err("byte length"))?)
                    .map_err(|_| err("byte length"))?,
            ),
            _ => return Err(err("byte length")),
        }),
        [Cbor::Uint(9), ..] => DataType::Temporal(temporal_type_from(parts)?),
        [Cbor::Uint(11), fields] => DataType::Struct(struct_fields_from(fields)?),
        [Cbor::Uint(12), key, value] => DataType::Map(
            MapType::new(type_from(key)?, type_from(value)?).map_err(|_| err("map type"))?,
        ),
        [Cbor::Uint(13), element, length] => DataType::Array(match length {
            Cbor::Null => ArrayType::variable(type_from(element)?),
            Cbor::Uint(length) => ArrayType::fixed(
                type_from(element)?,
                u32::try_from(*length).map_err(|_| err("array length"))?,
            )
            .map_err(|_| err("array length"))?,
            _ => return Err(err("array length")),
        }),
        [Cbor::Uint(14), Cbor::Array(symbols)] => DataType::Enum(
            EnumType::new(
                symbols
                    .iter()
                    .map(|symbol| Ok(text_of(symbol, "enum symbol")?.to_string()))
                    .collect::<Result<Vec<_>, DecodeError>>()?,
            )
            .map_err(|_| err("enum type"))?,
        ),
        [Cbor::Uint(15), Cbor::Array(branches)] => DataType::structural_union(
            branches
                .iter()
                .map(type_from)
                .collect::<Result<Vec<_>, _>>()?,
        )
        .map_err(|_| err("structural union"))?,
        [Cbor::Uint(16), Cbor::Array(cases)] => DataType::TaggedUnion(
            TaggedUnionType::new(
                cases
                    .iter()
                    .map(|case| match array_of(case, "tagged-union case")? {
                        [Cbor::Text(name), fields] => {
                            TaggedCase::new(name.clone(), struct_fields_from(fields)?)
                                .map_err(|_| err("tagged-union case"))
                        }
                        _ => Err(err("tagged-union case")),
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            )
            .map_err(|_| err("tagged union"))?,
        ),
        _ => return Err(err("data type")),
    };
    built.validate().map_err(|_| err("data type budget"))?;
    Ok(built)
}

fn integer_width_from(width: u64) -> Result<IntegerWidth, DecodeError> {
    match width {
        8 => Ok(IntegerWidth::Bits8),
        16 => Ok(IntegerWidth::Bits16),
        32 => Ok(IntegerWidth::Bits32),
        64 => Ok(IntegerWidth::Bits64),
        _ => Err(err("integer width")),
    }
}

fn struct_fields_from(tree: &Cbor) -> Result<StructType, DecodeError> {
    StructType::new(
        array_of(tree, "struct fields")?
            .iter()
            .map(|field| match array_of(field, "struct field type")? {
                [Cbor::Text(name), ty] => Ok(TypeField::new(name.clone(), type_from(ty)?)),
                _ => Err(err("struct field type")),
            })
            .collect::<Result<Vec<_>, _>>()?,
    )
    .map_err(|_| err("struct type"))
}

fn temporal_type_from(parts: &[Cbor]) -> Result<TemporalType, DecodeError> {
    let time_unit = |code: u64| match code {
        1 => Ok(TimeUnit::Second),
        2 => Ok(TimeUnit::Millisecond),
        3 => Ok(TimeUnit::Microsecond),
        4 => Ok(TimeUnit::Nanosecond),
        _ => Err(err("time unit")),
    };
    match parts {
        [Cbor::Uint(9), Cbor::Uint(1), Cbor::Uint(unit)] => Ok(TemporalType::Date(match unit {
            1 => DateUnit::Day,
            2 => DateUnit::Millisecond,
            _ => return Err(err("date unit")),
        })),
        [Cbor::Uint(9), Cbor::Uint(2), Cbor::Uint(unit)] => {
            Ok(TemporalType::Time(time_unit(*unit)?))
        }
        [Cbor::Uint(9), Cbor::Uint(3), Cbor::Uint(unit), zone] => Ok(TemporalType::Timestamp {
            unit: time_unit(*unit)?,
            zone: match zone {
                Cbor::Null => TimeZone::Utc,
                Cbor::Text(name) => {
                    TimeZone::named(name.clone()).map_err(|_| err("timestamp timezone"))?
                }
                _ => return Err(err("timestamp timezone")),
            },
        }),
        [Cbor::Uint(9), Cbor::Uint(4), Cbor::Uint(unit)] => {
            Ok(TemporalType::TimestampNtz(time_unit(*unit)?))
        }
        [Cbor::Uint(9), Cbor::Uint(5), Cbor::Uint(unit)] => {
            Ok(TemporalType::Duration(time_unit(*unit)?))
        }
        [Cbor::Uint(9), Cbor::Uint(6), Cbor::Uint(unit)] => {
            Ok(TemporalType::Interval(match unit {
                1 => IntervalUnit::YearMonth,
                2 => IntervalUnit::DayTime,
                3 => IntervalUnit::MonthDayNano,
                4 => IntervalUnit::MonthDayMillisUnsigned,
                _ => return Err(err("interval unit")),
            }))
        }
        _ => Err(err("temporal type")),
    }
}

fn expr_from(tree: &Cbor) -> Result<expr::Expr, DecodeError> {
    let parts = array_of(tree, "expression")?;
    match parts {
        [Cbor::Uint(1), value] => Ok(expr::Expr::Literal(value_from(value)?)),
        [Cbor::Uint(10), Cbor::Uint(index)] => Ok(expr::Expr::Asset(
            u32::try_from(*index).map_err(|_| err("asset index"))?,
        )),
        [Cbor::Uint(2), Cbor::Array(segments)] => Ok(expr::Expr::Path(
            segments
                .iter()
                .map(|seg| Ok(text_of(seg, "path segment")?.to_string()))
                .collect::<Result<Vec<_>, DecodeError>>()?,
        )),
        [Cbor::Uint(3), Cbor::Uint(op), expr] => Ok(expr::Expr::Unary {
            op: match op {
                1 => expr::UnaryOp::Not,
                2 => expr::UnaryOp::Neg,
                _ => return Err(err("unary op")),
            },
            expr: alloc::boxed::Box::new(expr_from(expr)?),
        }),
        [Cbor::Uint(4), Cbor::Uint(op), left, right] => Ok(expr::Expr::Binary {
            op: binop_from(*op)?,
            left: alloc::boxed::Box::new(expr_from(left)?),
            right: alloc::boxed::Box::new(expr_from(right)?),
        }),
        [Cbor::Uint(5), cond, then, otherwise] => Ok(expr::Expr::Ternary {
            cond: alloc::boxed::Box::new(expr_from(cond)?),
            then: alloc::boxed::Box::new(expr_from(then)?),
            otherwise: alloc::boxed::Box::new(expr_from(otherwise)?),
        }),
        [Cbor::Uint(6), base, index] => Ok(expr::Expr::Index {
            base: alloc::boxed::Box::new(expr_from(base)?),
            index: alloc::boxed::Box::new(expr_from(index)?),
        }),
        [Cbor::Uint(7), Cbor::Uint(function), Cbor::Array(args)] => Ok(expr::Expr::Call {
            function: match function {
                1 => expr::Function::Duration,
                2 => expr::Function::Timestamp,
                3 => expr::Function::Size,
                4 => expr::Function::Decimal,
                5 => expr::Function::Asset,
                _ => return Err(err("function")),
            },
            args: args.iter().map(expr_from).collect::<Result<Vec<_>, _>>()?,
        }),
        [Cbor::Uint(8), Cbor::Array(items)] => Ok(expr::Expr::List(
            items.iter().map(expr_from).collect::<Result<Vec<_>, _>>()?,
        )),
        [Cbor::Uint(9), Cbor::Array(pairs)] => Ok(expr::Expr::MapLit(
            pairs
                .iter()
                .map(|pair| match array_of(pair, "map literal entry")? {
                    [Cbor::Text(key), value] => Ok((key.clone(), expr_from(value)?)),
                    _ => Err(err("map literal entry")),
                })
                .collect::<Result<Vec<_>, _>>()?,
        )),
        [Cbor::Uint(11), fields] => Ok(expr::Expr::Struct(expression_fields_from(fields)?)),
        [Cbor::Uint(12), union, Cbor::Text(case), fields] => {
            let DataType::TaggedUnion(union) = type_from(union)? else {
                return Err(err("tagged constructor type"));
            };
            Ok(expr::Expr::Tagged {
                union: alloc::boxed::Box::new(union),
                case: case.clone(),
                fields: expression_fields_from(fields)?,
            })
        }
        [Cbor::Uint(13), value, target] => Ok(expr::Expr::Is {
            expr: alloc::boxed::Box::new(expr_from(value)?),
            target: alloc::boxed::Box::new(narrow_target_from(target)?),
        }),
        [Cbor::Uint(14), value, target] => Ok(expr::Expr::As {
            expr: alloc::boxed::Box::new(expr_from(value)?),
            target: alloc::boxed::Box::new(narrow_target_from(target)?),
        }),
        _ => Err(err("expression")),
    }
}

fn expression_fields_from(tree: &Cbor) -> Result<Vec<(String, expr::Expr)>, DecodeError> {
    array_of(tree, "expression fields")?
        .iter()
        .map(|field| match array_of(field, "expression field")? {
            [Cbor::Text(name), value] => Ok((name.clone(), expr_from(value)?)),
            _ => Err(err("expression field")),
        })
        .collect()
}

fn narrow_target_from(tree: &Cbor) -> Result<expr::NarrowTarget, DecodeError> {
    match array_of(tree, "narrow target")? {
        [Cbor::Uint(1), ty] => Ok(expr::NarrowTarget::Type(alloc::boxed::Box::new(
            expr::NarrowType::new(type_from(ty)?),
        ))),
        [Cbor::Uint(1), ty, discriminator] => {
            let [Cbor::Text(field), Cbor::Array(entries)] =
                array_of(discriminator, "discriminator")?
            else {
                return Err(err("discriminator"));
            };
            let mut mapping = BTreeMap::new();
            for entry in entries {
                let [Cbor::Text(external), Cbor::Text(case)] =
                    array_of(entry, "discriminator mapping")?
                else {
                    return Err(err("discriminator mapping"));
                };
                if mapping.insert(external.clone(), case.clone()).is_some() {
                    return Err(err("duplicate discriminator mapping"));
                }
            }
            let discriminator =
                Discriminator::new(field.clone(), mapping).map_err(|_| err("discriminator"))?;
            Ok(expr::NarrowTarget::Type(alloc::boxed::Box::new(
                expr::NarrowType::discriminated(type_from(ty)?, discriminator)
                    .map_err(|_| err("narrow target"))?,
            )))
        }
        [Cbor::Uint(2), Cbor::Text(name), payload] => {
            let payload = type_from(payload)?;
            if !matches!(payload, DataType::Struct(_)) || name.is_empty() {
                return Err(err("case target"));
            }
            Ok(expr::NarrowTarget::Case {
                name: name.clone(),
                payload: alloc::boxed::Box::new(payload),
            })
        }
        _ => Err(err("narrow target")),
    }
}

fn binop_from(code: u64) -> Result<expr::BinaryOp, DecodeError> {
    Ok(match code {
        1 => expr::BinaryOp::Mul,
        2 => expr::BinaryOp::Div,
        3 => expr::BinaryOp::Rem,
        4 => expr::BinaryOp::Add,
        5 => expr::BinaryOp::Sub,
        6 => expr::BinaryOp::Eq,
        7 => expr::BinaryOp::Ne,
        8 => expr::BinaryOp::Lt,
        9 => expr::BinaryOp::Le,
        10 => expr::BinaryOp::Gt,
        11 => expr::BinaryOp::Ge,
        12 => expr::BinaryOp::And,
        13 => expr::BinaryOp::Or,
        _ => return Err(err("binary op")),
    })
}

/// Reads a `tagged_value` back into the core value domain — the inverse of the
/// canon's `value_tagged`, over the tags the plan can carry.
fn value_from(tree: &Cbor) -> Result<Value, DecodeError> {
    match array_of(tree, "tagged value")? {
        [Cbor::Uint(1), Cbor::Bool(v)] => Ok(Value::Bool(*v)),
        [Cbor::Uint(2), value] | [Cbor::Uint(3), value] => {
            // Tags 2 (int) and 3 (long) are both the signed domain —
            // the registry keeps them distinct so the seal shows the
            // honest width; the decoded value is one i64 either way.
            Ok(Value::Int(int_from(value)?))
        }
        [Cbor::Uint(5), Cbor::Double(v)] => Ok(Value::Double(v.value())),
        [Cbor::Uint(7), Cbor::Bytes(v)] => Ok(Value::Bytes(v.clone())),
        [Cbor::Uint(8), Cbor::Text(v)] => Ok(Value::Text(v.clone())),
        [Cbor::Uint(9), Cbor::Null] => Ok(Value::Null),
        [Cbor::Uint(23), Cbor::Uint(v)] => Ok(Value::Uint(*v)),
        [Cbor::Uint(10), payload] => match array_of(payload, "decimal")? {
            [Cbor::Bytes(unscaled), scale] => {
                if unscaled.is_empty() || unscaled.len() > 16 {
                    return Err(err("decimal coefficient"));
                }
                let mut wide = if unscaled[0] & 0x80 != 0 {
                    [0xFF_u8; 16]
                } else {
                    [0x00_u8; 16]
                };
                wide[16 - unscaled.len()..].copy_from_slice(unscaled);
                let value = i128::from_be_bytes(wide);
                let scale_wide = int_from(scale)?;
                // The value registry carries the decimal128 domain. Whether
                // the value is a declared-scale decimal or a schema-less
                // number is decided by its transported DataType, not by a
                // second decoder-local fold.
                let scale = i32::try_from(scale_wide).map_err(|_| err("decimal scale"))?;
                if !crate::datatype::decimal_in_domain(value, scale) {
                    return Err(err("decimal form"));
                }
                Ok(Value::Decimal {
                    unscaled: value,
                    scale,
                })
            }
            _ => Err(err("decimal")),
        },
        [Cbor::Uint(12), value] => Ok(Value::Timestamp(int_from(value)?)),
        [Cbor::Uint(11), value] => Ok(Value::Date(int_from(value)?)),
        [Cbor::Uint(13), value] => Ok(Value::TimestampNtz(int_from(value)?)),
        [Cbor::Uint(14), value] => Ok(Value::Time(int_from(value)?)),
        [Cbor::Uint(15), Cbor::Bytes(value)] => Ok(Value::Uuid(
            value
                .as_slice()
                .try_into()
                .map_err(|_| err("uuid length"))?,
        )),
        [Cbor::Uint(16), value] => Ok(Value::Duration(int_from(value)?)),
        [Cbor::Uint(17), Cbor::Array(pairs)] => {
            // The struct literal — bare text keys, declaration order AS
            // SEALED (sorted). Its surrounding DataType supplies the closed
            // field contract; validation checks conformance before encoding.
            let mut fields = alloc::vec::Vec::new();
            for pair in pairs {
                match array_of(pair, "struct field")? {
                    [Cbor::Text(name), value] => {
                        fields.push((name.clone(), value_from(value)?));
                    }
                    _ => return Err(err("struct field")),
                }
            }
            Ok(Value::Struct(fields))
        }
        [Cbor::Uint(18), Cbor::Array(pairs)] => map_value_from(pairs),
        [Cbor::Uint(19), Cbor::Array(items)] => Ok(Value::Array(
            items
                .iter()
                .map(value_from)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        [Cbor::Uint(20), Cbor::Text(symbol)] => Ok(Value::Enum(symbol.clone())),
        [Cbor::Uint(21), payload] => match array_of(payload, "union value")? {
            [Cbor::Uint(branch), value] => Ok(Value::StructuralUnion {
                branch: u16::try_from(*branch).map_err(|_| err("union branch"))?,
                value: alloc::boxed::Box::new(value_from(value)?),
            }),
            [Cbor::Text(case), value] => {
                let Value::Struct(payload) = value_from(value)? else {
                    return Err(err("tagged-union payload"));
                };
                Ok(Value::TaggedUnion(alloc::boxed::Box::new(
                    crate::value::TaggedUnionValue {
                        case: case.clone(),
                        payload,
                    },
                )))
            }
            _ => Err(err("union value")),
        },
        [Cbor::Uint(24), payload] => match array_of(payload, "interval")? {
            [months, days, nanoseconds] => Ok(Value::Interval {
                months: int_from(months)?,
                days: int_from(days)?,
                nanoseconds: int_from(nanoseconds)?,
            }),
            _ => Err(err("interval")),
        },
        _ => Err(err("tagged value")),
    }
}

fn map_value_from(pairs: &[Cbor]) -> Result<Value, DecodeError> {
    let mut decoded = Vec::with_capacity(pairs.len());
    let mut all_text = true;
    for pair in pairs {
        let [key, value] = array_of(pair, "map entry")? else {
            return Err(err("map entry"));
        };
        let key = value_from(key)?;
        all_text &= matches!(key, Value::Text(_));
        decoded.push((key, value_from(value)?));
    }
    if all_text {
        let mut map = BTreeMap::new();
        for (key, value) in decoded {
            let Value::Text(key) = key else {
                return Err(err("text map key"));
            };
            if map.insert(key, value).is_some() {
                return Err(err("duplicate map key"));
            }
        }
        Ok(Value::Map(map))
    } else {
        Ok(Value::KeyedMap(decoded))
    }
}

fn signed_tree(value: i64) -> Cbor {
    if value >= 0 {
        Cbor::Uint(value.unsigned_abs())
    } else {
        Cbor::Negative((-1 - value).unsigned_abs())
    }
}

fn signed_from(value: &Cbor) -> Result<i64, DecodeError> {
    match value {
        Cbor::Uint(value) => i64::try_from(*value).map_err(|_| err("signed integer")),
        Cbor::Negative(value) => {
            let magnitude = i128::from(*value);
            i64::try_from(-1_i128 - magnitude).map_err(|_| err("signed integer"))
        }
        _ => Err(err("signed integer")),
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

    fn tree(root: expr::Expr) -> expr::Tree {
        expr::Tree {
            ty: DataType::Variant,
            root,
        }
    }

    /// One plan exercising every construct the builders encode.
    fn rich_plan() -> ExecutionPlan {
        let map_value = {
            let mut entries = BTreeMap::new();
            entries.insert(
                text("k"),
                Value::Array(vec![
                    Value::Int(-3),
                    Value::Bytes(vec![1]),
                    Value::Double(1.5),
                    Value::Timestamp(7),
                ]),
            );
            Value::Map(entries)
        };
        let expr = expr::Expr::Ternary {
            cond: Box::new(expr::Expr::Binary {
                op: expr::BinaryOp::Lt,
                left: Box::new(expr::Expr::Path(vec![text("seen")])),
                right: Box::new(expr::Expr::Literal(Value::Int(3))),
            }),
            then: Box::new(expr::Expr::Call {
                function: expr::Function::Size,
                args: vec![expr::Expr::List(vec![expr::Expr::MapLit(vec![(
                    text("k"),
                    expr::Expr::Unary {
                        op: expr::UnaryOp::Neg,
                        expr: Box::new(expr::Expr::Literal(Value::Int(1))),
                    },
                )])])],
            }),
            otherwise: Box::new(expr::Expr::Index {
                base: Box::new(expr::Expr::Path(vec![text("data"), text("f")])),
                index: Box::new(expr::Expr::Literal(Value::Uint(0))),
            }),
        };
        ExecutionPlan {
            nodes: vec![
                Node {
                    path: node::Path::new(["relay"]).unwrap(),
                    kind: NodeKind::Operator,
                    ports: vec![
                        PortDecl::untyped(port("extra"), node::Direction::Output),
                        PortDecl {
                            port: port("typed"),
                            direction: node::Direction::Input,
                            unwired: crate::plan::UnwiredPolicy::RequireWire,
                            ty: Some(DataType::text_map(DataType::array(DataType::Text))),
                        },
                    ],
                    vars: vec![VarCell {
                        name: text("seen"),
                        ty: DataType::text_map(DataType::array(DataType::INT)),
                        init: Some(map_value.clone()),
                    }],
                    partition: None,
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
                    }),
                    ports: vec![],
                    vars: vec![],
                    partition: None,
                },
            ],
            boundary: vec![
                PortDecl {
                    port: port("entry"),
                    direction: node::Direction::Input,
                    unwired: crate::plan::UnwiredPolicy::RequireWire,
                    ty: Some(DataType::array(DataType::INT)),
                },
                PortDecl::untyped(port("feed"), node::Direction::Output),
            ],
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
                                target: WriteTarget::App(text("m.dotted")),
                                expr: tree(expr::Expr::Literal(Value::Null)),
                            },
                            FieldWrite {
                                target: WriteTarget::Ext(text("content_type")),
                                expr: tree(expr::Expr::Literal(Value::Text(text("t")))),
                            },
                        ]),
                    },
                    Step::Emit {
                        port: port("out"),
                        value: Emission::Replace(vec![FieldWrite {
                            target: WriteTarget::Data(vec![]),
                            expr: tree(expr::Expr::Literal(Value::Duration(5))),
                        }]),
                    },
                ],
            }],
            app_schema: vec![(text("m.dotted"), DataType::Variant)],
            ext_schema: vec![(text("content_type"), DataType::Text)],
            assets: vec![],
        }
    }

    /// A one-step plan carrying one expression — the chassis for the
    /// nested chains.
    fn one_step_plan(root: expr::Expr) -> ExecutionPlan {
        ExecutionPlan {
            nodes: vec![Node {
                path: node::Path::new(["o"]).unwrap(),
                kind: NodeKind::Operator,
                ports: vec![],
                vars: vec![],
                partition: None,
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
            app_schema: vec![],
            ext_schema: vec![],
            assets: vec![],
        }
    }

    #[test]
    fn v7_type_registry_is_total_and_pins_its_discriminants() {
        let enumeration = DataType::Enum(EnumType::new(vec![text("a"), text("b")]).unwrap());
        let structural = DataType::structural_union(vec![DataType::Null, DataType::Text]).unwrap();
        let tagged = DataType::TaggedUnion(
            TaggedUnionType::new(vec![
                TaggedCase::new("Done", StructType::new(vec![]).unwrap()).unwrap(),
            ])
            .unwrap(),
        );
        let cases = [
            (DataType::Null, Cbor::Uint(1)),
            (DataType::Bool, Cbor::Uint(2)),
            (
                DataType::INT,
                Cbor::Array(vec![Cbor::Uint(3), Cbor::Uint(1), Cbor::Uint(32)]),
            ),
            (
                DataType::DOUBLE,
                Cbor::Array(vec![Cbor::Uint(4), Cbor::Uint(64)]),
            ),
            (
                DataType::Decimal(DecimalType::new(10, 2).unwrap()),
                Cbor::Array(vec![Cbor::Uint(5), Cbor::Uint(10), Cbor::Uint(2)]),
            ),
            (DataType::Number, Cbor::Uint(6)),
            (DataType::Text, Cbor::Uint(7)),
            (
                DataType::BYTES,
                Cbor::Array(vec![Cbor::Uint(8), Cbor::Null]),
            ),
            (
                DataType::DURATION,
                Cbor::Array(vec![Cbor::Uint(9), Cbor::Uint(5), Cbor::Uint(4)]),
            ),
            (DataType::Uuid, Cbor::Uint(10)),
            (
                DataType::empty_struct(),
                Cbor::Array(vec![Cbor::Uint(11), Cbor::Array(vec![])]),
            ),
            (
                DataType::text_map(DataType::Text),
                Cbor::Array(vec![Cbor::Uint(12), Cbor::Uint(7), Cbor::Uint(7)]),
            ),
            (
                DataType::array(DataType::INT),
                Cbor::Array(vec![
                    Cbor::Uint(13),
                    Cbor::Array(vec![Cbor::Uint(3), Cbor::Uint(1), Cbor::Uint(32)]),
                    Cbor::Null,
                ]),
            ),
            (
                enumeration,
                Cbor::Array(vec![
                    Cbor::Uint(14),
                    Cbor::Array(vec![Cbor::Text(text("a")), Cbor::Text(text("b"))]),
                ]),
            ),
            (
                structural,
                Cbor::Array(vec![
                    Cbor::Uint(15),
                    Cbor::Array(vec![Cbor::Uint(1), Cbor::Uint(7)]),
                ]),
            ),
            (
                tagged,
                Cbor::Array(vec![
                    Cbor::Uint(16),
                    Cbor::Array(vec![Cbor::Array(vec![
                        Cbor::Text(text("Done")),
                        Cbor::Array(vec![]),
                    ])]),
                ]),
            ),
            (DataType::Variant, Cbor::Uint(17)),
        ];
        for (ty, wire) in cases {
            assert_eq!(type_tree(&ty), wire);
            assert_eq!(type_from(&wire).unwrap(), ty);
        }
    }

    #[test]
    fn v7_type_parameters_have_one_pinned_spelling() {
        let integers = [
            (IntegerType::Signed(IntegerWidth::Bits8), 1, 8),
            (IntegerType::Signed(IntegerWidth::Bits16), 1, 16),
            (IntegerType::Signed(IntegerWidth::Bits32), 1, 32),
            (IntegerType::Signed(IntegerWidth::Bits64), 1, 64),
            (IntegerType::Unsigned(IntegerWidth::Bits8), 2, 8),
            (IntegerType::Unsigned(IntegerWidth::Bits16), 2, 16),
            (IntegerType::Unsigned(IntegerWidth::Bits32), 2, 32),
            (IntegerType::Unsigned(IntegerWidth::Bits64), 2, 64),
        ];
        for (integer, signedness, width) in integers {
            let wire = Cbor::Array(vec![
                Cbor::Uint(3),
                Cbor::Uint(signedness),
                Cbor::Uint(width),
            ]);
            assert_eq!(type_tree(&DataType::Integer(integer)), wire);
            assert_eq!(type_from(&wire), Ok(DataType::Integer(integer)));
        }

        let floats = [
            (FloatType::Binary16, 16),
            (FloatType::Binary32, 32),
            (FloatType::Binary64, 64),
        ];
        for (float, width) in floats {
            let wire = Cbor::Array(vec![Cbor::Uint(4), Cbor::Uint(width)]);
            assert_eq!(type_tree(&DataType::Float(float)), wire);
            assert_eq!(type_from(&wire), Ok(DataType::Float(float)));
        }

        let temporal = [
            (TemporalType::Date(DateUnit::Day), vec![1, 1]),
            (TemporalType::Date(DateUnit::Millisecond), vec![1, 2]),
            (TemporalType::Time(TimeUnit::Second), vec![2, 1]),
            (TemporalType::Time(TimeUnit::Millisecond), vec![2, 2]),
            (TemporalType::Time(TimeUnit::Microsecond), vec![2, 3]),
            (TemporalType::Time(TimeUnit::Nanosecond), vec![2, 4]),
            (TemporalType::TimestampNtz(TimeUnit::Nanosecond), vec![4, 4]),
            (TemporalType::Duration(TimeUnit::Microsecond), vec![5, 3]),
            (TemporalType::Interval(IntervalUnit::YearMonth), vec![6, 1]),
            (TemporalType::Interval(IntervalUnit::DayTime), vec![6, 2]),
            (
                TemporalType::Interval(IntervalUnit::MonthDayNano),
                vec![6, 3],
            ),
            (
                TemporalType::Interval(IntervalUnit::MonthDayMillisUnsigned),
                vec![6, 4],
            ),
        ];
        for (temporal, parameters) in temporal {
            let mut parts = vec![Cbor::Uint(9)];
            parts.extend(parameters.into_iter().map(Cbor::Uint));
            let wire = Cbor::Array(parts);
            let ty = DataType::Temporal(temporal);
            assert_eq!(type_tree(&ty), wire);
            assert_eq!(type_from(&wire), Ok(ty));
        }

        let utc_timestamp = DataType::Temporal(TemporalType::Timestamp {
            unit: TimeUnit::Second,
            zone: TimeZone::Utc,
        });
        let utc_wire = Cbor::Array(vec![
            Cbor::Uint(9),
            Cbor::Uint(3),
            Cbor::Uint(1),
            Cbor::Null,
        ]);
        assert_eq!(type_tree(&utc_timestamp), utc_wire);
        assert_eq!(type_from(&utc_wire), Ok(utc_timestamp));

        let named_timestamp = DataType::Temporal(TemporalType::Timestamp {
            unit: TimeUnit::Millisecond,
            zone: TimeZone::named("Europe/Paris").unwrap(),
        });
        let named_wire = Cbor::Array(vec![
            Cbor::Uint(9),
            Cbor::Uint(3),
            Cbor::Uint(2),
            Cbor::Text(text("Europe/Paris")),
        ]);
        assert_eq!(type_tree(&named_timestamp), named_wire);
        assert_eq!(type_from(&named_wire), Ok(named_timestamp));

        let fixed_bytes = DataType::Bytes(BinaryType::Fixed(ByteLength::new(32).unwrap()));
        let fixed_bytes_wire = Cbor::Array(vec![Cbor::Uint(8), Cbor::Uint(32)]);
        assert_eq!(type_tree(&fixed_bytes), fixed_bytes_wire);
        assert_eq!(type_from(&fixed_bytes_wire), Ok(fixed_bytes));

        let fixed_array = DataType::Array(ArrayType::fixed(DataType::Text, 4).unwrap());
        let fixed_array_wire = Cbor::Array(vec![Cbor::Uint(13), Cbor::Uint(7), Cbor::Uint(4)]);
        assert_eq!(type_tree(&fixed_array), fixed_array_wire);
        assert_eq!(type_from(&fixed_array_wire), Ok(fixed_array));

        let signed_decimal = DataType::Decimal(DecimalType::new(18, -3).unwrap());
        let signed_decimal_wire =
            Cbor::Array(vec![Cbor::Uint(5), Cbor::Uint(18), Cbor::Negative(2)]);
        assert_eq!(type_tree(&signed_decimal), signed_decimal_wire);
        assert_eq!(type_from(&signed_decimal_wire), Ok(signed_decimal));
    }

    #[test]
    fn v7_rejects_each_malformed_type_table_family() {
        let malformed = [
            Cbor::Array(vec![Cbor::Uint(3), Cbor::Uint(3), Cbor::Uint(32)]),
            Cbor::Array(vec![Cbor::Uint(3), Cbor::Uint(1), Cbor::Uint(24)]),
            Cbor::Array(vec![Cbor::Uint(4), Cbor::Uint(128)]),
            Cbor::Array(vec![Cbor::Uint(5), Cbor::Uint(0), Cbor::Uint(0)]),
            Cbor::Array(vec![Cbor::Uint(8), Cbor::Uint(0)]),
            Cbor::Array(vec![Cbor::Uint(9), Cbor::Uint(2), Cbor::Uint(9)]),
            Cbor::Array(vec![
                Cbor::Uint(11),
                Cbor::Array(vec![
                    Cbor::Array(vec![Cbor::Text(text("x")), Cbor::Uint(7)]),
                    Cbor::Array(vec![Cbor::Text(text("x")), Cbor::Uint(7)]),
                ]),
            ]),
            Cbor::Array(vec![
                Cbor::Uint(12),
                Cbor::Array(vec![Cbor::Uint(13), Cbor::Uint(7), Cbor::Null]),
                Cbor::Uint(7),
            ]),
            Cbor::Array(vec![Cbor::Uint(14), Cbor::Array(vec![])]),
            Cbor::Array(vec![
                Cbor::Uint(15),
                Cbor::Array(vec![Cbor::Uint(7), Cbor::Uint(7)]),
            ]),
            Cbor::Array(vec![Cbor::Uint(16), Cbor::Array(vec![])]),
        ];
        for wire in malformed {
            assert!(
                type_from(&wire).is_err(),
                "malformed type admitted: {wire:?}"
            );
        }

        let unsorted = Cbor::Array(vec![
            Cbor::Uint(11),
            Cbor::Array(vec![
                Cbor::Array(vec![Cbor::Text(text("z")), Cbor::Uint(7)]),
                Cbor::Array(vec![Cbor::Text(text("a")), Cbor::Uint(7)]),
            ]),
        ]);
        let normalized = type_tree(&type_from(&unsorted).unwrap());
        assert_ne!(normalized, unsorted, "source order is not a second seal");
    }

    #[test]
    fn every_v7_expression_form_round_trips_with_its_resolved_target() {
        let payload = StructType::new(vec![TypeField::new("message", DataType::Text)]).unwrap();
        let union = TaggedUnionType::new(vec![TaggedCase::new("Failed", payload.clone()).unwrap()])
            .unwrap();
        let mut mapping = BTreeMap::new();
        mapping.insert(text("failure"), text("Failed"));
        let discriminator = Discriminator::new("kind", mapping).unwrap();
        let target = expr::NarrowTarget::Type(Box::new(
            expr::NarrowType::discriminated(DataType::TaggedUnion(union.clone()), discriminator)
                .unwrap(),
        ));
        let forms = [
            expr::Expr::Struct(vec![(
                text("message"),
                expr::Expr::Literal(Value::Text(text("bad"))),
            )]),
            expr::Expr::Tagged {
                union: Box::new(union),
                case: text("Failed"),
                fields: vec![(
                    text("message"),
                    expr::Expr::Literal(Value::Text(text("bad"))),
                )],
            },
            expr::Expr::Is {
                expr: Box::new(expr::Expr::Path(vec![text("data")])),
                target: Box::new(target.clone()),
            },
            expr::Expr::As {
                expr: Box::new(expr::Expr::Path(vec![text("data")])),
                target: Box::new(expr::NarrowTarget::Case {
                    name: text("Failed"),
                    payload: Box::new(DataType::Struct(payload)),
                }),
            },
        ];
        for form in forms {
            assert_eq!(expr_from(&expr_tree(&form)).unwrap(), form);
        }
    }

    #[test]
    fn every_published_value_family_round_trips_in_v7() {
        let mut text_map = BTreeMap::new();
        text_map.insert(text("k"), Value::Bool(true));
        let values = vec![
            Value::Null,
            Value::Bool(true),
            Value::Int(i64::MIN),
            Value::Uint(u64::MAX),
            Value::Double(-0.0),
            Value::Text(text("x")),
            Value::Decimal {
                unscaled: 100,
                scale: 0,
            },
            Value::Bytes(vec![1, 2]),
            Value::Duration(1),
            Value::Timestamp(2),
            Value::Date(3),
            Value::Time(4),
            Value::TimestampNtz(5),
            Value::Interval {
                months: 1,
                days: 2,
                nanoseconds: 3,
            },
            Value::Uuid([7; 16]),
            Value::Struct(vec![(text("x"), Value::Int(1))]),
            Value::Map(text_map),
            Value::KeyedMap(vec![(Value::Int(1), Value::Text(text("one")))]),
            Value::Array(vec![Value::Int(1)]),
            Value::Enum(text("Ready")),
            Value::StructuralUnion {
                branch: 1,
                value: Box::new(Value::Text(text("x"))),
            },
            Value::TaggedUnion(Box::new(crate::value::TaggedUnionValue {
                case: text("Done"),
                payload: vec![(text("code"), Value::Int(0))],
            })),
        ];
        for value in values {
            assert_eq!(value_from(&value_tagged(&value)).unwrap(), value);
        }
    }

    #[test]
    fn the_resolved_table_is_mandatory_and_cannot_lie() {
        let validated = ValidatedExecutionPlan::try_new(ExecutionPlan {
            nodes: vec![],
            boundary: vec![],
            edges: vec![],
            handlers: vec![],
            app_schema: vec![],
            ext_schema: vec![],
            assets: vec![],
        })
        .unwrap();
        let hash = SemanticHash::from_bytes([0; 32]);
        let package = encode(&validated, &hash);

        let mutate = |edit: &dyn Fn(&mut BTreeMap<u64, Cbor>)| {
            let (Cbor::Map(mut envelope), _) =
                canonical_cbor::decode(&package[MAGIC.len()..]).expect("envelope")
            else {
                panic!("map envelope");
            };
            let Cbor::Bytes(body) = envelope.get(&3).unwrap() else {
                panic!("body");
            };
            let (Cbor::Map(mut plan), _) = canonical_cbor::decode(body).expect("body") else {
                panic!("map body");
            };
            edit(&mut plan);
            let body = canonical_cbor::encode(&Cbor::Map(plan));
            envelope.insert(3, Cbor::Bytes(body.clone()));
            envelope.insert(
                4,
                Cbor::Bytes(checksum(FORMAT_VERSION, &[0; 32], &body).to_vec()),
            );
            let mut bytes = MAGIC.to_vec();
            bytes.extend_from_slice(&canonical_cbor::encode(&Cbor::Map(envelope)));
            bytes
        };

        let missing = mutate(&|plan| {
            plan.remove(&8);
        });
        assert!(matches!(decode(&missing), Err(DecodeError::Malformed(_))));

        let lying = mutate(&|plan| {
            plan.insert(
                8,
                Cbor::Array(vec![
                    Cbor::Array(vec![Cbor::Array(vec![
                        Cbor::Text(text("ghost")),
                        Cbor::Uint(1),
                        Cbor::Uint(7),
                        Cbor::Uint(2),
                    ])]),
                    Cbor::Array(vec![]),
                ]),
            );
        });
        assert_eq!(
            decode(&lying).unwrap_err(),
            DecodeError::ResolvedPortTableMismatch
        );

        let invalid_union = mutate(&|plan| {
            plan.insert(
                8,
                Cbor::Array(vec![
                    Cbor::Array(vec![Cbor::Array(vec![
                        Cbor::Text(text("ghost")),
                        Cbor::Uint(1),
                        Cbor::Array(vec![
                            Cbor::Uint(15),
                            Cbor::Array(vec![Cbor::Uint(7), Cbor::Uint(7)]),
                        ]),
                        Cbor::Uint(2),
                    ])]),
                    Cbor::Array(vec![]),
                ]),
            );
        });
        assert!(matches!(
            decode(&invalid_union),
            Err(DecodeError::Malformed(_))
        ));

        let malformed_rows = [
            Cbor::Array(vec![]),
            Cbor::Array(vec![Cbor::Array(vec![]), Cbor::Uint(0)]),
            Cbor::Array(vec![
                Cbor::Array(vec![Cbor::Array(vec![
                    Cbor::Text(text("")),
                    Cbor::Uint(1),
                    Cbor::Uint(7),
                    Cbor::Uint(1),
                ])]),
                Cbor::Array(vec![]),
            ]),
            Cbor::Array(vec![
                Cbor::Array(vec![Cbor::Array(vec![
                    Cbor::Text(text("in")),
                    Cbor::Uint(3),
                    Cbor::Uint(7),
                    Cbor::Uint(1),
                ])]),
                Cbor::Array(vec![]),
            ]),
            Cbor::Array(vec![
                Cbor::Array(vec![Cbor::Array(vec![
                    Cbor::Text(text("in")),
                    Cbor::Uint(1),
                    Cbor::Uint(7),
                    Cbor::Uint(3),
                ])]),
                Cbor::Array(vec![]),
            ]),
        ];
        for table in malformed_rows {
            assert!(
                resolved_table_from(&table).is_err(),
                "malformed resolved table admitted: {table:?}"
            );
        }

        let wrong_node_count = mutate(&|plan| {
            plan.insert(
                8,
                Cbor::Array(vec![
                    Cbor::Array(vec![]),
                    Cbor::Array(vec![Cbor::Array(vec![])]),
                ]),
            );
        });
        assert_eq!(
            decode(&wrong_node_count).unwrap_err(),
            DecodeError::ResolvedPortTableMismatch
        );
    }

    #[test]
    fn the_level_walker_matches_the_builders() {
        let mut nested_binary = expr::Expr::Literal(Value::Int(1));
        let mut nested_maplit = expr::Expr::Literal(Value::Int(1));
        for _ in 0..60 {
            nested_binary = expr::Expr::Binary {
                op: expr::BinaryOp::Add,
                left: Box::new(nested_binary),
                right: Box::new(expr::Expr::Literal(Value::Int(0))),
            };
            nested_maplit = expr::Expr::MapLit(vec![(text("k"), nested_maplit)]);
        }
        let mut nested_value = Value::Int(1);
        let mut nested_type = DataType::INT;
        for _ in 0..60 {
            nested_value = Value::Array(vec![nested_value]);
            nested_type = DataType::text_map(nested_type);
        }
        let mut deep_type_plan = one_step_plan(expr::Expr::Literal(Value::Null));
        deep_type_plan.handlers[0].body = vec![Step::Let {
            name: text("x"),
            expr: expr::Tree {
                ty: nested_type.clone(),
                root: expr::Expr::Literal(Value::Null),
            },
        }];
        let payload = StructType::new(vec![TypeField::new("message", DataType::Text)]).unwrap();
        let union = TaggedUnionType::new(vec![TaggedCase::new("Failed", payload.clone()).unwrap()])
            .unwrap();
        let tagged_value = Value::TaggedUnion(Box::new(crate::value::TaggedUnionValue {
            case: text("Failed"),
            payload: vec![(text("message"), Value::Text(text("bad")))],
        }));
        let mut mapping = BTreeMap::new();
        mapping.insert(text("failure"), text("Failed"));
        let discriminated = expr::NarrowTarget::Type(Box::new(
            expr::NarrowType::discriminated(
                DataType::TaggedUnion(union.clone()),
                Discriminator::new("kind", mapping).unwrap(),
            )
            .unwrap(),
        ));
        let tagged_is = expr::Expr::Is {
            expr: Box::new(expr::Expr::Tagged {
                union: Box::new(union),
                case: text("Failed"),
                fields: vec![(text("message"), expr::Expr::Literal(tagged_value.clone()))],
            }),
            target: Box::new(discriminated),
        };
        let structured_as = expr::Expr::Struct(vec![(
            text("result"),
            expr::Expr::As {
                expr: Box::new(expr::Expr::Literal(tagged_value)),
                target: Box::new(expr::NarrowTarget::Case {
                    name: text("Failed"),
                    payload: Box::new(DataType::Struct(payload)),
                }),
            },
        )]);
        let plans = [
            ExecutionPlan {
                nodes: vec![],
                boundary: vec![],
                edges: vec![],
                handlers: vec![],
                app_schema: vec![],
                ext_schema: vec![],
                assets: vec![],
            },
            rich_plan(),
            one_step_plan(nested_binary),
            one_step_plan(nested_maplit),
            one_step_plan(expr::Expr::Literal(nested_value)),
            deep_type_plan,
            one_step_plan(tagged_is),
            one_step_plan(structured_as),
        ];
        for plan in &plans {
            assert_eq!(
                deepest_level(plan),
                levels(&plan_tree(plan)),
                "the walker's arithmetic drifted from the builders"
            );
        }

        let resolved_plan = ExecutionPlan {
            nodes: vec![],
            boundary: vec![PortDecl {
                port: port("deep"),
                direction: node::Direction::Input,
                unwired: crate::plan::UnwiredPolicy::RequireWire,
                ty: Some(nested_type),
            }],
            edges: vec![],
            handlers: vec![],
            app_schema: vec![],
            ext_schema: vec![],
            assets: vec![],
        };
        let resolved = crate::plan::type_resolution::resolve(&resolved_plan)
            .expect("the declared boundary type resolves");
        assert_eq!(
            deepest_level_with_resolved(&resolved_plan, &resolved),
            levels(&plan_tree_with_resolved(&resolved_plan, Some(&resolved))),
            "the resolved-port table drifted from the depth walker"
        );
    }
}
