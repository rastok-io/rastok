//! Retry retains one prepared operation. This is a projection of committed
//! attempt facts, sharing the Worker's mailbox budget and dated scheduler.

use super::*;
use crate::envelope;
use crate::instance;
use crate::journal::loop_protocol::{Fact as LoopFact, Phase as LoopPhase};
use crate::journal::retry_protocol::{self as protocol, AttemptId, Fact, Fold, Packet, Phase};
use crate::journal::timing::{Deadline, Deadlines};
use crate::journal::{CallRecord, CallStatus, Closure, Record, TransactionRecord};

pub(super) fn error(error: protocol::Error) -> VmError {
    VmError::Journal(crate::journal::Error::InvalidRetryProtocol(error))
}

impl Vm<'_> {
    pub(super) fn retry_attempt(
        &self,
        instance: &instance::Generation,
        delivery: &envelope::Delivery,
    ) -> Option<AttemptId> {
        let reference = ids::delivery_id(&delivery.edge, delivery.envelope.meta().sys().id());
        self.retry_manager
            .as_deref()?
            .operations()
            .find_map(|(op, state)| {
                (state.instance == *instance
                    && state.delivery == reference
                    && state.phase != Phase::Closed)
                    .then_some(AttemptId {
                        operation: *op,
                        ordinal: state.ordinal,
                    })
            })
    }

    pub(super) fn pinned_retry_intent(
        &self,
        instance: &instance::Generation,
        delivery: &envelope::Delivery,
    ) -> Option<Intent> {
        let attempt = self.retry_attempt(instance, delivery)?;
        let state = self
            .retry_manager
            .as_deref()?
            .operation(attempt.operation)?;
        Some(Intent {
            call_id: ids::call_id(state.delivery, state.pin.prepared.digest()),
            emitting_id: delivery.envelope.meta().sys().id(),
            edge: delivery.edge.clone(),
            connector_path: node::Path::new(instance.address().owner().segments().iter().cloned())
                .ok()?,
            prepared: state.pin.prepared.clone(),
            idempotency_key: state.pin.key.clone(),
            resolved_settings: state.pin.settings.clone(),
            logical_time: self.now,
        })
    }

    pub(super) fn open_retry_operation(
        &mut self,
        journal: &mut dyn Journal,
        slot: Slot,
        instance: &instance::Generation,
        delivery: &envelope::Delivery,
        intent: &Intent,
    ) -> Result<bool, VmError> {
        let node = &self.plan.nodes[slot.index()];
        let Some(spec) = self.plan.owners[node.owner.index()].retry_spec() else {
            return Ok(false);
        };
        if self.retry_attempt(instance, delivery).is_some() {
            return Ok(false);
        }
        // Closing drains already admitted work as one ordinary Call. It must
        // neither open a new operation nor renew the request's queue budget.
        if self.closing.contains(instance) {
            return Ok(false);
        }
        let connector = node
            .connector()
            .ok_or_else(|| error(protocol::Error::OriginalBinding))?;
        let call = connector
            .call_semantics()
            .ok_or_else(|| error(protocol::Error::Policy))?;
        let policy = policy(connector, spec)?;
        let plan = self
            .original_hash
            .ok_or_else(|| error(protocol::Error::OriginalBinding))?;
        let opening = ids::delivery_id(&delivery.edge, delivery.envelope.meta().sys().id());
        let operation = protocol::operation_ref(instance, &spec.entry, opening);
        let packet = Packet {
            context: None,
            facts: vec![
                Fact::Opened {
                    operation,
                    opening,
                    entry: spec.entry.clone(),
                    plan,
                    contract: connector.contract.clone(),
                    pin: protocol::Pin {
                        envelope: intent.emitting_id,
                        prepared: intent.prepared.clone(),
                        settings: intent.resolved_settings.clone(),
                        key: intent.idempotency_key.clone(),
                        opened_at: intent.logical_time,
                    },
                },
                Fact::Admitted {
                    attempt: AttemptId {
                        operation,
                        ordinal: 1,
                    },
                    envelope: delivery.envelope.clone(),
                    delivery: opening,
                },
            ],
        };
        let mut next = self
            .retry_manager
            .as_deref()
            .cloned()
            .ok_or_else(|| error(protocol::Error::Policy))?;
        next.apply(
            &packet,
            Some(&protocol::Opening {
                instance,
                delivery: opening,
                plan,
                contract: &connector.contract,
                call,
                policy: &policy,
                envelope: &delivery.envelope,
            }),
            self.now,
        )
        .map_err(error)?;
        let mut record = manager_record(instance, intent.emitting_id, packet);
        let context = self.graph_context(delivery);
        if !context.is_empty() {
            record.loop_record = Some(Box::new(crate::journal::loop_protocol::Packet {
                context: context.to_vec(),
                ..Default::default()
            }));
        }
        journal.commit(record).map_err(VmError::Journal)?;
        self.retry_manager = Some(Box::new(next));
        Ok(true)
    }

    pub(super) fn retry_call_opened(
        &mut self,
        attempt: Option<AttemptId>,
        intent: &Intent,
    ) -> Result<(), VmError> {
        if let Some(attempt) = attempt {
            self.retry_manager
                .as_deref_mut()
                .ok_or_else(|| error(protocol::Error::UnknownOperation))?
                .open_call(attempt, intent)
                .map_err(error)?;
        }
        Ok(())
    }

    /// All closure information comes from the winning retained EGRESS, not
    /// the later completion transaction. In particular Closing cannot turn a
    /// previously held batch into a graph publication.
    pub(super) fn synchronize_retry_closures(
        &mut self,
        journal: &dyn Journal,
    ) -> Result<(), VmError> {
        let Some(fold) = self.retry_manager.as_deref() else {
            return Ok(());
        };
        let pending: BTreeSet<_> = fold
            .operations()
            .filter_map(|(_, state)| {
                (state.phase == Phase::CallOpen)
                    .then_some(state.call)
                    .flatten()
            })
            .collect();
        if pending.is_empty() {
            return Ok(());
        }
        let mut closures = Vec::new();
        journal
            .visit_records(&mut |record| {
                if let Record::Egress(crate::journal::EgressRecord::Call(record)) = record
                    && record.status != CallStatus::Open
                    && pending.contains(&record.intent.call_id)
                {
                    closures.push(record.clone());
                }
            })
            .map_err(VmError::Journal)?;
        for record in closures {
            self.retain_retry_closure(&record)?;
        }
        Ok(())
    }

    pub(super) fn retain_retry_closure(&mut self, record: &CallRecord) -> Result<(), VmError> {
        let Some(attempt) = record
            .retry_record
            .as_deref()
            .and_then(|packet| packet.context)
        else {
            return Ok(());
        };
        let connector = self.plan.nodes[self.slot_of(&record.intent.connector_path).index()]
            .connector()
            .ok_or_else(|| error(protocol::Error::OriginalBinding))?;
        let case = closure_case(connector, &record.status)?;
        let fold = self
            .retry_manager
            .as_deref_mut()
            .ok_or_else(|| error(protocol::Error::UnknownOperation))?;
        fold.settle(
            attempt,
            &protocol::Settlement {
                intent: &record.intent,
                case,
                closed_at: record
                    .closed_at
                    .ok_or_else(|| error(protocol::Error::Timing))?,
                schedule: record.deadlines.as_deref(),
            },
        )
        .map_err(error)?;
        if let Some(schedule) = record.deadlines.as_deref() {
            let state = fold
                .operation(attempt.operation)
                .ok_or_else(|| error(protocol::Error::UnknownOperation))?;
            for arm in schedule.items() {
                self.scheduler.arm(
                    arm.due_at,
                    DatedPayload::RetryDelay {
                        instance: state.instance.clone(),
                        attempt,
                        admission: arm.reference,
                    },
                );
            }
        }
        Ok(())
    }

    pub(super) fn prepare_retry_completion(
        &self,
        instance: &instance::Generation,
        delivery: &envelope::Delivery,
        call: CallId,
        record: &mut TransactionRecord,
    ) -> Result<Option<(Box<Fold>, bool)>, VmError> {
        let Some(attempt) = self.retry_attempt(instance, delivery) else {
            return Ok(None);
        };
        let fold = self
            .retry_manager
            .as_deref()
            .ok_or_else(|| error(protocol::Error::UnknownOperation))?;
        let state = fold
            .operation(attempt.operation)
            .ok_or_else(|| error(protocol::Error::UnknownOperation))?;
        let batch = protocol::Batch {
            attempt,
            call,
            emissions: record
                .emissions
                .iter()
                .map(|e| e.envelope.meta().sys().id())
                .collect(),
        };
        let held = state.withholds();
        let packet = Packet {
            context: Some(attempt),
            facts: if held {
                vec![Fact::Held(batch)]
            } else {
                vec![
                    Fact::Published(batch),
                    Fact::Close {
                        attempt,
                        reason: protocol::CloseReason::Final,
                    },
                ]
            },
        };
        fold.validate_emissions(&packet, &record.emissions)
            .map_err(error)?;
        let mut next = Box::new(fold.clone());
        next.apply(&packet, None, self.now).map_err(error)?;
        record.retry_record = Some(Box::new(packet));
        Ok(Some((next, held)))
    }

    pub(super) fn retry_manager_ready(&self) -> bool {
        self.retry_manager.as_deref().is_some_and(|fold| {
            fold.operations().any(|(_, state)| {
                state.phase == Phase::Held
                    || (state.phase == Phase::AwaitingAdmission
                        && (self.closing.contains(&state.instance)
                            || state.due_at.is_none_or(|due| due <= self.now)))
            })
        })
    }

    pub(super) fn retry_has_work(&self, instance: Option<&instance::Generation>) -> bool {
        self.retry_manager.as_deref().is_some_and(|fold| {
            fold.operations().any(|(_, state)| {
                state.phase != Phase::Closed
                    && instance.is_none_or(|instance| state.instance == *instance)
            })
        })
    }

    pub(super) fn recurrence_admission_load(&self) -> usize {
        let mut occupied = self
            .loop_admission_load()
            .saturating_add(self.graph_admission_load());
        let Some(fold) = self.retry_manager.as_deref() else {
            return occupied;
        };
        occupied = occupied.saturating_add(
            fold.operations()
                .filter(|(_, state)| state.phase != Phase::Closed)
                .count(),
        );
        for ((slot, instance), queue) in &self.scheduler.mailboxes.queues {
            let Some(spec) =
                self.plan.owners[self.plan.nodes[slot.index()].owner.index()].retry_spec()
            else {
                continue;
            };
            occupied = occupied.saturating_add(
                queue
                    .iter()
                    .filter(|delivery| {
                        delivery.port == spec.entry
                            && self.retry_attempt(instance, delivery).is_none()
                    })
                    .count(),
            );
        }
        occupied
    }

    /// Dated Loop and Retry admissions share the retained arm order even
    /// after the scheduler has acknowledged their wakeups. Derive this head
    /// from current folds and the journal, without a second ready queue.
    pub(super) fn next_dated_recurrence(
        &self,
        journal: &dyn Journal,
    ) -> Result<Option<Id>, VmError> {
        let loops = self.loop_manager.as_deref();
        let retries = self.retry_manager.as_deref();
        if loops.is_none() && retries.is_none() {
            return Ok(None);
        }
        let mut head = None;
        journal
            .visit_records(&mut |record| {
                let Some(schedule) = record.deadlines() else {
                    return;
                };
                for arm in schedule.items().iter().filter(|arm| {
                    arm.kind == crate::journal::DeadlineKind::RecurrenceDelay
                        && arm.due_at <= self.now
                }) {
                    let eligible = match record {
                        Record::Transaction(record) => {
                            record.loop_protocol().is_some_and(|packet| {
                                packet.facts.iter().any(|fact| {
                                    let LoopFact::Advance { run, .. } = fact else {
                                        return false;
                                    };
                                    loops.and_then(|fold| fold.run(*run)).is_some_and(|state| {
                                        state.phase == LoopPhase::AwaitingAdmission
                                            && state.admission == arm.reference
                                            && state.due_at == Some(arm.due_at)
                                            && self.graph_admission_is_head(*run)
                                            && !self.closing.contains(&state.instance)
                                    })
                                })
                            })
                        }
                        Record::Egress(crate::journal::EgressRecord::Call(call)) => call
                            .retry_record
                            .as_deref()
                            .and_then(|packet| packet.context)
                            .and_then(|attempt| {
                                retries.and_then(|fold| fold.operation(attempt.operation))
                            })
                            .is_some_and(|state| {
                                state.phase == Phase::AwaitingAdmission
                                    && state.call == Some(call.intent.call_id)
                                    && state.due_at == Some(arm.due_at)
                                    && !self.closing.contains(&state.instance)
                            }),
                        _ => false,
                    };
                    // Equal dates keep the first retained arm, not a policy
                    // priority or the ordering of derived run identities.
                    if eligible && head.is_none_or(|(due, _)| arm.due_at < due) {
                        head = Some((arm.due_at, arm.reference));
                    }
                }
            })
            .map_err(VmError::Journal)?;
        Ok(head.map(|(_, reference)| reference))
    }

    pub(super) fn step_retry_manager(
        &mut self,
        journal: &mut dyn Journal,
        dated_admission: Option<Id>,
    ) -> Result<bool, VmError> {
        if !self.retry_manager_ready() {
            return Ok(false);
        }
        let fold = self
            .retry_manager
            .as_deref()
            .ok_or_else(|| error(protocol::Error::UnknownOperation))?;
        // Equal deadlines inherit the winning closures' arm order. Hash order
        // is stable, but is not the retained scheduler tie-breaker. Reading the
        // prefix also preserves this order after a late recovery.
        let mut selected = None;
        let mut deferred = false;
        let mut index = 0usize;
        journal
            .visit_records(&mut |record| {
                if let Record::Egress(crate::journal::EgressRecord::Call(call)) = record
                    && call.status != CallStatus::Open
                    && let Some(attempt) = call
                        .retry_record
                        .as_deref()
                        .and_then(|packet| packet.context)
                    && let Some(state) = fold.operation(attempt.operation)
                    && state.call == Some(call.intent.call_id)
                {
                    let closing = self.closing.contains(&state.instance);
                    let ready = state.phase == Phase::Held
                        || (state.phase == Phase::AwaitingAdmission
                            && (closing || state.due_at.is_none_or(|due| due <= self.now)));
                    let in_turn = state.phase != Phase::AwaitingAdmission
                        || closing
                        || state.due_at.is_none()
                        || dated_admission.is_none_or(|reference| {
                            call.deadlines.as_deref().is_some_and(|schedule| {
                                schedule
                                    .items()
                                    .iter()
                                    .any(|arm| arm.reference == reference)
                            })
                        });
                    if ready && in_turn {
                        let key = (state.due_at.unwrap_or(i64::MIN), index);
                        if selected
                            .as_ref()
                            .is_none_or(|(previous, _)| key < *previous)
                        {
                            selected = Some((key, attempt.operation));
                        }
                    } else if ready {
                        deferred = true;
                    }
                }
                index += 1;
            })
            .map_err(VmError::Journal)?;
        let Some((_, operation)) = selected else {
            return if deferred {
                Ok(false)
            } else {
                Err(error(protocol::Error::Transition))
            };
        };
        let state = fold
            .operation(operation)
            .ok_or_else(|| error(protocol::Error::UnknownOperation))?
            .clone();
        let attempt = AttemptId {
            operation,
            ordinal: state.ordinal,
        };
        let mut facts = Vec::new();
        let mut admission = None;
        let slot = self.slot_of(
            &node::Path::new(state.instance.address().owner().segments().iter().cloned())
                .map_err(|_| error(protocol::Error::OriginalBinding))?,
        );
        if let Some(batch) = &state.pending {
            facts.push(Fact::Consumed(batch.clone()));
        }
        if self.closing.contains(&state.instance) {
            facts.push(Fact::Close {
                attempt,
                reason: protocol::CloseReason::Stopped,
            });
        } else if state.phase == Phase::Held {
            facts.push(Fact::Advance {
                from: attempt,
                to: state.ordinal + 1,
                call: state
                    .call
                    .ok_or_else(|| error(protocol::Error::Transition))?,
            });
        } else {
            if !self.scheduler.mailboxes.has_room(slot, &state.instance, 0) {
                return Ok(false);
            }
            let parent = journal
                .find_envelope(
                    state
                        .terminal_parent()
                        .ok_or_else(|| error(protocol::Error::Batch))?,
                )
                .ok_or_else(|| error(protocol::Error::Batch))?;
            let id = protocol::admission_ref(
                operation,
                state.ordinal + 1,
                state
                    .call
                    .ok_or_else(|| error(protocol::Error::Transition))?,
            );
            let envelope = Envelope::new(
                Meta::new(MetaSys::emission(
                    id,
                    parent.meta().sys(),
                    self.graph_version.0,
                ))
                .with_app(state.original.meta().app().clone())
                .with_ext(state.original.meta().ext().clone()),
                state.original.data().clone(),
            );
            let edge = protocol::admission_edge(operation, &state.entry);
            facts.push(Fact::Admitted {
                attempt: AttemptId {
                    operation,
                    ordinal: state.ordinal + 1,
                },
                delivery: ids::delivery_id(&edge, id),
                envelope: envelope.clone(),
            });
            admission = Some((edge, envelope));
        }
        let packet = Packet {
            context: None,
            facts,
        };
        let mut next = self
            .retry_manager
            .clone()
            .ok_or_else(|| error(protocol::Error::UnknownOperation))?;
        next.apply(&packet, None, self.now).map_err(error)?;
        let mut record = manager_record(
            &state.instance,
            state
                .terminal_parent()
                .unwrap_or(state.envelope.meta().sys().id()),
            packet,
        );
        let context = self.graph_context_for(state.opening).to_vec();
        if !context.is_empty() {
            record.loop_record = Some(Box::new(crate::journal::loop_protocol::Packet {
                context: context.clone(),
                ..Default::default()
            }));
        }
        let mut timeout = None;
        if let Some((edge, envelope)) = &admission {
            let reference = ids::delivery_id(edge, envelope.meta().sys().id());
            record.routes.push(RoutedDelivery {
                delivery: envelope.meta().sys().id(),
                edge: edge.clone(),
                target: state.instance.clone(),
            });
            if let Some(packet) = record.loop_record.as_deref_mut() {
                packet
                    .routes
                    .push(crate::journal::loop_protocol::RoutedContext {
                        delivery: reference,
                        runs: context.clone(),
                    });
            }
            timeout = self.plan.nodes[slot.index()].connector().and_then(|call| {
                match call.attribute("timeout") {
                    Some(AttrSetting::Value(crate::value::Value::Duration(ns))) => Some(*ns),
                    _ => None,
                }
            });
            let ns = timeout.ok_or_else(|| error(protocol::Error::Policy))?;
            record
                .deadline_ids
                .push((crate::journal::DeadlineKind::CallTimeout, reference));
            self.retain_timer_arms(
                &mut record,
                vec![Deadline {
                    kind: crate::journal::DeadlineKind::CallTimeout,
                    reference,
                    due_at: self.now.saturating_add(ns),
                }],
            )?;
        }
        journal.commit(record).map_err(VmError::Journal)?;
        self.retry_manager = Some(next);
        if let Some((edge, envelope)) = admission {
            if !context.is_empty() {
                self.install_graph_contexts(vec![crate::journal::loop_protocol::RoutedContext {
                    delivery: ids::delivery_id(&edge, envelope.meta().sys().id()),
                    runs: context,
                }]);
            }
            self.scheduler
                .mailboxes
                .deliver(
                    slot,
                    &state.instance,
                    &state.instance,
                    state.entry,
                    edge.clone(),
                    envelope.clone(),
                )
                .map_err(|_| error(protocol::Error::AdmissionIdentity))?;
            if let Some(ns) = timeout {
                self.scheduler.arm(
                    self.now.saturating_add(ns),
                    DatedPayload::DeliveryBudget {
                        connector: self.plan.nodes[slot.index()].path.clone(),
                        edge,
                        delivery: envelope.meta().sys().id(),
                        instance: state.instance,
                    },
                );
            }
        }
        Ok(true)
    }
}

