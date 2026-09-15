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
//! reason. This codec covers the closed subset the tables need —
//! writing by construction, and, since the plan package needed to
//! read its own bytes back, a STRICT reader whose success certifies
//! the bytes were canonical (`Trace::from_stored` reads goldens
//! through it too) — and it changes only when the spec does.

#[cfg(test)]
use crate::envelope::{Data, Envelope};
use crate::journal::{
    Closure, OutboundRecord, OutboundStatus, Record, SessionIngressRecord, SessionStatus,
};
use rastok_core::canonical_cbor::{Value as Cbor, decode, encode};
use rastok_core::identity::encoding::{
    data_spine as data_form, envelope_spine as spine, value_tagged,
};
use std::collections::BTreeMap;
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
            let (tree, used) = decode(bytes).ok()?;
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
                match right.get(key) {
                    None => return (at, render(value), String::from("absent")),
                    Some(other) if other != value => {
                        return explain(value, other, at);
                    }
                    Some(_) => {}
                }
            }
            for (key, value) in right {
                if !left.contains_key(key) {
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
        (Cbor::Bytes(left), Cbor::Bytes(right)) if path.ends_with("data(8)[1]") => {
            // Golden DATA retains bytes, not the materialized/encoded distinction.
            // Label this an interpretation of a recognized canonical value, never
            // pretend that an arbitrary raw byte payload had a retained text type.
            if let (Ok((left_tree, left_used)), Ok((right_tree, right_used))) =
                (decode(left), decode(right))
                && left_used == left.len()
                && right_used == right.len()
                && rastok_core::identity::encoding::value_from_tagged(&left_tree).is_ok()
                && rastok_core::identity::encoding::value_from_tagged(&right_tree).is_ok()
            {
                return explain(
                    &left_tree,
                    &right_tree,
                    format!("{path} (canonical tagged-value view)"),
                );
            }
            (path, render(a), render(b))
        }
        _ => (path, render(a), render(b)),
    }
}

/// Renders a first divergence for a user: expected, actual and retained causal
/// context. Policy-bearing plans hide values conservatively because a stored
/// golden has no independent schema-policy table. This never changes the seal.
#[must_use]
pub fn explain_diff(
    actual: &Trace,
    expected: &Trace,
    plan: &crate::plan::ValidatedExecutionPlan,
) -> Option<String> {
    let difference = diff(actual, expected)?;
    let classified = crate::data_policy::requires_redaction;
    let redact = plan.resolved_ports().boundary().iter().any(|p| classified(p.annotations()))
        || plan.nodes.iter().enumerate().any(|(index, node)| {
            plan.resolved_ports().node(crate::plan::Slot::new(index as u32))
                .is_some_and(|ports| ports.iter().any(|p| classified(p.annotations())))
                || node.vars.iter().any(|v| classified(&v.annotations))
        })
        || plan.owners.iter().any(|owner| {
            owner.ports.iter().any(|p| classified(&p.annotations))
                || owner.vals.iter().any(|v| classified(&v.annotations))
        })
        || plan.handlers.iter().any(|h| h.dispatch.steps().any(|step| {
            matches!(step, crate::plan::DispatchStep::Let { annotations, .. } if classified(annotations))
        }));
    let cause = actual
        .records
        .get(difference.record)
        .or_else(|| expected.records.get(difference.record))
        .map(|record| record_context(&record.tree))
        .unwrap_or_else(|| String::from("end of retained trace"));
    Some(format!(
        "record {}: {} — expected {}; got {}; cause: {}",
        difference.record,
        difference.path,
        if redact {
            "[REDACTED]"
        } else {
            &difference.right
        },
        if redact {
            "[REDACTED]"
        } else {
            &difference.left
        },
        if redact { "[REDACTED]" } else { &cause },
    ))
}

