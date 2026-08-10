//! Data and metadata carried through the graph.
//!
//! Governing rule: [`envelope.md`] — the single source of truth for the
//! envelope's structure, identity, and `meta.sys` reserved fields. An
//! envelope is immutable: a transaction never mutates one, it emits new
//! ones.
//!
//! This module owns the value types and the identity framing primitive.
//! The full derivation recipes (`root_id`, `emission_id`) belong to the
//! journal's `ids` module. The `Union` and `Variant` arms of the
//! materialized value domain join with the checker work that gives them
//! meaning.
//!
//! [`envelope.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/envelope.md

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::OnceCell;
use core::fmt;

/// Deterministically derived 128-bit identity (`envelope.md` §2). The
/// physical type is a fixed 128-bit value; no random bits ever enter an
/// id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id([u8; 16]);

impl Id {
    /// Derives an id from a domain tag and length-prefixed frames — the
    /// injective framing of `envelope.md` §2:
    ///
    /// ```text
    /// input    = domain_tag ++ ( u32_le(len(field)) ++ bytes(field) )*
    /// id       = first 128 bits of blake3(input)
    /// ```
    ///
    /// Length-prefixing kills concatenation ambiguity; the domain byte
    /// separates derivations whose arities could otherwise realign. The
    /// recipe vocabulary (which tag, which fields, their encodings) is
    /// owned by the journal's `ids` module.
    #[must_use]
    pub fn derive(domain_tag: u8, fields: &[&[u8]]) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&[domain_tag]);
        for field in fields {
            let len = u32::try_from(field.len())
                .unwrap_or_else(|_| unreachable!("an identity frame never approaches 4 GiB"));
            hasher.update(&len.to_le_bytes());
            hasher.update(field);
        }
        let mut bytes = [0u8; 16];
        hasher.finalize_xof().fill(&mut bytes);
        Self(bytes)
    }

    /// Wraps raw id bytes, for journal decoding and fixtures.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// The raw 16 bytes — the `parent_id` encoding in the framing.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Scalar leaf of the materialized data domain, shared by `data` values
/// and `meta` namespace leaves.
#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    /// Boolean.
    Bool(bool),
    /// 64-bit signed integer.
    Int(i64),
    /// IEEE-754 double. Carries no `Eq`: `NaN != NaN` stands.
    Double(f64),
    /// UTF-8 text.
    Text(String),
    /// Opaque bytes.
    Bytes(Vec<u8>),
    /// An exact decimal — `unscaled × 10^(−scale)`, at most 38
    /// significant digits (D-2026-08-10-7). Typed decimals keep
    /// their declared scale; dynamic ones the minimal scale.
    Decimal {
        /// The unscaled coefficient.
        unscaled: i128,
        /// Digits after the point (negative = trailing zeros).
        scale: i32,
    },
}

/// Materialized Rastok data domain — the in-memory view of `data`.
///
/// Struct equality compares fields BY NAME: declaration order is
/// presentation, not semantics (identity.md — the canonical seal
/// sorts), so two structs with one canonical identity are one
/// value. Well-formedness (unique field names) is enforced at the
/// machine's doors, never resolved by "first found".
#[derive(Debug, Clone)]
pub enum Value {
    /// Scalar leaf.
    Scalar(Scalar),
    /// Named fields in declaration order.
    Struct(Vec<(String, Value)>),
    /// Ordered homogeneous elements.
    Array(Vec<Value>),
    /// Keyed values; M0 admits text keys.
    Map(BTreeMap<String, Value>),
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Scalar(a), Self::Scalar(b)) => a == b,
            // Sorted-field comparison — by name like the canonical
            // seal, total on malformed duplicates (the cel twin
            // documents the contract).
            (Self::Struct(a), Self::Struct(b)) => {
                if a.len() != b.len() {
                    return false;
                }
                fn sorted(fields: &[(String, Value)]) -> Vec<&(String, Value)> {
                    let mut refs: Vec<&(String, Value)> = fields.iter().collect();
                    refs.sort_by(|x, y| x.0.cmp(&y.0));
                    refs
                }
                sorted(a)
                    .into_iter()
                    .zip(sorted(b))
                    .all(|(x, y)| x.0 == y.0 && x.1 == y.1)
            }
            (Self::Array(a), Self::Array(b)) => a == b,
            (Self::Map(a), Self::Map(b)) => a == b,
            _ => false,
        }
    }
}

