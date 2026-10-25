//! Read-only projections of the live execution, independent of presentation.
//!
//! These views expose instance phases, mailbox contents and open exchanges
//! without advancing work or changing the journal. Owned snapshots remain
//! valid after the next execution step; they are not recovery authority.
//! Port and route names are resolved from the validated plan here. Formatting,
//! redaction and debugger navigation belong to the hosted presentation layer.
//!
//! This is a child of `execution` so projection can read private state without
//! making the scheduler's fields a public mutation interface.

use super::Execution;
use crate::connector::exchange::CallId;
use crate::connector::exchange::InboundCallId;
use crate::connector::exchange::SessionId;
use crate::envelope::Envelope;
use crate::{instance, node};
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// An envelope seen at a port, leaving its producer or arriving at a target.
/// This is the debugger's live log, not another journal record. Readers and
/// Lab expectations inspect windows of it without consuming any delivery.
#[derive(Debug, Clone, PartialEq)]
pub struct EnvelopeView {
    /// The observed node, or `None` at the App boundary.
    pub node: Option<node::OwnerPath>,
    /// The source or destination port where the envelope was seen.
    pub port: node::Port,
    /// The observed envelope.
    pub envelope: Envelope,
}

/// Read-only lifecycle state exposed to hosted debuggers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstancePhaseInspection {
    /// The instance accepts ordinary work.
    Open,
    /// External admission is closed while admitted work drains.
    Draining,
    /// Open effects are being settled under the closing budget.
    Settling,
    /// The terminal lifecycle transaction is waiting to commit.
    Stopping,
}

/// One live logical instance. Graphs have no private state cells.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceInspection {
    /// Canonical logical address plus journal-derived generation.
    pub reference: instance::Generation,
    /// Static lexical owner.
    pub owner: node::OwnerPath,
    /// Current replay-derivable lifecycle phase.
    pub phase: InstancePhaseInspection,
    /// Executable Node at this owner, absent for a Graph owner.
    pub node: Option<node::OwnerPath>,
    /// Current Node-private cells with defaults applied.
    pub state: BTreeMap<String, crate::value::Value>,
}

/// One live per-instance mailbox exposed to hosted debuggers.
#[derive(Debug, Clone, PartialEq)]
pub struct MailboxInspection {
    /// Executable Node that owns this FIFO.
    pub node: node::OwnerPath,
    /// Exact logical instance generation owning the FIFO.
    pub instance: instance::Generation,
    /// Whether execution has reserved the mailbox head for an open protocol.
    pub claimed: bool,
    /// Current FIFO contents in delivery order.
    pub deliveries: Vec<DeliveryInspection>,
}

/// A queued delivery projected for a reader outside the execution. Route
/// indices are resolved here so a snapshot never depends on a live table.
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryInspection {
    /// Arrival port, resolved from the delivery route.
    pub port: node::Port,
    /// Canonical cursor path, not the execution-local route index.
    pub edge: String,
    /// Position in the source instance's stream on this route.
    pub seq: u64,
    /// Exact upstream instance generation.
    pub source_instance: instance::Generation,
    /// Queued envelope.
    pub envelope: Envelope,
}

/// Typed identity of one retained Call obligation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CallReferenceInspection {
    /// Graph-to-world Call identity.
    Outbound(CallId),
    /// World-to-graph Call identity.
    Inbound(InboundCallId),
}

/// One Call held by the Execution at the inspection boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallInspection {
    /// Typed Call identity.
    pub reference: CallReferenceInspection,
    /// Connector owning the protocol.
    pub connector: node::OwnerPath,
    /// Exact logical instance generation.
    pub instance: instance::Generation,
    /// Whether the terminal observation is retained but not yet consumed.
    pub completion_recorded: bool,
}

/// Direction in which one Session was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionDirectionInspection {
    /// Graph opened the Session toward the world.
    Outbound,
    /// World opened the Session toward the graph.
    Inbound,
}

/// One long-lived Session held by the Execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInspection {
    /// Logical Session identity.
    pub reference: SessionId,
    /// Direction of the opening.
    pub direction: SessionDirectionInspection,
    /// Connector owning the protocol.
    pub connector: node::OwnerPath,
    /// Exact logical instance generation.
    pub instance: instance::Generation,
    /// Whether the opening handshake completed.
    pub opened: bool,
    /// Journaled writes not yet locally settled.
    pub pending_writes: usize,
    /// Whether a terminal observation is retained but not yet consumed.
    pub completion_recorded: bool,
}