fn record_context(tree: &Cbor) -> String {
    let Cbor::Map(fields) = tree else {
        return String::from("unrecognized retained record");
    };
    let field = |key| {
        fields
            .get(&key)
            .map(render)
            .unwrap_or_else(|| String::from("absent"))
    };
    match fields.get(&10) {
        Some(Cbor::Uint(1)) => format!(
            "INBOUND {} · port {} · sequence {}",
            field(21),
            fields
                .get(&24)
                .map(render)
                .unwrap_or_else(|| String::from("in")),
            field(22),
        ),
        Some(Cbor::Uint(2)) => format!(
            "TRANSACTION {} · trigger {} · cause_id {}",
            field(30),
            field(32),
            field(39),
        ),
        Some(Cbor::Uint(3)) => format!(
            "OUTBOUND {} · emitting_id {} · call_key {}",
            field(41),
            field(40),
            field(42),
        ),
        _ => String::from("unrecognized retained record"),
    }
}

fn render(value: &Cbor) -> String {
    match value {
        Cbor::Uint(n) => n.to_string(),
        // Major type 1 carries −1−n; i128 renders the whole range.
        Cbor::Negative(n) => (-1 - i128::from(*n)).to_string(),
        Cbor::Bool(b) => b.to_string(),
        Cbor::Null => String::from("null"),
        Cbor::Double(d) => d.value().to_string(),
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
        24 => "ingress_port",
        25 => "routes",
        26 => "session_ingress",
        27 => "source_connector_path",
        28 => "inbound_call",
        30 => "node_path",
        31 => "instance_ref",
        32 => "trigger",
        33 => "consumed",
        34 => "join_inputs",
        35 => "state_delta",
        36 => "emissions",
        37 => "timers",
        38 => "routes",
        39 => "cause_id",
        40 => "emitting_id",
        41 => "connector_path",
        42 => "call_key",
        43 => "request_digest",
        44 => "status",
        45 => "closure",
        46 => "edge_cursor",
        47 => "edge",
        48 => "parent_instance_ref",
        49 => "close_scope_ref",
        50 => "session_exit_ref",
        51 => "session_fact",
        52 => "inbound_call_exit_ref",
        53 => "loop_fact",
        54 => "logical_time",
        55 => "loop_protocol",
        57 => "instance_commands",
        _ => return format!("key {key}"),
    };
    format!("{name}({key})")
}

// ----- projection: the per-species tables -----