/// Per-envelope representation tag of encoded `data` bytes
/// (`envelope.md` §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Encoding {
    /// Opaque bytes; `meta.ext.content_type` names them externally.
    Raw,
    /// Avro-encoded document.
    Avro,
    /// Arrow IPC-encoded table.
    ArrowIpc,
}

impl fmt::Display for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Raw => "raw",
            Self::Avro => "avro",
            Self::ArrowIpc => "arrow-ipc",
        })
    }
}

/// The payload — the normative field's two states (`envelope.md` §5).
#[derive(Debug, Clone)]
pub enum Data {
    /// Original bytes preserved, with their representation tag. The
    /// materialized cache is lazy, never causal, and never serialized;
    /// equality ignores it.
    Encoded {
        /// How the bytes are represented now.
        encoding: Encoding,
        /// The original bytes, preserved untouched.
        bytes: Vec<u8>,
        /// Lazy materialization, filled at most once on first
        /// structural access.
        value_cache: OnceCell<Value>,
    },
    /// A value born in memory from the graph, with no original bytes.
    Materialized(Value),
}

impl Data {
    /// Wraps encoded bytes with an empty materialization cache.
    #[must_use]
    pub fn encoded(encoding: Encoding, bytes: impl Into<Vec<u8>>) -> Self {
        Self::Encoded {
            encoding,
            bytes: bytes.into(),
            value_cache: OnceCell::new(),
        }
    }

    /// Wraps a value born in memory.
    #[must_use]
    pub fn materialized(value: Value) -> Self {
        Self::Materialized(value)
    }

    /// The materialized view, if one exists without decoding work:
    /// the born-in-memory value, or an already-filled cache.
    #[must_use]
    pub fn materialized_value(&self) -> Option<&Value> {
        match self {
            Self::Materialized(value) => Some(value),
            Self::Encoded { value_cache, .. } => value_cache.get(),
        }
    }

    /// The materialized view, decoding through `decode` on first
    /// structural access — the transcode-once rule of `envelope.md`
    /// §5.1. A born-in-memory value never decodes; a successful decode
    /// caches at most once. A failed decode caches nothing: the typed
    /// error reaches the caller, and a later attempt may retry.
    ///
    /// # Errors
    ///
    /// Whatever `decode` reports — the decode error stays typed, never
    /// flattened or swallowed.
    pub fn materialize_with<E>(
        &self,
        decode: impl FnOnce(Encoding, &[u8]) -> Result<Value, E>,
    ) -> Result<&Value, E> {
        match self {
            Self::Materialized(value) => Ok(value),
            Self::Encoded {
                encoding,
                bytes,
                value_cache,
            } => {
                if let Some(value) = value_cache.get() {
                    return Ok(value);
                }
                let value = decode(*encoding, bytes)?;
                Ok(value_cache.get_or_init(|| value))
            }
        }
    }
}

impl PartialEq for Data {
    /// Equality is causal: encoded data compares by encoding and bytes,
    /// never by the cache.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Encoded {
                    encoding, bytes, ..
                },
                Self::Encoded {
                    encoding: other_encoding,
                    bytes: other_bytes,
                    ..
                },
            ) => encoding == other_encoding && bytes == other_bytes,
            (Self::Materialized(value), Self::Materialized(other_value)) => value == other_value,
            _ => false,
        }
    }
}

