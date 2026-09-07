//! Reconstruct committed deliveries and private state without firing a
//! handler. Unsupported physical obligations refuse before replacing the VM.

use super::*;
use crate::journal::{Record, TransactionRecord, loop_protocol as protocol};

mod calls;

impl Vm<'_> {
    /// Restores Operator and outbound Call instances, timed joins, lifetimes,
    /// Closing, mailboxes/cursors, and command Loops from the committed journal prefix. No
    /// producer executes and no new record is appended during reconstruction.
    /// The explicitly supplied boot time is the resume instant; Loop deadlines
    /// come from retained `Advance` facts, never from that instant.
    ///
    /// A scheduler-owned Call closure needs the pure Contract projection from
    /// [`Self::recover_with_bindings`]. Event, inbound Call and Session
    /// profiles still require their complete exchange recovery fold.
    ///
    /// # Errors
    /// An unsupported recovery profile, an inconsistent committed prefix, or
    /// a journal read failure. On every error the original VM is unchanged.
    pub fn recover_from_journal(&mut self, journal: &dyn Journal) -> Result<(), VmError> {
        self.recover_with_bindings(journal, &mut NoOutboundBindings)
    }

    /// Reconstructs the same retained state, with Contract projections for
    /// scheduler-owned Call closures. Only `scheduler_completion` is used;
    /// recovery never prepares, starts, polls or cancels a physical effect.
    ///
    /// # Errors
    /// The same atomic recovery refusals as [`Self::recover_from_journal`],
    /// or an unavailable/invalid pure Contract projection.
    pub fn recover_with_bindings<B: OutboundConnectorBindings + ?Sized>(
        &mut self,
        journal: &dyn Journal,
        bindings: &mut B,
    ) -> Result<(), VmError> {
        self.preflight(journal)?;
        if self.plan.nodes.iter().any(|node| {
            node.connector()
                .is_some_and(|connector| connector.call_semantics().is_none())
        }) {
            return Err(VmError::RecoveryUnavailable {
                reason: "Event, inbound Call and Session obligations need their recovery fold",
            });
        }
        let resume_at = self.now;
        let mut candidate = Self::boot_at(self.plan, self.graph_version, 0);
        candidate.expected_deployment = self.expected_deployment.clone();
        candidate.unsupported_data_policy = self.unsupported_data_policy.clone();
        candidate.missing_deployment_binding = self.missing_deployment_binding.clone();
        let mut result = Ok(());
        let mut index = 0;
        let mut last_retained = None;
        let mut admissions = crate::journal::admission::AdmissionPrefix::default();
        journal
            .visit_records(&mut |record| {
                if result.is_ok() {
                    let instant = record
                        .timer_schedule()
                        .map(|schedule| schedule.armed_at())
                        .or_else(|| match record {
                            Record::Transaction(record) => record.logical_time,
                            Record::Outbound(crate::journal::OutboundRecord::Call(record))
                                if record.status == crate::journal::OutboundStatus::Open =>
                            {
                                Some(record.intent.logical_time)
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
                            .and_then(|()| admissions.apply(record).map_err(VmError::Journal))
                            .and_then(|()| candidate.promote_waiting(&admissions));
                        if result.is_ok() {
                            candidate.progress_closures();
                        }
                    }
                }
                index += 1;
            })
            .map_err(VmError::Journal)?;
        result?;
        candidate.dispatcher = Dispatcher::resume(journal);
        candidate.now = resume_at;
        *self = candidate;
        Ok(())
    }

    fn restore_record<B: OutboundConnectorBindings + ?Sized>(
        &mut self,
        journal: &dyn Journal,
        admissions: &impl InstanceAdmissions,
        bindings: &mut B,
        index: usize,
        record: &Record,
        resume_at: i64,
    ) -> Result<(), VmError> {
        match record {
            Record::Inbound(inbound) => {
                if inbound.connector_path.is_some() || inbound.exchange.is_some() {
                    return Err(invalid(
                        index,
                        "Connector ingress is not part of this recovery profile",
                    ));
                }
                if inbound.envelope.meta().sys().graph_version() != self.graph_version.0 {
                    return Err(invalid(
                        index,
                        "the ingress graph version differs from the deployed plan",
                    ));
                }
                if inbound
                    .timer_schedule
                    .iter()
                    .flat_map(|schedule| schedule.arms())
                    .any(|arm| {
                        !matches!(
                            arm.kind,
                            crate::journal::timer_kind::CALL_TIMEOUT
                                | crate::journal::timer_kind::JOIN_WITHIN
                                | crate::journal::timer_kind::INSTANCE_LIFETIME
                                | crate::journal::timer_kind::SESSION_HANDSHAKE
                        )
                    })
                {
                    return Err(invalid(
                        index,
                        "ingress carries a timer without an admitted obligation",
                    ));
                }
                let mut candidates = BTreeMap::new();
                let routes = self.prepare_routes(
                    admissions,
                    &EndpointRef {
                        node: None,
                        port: inbound.port.clone(),
                    },
                    None,
                    &inbound.envelope,
                    &mut candidates,
                )?;
                let mut expected = self.inbound_route_targets(&routes);
                expected.sort();
                let mut retained = inbound.routes.clone();
                retained.sort();
                if expected != retained {
                    return Err(invalid(
                        index,
                        "ingress routes disagree with the plan and instance addresses",
                    ));
                }
                self.check_room_for_routes(core::iter::once(routes.as_slice()))?;
                self.check_recovered_delivery_timers(
                    index,
                    inbound.timer_schedule.as_deref(),
                    &self.route_timer_arms(&routes, &candidates, &inbound.envelope),
                )?;
                self.route_prepared(routes, &candidates, &inbound.envelope, None);
                Ok(())
            }
            Record::Transaction(transaction) => {
                self.restore_transaction(journal, admissions, index, transaction, resume_at)?;
                if self.has_loops {
                    self.loop_manager
                        .get_or_insert_with(|| Box::new(protocol::Fold::default()))
                        .apply_transaction(transaction)
                        .map_err(|error| {
                            VmError::Journal(crate::journal::Error::InvalidLoopProtocol(error))
                        })?;
                }
                Ok(())
            }
            Record::Outbound(crate::journal::OutboundRecord::Call(record)) => {
                self.restore_call(journal, bindings, index, record)
            }
            Record::Outbound(_) => Err(invalid(
                index,
                "an outbound obligation requires exchange recovery",
            )),
        }
    }

    fn restore_transaction(
        &mut self,
        journal: &dyn Journal,
        admissions: &impl InstanceAdmissions,
        index: usize,
        record: &TransactionRecord,
        resume_at: i64,
    ) -> Result<(), VmError> {
        crate::journal::timing::validate_transaction(record)
            .map_err(|_| invalid(index, "retained deadlines contradict the accepting record"))?;
        if record
            .timer_schedule
            .iter()
            .flat_map(|schedule| schedule.arms())
            .any(|arm| {
                !matches!(
                    arm.kind,
                    crate::journal::timer_kind::CALL_TIMEOUT
                        | crate::journal::timer_kind::JOIN_WITHIN
                        | crate::journal::timer_kind::INSTANCE_LIFETIME
                        | crate::journal::timer_kind::LOOP_DELAY
                        | crate::journal::timer_kind::INSTANCE_DRAIN
                        | crate::journal::timer_kind::INSTANCE_SETTLEMENT
                ) || (matches!(
                    arm.kind,
                    crate::journal::timer_kind::INSTANCE_DRAIN
                        | crate::journal::timer_kind::INSTANCE_SETTLEMENT
                ) && record.close_scope.is_none())
            })
        {
            return Err(invalid(
                index,
                "a retained timer has no admitted transaction obligation",
            ));
        }
        if record.loop_fact().is_some()
            || record.exchange_exit.is_some()
            || record.timers.iter().any(|(kind, _)| {
                !matches!(
                    *kind,
                    crate::journal::timer_kind::LOOP_DELAY
                        | crate::journal::timer_kind::CALL_TIMEOUT
                        | crate::journal::timer_kind::JOIN_WITHIN
                        | crate::journal::timer_kind::INSTANCE_DRAIN
                        | crate::journal::timer_kind::INSTANCE_SETTLEMENT
                )
            })
        {
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
        if self.plan.owners[owner.index()].path != record.owner_path {
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
            node::Trigger::Lifecycle(node::Lifecycle::Next) if record.loop_protocol().is_some() => {
            }
            _ => {
                return Err(invalid(
                    index,
                    "the lifecycle transition needs a dedicated recovery fold",
                ));
            }
        }

        let slot = match self.plan.owners[owner.index()].kind {
            OwnerKind::Graph => {
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
                self.check_recovered_delivery_timers(index, record.timer_schedule.as_deref(), &[])?;
                self.restore_instance_closure(journal, index, record, None)?;
                if record.trigger == node::Trigger::Lifecycle(node::Lifecycle::Stop) {
                    self.discard_graph_admissions(&record.instance);
                    self.instances.remove(&record.instance);
                }
                return Ok(());
            }
            OwnerKind::Node(slot) => slot,
        };
        self.check_consumed_loop_context(index, slot, record)?;
        if self.graph_loops.is_some() {
            let mut derived = record.clone();
            if let Some(crate::journal::LoopRecord::R2(packet)) = derived.loop_record.as_deref_mut()
            {
                packet.produced.clear();
            }
            self.retain_graph_verdicts(slot, &mut derived)?;
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
        for consumed in record
            .consumed
            .iter()
            .filter(|_| record.trigger != node::Trigger::Lifecycle(node::Lifecycle::Stop))
        {
            if self.restore_call_consumption(admissions, index, slot, record, consumed)? {
                continue;
            }
            let delivery = self
                .scheduler
                .mailboxes
                .queued(slot, &record.instance)
                .and_then(|queue| {
                    queue.iter().find(|delivery| {
                        delivery.edge == consumed.edge
                            && delivery.source_instance == consumed.source
                            && delivery.seq == consumed.seq
                    })
                })
                .ok_or_else(|| invalid(index, "a consumed cursor has no pending delivery"))?
                .clone();
            if record.close_scope.is_some()
                && record.trigger == node::Trigger::Port(delivery.port.clone())
            {
                consumed_cause = Some(delivery.envelope.clone());
            }
            self.scheduler.mailboxes.remove_by_ref(
                slot,
                &record.instance,
                &delivery.edge,
                delivery.envelope.meta().sys().id(),
            );
        }
        if !record
            .state_delta
            .windows(2)
            .all(|pair| pair[0].0 < pair[1].0)
            || record.state_delta.iter().any(|(name, value)| {
                !self.plan.nodes[slot.index()]
                    .vars
                    .iter()
                    .any(|var| var.name == *name && var.ty.accepts(value))
            })
        {
            return Err(invalid(
                index,
                "a private-state delta violates the declared cells",
            ));
        }

        let prepared =
            self.validate_and_prepare_emissions(admissions, slot, &record.instance, record)?;
        if self.graph_loops.is_some()
            && record
                .loop_protocol()
                .map_or(&[][..], |packet| packet.routes.as_slice())
                != prepared.contexts
        {
            return Err(invalid(
                index,
                "routed contexts differ from the consumed Graph application",
            ));
        }
        let mut expected_arms = self.emission_timer_arms(&prepared);
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
                    if let Some(AttrSetting::Value(crate::value::Value::Duration(timeout))) =
                        self.plan.nodes[slot.index()]
                            .connector()
                            .and_then(|spec| spec.attribute("timeout"))
                    {
                        expected_arms.push(crate::journal::timing::TimerArm {
                            kind: crate::journal::timer_kind::CALL_TIMEOUT,
                            reference: ids::delivery_ref(
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
        if expected != record.routes {
            return Err(invalid(
                index,
                "the retained deliveries differ from the selected routes",
            ));
        }
        self.check_recovered_delivery_timers(
            index,
            record.timer_schedule.as_deref(),
            &expected_arms,
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
                    self.scheduler.arm(
                        *due_at,
                        DatedPayload::LoopDelay {
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
            let timer = expected_arms
                .last()
                .filter(|arm| arm.kind == crate::journal::timer_kind::CALL_TIMEOUT);
            if let Some(timer) = timer {
                self.scheduler.arm(
                    timer.due_at,
                    DatedPayload::DeliveryBudget {
                        connector: self.plan.nodes[slot.index()].path.clone(),
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
        slot: Slot,
        record: &TransactionRecord,
    ) -> Result<(), VmError> {
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
            let matches = |delivery: &&Delivery| {
                delivery.edge == consumed.edge
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
            let entry = self.plan.owners[self.plan.nodes[slot.index()].owner.index()]
                .loop_spec()
                .is_some_and(|spec| spec.entry == delivery.port);
            if entry && application.is_none() {
                return Err(invalid(
                    index,
                    "a Loop entry is consumed before its retained run admission",
                ));
            }
            let run = application.map(|(run, _)| run);
            if position == 0 {
                context.extend(run);
            } else if context.as_slice() != run.as_slice() {
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
    ) -> Result<(), VmError> {
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
                    InstancePhase::Closing { scope, .. } => Some(scope.clone()),
                    _ => None,
                })
                .ok_or_else(|| invalid(index, "a stop has no retained close request"))?;
            let drain_at = self.scheduler.dated.iter().find_map(|((at, _), payload)| {
                matches!(payload, DatedPayload::InstanceClosing { instance, deadline: ClosingDeadline::Drain, .. } if instance == &scope).then_some(*at)
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
                edge: delivery.edge.clone(),
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
    ) -> Result<(), VmError> {
        let Some(scope) = record.close_scope.as_ref() else {
            return Ok(());
        };
        if scope != &record.instance
            || !self
                .instances
                .get(scope)
                .is_some_and(|instance| matches!(instance.phase, InstancePhase::Open))
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
                        Record::Inbound(inbound)
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
            .map_err(VmError::Journal)?;
        let cause = cause
            .ok_or_else(|| invalid(index, "a close request has no earlier causal envelope"))?;
        let expected = self.closing_timer_arms(scope);
        let retained: Vec<_> = record
            .timer_schedule
            .iter()
            .flat_map(|schedule| schedule.arms())
            .filter(|arm| {
                matches!(
                    arm.kind,
                    crate::journal::timer_kind::INSTANCE_DRAIN
                        | crate::journal::timer_kind::INSTANCE_SETTLEMENT
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
        self.arm_closing_deadlines(scope, cause.meta().sys().id());
        Ok(())
    }

    pub(super) fn check_recovered_delivery_timers(
        &self,
        index: usize,
        schedule: Option<&crate::journal::timing::TimerSchedule>,
        expected: &[crate::journal::timing::TimerArm],
    ) -> Result<(), VmError> {
        let retained: Vec<_> = schedule
            .into_iter()
            .flat_map(|schedule| schedule.arms())
            .filter(|arm| {
                matches!(
                    arm.kind,
                    crate::journal::timer_kind::CALL_TIMEOUT
                        | crate::journal::timer_kind::JOIN_WITHIN
                        | crate::journal::timer_kind::SESSION_HANDSHAKE
                        | crate::journal::timer_kind::INSTANCE_LIFETIME
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

fn invalid(record: usize, reason: &'static str) -> VmError {
    VmError::InvalidRecovery { record, reason }
}
