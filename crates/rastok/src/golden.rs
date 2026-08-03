//! Golden evidence — the canonical projection of causal records and
//! the first meaningful divergence.
//!
//! Two byte formats exist and are never the same object: the durable
//! journal encoding (free to evolve) and the canonical form of
//! `identity.md` §3 (fixed — what golden traces and cross-version
//! replay compare). This module writes the second: deterministic CBOR
//! per RFC 8949 §4.2 — shortest-form integers, definite lengths,
//! fixed unsigned map keys emitted in table order — sealed by blake3
//! over the concatenated records. [`diff`] exists to *explain* a
//! broken seal: the first record, and the first key path inside it,
//! where two traces disagree.
//!
//! # Why CBOR, and why no library
//!
//! A trace is *evidence*, read by parties who are not this codebase —
//! auditors, second implementations, conformance tooling. A standard
//! substrate keeps that evidence legible without our code (any CBOR
//! inspector decodes a golden line today), and the extend-only table
//! discipline — append keys, never renumber, absent = omitted — works
//! precisely because CBOR maps are navigable without a schema. A
//! fully bespoke byte format would still need every line of this
//! encoder, plus its own byte-grammar specification and versioning
//! machinery, minus decades of review; RFC 8949 is IETF STD 94 and
//! will not move under us. (Bytes that are written but never read
//! back — the id framings — are rightly bespoke: injectivity is
//! their only contract. Bytes only we read back — the durable
//! journal — are free to choose either way.)
//!
//! No encoding library is used because conformance is not the
//! requirement — *byte identity under this profile* is. The standard
//! admits many valid encodings of one value; §4.2 is a recommended
//! profile; and identity.md deviates from even that where it must
//! (float width is fixed by the type tag, never shortest-form, so a
//! `1.5` cannot silently narrow). A conformant library implements
//! *its* choices among the legal encodings and may revise them at
//! any version — each revision a broken seal for a non-semantic
//! reason. This encoder covers the closed subset the tables need,
//! encode-only (decoding, where CBOR's complexity and attack surface
//! live, never happens here), and changes only when the spec does.

use crate::envelope::{Data, Encoding, Envelope, Role, Scalar};
use crate::journal::{Closure, OutboundStatus, Record};
use crate::{cel, envelope};
use std::fmt;

/// One projected trace: the canonical bytes of every causal record,
/// with the structured tree kept beside them so a divergence can be
/// named instead of hex-dumped.
#[derive(Debug, Clone, PartialEq)]
pub struct Trace {
    records: Vec<CanonicalRecord>,
}

#[derive(Debug, Clone, PartialEq)]
struct CanonicalRecord {
    tree: Cbor,
    bytes: Vec<u8>,
}

impl Trace {
    /// The number of projected records.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the trace is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The canonical bytes of one record.
    #[must_use]
    pub fn record_bytes(&self, index: usize) -> Option<&[u8]> {
        self.records
            .get(index)
            .map(|record| record.bytes.as_slice())
    }

    /// The fast seal: blake3 over the concatenated canonical records.
    #[must_use]
    pub fn seal(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        for record in &self.records {
            hasher.update(&record.bytes);
        }
        let mut seal = [0u8; 32];
        hasher.finalize_xof().fill(&mut seal);
        seal
    }