/// The envelope's role (`envelope.md` §1.1) — an extensible vocabulary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Role {
    /// Ordinary forward flow.
    #[default]
    Default,
    /// Saga compensation flow.
    Compensate,
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Default => "default",
            Self::Compensate => "compensate",
        })
    }
}

/// Runtime-owned identity and lineage metadata — exactly the five-field
/// Tier-1 spine of `envelope.md` §1.1. Tier-2 birth stamps join with
/// the aspect work; envelopes born outside an aspect scope carry
/// nothing.
///
/// Read-only by construction: built through [`MetaSys::root`] or
/// [`MetaSys::emission`], which encode the lineage invariants — a root
/// is its own origin, and an emission inherits its parent's origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaSys {
    id: Id,
    parent_id: Option<Id>,
    origin_id: Id,
    role: Role,
    graph_version: u64,
}

impl MetaSys {
    /// Mints the spine of a root envelope: no parent, and the envelope
    /// is its own origin (`origin_id == id`).
    #[must_use]
    pub fn root(id: Id, graph_version: u64) -> Self {
        Self {
            id,
            parent_id: None,
            origin_id: id,
            role: Role::Default,
            graph_version,
        }
    }

    /// Mints the spine of an emitted envelope: the parent is the cause,
    /// and the origin is inherited from it unchanged.
    #[must_use]
    pub fn emission(id: Id, parent: &MetaSys, graph_version: u64) -> Self {
        Self {
            id,
            parent_id: Some(parent.id),
            origin_id: parent.origin_id,
            role: Role::Default,
            graph_version,
        }
    }

    /// Sets the role at mint time.
    #[must_use]
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Unique derived identity.
    #[must_use]
    pub fn id(&self) -> Id {
        self.id
    }

    /// The immediate ancestor envelope; `None` at inbound. The chain of
    /// parents is the lineage.
    #[must_use]
    pub fn parent_id(&self) -> Option<Id> {
        self.parent_id
    }

    /// The lineage root's id; equals [`Self::id`] on a root envelope.
    #[must_use]
    pub fn origin_id(&self) -> Id {
        self.origin_id
    }

    /// The envelope's role.
    #[must_use]
    pub fn role(&self) -> Role {
        self.role
    }

    /// Sequential build number of the emitting graph; the manifest maps
    /// it to hashes and schemas.
    #[must_use]
    pub fn graph_version(&self) -> u64 {
        self.graph_version
    }

    /// Whether this envelope is a lineage root.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.parent_id.is_none()
    }
}

/// Leaf of a writable `meta` namespace: flat by type — a scalar or an
/// array of scalars, never nested (`envelope.md` preamble).
#[derive(Debug, Clone, PartialEq)]
pub enum MetaValue {
    /// Scalar leaf.
    Scalar(Scalar),
    /// Array-of-scalar leaf.
    Array(Vec<Scalar>),
}

/// One writable, flat `meta` namespace.
pub type Namespace = BTreeMap<String, MetaValue>;

/// Where an envelope-builder write lands — resolved at check, never
/// re-interpreted from strings at run time. The checker owns the
/// zone rules (three zones, no alias, `meta.sys` refused —
/// D-2026-08-10-1); the runtime only executes a verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteTarget {
    /// A payload write; the empty path replaces `data` whole.
    Data(Vec<String>),
    /// A field of the application zone — the FLAT dotted key
    /// (`"audit.origin"`); the projection nests, storage does not
    /// (D-2026-08-10-1).
    App(String),
    /// A field of the external-context zone — flat key, vocabulary
    /// owned by the connector sheets plus the standard
    /// `content_type` (D-2026-08-10-4).
    Ext(String),
}

/// Runtime and application metadata: the reserved `sys` spine and
/// the two writable zones — `app` (application-owned) and `ext`
/// (external-boundary context). Exactly three zones, no other root
/// (D-2026-08-10-1).
#[derive(Debug, Clone, PartialEq)]
pub struct Meta {
    sys: MetaSys,
    app: Namespace,
    ext: Namespace,
}