pub(super) fn policy(
    connector: &crate::plan::ConnectorSpec,
    spec: &crate::plan::RetrySpec,
) -> Result<protocol::Policy, VmError> {
    let attempts = match spec.attempts.resolved {
        crate::value::Value::Int(n) => u64::try_from(n).ok(),
        crate::value::Value::Uint(n) => Some(n),
        _ => None,
    }
    .ok_or_else(|| error(protocol::Error::Policy))?;
    protocol::Policy::try_new(
        connector
            .call_semantics()
            .ok_or_else(|| error(protocol::Error::Policy))?,
        attempts,
        spec.cases.clone(),
        spec.delay.clone(),
        connector.is_idempotent(),
    )
    .map_err(error)
}

pub(super) fn closure_schedule(
    fold: Option<&Fold>,
    intent: &Intent,
    completion: &Completion,
    now: i64,
) -> Result<Option<Box<Deadlines>>, protocol::Error> {
    let Some(fold) = fold else { return Ok(None) };
    let Some((op, state)) = fold
        .operations()
        .find(|(_, state)| state.call == Some(intent.call_id))
    else {
        return Ok(None);
    };
    let case = completion
        .outputs()
        .last()
        .and_then(|output| output.data.materialized_value())
        .and_then(|value| {
            if let crate::value::Value::TaggedUnion(value) = value {
                Some(value.case.as_str())
            } else {
                None
            }
        })
        .ok_or(protocol::Error::Terminal)?;
    if fold.is_closing(&state.instance) || !state.policy().repeats(state.ordinal, case) {
        return Ok(None);
    }
    state
        .policy()
        .due_after(state.ordinal, now)
        .map(|due_at| {
            Deadlines::try_new(
                now,
                vec![Deadline {
                    kind: crate::journal::DeadlineKind::RecurrenceDelay,
                    reference: protocol::admission_ref(*op, state.ordinal + 1, intent.call_id),
                    due_at,
                }],
            )
            .map(Box::new)
            .map_err(|_| protocol::Error::Timing)
        })
        .transpose()
}