fn loop_packet_tree(packet: &crate::journal::loop_protocol::Packet) -> Cbor {
    use crate::journal::loop_protocol::{CloseReason, CommandValue, Fact, OpeningAdmissionRef};
    let id = |id: crate::envelope::Id| Cbor::Bytes(id.as_bytes().to_vec());
    let mut fields = BTreeMap::new();
    if !packet.context.is_empty() {
        fields.insert(
            1,
            Cbor::Array(packet.context.iter().map(|run| id(run.id())).collect()),
        );
    }
    if !packet.routes.is_empty() {
        fields.insert(
            2,
            Cbor::Array(
                packet
                    .routes
                    .iter()
                    .map(|route| {
                        Cbor::Array(vec![
                            id(route.delivery),
                            Cbor::Array(route.runs.iter().map(|run| id(run.id())).collect()),
                        ])
                    })
                    .collect(),
            ),
        );
    }
    if !packet.produced.is_empty() {
        fields.insert(
            3,
            Cbor::Array(
                packet
                    .produced
                    .iter()
                    .map(|command| {
                        let value = match &command.value {
                            CommandValue::Next(state) => spine(state),
                            CommandValue::Again(cause) => id(*cause),
                            CommandValue::Done { cause, kind } => {
                                Cbor::Array(vec![id(*cause), Cbor::Uint(u64::from(kind.tag()))])
                            }
                        };
                        Cbor::Array(vec![
                            id(command.reference.id()),
                            id(command.run.id()),
                            Cbor::Uint(command.iteration),
                            Cbor::Array(vec![Cbor::Uint(u64::from(command.value.tag())), value]),
                        ])
                    })
                    .collect(),
            ),
        );
    }
    if !packet.consumed.is_empty() {
        fields.insert(
            4,
            Cbor::Array(
                packet
                    .consumed
                    .iter()
                    .map(|command| id(command.id()))
                    .collect(),
            ),
        );
    }
    if !packet.facts.is_empty() {
        fields.insert(
            5,
            Cbor::Array(
                packet
                    .facts
                    .iter()
                    .map(|fact| {
                        Cbor::Array(match fact {
                            Fact::Open {
                                run,
                                instance,
                                opening,
                                entry,
                                fuel,
                                state,
                            } => {
                                let opening = match opening {
                                    OpeningAdmissionRef::Ingress { inbound, port } => {
                                        Cbor::Array(vec![
                                            Cbor::Uint(1),
                                            id(*inbound),
                                            Cbor::Text(port.name().into()),
                                        ])
                                    }
                                    OpeningAdmissionRef::Delivery(delivery) => {
                                        Cbor::Array(vec![Cbor::Uint(2), id(*delivery)])
                                    }
                                };
                                vec![
                                    Cbor::Uint(1),
                                    id(run.id()),
                                    instance.tree(),
                                    opening,
                                    Cbor::Text(entry.name().into()),
                                    Cbor::Uint(*fuel),
                                    spine(state),
                                ]
                            }
                            Fact::Advance {
                                run,
                                from_iteration,
                                to_iteration,
                                remaining,
                                command,
                                state,
                                due_at: _,
                            } => vec![
                                Cbor::Uint(2),
                                id(run.id()),
                                Cbor::Uint(*from_iteration),
                                Cbor::Uint(*to_iteration),
                                Cbor::Uint(*remaining),
                                id(command.id()),
                                spine(state),
                            ],
                            Fact::Admit {
                                run,
                                iteration,
                                admission,
                                envelope,
                            } => vec![
                                Cbor::Uint(3),
                                id(run.id()),
                                Cbor::Uint(*iteration),
                                id(*admission),
                                spine(envelope),
                            ],
                            Fact::Close {
                                run,
                                iteration,
                                reason,
                            } => vec![
                                Cbor::Uint(4),
                                id(run.id()),
                                Cbor::Uint(*iteration),
                                Cbor::Uint(match reason {
                                    CloseReason::Natural => 1,
                                    CloseReason::ExplicitDone => 2,
                                    CloseReason::Stopped => 3,
                                }),
                            ],
                            Fact::Expired {
                                run,
                                iteration,
                                command,
                                last_state,
                                output,
                            } => vec![
                                Cbor::Uint(5),
                                id(run.id()),
                                Cbor::Uint(*iteration),
                                id(command.id()),
                                spine(last_state),
                                spine(output),
                            ],
                        })
                    })
                    .collect(),
            ),
        );
    }
    Cbor::Map(fields)
}