    /// The seal, rendered as lowercase hex — what a golden file pins.
    #[must_use]
    pub fn seal_hex(&self) -> String {
        self.seal()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

impl Trace {
    /// Rebuilds a trace from stored canonical record bytes — how a
    /// blessed golden re-enters the diff. Returns `None` on bytes
    /// outside the tables' subset; a golden this build cannot read
    /// is itself a divergence worth failing on.
    #[must_use]
    pub fn from_stored(records: &[Vec<u8>]) -> Option<Self> {
        let mut out = Vec::new();
        for bytes in records {
            let (tree, used) = decode(bytes)?;
            if used != bytes.len() {
                return None;
            }
            out.push(CanonicalRecord {
                tree,
                bytes: bytes.clone(),
            });
        }
        Some(Self { records: out })
    }
}

/// Decodes one canonical value; returns the tree and the bytes
/// consumed. Only the subset the tables emit is accepted.
fn decode(bytes: &[u8]) -> Option<(Cbor, usize)> {
    let first = *bytes.first()?;
    let major = first >> 5;
    match (major, first) {
        (_, 0xf4) => Some((Cbor::Bool(false), 1)),
        (_, 0xf5) => Some((Cbor::Bool(true), 1)),
        (_, 0xf6) => Some((Cbor::Null, 1)),
        (_, 0xfb) => {
            let raw: [u8; 8] = bytes.get(1..9)?.try_into().ok()?;
            Some((Cbor::Double(f64::from_bits(u64::from_be_bytes(raw))), 9))
        }
        (0, _) => {
            let (value, used) = argument(bytes)?;
            Some((Cbor::Uint(value), used))
        }
        (1, _) => {
            let (value, used) = argument(bytes)?;
            Some((Cbor::Int(-1 - i64::try_from(value).ok()?), used))
        }
        (2, _) => {
            let (length, used) = argument(bytes)?;
            let length = usize::try_from(length).ok()?;
            let payload = bytes.get(used..used + length)?;
            Some((Cbor::Bytes(payload.to_vec()), used + length))
        }
        (3, _) => {
            let (length, used) = argument(bytes)?;
            let length = usize::try_from(length).ok()?;
            let payload = bytes.get(used..used + length)?;
            Some((
                Cbor::Text(String::from_utf8(payload.to_vec()).ok()?),
                used + length,
            ))
        }
        (4, _) => {
            let (count, mut used) = argument(bytes)?;
            let mut items = Vec::new();
            for _ in 0..count {
                let (item, item_used) = decode(bytes.get(used..)?)?;
                items.push(item);
                used += item_used;
            }
            Some((Cbor::Array(items), used))
        }
        (5, _) => {
            let (count, mut used) = argument(bytes)?;
            let mut pairs = Vec::new();
            for _ in 0..count {
                let (key, key_used) = decode(bytes.get(used..)?)?;
                used += key_used;
                let Cbor::Uint(key) = key else {
                    return None;
                };
                let (value, value_used) = decode(bytes.get(used..)?)?;
                used += value_used;
                pairs.push((key, value));
            }
            Some((Cbor::Map(pairs), used))
        }
        _ => None,
    }
}

/// A shortest-form head's argument and consumed length.
fn argument(bytes: &[u8]) -> Option<(u64, usize)> {
    let info = bytes.first()? & 0x1f;
    match info {
        0..=23 => Some((u64::from(info), 1)),
        24 => Some((u64::from(*bytes.get(1)?), 2)),
        25 => Some((
            u64::from(u16::from_be_bytes(bytes.get(1..3)?.try_into().ok()?)),
            3,
        )),
        26 => Some((
            u64::from(u32::from_be_bytes(bytes.get(1..5)?.try_into().ok()?)),
            5,
        )),
        27 => Some((u64::from_be_bytes(bytes.get(1..9)?.try_into().ok()?), 9)),
        _ => None,
    }
}

/// Projects a journal's records into their canonical form.
#[must_use]
pub fn project(records: &[Record]) -> Trace {
    Trace {
        records: records
            .iter()
            .map(|record| {
                let tree = record_tree(record);
                let bytes = encode(&tree);
                CanonicalRecord { tree, bytes }
            })
            .collect(),
    }
}

// ----- the divergence -----

/// The first semantic difference between two traces.
#[derive(Debug, Clone, PartialEq)]
pub struct Divergence {
    /// The index of the first differing record.
    pub record: usize,
    /// The key path to the differing value, table names included.
    pub path: String,
    /// The value on the left, rendered.
    pub left: String,
    /// The value on the right, rendered.
    pub right: String,
}

impl fmt::Display for Divergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "record {}: {} — {} ≠ {}",
            self.record, self.path, self.left, self.right
        )
    }
}

