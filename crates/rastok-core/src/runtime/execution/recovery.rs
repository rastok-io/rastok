//! Reconstruct committed deliveries and private state without firing a
//! handler. Unsupported physical obligations refuse before replacing live state.

use super::*;
use crate::envelope;
use crate::instance;
use crate::journal::{Record, TransactionRecord, loop_protocol as protocol};

mod calls;
mod exchanges;
mod ingress;
mod loops;
mod retries;

impl Execution<'_> {
    /// Restores Executor instances, exchanges, timed joins, lifetimes,
    /// Closing, mailboxes/cursors, and command Loops from the committed journal prefix. No
    /// producer executes and no new record is appended during reconstruction.
    /// The explicitly supplied boot time is the resume instant; Loop deadlines
    /// come from retained `Advance` facts, never from that instant.
    ///
    /// A scheduler-owned Call closure needs the pure Contract projection from
    /// [`Self::recover_with_bindings`]. Reconstructed exchanges retain their
    /// obligations; recovery never reopens a socket or repeats a write. An
    /// inbound response already eligible for handoff is conservatively held
    /// until its original transport supplies a terminal fact.
    ///
    /// # Errors
    /// An unsupported recovery profile, an inconsistent committed prefix, or
    /// a journal read failure. On every error the original execution is unchanged.
    pub fn recover_from_journal(&mut self, journal: &dyn Journal) -> Result<(), ExecutionFault> {
        self.recover_with_bindings(journal, &mut NoOutboundBindings)
    }

    /// Reconstructs the same retained state, with Contract projections for
    /// scheduler-owned Call closures. Only `scheduler_completion` is used;
    /// recovery never prepares, starts, polls or cancels a physical effect.
    ///
    /// # Errors
    /// The same atomic recovery refusals as [`Self::recover_from_journal`],
    /// or an unavailable/invalid pure Contract projection.
    pub fn recover_with_bindings(
        &mut self,
        journal: &dyn Journal,
        bindings: &mut dyn OutboundDrivers,
    ) -> Result<(), ExecutionFault> {
        self.preflight(journal)?;
        let resume_at = self.now;
        let mut candidate = Self::new_at(self.plan, self.graph_version, 0);
        candidate.original_hash = self.original_hash;
        candidate.expected_deployment = self.expected_deployment.clone();
        candidate.unsupported_data_policy = self.unsupported_data_policy.clone();
        candidate.missing_deployment_binding = self.missing_deployment_binding.clone();
        let mut result = Ok(());
        let mut index = 0;
        let mut last_retained = None;
        let mut admissions =
            crate::journal::admission::AdmissionPrefix::retaining_instance_commands(
                self.instance_commands.is_some(),
            );
        journal
            .visit_records(&mut |record| {
                if result.is_ok() {
                    let instant = record
                        .deadlines()
                        .map(|schedule| schedule.created_at())
                        .or_else(|| match record {
                            Record::Transaction(record) => record.logical_time.or_else(|| {
                                record
                                    .retry_record
                                    .as_deref()
                                    .and_then(|packet| packet.facts.first())
                                    .and_then(|fact| match fact {
                                        crate::journal::retry_protocol::Fact::Opened {
                                            pin,
                                            ..
                                        } => Some(pin.opened_at),
                                        _ => None,
                                    })
                            }),
                            Record::Egress(crate::journal::EgressRecord::Call(record)) => {
                                record.closed_at.or(Some(record.intent.logical_time))
                            }
                            _ => None,
                        });
                    if let Some(instant) = instant {
                        if instant > resume_at {
                            result = Err(invalid(
                                index,
                                "the resume instant precedes retained logical time",
                            ));
                        } else if last_retained.is_some_and(|previous| instant < previous) {
                            result = Err(invalid(index, "retained logical time moves backwards"));
                        } else {
                            last_retained = Some(instant);
                            candidate.now = instant;
                        }
                    }
                    if result.is_ok() {
                        result = candidate
                            .restore_record(
                                journal,
                                &admissions,
                                bindings,
                                index,
                                record,
                                resume_at,
                            )
                            .and_then(|()| {
                                admissions.apply(record).map_err(ExecutionFault::Journal)
                            })
                            .and_then(|()| candidate.promote_waiting(&admissions));
                        if result.is_ok() {
                            candidate.progress_closures();
                        }
                    }
                }
                index += 1;
            })
            .map_err(ExecutionFault::Journal)?;
        result?;
        if candidate.instance_commands.is_some() {
            let fold = admissions
                .instance_commands()
                .map_err(ExecutionFault::Journal)?;
            candidate.instance_commands = Some(
                fold.commands()
                    .filter(|(_, state)| {
                        *state == crate::journal::instance_stop::Disposition::Pending
                    })
                    .map(|(command, _)| command.clone())
                    .collect(),
            );
        }
        candidate.retain_recovered_responses()?;
        candidate.poller = DriverPoller::resume(journal);
        candidate.now = resume_at;
        *self = candidate;
        Ok(())
    }

    fn restore_record(
        &mut self,
        journal: &dyn Journal,
        admissions: &dyn InstanceAdmissions,
        bindings: &mut dyn OutboundDrivers,
        index: usize,
        record: &Record,
        resume_at: i64,
    ) -> Result<(), ExecutionFault> {
        if self.instance_commands.is_none()
            && match record {
                Record::Ingress(record) => record.instance_commands.is_some(),
                Record::Transaction(record) => record.instance_commands.is_some(),
                Record::Egress(_) => false,
            }
        {
            return Err(invalid(
                index,
                "instance closure commands are absent from this plan",
            ));
        }
        match record {
            Record::Ingress(inbound) => self.restore_ingress(admissions, index, inbound),
            Record::Transaction(transaction) => {
                self.restore_transaction(journal, admissions, index, transaction, resume_at)?;
                if self.has_loops {
                    self.loop_manager
                        .get_or_insert_with(|| Box::new(protocol::LoopView::default()))
                        .apply_transaction(transaction)
                        .map_err(|error| {
                            ExecutionFault::Journal(crate::journal::Error::InvalidLoopProtocol(
                                error,
                            ))
                        })?;
                }
                Ok(())
            }
            Record::Egress(crate::journal::EgressRecord::Call(record)) => {
                self.restore_call(journal, bindings, index, record)
            }
            Record::Egress(crate::journal::EgressRecord::Session(record)) => {
                self.restore_session(index, record)
            }
        }
    }

    fn restore_transaction(
        &mut self,
        journal: &dyn Journal,
        admissions: &dyn InstanceAdmissions,
        index: usize,
        record: &TransactionRecord,
        resume_at: i64,
    ) -> Result<(), ExecutionFault> {
        crate::journal::retry_protocol::validate_transaction(record)
            .map_err(super::retry_manager::error)?;
        if record.trigger == node::Trigger::Lifecycle(node::Lifecycle::Retry) {
            return self.restore_retry_manager(index, record);
        }
        crate::journal::acceptance::validate_transaction(record)
            .map_err(|_| invalid(index, "unsealed deadlines contradict the accepting record"))?;
        if record
            .deadlines
            .iter()
            .flat_map(|schedule| schedule.items())
            .any(|deadline| {
                !matches!(
                    deadline.kind,
                    crate::journal::DeadlineKind::CallTimeout
                        | crate::journal::DeadlineKind::SessionHandshake
                        | crate::journal::DeadlineKind::JoinWithin
                        | crate::journal::DeadlineKind::InstanceLifetime
                        | crate::journal::DeadlineKind::RecurrenceDelay
                        | crate::journal::DeadlineKind::InstanceDrain
                        | crate::journal::DeadlineKind::InstanceSettlement
                ) || (matches!(
                    deadline.kind,
                    crate::journal::DeadlineKind::InstanceDrain
                        | crate::journal::DeadlineKind::InstanceSettlement
                ) && record.close_scope.is_none())
            })
        {
            return Err(invalid(
                index,
                "a retained deadline has no admitted transaction obligation",
            ));
        }
        if record.deadline_ids.iter().any(|(kind, _)| {
            !matches!(
                *kind,
                crate::journal::DeadlineKind::RecurrenceDelay
                    | crate::journal::DeadlineKind::CallTimeout
                    | crate::journal::DeadlineKind::SessionHandshake
                    | crate::journal::DeadlineKind::JoinWithin
                    | crate::journal::DeadlineKind::InstanceDrain
                    | crate::journal::DeadlineKind::InstanceSettlement
            )
        }) {
            return Err(invalid(
                index,
                "the transaction carries an unsupported recovery obligation",
            ));
        }
        if record
            .logical_time
            .is_some_and(|instant| instant > self.now)
        {
            return Err(invalid(
                index,
                "the resume instant precedes retained logical time",
            ));
        }
        self.restore_stop_residuals(index, record, resume_at)?;
        let instance = self
            .instances
            .get(&record.instance)
            .ok_or_else(|| invalid(index, "the transaction owner has no admitted instance"))?;
        let owner = instance.owner;
        if self.plan.nodes[owner.index()].path != record.owner_path {
            return Err(invalid(
                index,
                "the transaction owner differs from its instance",
            ));
        }
        match &record.trigger {
            node::Trigger::Lifecycle(node::Lifecycle::Start | node::Lifecycle::Stop) => {
                let work = self.lifecycle.front().ok_or_else(|| {
                    invalid(index, "a lifecycle transition has no pending obligation")
                })?;
                if work.instance != record.instance
                    || node::Trigger::Lifecycle(work.lifecycle) != record.trigger
                    || (work.lifecycle == node::Lifecycle::Start
                        && work.parent_instance != record.parent_instance)
                    || Some(work.cause.meta().sys().id()) != record.cause
                {
                    return Err(invalid(
                        index,
                        "the lifecycle start contradicts its opening",
                    ));
                }
                self.lifecycle.pop_front();
            }
            node::Trigger::Port(_) => {}
            node::Trigger::Lifecycle(
                node::Lifecycle::Timeout
                | node::Lifecycle::Recover
                | node::Lifecycle::Evict
                | node::Lifecycle::Closing,
            ) => {}
            node::Trigger::Lifecycle(node::Lifecycle::Reply | node::Lifecycle::Exit)
                if record.exchange_exit.is_some() => {}
            node::Trigger::Lifecycle(node::Lifecycle::Next) if record.loop_protocol().is_some() => {
            }
            _ => {
                return Err(invalid(
                    index,
                    "the lifecycle transition needs a dedicated recovery fold",
                ));
            }
        }

        let slot = match self.plan.nodes[owner.index()].kind() {
            node::Kind::Graph => {
                if record.loop_protocol().is_some()
                    && record.trigger == node::Trigger::Lifecycle(node::Lifecycle::Next)
                {
                    return self.restore_graph_manager(
                        journal, admissions, index, owner, record, resume_at,
                    );
                }
                let stopped = record.trigger == node::Trigger::Lifecycle(node::Lifecycle::Stop);
                let mut expected = record.clone();
                expected.loop_record = None;
                if stopped {
                    self.close_instance_loops(&record.instance, &mut expected)?;
                }
                if (!stopped && !record.consumed.is_empty())
                    || !record.state_delta.is_empty()
                    || !record.emissions.is_empty()
                    || !record.routes.is_empty()
                    || record.loop_record != expected.loop_record
                {
                    return Err(invalid(
                        index,
                        "a passive Graph lifecycle carries executable work",
                    ));
                }
                self.check_recovered_delivery_deadlines(index, record.deadlines.as_deref(), &[])?;
                self.restore_instance_closure(journal, index, record, None)?;
                if record.trigger == node::Trigger::Lifecycle(node::Lifecycle::Stop) {
                    self.discard_graph_admissions(&record.instance);
                    self.instances.remove(&record.instance);
                }
                return Ok(());
            }
            node::Kind::Operator | node::Kind::Connector => owner,
        };
        let exchange_consumed = self.restore_exchange_exit(index, slot, record)?;
        if record.trigger == node::Trigger::Lifecycle(node::Lifecycle::Next)
            && record.loop_protocol().is_some()
        {
            self.check_node_manager(journal, index, slot, record, resume_at)?;
        } else if record.exchange_exit.is_none() {
            self.check_consumed_loop_context(index, slot, record)?;
        }
        let retry_held = super::retry_manager::is_held(record);
        if self.graph_loops.is_some() && !retry_held {
            let mut derived = record.clone();
            if let Some(packet) = derived.loop_record.as_deref_mut() {
                packet.produced.retain(|command| {
                    self.loop_manager
                        .as_deref()
                        .and_then(|fold| fold.run(command.run))
                        .is_some_and(|state| state.instance == record.instance)
                });
            }
            self.retain_graph_verdicts(&mut derived)?;
            if record.loop_protocol().map(|packet| &packet.produced)
                != derived.loop_protocol().map(|packet| &packet.produced)
            {
                return Err(invalid(
                    index,
                    "Graph verdicts differ from the consumed context and output routes",
                ));
            }
        }
        let mut consumed_cause = None;
        let mut consumed_call = false;
        for consumed in record
            .consumed
            .iter()
            .filter(|_| record.trigger != node::Trigger::Lifecycle(node::Lifecycle::Stop))
        {
            if exchange_consumed.contains(consumed) {
                continue;
            }
            if self.restore_call_consumption(admissions, index, slot, record, consumed)? {
                consumed_call = true;
                continue;
            }
            let delivery = self
                .scheduler
                .mailboxes
                .queued(slot, &record.instance)
                .and_then(|queue| {
                    queue.iter().find(|delivery| {
                        self.scheduler.mailboxes.routes.text(delivery.edge) == consumed.edge
                            && delivery.source_instance == consumed.source
                            && delivery.seq == consumed.seq
                    })
                })
                .ok_or_else(|| invalid(index, "a consumed cursor has no pending delivery"))?
                .clone();
            if record.close_scope.is_some()
                && record.trigger
                    == node::Trigger::Port(
                        self.scheduler.mailboxes.routes.port(delivery.edge).clone(),
                    )
            {
                consumed_cause = Some(delivery.envelope.clone());
            }
            self.scheduler.mailboxes.remove_by_index(
                slot,
                &record.instance,
                delivery.edge,
                delivery.envelope.meta().sys().id(),
            );
        }
        if record.retry_record.is_some() && !consumed_call {
            return Err(invalid(
                index,
                "a Retry disposition requires consumption of its retained Connector Call",
            ));
        }
        if !record
            .state_delta
            .windows(2)
            .all(|pair| pair[0].0 < pair[1].0)
            || record.state_delta.iter().any(|(name, value)| {
                !self.plan.nodes[slot.index()]
                    .vars()
                    .iter()
                    .any(|var| var.name == *name && var.ty.accepts(value))
            })
        {
            return Err(invalid(
                index,
                "a private-state delta violates the declared cells",
            ));
        }

        let prepared = if retry_held {
            super::retry_manager::hold_emissions(record)
        } else {
            self.validate_and_prepare_emissions(admissions, slot, &record.instance, record)?
        };
        if record
            .instance_commands
            .as_deref()
            .map(|packet| packet.produced.as_slice())
            .unwrap_or_default()
            != prepared.commands
        {
            return Err(invalid(
                index,
                "instance commands differ from the emitted values and exact prefix targets",
            ));
        }
        let mut expected_contexts = prepared.contexts.clone();
        let mut expected_deadlines = self.emission_deadlines(&prepared);
        let mut expected = self.routed_deliveries(&prepared);
        let mut recurrence = None;
        if let Some(packet) = record.loop_protocol() {
            for fact in &packet.facts {
                if let protocol::Fact::Admit {
                    run,
                    iteration,
                    envelope,
                    ..
                } = fact
                    && *iteration > 0
                {
                    let state = self
                        .loop_manager
                        .as_deref()
                        .and_then(|fold| fold.run(*run))
                        .ok_or_else(|| invalid(index, "a recurrence has no opening fact"))?;
                    if recurrence.is_some() || state.instance != record.instance {
                        return Err(invalid(
                            index,
                            "an admission targets another run or repeats its delivery",
                        ));
                    }
                    let edge = super::loop_manager::manager_edge(*run, &state.entry);
                    expected.push(RoutedDelivery {
                        delivery: envelope.meta().sys().id(),
                        edge: edge.clone(),
                        target: record.instance.clone(),
                    });
                    recurrence = Some((state.entry.clone(), edge, envelope.clone()));
                    if !state.enclosing.is_empty() {
                        expected_contexts.push(protocol::RoutedContext {
                            delivery: ids::delivery_id(
                                &super::loop_manager::manager_edge(*run, &state.entry),
                                envelope.meta().sys().id(),
                            ),
                            runs: state.enclosing.clone(),
                        });
                    }
                    if let Some(AttrSetting::Value(crate::value::Value::Duration(timeout))) =
                        self.plan.nodes[slot.index()]
                            .connector()
                            .and_then(|spec| spec.attribute("timeout"))
                    {
                        expected_deadlines.push(crate::journal::deadline::Deadline {
                            kind: crate::journal::DeadlineKind::CallTimeout,
                            reference: ids::delivery_id(
                                &super::loop_manager::manager_edge(*run, &state.entry),
                                envelope.meta().sys().id(),
                            ),
                            due_at: self.now.saturating_add(*timeout),
                        });
                    }
                }
            }
        }
        expected.sort();
        expected_contexts.sort_by_key(|route| route.delivery);
        if record
            .loop_protocol()
            .map_or(&[][..], |packet| packet.routes.as_slice())
            != expected_contexts
        {
            return Err(invalid(
                index,
                "routed contexts differ from the exact application",
            ));
        }
        if expected != record.routes {
            return Err(invalid(
                index,
                "the retained deliveries differ from the selected routes",
            ));
        }
        self.check_recovered_delivery_deadlines(
            index,
            record.deadlines.as_deref(),
            &expected_deadlines,
        )?;
        self.state.apply_unchecked(
            self.plan.plan(),
            slot,
            &record.instance,
            &record.state_delta,
        );
        self.restore_instance_closure(journal, index, record, consumed_cause)?;
        self.route_emissions(slot, &record.instance, prepared);
        if let Some(packet) = record.loop_protocol() {
            for fact in &packet.facts {
                if let protocol::Fact::Advance {
                    run,
                    to_iteration,
                    command,
                    due_at: Some(due_at),
                    ..
                } = fact
                {
                    self.scheduler.schedule(
                        *due_at,
                        Obligation::LoopDelay {
                            owner,
                            instance: record.instance.clone(),
                            run: *run,
                            iteration: *to_iteration,
                            state: protocol::admission_ref(*run, *to_iteration, *command),
                        },
                    );
                }
            }
        }
        if let Some((port, edge, envelope)) = recurrence {
            let deadline = expected_deadlines
                .last()
                .filter(|deadline| deadline.kind == crate::journal::DeadlineKind::CallTimeout);
            if let Some(deadline) = deadline {
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
            self.scheduler
                .mailboxes
                .deliver(
                    slot,
                    &record.instance,
                    &record.instance,
                    port,
                    edge,
                    envelope,
                )
                .map_err(|_| invalid(index, "a recurrent delivery exceeds its mailbox capacity"))?;
            self.update_graph_loop_contexts(expected_contexts);
        }
        if record.trigger == node::Trigger::Lifecycle(node::Lifecycle::Stop) {
            self.state
                .take_unchecked(self.plan.plan(), slot, &record.instance);
            self.instances.remove(&record.instance);
        }
        Ok(())
    }

    fn check_consumed_loop_context(
        &self,
        index: usize,
        slot: NodeIndex,
        record: &TransactionRecord,
    ) -> Result<(), ExecutionFault> {
        if !self.has_loops {
            return Ok(());
        }
        let mut context = Vec::new();
        for (position, consumed) in record
            .consumed
            .iter()
            .filter(|_| record.trigger != node::Trigger::Lifecycle(node::Lifecycle::Stop))
            .enumerate()
        {
            let matches = |delivery: &&envelope::Delivery| {
                self.scheduler.mailboxes.routes.text(delivery.edge) == consumed.edge
                    && delivery.source_instance == consumed.source
                    && delivery.seq == consumed.seq
            };
            // A Call keeps its delivery claimed until its completion
            // transaction; opening it does not detach the causal context.
            let delivery = self
                .scheduler
                .mailboxes
                .queued(slot, &record.instance)
                .into_iter()
                .flatten()
                .chain(
                    self.pending
                        .values()
                        .filter(|pending| {
                            pending.slot == slot && pending.instance == record.instance
                        })
                        .map(|pending| &pending.delivery),
                )
                .find(matches)
                .ok_or_else(|| invalid(index, "a consumed Loop cursor has no retained delivery"))?;
            let application = self.loop_application(&record.instance, delivery);
            let entry = self.plan.nodes[slot.index()]
                .loop_spec()
                .is_some_and(|spec| {
                    &spec.entry == self.scheduler.mailboxes.routes.port(delivery.edge)
                });
            if entry && application.is_none() {
                return Err(invalid(
                    index,
                    "a Loop entry is consumed before its retained run admission",
                ));
            }
            let mut runs = self.graph_context(delivery).to_vec();
            if let Some((run, _)) = application
                && runs.last() != Some(&run)
            {
                runs.push(run);
            }
            if position == 0 {
                context = runs;
            } else if context != runs {
                return Err(invalid(
                    index,
                    "consumed deliveries belong to different Loop contexts",
                ));
            }
        }
        let retained = record
            .loop_protocol()
            .map_or(&[][..], |packet| packet.context.as_slice());
        if context != retained {
            return Err(invalid(
                index,
                "the Loop context differs from the exact consumed admission",
            ));
        }
        Ok(())
    }

    fn restore_stop_residuals(
        &mut self,
        index: usize,
        record: &TransactionRecord,
        resume_at: i64,
    ) -> Result<(), ExecutionFault> {
        if record.trigger != node::Trigger::Lifecycle(node::Lifecycle::Stop) {
            return Ok(());
        }
        if !self
            .lifecycle
            .iter()
            .any(|work| work.instance == record.instance && work.lifecycle == node::Lifecycle::Stop)
        {
            let scope = self
                .instances
                .get(&record.instance)
                .and_then(|instance| match &instance.phase {
                    instance::Phase::Closing { scope, .. } => Some(scope.clone()),
                    _ => None,
                })
                .ok_or_else(|| invalid(index, "a stop has no retained close request"))?;
            let drain_at = self.scheduler.dated.iter().find_map(|((at, _), payload)| {
                matches!(payload, Obligation::InstanceClosing { instance, deadline: ClosingDeadline::Drain, .. } if instance == &scope).then_some(*at)
            }).filter(|at| *at <= resume_at).ok_or_else(|| invalid(index, "a non-quiescent stop has no elapsed retained drain deadline"))?;
            // The stop seals every residual cursor of its instance. Its
            // ability to run proves that this scope's retained drain has won;
            // reconstruct that local state, never repeat physical cancellation.
            self.now = self.now.max(drain_at);
            self.begin_settlement(&scope);
            self.progress_closures();
        }
        let mut expected: Vec<_> = self
            .instances
            .get(&record.instance)
            .into_iter()
            .flat_map(|instance| &instance.residual)
            .map(|delivery| crate::journal::ConsumedPosition {
                edge: self
                    .scheduler
                    .mailboxes
                    .routes
                    .text(delivery.edge)
                    .to_string(),
                source: delivery.source_instance.clone(),
                seq: delivery.seq,
            })
            .collect();
        expected.extend(self.graph_stop_cursors(&record.instance));
        expected.sort();
        if expected != record.consumed {
            return Err(invalid(
                index,
                "a stop does not consume exactly its retained residual cursors",
            ));
        }
        Ok(())
    }

    fn restore_instance_closure(
        &mut self,
        journal: &dyn Journal,
        index: usize,
        record: &TransactionRecord,
        consumed_cause: Option<Envelope>,
    ) -> Result<(), ExecutionFault> {
        let Some(scope) = record.close_scope.as_ref() else {
            return Ok(());
        };
        if scope != &record.instance
            || !self
                .instances
                .get(scope)
                .is_some_and(|instance| matches!(instance.phase, instance::Phase::Open))
        {
            return Err(invalid(
                index,
                "a close request crosses or repeats its live instance",
            ));
        }
        let mut cause = consumed_cause;
        let mut position = 0;
        journal
            .visit_records(&mut |earlier| {
                if position < index {
                    match earlier {
                        Record::Ingress(inbound)
                            if Some(inbound.envelope.meta().sys().id()) == record.cause =>
                        {
                            cause = Some(inbound.envelope.clone())
                        }
                        Record::Transaction(transaction) => {
                            for emission in &transaction.emissions {
                                if Some(emission.envelope.meta().sys().id()) == record.cause {
                                    cause = Some(emission.envelope.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                }
                position += 1;
            })
            .map_err(ExecutionFault::Journal)?;
        let cause = cause
            .ok_or_else(|| invalid(index, "a close request has no earlier causal envelope"))?;
        let expected = self.closing_deadlines(scope);
        let retained: Vec<_> = record
            .deadlines
            .iter()
            .flat_map(|schedule| schedule.items())
            .filter(|deadline| {
                matches!(
                    deadline.kind,
                    crate::journal::DeadlineKind::InstanceDrain
                        | crate::journal::DeadlineKind::InstanceSettlement
                )
            })
            .copied()
            .collect();
        if expected != retained {
            return Err(invalid(
                index,
                "closing deadlines are missing or contradict the close request",
            ));
        }
        self.activate_close(scope, cause.clone());
        self.schedule_closing_deadlines(scope, cause.meta().sys().id());
        Ok(())
    }

    pub(super) fn check_recovered_delivery_deadlines(
        &self,
        index: usize,
        schedule: Option<&crate::journal::deadline::Deadlines>,
        expected: &[crate::journal::deadline::Deadline],
    ) -> Result<(), ExecutionFault> {
        let retained: Vec<_> = schedule
            .into_iter()
            .flat_map(|schedule| schedule.items())
            .filter(|deadline| {
                matches!(
                    deadline.kind,
                    crate::journal::DeadlineKind::CallTimeout
                        | crate::journal::DeadlineKind::JoinWithin
                        | crate::journal::DeadlineKind::SessionHandshake
                        | crate::journal::DeadlineKind::InstanceLifetime
                )
            })
            .copied()
            .collect();
        if retained != expected {
            return Err(invalid(
                index,
                "delivery deadlines are missing or contradict the accepting routes",
            ));
        }
        Ok(())
    }
}

fn invalid(record: usize, reason: &'static str) -> ExecutionFault {
    ExecutionFault::InvalidRecovery { record, reason }
}
