//! Exchange WAL and terminal cursors. Reconstructing an intent never re-offers it.

use super::*;
use crate::journal::{ConsumedPosition, InboundExchangeExitRecord, SessionRecord, SessionStatus};

impl Vm<'_> {
    pub(super) fn restore_session(
        &mut self,
        index: usize,
        record: &SessionRecord,
    ) -> Result<(), VmError> {
        let slot = self
            .plan
            .nodes
            .iter()
            .position(|node| node.path == record.connector_path)
            .map(|slot| Slot::new(slot as u32))
            .ok_or_else(|| invalid(index, "a Session fact has no planned Connector"))?;
        let spec = self.plan.nodes[slot.index()]
            .connector()
            .ok_or_else(|| invalid(index, "a Session fact contradicts its Contract profile"))?;
        let opening = session_opening(&spec.contract, spec.roles, spec.exchange)
            .ok_or_else(|| invalid(index, "the Contract has no Session opening rule"))?;
        match &record.status {
            SessionStatus::Opening(intent) => {
                if opening != SessionOpening::Local
                    || self.sessions.contains_key(&record.session_ref)
                    || intent.connector_path != record.connector_path
                    || intent.session_ref != record.session_ref
                    || ids::outbound_session_ref(
                        &intent.connector_path,
                        intent.emitting_id,
                        intent.prepared.digest(),
                    ) != intent.session_ref
                {
                    return Err(invalid(
                        index,
                        "a Session opening contradicts its identity or Contract",
                    ));
                }
                let (instance, delivery) =
                    self.retained_session_delivery(index, slot, intent.emitting_id)?;
                if self
                    .sessions
                    .values()
                    .any(|session| session.slot == slot && session.instance == instance)
                {
                    return Err(invalid(
                        index,
                        "an outbound Session instance already has an opening",
                    ));
                }
                self.scheduler.mailboxes.remove_by_ref(
                    slot,
                    &instance,
                    &delivery.edge,
                    intent.emitting_id,
                );
                self.scheduler.mailboxes.claim(slot, &instance);
                self.sessions.insert(
                    record.session_ref,
                    PendingSession {
                        slot,
                        instance,
                        opening: delivery.envelope.clone(),
                        open_intent: Some(intent.clone()),
                        opening_delivery: Some(delivery),
                        opened: false,
                        opening_started: true,
                        opening_claimed: true,
                        close_started: false,
                    },
                );
            }
            SessionStatus::Opened => {
                let session = self
                    .sessions
                    .get_mut(&record.session_ref)
                    .filter(|session| {
                        session.slot == slot && !session.opened && session.open_intent.is_some()
                    })
                    .ok_or_else(|| {
                        invalid(index, "a Session confirmation has no pending opening")
                    })?;
                session.opened = true;
                if session.opening_claimed {
                    self.scheduler.mailboxes.release(slot, &session.instance);
                    session.opening_claimed = false;
                }
            }
            SessionStatus::WriteOpen(intent) => {
                let session = self
                    .sessions
                    .get(&record.session_ref)
                    .filter(|session| session.slot == slot && session.opened)
                    .ok_or_else(|| {
                        invalid(index, "a write has no open Session at this Connector")
                    })?;
                if intent.connector_path != record.connector_path
                    || intent.session_ref != record.session_ref
                    || ids::session_write_ref(
                        intent.session_ref,
                        intent.emitting_id,
                        intent.prepared.digest(),
                    ) != intent.write_ref
                    || self.session_writes.contains_key(&intent.write_ref)
                {
                    return Err(invalid(
                        index,
                        "a Session write contradicts its identity or repeats an intent",
                    ));
                }
                let (instance, delivery) =
                    self.retained_session_delivery(index, slot, intent.emitting_id)?;
                let settings = self.resolve_invocation_settings(
                    &instance,
                    spec,
                    &record.connector_path,
                    &delivery.envelope,
                )?;
                let selected = self.select_session_ref(spec, &delivery.envelope, &settings)?;
                if session.instance != instance
                    || selected.is_some_and(|selected| selected != record.session_ref)
                    || (selected.is_none() && opening == SessionOpening::Peer)
                {
                    return Err(invalid(
                        index,
                        "a Session write crosses its source instance or correlator",
                    ));
                }
                self.scheduler.mailboxes.remove_by_ref(
                    slot,
                    &instance,
                    &delivery.edge,
                    intent.emitting_id,
                );
                self.session_writes.insert(
                    intent.write_ref,
                    PendingSessionWrite {
                        intent: intent.clone(),
                        slot,
                        instance: instance.clone(),
                        delivery,
                        started: true,
                    },
                );
                let held = self
                    .session_writes
                    .values()
                    .filter(|write| write.slot == slot && write.instance == instance)
                    .count();
                if held > WORKER_MAILBOX_CAPACITY {
                    return Err(invalid(
                        index,
                        "a retained Session write exceeds its mailbox capacity",
                    ));
                }
                if held == WORKER_MAILBOX_CAPACITY {
                    self.scheduler.mailboxes.claim(slot, &instance);
                }
            }
            SessionStatus::WriteSettled(reference) | SessionStatus::WriteAbandoned(reference) => {
                let write = self
                    .session_writes
                    .remove(reference)
                    .filter(|write| {
                        write.slot == slot && write.intent.session_ref == record.session_ref
                    })
                    .ok_or_else(|| {
                        invalid(index, "a write settlement has no matching pending intent")
                    })?;
                self.scheduler.mailboxes.release(slot, &write.instance);
            }
        }
        Ok(())
    }

    fn retained_session_delivery(
        &self,
        index: usize,
        slot: Slot,
        id: crate::envelope::Id,
    ) -> Result<(InstanceRef, Delivery), VmError> {
        let mut matches = self.instances.iter().filter_map(|(instance, state)| {
            if state.owner != self.plan.nodes[slot.index()].owner
                || self.lifecycle.iter().any(|work| {
                    work.instance == *instance && work.lifecycle == node::Lifecycle::Start
                })
            {
                return None;
            }
            let delivery = self.scheduler.mailboxes.queued(slot, instance)?.front()?;
            (delivery.envelope.meta().sys().id() == id)
                .then(|| (instance.clone(), delivery.clone()))
        });
        let delivery = matches
            .next()
            .ok_or_else(|| invalid(index, "a Session intent has no ready retained delivery"))?;
        if matches.next().is_some() {
            return Err(invalid(
                index,
                "a Session intent cannot identify a unique delivery",
            ));
        }
        Ok(delivery)
    }

    pub(super) fn restore_exchange_exit(
        &mut self,
        index: usize,
        slot: Slot,
        record: &TransactionRecord,
    ) -> Result<Vec<ConsumedPosition>, VmError> {
        let Some(exchange) = record.exchange_exit.as_deref() else {
            return Ok(Vec::new());
        };
        if !record.state_delta.is_empty()
            || record.loop_record.is_some()
            || !record.join_inputs.is_empty()
            || record.parent_instance.is_some()
            || record.close_scope.is_some()
        {
            return Err(invalid(
                index,
                "an exchange terminal carries unrelated application state",
            ));
        }
        let completion = Completion::try_new(
            record
                .emissions
                .iter()
                .map(|emission| crate::boundary::PortOutput {
                    port: emission.port.clone(),
                    data: emission.envelope.data().clone(),
                    ext: emission.envelope.meta().ext().clone(),
                })
                .collect(),
        )
        .map_err(|_| invalid(index, "an exchange terminal has no unique final exit"))?;
        PlanGate { plan: self.plan }
            .admit(&self.plan.nodes[slot.index()].path, &completion)
            .map_err(|_| invalid(index, "an exchange terminal violates its Contract"))?;
        let executor = HandlerExecutor::at(self.plan, self.graph_version, self.now);
        let mut held = Vec::new();
        let expected = match exchange {
            InboundExchangeExitRecord::Call(reference) => {
                let pending = self
                    .inbound_calls
                    .get(reference)
                    .filter(|call| call.slot == slot && call.instance == record.instance)
                    .ok_or_else(|| {
                        invalid(
                            index,
                            "an inbound Call terminal has no matching open request",
                        )
                    })?;
                match record.consumed.as_slice() {
                    [] => executor.inbound_call_completion(
                        slot,
                        &pending.instance,
                        &pending.request,
                        *reference,
                        &completion,
                    ),
                    [cursor] => {
                        let delivery =
                            self.recovered_exchange_cursor(index, slot, &record.instance, cursor)?;
                        let spec = self.plan.nodes[slot.index()].connector().ok_or_else(|| {
                            invalid(index, "an exchange terminal has no Contract")
                        })?;
                        if self.select_inbound_call_ref(spec, &delivery.envelope)?
                            != Some(*reference)
                        {
                            return Err(invalid(
                                index,
                                "an inbound response consumes another request's delivery",
                            ));
                        }
                        let expected = executor.inbound_call_response_completion(
                            slot,
                            &pending.instance,
                            &pending.request,
                            &delivery,
                            *reference,
                            &completion,
                        );
                        held.push(delivery);
                        expected
                    }
                    _ => {
                        return Err(invalid(
                            index,
                            "an inbound Call consumes more than one response",
                        ));
                    }
                }
            }
            InboundExchangeExitRecord::Session(reference) => {
                let session = self
                    .sessions
                    .get(reference)
                    .filter(|session| session.slot == slot && session.instance == record.instance)
                    .ok_or_else(|| invalid(index, "a Session terminal has no matching opening"))?;
                if self
                    .session_writes
                    .values()
                    .any(|write| write.intent.session_ref == *reference)
                {
                    return Err(invalid(
                        index,
                        "a Session exits before its write obligations settle",
                    ));
                }
                if let Some(delivery) = &session.opening_delivery {
                    held.push(delivery.clone());
                }
                let required = held.len();
                if record.consumed.len() < required || record.consumed.len() > required + 1 {
                    return Err(invalid(
                        index,
                        "a Session terminal has missing or extra consumption",
                    ));
                }
                if let Some(cursor) = record.consumed.get(required) {
                    let delivery =
                        self.recovered_exchange_cursor(index, slot, &record.instance, cursor)?;
                    let spec = self.plan.nodes[slot.index()]
                        .connector()
                        .ok_or_else(|| invalid(index, "a Session has no Contract"))?;
                    let settings = self.resolve_invocation_settings(
                        &record.instance,
                        spec,
                        &self.plan.nodes[slot.index()].path,
                        &delivery.envelope,
                    )?;
                    if self
                        .select_session_ref(spec, &delivery.envelope, &settings)?
                        .is_some_and(|selected| selected != *reference)
                    {
                        return Err(invalid(
                            index,
                            "a rejected write crossed its logical Session",
                        ));
                    }
                    held.push(delivery);
                }
                executor.session_completion(
                    slot,
                    &session.instance,
                    &session.opening,
                    *reference,
                    &completion,
                    &held.iter().collect::<Vec<_>>(),
                )
            }
        }
        .map_err(VmError::Fault)?
        .record;
        if record.trigger != expected.trigger
            || record.cause != expected.cause
            || record.consumed != expected.consumed
            || record.emissions != expected.emissions
        {
            return Err(invalid(
                index,
                "the exchange exit differs from its retained opening, cause or consumption",
            ));
        }
        for delivery in held {
            self.scheduler.mailboxes.remove_by_ref(
                slot,
                &record.instance,
                &delivery.edge,
                delivery.envelope.meta().sys().id(),
            );
        }
        match exchange {
            InboundExchangeExitRecord::Call(reference) => {
                self.inbound_calls.remove(reference);
            }
            InboundExchangeExitRecord::Session(reference) => {
                self.sessions.remove(reference);
            }
        }
        self.scheduler.mailboxes.release(slot, &record.instance);
        Ok(expected.consumed)
    }

    fn recovered_exchange_cursor(
        &self,
        index: usize,
        slot: Slot,
        instance: &InstanceRef,
        cursor: &ConsumedPosition,
    ) -> Result<Delivery, VmError> {
        self.scheduler
            .mailboxes
            .queued(slot, instance)
            .into_iter()
            .flatten()
            .find(|delivery| {
                delivery.edge == cursor.edge
                    && delivery.source_instance == cursor.source
                    && delivery.seq == cursor.seq
            })
            .cloned()
            .ok_or_else(|| invalid(index, "an exchange terminal consumes no retained cursor"))
    }

    pub(super) fn retain_recovered_responses(&mut self) -> Result<(), VmError> {
        // A retained response may have reached the peer before the crash.
        // No durable fact distinguishes not-started from started: hold it for
        // the original exchange's terminal event, never offer it a second time.
        let mut after = None;
        while let Some((slot, instance, delivery)) =
            self.scheduler
                .mailboxes
                .next_runnable_key(after.as_ref(), |slot, instance| {
                    self.plan.nodes[slot.index()]
                        .connector()
                        .is_some_and(|spec| {
                            spec.roles.has_inbound()
                                && !spec.roles.has_outbound()
                                && spec.exchange == ConnectorExchange::Call
                        })
                        && !self.lifecycle.iter().any(|work| {
                            work.instance == *instance && work.lifecycle == node::Lifecycle::Start
                        })
                })
        {
            after = Some((slot, instance.clone()));
            let Some(spec) = self.plan.nodes[slot.index()].connector() else {
                continue;
            };
            let Some(reference) = self.select_inbound_call_ref(spec, &delivery.envelope)? else {
                continue;
            };
            let Some(pending) = self.inbound_calls.get_mut(&reference).filter(|pending| {
                pending.slot == slot && pending.instance == instance && pending.response.is_none()
            }) else {
                continue;
            };
            self.scheduler.mailboxes.remove_by_ref(
                slot,
                &instance,
                &delivery.edge,
                delivery.envelope.meta().sys().id(),
            );
            self.scheduler.mailboxes.claim(slot, &instance);
            pending.response = Some(PendingInboundCallResponse {
                delivery,
                started: true,
            });
        }
        Ok(())
    }
}