/// The first meaningful divergence between two traces, or `None`
/// when their canonical bytes agree everywhere.
#[must_use]
pub fn diff(ours: &Trace, theirs: &Trace) -> Option<Divergence> {
    for (index, (a, b)) in ours.records.iter().zip(&theirs.records).enumerate() {
        if a.bytes != b.bytes {
            let (path, left, right) = explain(&a.tree, &b.tree, String::new());
            return Some(Divergence {
                record: index,
                path,
                left,
                right,
            });
        }
    }
    if ours.records.len() != theirs.records.len() {
        let index = ours.records.len().min(theirs.records.len());
        return Some(Divergence {
            record: index,
            path: String::from("record count"),
            left: ours.records.len().to_string(),
            right: theirs.records.len().to_string(),
        });
    }
    None
}

/// Walks two trees to the first difference, accumulating the named
/// key path.
fn explain(a: &Cbor, b: &Cbor, path: String) -> (String, String, String) {
    match (a, b) {
        (Cbor::Map(left), Cbor::Map(right)) => {
            for (key, value) in left {
                let at = format!(
                    "{path}{}{}",
                    if path.is_empty() { "" } else { " . " },
                    key_name(*key)
                );
                match right.iter().find(|(other, _)| other == key) {
                    None => return (at, render(value), String::from("absent")),
                    Some((_, other)) if other != value => {
                        return explain(value, other, at);
                    }
                    Some(_) => {}
                }
            }
            for (key, value) in right {
                if !left.iter().any(|(other, _)| other == key) {
                    let at = format!(
                        "{path}{}{}",
                        if path.is_empty() { "" } else { " . " },
                        key_name(*key)
                    );
                    return (at, String::from("absent"), render(value));
                }
            }
            (path, render(a), render(b))
        }
        (Cbor::Array(left), Cbor::Array(right)) => {
            for (index, (item, other)) in left.iter().zip(right).enumerate() {
                if item != other {
                    return explain(item, other, format!("{path}[{index}]"));
                }
            }
            if left.len() != right.len() {
                let at = format!("{path}.len");
                return (at, left.len().to_string(), right.len().to_string());
            }
            (path, render(a), render(b))
        }
        _ => (path, render(a), render(b)),
    }
}

fn render(value: &Cbor) -> String {
    match value {
        Cbor::Uint(n) => n.to_string(),
        Cbor::Int(n) => n.to_string(),
        Cbor::Bool(b) => b.to_string(),
        Cbor::Null => String::from("null"),
        Cbor::Double(d) => d.to_string(),
        Cbor::Text(text) => format!("{text:?}"),
        Cbor::Bytes(bytes) => {
            let hex: String = bytes
                .iter()
                .take(8)
                .map(|byte| format!("{byte:02x}"))
                .collect();
            if bytes.len() > 8 {
                format!("{hex}…({} bytes)", bytes.len())
            } else {
                hex
            }
        }
        Cbor::Array(items) => format!("[{} items]", items.len()),
        Cbor::Map(pairs) => format!("{{{} keys}}", pairs.len()),
    }
}

/// The extend-only key registry, named for humans — `trace dump` is
/// derived and non-normative; the numbers are the contract.
fn key_name(key: u64) -> String {
    let name = match key {
        1 => "id",
        2 => "parent_id",
        3 => "origin_id",
        4 => "role",
        5 => "graph_version",
        6 => "mime_type",
        7 => "tier2",
        8 => "data",
        9 => "app_meta",
        10 => "kind",
        20 => "envelope",
        21 => "connector_path",
        22 => "inbound_seq",
        23 => "replica",
        30 => "node_path",
        31 => "partition_key",
        32 => "trigger",
        33 => "consumed",
        34 => "join_inputs",
        35 => "state_delta",
        36 => "emissions",
        37 => "timers",
        40 => "emitting_id",
        41 => "connector_path",
        42 => "call_key",
        43 => "request_digest",
        44 => "status",
        45 => "closure",
        46 => "edge_cursor",
        _ => return format!("key {key}"),
    };
    format!("{name}({key})")
}

// ----- projection: the per-species tables -----