impl Meta {
    /// Starts metadata around a minted spine, with empty writable
    /// namespaces.
    #[must_use]
    pub fn new(sys: MetaSys) -> Self {
        Self {
            sys,
            app: Namespace::new(),
            ext: Namespace::new(),
        }
    }

    /// Replaces the `app` namespace at construction time.
    #[must_use]
    pub fn with_app(mut self, app: Namespace) -> Self {
        self.app = app;
        self
    }

    /// Sets the external-context zone at construction time.
    #[must_use]
    pub fn with_ext(mut self, ext: Namespace) -> Self {
        self.ext = ext;
        self
    }

    /// The reserved, runtime-owned spine.
    #[must_use]
    pub fn sys(&self) -> &MetaSys {
        &self.sys
    }

    /// The application zone — flat dotted keys, read/write.
    #[must_use]
    pub fn app(&self) -> &Namespace {
        &self.app
    }

    /// The external-context zone — flat dotted keys, written by
    /// boundaries and rewritable by handlers (D-2026-08-10-1).
    #[must_use]
    pub fn ext(&self) -> &Namespace {
        &self.ext
    }
}

/// Immutable emitted message: `meta` written on the outside, `data`
/// sealed inside. There is no mutating surface — a transaction emits
/// new envelopes instead.
#[derive(Debug, Clone, PartialEq)]
pub struct Envelope {
    meta: Meta,
    data: Data,
}

impl Envelope {
    /// Seals metadata and payload into an envelope.
    #[must_use]
    pub fn new(meta: Meta, data: Data) -> Self {
        Self { meta, data }
    }

    /// The metadata.
    #[must_use]
    pub fn meta(&self) -> &Meta {
        &self.meta
    }

