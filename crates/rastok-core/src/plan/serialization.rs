//! The plan package — the transportable, verifiable serialization
//! of an [`ExecutionPlan`], per `docs/architecture/plan-package.md`.
//!
//! Format v24: the 4-byte magic `RKPB`, then one canonical CBOR map —
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

use crate::canonical;
use crate::canonical::{type_from, type_tree};
use crate::instance;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::{format, vec};

use crate::canonical_cbor::{self, Value as Cbor};
use crate::datatype::{DataType, Discriminator};
use crate::envelope::WriteTarget;
use crate::expr;

use crate::node;
use crate::node::NodeIndex;
use crate::plan::{
    AttrSetting, ConnectorSpec, DelayPolicy, DispatchStep, EmitValue, EvaluationSite,
    ExecutionPlan, ExtensionSpec, FieldWrite, Handler, LoopSpec, Node, NodeKind, Operator,
    OwnerValue, PlanConstant, PlanValidationError, PortDecl, SemanticHash, ValidatedExecutionPlan,
    VarCell,
};
use crate::schema_annotation::{
    AnnotationApplication, AnnotationBinding, AnnotationSet, AnnotationTable, DataAnnotation,
    PersonalDataClass, SchemaPath, SchemaPathSegment, SensitivityLevel,
};
use crate::value::Value;

pub mod fields;

pub(super) fn route_order_key(route: &super::RouteRule) -> Vec<u8> {
    canonical_cbor::encode(&fields::route_tree(route))
}

/// The package's leading magic.
pub const MAGIC: [u8; 4] = *b"RKPB";

/// The categorical package format: recursive Dispatch, independent routes,
/// explicit Extensions and dispositions, and exact `std/` Contracts. The
/// compact registry and ccbor values have one reading; every other package
/// version refuses before its body can be interpreted.
pub const FORMAT_VERSION: u32 = 24;

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