fn record_tree(record: &Record) -> Cbor {
    match record {
        Record::Inbound(inbound) => Cbor::Map(vec![
            (10, Cbor::Uint(1)),
            (20, spine(&inbound.envelope)),
            (
                21,
                Cbor::Text(inbound.authority.connector_path().to_string()),
            ),
            (22, Cbor::Uint(inbound.seq)),
            (23, Cbor::Uint(inbound.authority.replica())),
        ]),
        Record::Transaction(transaction) => {
            let mut pairs = vec![
                (10, Cbor::Uint(2)),
                (30, Cbor::Text(transaction.node_path.to_string())),
                (32, Cbor::Text(transaction.trigger.to_string())),
            ];
            let mut consumed: Vec<&crate::journal::ConsumedPosition> =
                transaction.consumed.iter().collect();
            consumed.sort();
            pairs.push((
                33,
                Cbor::Array(
                    consumed
                        .iter()
                        .map(|position| {
                            Cbor::Array(vec![
                                Cbor::Text(position.edge.clone()),
                                // The single-cell slice: the source
                                // partition is the empty key.
                                Cbor::Bytes(Vec::new()),
                                Cbor::Uint(position.seq),
                            ])
                        })
                        .collect(),
                ),
            ));
            if !transaction.state_delta.is_empty() {
                let mut delta: Vec<_> = transaction.state_delta.iter().collect();
                delta.sort_by(|a, b| a.0.cmp(&b.0));
                pairs.push((
                    35,
                    Cbor::Array(
                        delta
                            .iter()
                            .map(|(field, value)| {
                                Cbor::Array(vec![Cbor::Text(field.clone()), cel_tagged(value)])
                            })
                            .collect(),
                    ),
                ));
            }
            pairs.push((
                36,
                Cbor::Array(
                    transaction
                        .emissions
                        .iter()
                        .map(|emission| {
                            Cbor::Array(vec![
                                Cbor::Text(emission.port.name().to_string()),
                                Cbor::Uint(u64::from(emission.index)),
                                spine(&emission.envelope),
                            ])
                        })
                        .collect(),
                ),
            ));
            if !transaction.timers.is_empty() {
                let mut timers: Vec<_> = transaction.timers.iter().collect();
                timers.sort();
                pairs.push((
                    37,
                    Cbor::Array(
                        timers
                            .into_iter()
                            .map(|(kind, reference)| {
                                Cbor::Array(vec![
                                    Cbor::Uint(u64::from(*kind)),
                                    Cbor::Bytes(reference.as_bytes().to_vec()),
                                ])
                            })
                            .collect(),
                    ),
                ));
            }
            Cbor::Map(pairs)
        }
        Record::Outbound(outbound) => {
            let mut pairs = vec![
                (10, Cbor::Uint(3)),
                (
                    40,
                    Cbor::Bytes(outbound.intent.emitting_id.as_bytes().to_vec()),
                ),
                (41, Cbor::Text(outbound.intent.connector_path.to_string())),
            ];
            if let Some(key) = &outbound.intent.idempotency_key {
                pairs.push((42, Cbor::Bytes(key.clone().into_bytes())));
            }
            pairs.push((43, Cbor::Bytes(outbound.intent.request_digest.to_vec())));
            let (status, closure) = match &outbound.status {
                OutboundStatus::Open => (1, None),
                OutboundStatus::Closed(Closure::Response(outcome)) => (
                    2,
                    Some(Cbor::Array(vec![Cbor::Uint(1), reply_spine(outcome)])),
                ),
                OutboundStatus::Closed(Closure::Timeout) => {
                    (2, Some(Cbor::Array(vec![Cbor::Uint(2)])))
                }
                OutboundStatus::Abandoned => (4, Some(Cbor::Array(Vec::new()))),
            };
            pairs.push((44, Cbor::Uint(status)));
            if let Some(closure) = closure {
                pairs.push((45, closure));
            }
            Cbor::Map(pairs)
        }
    }
}

