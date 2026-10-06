//! Exact-instance closure requests. Production and the winning `l:closing`
//! are separate facts; losing requests are settled by that same close fact.

use super::{IngressRecord, Record, TransactionRecord};
use crate::envelope::Id;
use crate::instance;
use crate::node;

use crate::node::{Lifecycle, Trigger};
use crate::plan::{Endpoint, EndpointOwner, InstanceStopTarget};
use alloc::collections::BTreeMap;
use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

/// Extend-only field in the canonical causal projection.
pub const CAUSAL_KEY: u64 = 42;

/// Authority fixed by the producer, never resolved again by the consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// Derived from every authority and cause field below.
    pub id: Id,
    /// Actual source emission (before transparent boundary composition).
    pub source: Endpoint,
    /// Exact emitting owner generation.
    pub source_instance: instance::Generation,
    /// Exact lexical wiring Graph generation.
    pub scope: instance::Generation,
    /// Exact generation to close, not a key or a current-owner lookup.
    pub target: instance::Generation,
    /// The emitted or inbound envelope which requested closure.
    pub cause: Id,
}

impl Command {
    /// Builds the command identity. Scope and permission are checked separately
    /// against the plan and the journal prefix before acceptance.
    #[must_use]
    pub fn new(
        source: Endpoint,
        source_instance: instance::Generation,
        scope: instance::Generation,
        target: instance::Generation,
        cause: Id,
    ) -> Self {
        let mut command = Self {
            id: cause,
            source,
            source_instance,
            scope,
            target,
            cause,
        };
        command.id = command.derived_id();
        command
    }

    fn derived_id(&self) -> Id {
        let kind = match self.source.owner {
            EndpointOwner::Graph(_) => 1,
            EndpointOwner::Node(_) => 2,
        };
        Id::derive(
            crate::envelope::IdKind::InstanceStop,
            &[
                &[kind],
                self.source.port.name().as_bytes(),
                self.source_instance.address().as_bytes(),
                &self.source_instance.generation().to_le_bytes(),
                self.scope.address().as_bytes(),
                &self.scope.generation().to_le_bytes(),
                self.target.address().as_bytes(),
                &self.target.generation().to_le_bytes(),
                self.cause.as_bytes(),
            ],
        )
    }
}

/// Sparse causal extension, omitted rather than stored empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    /// Commands in strictly increasing identity order.
    pub produced: Vec<Command>,
    /// Present only on the winning target-owned `l:closing` transaction.
    pub consumed: Option<Id>,
}

impl Packet {
    /// Canonical field value shared by causal projection and full recording.
    #[must_use]
    pub fn tree(&self) -> crate::canonical_cbor::Value {
        use crate::canonical_cbor::Value as Cbor;
        let mut fields = BTreeMap::new();
        if !self.produced.is_empty() {
            fields.insert(
                1,
                Cbor::Array(
                    self.produced
                        .iter()
                        .map(|command| {
                            Cbor::Array(vec![
                                id_tree(command.id),
                                Cbor::Uint(match command.source.owner {
                                    EndpointOwner::Graph(_) => 1,
                                    EndpointOwner::Node(_) => 2,
                                }),
                                Cbor::Text(command.source.port.name().to_string()),
                                command.source_instance.tree(),
                                command.scope.tree(),
                                command.target.tree(),
                                id_tree(command.cause),
                            ])
                        })
                        .collect(),
                ),
            );
        }
        if let Some(consumed) = self.consumed {
            fields.insert(2, id_tree(consumed));
        }
        Cbor::Map(fields)
    }

    /// Reads the exact field algebra. Acceptance additionally requires the
    /// prefix proof in [`Fold`] and the plan's routing/target proof.
    ///
    /// # Errors
    /// Unknown keys, wrong arities, invalid references or command identities,
    /// empty packets, mixed production/consumption, or noncanonical ordering.
    pub fn from_tree(tree: &crate::canonical_cbor::Value) -> Result<Self, Error> {
        use crate::canonical_cbor::Value as Cbor;
        let Cbor::Map(fields) = tree else {
            return Err(Error::PacketShape);
        };
        if fields.len() != 1 {
            return Err(Error::PacketShape);
        }
        if let Some(consumed) = fields.get(&2) {
            return Ok(Self {
                produced: Vec::new(),
                consumed: Some(read_id(consumed)?),
            });
        }
        let Some(Cbor::Array(commands)) = fields.get(&1) else {
            return Err(Error::PacketShape);
        };
        if commands.is_empty() {
            return Err(Error::PacketShape);
        }
        let mut produced = Vec::with_capacity(commands.len());
        for value in commands {
            let Cbor::Array(row) = value else {
                return Err(Error::PacketShape);
            };
            let [
                id,
                Cbor::Uint(kind),
                Cbor::Text(port),
                source,
                scope,
                target,
                cause,
            ] = row.as_slice()
            else {
                return Err(Error::PacketShape);
            };
            let source_instance = read_instance(source)?;
            let path = source_instance.address().owner().clone();
            let source = Endpoint {
                owner: match kind {
                    1 => EndpointOwner::Graph(path),
                    2 => EndpointOwner::Node(
                        crate::node::Path::new(path.segments()).map_err(|_| Error::Source)?,
                    ),
                    _ => return Err(Error::PacketShape),
                },
                port: crate::node::Port::new(port).map_err(|_| Error::Source)?,
            };
            let command = Command::new(
                source,
                source_instance,
                read_instance(scope)?,
                read_instance(target)?,
                read_id(cause)?,
            );
            if read_id(id)? != command.id {
                return Err(Error::CommandIdentity);
            }
            if produced
                .last()
                .is_some_and(|prior: &Command| prior.id >= command.id)
            {
                return Err(Error::PacketShape);
            }
            produced.push(command);
        }
        Ok(Self {
            produced,
            consumed: None,
        })
    }
}