fn record_tree(record: &Record) -> Cbor {
    match record {
        Record::Inbound(inbound) => {
            let mut pairs = BTreeMap::from([
                (10, Cbor::Uint(1)),
                (20, spine(&inbound.envelope)),
                (
                    21,
                    Cbor::Text(inbound.provenance.connector_path().to_string()),
                ),
                (22, Cbor::Uint(inbound.seq)),
                (23, Cbor::Uint(inbound.provenance.replica())),
            ]);
            if let Some(packet) = inbound.instance_commands.as_deref() {
                pairs.insert(crate::journal::instance_stop::CAUSAL_KEY, packet.tree());
            }
            // Key 24: the source port — boundary input without key 27,
            // Connector output with it. Conventional `in` remains omitted,
            // so every existing boundary trace stays byte-identical.
            if inbound.port.name() != "in" {
                pairs.insert(24, Cbor::Text(inbound.port.name().to_string()));
            }
            if !inbound.routes.is_empty() {
                let mut routes = inbound.routes.clone();
                routes.sort();
                pairs.insert(
                    25,
                    Cbor::Array(
                        routes
                            .into_iter()
                            .map(|route| {
                                Cbor::Array(vec![Cbor::Text(route.edge), route.target.tree()])
                            })
                            .collect(),
                    ),
                );
            }
            if let Some(exchange) = inbound.exchange.as_deref() {
                match exchange {
                    crate::journal::InboundExchangeRecord::Call(call) => {
                        let mut fields = vec![
                            Cbor::Bytes(call.reference.id().as_bytes().to_vec()),
                            call.instance.tree(),
                        ];
                        if !call.timers.is_empty() {
                            fields.push(Cbor::Array(
                                call.timers
                                    .iter()
                                    .map(|(kind, reference)| {
                                        Cbor::Array(vec![
                                            Cbor::Uint(u64::from(*kind)),
                                            Cbor::Bytes(reference.as_bytes().to_vec()),
                                        ])
                                    })
                                    .collect(),
                            ));
                        }
                        pairs.insert(28, Cbor::Array(fields));
                    }
                    crate::journal::InboundExchangeRecord::Session(session) => {
                        let (kind, reference) = match session {
                            SessionIngressRecord::Open(reference) => (1, reference),
                            SessionIngressRecord::Message(reference) => (2, reference),
                        };
                        pairs.insert(
                            26,
                            Cbor::Array(vec![
                                Cbor::Uint(kind),
                                Cbor::Bytes(reference.id().as_bytes().to_vec()),
                            ]),
                        );
                    }
                }
            }
            if let Some(connector) = &inbound.connector_path {
                pairs.insert(
                    27,
                    Cbor::Array(
                        connector
                            .segments()
                            .iter()
                            .cloned()
                            .map(Cbor::Text)
                            .collect(),
                    ),
                );
            }
            Cbor::Map(pairs)
        }
        Record::Transaction(transaction) => {
            let mut pairs = BTreeMap::from([
                (10, Cbor::Uint(2)),
                (30, Cbor::Text(transaction.owner_path.to_string())),
                (32, Cbor::Text(transaction.trigger.to_string())),
            ]);
            pairs.insert(31, transaction.instance.tree());
            if let Some(parent) = &transaction.parent_instance {
                pairs.insert(48, parent.tree());
            }
            if let Some(scope) = &transaction.close_scope {
                pairs.insert(49, scope.tree());
            }
            match transaction.exchange_exit.as_deref() {
                Some(crate::journal::InboundExchangeExitRecord::Session(session)) => {
                    pairs.insert(50, Cbor::Bytes(session.id().as_bytes().to_vec()));
                }
                Some(crate::journal::InboundExchangeExitRecord::Call(call)) => {
                    pairs.insert(52, Cbor::Bytes(call.id().as_bytes().to_vec()));
                }
                None => {}
            }
            if let Some(logical_time) = transaction.logical_time {
                pairs.insert(
                    54,
                    if logical_time >= 0 {
                        Cbor::Uint(logical_time as u64)
                    } else {
                        Cbor::Negative(logical_time.unsigned_abs() - 1)
                    },
                );
            }
            if let Some(packet) = transaction.loop_protocol() {
                pairs.insert(55, loop_packet_tree(packet));
            }
            if let Some(packet) = transaction.instance_commands.as_deref() {
                pairs.insert(crate::journal::instance_stop::CAUSAL_KEY, packet.tree());
            }
            if let Some(cause) = transaction.cause {
                pairs.insert(39, Cbor::Bytes(cause.as_bytes().to_vec()));
            }
            let mut consumed: Vec<&crate::journal::ConsumedPosition> =
                transaction.consumed.iter().collect();
            consumed.sort();
            pairs.insert(
                33,
                Cbor::Array(
                    consumed
                        .iter()
                        .map(|position| {
                            Cbor::Array(vec![
                                Cbor::Text(position.edge.clone()),
                                position.source.tree(),
                                Cbor::Uint(position.seq),
                            ])
                        })
                        .collect(),
                ),
            );
            if !transaction.join_inputs.is_empty() {
                pairs.insert(
                    34,
                    Cbor::Array(
                        transaction
                            .join_inputs
                            .iter()
                            .map(|id| Cbor::Bytes(id.as_bytes().to_vec()))
                            .collect(),
                    ),
                );
            }
            if !transaction.state_delta.is_empty() {
                let mut delta: Vec<_> = transaction.state_delta.iter().collect();
                delta.sort_by(|a, b| a.0.cmp(&b.0));
                pairs.insert(
                    35,
                    Cbor::Array(
                        delta
                            .iter()
                            .map(|(field, value)| {
                                Cbor::Array(vec![Cbor::Text(field.clone()), value_tagged(value)])
                            })
                            .collect(),
                    ),
                );
            }
            pairs.insert(
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
            );
            if !transaction.timers.is_empty() {
                let mut timers: Vec<_> = transaction.timers.iter().collect();
                timers.sort();
                pairs.insert(
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
                );
            }
            if !transaction.routes.is_empty() {
                let mut routes = transaction.routes.clone();
                routes.sort();
                pairs.insert(
                    38,
                    Cbor::Array(
                        routes
                            .into_iter()
                            .map(|route| {
                                Cbor::Array(vec![
                                    Cbor::Bytes(route.delivery.as_bytes().to_vec()),
                                    Cbor::Text(route.edge),
                                    route.target.tree(),
                                ])
                            })
                            .collect(),
                    ),
                );
            }
            Cbor::Map(pairs)
        }
        Record::Outbound(OutboundRecord::Call(outbound)) => {
            let mut pairs = BTreeMap::from([
                (10, Cbor::Uint(3)),
                (
                    40,
                    Cbor::Bytes(outbound.intent.emitting_id.as_bytes().to_vec()),
                ),
                (41, Cbor::Text(outbound.intent.connector_path.to_string())),
            ]);
            if let Some(key) = &outbound.intent.idempotency_key {
                pairs.insert(42, Cbor::Bytes(key.clone().into_bytes()));
            }
            pairs.insert(43, Cbor::Bytes(outbound.intent.request_digest().to_vec()));
            let (status, closure) = match &outbound.status {
                OutboundStatus::Open => (1, None),
                OutboundStatus::Closed(Closure::Response(completion)) => (
                    2,
                    Some(Cbor::Array(vec![
                        Cbor::Uint(1),
                        completion_spine(completion),
                    ])),
                ),
                OutboundStatus::Closed(Closure::Timeout) => {
                    (2, Some(Cbor::Array(vec![Cbor::Uint(2)])))
                }
                OutboundStatus::Abandoned => (4, Some(Cbor::Array(Vec::new()))),
            };
            pairs.insert(44, Cbor::Uint(status));
            if let Some(closure) = closure {
                pairs.insert(45, closure);
            }
            // Key 47: the causing delivery's canonical edge — with
            // key 40 the record reconstructs delivery_ref and hence
            // call_ref, autonomously (identity.md §3).
            pairs.insert(47, Cbor::Text(outbound.intent.edge.clone()));
            Cbor::Map(pairs)
        }
        Record::Outbound(OutboundRecord::Session(session)) => {
            let fact = match &session.status {
                SessionStatus::Opening(intent) => Cbor::Array(vec![
                    Cbor::Uint(1),
                    Cbor::Bytes(session.session_ref.id().as_bytes().to_vec()),
                    Cbor::Bytes(intent.emitting_id.as_bytes().to_vec()),
                    Cbor::Text(session.connector_path.to_string()),
                    Cbor::Bytes(intent.request_digest().to_vec()),
                ]),
                SessionStatus::Opened => Cbor::Array(vec![
                    Cbor::Uint(2),
                    Cbor::Bytes(session.session_ref.id().as_bytes().to_vec()),
                    Cbor::Text(session.connector_path.to_string()),
                ]),
                SessionStatus::WriteOpen(intent) => Cbor::Array(vec![
                    Cbor::Uint(3),
                    Cbor::Bytes(session.session_ref.id().as_bytes().to_vec()),
                    Cbor::Bytes(intent.write_ref.id().as_bytes().to_vec()),
                    Cbor::Bytes(intent.emitting_id.as_bytes().to_vec()),
                    Cbor::Text(session.connector_path.to_string()),
                    Cbor::Bytes(intent.message_digest().to_vec()),
                ]),
                SessionStatus::WriteSettled(reference) => Cbor::Array(vec![
                    Cbor::Uint(4),
                    Cbor::Bytes(session.session_ref.id().as_bytes().to_vec()),
                    Cbor::Bytes(reference.id().as_bytes().to_vec()),
                    Cbor::Text(session.connector_path.to_string()),
                ]),
                SessionStatus::WriteAbandoned(reference) => Cbor::Array(vec![
                    Cbor::Uint(5),
                    Cbor::Bytes(session.session_ref.id().as_bytes().to_vec()),
                    Cbor::Bytes(reference.id().as_bytes().to_vec()),
                    Cbor::Text(session.connector_path.to_string()),
                ]),
            };
            Cbor::Map(BTreeMap::from([(10, Cbor::Uint(3)), (51, fact)]))
        }
    }
}