/// The envelope spine, keys 1–9. Absent fields are omitted keys,
/// never nulls.
fn spine(envelope: &Envelope) -> Cbor {
    let sys = envelope.meta().sys();
    let mut pairs = vec![(1, Cbor::Bytes(sys.id().as_bytes().to_vec()))];
    if let Some(parent) = sys.parent_id() {
        pairs.push((2, Cbor::Bytes(parent.as_bytes().to_vec())));
    }
    pairs.push((3, Cbor::Bytes(sys.origin_id().as_bytes().to_vec())));
    if sys.role() != Role::Default {
        pairs.push((4, Cbor::Text(sys.role().to_string())));
    }
    pairs.push((5, Cbor::Uint(sys.graph_version())));
    if let Some(mime) = sys.mime_type() {
        pairs.push((6, Cbor::Text(String::from(mime))));
    }
    pairs.push((8, data_form(envelope.data())));
    let mut app_meta = Vec::new();
    for (field, value) in envelope.meta().app() {
        app_meta.push((String::from("app"), field.clone(), value));
    }
    for (namespace, fields) in envelope.meta().customs() {
        for (field, value) in fields {
            app_meta.push((namespace.clone(), field.clone(), value));
        }
    }
    if !app_meta.is_empty() {
        app_meta.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        pairs.push((
            9,
            Cbor::Array(
                app_meta
                    .into_iter()
                    .map(|(namespace, field, value)| {
                        Cbor::Array(vec![
                            Cbor::Text(namespace),
                            Cbor::Text(field),
                            meta_tagged(value),
                        ])
                    })
                    .collect(),
            ),
        ));
    }
    Cbor::Map(pairs)
}

/// `data` (spine key 8): `[encoding_tag, bytes]`. Encoded payloads
/// seal their original bytes, never re-encoded. A materialized value
/// has no storage bytes, so its canonical bytes are its own
/// `tagged_value` CBOR under `raw` — deterministic and injective,
/// this build's materialization rule.
fn data_form(data: &Data) -> Cbor {
    match data {
        Data::Encoded {
            encoding, bytes, ..
        } => Cbor::Array(vec![
            Cbor::Uint(match encoding {
                Encoding::Raw => 1,
                Encoding::Avro => 2,
                Encoding::ArrowIpc => 3,
            }),
            Cbor::Bytes(bytes.clone()),
        ]),
        Data::Materialized(value) => Cbor::Array(vec![
            Cbor::Uint(1),
            Cbor::Bytes(encode(&value_tagged(value))),
        ]),
    }
}

/// A closure reply is raw world material — no minted identity yet
/// (the connector's transaction mints the outcome envelope). This
/// build seals it as a data-only spine; the full reply spine joins
/// when closures carry envelopes.
fn reply_spine(outcome: &crate::boundary::CallOutcome) -> Cbor {
    use crate::boundary::CallOutcome;
    match outcome {
        CallOutcome::Success(data) => Cbor::Map(vec![(8, data_form(data))]),
        CallOutcome::Failure(error) => {
            let wrapper = Cbor::Array(vec![
                Cbor::Uint(17),
                Cbor::Array(vec![
                    Cbor::Array(vec![
                        Cbor::Text(String::from("detail")),
                        data_form(&error.data),
                    ]),
                    Cbor::Array(vec![
                        Cbor::Text(String::from("kind")),
                        Cbor::Array(vec![Cbor::Uint(8), Cbor::Text(error.kind.clone())]),
                    ]),
                ]),
            ]);
            Cbor::Map(vec![(
                8,
                Cbor::Array(vec![Cbor::Uint(1), Cbor::Bytes(encode(&wrapper))]),
            )])
        }
    }
}

/// A `tagged_value` for an envelope value: `[type_tag, value]` per
/// the fixed registry.
fn value_tagged(value: &envelope::Value) -> Cbor {
    use envelope::Value;
    match value {
        Value::Scalar(Scalar::Bool(v)) => Cbor::Array(vec![Cbor::Uint(1), Cbor::Bool(*v)]),
        Value::Scalar(Scalar::Int(v)) => int_tagged(*v),
        Value::Scalar(Scalar::Double(v)) => Cbor::Array(vec![Cbor::Uint(5), Cbor::Double(*v)]),
        Value::Scalar(Scalar::Text(v)) => Cbor::Array(vec![Cbor::Uint(8), Cbor::Text(v.clone())]),
        Value::Scalar(Scalar::Bytes(v)) => Cbor::Array(vec![Cbor::Uint(7), Cbor::Bytes(v.clone())]),
        Value::Struct(fields) => {
            // Canonical sorted pairs — construction order is not
            // identity material.
            let mut sorted: Vec<_> = fields.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            Cbor::Array(vec![
                Cbor::Uint(17),
                Cbor::Array(
                    sorted
                        .into_iter()
                        .map(|(field, value)| {
                            Cbor::Array(vec![Cbor::Text(field.clone()), value_tagged(value)])
                        })
                        .collect(),
                ),
            ])
        }
        Value::Map(entries) => {
            // Sorted by the KEY'S canonical tagged bytes, not by
            // Rust's string order — CBOR text encodes its length
            // first, so "b" precedes "aa".
            let mut pairs: Vec<(Vec<u8>, Cbor)> = entries
                .iter()
                .map(|(key, value)| {
                    let tagged_key = Cbor::Array(vec![Cbor::Uint(8), Cbor::Text(key.clone())]);
                    (
                        encode(&tagged_key),
                        Cbor::Array(vec![tagged_key, value_tagged(value)]),
                    )
                })
                .collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            Cbor::Array(vec![
                Cbor::Uint(18),
                Cbor::Array(pairs.into_iter().map(|(_, pair)| pair).collect()),
            ])
        }
        Value::Array(items) => Cbor::Array(vec![
            Cbor::Uint(19),
            Cbor::Array(items.iter().map(value_tagged).collect()),
        ]),
    }
}

