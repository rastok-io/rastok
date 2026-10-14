//! Bind Retry facts to the original program and exact prefix deliveries.
//! The protocol validates policy transitions; this fold independently proves
//! owner, cursor, deadline, spatial disposition and enclosing Loop authority.

use super::*;
use crate::journal::deadline::Deadline;
use crate::journal::retry_protocol::{self as retry, Fact};

impl Execution<'_> {
    pub(super) fn restore_retry_manager(
        &mut self,
        index: usize,
        record: &TransactionRecord,
    ) -> Result<(), ExecutionFault> {
        let instance = self
            .instances
            .get(&record.instance)
            .ok_or_else(|| invalid(index, "a Retry manager has no admitted owner"))?;
        let owner = &self.plan.nodes[instance.owner.index()];
        let slot = instance.owner;
        let spec = owner
            .retry_spec()
            .ok_or_else(|| invalid(index, "the original owner has no Retry"))?;
        let connector = self.plan.nodes[slot.index()]
            .connector()
            .ok_or_else(|| invalid(index, "the Retry owner is not a Connector"))?;
        let call = connector
            .call_semantics()
            .ok_or_else(|| invalid(index, "Retry requires a finite Call"))?;
        if owner.path != record.owner_path
            || self.lifecycle.iter().any(|work| {
                work.instance == record.instance && work.lifecycle == node::Lifecycle::Start
            })
        {
            return Err(invalid(
                index,
                "a Retry manager precedes its exact owner's start",
            ));
        }
        let hash = self.original_hash.ok_or_else(|| {
            invalid(
                index,
                "Retry recovery requires the original semantic identity",
            )
        })?;
        let packet = record
            .retry_record
            .as_deref()
            .ok_or_else(|| invalid(index, "missing Retry packet"))?;
        let fact = packet
            .facts
            .first()
            .ok_or_else(|| invalid(index, "empty Retry packet"))?;
        let mut next = self
            .retry_manager
            .clone()
            .ok_or_else(|| invalid(index, "missing Retry policy fold"))?;
        let mut routed = None;
        let (context, cause, opening_delivery) = if let Fact::Opened { opening, pin, .. } = fact {
            let delivery = self
                .scheduler
                .mailboxes
                .queued(slot, &record.instance)
                .into_iter()
                .flatten()
                .find(|delivery| {
                    ids::delivery_id(&delivery.edge, delivery.envelope.meta().sys().id())
                        == *opening
                })
                .ok_or_else(|| invalid(index, "Retry opening has no exact queued admission"))?
                .clone();
            if delivery.port != spec.entry || pin.opened_at != self.now {
                return Err(invalid(
                    index,
                    "Retry opening changed its entry or preparation instant",
                ));
            }
            let settings = self.resolve_invocation_settings(
                &record.instance,
                connector,
                self.executor_path(slot)?,
                &delivery.envelope,
            )?;
            if pin.settings != settings {
                return Err(invalid(
                    index,
                    "Retry preparation changed its original invocation settings",
                ));
            }
            (
                self.graph_context(&delivery).to_vec(),
                delivery.envelope.meta().sys().id(),
                Some(delivery),
            )
        } else {
            let state = next
                .operation(fact.operation())
                .filter(|state| {
                    state.instance == record.instance
                        && state.plan == hash
                        && state.contract == connector.contract
                })
                .ok_or_else(|| {
                    invalid(
                        index,
                        "Retry fact crosses its original owner, plan or Contract",
                    )
                })?;
            if let [Fact::Admitted { envelope, .. }] = packet.facts.as_slice()
                && state.ordinal > 0
            {
                if !self.scheduler.mailboxes.has_room(slot, &state.instance, 0) {
                    return Err(invalid(
                        index,
                        "Retry admission exceeds the shared mailbox capacity",
                    ));
                }
                routed = Some((
                    retry::admission_edge(fact.operation(), &state.entry),
                    envelope.clone(),
                ));
            }
            (
                self.graph_context_for(state.opening).to_vec(),
                state
                    .terminal_parent()
                    .unwrap_or(state.envelope.meta().sys().id()),
                None,
            )
        };
        let policy = super::super::retry_manager::policy(connector, spec)?;
        let authority = opening_delivery.as_ref().map(|delivery| retry::Opening {
            instance: &record.instance,
            delivery: ids::delivery_id(&delivery.edge, delivery.envelope.meta().sys().id()),
            plan: hash,
            contract: &connector.contract,
            call,
            policy: &policy,
            envelope: &delivery.envelope,
        });
        next.apply(packet, authority.as_ref(), self.now)
            .map_err(super::super::retry_manager::error)?;
        let mut expected_routes = Vec::new();
        let mut expected_deadlines = Vec::new();
        let mut expected_loop = (!context.is_empty()).then(|| {
            Box::new(protocol::LoopRecord {
                context: context.clone(),
                ..Default::default()
            })
        });
        if let Some((edge, envelope)) = &routed {
            let reference = ids::delivery_id(edge, envelope.meta().sys().id());
            expected_routes.push(RoutedDelivery {
                delivery: envelope.meta().sys().id(),
                edge: edge.clone(),
                target: record.instance.clone(),
            });
            let Some(AttrSetting::Value(crate::value::Value::Duration(timeout))) =
                connector.attribute("timeout")
            else {
                return Err(invalid(
                    index,
                    "Retry admission has no bounded Call timeout",
                ));
            };
            expected_deadlines.push(Deadline {
                kind: crate::journal::DeadlineKind::CallTimeout,
                reference,
                due_at: self.now.saturating_add(*timeout),
            });
            if let Some(packet) = expected_loop.as_deref_mut() {
                packet.routes.push(protocol::RoutedContext {
                    delivery: reference,
                    runs: context.clone(),
                });
            }
        }
        if record.cause != Some(cause)
            || record.routes != expected_routes
            || record.loop_record != expected_loop
            || record.deadline_ids
                != expected_deadlines
                    .iter()
                    .map(|deadline| (deadline.kind, deadline.reference))
                    .collect::<Vec<_>>()
        {
            return Err(invalid(
                index,
                "Retry manager changed its cause, routes, contexts or deadline presence",
            ));
        }
        self.check_recovered_delivery_deadlines(
            index,
            record.deadlines.as_deref(),
            &expected_deadlines,
        )?;
        if let Some((edge, envelope)) = routed {
            self.scheduler
                .mailboxes
                .deliver(
                    slot,
                    &record.instance,
                    &record.instance,
                    spec.entry.clone(),
                    edge.clone(),
                    envelope.clone(),
                )
                .map_err(|_| invalid(index, "Retry admission exceeded its reserved mailbox"))?;
            for deadline in expected_deadlines {
                self.scheduler.schedule(
                    deadline.due_at,
                    Obligation::DeliveryBudget {
                        connector: self.executor_path(slot)?.clone(),
                        edge: edge.clone(),
                        delivery: envelope.meta().sys().id(),
                        instance: record.instance.clone(),
                    },
                );
            }
            if let Some(packet) = expected_loop {
                self.update_graph_loop_contexts(packet.routes);
            }
        }
        self.retry_manager = Some(next);
        Ok(())
    }
}
