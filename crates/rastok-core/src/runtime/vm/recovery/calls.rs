//! Rebuild the held request and the close-once outcome separately. An open
//! record never grants permission to start that old effect again.

use super::*;
use crate::journal::{CallRecord, Closure, ConsumedPosition, OutboundStatus};

impl Vm<'_> {
    pub(super) fn restore_call(
        &mut self,
        journal: &dyn Journal,
        bindings: &mut dyn OutboundConnectorBindings,
        index: usize,
        record: &CallRecord,
    ) -> Result<(), VmError> {
        let intent = &record.intent;
        let slot = self
            .plan
            .nodes
            .iter()
            .position(|node| node.path == intent.connector_path)
            .map(|index| Slot::new(index as u32))
            .ok_or_else(|| invalid(index, "a retained Call has no planned Connector"))?;
        let spec = self.plan.nodes[slot.index()]
            .connector()
            .filter(|spec| spec.call_semantics().is_some())
            .ok_or_else(|| invalid(index, "a retained Call contradicts its Connector profile"))?;
        if intent.call_id
            != ids::call_id(
                ids::delivery_id(&intent.edge, intent.emitting_id),
                intent.prepared.digest(),
            )
        {
            return Err(invalid(
                index,
                "a retained Call contradicts its request identity",
            ));
        }
        if record.status == OutboundStatus::Open {
            if self.pending.contains_key(&intent.call_id) {
                return Err(invalid(index, "a retained Call opens twice"));
            }
            let (instance, delivery) = self
                .scheduler
                .mailboxes
                .find_by_ref_any(slot, &intent.edge, intent.emitting_id)
                .map(|(instance, delivery)| (instance, delivery.clone()))
                .ok_or_else(|| invalid(index, "a Call opening has no retained request delivery"))?;
            if !self.instances.contains_key(&instance)
                || self.lifecycle.iter().any(|work| {
                    work.instance == instance && work.lifecycle == node::Lifecycle::Start
                })
            {
                return Err(invalid(index, "a Call opens before its owner starts"));
            }
            if self.plan.owners[self.plan.nodes[slot.index()].owner.index()]
                .loop_spec()
                .is_some_and(|spec| spec.entry == delivery.port)
                && self.loop_application(&instance, &delivery).is_none()
            {
                return Err(invalid(
                    index,
                    "a Call opens before its retained Loop admission",
                ));
            }
            let retry = self.retry_attempt(&instance, &delivery);
            if record
                .retry_record
                .as_deref()
                .and_then(|packet| packet.context)
                != retry
            {
                return Err(invalid(
                    index,
                    "the Call changed its admitted Retry context",
                ));
            }
            if retry.is_some() {
                self.retry_call_opened(retry, intent)?;
            } else {
                if self.plan.owners[self.plan.nodes[slot.index()].owner.index()]
                    .retry_spec()
                    .is_some()
                    && !self.closing.contains(&instance)
                {
                    return Err(invalid(
                        index,
                        "a Retry Call has no operation admission or retained Closing",
                    ));
                }
                let settings = self.resolve_invocation_settings(
                    &instance,
                    spec,
                    &intent.connector_path,
                    &delivery.envelope,
                )?;
                if settings != intent.resolved_settings {
                    return Err(invalid(
                        index,
                        "the Call settings contradict their retained request",
                    ));
                }
            }
            self.scheduler.mailboxes.remove_by_ref(
                slot,
                &instance,
                &intent.edge,
                intent.emitting_id,
            );
            self.scheduler.mailboxes.claim(slot, &instance);
            self.pending.insert(
                intent.call_id,
                PendingCall {
                    slot,
                    delivery,
                    instance,
                },
            );
            return Ok(());
        }
        let pending = self
            .pending
            .get(&intent.call_id)
            .ok_or_else(|| invalid(index, "a Call closure has no retained opening"))?;
        if journal.find_call(&intent.call_id).as_ref() != Some(intent)
            || pending.slot != slot
            || pending.delivery.edge != intent.edge
            || pending.delivery.envelope.meta().sys().id() != intent.emitting_id
            || self
                .settled
                .iter()
                .any(|entry| entry.event.call_id == intent.call_id)
        {
            return Err(invalid(
                index,
                "a Call closure is duplicated or crosses its request",
            ));
        }
        let (completion, trigger) = match &record.status {
            OutboundStatus::Closed(Closure::Response(completion)) => (
                completion.clone(),
                node::Trigger::Port(pending.delivery.port.clone()),
            ),
            OutboundStatus::Closed(Closure::Timeout) | OutboundStatus::Abandoned => {
                let terminal = if record.status == OutboundStatus::Abandoned {
                    SchedulerCompletion::OutcomeUnknown
                } else {
                    SchedulerCompletion::TimeoutExpired
                };
                let completion = bindings
                    .scheduler_completion(&intent.connector_path, terminal)
                    .map_err(crate::boundary::DispatchError::Driver)
                    .map_err(VmError::Dispatch)?;
                (
                    completion,
                    node::Trigger::Lifecycle(if record.status == OutboundStatus::Abandoned {
                        node::Lifecycle::Recover
                    } else {
                        node::Lifecycle::Timeout
                    }),
                )
            }
            OutboundStatus::Open => return Err(invalid(index, "duplicate Call opening")),
        };
        PlanGate { plan: self.plan }
            .admit(&intent.connector_path, &completion)
            .map_err(|_| invalid(index, "the retained Call completion violates its Contract"))?;
        let attempt = self.retry_attempt(&pending.instance, &pending.delivery);
        if record
            .retry_record
            .as_deref()
            .and_then(|packet| packet.context)
            != attempt
        {
            return Err(invalid(index, "the Call closure changed its Retry context"));
        }
        self.retain_retry_closure(record)?;
        self.settled.push_back(SettledEntry {
            event: BoundaryEvent {
                call_id: intent.call_id,
                connector_path: intent.connector_path.clone(),
                emitting_id: intent.emitting_id,
                completion,
            },
            trigger,
        });
        Ok(())
    }

    pub(super) fn restore_call_consumption(
        &mut self,
        admissions: &dyn InstanceAdmissions,
        index: usize,
        slot: Slot,
        record: &TransactionRecord,
        consumed: &ConsumedPosition,
    ) -> Result<bool, VmError> {
        let Some((call, pending)) = self
            .pending
            .iter()
            .find(|(_, pending)| {
                pending.slot == slot
                    && pending.instance == record.instance
                    && pending.delivery.edge == consumed.edge
                    && pending.delivery.source_instance == consumed.source
                    && pending.delivery.seq == consumed.seq
            })
            .map(|(call, pending)| (*call, pending.clone()))
        else {
            return Ok(false);
        };
        let settled = self
            .settled
            .iter()
            .position(|entry| entry.event.call_id == call)
            .ok_or_else(|| invalid(index, "a Call transaction consumes an unsettled request"))?;
        let entry = &self.settled[settled];
        let mut expected = HandlerExecutor::at(self.plan, self.graph_version, self.now)
            .connector_completion(
                slot,
                &pending.instance,
                &pending.delivery,
                &entry.trigger,
                &entry.event.completion,
            )
            .map_err(VmError::Fault)?;
        if record.trigger != entry.trigger
            || record.consumed != expected.record.consumed
            || record.emissions != expected.record.emissions
            || !record.state_delta.is_empty()
        {
            return Err(invalid(
                index,
                "a Call transaction contradicts its retained completion",
            ));
        }
        let retry = self.prepare_retry_completion(
            &pending.instance,
            &pending.delivery,
            call,
            &mut expected.record,
        )?;
        let held = retry.as_ref().is_some_and(|(_, held)| *held);
        if held {
            let context = self.graph_context(&pending.delivery);
            if !context.is_empty() {
                expected.record.loop_record = Some(Box::new(protocol::Packet {
                    context: context.to_vec(),
                    ..Default::default()
                }));
            }
        } else {
            self.prepare_call_completion(
                slot,
                &pending.instance,
                &pending.delivery,
                &mut expected.record,
            )?;
        }
        if self.graph_loops.is_some() && !held {
            let prepared = self.validate_and_prepare_emissions(
                admissions,
                slot,
                &pending.instance,
                &expected.record,
            )?;
            self.retain_emission_routes(&mut expected.record, &prepared)?;
        }
        if record.loop_record != expected.record.loop_record
            || record.retry_record != expected.record.retry_record
        {
            return Err(invalid(
                index,
                "a Call transaction contradicts its retained Loop verdict",
            ));
        }
        if let Some((fold, _)) = retry {
            self.retry_manager = Some(fold);
        }
        self.settled.remove(settled);
        self.pending.remove(&call);
        if !self
            .pending
            .values()
            .any(|other| other.slot == slot && other.instance == pending.instance)
        {
            self.scheduler.mailboxes.release(slot, &pending.instance);
        }
        Ok(true)
    }
}