/// A `tagged_value` for a CEL value (state deltas).
fn cel_tagged(value: &cel::Value) -> Cbor {
    match value {
        cel::Value::Bool(v) => Cbor::Array(vec![Cbor::Uint(1), Cbor::Bool(*v)]),
        cel::Value::Int(v) => int_tagged(*v),
        cel::Value::Uint(v) => Cbor::Array(vec![Cbor::Uint(3), Cbor::Uint(*v)]),
        cel::Value::Double(v) => Cbor::Array(vec![Cbor::Uint(5), Cbor::Double(*v)]),
        cel::Value::Text(v) => Cbor::Array(vec![Cbor::Uint(8), Cbor::Text(v.clone())]),
        cel::Value::Bytes(v) => Cbor::Array(vec![Cbor::Uint(7), Cbor::Bytes(v.clone())]),
        cel::Value::Duration(v) => Cbor::Array(vec![Cbor::Uint(16), int_cbor(*v)]),
        cel::Value::Timestamp(v) => Cbor::Array(vec![Cbor::Uint(12), int_cbor(*v)]),
        cel::Value::Null => Cbor::Array(vec![Cbor::Uint(9), Cbor::Null]),
        cel::Value::List(items) => Cbor::Array(vec![
            Cbor::Uint(19),
            Cbor::Array(items.iter().map(cel_tagged).collect()),
        ]),
        cel::Value::Map(entries) => {
            let mut pairs: Vec<(Vec<u8>, Cbor)> = entries
                .iter()
                .map(|(key, value)| {
                    let tagged_key = Cbor::Array(vec![Cbor::Uint(8), Cbor::Text(key.clone())]);
                    (
                        encode(&tagged_key),
                        Cbor::Array(vec![tagged_key, cel_tagged(value)]),
                    )
                })
                .collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            Cbor::Array(vec![
                Cbor::Uint(18),
                Cbor::Array(pairs.into_iter().map(|(_, pair)| pair).collect()),
            ])
        }
    }
}

/// A `tagged_value` for writable metadata (flat by type).
fn meta_tagged(value: &envelope::MetaValue) -> Cbor {
    match value {
        envelope::MetaValue::Scalar(scalar) => {
            value_tagged(&envelope::Value::Scalar(scalar.clone()))
        }
        envelope::MetaValue::Array(items) => Cbor::Array(vec![
            Cbor::Uint(19),
            Cbor::Array(
                items
                    .iter()
                    .map(|scalar| value_tagged(&envelope::Value::Scalar(scalar.clone())))
                    .collect(),
            ),
        ]),
    }
}

/// `int` (tag 2) inside the 32-bit range, `long` (tag 3) beyond — the
/// DSL's `int` is 32-bit; runtime arithmetic that leaves the range
/// seals honestly as a long.
fn int_tagged(value: i64) -> Cbor {
    let tag = if i32::try_from(value).is_ok() { 2 } else { 3 };
    Cbor::Array(vec![Cbor::Uint(tag), int_cbor(value)])
}

fn int_cbor(value: i64) -> Cbor {
    if value >= 0 {
        Cbor::Uint(u64::try_from(value).unwrap_or_else(|_| unreachable!("non-negative")))
    } else {
        Cbor::Int(value)
    }
}