/// Read-only snapshot of the current execution state.
///
/// It is deliberately a clone: a debugger can retain and format it without
/// borrowing the scheduler or becoming an execution authority.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionInspection {
    /// Capacity shared by all incoming routes of one instance mailbox.
    pub mailbox_capacity: usize,
    /// Live logical instances in canonical reference order.
    pub instances: Vec<InstanceInspection>,
    /// Current bounded mailbox contents in canonical owner/instance order.
    pub mailboxes: Vec<MailboxInspection>,
    /// Retained Call obligations in canonical identity order.
    pub calls: Vec<CallInspection>,
    /// Retained Sessions in canonical identity order.
    pub sessions: Vec<SessionInspection>,
}

impl Execution<'_> {
    /// Read-only envelope views in observation order: emissions at their
    /// source ports and deliveries at their destinations.
    #[must_use]
    pub fn envelope_views(&self) -> &[EnvelopeView] {
        &self.observed
    }

    /// Read-only operations from the accepted journal fold. Inspection borrows
    /// these states without copying pinned requests or altering scheduling.
    pub fn retry_operations(
        &self,
    ) -> impl Iterator<
        Item = (
            &crate::journal::retry_protocol::OperationId,
            &crate::journal::retry_protocol::Operation,
        ),
    > {
        self.retry_manager.iter().flat_map(|fold| fold.operations())
    }

    /// Clones one read-only debugger view without changing scheduler state.
    #[must_use]
    pub fn inspection(&self) -> ExecutionInspection {
        let instances = self
            .instances
            .iter()
            .map(|(reference, instance)| {
                let owner = &self.plan.nodes[instance.owner.index()];
                let (node, state) = match owner.kind() {
                    node::Kind::Graph => (None, BTreeMap::new()),
                    node::Kind::Operator | node::Kind::Connector => (
                        Some(owner.path.clone()),
                        self.state
                            .cell_unchecked(self.plan.plan(), instance.owner, reference),
                    ),
                };
                let phase = match instance.phase {
                    instance::Phase::Open => InstancePhaseInspection::Open,
                    instance::Phase::Closing {
                        stage: instance::ClosingStage::Draining,
                        ..
                    } => InstancePhaseInspection::Draining,
                    instance::Phase::Closing {
                        stage: instance::ClosingStage::Settling,
                        ..
                    } => InstancePhaseInspection::Settling,
                    instance::Phase::Stopping { .. } => InstancePhaseInspection::Stopping,
                };
                InstanceInspection {
                    reference: reference.clone(),
                    owner: owner.path.clone(),
                    phase,
                    node,
                    state,
                }
            })
            .collect();
        let mut calls =
            Vec::with_capacity(self.pending.len().saturating_add(self.inbound_calls.len()));
        calls.extend(self.pending.iter().map(|(reference, pending)| {
            CallInspection {
                reference: CallReferenceInspection::Outbound(*reference),
                connector: self.plan.nodes[pending.slot.index()].path.clone(),
                instance: pending.instance.clone(),
                completion_recorded: self
                    .settled
                    .iter()
                    .any(|entry| entry.event.call_id == *reference),
            }
        }));
        calls.extend(self.inbound_calls.iter().map(|(reference, pending)| {
            CallInspection {
                reference: CallReferenceInspection::Inbound(*reference),
                connector: self.plan.nodes[pending.slot.index()].path.clone(),
                instance: pending.instance.clone(),
                completion_recorded: self
                    .settled_inbound_calls
                    .iter()
                    .any(|event| event.call_id == *reference),
            }
        }));
        calls.sort_by_key(|call| call.reference);
        let sessions = self
            .sessions
            .iter()
            .map(|(reference, session)| SessionInspection {
                reference: *reference,
                direction: if session.open_intent.is_some() {
                    SessionDirectionInspection::Outbound
                } else {
                    SessionDirectionInspection::Inbound
                },
                connector: self.plan.nodes[session.slot.index()].path.clone(),
                instance: session.instance.clone(),
                opened: session.opened,
                pending_writes: self
                    .session_writes
                    .values()
                    .filter(|write| write.intent.session_id == *reference)
                    .count(),
                completion_recorded: self
                    .settled_sessions
                    .iter()
                    .any(|settled| settled.session == *reference),
            })
            .collect();
        let mailboxes = self
            .scheduler
            .mailboxes
            .inspection()
            .into_iter()
            .map(|(slot, instance, claimed, deliveries)| MailboxInspection {
                node: self.plan.nodes[slot.index()].path.clone(),
                instance,
                claimed,
                deliveries: deliveries
                    .into_iter()
                    .map(|delivery| DeliveryInspection {
                        port: self.scheduler.mailboxes.routes.port(delivery.edge).clone(),
                        edge: self
                            .scheduler
                            .mailboxes
                            .routes
                            .text(delivery.edge)
                            .to_string(),
                        seq: delivery.seq,
                        source_instance: delivery.source_instance,
                        envelope: delivery.envelope,
                    })
                    .collect(),
            })
            .collect();
        ExecutionInspection {
            mailbox_capacity: self.scheduler.mailboxes.capacity,
            instances,
            mailboxes,
            calls,
            sessions,
        }
    }
}
