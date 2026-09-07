//! Reconstruct committed deliveries and private state without firing a
//! handler. Unsupported physical obligations refuse before replacing the VM.

use super::*;
use crate::journal::{Record, TransactionRecord, loop_protocol as protocol};

impl Vm<'_> {
    /// Restores persistent Operator instances, their mailboxes/cursors, and
    /// command-based Operator Loops from the committed journal prefix. No
    /// producer executes and no new record is appended during reconstruction.
    /// The explicitly supplied boot time is the resume instant; Loop deadlines
    /// come from retained `Advance` facts, never from that instant.
    ///
    /// Connector obligations, timed joins and non-persistent lifetimes require
    /// their own retained recovery facts and are not admitted by this gate.
    ///
    /// # Errors
    /// An unsupported recovery profile, an inconsistent committed prefix, or
    /// a journal read failure. On every error the original VM is unchanged.
    pub fn recover_from_journal(&mut self, journal: &dyn Journal) -> Result<(), VmError> {
        self.preflight(journal)?;
        if self
            .plan
            .nodes
            .iter()
            .any(|node| !matches!(node.execution, NodeExecution::Operator))
        {
            return Err(VmError::RecoveryUnavailable {
                reason: "Connector obligations need the exchange recovery fold",
            });
        }
        if self
            .plan
            .owners
            .iter()
            .any(|owner| !matches!(owner.instance.lifetime, InstanceLifetime::Persistent))
        {
            return Err(VmError::RecoveryUnavailable {
                reason: "lifetime recovery needs retained admission and closing deadlines",
            });
        }
        if self.plan.handlers.iter().any(|handler| {
            matches!(
                handler.activation,
                node::HandlerActivation::Join {
                    within_ns: Some(_),
                    ..
                }
            )
        }) {
            return Err(VmError::RecoveryUnavailable {
                reason: "timed join recovery needs retained candidate deadlines",
            });
        }

        let mut candidate = Self::boot_at(self.plan, self.graph_version, self.now);
        candidate.expected_deployment = self.expected_deployment.clone();
        candidate.unsupported_data_policy = self.unsupported_data_policy.clone();
        candidate.missing_deployment_binding = self.missing_deployment_binding.clone();
        let mut result = Ok(());
        let mut index = 0;
        journal
            .visit_records(&mut |record| {
                if result.is_ok() {
                    result = candidate.restore_record(journal, index, record);
                }
                index += 1;
            })
            .map_err(VmError::Journal)?;
        result?;
        if let Some(fold) = candidate.loop_manager.as_deref() {
            for (run, state) in fold.runs() {
                if state.phase == protocol::Phase::AwaitingAdmission
                    && let Some(due_at) = state.due_at
                {
                    let owner = candidate
                        .instances
                        .get(&state.instance)
                        .ok_or_else(|| invalid(index, "a live Loop has no reconstructed instance"))?
                        .owner;
                    candidate.scheduler.arm(
                        due_at,
                        DatedPayload::LoopDelay {
                            owner,
                            instance: state.instance.clone(),
                            run: *run,
                            iteration: state.iteration,
                            state: state.state.meta().sys().id(),
                        },
                    );
                }
            }
        }
        *self = candidate;
        Ok(())
    }

    fn restore_record(
        &mut self,
        journal: &dyn Journal,
        index: usize,
        record: &Record,
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
                let mut candidates = BTreeMap::new();
                let routes = self.prepare_routes(
                    journal,
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
                self.route_prepared(routes, &candidates, &inbound.envelope, None);
                Ok(())
            }
            Record::Transaction(transaction) => {
                self.restore_transaction(journal, index, transaction)
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
        index: usize,
        record: &TransactionRecord,
    ) -> Result<(), VmError> {
        crate::journal::timing::validate_transaction(record)
            .map_err(|_| invalid(index, "retained deadlines contradict the accepting record"))?;
        if record.close_scope.is_some()
            || matches!(
                record.trigger,
                node::Trigger::Lifecycle(node::Lifecycle::Stop)
            )
        {
            return Err(VmError::RecoveryUnavailable {
                reason: "instance closure needs the generation and closing recovery fold",
            });
        }
        if record.loop_fact().is_some()
            || record.exchange_exit.is_some()
            || record
                .timers
                .iter()
                .any(|(kind, _)| *kind != crate::journal::timer_kind::LOOP_DELAY)
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
            node::Trigger::Lifecycle(node::Lifecycle::Start) => {
                let work = self
                    .lifecycle
                    .front()
                    .ok_or_else(|| invalid(index, "a lifecycle start has no admitted opening"))?;
                if work.instance != record.instance
                    || work.parent_instance != record.parent_instance
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
                if !record.consumed.is_empty()
                    || !record.state_delta.is_empty()
                    || !record.emissions.is_empty()
                    || !record.routes.is_empty()
                    || record.loop_protocol().is_some()
                {
                    return Err(invalid(
                        index,
                        "a passive Graph lifecycle carries executable work",
                    ));
                }
                return Ok(());
            }
            OwnerKind::Node(slot) => slot,
        };
        for consumed in &record.consumed {
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
            self.validate_and_prepare_emissions(journal, slot, &record.instance, record)?;
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
        if let Some(packet) = record.loop_protocol() {
            self.loop_manager
                .get_or_insert_with(|| Box::new(protocol::Fold::default()))
                .apply(packet)
                .map_err(|error| {
                    VmError::Journal(crate::journal::Error::InvalidLoopProtocol(error))
                })?;
        }
        self.state.apply_unchecked(
            self.plan.plan(),
            slot,
            &record.instance,
            &record.state_delta,
        );
        self.route_emissions(slot, &record.instance, prepared);
        if let Some((port, edge, envelope)) = recurrence {
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
        Ok(())
    }
}

fn invalid(record: usize, reason: &'static str) -> VmError {
    VmError::InvalidRecovery { record, reason }
}