// ----- the deterministic CBOR encoder (RFC 8949 §4.2) -----

/// The canonical CBOR subset the tables need. Map keys are the fixed
/// unsigned registry keys, emitted in table order — which is sorted
/// order, satisfying §4.2 at zero runtime cost.
#[derive(Debug, Clone, PartialEq)]
enum Cbor {
    Uint(u64),
    /// A negative integer (major type 1); non-negatives use `Uint`.
    Int(i64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Cbor>),
    Map(Vec<(u64, Cbor)>),
    Bool(bool),
    Null,
    /// Always IEEE-754 binary64 — width fixed by the type tag, the
    /// stated exception to shortest-form. NaN canonicalizes to the
    /// one quiet NaN; `-0.0` stays distinct from `+0.0`.
    Double(f64),
}

fn encode(value: &Cbor) -> Vec<u8> {
    let mut out = Vec::new();
    write(value, &mut out);
    out
}

fn write(value: &Cbor, out: &mut Vec<u8>) {
    match value {
        Cbor::Uint(n) => head(0, *n, out),
        Cbor::Int(n) => {
            debug_assert!(*n < 0, "non-negatives encode as Uint");
            let magnitude = u64::try_from(-1 - *n).unwrap_or_else(|_| unreachable!("i64 fits"));
            head(1, magnitude, out);
        }
        Cbor::Bytes(bytes) => {
            head(2, bytes.len() as u64, out);
            out.extend_from_slice(bytes);
        }
        Cbor::Text(text) => {
            head(3, text.len() as u64, out);
            out.extend_from_slice(text.as_bytes());
        }
        Cbor::Array(items) => {
            head(4, items.len() as u64, out);
            for item in items {
                write(item, out);
            }
        }
        Cbor::Map(pairs) => {
            debug_assert!(
                pairs.windows(2).all(|window| window[0].0 < window[1].0),
                "table order is sorted order"
            );
            head(5, pairs.len() as u64, out);
            for (key, item) in pairs {
                head(0, *key, out);
                write(item, out);
            }
        }
        Cbor::Bool(b) => out.push(if *b { 0xf5 } else { 0xf4 }),
        Cbor::Null => out.push(0xf6),
        Cbor::Double(d) => {
            let bits = if d.is_nan() {
                0x7ff8_0000_0000_0000
            } else {
                d.to_bits()
            };
            out.push(0xfb);
            out.extend_from_slice(&bits.to_be_bytes());
        }
    }
}