pub(super) fn closure_case<'a>(
    connector: &crate::plan::ConnectorSpec,
    status: &'a CallStatus,
) -> Result<&'a str, VmError> {
    match status {
        CallStatus::Closed(Closure::Response(completion)) => {
            let call = connector
                .call_semantics()
                .ok_or_else(|| error(protocol::Error::Policy))?;
            completion
                .outputs()
                .iter()
                .find(|output| output.port.name() == call.terminal())
                .and_then(|output| output.data.materialized_value())
                .and_then(|value| call.terminal_case(value))
                .ok_or_else(|| error(protocol::Error::Terminal))
        }
        CallStatus::Closed(Closure::Timeout) => Ok("TimeoutExpired"),
        CallStatus::Abandoned => Ok("OutcomeUnknown"),
        CallStatus::Open => Err(error(protocol::Error::Transition)),
    }
}

pub(super) fn is_held(record: &TransactionRecord) -> bool {
    record
        .retry_record
        .as_deref()
        .is_some_and(|packet| matches!(packet.facts.as_slice(), [Fact::Held(_)]))
}

pub(super) fn hold_emissions(record: &TransactionRecord) -> PreparedEmissions {
    PreparedEmissions {
        emissions: record
            .emissions
            .iter()
            .cloned()
            .map(|emission| PreparedEmission {
                emission,
                routes: Vec::new(),
            })
            .collect(),
        candidates: BTreeMap::new(),
        contexts: Vec::new(),
        commands: Vec::new(),
    }
}

pub(super) fn manager_record(
    instance: &instance::Generation,
    cause: Id,
    packet: Packet,
) -> TransactionRecord {
    TransactionRecord {
        owner_path: instance.address().owner().clone(),
        instance: instance.clone(),
        parent_instance: None,
        close_scope: None,
        trigger: node::Trigger::Lifecycle(node::Lifecycle::Retry),
        cause: Some(cause),
        exchange_exit: None,
        loop_record: None,
        instance_commands: None,
        retry_record: Some(Box::new(packet)),
        deadlines: None,
        logical_time: None,
        consumed: vec![],
        join_inputs: vec![],
        state_delta: vec![],
        emissions: vec![],
        routes: vec![],
        deadline_ids: vec![],
    }
}
