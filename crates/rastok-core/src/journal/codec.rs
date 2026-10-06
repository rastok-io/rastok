//! Lossless record codec, shared by journal tooling and diagnostics.
//! These rows are not the causal seal: encoded DATA, Completion metadata,
//! prepared bytes and deadlines survive. File framing, version negotiation,
//! checksums, policy and I/O belong to the hosted recorder, not this codec.

use crate::boundary::*;
use crate::canonical;
use crate::canonical_cbor::Value as Cbor;
use crate::envelope::{
    Data, Encoding, Envelope, Id, Meta, MetaSys, MetaValue, Namespace, Role, Scalar,
};
use crate::instance;

use crate::journal::loop_protocol as lp;
use crate::journal::{self, Record};
use crate::node::{self, Lifecycle, Port, Trigger};

use crate::value::Value;
use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::ToString;
use alloc::vec::Vec;
use core::fmt;

mod retry;
pub use retry::retry_packet;

/// Lossless carrier decoding failure, distinct from journal/plan acceptance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A row violates the closed shape, value or retained-reference contract.
    Invalid(&'static str),
    /// A typed canonical field cannot be decoded.
    Read(crate::canonical_cbor::DecodeError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(f, "invalid retained record: {reason}"),
            Self::Read(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for Error {}

fn list(items: impl IntoIterator<Item = Cbor>) -> Cbor {
    Cbor::Array(items.into_iter().collect())
}
fn word(value: impl ToString) -> Cbor {
    Cbor::Text(value.to_string())
}
fn integer(value: i64) -> Cbor {
    if value >= 0 {
        Cbor::Uint(value as u64)
    } else {
        Cbor::Negative(value.unsigned_abs() - 1)
    }
}
fn id(value: Id) -> Cbor {
    Cbor::Bytes(value.as_bytes().to_vec())
}
fn optional<T>(value: Option<&T>, encode: impl FnOnce(&T) -> Cbor) -> Cbor {
    value.map_or(Cbor::Null, encode)
}
fn array<const N: usize>(value: &Cbor) -> Result<&[Cbor; N], Error> {
    items(value)?
        .try_into()
        .map_err(|_| Error::Invalid("row arity"))
}
fn items(value: &Cbor) -> Result<&[Cbor], Error> {
    match value {
        Cbor::Array(items) => Ok(items),
        _ => Err(Error::Invalid("array")),
    }
}
fn text(value: &Cbor) -> Result<&str, Error> {
    match value {
        Cbor::Text(value) => Ok(value),
        _ => Err(Error::Invalid("text")),
    }
}
fn bytes(value: &Cbor) -> Result<&[u8], Error> {
    match value {
        Cbor::Bytes(value) => Ok(value),
        _ => Err(Error::Invalid("bytes")),
    }
}
fn uint(value: &Cbor) -> Result<u64, Error> {
    match value {
        Cbor::Uint(value) => Ok(*value),
        _ => Err(Error::Invalid("unsigned integer")),
    }
}
fn signed(value: &Cbor) -> Result<i64, Error> {
    match value {
        Cbor::Uint(value) => i64::try_from(*value).map_err(|_| Error::Invalid("integer range")),
        Cbor::Negative(value) => i64::try_from(*value)
            .map(|value| -1 - value)
            .map_err(|_| Error::Invalid("integer range")),
        _ => Err(Error::Invalid("integer")),
    }
}
fn read_id(value: &Cbor) -> Result<Id, Error> {
    Ok(Id::from_bytes(
        bytes(value)?
            .try_into()
            .map_err(|_| Error::Invalid("128-bit identity"))?,
    ))
}
fn read_optional<T>(
    value: &Cbor,
    decode: impl FnOnce(&Cbor) -> Result<T, Error>,
) -> Result<Option<T>, Error> {
    if matches!(value, Cbor::Null) {
        Ok(None)
    } else {
        decode(value).map(Some)
    }
}
fn read_port(value: &Cbor) -> Result<Port, Error> {
    Port::new(text(value)?).map_err(|_| Error::Invalid("port"))
}
fn read_path(value: &Cbor) -> Result<node::Path, Error> {
    node::Path::new(text(value)?.split('.')).map_err(|_| Error::Invalid("node path"))
}
fn read_owner(value: &Cbor) -> Result<node::OwnerPath, Error> {
    let value = text(value)?;
    node::OwnerPath::new(value.split('.').filter(|value| !value.is_empty()))
        .map_err(|_| Error::Invalid("owner path"))
}
fn instance(value: &Cbor) -> Result<instance::Generation, Error> {
    let [address, generation] = array(value)?;
    Ok(instance::Generation::new(
        instance::Identity::decode(bytes(address)?).map_err(Error::Read)?,
        uint(generation)?,
    ))
}
fn data(value: &Data) -> Cbor {
    match value {
        Data::Materialized(value) => list([Cbor::Uint(0), canonical::value(value)]),
        Data::Encoded {
            encoding, bytes, ..
        } => list([
            Cbor::Uint(match encoding {
                Encoding::Raw => 1,
                Encoding::Avro => 2,
                Encoding::ArrowIpc => 3,
            }),
            Cbor::Bytes(bytes.to_vec()),
        ]),
    }
}
fn read_data(value: &Cbor) -> Result<Data, Error> {
    let [tag, payload] = array(value)?;
    match uint(tag)? {
        0 => {
            let value = canonical::decode_value(payload).map_err(Error::Read)?;
            value
                .check_consistency()
                .map_err(|_| Error::Invalid("logical value"))?;
            Ok(Data::Materialized(value))
        }
        tag @ 1..=3 => Ok(Data::encoded(
            match tag {
                1 => Encoding::Raw,
                2 => Encoding::Avro,
                _ => Encoding::ArrowIpc,
            },
            bytes(payload)?.to_vec(),
        )),
        _ => Err(Error::Invalid("DATA representation")),
    }
}
fn namespace(value: &Namespace) -> Cbor {
    list(
        value
            .iter()
            .map(|(name, value)| list([word(name), canonical::meta_value(value)])),
    )
}
fn scalar(value: Value) -> Result<Scalar, Error> {
    Ok(match value {
        Value::Bool(value) => Scalar::Bool(value),
        Value::Int(value) => Scalar::Int(value),
        Value::Double(value) => Scalar::Double(value),
        Value::Text(value) => Scalar::Text(value),
        Value::Bytes(value) => Scalar::Bytes(value),
        Value::Decimal { unscaled, scale } => Scalar::Decimal { unscaled, scale },
        _ => return Err(Error::Invalid("metadata scalar")),
    })
}
fn scalar_array(value: Value) -> Result<Vec<Scalar>, Error> {
    let Value::Array(values) = value else {
        return Err(Error::Invalid("metadata array"));
    };
    values.into_iter().map(scalar).collect()
}
fn read_namespace(value: &Cbor) -> Result<Namespace, Error> {
    let mut namespace = Namespace::new();
    for field in items(value)? {
        let [name, value] = array(field)?;
        let value = match canonical::decode_value(value).map_err(Error::Read)? {
            Value::Array(values) => {
                MetaValue::Array(values.into_iter().map(scalar).collect::<Result<_, _>>()?)
            }
            Value::Map(values) => MetaValue::Map(
                values
                    .into_iter()
                    .map(|(key, value)| Ok((key, scalar_array(value)?)))
                    .collect::<Result<_, Error>>()?,
            ),
            value => MetaValue::Scalar(scalar(value)?),
        };
        if namespace.insert(text(name)?.to_owned(), value).is_some() {
            return Err(Error::Invalid("duplicate metadata field"));
        }
    }
    Ok(namespace)
}
fn envelope(value: &Envelope) -> Cbor {
    let sys = value.meta().sys();
    list([
        id(sys.id()),
        optional(sys.parent_id().as_ref(), |value| id(*value)),
        id(sys.origin_id()),
        word(sys.role()),
        Cbor::Uint(sys.graph_version()),
        namespace(value.meta().app()),
        namespace(value.meta().ext()),
        data(value.data()),
    ])
}
fn deadline_ids(values: &[(journal::DeadlineKind, Id)]) -> Cbor {
    list(values.iter().map(|(kind, reference)| {
        list([
            Cbor::Uint(u64::from(journal::deadline_kind_tag(*kind))),
            id(*reference),
        ])
    }))
}
fn read_deadline_ids(value: &Cbor) -> Result<Vec<(journal::DeadlineKind, Id)>, Error> {
    items(value)?
        .iter()
        .map(|value| {
            let [kind, reference] = array(value)?;
            Ok((
                journal::DeadlineKind::try_from(
                    u8::try_from(uint(kind)?)
                        .map_err(|_| Error::Invalid("deadline kind exceeds u8"))?,
                )
                .map_err(|_| Error::Invalid("unknown deadline kind"))?,
                read_id(reference)?,
            ))
        })
        .collect()
}
fn schedule(value: Option<&journal::timing::Deadlines>) -> Cbor {
    optional(value, |value| Cbor::Bytes(value.encode()))
}
fn read_schedule(value: &Cbor) -> Result<Option<Box<journal::timing::Deadlines>>, Error> {
    read_optional(value, |value| {
        journal::timing::Deadlines::decode(bytes(value)?)
            .map(Box::new)
            .map_err(Error::Read)
    })
}
fn completion(value: &Completion) -> Cbor {
    list(value.outputs().iter().map(|output| {
        list([
            word(&output.port),
            data(&output.data),
            namespace(&output.ext),
        ])
    }))
}
fn read_completion(value: &Cbor) -> Result<Completion, Error> {
    let outputs = items(value)?
        .iter()
        .map(|value| {
            let [port, data, ext] = array(value)?;
            Ok(PortOutput {
                port: read_port(port)?,
                data: read_data(data)?,
                ext: read_namespace(ext)?,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Completion::try_new(outputs).map_err(|_| Error::Invalid("Call completion"))
}
fn intent(value: &Intent) -> Cbor {
    list([
        id(value.call_id.id()),
        id(value.emitting_id),
        word(&value.edge),
        word(&value.connector_path),
        Cbor::Bytes(value.prepared.bytes().to_vec()),
        optional(value.idempotency_key.as_ref(), |value| word(value)),
        list(
            value
                .resolved_settings
                .iter()
                .map(|(name, value)| list([word(name), canonical::value(value)])),
        ),
        integer(value.logical_time),
    ])
}
fn read_intent(value: &Cbor) -> Result<Intent, Error> {
    let [call, emitting, edge, path, prepared, key, settings, at] = array(value)?;
    let mut resolved_settings = BTreeMap::new();
    for setting in items(settings)? {
        let [name, value] = array(setting)?;
        if resolved_settings
            .insert(
                text(name)?.to_owned(),
                canonical::decode_value(value).map_err(Error::Read)?,
            )
            .is_some()
        {
            return Err(Error::Invalid("duplicate resolved setting"));
        }
    }
    Ok(Intent {
        call_id: CallId::new(read_id(call)?),
        emitting_id: read_id(emitting)?,
        edge: text(edge)?.to_owned(),
        connector_path: read_path(path)?,
        prepared: PreparedRequest::seal(bytes(prepared)?.to_vec()),
        idempotency_key: read_optional(key, |value| Ok(text(value)?.to_owned()))?,
        resolved_settings,
        logical_time: signed(at)?,
    })
}

/// Encode the complete record, sealed and unsealed fields, without file framing.
#[must_use]
pub fn encode(value: &Record) -> Cbor {
    match value {
        Record::Ingress(value) => list([
            Cbor::Uint(1),
            word(value.provenance.connector_path()),
            Cbor::Uint(value.provenance.replica()),
            word(&value.port),
            optional(value.connector_path.as_ref(), |value| word(value)),
            Cbor::Uint(value.seq),
            envelope(&value.envelope),
            list(
                value
                    .routes
                    .iter()
                    .map(|route| list([word(&route.edge), route.target.tree()])),
            ),
            optional(value.exchange.as_deref(), |exchange| match exchange {
                journal::InboundExchangeRecord::Call(call) => list([
                    Cbor::Uint(1),
                    id(call.reference.id()),
                    call.instance.tree(),
                    deadline_ids(&call.deadline_ids),
                ]),
                journal::InboundExchangeRecord::Session(session) => match session {
                    journal::SessionIngressRecord::Open(reference) => {
                        list([Cbor::Uint(2), id(reference.id())])
                    }
                    journal::SessionIngressRecord::Message(reference) => {
                        list([Cbor::Uint(3), id(reference.id())])
                    }
                },
            }),
            schedule(value.deadlines.as_deref()),
            optional(
                value.instance_commands.as_deref(),
                journal::instance_stop::Packet::tree,
            ),
        ]),
        Record::Transaction(value) => {
            list([
                Cbor::Uint(2),
                word(&value.owner_path),
                value.instance.tree(),
                optional(value.parent_instance.as_ref(), instance::Generation::tree),
                optional(value.close_scope.as_ref(), instance::Generation::tree),
                word(&value.trigger),
                optional(value.cause.as_ref(), |value| id(*value)),
                optional(value.exchange_exit.as_deref(), |exit| match exit {
                    journal::InboundExchangeExitRecord::Call(reference) => {
                        list([Cbor::Uint(1), id(reference.id())])
                    }
                    journal::InboundExchangeExitRecord::Session(reference) => {
                        list([Cbor::Uint(2), id(reference.id())])
                    }
                }),
                optional(value.loop_record.as_deref(), loop_packet),
                optional(value.logical_time.as_ref(), |value| integer(*value)),
                list(value.consumed.iter().map(|value| {
                    list([
                        word(&value.edge),
                        value.source.tree(),
                        Cbor::Uint(value.seq),
                    ])
                })),
                list(value.join_inputs.iter().map(|value| id(*value))),
                list(
                    value
                        .state_delta
                        .iter()
                        .map(|(name, value)| list([word(name), canonical::value(value)])),
                ),
                list(value.emissions.iter().map(|value| {
                    list([
                        word(&value.port),
                        Cbor::Uint(u64::from(value.index)),
                        envelope(&value.envelope),
                    ])
                })),
                list(value.routes.iter().map(|value| {
                    list([id(value.delivery), word(&value.edge), value.target.tree()])
                })),
                deadline_ids(&value.deadline_ids),
                schedule(value.deadlines.as_deref()),
                optional(
                    value.instance_commands.as_deref(),
                    journal::instance_stop::Packet::tree,
                ),
                optional(value.retry_record.as_deref(), retry_packet),
            ])
        }
        Record::Egress(journal::EgressRecord::Call(value)) => list([
            Cbor::Uint(3),
            intent(&value.intent),
            match &value.status {
                journal::CallStatus::Open => list([Cbor::Uint(1)]),
                journal::CallStatus::Closed(journal::Closure::Response(value)) => {
                    list([Cbor::Uint(2), completion(value)])
                }
                journal::CallStatus::Closed(journal::Closure::Timeout) => list([Cbor::Uint(3)]),
                journal::CallStatus::Abandoned => list([Cbor::Uint(4)]),
            },
            optional(value.closed_at.as_ref(), |value| integer(*value)),
            schedule(value.deadlines.as_deref()),
            optional(value.retry_record.as_deref(), retry_packet),
        ]),
        Record::Egress(journal::EgressRecord::Session(value)) => list([
            Cbor::Uint(4),
            word(&value.connector_path),
            id(value.session_id.id()),
            match &value.status {
                journal::SessionStatus::Opening(value) => list([
                    Cbor::Uint(1),
                    id(value.emitting_id),
                    Cbor::Bytes(value.prepared.bytes().to_vec()),
                ]),
                journal::SessionStatus::Opened => list([Cbor::Uint(2)]),
                journal::SessionStatus::WriteOpen(value) => list([
                    Cbor::Uint(3),
                    id(value.write_ref.id()),
                    id(value.emitting_id),
                    Cbor::Bytes(value.prepared.bytes().to_vec()),
                ]),
                journal::SessionStatus::WriteSettled(value) => {
                    list([Cbor::Uint(4), id(value.id())])
                }
                journal::SessionStatus::WriteAbandoned(value) => {
                    list([Cbor::Uint(5), id(value.id())])
                }
            },
        ]),
    }
}

fn loop_packet(value: &lp::Packet) -> Cbor {
    list([
        list(value.context.iter().map(|value| id(value.id()))),
        list(value.routes.iter().map(|value| {
            list([
                id(value.delivery),
                list(value.runs.iter().map(|value| id(value.id()))),
            ])
        })),
        list(value.produced.iter().map(|value| {
            list([
                id(value.reference.id()),
                id(value.run.id()),
                Cbor::Uint(value.iteration),
                match &value.value {
                    lp::CommandValue::Next(value) => list([Cbor::Uint(1), envelope(value)]),
                    lp::CommandValue::Again(value) => list([Cbor::Uint(2), id(*value)]),
                    lp::CommandValue::Done { cause, kind } => {
                        list([Cbor::Uint(3), id(*cause), Cbor::Uint(u64::from(kind.tag()))])
                    }
                },
            ])
        })),
        list(value.consumed.iter().map(|value| id(value.id()))),
        list(value.facts.iter().map(loop_fact)),
    ])
}
fn loop_fact(value: &lp::Fact) -> Cbor {
    use lp::Fact;
    match value {
        Fact::Open {
            run,
            instance,
            opening,
            entry,
            fuel,
            state,
        } => list([
            Cbor::Uint(1),
            id(run.id()),
            instance.tree(),
            match opening {
                lp::OpeningAdmissionRef::Ingress {
                    inbound: ingress,
                    port,
                } => list([Cbor::Uint(1), id(*ingress), word(port)]),
                lp::OpeningAdmissionRef::Delivery(value) => list([Cbor::Uint(2), id(*value)]),
            },
            word(entry),
            Cbor::Uint(*fuel),
            envelope(state),
        ]),
        Fact::Advance {
            run,
            from_iteration,
            to_iteration,
            remaining,
            command,
            state,
            due_at,
        } => list([
            Cbor::Uint(2),
            id(run.id()),
            Cbor::Uint(*from_iteration),
            Cbor::Uint(*to_iteration),
            Cbor::Uint(*remaining),
            id(command.id()),
            envelope(state),
            optional(due_at.as_ref(), |value| integer(*value)),
        ]),
        Fact::Admit {
            run,
            iteration,
            admission,
            envelope: value,
        } => list([
            Cbor::Uint(3),
            id(run.id()),
            Cbor::Uint(*iteration),
            id(*admission),
            envelope(value),
        ]),
        Fact::Close {
            run,
            iteration,
            reason,
        } => list([
            Cbor::Uint(4),
            id(run.id()),
            Cbor::Uint(*iteration),
            Cbor::Uint(match reason {
                lp::CloseReason::Natural => 1,
                lp::CloseReason::ExplicitDone => 2,
                lp::CloseReason::Stopped => 3,
            }),
        ]),
        Fact::Expired {
            run,
            iteration,
            command,
            last_state,
            output,
        } => list([
            Cbor::Uint(5),
            id(run.id()),
            Cbor::Uint(*iteration),
            id(command.id()),
            envelope(last_state),
            envelope(output),
        ]),
    }
}

/// Prefix-aware carrier reader. Parents and command producers must precede
/// their references. Discard a reader after an error; decoded records still
/// require independent journal and plan validation before use.
#[derive(Debug, Default)]
pub struct Decoder {
    envelopes: BTreeMap<Id, Envelope>,
    commands: BTreeMap<Id, lp::CommandId>,
}
impl Decoder {
    fn envelope(&mut self, value: &Cbor) -> Result<Envelope, Error> {
        let [identity, parent, origin, role, version, app, ext, data] = array(value)?;
        let identity = read_id(identity)?;
        let origin = read_id(origin)?;
        let version = uint(version)?;
        let sys = match read_optional(parent, read_id)? {
            Some(parent) => MetaSys::emission(
                identity,
                self.envelopes
                    .get(&parent)
                    .ok_or(Error::Invalid("envelope parent is not in retained prefix"))?
                    .meta()
                    .sys(),
                version,
            ),
            None => MetaSys::root(identity, version),
        }
        .with_role(match text(role)? {
            "default" => Role::Default,
            "compensate" => Role::Compensate,
            _ => return Err(Error::Invalid("envelope role")),
        });
        if sys.origin_id() != origin {
            return Err(Error::Invalid("envelope origin"));
        }
        let decoded = Envelope::new(
            Meta::new(sys)
                .with_app(read_namespace(app)?)
                .with_ext(read_namespace(ext)?),
            read_data(data)?,
        );
        if self
            .envelopes
            .get(&identity)
            .is_some_and(|previous| envelope(previous) != envelope(&decoded))
        {
            return Err(Error::Invalid("inconsistent repeated envelope"));
        }
        self.envelopes.insert(identity, decoded.clone());
        Ok(decoded)
    }
    fn command(&self, value: &Cbor) -> Result<lp::CommandId, Error> {
        self.commands
            .get(&read_id(value)?)
            .copied()
            .ok_or(Error::Invalid("command has no retained producer"))
    }
    /// Decode one row using only its already retained prefix.
    ///
    /// # Errors
    /// Unknown/malformed rows, missing parents/producers, contradictory repeated
    /// envelopes or invalid typed values. No physical Driver is reachable.
    pub fn decode(&mut self, value: &Cbor) -> Result<Record, Error> {
        let record = match items(value)?.first().map(uint).transpose()? {
            Some(1) => self.inbound(value),
            Some(2) => self.transaction(value),
            Some(3) => self.call(value),
            Some(4) => self.session(value),
            _ => Err(Error::Invalid("record kind")),
        }?;
        if let Record::Transaction(record) = &record {
            journal::retry_protocol::validate_transaction(record)
                .map_err(|_| Error::Invalid("Retry transaction carrier"))?;
        }
        Ok(record)
    }
    fn inbound(&mut self, value: &Cbor) -> Result<Record, Error> {
        let [
            _,
            provenance,
            replica,
            port,
            connector,
            seq,
            envelope,
            routes,
            exchange,
            schedule,
            instance_commands,
        ] = array(value)?;
        if uint(replica)? != 0 {
            return Err(Error::Invalid(
                "recording requires the single-process replica",
            ));
        }
        let envelope = self.envelope(envelope)?;
        let exchange = read_optional(exchange, |value| {
            Ok(Box::new(match items(value)? {
                [Cbor::Uint(1), reference, owner, timers] => {
                    journal::InboundExchangeRecord::Call(journal::InboundCallIngressRecord {
                        reference: InboundCallId::from_root(read_id(reference)?),
                        instance: instance(owner)?,
                        deadline_ids: read_deadline_ids(timers)?,
                    })
                }
                [Cbor::Uint(2), reference] => journal::InboundExchangeRecord::Session(
                    journal::SessionIngressRecord::Open(SessionId::new(read_id(reference)?)),
                ),
                [Cbor::Uint(3), reference] => journal::InboundExchangeRecord::Session(
                    journal::SessionIngressRecord::Message(SessionId::new(read_id(reference)?)),
                ),
                _ => return Err(Error::Invalid("inbound exchange")),
            }))
        })?;
        Ok(Record::Ingress(Box::new(journal::IngressRecord {
            provenance: IngressEnvelopeProvenance::single(read_path(provenance)?),
            port: read_port(port)?,
            connector_path: read_optional(connector, read_path)?,
            seq: uint(seq)?,
            envelope,
            exchange,
            deadlines: read_schedule(schedule)?,
            instance_commands: read_optional(instance_commands, |value| {
                journal::instance_stop::Packet::from_tree(value)
                    .map(Box::new)
                    .map_err(|_| Error::Invalid("instance-stop packet"))
            })?,
            routes: items(routes)?
                .iter()
                .map(|value| {
                    let [edge, target] = array(value)?;
                    Ok(journal::RouteTarget {
                        edge: text(edge)?.to_owned(),
                        target: instance(target)?,
                    })
                })
                .collect::<Result<_, Error>>()?,
        })))
    }
    fn transaction(&mut self, value: &Cbor) -> Result<Record, Error> {
        let [
            _,
            owner,
            instance_ref,
            parent,
            close,
            trigger,
            cause,
            exit,
            packet,
            at,
            consumed,
            joined,
            delta,
            emitted,
            routed,
            timers,
            schedule,
            instance_commands,
            retry,
        ] = array(value)?;
        let emissions = items(emitted)?
            .iter()
            .map(|value| {
                let [port, index, envelope] = array(value)?;
                Ok(journal::Emission {
                    port: read_port(port)?,
                    index: u32::try_from(uint(index)?)
                        .map_err(|_| Error::Invalid("emission index"))?,
                    envelope: self.envelope(envelope)?,
                })
            })
            .collect::<Result<_, Error>>()?;
        let trigger = text(trigger)?;
        let trigger = if let Some(port) = trigger.strip_prefix("p:") {
            Trigger::Port(Port::new(port).map_err(|_| Error::Invalid("trigger port"))?)
        } else {
            Trigger::Lifecycle(match trigger {
                "l:start" => Lifecycle::Start,
                "l:stop" => Lifecycle::Stop,
                "l:closing" => Lifecycle::Closing,
                "l:reply" => Lifecycle::Reply,
                "l:exit" => Lifecycle::Exit,
                "l:compensate" => Lifecycle::Compensate,
                "l:next" => Lifecycle::Next,
                "l:timeout" => Lifecycle::Timeout,
                "l:evict" => Lifecycle::Evict,
                "l:recover" => Lifecycle::Recover,
                "l:retry" => Lifecycle::Retry,
                _ => return Err(Error::Invalid("transaction trigger")),
            })
        };
        Ok(Record::Transaction(Box::new(journal::TransactionRecord {
            retry_record: read_optional(retry, |value| self.retry_packet(value).map(Box::new))?,
            owner_path: read_owner(owner)?,
            instance: instance(instance_ref)?,
            parent_instance: read_optional(parent, instance)?,
            close_scope: read_optional(close, instance)?,
            trigger,
            cause: read_optional(cause, read_id)?,
            exchange_exit: read_optional(exit, |value| {
                let [tag, reference] = array(value)?;
                Ok(Box::new(match uint(tag)? {
                    1 => journal::InboundExchangeExitRecord::Call(InboundCallId::from_root(
                        read_id(reference)?,
                    )),
                    2 => journal::InboundExchangeExitRecord::Session(SessionId::new(read_id(
                        reference,
                    )?)),
                    _ => return Err(Error::Invalid("inbound exit kind")),
                }))
            })?,
            loop_record: read_optional(packet, |value| self.loop_packet(value).map(Box::new))?,
            logical_time: read_optional(at, signed)?,
            consumed: items(consumed)?
                .iter()
                .map(|value| {
                    let [edge, source, seq] = array(value)?;
                    Ok(journal::ConsumedPosition {
                        edge: text(edge)?.to_owned(),
                        source: instance(source)?,
                        seq: uint(seq)?,
                    })
                })
                .collect::<Result<_, Error>>()?,
            join_inputs: items(joined)?
                .iter()
                .map(read_id)
                .collect::<Result<_, _>>()?,
            state_delta: items(delta)?
                .iter()
                .map(|value| {
                    let [name, value] = array(value)?;
                    Ok((
                        text(name)?.to_owned(),
                        canonical::decode_value(value).map_err(Error::Read)?,
                    ))
                })
                .collect::<Result<_, Error>>()?,
            emissions,
            routes: items(routed)?
                .iter()
                .map(|value| {
                    let [delivery, edge, target] = array(value)?;
                    Ok(journal::RoutedDelivery {
                        delivery: read_id(delivery)?,
                        edge: text(edge)?.to_owned(),
                        target: instance(target)?,
                    })
                })
                .collect::<Result<_, Error>>()?,
            deadline_ids: read_deadline_ids(timers)?,
            deadlines: read_schedule(schedule)?,
            instance_commands: read_optional(instance_commands, |value| {
                journal::instance_stop::Packet::from_tree(value)
                    .map(Box::new)
                    .map_err(|_| Error::Invalid("instance-stop packet"))
            })?,
        })))
    }
    fn call(&mut self, value: &Cbor) -> Result<Record, Error> {
        let [_, intent, status, closed_at, schedule, retry] = array(value)?;
        let status = match items(status)? {
            [Cbor::Uint(1)] => journal::CallStatus::Open,
            [Cbor::Uint(2), completion] => journal::CallStatus::Closed(journal::Closure::Response(
                read_completion(completion)?,
            )),
            [Cbor::Uint(3)] => journal::CallStatus::Closed(journal::Closure::Timeout),
            [Cbor::Uint(4)] => journal::CallStatus::Abandoned,
            _ => return Err(Error::Invalid("Call status")),
        };
        let record = journal::CallRecord {
            intent: read_intent(intent)?,
            status,
            closed_at: read_optional(closed_at, signed)?,
            deadlines: read_schedule(schedule)?,
            retry_record: read_optional(retry, |value| self.retry_packet(value).map(Box::new))?,
        };
        journal::timing::validate_call(&record).map_err(Error::Read)?;
        Ok(Record::Egress(journal::EgressRecord::Call(Box::new(
            record,
        ))))
    }
    fn session(&mut self, value: &Cbor) -> Result<Record, Error> {
        let [_, path, reference, status] = array(value)?;
        let connector_path = read_path(path)?;
        let session_id = SessionId::new(read_id(reference)?);
        let status = match items(status)? {
            [Cbor::Uint(1), emitting, prepared] => {
                journal::SessionStatus::Opening(SessionOpenIntent {
                    session_id,
                    connector_path: connector_path.clone(),
                    emitting_id: read_id(emitting)?,
                    prepared: PreparedRequest::seal(bytes(prepared)?.to_vec()),
                })
            }
            [Cbor::Uint(2)] => journal::SessionStatus::Opened,
            [Cbor::Uint(3), reference, emitting, prepared] => {
                journal::SessionStatus::WriteOpen(SessionWriteIntent {
                    session_id,
                    connector_path: connector_path.clone(),
                    write_ref: SessionWriteId::new(read_id(reference)?),
                    emitting_id: read_id(emitting)?,
                    prepared: PreparedRequest::seal(bytes(prepared)?.to_vec()),
                })
            }
            [Cbor::Uint(4), reference] => {
                journal::SessionStatus::WriteSettled(SessionWriteId::new(read_id(reference)?))
            }
            [Cbor::Uint(5), reference] => {
                journal::SessionStatus::WriteAbandoned(SessionWriteId::new(read_id(reference)?))
            }
            _ => return Err(Error::Invalid("Session status")),
        };
        Ok(Record::Egress(journal::EgressRecord::Session(Box::new(
            journal::SessionRecord {
                connector_path,
                session_id,
                status,
            },
        ))))
    }
    fn loop_packet(&mut self, value: &Cbor) -> Result<lp::Packet, Error> {
        let [context, routes, produced, consumed, facts] = array(value)?;
        let produced = items(produced)?
            .iter()
            .map(|value| {
                let [reference, run, iteration, value] = array(value)?;
                let value = match items(value)? {
                    [Cbor::Uint(1), value] => lp::CommandValue::Next(self.envelope(value)?),
                    [Cbor::Uint(2), cause] => lp::CommandValue::Again(read_id(cause)?),
                    [Cbor::Uint(3), cause, kind] => lp::CommandValue::Done {
                        cause: read_id(cause)?,
                        kind: match uint(kind)? {
                            1 => lp::DoneKind::Natural,
                            2 => lp::DoneKind::Explicit,
                            _ => return Err(Error::Invalid("Done provenance")),
                        },
                    },
                    _ => return Err(Error::Invalid("Loop command")),
                };
                let command = lp::CommandProduced::new(
                    journal::LoopRunId::new(read_id(run)?),
                    uint(iteration)?,
                    value,
                );
                if command.reference.id() != read_id(reference)? {
                    return Err(Error::Invalid("command identity"));
                }
                self.commands
                    .insert(command.reference.id(), command.reference);
                Ok(command)
            })
            .collect::<Result<_, Error>>()?;
        Ok(lp::Packet {
            context: read_runs(context)?,
            routes: items(routes)?
                .iter()
                .map(|value| {
                    let [delivery, runs] = array(value)?;
                    Ok(lp::RoutedContext {
                        delivery: read_id(delivery)?,
                        runs: read_runs(runs)?,
                    })
                })
                .collect::<Result<_, Error>>()?,
            produced,
            consumed: items(consumed)?
                .iter()
                .map(|value| self.command(value))
                .collect::<Result<_, _>>()?,
            facts: items(facts)?
                .iter()
                .map(|value| self.loop_fact(value))
                .collect::<Result<_, _>>()?,
        })
    }
    fn loop_fact(&mut self, value: &Cbor) -> Result<lp::Fact, Error> {
        use lp::Fact;
        let run = |value: &Cbor| read_id(value).map(journal::LoopRunId::new);
        Ok(match items(value)? {
            [
                Cbor::Uint(1),
                reference,
                instance_ref,
                opening,
                entry,
                fuel,
                state,
            ] => Fact::Open {
                run: run(reference)?,
                instance: instance(instance_ref)?,
                opening: match items(opening)? {
                    [Cbor::Uint(1), inbound, port] => lp::OpeningAdmissionRef::Ingress {
                        inbound: read_id(inbound)?,
                        port: read_port(port)?,
                    },
                    [Cbor::Uint(2), delivery] => {
                        lp::OpeningAdmissionRef::Delivery(read_id(delivery)?)
                    }
                    _ => return Err(Error::Invalid("Loop opening")),
                },
                entry: read_port(entry)?,
                fuel: uint(fuel)?,
                state: self.envelope(state)?,
            },
            [
                Cbor::Uint(2),
                reference,
                from,
                to,
                remaining,
                command,
                state,
                due,
            ] => Fact::Advance {
                run: run(reference)?,
                from_iteration: uint(from)?,
                to_iteration: uint(to)?,
                remaining: uint(remaining)?,
                command: self.command(command)?,
                state: self.envelope(state)?,
                due_at: read_optional(due, signed)?,
            },
            [Cbor::Uint(3), reference, iteration, admission, envelope] => Fact::Admit {
                run: run(reference)?,
                iteration: uint(iteration)?,
                admission: read_id(admission)?,
                envelope: self.envelope(envelope)?,
            },
            [Cbor::Uint(4), reference, iteration, reason] => Fact::Close {
                run: run(reference)?,
                iteration: uint(iteration)?,
                reason: match uint(reason)? {
                    1 => lp::CloseReason::Natural,
                    2 => lp::CloseReason::ExplicitDone,
                    3 => lp::CloseReason::Stopped,
                    _ => return Err(Error::Invalid("Loop close reason")),
                },
            },
            [Cbor::Uint(5), reference, iteration, command, last, output] => Fact::Expired {
                run: run(reference)?,
                iteration: uint(iteration)?,
                command: self.command(command)?,
                last_state: self.envelope(last)?,
                output: self.envelope(output)?,
            },
            _ => return Err(Error::Invalid("Loop fact")),
        })
    }
}
fn read_runs(value: &Cbor) -> Result<Vec<journal::LoopRunId>, Error> {
    items(value)?
        .iter()
        .map(|value| read_id(value).map(journal::LoopRunId::new))
        .collect()
}