/// A shortest-form head: major type + argument.
fn head(major: u8, argument: u64, out: &mut Vec<u8>) {
    let major = major << 5;
    if argument < 24 {
        out.push(major | u8::try_from(argument).unwrap_or_else(|_| unreachable!("< 24")));
    } else if argument <= u64::from(u8::MAX) {
        out.push(major | 24);
        out.push(u8::try_from(argument).unwrap_or_else(|_| unreachable!("checked")));
    } else if argument <= u64::from(u16::MAX) {
        out.push(major | 25);
        out.extend_from_slice(
            &u16::try_from(argument)
                .unwrap_or_else(|_| unreachable!("checked"))
                .to_be_bytes(),
        );
    } else if argument <= u64::from(u32::MAX) {
        out.push(major | 26);
        out.extend_from_slice(
            &u32::try_from(argument)
                .unwrap_or_else(|_| unreachable!("checked"))
                .to_be_bytes(),
        );
    } else {
        out.push(major | 27);
        out.extend_from_slice(&argument.to_be_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 8949 Appendix A vectors, restricted to the subset the
    /// tables use — the encoder is pinned to the standard
    /// independently of any trace.
    #[test]
    fn the_encoder_matches_the_rfc_8949_vectors() {
        let cases: Vec<(Cbor, &[u8])> = vec![
            (Cbor::Uint(0), &[0x00]),
            (Cbor::Uint(23), &[0x17]),
            (Cbor::Uint(24), &[0x18, 0x18]),
            (Cbor::Uint(100), &[0x18, 0x64]),
            (Cbor::Uint(1000), &[0x19, 0x03, 0xe8]),
            (Cbor::Uint(1_000_000), &[0x1a, 0x00, 0x0f, 0x42, 0x40]),
            (
                Cbor::Uint(1_000_000_000_000),
                &[0x1b, 0x00, 0x00, 0x00, 0xe8, 0xd4, 0xa5, 0x10, 0x00],
            ),
            (Cbor::Int(-1), &[0x20]),
            (Cbor::Int(-10), &[0x29]),
            (Cbor::Int(-100), &[0x38, 0x63]),
            (Cbor::Int(-1000), &[0x39, 0x03, 0xe7]),
            (Cbor::Bool(false), &[0xf4]),
            (Cbor::Bool(true), &[0xf5]),
            (Cbor::Null, &[0xf6]),
            (
                Cbor::Double(1.1),
                &[0xfb, 0x3f, 0xf1, 0x99, 0x99, 0x99, 0x99, 0x99, 0x9a],
            ),
            (
                Cbor::Double(-4.1),
                &[0xfb, 0xc0, 0x10, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66],
            ),
            (Cbor::Bytes(vec![]), &[0x40]),
            (
                Cbor::Bytes(vec![0x01, 0x02, 0x03, 0x04]),
                &[0x44, 0x01, 0x02, 0x03, 0x04],
            ),
            (Cbor::Text(String::new()), &[0x60]),
            (
                Cbor::Text(String::from("IETF")),
                &[0x64, 0x49, 0x45, 0x54, 0x46],
            ),
            (
                Cbor::Text(String::from("\u{20ac}")),
                &[0x63, 0xe2, 0x82, 0xac],
            ),
            (Cbor::Array(vec![]), &[0x80]),
            (
                Cbor::Array(vec![Cbor::Uint(1), Cbor::Uint(2), Cbor::Uint(3)]),
                &[0x83, 0x01, 0x02, 0x03],
            ),
            (Cbor::Map(vec![]), &[0xa0]),
            (
                Cbor::Map(vec![(1, Cbor::Uint(2)), (3, Cbor::Uint(4))]),
                &[0xa2, 0x01, 0x02, 0x03, 0x04],
            ),
        ];
        for (value, expected) in cases {
            assert_eq!(encode(&value), expected, "{value:?}");
        }
    }

    /// Map keys sort by their canonical tagged bytes — CBOR text
    /// encodes its length first, so `"b"` precedes `"aa"`, against
    /// Rust's string order.
    #[test]
    fn map_keys_sort_by_canonical_bytes_not_string_order() {
        let mut entries = std::collections::BTreeMap::new();
        entries.insert(
            String::from("aa"),
            crate::envelope::Value::Scalar(Scalar::Bool(true)),
        );
        entries.insert(
            String::from("b"),
            crate::envelope::Value::Scalar(Scalar::Bool(false)),
        );
        let Cbor::Array(parts) = value_tagged(&crate::envelope::Value::Map(entries)) else {
            panic!("a tagged value");
        };
        let Cbor::Array(pairs) = &parts[1] else {
            panic!("the pair list");
        };
        let Cbor::Array(first) = &pairs[0] else {
            panic!("a pair");
        };
        let Cbor::Array(key) = &first[0] else {
            panic!("a tagged key");
        };
        assert_eq!(
            key[1],
            Cbor::Text(String::from("b")),
            "shorter text is byte-smaller"
        );
    }

    /// The profile's stated deviations from shortest-form: doubles
    /// hold binary64 regardless of value, and NaN is the one
    /// canonical quiet NaN.
    #[test]
    fn floats_hold_their_fixed_width_and_canonical_nan() {
        // 1.5 is exactly representable in binary16; the profile
        // still seals binary64.
        assert_eq!(
            encode(&Cbor::Double(1.5)),
            [0xfb, 0x3f, 0xf8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        let payload_nan = f64::from_bits(0x7ff8_0000_0000_1234);
        assert_eq!(
            encode(&Cbor::Double(payload_nan)),
            [0xfb, 0x7f, 0xf8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
            "every NaN seals as the canonical quiet NaN"
        );
        assert_eq!(
            encode(&Cbor::Double(-0.0)),
            [0xfb, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
            "-0.0 stays distinct from +0.0"
        );
    }
}