fn id_tree(id: Id) -> crate::canonical_cbor::Value {
    crate::canonical_cbor::Value::Bytes(id.as_bytes().to_vec())
}

fn read_id(value: &crate::canonical_cbor::Value) -> Result<Id, Error> {
    let crate::canonical_cbor::Value::Bytes(bytes) = value else {
        return Err(Error::PacketShape);
    };
    Ok(Id::from_bytes(
        bytes
            .as_slice()
            .try_into()
            .map_err(|_| Error::PacketShape)?,
    ))
}

fn read_instance(value: &crate::canonical_cbor::Value) -> Result<instance::Generation, Error> {
    use crate::canonical_cbor::Value as Cbor;
    let Cbor::Array(row) = value else {
        return Err(Error::PacketShape);
    };
    let [Cbor::Bytes(address), Cbor::Uint(generation)] = row.as_slice() else {
        return Err(Error::PacketShape);
    };
    Ok(instance::Generation::new(
        crate::instance::Identity::decode(address).map_err(|_| Error::Target)?,
        *generation,
    ))
}

/// Result derived from the prefix. A redundant request creates no false
/// lifecycle record and does not renew the original Closing deadlines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Producer committed; the lifecycle controller has not applied it.
    Pending,
    /// This request caused the retained close transaction.
    Consumed {
        /// Zero-based position of the winning `l:closing`.
        record: usize,
    },
    /// That close fact already governs this exact instance or its ancestor.
    Superseded {
        /// Zero-based position of the already authoritative close.
        record: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Instance {
    parent: Option<instance::Generation>,
    stopped: bool,
    closing: Option<usize>,
}

/// Pure reconstruction of lifecycle links and command dispositions. No wall
/// clock, Driver, latest-generation lookup, or mutable queue is authoritative.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fold {
    instances: BTreeMap<instance::Generation, Instance>,
    commands: Vec<(Command, Disposition)>,
    records: usize,
}

/// Closed refusal algebra for production, selection and consumption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Empty, mixed production/consumption, or noncanonical field shape.
    PacketShape,
    /// Identity does not derive from the retained authority and cause.
    CommandIdentity,
    /// The same command identity was produced twice.
    DuplicateCommand,
    /// The source does not belong to its claimed record or lexical context.
    Source,
    /// No child ever opened in the exact selected parent generation.
    MissingTarget,
    /// Several child references fit; none can be chosen implicitly.
    AmbiguousTarget,
    /// The target differs from the independently selected instance.
    Target,
    /// Lifecycle facts contradict their exact parent-generation tree.
    Lifecycle,
    /// Consumption repeats or does not commit the command's own target close.
    Consumption,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::PacketShape => "instance-stop packet is empty, mixed, or non-canonical",
            Self::CommandIdentity => "instance-stop identity contradicts its authority or cause",
            Self::DuplicateCommand => "instance-stop command was already produced",
            Self::Source => "instance-stop source has no matching retained emission or scope",
            Self::MissingTarget => "instance-stop target has never opened in this instance context",
            Self::AmbiguousTarget => "instance-stop target is ambiguous in this instance context",
            Self::Target => "instance-stop target contradicts the exact parent-generation binding",
            Self::Lifecycle => "instance-stop lifecycle prefix is inconsistent",
            Self::Consumption => {
                "instance-stop consumption has no pending command and matching target-owned close"
            }
        })
    }
}

impl core::error::Error for Error {}

fn owner_path(owner: &EndpointOwner) -> node::OwnerPath {
    match owner {
        EndpointOwner::Graph(path) => path.clone(),
        EndpointOwner::Node(path) => node::OwnerPath::from(path),
    }
}