impl From<crate::canonical::TypeDecodeError> for DecodeError {
    fn from(error: crate::canonical::TypeDecodeError) -> Self {
        Self::Malformed(error.to_string())
    }
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
    let mut pairs = BTreeMap::from([(12, Cbor::Array(plan.nodes.iter().map(node_tree).collect()))]);
    // Zone schemas travel with the plan so validation needs no Registry.
    // An absent schema is an omitted key.
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
                    let mut fields = vec![
                        Cbor::Text(port.port().name().to_string()),
                        Cbor::Uint(match port.direction() {
                            node::PortDirection::Input => 1,
                            node::PortDirection::Output => 2,
                        }),
                        type_tree(port.ty()),
                        Cbor::Uint(match port.origin() {
                            super::TypeOrigin::Declared => 1,
                            super::TypeOrigin::Inferred => 2,
                        }),
                    ];
                    if !port.annotations().is_empty() {
                        fields.push(annotation_table_tree(port.annotations()));
                    }
                    Cbor::Array(fields)
                })
                .collect(),
        )
    };
    Cbor::Array(vec![
        row(table.boundary()),
        Cbor::Array(
            (0..)
                .map(NodeIndex::new)
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
            let parts = array_of(port, "resolved port")?;
            let (name, direction, ty, origin, annotations) = match parts {
                [
                    Cbor::Text(name),
                    Cbor::Uint(direction),
                    ty,
                    Cbor::Uint(origin),
                ] => (name, direction, ty, origin, AnnotationTable::empty()),
                [
                    Cbor::Text(name),
                    Cbor::Uint(direction),
                    ty,
                    Cbor::Uint(origin),
                    annotations,
                ] => (
                    name,
                    direction,
                    ty,
                    origin,
                    annotation_table_from(annotations)?,
                ),
                _ => return Err(err("resolved port")),
            };
            node::Port::new(name.clone()).map_err(|_| err("resolved port name"))?;
            if !matches!(direction, 1 | 2) {
                return Err(err("resolved port direction"));
            }
            if !matches!(origin, 1 | 2) {
                return Err(err("resolved port origin"));
            }
            let mut fields = vec![
                Cbor::Text(name.clone()),
                Cbor::Uint(*direction),
                type_tree(&type_from(ty)?),
                Cbor::Uint(*origin),
            ];
            if !annotations.is_empty() {
                fields.push(annotation_table_tree(&annotations));
            }
            decoded.push(Cbor::Array(fields));
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
    let body = match &node.body {
        NodeKind::Graph(graph) => {
            Cbor::Array(vec![Cbor::Uint(1), fields::routing_tree(&graph.wiring)])
        }
        NodeKind::Operator(operator) => Cbor::Array(vec![
            Cbor::Uint(2),
            Cbor::Array(operator.vars.iter().map(var_tree).collect()),
            Cbor::Array(operator.handlers.iter().map(handler_tree).collect()),
        ]),
        NodeKind::Connector(spec) => Cbor::Array(vec![Cbor::Uint(3), spec_tree(spec)]),
    };
    Cbor::Map(BTreeMap::from([
        (1, Cbor::Text(node.path.to_string())),
        (
            2,
            node.parent
                .map_or(Cbor::Null, |index| Cbor::Uint(index.index() as u64)),
        ),
        (
            3,
            Cbor::Array(node.ports.iter().map(port_decl_tree).collect()),
        ),
        (4, instance_tree(&node.instance)),
        (
            5,
            Cbor::Array(node.vals.iter().map(owner_value_tree).collect()),
        ),
        (
            6,
            Cbor::Array(node.extensions.iter().map(extension_spec_tree).collect()),
        ),
        (7, body),
    ]))
}

fn extension_spec_tree(extension: &ExtensionSpec) -> Cbor {
    match extension {
        ExtensionSpec::App => Cbor::Array(vec![Cbor::Uint(1)]),
        ExtensionSpec::Loop(spec) => Cbor::Array(vec![Cbor::Uint(2), loop_spec_tree(spec)]),
        ExtensionSpec::Retry(spec) => Cbor::Array(vec![Cbor::Uint(3), retry_spec_tree(spec)]),
    }
}

fn retry_spec_tree(spec: &super::RetrySpec) -> Cbor {
    let mut fields = BTreeMap::from([
        (1, plan_constant_tree(&spec.attempts)),
        (
            2,
            Cbor::Array(spec.cases.iter().cloned().map(Cbor::Text).collect()),
        ),
        (4, Cbor::Text(spec.entry.name().into())),
        (5, Cbor::Text(spec.terminal.name().into())),
    ]);
    if let Some(delay) = &spec.delay {
        fields.insert(3, loop_delay_tree(delay));
    }
    Cbor::Map(fields)
}

fn loop_spec_tree(spec: &LoopSpec) -> Cbor {
    let mut fields = BTreeMap::from([
        (4, Cbor::Text(spec.entry.name().into())),
        (
            5,
            Cbor::Array(
                spec.terminals
                    .iter()
                    .map(|port| Cbor::Text(port.name().into()))
                    .collect(),
            ),
        ),
    ]);
    if let Some(fuel) = &spec.fuel {
        fields.insert(1, plan_constant_tree(fuel));
    }
    if let Some(delay) = &spec.delay {
        fields.insert(2, loop_delay_tree(delay));
    }
    if let Some(proof) = &spec.proof {
        fields.insert(3, loop_proof_tree(proof));
    }
    if let Some(expired) = &spec.expired {
        fields.insert(
            6,
            Cbor::Array(vec![
                Cbor::Text(expired.port.name().into()),
                type_tree(&expired.ty),
            ]),
        );
    }
    Cbor::Map(fields)
}

fn loop_proof_tree(proof: &crate::loop_proof::Certificate) -> Cbor {
    let mut fields = BTreeMap::from([
        (1, tree_tree(&proof.measure)),
        (
            3,
            Cbor::Array(proof.decreases.iter().copied().map(Cbor::Uint).collect()),
        ),
    ]);
    if let Some(invariant) = &proof.invariant {
        fields.insert(2, tree_tree(invariant));
    }
    Cbor::Map(fields)
}

fn loop_delay_tree(delay: &DelayPolicy) -> Cbor {
    match delay {
        DelayPolicy::Fixed { after } => Cbor::Array(vec![Cbor::Uint(1), plan_constant_tree(after)]),
        DelayPolicy::Exponential { first, ceiling } => Cbor::Array(vec![
            Cbor::Uint(2),
            plan_constant_tree(first),
            plan_constant_tree(ceiling),
        ]),
    }
}

fn plan_constant_tree(constant: &PlanConstant) -> Cbor {
    Cbor::Array(vec![
        tree_tree(&constant.expr),
        canonical::value(&constant.resolved),
    ])
}

fn instance_tree(instance: &instance::InstanceSpec) -> Cbor {
    let mut fields = BTreeMap::from([(2, lifetime_tree(&instance.lifetime))]);
    if let Some(key) = &instance.key {
        fields.insert(1, tree_tree(key));
    }
    if let Some(closing) = instance.closing {
        fields.insert(
            3,
            Cbor::Array(vec![
                canonical::signed(closing.drain_for),
                canonical::signed(closing.settle_for),
            ]),
        );
    }
    Cbor::Map(fields)
}

fn lifetime_tree(lifetime: &instance::Lifetime) -> Cbor {
    match lifetime {
        instance::Lifetime::Persistent => Cbor::Array(vec![Cbor::Uint(1)]),
        instance::Lifetime::Idle(nanos) => {
            Cbor::Array(vec![Cbor::Uint(2), canonical::signed(*nanos)])
        }
        instance::Lifetime::Fixed(nanos) => {
            Cbor::Array(vec![Cbor::Uint(3), canonical::signed(*nanos)])
        }
        instance::Lifetime::SinglePass => Cbor::Array(vec![Cbor::Uint(4)]),
        instance::Lifetime::Tumbling(nanos) => {
            Cbor::Array(vec![Cbor::Uint(5), canonical::signed(*nanos)])
        }
    }
}

fn site_tree(site: EvaluationSite) -> Cbor {
    Cbor::Uint(match site {
        EvaluationSite::Plan => 1,
        EvaluationSite::Boot => 2,
        EvaluationSite::Instance => 3,
        EvaluationSite::Invocation => 4,
    })
}

fn owner_value_tree(value: &OwnerValue) -> Cbor {
    let mut fields = BTreeMap::from([
        (1, Cbor::Text(value.name.clone())),
        (2, site_tree(value.site)),
        (3, tree_tree(&value.expr)),
    ]);
    if let Some(resolved) = &value.resolved {
        fields.insert(4, canonical::value(resolved));
    }
    if !value.annotations.is_empty() {
        fields.insert(5, annotation_table_tree(&value.annotations));
    }
    Cbor::Map(fields)
}

fn spec_tree(spec: &ConnectorSpec) -> Cbor {
    let behavior = Cbor::Uint(match spec.behavior {
        node::Behavior::Deterministic => 1,
        node::Behavior::Probabilistic => 2,
    });
    let mut fields = BTreeMap::from([
        (1, Cbor::Text(spec.contract.as_str().into())),
        (2, behavior),
        (
            3,
            Cbor::Array(
                spec.attributes
                    .iter()
                    .map(|(name, setting)| {
                        Cbor::Array(vec![Cbor::Text(name.clone()), attr_tree(setting)])
                    })
                    .collect(),
            ),
        ),
        (4, Cbor::Uint(u64::from(spec.roles.tag()))),
        (5, Cbor::Uint(u64::from(spec.exchange.tag()))),
    ]);
    if let Some(guarantee) = &spec.idempotent {
        fields.insert(6, plan_constant_tree(guarantee));
    }
    Cbor::Map(fields)
}

fn attr_tree(setting: &AttrSetting) -> Cbor {
    match setting {
        AttrSetting::Word(word) => Cbor::Array(vec![Cbor::Uint(1), Cbor::Text(word.clone())]),
        AttrSetting::Value(value) => Cbor::Array(vec![Cbor::Uint(2), canonical::value(value)]),
        AttrSetting::Expr { site, expr } => {
            Cbor::Array(vec![Cbor::Uint(3), site_tree(*site), tree_tree(expr)])
        }
    }
}

fn port_decl_tree(decl: &PortDecl) -> Cbor {
    let mut parts = vec![
        Cbor::Text(decl.port.name().to_string()),
        Cbor::Uint(match decl.direction {
            node::PortDirection::Input => 1,
            node::PortDirection::Output => 2,
        }),
    ];
    // An absent type denotes inference; the optional annotation table uses
    // an explicit null in that position so the array has only one reading.
    if let Some(ty) = &decl.ty {
        parts.push(type_tree(ty));
    } else if !decl.annotations.is_empty() {
        // The sparse table belongs beside the effective resolved schema. A
        // foreign frontend may annotate an inferred port, so `null` preserves
        // the raw inference variable without making the table ambiguous.
        parts.push(Cbor::Null);
    }
    if !decl.annotations.is_empty() {
        parts.push(annotation_table_tree(&decl.annotations));
    }
    Cbor::Array(parts)
}

fn var_tree(var: &VarCell) -> Cbor {
    let mut pairs = BTreeMap::from([(1, Cbor::Text(var.name.clone())), (2, type_tree(&var.ty))]);
    if let Some(init) = &var.init {
        pairs.insert(3, canonical::value(init));
    }
    if !var.annotations.is_empty() {
        pairs.insert(4, annotation_table_tree(&var.annotations));
    }
    Cbor::Map(pairs)
}

fn handler_tree(handler: &Handler) -> Cbor {
    let activation = match &handler.activation {
        node::HandlerActivation::Port(port) => {
            Cbor::Array(vec![Cbor::Uint(1), Cbor::Text(port.name().to_string())])
        }
        node::HandlerActivation::Join {
            ports,
            within_ns,
            last,
        } => Cbor::Array(vec![
            Cbor::Uint(3),
            Cbor::Array(
                ports
                    .iter()
                    .map(|port| Cbor::Text(port.name().to_string()))
                    .collect(),
            ),
            within_ns.map_or(Cbor::Null, canonical::signed),
            last.map_or(Cbor::Null, |count| Cbor::Uint(u64::from(count))),
        ]),
        node::HandlerActivation::Lifecycle(lifecycle) => Cbor::Array(vec![
            Cbor::Uint(2),
            Cbor::Uint(u64::from(u8::from(*lifecycle))),
        ]),
    };
    let mut pairs = BTreeMap::from([(2, activation)]);
    if let Some(guard) = &handler.guard {
        pairs.insert(3, tree_tree(guard));
    }
    pairs.insert(4, fields::dispatch_tree(&handler.dispatch));
    Cbor::Map(pairs)
}

fn step_tree(step: &DispatchStep) -> Cbor {
    match step {
        DispatchStep::Let {
            name,
            value: expr,
            annotations,
        } => {
            let mut fields = vec![Cbor::Uint(1), Cbor::Text(name.clone()), tree_tree(expr)];
            if !annotations.is_empty() {
                fields.push(annotation_table_tree(annotations));
            }
            Cbor::Array(fields)
        }
        DispatchStep::Emit { port, value } => Cbor::Array(vec![
            Cbor::Uint(2),
            Cbor::Text(port.name().to_string()),
            emission_tree(value),
        ]),
        DispatchStep::SetVar { name, value: expr } => Cbor::Array(vec![
            Cbor::Uint(3),
            Cbor::Text(name.clone()),
            tree_tree(expr),
        ]),
        DispatchStep::Stop => Cbor::Array(vec![Cbor::Uint(4)]),
        DispatchStep::MultiEmit { ports, values } => Cbor::Array(vec![
            Cbor::Uint(5),
            Cbor::Array(
                ports
                    .iter()
                    .map(|port| Cbor::Text(port.name().to_string()))
                    .collect(),
            ),
            match values {
                super::MultiEmitValues::Broadcast(value) => {
                    Cbor::Array(vec![Cbor::Uint(1), emission_tree(value)])
                }
                super::MultiEmitValues::Tuple(values) => Cbor::Array(vec![
                    Cbor::Uint(2),
                    Cbor::Array(values.iter().map(emission_tree).collect()),
                ]),
            },
        ]),
        DispatchStep::LoopCommand(command) => {
            let mut fields = vec![Cbor::Uint(u64::from(command.kind().tag()))];
            if let super::LoopCommand::Next(value) = command {
                fields.push(emission_tree(value));
            }
            Cbor::Array(vec![Cbor::Uint(6), Cbor::Array(fields)])
        }
    }
}

fn emission_tree(emission: &EmitValue) -> Cbor {
    match emission {
        EmitValue::Identity => Cbor::Array(vec![Cbor::Uint(1)]),
        EmitValue::Patch(writes) => Cbor::Array(vec![Cbor::Uint(2), writes_tree(writes)]),
        EmitValue::FreshData(writes) => Cbor::Array(vec![Cbor::Uint(3), writes_tree(writes)]),
        EmitValue::Data(tree) => Cbor::Array(vec![Cbor::Uint(4), tree_tree(tree)]),
        EmitValue::Nothing => Cbor::Array(vec![Cbor::Uint(5)]),
    }
}

fn writes_tree(writes: &[FieldWrite]) -> Cbor {
    Cbor::Array(
        writes
            .iter()
            .map(|write| Cbor::Array(vec![target_tree(&write.target), tree_tree(&write.value)]))
            .collect(),
    )
}

fn target_tree(target: &WriteTarget) -> Cbor {
    match target {
        WriteTarget::Data(path) => Cbor::Array(vec![
            Cbor::Uint(1),
            Cbor::Array(path.iter().map(|seg| Cbor::Text(seg.clone())).collect()),
        ]),
        WriteTarget::App(field) => Cbor::Array(vec![Cbor::Uint(2), Cbor::Text(field.clone())]),
        WriteTarget::Ext(field) => Cbor::Array(vec![Cbor::Uint(3), Cbor::Text(field.clone())]),
    }
}

fn annotation_table_tree(table: &AnnotationTable) -> Cbor {
    Cbor::Array(
        table
            .entries()
            .iter()
            .map(|(path, set)| Cbor::Array(vec![schema_path_tree(path), annotation_set_tree(set)]))
            .collect(),
    )
}

fn schema_path_tree(path: &SchemaPath) -> Cbor {
    Cbor::Array(
        path.segments()
            .iter()
            .map(|segment| match segment {
                SchemaPathSegment::Field(name) => {
                    Cbor::Array(vec![Cbor::Uint(1), Cbor::Text(name.clone())])
                }
                SchemaPathSegment::ArrayElement => Cbor::Array(vec![Cbor::Uint(2)]),
                SchemaPathSegment::MapKey => Cbor::Array(vec![Cbor::Uint(3)]),
                SchemaPathSegment::MapValue => Cbor::Array(vec![Cbor::Uint(4)]),
                SchemaPathSegment::StructuralUnionBranch(index) => {
                    Cbor::Array(vec![Cbor::Uint(5), Cbor::Uint(u64::from(*index))])
                }
                SchemaPathSegment::TaggedUnionCase(name) => {
                    Cbor::Array(vec![Cbor::Uint(6), Cbor::Text(name.clone())])
                }
            })
            .collect(),
    )
}

fn annotation_set_tree(set: &AnnotationSet) -> Cbor {
    Cbor::Array(
        set.annotations()
            .iter()
            .map(|annotation| match annotation {
                DataAnnotation::Declared(application) => Cbor::Array(vec![
                    Cbor::Uint(4),
                    Cbor::Uint(match application.binding() {
                        AnnotationBinding::Metadata => 1,
                        AnnotationBinding::Measure => 2,
                    }),
                    Cbor::Text(application.definition().to_string()),
                    type_tree(application.argument_type()),
                    canonical::value(application.arguments()),
                ]),
                DataAnnotation::Sensitive(level) => Cbor::Array(vec![
                    Cbor::Uint(1),
                    Cbor::Uint(match level {
                        SensitivityLevel::Internal => 1,
                        SensitivityLevel::Confidential => 2,
                        SensitivityLevel::Restricted => 3,
                    }),
                ]),
                DataAnnotation::Secret => Cbor::Array(vec![Cbor::Uint(2)]),
                DataAnnotation::Personal(class) => Cbor::Array(vec![
                    Cbor::Uint(3),
                    Cbor::Uint(
                        u64::from(class.is_special_category())
                            | (u64::from(class.is_criminal_offence()) << 1)
                            | (u64::from(class.is_identifier()) << 2),
                    ),
                ]),
            })
            .collect(),
    )
}

fn tree_tree(tree: &expr::Tree) -> Cbor {
    Cbor::Array(vec![type_tree(&tree.ty), expr_tree(&tree.root)])
}

fn expr_tree(expr: &expr::Expr) -> Cbor {
    match expr {
        expr::Expr::Literal(value) => Cbor::Array(vec![Cbor::Uint(1), canonical::value(value)]),
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
                expr::Function::Decimal => 4,
                expr::Function::MediaType => 6,
                expr::Function::Has => 7,
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
        Dispatch(&'a super::Dispatch, usize),
        Step(&'a DispatchStep, usize),
    }
    /// A `tree_tree`: its type and expression roots sit at `level`.
    fn tree<'a>(tree: &'a expr::Tree, level: usize, stack: &mut Vec<Item<'a>>) {
        stack.push(Item::Ty(&tree.ty, level));
        stack.push(Item::Expr(&tree.root, level));
    }
    fn emission<'a>(
        value: &'a EmitValue,
        level: usize,
        max: &mut usize,
        stack: &mut Vec<Item<'a>>,
    ) {
        *max = (*max).max(level + 1);
        match value {
            EmitValue::Identity | EmitValue::Nothing => {}
            EmitValue::Data(value) => tree(value, level + 2, stack),
            EmitValue::Patch(writes) | EmitValue::FreshData(writes) => {
                for write in writes {
                    *max = (*max).max(level + if matches!(&write.target, WriteTarget::Data(path) if !path.is_empty()) { 5 } else { 4 });
                    tree(&write.value, level + 4, stack);
                }
            }
        }
    }
    /// A `plan_constant_tree` at `level`: its tree and tagged value are one
    /// level down, and the tree's type/expression roots one level below that.
    fn plan_constant<'a>(
        constant: &'a PlanConstant,
        level: usize,
        max: &mut usize,
        stack: &mut Vec<Item<'a>>,
    ) {
        *max = (*max).max(level + 1);
        tree(&constant.expr, level + 2, stack);
        stack.push(Item::Val(&constant.resolved, level + 1));
    }
    /// An annotation table at `level`: row, path/set, and segment/annotation
    /// arrays add four fixed levels. Path length changes width, never depth.
    fn annotations<'a>(
        table: &'a AnnotationTable,
        level: usize,
        max: &mut usize,
        stack: &mut Vec<Item<'a>>,
    ) {
        if !table.is_empty() {
            *max = (*max).max(level + 4);
        }
        for set in table.entries().values() {
            for annotation in set.annotations() {
                if let DataAnnotation::Declared(application) = annotation {
                    stack.push(Item::Ty(application.argument_type(), level + 4));
                    stack.push(Item::Val(application.arguments(), level + 4));
                }
            }
        }
    }
    // The root map's table arrays sit at level 1.
    let mut max = 1usize;
    let mut stack: Vec<Item<'_>> = Vec::new();
    for route in plan.reference_routes() {
        // Nodes 1, Node 2, Graph body 3, wiring 4, rule 5, endpoint 6,
        // Node index and port name 7.
        max = max.max(7);
        let (sets, level): (Vec<&[super::Destination]>, usize) = match &route.selection {
            super::RouteSelection::All(destinations) => (vec![destinations], 7),
            super::RouteSelection::Cases(arms) => {
                if !arms.is_empty() {
                    max = max.max(9);
                }
                for arm in arms {
                    if !arm.cases.is_empty() {
                        max = max.max(10);
                    }
                }
                (
                    arms.iter().map(|arm| arm.destinations.as_slice()).collect(),
                    9,
                )
            }
        };
        for destination in sets.into_iter().flatten() {
            max = max.max(
                level
                    + match destination {
                        super::Destination::Port(_) => 3,
                        super::Destination::Discard => 2,
                        super::Destination::LoopCommand(_) => 2,
                        super::Destination::InstanceStop(_) => 2,
                    },
            );
        }
    }
    for node in &plan.nodes {
        // node map at 2: path, kind, and the two table arrays at 3.
        max = max.max(3);
        if let NodeKind::Connector(spec) = &node.body {
            // kind array at 3, spec map at 4, its fields at 5.
            max = max.max(5);
            if let Some(guarantee) = &spec.idempotent {
                plan_constant(guarantee, 5, &mut max, &mut stack);
            }
            for (_, setting) in &spec.attributes {
                // entry array at 6, name and attr_tree at 7, the
                // attr_tree's tag and payload at 8.
                max = max.max(8);
                match setting {
                    AttrSetting::Value(value) => stack.push(Item::Val(value, 8)),
                    // The expression payload is a tree_tree rooted at level
                    // 8; its type and expression children therefore start at
                    // level 9.
                    AttrSetting::Expr { expr, .. } => tree(expr, 9, &mut stack),
                    AttrSetting::Word(_) => {}
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
            annotations(&decl.annotations, 5, &mut max, &mut stack);
        }
        for var in node.vars() {
            // Operator body at 3, vars at 4, variable map at 5, fields at 6.
            max = max.max(6);
            stack.push(Item::Ty(&var.ty, 6));
            if let Some(init) = &var.init {
                stack.push(Item::Val(init, 6));
            }
            annotations(&var.annotations, 6, &mut max, &mut stack);
        }
    }
    for owner in &plan.nodes {
        // owner map at 2, fields and instance map at 3, instance fields at 4.
        max = max.max(5);
        if let Some(key) = &owner.instance.key {
            tree(key, 5, &mut stack);
        }
        for value in &owner.vals {
            // owner-value map at 4, fields at 5, tree roots at 6.
            tree(&value.expr, 6, &mut stack);
            if let Some(resolved) = &value.resolved {
                stack.push(Item::Val(resolved, 5));
            }
            annotations(&value.annotations, 5, &mut max, &mut stack);
        }
        for extension in &owner.extensions {
            // Extension array at 3, tagged payload at 4, policy map at 5.
            let delay = match extension {
                ExtensionSpec::App => {
                    max = max.max(5);
                    continue;
                }
                ExtensionSpec::Retry(spec) => {
                    max = max.max(if spec.cases.is_empty() { 6 } else { 7 });
                    plan_constant(&spec.attempts, 6, &mut max, &mut stack);
                    &spec.delay
                }
                ExtensionSpec::Loop(spec) => {
                    max = max.max(if spec.terminals.is_empty() { 6 } else { 7 });
                    if let Some(expired) = &spec.expired {
                        stack.push(Item::Ty(&expired.ty, 7));
                    }
                    if let Some(fuel) = &spec.fuel {
                        plan_constant(fuel, 6, &mut max, &mut stack);
                    }
                    if let Some(proof) = &spec.proof {
                        tree(&proof.measure, 7, &mut stack);
                        if let Some(invariant) = &proof.invariant {
                            tree(invariant, 7, &mut stack);
                        }
                        max = max.max(7);
                    }
                    &spec.delay
                }
            };
            match delay {
                None => {}
                Some(DelayPolicy::Fixed { after }) => plan_constant(after, 7, &mut max, &mut stack),
                Some(DelayPolicy::Exponential { first, ceiling }) => {
                    plan_constant(first, 7, &mut max, &mut stack);
                    plan_constant(ceiling, 7, &mut max, &mut stack);
                }
            }
        }
    }
    for (_, handler) in plan.handlers() {
        // Operator body at 3, handlers at 4, handler map at 5,
        // activation at 6 and its fields at 7.
        max = max.max(7);
        if let Some(guard) = &handler.guard {
            // tree_tree at 6, type and expression roots at 7.
            tree(guard, 7, &mut stack);
        }
        stack.push(Item::Dispatch(&handler.dispatch, 6));
    }
    while let Some(item) = stack.pop() {
        match item {
            Item::Dispatch(dispatch, level) => {
                use super::Dispatch;
                max = max.max(level + 1);
                let steps = match dispatch {
                    Dispatch::Linear(steps) => steps,
                    Dispatch::ClosedMatch {
                        prefix,
                        scrutinee,
                        arms,
                    } => {
                        tree(scrutinee, level + 2, &mut stack);
                        for arm in arms {
                            max = max.max(level + 4);
                            stack.push(Item::Dispatch(&arm.dispatch, level + 3));
                        }
                        prefix
                    }
                    Dispatch::OpenMatch {
                        prefix,
                        arms,
                        fallback,
                    } => {
                        for arm in arms {
                            tree(&arm.predicate, level + 4, &mut stack);
                            stack.push(Item::Dispatch(&arm.dispatch, level + 3));
                        }
                        stack.push(Item::Dispatch(fallback, level + 1));
                        prefix
                    }
                };
                stack.extend(steps.iter().map(|step| Item::Step(step, level + 2)));
            }
            Item::Step(step, level) => {
                max = max.max(level + 1);
                match step {
                    DispatchStep::Let {
                        value: expr,
                        annotations: table,
                        ..
                    } => {
                        tree(expr, level + 2, &mut stack);
                        annotations(table, level + 1, &mut max, &mut stack);
                    }
                    DispatchStep::SetVar { value: expr, .. } => tree(expr, level + 2, &mut stack),
                    DispatchStep::Stop => {}
                    DispatchStep::Emit { value, .. } => {
                        emission(value, level + 1, &mut max, &mut stack)
                    }
                    DispatchStep::MultiEmit { values, .. } => {
                        max = max.max(level + 2);
                        match values {
                            super::MultiEmitValues::Broadcast(value) => {
                                emission(value, level + 2, &mut max, &mut stack)
                            }
                            super::MultiEmitValues::Tuple(values) => {
                                for value in values {
                                    emission(value, level + 3, &mut max, &mut stack);
                                }
                            }
                        }
                    }
                    DispatchStep::LoopCommand(command) => {
                        max = max.max(level + 2);
                        if let super::LoopCommand::Next(value) = command {
                            emission(value, level + 2, &mut max, &mut stack);
                        }
                    }
                }
            }
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
                max = max.max(level + tagged_value_level(value));
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
                | DataType::MediaType
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
        max = max.max(4 + annotation_table_level(port.annotations()));
    }
    let mut slot = 0_u32;
    while let Some(row) = resolved.node(NodeIndex::new(slot)) {
        max = max.max(3);
        for port in row {
            max = max.max(5 + data_type_level(port.ty()));
            max = max.max(5 + annotation_table_level(port.annotations()));
        }
        let Some(next) = slot.checked_add(1) else {
            break;
        };
        slot = next;
    }
    max
}

fn annotation_table_level(table: &AnnotationTable) -> usize {
    if table.is_empty() {
        return 0;
    }
    let mut max = 4;
    for set in table.entries().values() {
        for annotation in set.annotations() {
            if let DataAnnotation::Declared(application) = annotation {
                max = max.max(4 + data_type_level(application.argument_type()));
                max = max.max(4 + tagged_value_level(application.arguments()));
            }
        }
    }
    max
}

/// Exact depth of the shared tagged-value encoding, without constructing CBOR.
fn tagged_value_level(value: &Value) -> usize {
    let mut pending = vec![(value, 0)];
    let mut max = 1;
    while let Some((value, level)) = pending.pop() {
        max = max.max(level + 1);
        match value {
            Value::Array(items) => pending.extend(items.iter().map(|item| (item, level + 2))),
            Value::Map(entries) => {
                for value in entries.values() {
                    max = max.max(level + 4); // tagged key at pair + 1
                    pending.push((value, level + 3));
                }
            }
            Value::KeyedMap(entries) => {
                for (key, value) in entries {
                    pending.push((key, level + 3));
                    pending.push((value, level + 3));
                }
            }
            Value::Struct(fields) => {
                for (_, value) in fields {
                    max = max.max(level + 3); // bare field name
                    pending.push((value, level + 3));
                }
            }
            Value::Decimal { .. } => max = max.max(level + 2),
            Value::StructuralUnion { value, .. } => pending.push((value, level + 2)),
            Value::TaggedUnion(tagged) => {
                max = max.max(level + 3);
                for (_, value) in &tagged.payload {
                    max = max.max(level + 5);
                    pending.push((value, level + 5));
                }
            }
            _ => {}
        }
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
            | DataType::MediaType
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
    let nodes = array_of(get(pairs, 12, "plan nodes")?, "plan nodes")?
        .iter()
        .map(node_from)
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
    Ok(ExecutionPlan::new(nodes, app_schema, ext_schema, assets))
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
    let fields = map_of(tree, "Node")?;
    let path_text = text_of(get(fields, 1, "Node path")?, "Node path")?;
    let path = if path_text.is_empty() {
        node::OwnerPath::root()
    } else {
        node::OwnerPath::new(path_text.split('.')).map_err(|_| err("Node path"))?
    };
    let parent = match get(fields, 2, "Node parent")? {
        Cbor::Null => None,
        value => Some(NodeIndex::new(u32_from(value, "Node parent")?)),
    };
    let ports = array_of(get(fields, 3, "Node ports")?, "Node ports")?
        .iter()
        .map(port_decl_from)
        .collect::<Result<Vec<_>, _>>()?;
    let instance = instance_from(get(fields, 4, "Node instance")?)?;
    let vals = array_of(get(fields, 5, "Node vals")?, "Node vals")?
        .iter()
        .map(owner_value_from)
        .collect::<Result<Vec<_>, _>>()?;
    let extensions = array_of(get(fields, 6, "Node extensions")?, "Node extensions")?
        .iter()
        .map(extension_spec_from)
        .collect::<Result<Vec<_>, _>>()?;
    let body = match array_of(get(fields, 7, "Node body")?, "Node body")? {
        [Cbor::Uint(1), wiring] => NodeKind::Graph(Box::new(crate::plan::Graph {
            wiring: array_of(wiring, "Graph wiring")?
                .iter()
                .map(fields::route_from)
                .collect::<Result<_, _>>()?,
        })),
        [Cbor::Uint(2), vars, handlers] => NodeKind::Operator(Box::new(Operator {
            vars: array_of(vars, "Operator vars")?
                .iter()
                .map(var_from)
                .collect::<Result<Vec<_>, _>>()?,
            handlers: array_of(handlers, "Operator handlers")?
                .iter()
                .map(handler_from)
                .collect::<Result<Vec<_>, _>>()?,
        })),
        [Cbor::Uint(3), spec] => NodeKind::Connector(Box::new(spec_from(spec)?)),
        _ => return Err(err("Node body")),
    };
    Ok(Node {
        path,
        parent,
        ports,
        instance,
        vals,
        extensions,
        body,
    })
}

fn extension_spec_from(tree: &Cbor) -> Result<ExtensionSpec, DecodeError> {
    match array_of(tree, "Extension specification")? {
        [Cbor::Uint(1)] => Ok(ExtensionSpec::App),
        [Cbor::Uint(2), spec] => Ok(ExtensionSpec::Loop(Box::new(loop_spec_from(spec)?))),
        [Cbor::Uint(3), spec] => Ok(ExtensionSpec::Retry(Box::new(retry_spec_from(spec)?))),
        _ => Err(err("Extension specification")),
    }
}

fn retry_spec_from(tree: &Cbor) -> Result<super::RetrySpec, DecodeError> {
    let fields = map_of(tree, "Retry specification")?;
    if fields.keys().any(|key| !matches!(key, 1..=5)) {
        return Err(err("unknown Retry field"));
    }
    Ok(super::RetrySpec {
        attempts: plan_constant_from(get(fields, 1, "Retry attempts")?)?,
        cases: array_of(get(fields, 2, "Retry cases")?, "Retry cases")?
            .iter()
            .map(|value| text_of(value, "Retry case").map(ToString::to_string))
            .collect::<Result<_, _>>()?,
        delay: find(fields, 3).map(loop_delay_from).transpose()?,
        entry: node::Port::new(text_of(get(fields, 4, "Retry entry")?, "Retry entry")?)
            .map_err(|_| err("Retry entry"))?,
        terminal: node::Port::new(text_of(
            get(fields, 5, "Retry terminal")?,
            "Retry terminal",
        )?)
        .map_err(|_| err("Retry terminal"))?,
    })
}

fn loop_spec_from(tree: &Cbor) -> Result<LoopSpec, DecodeError> {
    let fields = map_of(tree, "Loop specification")?;
    Ok(LoopSpec {
        entry: node::Port::new(text_of(get(fields, 4, "Loop entry")?, "Loop entry")?)
            .map_err(|_| err("Loop entry"))?,
        terminals: array_of(get(fields, 5, "Loop terminals")?, "Loop terminals")?
            .iter()
            .map(|port| {
                node::Port::new(text_of(port, "Loop terminal")?).map_err(|_| err("Loop terminal"))
            })
            .collect::<Result<_, _>>()?,
        fuel: find(fields, 1).map(plan_constant_from).transpose()?,
        delay: find(fields, 2).map(loop_delay_from).transpose()?,
        proof: find(fields, 3).map(loop_proof_from).transpose()?,
        expired: find(fields, 6)
            .map(|value| {
                let [port, ty] = array_of(value, "Loop exhaustion target")? else {
                    return Err(err("Loop exhaustion target"));
                };
                Ok(super::LoopExpiredOutput {
                    port: node::Port::new(text_of(port, "Loop exhaustion port")?)
                        .map_err(|_| err("Loop exhaustion port"))?,
                    ty: type_from(ty)?,
                })
            })
            .transpose()?,
    })
}

fn loop_proof_from(tree: &Cbor) -> Result<crate::loop_proof::Certificate, DecodeError> {
    let fields = map_of(tree, "Loop proof")?;
    let decreases = array_of(
        get(fields, 3, "Loop proof decreases")?,
        "Loop proof decreases",
    )?
    .iter()
    .map(|value| match value {
        Cbor::Uint(value) => Ok(*value),
        _ => Err(err("Loop proof decrease")),
    })
    .collect::<Result<Vec<_>, _>>()?;
    Ok(crate::loop_proof::Certificate {
        measure: tree_from(get(fields, 1, "Loop proof measure")?)?,
        invariant: find(fields, 2).map(tree_from).transpose()?,
        decreases,
    })
}

fn loop_delay_from(tree: &Cbor) -> Result<DelayPolicy, DecodeError> {
    match array_of(tree, "Loop delay")? {
        [Cbor::Uint(1), after] => Ok(DelayPolicy::Fixed {
            after: plan_constant_from(after)?,
        }),
        [Cbor::Uint(2), first, ceiling] => Ok(DelayPolicy::Exponential {
            first: plan_constant_from(first)?,
            ceiling: plan_constant_from(ceiling)?,
        }),
        _ => Err(err("Loop delay")),
    }
}

fn plan_constant_from(tree: &Cbor) -> Result<PlanConstant, DecodeError> {
    match array_of(tree, "plan constant")? {
        [expr, resolved] => Ok(PlanConstant {
            expr: tree_from(expr)?,
            resolved: value_from(resolved)?,
        }),
        _ => Err(err("plan constant")),
    }
}

fn instance_from(tree: &Cbor) -> Result<instance::InstanceSpec, DecodeError> {
    let fields = map_of(tree, "instance")?;
    Ok(instance::InstanceSpec {
        key: find(fields, 1).map(tree_from).transpose()?,
        lifetime: lifetime_from(get(fields, 2, "instance lifetime")?)?,
        closing: find(fields, 3).map(closing_from).transpose()?,
    })
}

fn closing_from(tree: &Cbor) -> Result<instance::Closing, DecodeError> {
    match array_of(tree, "instance closing")? {
        [drain_for, settle_for] => Ok(instance::Closing {
            drain_for: signed_from(drain_for)?,
            settle_for: signed_from(settle_for)?,
        }),
        _ => Err(err("instance closing")),
    }
}

fn lifetime_from(tree: &Cbor) -> Result<instance::Lifetime, DecodeError> {
    match array_of(tree, "instance lifetime")? {
        [Cbor::Uint(1)] => Ok(instance::Lifetime::Persistent),
        [Cbor::Uint(2), nanos] => Ok(instance::Lifetime::Idle(signed_from(nanos)?)),
        [Cbor::Uint(3), nanos] => Ok(instance::Lifetime::Fixed(signed_from(nanos)?)),
        [Cbor::Uint(4)] => Ok(instance::Lifetime::SinglePass),
        [Cbor::Uint(5), nanos] => Ok(instance::Lifetime::Tumbling(signed_from(nanos)?)),
        _ => Err(err("instance lifetime")),
    }
}

fn site_from(tree: &Cbor) -> Result<EvaluationSite, DecodeError> {
    match tree {
        Cbor::Uint(1) => Ok(EvaluationSite::Plan),
        Cbor::Uint(2) => Ok(EvaluationSite::Boot),
        Cbor::Uint(3) => Ok(EvaluationSite::Instance),
        Cbor::Uint(4) => Ok(EvaluationSite::Invocation),
        _ => Err(err("evaluation site")),
    }
}

fn annotation_table_from(tree: &Cbor) -> Result<AnnotationTable, DecodeError> {
    let entries = array_of(tree, "annotation table")?
        .iter()
        .map(|entry| match array_of(entry, "annotation row")? {
            [path, annotations] => Ok((schema_path_from(path)?, annotation_set_from(annotations)?)),
            _ => Err(err("annotation row")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    AnnotationTable::new(entries)
        .map_err(|error| DecodeError::Malformed(format!("invalid annotation table: {error}")))
}

fn schema_path_from(tree: &Cbor) -> Result<SchemaPath, DecodeError> {
    let segments = array_of(tree, "schema path")?
        .iter()
        .map(|segment| match array_of(segment, "schema path segment")? {
            [Cbor::Uint(1), Cbor::Text(name)] => Ok(SchemaPathSegment::Field(name.clone())),
            [Cbor::Uint(2)] => Ok(SchemaPathSegment::ArrayElement),
            [Cbor::Uint(3)] => Ok(SchemaPathSegment::MapKey),
            [Cbor::Uint(4)] => Ok(SchemaPathSegment::MapValue),
            [Cbor::Uint(5), Cbor::Uint(index)] => Ok(SchemaPathSegment::StructuralUnionBranch(
                u16::try_from(*index).map_err(|_| err("structural union branch index"))?,
            )),
            [Cbor::Uint(6), Cbor::Text(name)] => {
                Ok(SchemaPathSegment::TaggedUnionCase(name.clone()))
            }
            _ => Err(err("schema path segment")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SchemaPath::new(segments))
}

fn annotation_set_from(tree: &Cbor) -> Result<AnnotationSet, DecodeError> {
    let annotations = array_of(tree, "annotation set")?
        .iter()
        .map(annotation_from)
        .collect::<Result<Vec<_>, _>>()?;
    AnnotationSet::new(annotations)
        .map_err(|error| DecodeError::Malformed(format!("invalid annotation set: {error}")))
}

fn annotation_from(tree: &Cbor) -> Result<DataAnnotation, DecodeError> {
    match array_of(tree, "annotation")? {
        [
            Cbor::Uint(4),
            Cbor::Uint(binding),
            Cbor::Text(definition),
            argument_type,
            arguments,
        ] => {
            let binding = match binding {
                1 => AnnotationBinding::Metadata,
                2 => AnnotationBinding::Measure,
                _ => return Err(err("unsupported annotation interpretation")),
            };
            let application = AnnotationApplication::new(
                binding,
                definition.clone(),
                type_from(argument_type)?,
                value_from(arguments)?,
            )
            .map_err(|error| {
                DecodeError::Malformed(format!("invalid annotation application: {error}"))
            })?;
            Ok(DataAnnotation::Declared(Box::new(application)))
        }
        [Cbor::Uint(1), Cbor::Uint(level)] => Ok(DataAnnotation::Sensitive(match level {
            1 => SensitivityLevel::Internal,
            2 => SensitivityLevel::Confidential,
            3 => SensitivityLevel::Restricted,
            _ => return Err(err("sensitivity level")),
        })),
        [Cbor::Uint(2)] => Ok(DataAnnotation::Secret),
        [Cbor::Uint(3), Cbor::Uint(flags)] if *flags <= 0b111 => Ok(DataAnnotation::Personal(
            PersonalDataClass::new(flags & 0b001 != 0, flags & 0b010 != 0, flags & 0b100 != 0),
        )),
        _ => Err(err("annotation")),
    }
}

fn owner_value_from(tree: &Cbor) -> Result<OwnerValue, DecodeError> {
    let fields = map_of(tree, "owner value")?;
    Ok(OwnerValue {
        name: text_of(get(fields, 1, "owner value name")?, "owner value name")?.to_string(),
        site: site_from(get(fields, 2, "owner value site")?)?,
        expr: tree_from(get(fields, 3, "owner value expression")?)?,
        resolved: find(fields, 4).map(value_from).transpose()?,
        annotations: find(fields, 5)
            .map(annotation_table_from)
            .transpose()?
            .unwrap_or_else(AnnotationTable::empty),
    })
}

fn spec_from(tree: &Cbor) -> Result<ConnectorSpec, DecodeError> {
    let pairs = map_of(tree, "connector spec")?;
    if pairs.keys().any(|key| !matches!(key, 1..=6)) {
        return Err(err("unknown Connector field"));
    }
    let contract =
        crate::connector::ContractId::parse(text_of(get(pairs, 1, "contract")?, "contract")?)
            .map_err(|reason| DecodeError::Malformed(format!("invalid connector id: {reason}")))?;
    let behavior = match get(pairs, 2, "behavior")? {
        Cbor::Uint(1) => node::Behavior::Deterministic,
        Cbor::Uint(2) => node::Behavior::Probabilistic,
        _ => return Err(err("behavior")),
    };
    let mut attributes = Vec::new();
    for entry in array_of(get(pairs, 3, "attributes")?, "attributes")? {
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
    let Cbor::Uint(role_tag) = get(pairs, 4, "connector roles")? else {
        return Err(err("connector roles"));
    };
    let role_tag = u8::try_from(*role_tag).map_err(|_| err("connector roles"))?;
    let roles = crate::connector::ConnectorDirection::from_tag(role_tag)
        .ok_or_else(|| err("connector roles"))?;
    let Cbor::Uint(exchange_tag) = get(pairs, 5, "connector exchange")? else {
        return Err(err("connector exchange"));
    };
    let exchange_tag = u8::try_from(*exchange_tag).map_err(|_| err("connector exchange"))?;
    let exchange = crate::connector::ExchangeKind::from_tag(exchange_tag)
        .ok_or_else(|| err("connector exchange"))?;
    Ok(ConnectorSpec {
        contract,
        roles,
        exchange,
        idempotent: find(pairs, 6).map(plan_constant_from).transpose()?,
        behavior,
        attributes,
    })
}

fn attr_from(tree: &Cbor) -> Result<AttrSetting, DecodeError> {
    match array_of(tree, "attr setting")? {
        [Cbor::Uint(1), Cbor::Text(word)] => Ok(AttrSetting::Word(word.clone())),
        [Cbor::Uint(2), value] => Ok(AttrSetting::Value(value_from(value)?)),
        [Cbor::Uint(3), site, expr] => Ok(AttrSetting::Expr {
            site: site_from(site)?,
            expr: tree_from(expr)?,
        }),
        _ => Err(err("attr setting")),
    }
}

fn port_decl_from(tree: &Cbor) -> Result<PortDecl, DecodeError> {
    let (name, direction, ty, annotations) = match array_of(tree, "port declaration")? {
        [Cbor::Text(name), Cbor::Uint(direction)] => {
            (name, direction, None, AnnotationTable::empty())
        }
        [Cbor::Text(name), Cbor::Uint(direction), ty] => (
            name,
            direction,
            Some(type_from(ty)?),
            AnnotationTable::empty(),
        ),
        [Cbor::Text(name), Cbor::Uint(direction), ty, annotations] => (
            name,
            direction,
            match ty {
                Cbor::Null => None,
                ty => Some(type_from(ty)?),
            },
            annotation_table_from(annotations)?,
        ),
        _ => return Err(err("port declaration")),
    };
    let direction = match direction {
        1 => node::PortDirection::Input,
        2 => node::PortDirection::Output,
        _ => return Err(err("port direction")),
    };
    Ok(PortDecl {
        port: node::Port::new(name.clone()).map_err(|_| err("port name"))?,
        direction,
        ty,
        annotations,
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
    let annotations = find(pairs, 4)
        .map(annotation_table_from)
        .transpose()?
        .unwrap_or_else(AnnotationTable::empty);
    Ok(VarCell {
        name,
        ty,
        init,
        annotations,
    })
}

fn u32_from(tree: &Cbor, detail: &str) -> Result<u32, DecodeError> {
    let Cbor::Uint(value) = tree else {
        return Err(err(detail));
    };
    u32::try_from(*value).map_err(|_| err(detail))
}

fn handler_from(tree: &Cbor) -> Result<Handler, DecodeError> {
    let pairs = map_of(tree, "handler")?;
    let activation = match array_of(get(pairs, 2, "activation")?, "activation")? {
        [Cbor::Uint(1), Cbor::Text(port)] => node::HandlerActivation::Port(
            node::Port::new(port.clone()).map_err(|_| err("activation port"))?,
        ),
        [Cbor::Uint(2), Cbor::Uint(lifecycle)] => node::HandlerActivation::Lifecycle(
            node::Lifecycle::try_from(
                u8::try_from(*lifecycle).map_err(|_| err("activation lifecycle"))?,
            )
            .map_err(|_| err("activation lifecycle"))?,
        ),
        [Cbor::Uint(3), ports, within, last] => node::HandlerActivation::Join {
            ports: array_of(ports, "join ports")?
                .iter()
                .map(|port| match port {
                    Cbor::Text(port) => node::Port::new(port.clone()).map_err(|_| err("join port")),
                    _ => Err(err("join port")),
                })
                .collect::<Result<Vec<_>, _>>()?,
            within_ns: match within {
                Cbor::Null => None,
                value => Some(signed_from(value)?),
            },
            last: match last {
                Cbor::Null => None,
                Cbor::Uint(value) => Some(u32::try_from(*value).map_err(|_| err("join last"))?),
                _ => return Err(err("join last")),
            },
        },
        _ => return Err(err("activation")),
    };
    let guard = match find(pairs, 3) {
        Some(tree) => Some(tree_from(tree)?),
        None => None,
    };
    let dispatch = fields::dispatch_from(get(pairs, 4, "handler dispatch")?, 0)?;
    Ok(Handler {
        activation,
        guard,
        dispatch,
    })
}

fn step_from(tree: &Cbor) -> Result<DispatchStep, DecodeError> {
    match array_of(tree, "step")? {
        [Cbor::Uint(1), Cbor::Text(name), expr] => Ok(DispatchStep::Let {
            name: name.clone(),
            value: tree_from(expr)?,
            annotations: AnnotationTable::empty(),
        }),
        [Cbor::Uint(1), Cbor::Text(name), expr, annotations] => Ok(DispatchStep::Let {
            name: name.clone(),
            value: tree_from(expr)?,
            annotations: annotation_table_from(annotations)?,
        }),
        [Cbor::Uint(2), Cbor::Text(port), emission] => Ok(DispatchStep::Emit {
            port: node::Port::new(port.clone()).map_err(|_| err("emit port"))?,
            value: emission_from(emission)?,
        }),
        [Cbor::Uint(3), Cbor::Text(name), expr] => Ok(DispatchStep::SetVar {
            name: name.clone(),
            value: tree_from(expr)?,
        }),
        [Cbor::Uint(4)] => Ok(DispatchStep::Stop),
        _ => Err(err("step")),
    }
}

fn emission_from(tree: &Cbor) -> Result<EmitValue, DecodeError> {
    match array_of(tree, "emission")? {
        [Cbor::Uint(1)] => Ok(EmitValue::Identity),
        [Cbor::Uint(2), writes] => Ok(EmitValue::Patch(writes_from(writes)?)),
        [Cbor::Uint(3), writes] => Ok(EmitValue::FreshData(writes_from(writes)?)),
        [Cbor::Uint(4), tree] => Ok(EmitValue::Data(tree_from(tree)?)),
        [Cbor::Uint(5)] => Ok(EmitValue::Nothing),
        _ => Err(err("emission")),
    }
}

fn writes_from(tree: &Cbor) -> Result<Vec<FieldWrite>, DecodeError> {
    array_of(tree, "writes")?
        .iter()
        .map(|entry| match array_of(entry, "write")? {
            [target, expr] => Ok(FieldWrite {
                target: target_from(target)?,
                value: tree_from(expr)?,
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
        [Cbor::Uint(3), Cbor::Text(field)] => Ok(WriteTarget::Ext(field.clone())),
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
                6 => expr::Function::MediaType,
                7 => expr::Function::Has,
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

fn value_from(tree: &Cbor) -> Result<Value, DecodeError> {
    crate::canonical::decode_value(tree).map_err(DecodeError::Read)
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
    use crate::datatype::{
        ArrayType, BinaryType, ByteLength, DataType, DateUnit, DecimalType, EnumType, FloatType,
        IntegerType, IntegerWidth, IntervalUnit, StructType, TaggedCase, TaggedUnionType,
        TemporalType, TimeUnit, TimeZone, TypeField,
    };
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

    #[test]
    fn handlers_belong_to_operator_bodies_and_reject_unknown_activations() {
        let handler = Handler {
            activation: node::HandlerActivation::Port(node::Port::new("in").unwrap()),
            guard: None,
            dispatch: crate::plan::Dispatch::Linear(vec![]),
        };
        let tree = handler_tree(&handler);
        assert_eq!(
            canonical_cbor::encode(&tree),
            [0xa2, 2, 0x82, 1, 0x62, b'i', b'n', 4, 0x82, 1, 0x80]
        );
        assert_eq!(handler_from(&tree).unwrap(), handler);
        let Cbor::Map(mut graph) = node_tree(&root_owner()) else {
            panic!("Node map")
        };
        graph.insert(
            7,
            Cbor::Array(vec![Cbor::Uint(1), Cbor::Array(vec![tree.clone()])]),
        );
        assert!(
            node_from(&Cbor::Map(graph)).is_err(),
            "a Graph cannot carry handlers"
        );
        let Cbor::Map(mut fields) = tree else {
            panic!("handler map")
        };
        fields.insert(
            2,
            Cbor::Array(vec![
                Cbor::Uint(4),
                Cbor::Text(text("child")),
                Cbor::Text(text("out")),
            ]),
        );
        assert_eq!(handler_from(&Cbor::Map(fields)), Err(err("activation")));
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

    fn root_owner() -> Node {
        Node {
            path: node::OwnerPath::root(),
            parent: None,
            body: NodeKind::Graph(Box::default()),
            ports: vec![],
            extensions: vec![ExtensionSpec::App],
            instance: instance::InstanceSpec {
                key: None,
                lifetime: instance::Lifetime::Persistent,
                closing: None,
            },
            vals: vec![],
        }
    }

    fn node_owner(name: &str) -> Node {
        Node {
            path: node::OwnerPath::new([name]).unwrap(),
            parent: Some(node::NodeIndex::new(0)),
            body: NodeKind::Operator(Box::new(Operator {
                vars: vec![],
                handlers: vec![],
            })),
            ports: vec![],
            extensions: vec![],
            instance: instance::InstanceSpec {
                key: None,
                lifetime: instance::Lifetime::Persistent,
                closing: None,
            },
            vals: vec![],
        }
    }

    fn instance_package() -> Vec<u8> {
        let mut instance_owner = node_owner("worker");
        instance_owner.instance = instance::InstanceSpec {
            key: Some(expr::Tree {
                root: expr::Expr::Path(vec![text("meta"), text("sys"), text("id")]),
                ty: DataType::Text,
            }),
            lifetime: instance::Lifetime::Idle(10),
            closing: None,
        };
        let validated = ValidatedExecutionPlan::try_new(ExecutionPlan {
            routes: vec![],
            nodes: vec![root_owner(), instance_owner],
            edges: vec![],
            app_schema: vec![],
            ext_schema: vec![],
            assets: vec![],
        })
        .expect("the instance probe validates");
        encode(&validated, &SemanticHash::from_bytes([0; 32]))
    }

    fn mutate_package_body(package: &[u8], edit: impl FnOnce(&mut BTreeMap<u64, Cbor>)) -> Vec<u8> {
        let (Cbor::Map(mut envelope), _) =
            canonical_cbor::decode(&package[MAGIC.len()..]).expect("envelope")
        else {
            panic!("map envelope");
        };
        let Cbor::Bytes(body) = envelope.get(&3).expect("body field") else {
            panic!("bytes body");
        };
        let (Cbor::Map(mut body), _) = canonical_cbor::decode(body).expect("plan body") else {
            panic!("map body");
        };
        edit(&mut body);
        let body = canonical_cbor::encode(&Cbor::Map(body));
        envelope.insert(3, Cbor::Bytes(body.clone()));
        envelope.insert(
            4,
            Cbor::Bytes(checksum(FORMAT_VERSION, &[0; 32], &body).to_vec()),
        );
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&canonical_cbor::encode(&Cbor::Map(envelope)));
        bytes
    }

    fn one_annotation(annotation: DataAnnotation) -> AnnotationSet {
        AnnotationSet::new(vec![annotation])
            .unwrap_or_else(|error| panic!("one annotation is canonical: {error}"))
    }

    fn annotated_boundary_schema(
        ty: DataType,
        annotations: AnnotationTable,
    ) -> ValidatedExecutionPlan {
        ValidatedExecutionPlan::try_new(ExecutionPlan {
            routes: vec![],
            nodes: vec![Node {
                ports: vec![PortDecl {
                    port: port("in"),
                    direction: node::PortDirection::Input,
                    ty: Some(ty),
                    annotations,
                }],
                ..root_owner()
            }],
            edges: vec![],
            app_schema: vec![],
            ext_schema: vec![],
            assets: vec![],
        })
        .unwrap_or_else(|error| panic!("annotated boundary plan validates: {error}"))
    }

    fn annotated_boundary_plan(annotation: DataAnnotation) -> ValidatedExecutionPlan {
        let annotations =
            AnnotationTable::new(vec![(SchemaPath::root(), one_annotation(annotation))])
                .unwrap_or_else(|error| panic!("one annotation row is canonical: {error}"));
        annotated_boundary_schema(DataType::INT, annotations)
    }

    #[test]
    fn declared_annotation_transport_checks_authority_flatness_and_typed_arguments() {
        let ty = DataType::Struct(
            StructType::new(vec![TypeField::new("text", DataType::Text)]).expect("flat schema"),
        );
        let value = Value::Struct(vec![(text("text"), Value::Text(text("description")))]);
        let wire = Cbor::Array(vec![
            Cbor::Uint(4),
            Cbor::Uint(1),
            Cbor::Text(text("note")),
            type_tree(&ty),
            canonical::value(&value),
        ]);
        let annotation = annotation_from(&wire).expect("explicit descriptive binding");
        let DataAnnotation::Declared(application) = &annotation else {
            panic!("declared metadata")
        };
        assert_eq!(application.argument_type(), &ty);
        assert_eq!(application.arguments(), &value);
        let Cbor::Array(fields) = &wire else {
            panic!("application tuple")
        };
        for (index, replacement) in [
            (1, Cbor::Uint(0)),
            (1, Cbor::Uint(2)), // a descriptive schema cannot acquire measure authority
            (1, Cbor::Uint(255)),
            (2, Cbor::Text(text("secret"))),
            (2, Cbor::Text(text("custom.personal"))),
            (2, Cbor::Text(text("std/measure@2"))),
            (3, type_tree(&DataType::Variant)),
            (
                3,
                type_tree(&DataType::Struct(
                    StructType::new(vec![TypeField::new(
                        "text",
                        DataType::array(DataType::Text),
                    )])
                    .expect("ordinary nested schema"),
                )),
            ),
            (4, canonical::value(&Value::Struct(vec![]))),
            (
                4,
                canonical::value(&Value::Struct(vec![(text("text"), Value::Int(7))])),
            ),
            (
                4,
                canonical::value(&Value::Struct(vec![
                    (text("text"), Value::Text(text("a"))),
                    (text("extra"), Value::Null),
                ])),
            ),
        ] {
            let mut altered = fields.clone();
            altered[index] = replacement;
            assert!(annotation_from(&Cbor::Array(altered)).is_err());
        }
        let plan = annotated_boundary_plan(annotation);
        let package = encode(&plan, &SemanticHash::from_bytes([0; 32]));
        assert_eq!(
            decode(&package).expect("validated application package").0,
            plan
        );
    }

    #[test]
    fn declared_annotation_depth_includes_types_and_constant_values() {
        for (field_type, value) in [
            (DataType::Text, Value::Text(text("label"))),
            (
                DataType::Decimal(DecimalType::new(38, 18).expect("decimal")),
                Value::Decimal {
                    unscaled: 12,
                    scale: 18,
                },
            ),
            (
                DataType::structural_union(vec![DataType::Null, DataType::Text]).expect("optional"),
                Value::StructuralUnion {
                    branch: 0,
                    value: Box::new(Value::Null),
                },
            ),
        ] {
            let ty = DataType::Struct(
                StructType::new(vec![TypeField::new("value", field_type)]).expect("flat schema"),
            );
            let annotation = DataAnnotation::Declared(Box::new(
                AnnotationApplication::new(
                    AnnotationBinding::Metadata,
                    text("note"),
                    ty,
                    Value::Struct(vec![(text("value"), value)]),
                )
                .expect("typed application"),
            ));
            let plan = annotated_boundary_plan(annotation);
            assert_eq!(deepest_level(plan.plan()), levels(&plan_tree(plan.plan())));
            let table = &plan.plan().boundary()[0].annotations;
            assert_eq!(
                annotation_table_level(table),
                levels(&annotation_table_tree(table))
            );
            let mut body = plan_tree(plan.plan());
            let Cbor::Map(fields) = &mut body else {
                panic!("plan map")
            };
            fields.insert(8, resolved_table_tree(plan.resolved_ports()));
            assert_eq!(
                deepest_level_with_resolved(plan.plan(), plan.resolved_ports()),
                levels(&body)
            );
        }
    }

    #[test]
    fn annotation_tags_outside_the_closed_registry_are_rejected() {
        for tag in [0, 5, 6, u64::MAX] {
            let error = annotation_from(&Cbor::Array(vec![Cbor::Uint(tag)]))
                .expect_err("only the four declared annotation forms are executable");
            assert!(error.to_string().contains("annotation"), "{error}");
        }
    }

    #[test]
    fn declared_annotations_require_the_complete_typed_application() {
        for fields in [
            vec![Cbor::Uint(4), Cbor::Uint(1)],
            vec![Cbor::Uint(4), Cbor::Uint(2), Cbor::Text(text("USD"))],
        ] {
            let error = annotation_from(&Cbor::Array(fields))
                .expect_err("a tag and a binding alone do not define an annotation");
            assert!(error.to_string().contains("annotation"), "{error}");
        }
    }

    #[test]
    fn business_measure_transport_rejects_forged_profiles_and_targets() {
        let ty = crate::schema_annotation::measure_argument_type().expect("standard schema");
        let DataType::Struct(schema) = &ty else {
            panic!("standard struct")
        };
        let constants = |fields: Vec<(String, Value)>| {
            Value::Struct(
                fields
                    .into_iter()
                    .map(|(name, value)| {
                        let field = schema
                            .fields()
                            .iter()
                            .find(|field| field.name() == name)
                            .expect("standard field");
                        (
                            name,
                            field
                                .ty()
                                .narrow(value, None)
                                .expect("typed field constant"),
                        )
                    })
                    .collect(),
            )
        };
        let arguments = constants(vec![
            (text("id"), Value::Text(text("carrots"))),
            (text("unit"), Value::Text(text("item"))),
        ]);
        let wire = vec![
            Cbor::Uint(4),
            Cbor::Uint(2),
            Cbor::Text(text("std/measure@3")),
            type_tree(&ty),
            canonical::value(&arguments),
        ];
        let annotation =
            annotation_from(&Cbor::Array(wire.clone())).expect("supported business consumer");
        let plan = annotated_boundary_plan(annotation);
        let package = encode(&plan, &SemanticHash::from_bytes([0; 32]));
        assert_eq!(
            decode(&package).expect("complete independent validation").0,
            plan
        );
        assert_eq!(deepest_level(plan.plan()), levels(&plan_tree(plan.plan())));
        for (index, replacement) in [
            (1, Cbor::Uint(1)),
            (1, Cbor::Uint(3)),
            (2, Cbor::Text(text("measure"))),
            (2, Cbor::Text(text("custom_measure"))),
            (2, Cbor::Text(text("std/measure@2"))),
            (3, type_tree(&DataType::Variant)),
            (4, canonical::value(&Value::Struct(vec![]))),
        ] {
            let mut altered = wire.clone();
            altered[index] = replacement;
            assert!(annotation_from(&Cbor::Array(altered)).is_err());
        }
        for (field, value) in [
            ("unit", Value::Null),
            ("unit", Value::Text(text(""))),
            ("id", Value::Text(text("Bad Name"))),
            ("unit", Value::Text(text(" item"))),
        ] {
            let Value::Struct(mut fields) = arguments.clone() else {
                panic!("struct")
            };
            let slot = fields
                .iter_mut()
                .find(|(name, _)| name == field)
                .expect("field");
            slot.1 = value;
            let value = Value::Struct(fields);
            let mut altered = wire.clone();
            altered[4] = canonical::value(&value);
            assert!(annotation_from(&Cbor::Array(altered)).is_err(), "{field}");
        }
        for target in [
            DataType::Text,
            DataType::Variant,
            DataType::Float(crate::datatype::FloatType::Binary64),
        ] {
            let mut forged = plan.plan().clone();
            forged.nodes[0].ports[0].ty = Some(target);
            assert!(
                ValidatedExecutionPlan::try_new(forged).is_err(),
                "numeric target checked independently"
            );
        }
    }

    #[test]
    fn one_metadata_definition_cannot_change_schema_between_plan_sites() {
        let note = |ty, value| {
            DataAnnotation::Declared(Box::new(
                AnnotationApplication::new(
                    AnnotationBinding::Metadata,
                    text("note"),
                    DataType::Struct(
                        StructType::new(vec![TypeField::new("value", ty)]).expect("flat schema"),
                    ),
                    Value::Struct(vec![(text("value"), value)]),
                )
                .expect("individually typed metadata"),
            ))
        };
        let original = annotated_boundary_plan(note(DataType::Text, Value::Text(text("a"))));
        let mut raw = original.plan().clone();
        let mut other = raw.boundary()[0].clone();
        other.port = port("other");
        other.annotations = AnnotationTable::new(vec![(
            SchemaPath::root(),
            one_annotation(note(DataType::Bool, Value::Bool(true))),
        )])
        .expect("other site");
        raw.nodes[0].ports.push(other);
        let error =
            ValidatedExecutionPlan::try_new(raw).expect_err("one declaring name has one schema");
        assert!(
            error.to_string().contains("inconsistent argument schemas"),
            "{error}"
        );
    }

    #[test]
    fn compact_policies_and_structural_paths_have_one_wire_spelling() {
        let root = AnnotationSet::new(vec![
            DataAnnotation::Sensitive(SensitivityLevel::Restricted),
            DataAnnotation::Secret,
        ])
        .unwrap_or_else(|error| panic!("closed annotation set: {error}"));
        let sensitive = || one_annotation(DataAnnotation::Sensitive(SensitivityLevel::Internal));
        let table = AnnotationTable::new(vec![
            (SchemaPath::root(), root),
            (
                SchemaPath::new(vec![SchemaPathSegment::Field(text("field"))]),
                sensitive(),
            ),
            (
                SchemaPath::new(vec![SchemaPathSegment::ArrayElement]),
                sensitive(),
            ),
            (
                SchemaPath::new(vec![SchemaPathSegment::MapKey]),
                sensitive(),
            ),
            (
                SchemaPath::new(vec![SchemaPathSegment::MapValue]),
                sensitive(),
            ),
            (
                SchemaPath::new(vec![SchemaPathSegment::StructuralUnionBranch(7)]),
                sensitive(),
            ),
            (
                SchemaPath::new(vec![SchemaPathSegment::TaggedUnionCase(text("Done"))]),
                sensitive(),
            ),
        ])
        .unwrap_or_else(|error| panic!("distinct structural paths: {error}"));
        let wire = annotation_table_tree(&table);
        assert_eq!(
            wire,
            Cbor::Array(vec![
                Cbor::Array(vec![
                    Cbor::Array(vec![]),
                    Cbor::Array(vec![
                        Cbor::Array(vec![Cbor::Uint(1), Cbor::Uint(3)]),
                        Cbor::Array(vec![Cbor::Uint(2)]),
                    ]),
                ]),
                Cbor::Array(vec![
                    Cbor::Array(vec![Cbor::Array(vec![
                        Cbor::Uint(1),
                        Cbor::Text(text("field")),
                    ])]),
                    Cbor::Array(vec![Cbor::Array(vec![Cbor::Uint(1), Cbor::Uint(1)])]),
                ]),
                Cbor::Array(vec![
                    Cbor::Array(vec![Cbor::Array(vec![Cbor::Uint(2)])]),
                    Cbor::Array(vec![Cbor::Array(vec![Cbor::Uint(1), Cbor::Uint(1)])]),
                ]),
                Cbor::Array(vec![
                    Cbor::Array(vec![Cbor::Array(vec![Cbor::Uint(3)])]),
                    Cbor::Array(vec![Cbor::Array(vec![Cbor::Uint(1), Cbor::Uint(1)])]),
                ]),
                Cbor::Array(vec![
                    Cbor::Array(vec![Cbor::Array(vec![Cbor::Uint(4)])]),
                    Cbor::Array(vec![Cbor::Array(vec![Cbor::Uint(1), Cbor::Uint(1)])]),
                ]),
                Cbor::Array(vec![
                    Cbor::Array(vec![Cbor::Array(vec![Cbor::Uint(5), Cbor::Uint(7)])]),
                    Cbor::Array(vec![Cbor::Array(vec![Cbor::Uint(1), Cbor::Uint(1)])]),
                ]),
                Cbor::Array(vec![
                    Cbor::Array(vec![Cbor::Array(vec![
                        Cbor::Uint(6),
                        Cbor::Text(text("Done")),
                    ])]),
                    Cbor::Array(vec![Cbor::Array(vec![Cbor::Uint(1), Cbor::Uint(1)])]),
                ]),
            ])
        );
        assert_eq!(annotation_table_from(&wire), Ok(table));
    }

    #[test]
    fn personal_data_flags_append_one_closed_annotation_wire_tag() {
        let set = AnnotationSet::new(vec![DataAnnotation::Personal(PersonalDataClass::new(
            true, true, true,
        ))])
        .unwrap_or_else(|error| panic!("personal annotation set: {error}"));
        let wire = annotation_set_tree(&set);
        assert_eq!(
            wire,
            Cbor::Array(vec![Cbor::Array(vec![Cbor::Uint(3), Cbor::Uint(0b111)])])
        );
        assert_eq!(
            annotation_set_from(&wire).unwrap_or_else(|error| panic!("personal wire: {error}")),
            set
        );
        assert!(annotation_from(&Cbor::Array(vec![Cbor::Uint(3), Cbor::Uint(8)])).is_err());
    }

    #[test]
    fn typed_annotations_round_trip_in_raw_and_resolved_port_tables() {
        let plan =
            annotated_boundary_plan(crate::schema_annotation::tests::measure("tokens", "token"));
        let package = encode(&plan, &SemanticHash::from_bytes([0; 32]));
        let (decoded, _) = decode(&package).expect("typed annotations read back");
        assert_eq!(decoded, plan);
        assert_eq!(
            decoded.plan().boundary()[0].annotations,
            *decoded.resolved_ports().boundary()[0].annotations()
        );
    }

    #[test]
    fn package_sabotage_cannot_erase_or_forge_resolved_annotations() {
        let plan =
            annotated_boundary_plan(DataAnnotation::Sensitive(SensitivityLevel::Confidential));
        let package = encode(&plan, &SemanticHash::from_bytes([0; 32]));

        let erase_claim = mutate_package_body(&package, |body| {
            let Cbor::Array(table) = body.get_mut(&8).unwrap_or_else(|| panic!("resolved table"))
            else {
                panic!("resolved table array");
            };
            let Cbor::Array(boundary) = &mut table[0] else {
                panic!("resolved boundary");
            };
            let Cbor::Array(port) = &mut boundary[0] else {
                panic!("resolved port");
            };
            let _ = port.pop();
        });
        assert_eq!(
            decode(&erase_claim),
            Err(DecodeError::ResolvedPortTableMismatch)
        );

        let erase_source = mutate_package_body(&package, |body| {
            let boundary = ports_mut(body, 0);
            let Cbor::Array(port) = &mut boundary[0] else {
                panic!("port declaration");
            };
            let _ = port.pop();
        });
        assert_eq!(
            decode(&erase_source),
            Err(DecodeError::ResolvedPortTableMismatch)
        );
    }

    #[test]
    fn sabotage_rederives_annotation_paths_and_measure_authority() {
        let plan =
            annotated_boundary_plan(crate::schema_annotation::tests::measure("tokens", "token"));
        let package = encode(&plan, &SemanticHash::from_bytes([0; 32]));

        let bad_path = mutate_package_body(&package, |body| {
            let boundary = ports_mut(body, 0);
            let Cbor::Array(port) = &mut boundary[0] else {
                panic!("port declaration");
            };
            let Cbor::Array(rows) = &mut port[3] else {
                panic!("annotation table");
            };
            let Cbor::Array(row) = &mut rows[0] else {
                panic!("annotation row");
            };
            row[0] = Cbor::Array(vec![Cbor::Array(vec![Cbor::Uint(2)])]);
        });
        assert!(matches!(
            decode(&bad_path),
            Err(DecodeError::Invalid(
                PlanValidationError::InvalidSchemaAnnotations {
                    reason: crate::schema_annotation::SchemaAnnotationError::InvalidPath { .. },
                    ..
                }
            ))
        ));

        let unknown_binding = mutate_package_body(&package, |body| {
            let boundary = ports_mut(body, 0);
            let Cbor::Array(port) = &mut boundary[0] else {
                panic!("port declaration");
            };
            let Cbor::Array(rows) = &mut port[3] else {
                panic!("annotation table");
            };
            let Cbor::Array(row) = &mut rows[0] else {
                panic!("annotation row");
            };
            let Cbor::Array(annotations) = &mut row[1] else {
                panic!("annotation set");
            };
            let Cbor::Array(measure) = &mut annotations[0] else {
                panic!("measure annotation");
            };
            measure[1] = Cbor::Uint(999);
        });
        assert!(matches!(
            decode(&unknown_binding),
            Err(DecodeError::Malformed(reason)) if reason.contains("unsupported annotation interpretation")
        ));
    }

    #[test]
    fn package_sabotage_cannot_detach_graph_policy_from_its_type() {
        let mut nested = root_owner();
        nested.extensions.clear();
        nested.path = node::OwnerPath::new(["nested"])
            .unwrap_or_else(|error| panic!("nested owner path: {error}"));
        nested.parent = Some(node::NodeIndex::new(0));
        nested.ports = vec![PortDecl {
            port: port("in"),
            direction: node::PortDirection::Input,

            ty: Some(DataType::Text),
            annotations: AnnotationTable::new(vec![(
                SchemaPath::root(),
                one_annotation(DataAnnotation::Secret),
            )])
            .unwrap_or_else(|error| panic!("one policy row: {error}")),
        }];
        let plan = ValidatedExecutionPlan::try_new(ExecutionPlan {
            routes: vec![],
            nodes: vec![root_owner(), nested],
            edges: vec![],
            app_schema: vec![],
            ext_schema: vec![],
            assets: vec![],
        })
        .unwrap_or_else(|error| panic!("typed nested Graph policy validates: {error}"));
        let package = encode(&plan, &SemanticHash::from_bytes([0; 32]));
        let detached = mutate_package_body(&package, |body| {
            let Cbor::Array(owners) = body.get_mut(&12).unwrap_or_else(|| panic!("owners")) else {
                panic!("owner array");
            };
            let Cbor::Map(nested) = &mut owners[1] else {
                panic!("nested owner");
            };
            let Cbor::Array(ports) = nested.get_mut(&3).unwrap_or_else(|| panic!("Graph ports"))
            else {
                panic!("Graph port array");
            };
            let Cbor::Array(port) = &mut ports[0] else {
                panic!("Graph port declaration");
            };
            port[2] = Cbor::Null;
        });
        assert!(matches!(
            decode(&detached),
            Err(DecodeError::Invalid(
                PlanValidationError::InvalidSchemaAnnotations {
                    reason: crate::schema_annotation::SchemaAnnotationError::MissingLogicalType,
                    ..
                }
            ))
        ));
    }

    #[test]
    fn package_sabotage_cannot_reorder_annotation_paths_or_kinds() {
        let ty = DataType::Struct(
            StructType::new(vec![
                TypeField::new("a", DataType::Text),
                TypeField::new("b", DataType::Text),
            ])
            .unwrap_or_else(|error| panic!("closed probe struct: {error}")),
        );
        let root = AnnotationSet::new(vec![
            DataAnnotation::Secret,
            DataAnnotation::Sensitive(SensitivityLevel::Restricted),
        ])
        .unwrap_or_else(|error| panic!("root annotation set: {error}"));
        let annotations = AnnotationTable::new(vec![
            (SchemaPath::root(), root),
            (
                SchemaPath::new(vec![SchemaPathSegment::Field(text("a"))]),
                one_annotation(DataAnnotation::Sensitive(SensitivityLevel::Internal)),
            ),
            (
                SchemaPath::new(vec![SchemaPathSegment::Field(text("b"))]),
                one_annotation(DataAnnotation::Sensitive(SensitivityLevel::Confidential)),
            ),
        ])
        .unwrap_or_else(|error| panic!("ordered annotation table: {error}"));
        let plan = annotated_boundary_schema(ty, annotations);
        let package = encode(&plan, &SemanticHash::from_bytes([0; 32]));

        let mutate_both_tables = |edit: fn(&mut [Cbor])| {
            mutate_package_body(&package, |body| {
                let boundary = ports_mut(body, 0);
                let Cbor::Array(port) = &mut boundary[0] else {
                    panic!("port declaration");
                };
                let Cbor::Array(raw_rows) = &mut port[3] else {
                    panic!("raw annotation table");
                };
                edit(raw_rows);

                let Cbor::Array(resolved) =
                    body.get_mut(&8).unwrap_or_else(|| panic!("resolved table"))
                else {
                    panic!("resolved table array");
                };
                let Cbor::Array(resolved_boundary) = &mut resolved[0] else {
                    panic!("resolved boundary");
                };
                let Cbor::Array(resolved_port) = &mut resolved_boundary[0] else {
                    panic!("resolved port");
                };
                let Cbor::Array(resolved_rows) = &mut resolved_port[4] else {
                    panic!("resolved annotation table");
                };
                edit(resolved_rows);
            })
        };

        fn reverse_rows(rows: &mut [Cbor]) {
            rows.reverse();
        }
        let reordered_paths = mutate_both_tables(reverse_rows);
        assert_eq!(
            decode(&reordered_paths),
            Err(DecodeError::NonCanonical(
                "the body is not the canonical encoding of its plan"
            ))
        );

        fn reverse_root_kind_order(rows: &mut [Cbor]) {
            let Cbor::Array(root) = &mut rows[0] else {
                panic!("root annotation row");
            };
            let Cbor::Array(kinds) = &mut root[1] else {
                panic!("root annotation set");
            };
            kinds.reverse();
        }
        let reordered_kinds = mutate_both_tables(reverse_root_kind_order);
        assert_eq!(
            decode(&reordered_kinds),
            Err(DecodeError::NonCanonical(
                "the body is not the canonical encoding of its plan"
            ))
        );
    }

    #[test]
    fn current_format_refuses_every_other_package_version() {
        let package = encode(
            &annotated_boundary_plan(DataAnnotation::Sensitive(SensitivityLevel::Internal)),
            &SemanticHash::from_bytes([0; 32]),
        );
        let (Cbor::Map(mut envelope), _) =
            canonical_cbor::decode(&package[MAGIC.len()..]).expect("current envelope")
        else {
            panic!("map envelope");
        };
        for version in
            (0..u64::from(FORMAT_VERSION)).chain([u64::from(FORMAT_VERSION) + 1, u64::MAX])
        {
            envelope.insert(1, Cbor::Uint(version));
            let mut previous = MAGIC.to_vec();
            previous.extend_from_slice(&canonical_cbor::encode(&Cbor::Map(envelope.clone())));
            assert_eq!(
                decode(&previous),
                Err(DecodeError::UnsupportedVersion(version))
            );
        }
    }

    #[test]
    fn port_directions_refuse_unknown_tags() {
        for unknown_tag in [3, 4] {
            let declaration = Cbor::Array(vec![Cbor::Text(text("out")), Cbor::Uint(unknown_tag)]);
            assert!(
                port_decl_from(&declaration).is_err(),
                "an unknown tag cannot become a port direction"
            );
        }
    }

    #[test]
    fn graph_wiring_names_the_app_root_and_rejects_an_absent_owner() {
        let rule = crate::plan::RouteRule {
            source: crate::plan::Endpoint {
                node: node::NodeIndex::ROOT,
                port: port("in"),
            },
            selection: crate::plan::RouteSelection::All(vec![crate::plan::Destination::Discard]),
        };
        let encoded = fields::route_tree(&rule);
        assert_eq!(fields::route_from(&encoded).unwrap(), rule);
        let Cbor::Array(mut fields) = encoded else {
            panic!("route");
        };
        fields[0] = Cbor::Array(vec![Cbor::Null, Cbor::Text(text("in"))]);
        assert_eq!(
            fields::route_from(&Cbor::Array(fields)),
            Err(err("Node index"))
        );
    }

    #[test]
    fn routes_are_mandatory_and_unknown_body_fields_are_not_ignored() {
        let package = instance_package();
        let absent = mutate_package_body(&package, |body| {
            let Cbor::Array(nodes) = body.get_mut(&12).expect("nodes") else {
                panic!("nodes")
            };
            let Cbor::Map(root) = &mut nodes[0] else {
                panic!("App Node")
            };
            let Cbor::Array(graph) = root.get_mut(&7).expect("body") else {
                panic!("Graph body")
            };
            graph.pop();
        });
        assert!(matches!(decode(&absent), Err(DecodeError::Malformed(_))));
        for key in [3, 10, 11] {
            let extra = mutate_package_body(&package, |body| {
                body.insert(key, Cbor::Array(vec![]));
            });
            assert_eq!(
                decode(&extra),
                Err(DecodeError::NonCanonical(
                    "the body is not the canonical encoding of its plan"
                ))
            );
        }
    }

    #[test]
    fn a_valid_checksum_cannot_restore_a_graph_handler_or_child_activation() {
        let package = instance_package();
        let graph_handler = mutate_package_body(&package, |body| {
            let Cbor::Array(nodes) = body.get_mut(&12).expect("nodes") else {
                panic!("nodes")
            };
            let Cbor::Map(root) = &mut nodes[0] else {
                panic!("root")
            };
            let Cbor::Array(graph) = root.get_mut(&7).expect("body") else {
                panic!("Graph body")
            };
            graph.push(Cbor::Array(vec![Cbor::Map(BTreeMap::from([
                (2, Cbor::Array(vec![Cbor::Uint(1), Cbor::Text(text("in"))])),
                (
                    4,
                    fields::dispatch_tree(&super::super::Dispatch::Linear(vec![])),
                ),
            ]))]));
        });
        assert_eq!(decode(&graph_handler), Err(err("Node body")));

        let child_activation = mutate_package_body(&package, |body| {
            let Cbor::Array(nodes) = body.get_mut(&12).expect("nodes") else {
                panic!("nodes")
            };
            let Cbor::Map(operator) = &mut nodes[1] else {
                panic!("operator")
            };
            let Cbor::Array(operator) = operator.get_mut(&7).expect("body") else {
                panic!("Operator body")
            };
            let Cbor::Array(handlers) = &mut operator[2] else {
                panic!("handlers")
            };
            handlers.push(Cbor::Map(BTreeMap::from([
                (
                    2,
                    Cbor::Array(vec![
                        Cbor::Uint(4),
                        Cbor::Text(text("child")),
                        Cbor::Text(text("out")),
                    ]),
                ),
                (
                    4,
                    fields::dispatch_tree(&super::super::Dispatch::Linear(vec![])),
                ),
            ])));
        });
        assert_eq!(decode(&child_activation), Err(err("activation")));
        assert!(
            extension_spec_from(&Cbor::Array(vec![
                Cbor::Uint(1),
                Cbor::Map(BTreeMap::new())
            ]))
            .is_err()
        );
    }

    fn ports_mut(body: &mut BTreeMap<u64, Cbor>, index: usize) -> &mut Vec<Cbor> {
        let Cbor::Array(nodes) = body.get_mut(&12).expect("nodes") else {
            panic!("Node table")
        };
        let Cbor::Map(node) = &mut nodes[index] else {
            panic!("Node row")
        };
        let Cbor::Array(ports) = node.get_mut(&3).expect("ports") else {
            panic!("port table")
        };
        ports
    }

    fn instance_mut(body: &mut BTreeMap<u64, Cbor>) -> &mut BTreeMap<u64, Cbor> {
        let Cbor::Array(owners) = body.get_mut(&12).expect("owners") else {
            panic!("owner table");
        };
        let Cbor::Map(owner) = owners.get_mut(1).expect("Node owner") else {
            panic!("Node owner row");
        };
        let Cbor::Map(instance) = owner.get_mut(&4).expect("instance") else {
            panic!("instance row");
        };
        instance
    }

    #[test]
    fn owner_lifetimes_sites_and_dynamic_attributes_have_one_spelling() {
        let lifetimes = [
            instance::Lifetime::Persistent,
            instance::Lifetime::Idle(1),
            instance::Lifetime::Fixed(2),
            instance::Lifetime::SinglePass,
            instance::Lifetime::Tumbling(3),
        ];
        for lifetime in lifetimes {
            assert_eq!(lifetime_from(&lifetime_tree(&lifetime)).unwrap(), lifetime);
        }
        let closing = instance::Closing {
            drain_for: 5,
            settle_for: 7,
        };
        assert_eq!(
            closing_from(&Cbor::Array(vec![
                canonical::signed(closing.drain_for),
                canonical::signed(closing.settle_for),
            ]))
            .unwrap(),
            closing
        );

        for site in [
            EvaluationSite::Plan,
            EvaluationSite::Boot,
            EvaluationSite::Instance,
            EvaluationSite::Invocation,
        ] {
            assert_eq!(site_from(&site_tree(site)).unwrap(), site);
        }

        let setting = AttrSetting::Expr {
            site: EvaluationSite::Instance,
            expr: expr::Tree {
                root: expr::Expr::Path(vec![text("instance"), text("key")]),
                ty: DataType::Text,
            },
        };
        assert_eq!(attr_from(&attr_tree(&setting)).unwrap(), setting);

        let owner = Node {
            path: node::OwnerPath::new(["workspace"]).unwrap(),
            parent: Some(node::NodeIndex::new(0)),
            body: NodeKind::Graph(Box::default()),
            ports: vec![],
            extensions: vec![],
            instance: instance::InstanceSpec {
                key: Some(expr::Tree {
                    root: expr::Expr::Path(vec![text("meta"), text("app"), text("workspace")]),
                    ty: DataType::Text,
                }),
                lifetime: instance::Lifetime::Fixed(30),
                closing: Some(closing),
            },
            vals: vec![OwnerValue {
                name: text("cwd"),
                site: EvaluationSite::Instance,
                expr: expr::Tree {
                    root: expr::Expr::Path(vec![text("instance"), text("key")]),
                    ty: DataType::Text,
                },
                resolved: None,
                annotations: AnnotationTable::empty(),
            }],
        };
        assert_eq!(node_from(&node_tree(&owner)).unwrap(), owner);
    }

    #[test]
    fn package_sabotage_cannot_change_an_instance_key_or_lifetime() {
        let package = instance_package();

        let lying_key_type = mutate_package_body(&package, |body| {
            let instance = instance_mut(body);
            let Cbor::Array(tree) = instance.get_mut(&1).expect("key") else {
                panic!("key tree");
            };
            tree[0] = type_tree(&DataType::BYTES);
        });
        assert_eq!(
            decode(&lying_key_type).unwrap_err(),
            DecodeError::Invalid(PlanValidationError::InstanceKeyTypeMismatch {
                owner: text("worker"),
                expected: DataType::Text,
                found: DataType::BYTES,
            })
        );

        let non_key_domain = mutate_package_body(&package, |body| {
            let instance = instance_mut(body);
            instance.insert(
                1,
                tree_tree(&expr::Tree {
                    root: expr::Expr::List(vec![expr::Expr::Literal(Value::Int(7))]),
                    ty: DataType::array(DataType::INT),
                }),
            );
        });
        assert_eq!(
            decode(&non_key_domain).unwrap_err(),
            DecodeError::Invalid(PlanValidationError::InvalidInstanceKey {
                owner: text("worker"),
                ty: DataType::array(DataType::INT),
            })
        );

        let zero_lifetime = mutate_package_body(&package, |body| {
            instance_mut(body).insert(2, lifetime_tree(&instance::Lifetime::Idle(0)));
        });
        assert_eq!(
            decode(&zero_lifetime).unwrap_err(),
            DecodeError::Invalid(PlanValidationError::InvalidInstanceLifetime {
                owner: text("worker"),
            })
        );

        let zero_closing = mutate_package_body(&package, |body| {
            instance_mut(body).insert(
                3,
                Cbor::Array(vec![canonical::signed(0), canonical::signed(1)]),
            );
        });
        assert_eq!(
            decode(&zero_closing).unwrap_err(),
            DecodeError::Invalid(PlanValidationError::InvalidClosingPolicy {
                owner: text("worker"),
            })
        );
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
                    Value::Timestamp(crate::time::Timestamp::from_nanoseconds(7)),
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
        let vars = vec![VarCell {
            name: text("seen"),
            ty: DataType::text_map(DataType::array(DataType::INT)),
            init: Some(map_value.clone()),
            annotations: AnnotationTable::empty(),
        }];
        let handlers = vec![Handler {
            activation: node::HandlerActivation::Port(port("in")),
            guard: Some(tree(expr.clone())),
            dispatch: crate::plan::Dispatch::Linear(vec![
                DispatchStep::Let {
                    name: text("x"),
                    value: tree(expr.clone()),
                    annotations: AnnotationTable::empty(),
                },
                DispatchStep::SetVar {
                    name: text("seen"),
                    value: tree(expr.clone()),
                },
                DispatchStep::Emit {
                    port: port("out"),
                    value: EmitValue::Identity,
                },
                DispatchStep::Emit {
                    port: port("out"),
                    value: EmitValue::Nothing,
                },
                DispatchStep::Emit {
                    port: port("out"),
                    value: EmitValue::Data(tree(expr.clone())),
                },
                DispatchStep::Emit {
                    port: port("out"),
                    value: EmitValue::Patch(vec![
                        FieldWrite {
                            target: WriteTarget::Data(vec![text("a"), text("b")]),
                            value: tree(expr),
                        },
                        FieldWrite {
                            target: WriteTarget::App(text("m.dotted")),
                            value: tree(expr::Expr::Literal(Value::Null)),
                        },
                        FieldWrite {
                            target: WriteTarget::Ext(text("content_type")),
                            value: tree(expr::Expr::Literal(Value::Text(text("t")))),
                        },
                    ]),
                },
                DispatchStep::Emit {
                    port: port("out"),
                    value: EmitValue::FreshData(vec![FieldWrite {
                        target: WriteTarget::Data(vec![]),
                        value: tree(expr::Expr::Literal(Value::Duration(5))),
                    }]),
                },
                DispatchStep::Stop,
            ]),
        }];
        ExecutionPlan {
            routes: vec![],
            nodes: vec![
                Node {
                    body: NodeKind::Graph(Box::new(crate::plan::Graph {
                        wiring: vec![
                            crate::plan::RouteRule {
                                source: crate::plan::Endpoint {
                                    node: node::NodeIndex::ROOT,
                                    port: port("in"),
                                },
                                selection: crate::plan::RouteSelection::All(vec![
                                    crate::plan::Destination::Port(crate::plan::Endpoint {
                                        node: node::NodeIndex::new(1),
                                        port: port("in"),
                                    }),
                                ]),
                            },
                            crate::plan::RouteRule {
                                source: crate::plan::Endpoint {
                                    node: node::NodeIndex::new(1),
                                    port: port("extra"),
                                },
                                selection: crate::plan::RouteSelection::All(vec![
                                    crate::plan::Destination::Discard,
                                ]),
                            },
                        ],
                    })),
                    ports: vec![
                        PortDecl {
                            port: port("entry"),
                            direction: node::PortDirection::Input,
                            ty: Some(DataType::array(DataType::INT)),
                            annotations: AnnotationTable::empty(),
                        },
                        PortDecl::untyped(port("feed"), node::PortDirection::Output),
                    ],
                    ..root_owner()
                },
                Node {
                    body: NodeKind::Operator(Box::new(Operator { vars, handlers })),
                    ports: vec![
                        PortDecl::untyped(port("extra"), node::PortDirection::Output),
                        PortDecl {
                            port: port("typed"),
                            direction: node::PortDirection::Input,

                            ty: Some(DataType::text_map(DataType::array(DataType::Text))),
                            annotations: AnnotationTable::empty(),
                        },
                    ],
                    ..node_owner("relay")
                },
                Node {
                    body: NodeKind::Connector(Box::new(ConnectorSpec {
                        contract: crate::connector::ContractId::parse(
                            crate::connector::stdlib::PROCESS_CONNECTOR_ID,
                        )
                        .unwrap(),
                        roles: crate::connector::ConnectorDirection::Outbound,
                        exchange: crate::connector::ExchangeKind::Call,
                        idempotent: Some(PlanConstant {
                            expr: expr::compile("true", &expr::Signature::new()).unwrap(),
                            resolved: Value::Bool(true),
                        }),
                        behavior: node::Behavior::Deterministic,
                        attributes: vec![
                            (text("cmd"), AttrSetting::Value(map_value)),
                            (text("codec"), AttrSetting::Word(text("lines"))),
                        ],
                    })),
                    ports: vec![],
                    ..node_owner("shell")
                },
            ],
            edges: vec![],
            app_schema: vec![(text("m.dotted"), DataType::Variant)],
            ext_schema: vec![(text("content_type"), DataType::Text)],
            assets: vec![],
        }
    }

    /// A one-step plan carrying one expression — the chassis for the
    /// nested chains.
    fn one_step_plan(root: expr::Expr) -> ExecutionPlan {
        ExecutionPlan {
            routes: vec![],
            nodes: vec![
                root_owner(),
                Node {
                    body: NodeKind::Operator(Box::new(Operator {
                        vars: vec![],
                        handlers: vec![Handler {
                            activation: node::HandlerActivation::Port(port("in")),
                            guard: None,
                            dispatch: crate::plan::Dispatch::Linear(vec![DispatchStep::Let {
                                name: text("x"),
                                value: tree(root),
                                annotations: AnnotationTable::empty(),
                            }]),
                        }],
                    })),
                    ports: vec![],
                    ..node_owner("o")
                },
            ],
            edges: vec![],
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
            (DataType::MediaType, Cbor::Uint(18)),
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
            expr::Expr::Call {
                function: expr::Function::MediaType,
                args: vec![expr::Expr::Literal(Value::Text(text("application/json")))],
            },
            expr::Expr::Call {
                function: expr::Function::Has,
                args: vec![
                    expr::Expr::Path(vec![text("data")]),
                    expr::Expr::Literal(Value::Text(text("usage"))),
                ],
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
            Value::Timestamp(crate::time::Timestamp::from_nanoseconds(2)),
            Value::Date(3),
            Value::Time(4),
            Value::TimestampNtz(crate::time::Timestamp::from_nanoseconds(5)),
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
            assert_eq!(value_from(&canonical::value(&value)).unwrap(), value);
        }
    }

    #[test]
    fn the_resolved_table_is_mandatory_and_cannot_lie() {
        let validated = ValidatedExecutionPlan::try_new(ExecutionPlan {
            routes: vec![],
            nodes: vec![root_owner()],
            edges: vec![],
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
                    Cbor::Array(vec![Cbor::Array(vec![]), Cbor::Array(vec![])]),
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
        deep_type_plan.nodes[1].handlers_mut()[0].dispatch =
            super::super::Dispatch::Linear(vec![DispatchStep::Let {
                name: text("x"),
                value: expr::Tree {
                    ty: nested_type.clone(),
                    root: expr::Expr::Literal(Value::Null),
                },
                annotations: AnnotationTable::empty(),
            }]);
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
        let policy = AnnotationTable::new(vec![(
            SchemaPath::root(),
            one_annotation(DataAnnotation::Sensitive(SensitivityLevel::Internal)),
        )])
        .unwrap_or_else(|error| panic!("one policy row: {error}"));
        let mut categorical = rich_plan();
        categorical.nodes[0].instance.key = Some(tree(expr::Expr::Literal(Value::Int(7))));
        categorical.nodes[0].vals = vec![OwnerValue {
            name: text("answer"),
            site: EvaluationSite::Plan,
            expr: tree(expr::Expr::Literal(Value::Int(42))),
            resolved: Some(Value::Int(42)),
            annotations: policy.clone(),
        }];
        let NodeKind::Connector(spec) = &mut categorical.nodes[2].body else {
            panic!("the rich plan's second node is a connector");
        };
        spec.attributes.push((
            text("cwd"),
            AttrSetting::Expr {
                site: EvaluationSite::Instance,
                expr: tree(expr::Expr::Path(vec![text("instance"), text("key")])),
            },
        ));
        spec.attributes.sort_by(|left, right| left.0.cmp(&right.0));
        categorical.nodes[0].ports.push(PortDecl {
            port: port("deep"),
            direction: node::PortDirection::Input,

            ty: Some(nested_type.clone()),
            annotations: policy.clone(),
        });
        categorical.nodes[0].extensions = vec![ExtensionSpec::Loop(Box::new(LoopSpec {
            entry: port("in"),
            terminals: vec![port("out")],
            expired: Some(crate::plan::LoopExpiredOutput {
                port: port("out"),
                ty: nested_type.clone(),
            }),
            fuel: Some(PlanConstant {
                expr: tree(nested_maplit.clone()),
                resolved: nested_value.clone(),
            }),
            delay: Some(DelayPolicy::Exponential {
                first: PlanConstant {
                    expr: tree(expr::Expr::Literal(Value::Duration(1))),
                    resolved: Value::Duration(1),
                },
                ceiling: PlanConstant {
                    expr: tree(expr::Expr::Literal(Value::Duration(2))),
                    resolved: Value::Duration(2),
                },
            }),
            proof: None,
        }))];

        let plans = [
            ExecutionPlan {
                routes: vec![],
                nodes: vec![root_owner()],
                edges: vec![],
                app_schema: vec![],
                ext_schema: vec![],
                assets: vec![],
            },
            categorical,
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

        for value in [
            EmitValue::Identity,
            EmitValue::Nothing,
            EmitValue::Data(tree(expr::Expr::Literal(Value::Int(1)))),
            EmitValue::Patch(vec![FieldWrite {
                target: WriteTarget::Data(vec![text("n")]),
                value: tree(expr::Expr::Literal(Value::Int(1))),
            }]),
            EmitValue::FreshData(vec![FieldWrite {
                target: WriteTarget::App(text("tenant")),
                value: tree(expr::Expr::Literal(Value::Text(text("alice")))),
            }]),
        ] {
            for statement in [
                DispatchStep::MultiEmit {
                    ports: vec![port("z"), port("a")],
                    values: super::super::MultiEmitValues::Broadcast(value.clone()),
                },
                DispatchStep::MultiEmit {
                    ports: vec![port("z"), port("a")],
                    values: super::super::MultiEmitValues::Tuple(vec![
                        value.clone(),
                        value.clone(),
                    ]),
                },
                DispatchStep::LoopCommand(super::super::LoopCommand::Next(value.clone())),
                DispatchStep::LoopCommand(super::super::LoopCommand::Again),
                DispatchStep::LoopCommand(super::super::LoopCommand::Done),
            ] {
                let mut raw = one_step_plan(expr::Expr::Literal(Value::Null));
                raw.nodes[1].handlers_mut()[0].dispatch =
                    super::super::Dispatch::Linear(vec![statement]);
                assert_eq!(deepest_level(&raw), levels(&plan_tree(&raw)));
                assert!(
                    matches!(
                        ValidatedExecutionPlan::try_new(raw),
                        Err(PlanValidationError::UnsupportedStatement { .. }
                            | PlanValidationError::WrongPortDirection { .. })
                    ),
                    "transport does not prove output targets or manager support"
                );
            }
        }

        let resolved_plan = ExecutionPlan {
            routes: vec![],
            nodes: vec![Node {
                ports: vec![PortDecl {
                    port: port("deep"),
                    direction: node::PortDirection::Input,
                    ty: Some(nested_type),
                    annotations: policy,
                }],
                ..root_owner()
            }],
            edges: vec![],
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