    /// The payload.
    #[must_use]
    pub fn data(&self) -> &Data {
        &self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn id_framing_is_injective() {
        let split_one = Id::derive(0x02, &[b"ab", b"c"]);
        let split_two = Id::derive(0x02, &[b"a", b"bc"]);
        assert_ne!(split_one, split_two);
        assert_eq!(split_one, Id::derive(0x02, &[b"ab", b"c"]));
    }

    #[test]
    fn id_domains_are_separated() {
        assert_ne!(Id::derive(0x01, &[b"x"]), Id::derive(0x02, &[b"x"]));
    }

    #[test]
    fn id_derivation_is_pinned_byte_for_byte() {
        let id = Id::derive(0x01, &[b"inbound", &7u64.to_le_bytes()]);
        assert_eq!(id.to_string(), "78cf0ba5304d78c7857726f182eb0706");
        assert_eq!(Id::from_bytes(*id.as_bytes()), id);
    }

    #[test]
    fn a_root_is_its_own_origin() {
        let id = Id::derive(0x01, &[b"inbound", &0u64.to_le_bytes()]);
        let root = MetaSys::root(id, 42);
        assert_eq!(root.origin_id(), root.id());
        assert_eq!(root.parent_id(), None);
        assert!(root.is_root());
        assert_eq!(root.role(), Role::Default);
        assert_eq!(root.graph_version(), 42);
    }

    #[test]
    fn a_lineage_keeps_its_origin_constant() {
        let root = MetaSys::root(Id::derive(0x01, &[b"inbound", &0u64.to_le_bytes()]), 1);
        let hop_one = MetaSys::emission(Id::derive(0x02, &[b"a"]), &root, 1);
        let hop_two = MetaSys::emission(Id::derive(0x02, &[b"b"]), &hop_one, 1);
        assert_eq!(hop_one.parent_id(), Some(root.id()));
        assert_eq!(hop_two.parent_id(), Some(hop_one.id()));
        assert_eq!(hop_one.origin_id(), root.id());
        assert_eq!(hop_two.origin_id(), root.id());
        assert!(!hop_two.is_root());
    }

    /// The test decoders' error type; any typed error travels intact.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct DecodeFault(&'static str);

    #[test]
    fn data_equality_is_causal_and_ignores_the_cache() {
        let plain = Data::encoded(Encoding::Raw, *b"{\"n\":1}");
        let cached = Data::encoded(Encoding::Raw, *b"{\"n\":1}");
        cached
            .materialize_with(|_, _| Ok::<_, DecodeFault>(Value::Scalar(Scalar::Int(1))))
            .unwrap();
        assert_eq!(plain, cached);
        assert_ne!(plain, Data::encoded(Encoding::Avro, *b"{\"n\":1}"));
    }

    #[test]
    fn materialization_decodes_at_most_once() {
        let data = Data::encoded(Encoding::Raw, *b"7");
        assert_eq!(data.materialized_value(), None);
        let mut decode_runs = 0;
        for _ in 0..3 {
            let value = data
                .materialize_with(|encoding, bytes| {
                    decode_runs += 1;
                    assert_eq!(encoding, Encoding::Raw);
                    assert_eq!(bytes, b"7");
                    Ok::<_, DecodeFault>(Value::Scalar(Scalar::Int(7)))
                })
                .unwrap();
            assert_eq!(value, &Value::Scalar(Scalar::Int(7)));
        }
        assert_eq!(decode_runs, 1);
        assert_eq!(
            data.materialized_value(),
            Some(&Value::Scalar(Scalar::Int(7)))
        );
    }

    #[test]
    fn a_failed_decode_stays_typed_and_caches_nothing() {
        let data = Data::encoded(Encoding::Raw, *b"not json");
        let fault = data
            .materialize_with(|_, _| Err::<Value, _>(DecodeFault("malformed document")))
            .unwrap_err();
        assert_eq!(fault, DecodeFault("malformed document"));
        // The failure poisoned nothing: no cache, and a later decode
        // may succeed.
        assert_eq!(data.materialized_value(), None);
        let value = data
            .materialize_with(|_, _| Ok::<_, DecodeFault>(Value::Scalar(Scalar::Int(1))))
            .unwrap();
        assert_eq!(value, &Value::Scalar(Scalar::Int(1)));
    }

    #[test]
    fn a_born_in_memory_value_never_decodes() {
        let data = Data::materialized(Value::Scalar(Scalar::Text(String::from("hi"))));
        let value = data
            .materialize_with(|_, _| unreachable!("no bytes to decode"))
            .unwrap_or_else(|_: DecodeFault| unreachable!());
        assert_eq!(value, &Value::Scalar(Scalar::Text(String::from("hi"))));
    }

    #[test]
    fn an_envelope_exposes_meta_and_data_read_only() {
        let sys = MetaSys::root(Id::derive(0x01, &[b"inbound", &0u64.to_le_bytes()]), 1);
        let mut app = Namespace::new();
        app.insert(
            String::from("session_key"),
            MetaValue::Scalar(Scalar::Text(String::from("s-1"))),
        );
        let mut ext = Namespace::new();
        ext.insert(
            String::from("content_type"),
            MetaValue::Scalar(Scalar::Text(String::from("application/json"))),
        );
        let envelope = Envelope::new(
            Meta::new(sys).with_app(app).with_ext(ext),
            Data::encoded(Encoding::Raw, *b"{}"),
        );
        assert_eq!(
            envelope.meta().ext().get("content_type"),
            Some(&MetaValue::Scalar(Scalar::Text(String::from(
                "application/json"
            ))))
        );
        assert_eq!(
            envelope.meta().app().get("session_key"),
            Some(&MetaValue::Scalar(Scalar::Text(String::from("s-1"))))
        );
        assert_eq!(envelope.data(), &Data::encoded(Encoding::Raw, *b"{}"));
    }
}