/// A completion is raw world material — no envelope identity exists yet.
/// The VM later mints every output in one TRANSACTION; this closure spine
/// seals only the ordered `(port, data)` drafts that won the close race.
fn completion_spine(completion: &crate::boundary::Completion) -> Cbor {
    Cbor::Array(
        completion
            .outputs()
            .iter()
            .map(|output| {
                Cbor::Array(vec![
                    Cbor::Text(output.port.name().to_string()),
                    data_form(&output.data),
                ])
            })
            .collect(),
    )
}

// ----- the deterministic CBOR encoder (RFC 8949 §4.2) -----

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_loop_protocol_registry_pins_open_admit_and_observed_time() {
        use crate::journal::loop_protocol::{Fact, OpeningAdmissionRef, Packet, run_ref};
        let owner = crate::plan::OwnerPath::root();
        let instance = crate::identity::InstanceRef::new(
            crate::identity::InstanceAddress::singleton(&owner),
            0,
        );
        let root = crate::envelope::Id::from_bytes([3; 16]);
        let state = Envelope::new(
            crate::envelope::Meta::new(crate::envelope::MetaSys::root(root, 1)),
            Data::materialized(crate::value::Value::Int(7)),
        );
        let entry = crate::node::Port::new("in").unwrap();
        let opening = OpeningAdmissionRef::Ingress {
            inbound: root,
            port: entry.clone(),
        };
        let run = run_ref(&instance, &opening);
        let record = Record::Transaction(Box::new(crate::journal::TransactionRecord {
            owner_path: owner,
            instance: instance.clone(),
            parent_instance: None,
            close_scope: None,
            trigger: crate::node::Trigger::Port(crate::node::Port::new("in").unwrap()),
            cause: Some(root),
            exchange_exit: None,
            loop_record: Some(Box::new(Packet {
                facts: vec![Fact::Open {
                    run,
                    instance: instance.clone(),
                    opening,
                    entry,
                    fuel: 9,
                    state: state.clone(),
                }],
                ..Packet::default()
            })),
            logical_time: None,
            timer_schedule: None,
            instance_commands: None,
            consumed: Vec::new(),
            join_inputs: Vec::new(),
            state_delta: Vec::new(),
            emissions: Vec::new(),
            routes: Vec::new(),
            timers: Vec::new(),
        }));

        let Cbor::Map(tree) = record_tree(&record) else {
            panic!("a transaction projects as a canonical map")
        };
        assert_eq!(
            tree.get(&55),
            Some(&Cbor::Map(BTreeMap::from([(
                5,
                Cbor::Array(vec![Cbor::Array(vec![
                    Cbor::Uint(1),
                    Cbor::Bytes(run.id().as_bytes().to_vec()),
                    instance.tree(),
                    Cbor::Array(vec![
                        Cbor::Uint(1),
                        Cbor::Bytes(root.as_bytes().to_vec()),
                        Cbor::Text("in".into())
                    ]),
                    Cbor::Text("in".into()),
                    Cbor::Uint(9),
                    spine(&state),
                ])])
            )])))
        );
        assert!(
            !tree.contains_key(&53),
            "retired record keys are not reused"
        );
        assert!(!tree.contains_key(&54), "time-insensitive facts omit time");

        let Record::Transaction(mut timed) = record.clone() else {
            unreachable!("the fixture is a transaction")
        };
        timed.logical_time = Some(-1);
        let Cbor::Map(timed_tree) = record_tree(&Record::Transaction(timed)) else {
            panic!("a transaction projects as a canonical map")
        };
        assert_eq!(timed_tree.get(&54), Some(&Cbor::Negative(0)));

        let Record::Transaction(mut admit) = record else {
            unreachable!("the fixture is a transaction")
        };
        admit.trigger = crate::node::Trigger::Lifecycle(crate::node::Lifecycle::Next);
        admit.loop_record = Some(Box::new(Packet {
            facts: vec![Fact::Admit {
                run,
                iteration: 0,
                admission: root,
                envelope: state.clone(),
            }],
            ..Packet::default()
        }));
        let Cbor::Map(admit_tree) = record_tree(&Record::Transaction(admit)) else {
            panic!("an admission projects as a canonical map")
        };
        assert_eq!(
            admit_tree.get(&55),
            Some(&Cbor::Map(BTreeMap::from([(
                5,
                Cbor::Array(vec![Cbor::Array(vec![
                    Cbor::Uint(3),
                    Cbor::Bytes(run.id().as_bytes().to_vec()),
                    Cbor::Uint(0),
                    Cbor::Bytes(root.as_bytes().to_vec()),
                    spine(&state),
                ])])
            )])))
        );
    }

    #[test]
    fn a_connector_root_seals_its_structural_source_path_under_key_27() {
        let source = crate::node::Path::new(["api", "listener"]).unwrap();
        let id = crate::envelope::Id::from_bytes([7; 16]);
        let record = Record::Inbound(Box::new(crate::journal::InboundRecord {
            timer_schedule: None,
            instance_commands: None,
            provenance: crate::boundary::InboundEnvelopeProvenance::single(source.clone()),
            port: crate::node::Port::new("out").unwrap(),
            connector_path: Some(source),
            seq: 0,
            envelope: Envelope::new(
                crate::envelope::Meta::new(crate::envelope::MetaSys::root(id, 1)),
                Data::materialized(crate::value::Value::Null),
            ),
            routes: Vec::new(),
            exchange: None,
        }));

        let Cbor::Map(tree) = record_tree(&record) else {
            panic!("an inbound record projects as a canonical map")
        };
        assert_eq!(
            tree.get(&27),
            Some(&Cbor::Array(vec![
                Cbor::Text(String::from("api")),
                Cbor::Text(String::from("listener")),
            ]))
        );
    }

    /// The diff must descend TO the float: `-0.0` and `+0.0` differ
    /// in bytes and now differ as VALUES too (bit equality), so
    /// `explain` names the exact field instead of stopping above an
    /// equality that contradicted the bytes.
    #[test]
    fn the_diff_names_a_zero_sign_divergence() {
        let record = |zero: f64| {
            encode(&Cbor::Map(BTreeMap::from([(
                8,
                Cbor::Double(rastok_core::canonical_cbor::CanonicalDouble::new(zero)),
            )])))
        };
        let ours = Trace::from_stored(&[record(-0.0)]).expect("canonical bytes");
        let theirs = Trace::from_stored(&[record(0.0)]).expect("canonical bytes");
        let divergence = diff(&ours, &theirs).expect("the traces differ");
        assert_eq!(divergence.record, 0);
        assert_eq!(divergence.path, "data(8)");
        assert_eq!(divergence.left, "-0");
        assert_eq!(divergence.right, "0");
    }
}