impl Fold {
    /// Reconstructs the complete supplied prefix or rejects it.
    ///
    /// # Errors
    /// The first invalid production, consumption or lifecycle fact.
    pub fn from_records(records: &[Record]) -> Result<Self, Error> {
        let mut fold = Self::default();
        for record in records {
            fold.apply_inner(record)?;
        }
        Ok(fold)
    }

    /// Applies one retained fact atomically; failure leaves the fold intact.
    ///
    /// # Errors
    /// A malformed record or a transition contradicting this exact prefix.
    pub fn apply(&mut self, record: &Record) -> Result<(), Error> {
        let mut next = self.clone();
        next.apply_inner(record)?;
        *self = next;
        Ok(())
    }

    /// Commands in producer record order, then canonical identity order.
    pub fn commands(&self) -> impl Iterator<Item = (&Command, Disposition)> {
        self.commands
            .iter()
            .map(|(command, state)| (command, *state))
    }

    /// The first retained close governing this exact generation, if any.
    #[must_use]
    pub fn closing_record(&self, instance: &instance::Generation) -> Option<usize> {
        self.instances.get(instance).and_then(|state| state.closing)
    }

    /// Resolves only within an exact parent-generation chain. An already
    /// closed unique child remains a valid redundant target. Multiple old
    /// generations never imply permission to choose the newest one.
    ///
    /// # Errors
    /// Missing source context, nonlocal target, absent child or ambiguous child.
    pub fn resolve(
        &self,
        source: &instance::Generation,
        target: &InstanceStopTarget,
    ) -> Result<(instance::Generation, instance::Generation), Error> {
        let mut cursor = source;
        let scope = loop {
            let instance = self.instances.get(cursor).ok_or(Error::Source)?;
            if cursor.address().owner() == &target.scope {
                break cursor;
            }
            cursor = instance.parent.as_ref().ok_or(Error::Source)?;
        };
        let target_owner = owner_path(&target.owner);
        if target_owner == target.scope {
            return Ok((scope.clone(), scope.clone()));
        }
        if target_owner.parent().as_ref() != Some(&target.scope) {
            return Err(Error::Target);
        }
        // A producing child (or its descendant) supplies its own exact
        // instance, even when the parent owns other keys at the same path.
        let mut cursor = source;
        while cursor != scope {
            if cursor.address().owner() == &target_owner {
                return Ok((scope.clone(), cursor.clone()));
            }
            cursor = self
                .instances
                .get(cursor)
                .and_then(|instance| instance.parent.as_ref())
                .ok_or(Error::Source)?;
        }
        let children: Vec<_> = self
            .instances
            .iter()
            .filter(|(reference, instance)| {
                reference.address().owner() == &target_owner
                    && instance.parent.as_ref() == Some(scope)
            })
            .collect();
        let live: Vec<_> = children
            .iter()
            .filter(|(_, instance)| !instance.stopped)
            .collect();
        let selected = match live.as_slice() {
            [(reference, _)] => *reference,
            [] => match children.as_slice() {
                [(reference, _)] => *reference,
                [] => return Err(Error::MissingTarget),
                _ => return Err(Error::AmbiguousTarget),
            },
            _ => return Err(Error::AmbiguousTarget),
        };
        Ok((scope.clone(), selected.clone()))
    }

    fn apply_inner(&mut self, record: &Record) -> Result<(), Error> {
        match record {
            Record::Ingress(inbound) => {
                if let Some(packet) = inbound.instance_commands.as_deref() {
                    if packet.consumed.is_some() {
                        return Err(Error::Consumption);
                    }
                    self.produce(packet, |command| inbound_source(inbound, command))?;
                }
            }
            Record::Transaction(transaction) => {
                if transaction.trigger == Trigger::Lifecycle(Lifecycle::Start) {
                    if self.instances.contains_key(&transaction.instance)
                        || transaction.parent_instance.as_ref().is_some_and(|parent| {
                            self.instances
                                .get(parent)
                                .is_none_or(|instance| instance.stopped)
                        })
                    {
                        return Err(Error::Lifecycle);
                    }
                    let closing = transaction
                        .parent_instance
                        .as_ref()
                        .and_then(|parent| self.instances.get(parent))
                        .and_then(|instance| instance.closing);
                    self.instances.insert(
                        transaction.instance.clone(),
                        Instance {
                            parent: transaction.parent_instance.clone(),
                            stopped: false,
                            closing,
                        },
                    );
                }
                if let Some(packet) = transaction.instance_commands.as_deref() {
                    if let Some(id) = packet.consumed {
                        self.consume(transaction, packet, id)?;
                    } else {
                        self.produce(packet, |command| transaction_source(transaction, command))?;
                    }
                }
                if let Some(scope) = transaction.close_scope.as_ref() {
                    if scope != &transaction.instance || !self.instances.contains_key(scope) {
                        return Err(Error::Lifecycle);
                    }
                    let affected: Vec<_> = self
                        .instances
                        .keys()
                        .filter(|reference| self.within(reference, scope))
                        .cloned()
                        .collect();
                    for reference in affected {
                        if let Some(instance) = self.instances.get_mut(&reference) {
                            instance.closing.get_or_insert(self.records);
                        }
                    }
                    for (command, state) in &mut self.commands {
                        if *state == Disposition::Pending
                            && let Some(record) = self
                                .instances
                                .get(&command.target)
                                .and_then(|instance| instance.closing)
                        {
                            *state = Disposition::Superseded { record };
                        }
                    }
                }
                if transaction.trigger == Trigger::Lifecycle(Lifecycle::Stop) {
                    let instance = self
                        .instances
                        .get_mut(&transaction.instance)
                        .ok_or(Error::Lifecycle)?;
                    if instance.stopped || instance.closing.is_none() {
                        return Err(Error::Lifecycle);
                    }
                    instance.stopped = true;
                }
            }
            Record::Egress(_) => {}
        }
        self.records += 1;
        Ok(())
    }

    fn within(&self, child: &instance::Generation, ancestor: &instance::Generation) -> bool {
        let mut cursor = Some(child);
        while let Some(reference) = cursor {
            if reference == ancestor {
                return true;
            }
            cursor = self
                .instances
                .get(reference)
                .and_then(|instance| instance.parent.as_ref());
        }
        false
    }

    fn produce(&mut self, packet: &Packet, source: impl Fn(&Command) -> bool) -> Result<(), Error> {
        if packet.produced.is_empty()
            || packet.consumed.is_some()
            || packet
                .produced
                .windows(2)
                .any(|pair| pair[0].id >= pair[1].id)
        {
            return Err(Error::PacketShape);
        }
        for command in &packet.produced {
            if owner_path(&command.source.owner) != *command.source_instance.address().owner()
                || command.derived_id() != command.id
            {
                return Err(Error::CommandIdentity);
            }
            if !source(command)
                || self
                    .instances
                    .get(&command.source_instance)
                    .is_none_or(|instance| instance.stopped)
            {
                return Err(Error::Source);
            }
            if self
                .commands
                .iter()
                .any(|(prior, _)| prior.id == command.id)
            {
                return Err(Error::DuplicateCommand);
            }
            let target = InstanceStopTarget {
                scope: command.scope.address().owner().clone(),
                owner: EndpointOwner::Graph(command.target.address().owner().clone()),
            };
            let (scope, target) = self.resolve(&command.source_instance, &target)?;
            if scope != command.scope || target != command.target {
                return Err(Error::Target);
            }
            let state = self.instances.get(&target).ok_or(Error::MissingTarget)?;
            let disposition =
                state
                    .closing
                    .map_or(Disposition::Pending, |record| Disposition::Superseded {
                        record,
                    });
            self.commands.push((command.clone(), disposition));
        }
        Ok(())
    }

    fn consume(
        &mut self,
        record: &TransactionRecord,
        packet: &Packet,
        id: Id,
    ) -> Result<(), Error> {
        if !packet.produced.is_empty()
            || record.trigger != Trigger::Lifecycle(Lifecycle::Closing)
            || record.close_scope.as_ref() != Some(&record.instance)
            || !record.consumed.is_empty()
            || !record.emissions.is_empty()
            || !record.state_delta.is_empty()
            || !record.routes.is_empty()
        {
            return Err(Error::Consumption);
        }
        if self
            .commands
            .iter()
            .find(|(_, state)| *state == Disposition::Pending)
            .is_none_or(|(command, _)| command.id != id)
        {
            return Err(Error::Consumption);
        }
        let (command, state) = self
            .commands
            .iter_mut()
            .find(|(command, _)| command.id == id)
            .ok_or(Error::Consumption)?;
        if *state != Disposition::Pending
            || command.target != record.instance
            || record.cause != Some(command.cause)
        {
            return Err(Error::Consumption);
        }
        *state = Disposition::Consumed {
            record: self.records,
        };
        Ok(())
    }
}

fn transaction_source(record: &TransactionRecord, command: &Command) -> bool {
    command.source_instance == record.instance
        && record.emissions.iter().any(|emission| {
            emission.port == command.source.port
                && emission.envelope.meta().sys().id() == command.cause
        })
}

fn inbound_source(record: &IngressRecord, command: &Command) -> bool {
    let owner = record.connector_path.as_ref().map_or_else(
        || EndpointOwner::Graph(node::OwnerPath::root()),
        |path| EndpointOwner::Node(path.clone()),
    );
    command.source.owner == owner
        && command.source.port == record.port
        && command.cause == record.envelope.meta().sys().id()
}

#[cfg(test)]
mod tests;
