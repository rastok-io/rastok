//! Command-driven Loop applications. The cached fold is replaced only from
//! committed journal facts; preparation never consumes a command or a cursor.

use super::*;
use crate::journal::loop_protocol::{
    self as protocol, CloseReason, CommandValue, Fact, Packet, Phase,
};

impl Vm<'_> {
    /// A delayed run still owns its future command slot. Count retained runs
    /// plus opening deliveries not yet folded into Open; neither an empty
    /// host mailbox nor moving a command to the timer releases that capacity.
    pub(super) fn loop_admission_load(&self) -> usize {
        let fold = self.loop_manager.as_deref();
        let live = |state: &protocol::RunState| {
            matches!(state.phase, Phase::Active | Phase::AwaitingAdmission)
        };
        let mut occupied = fold.map_or(0, |fold| {
            fold.runs().filter(|(_, state)| live(state)).count()
        });
        for ((slot, instance), queue) in &self.scheduler.mailboxes.queues {
            let Some(spec) =
                self.plan.owners[self.plan.nodes[slot.index()].owner.index()].loop_spec()
            else {
                continue;
            };
            for delivery in queue.iter().filter(|delivery| delivery.port == spec.entry) {
                let reference =
                    ids::delivery_ref(&delivery.edge, delivery.envelope.meta().sys().id());
                let retained = fold.is_some_and(|fold| {
                    fold.runs().any(|(run, state)| {
                        live(state)
                            && state.instance == *instance
                            && ((state.iteration == 0 && state.admission == reference)
                                || (state.iteration > 0
                                    && state.admission == delivery.envelope.meta().sys().id()
                                    && delivery.edge == manager_edge(*run, &state.entry)))
                    })
                });
                if !retained {
                    occupied = occupied.saturating_add(1);
                }
            }
        }
        occupied
    }

    pub(super) fn close_instance_loops(
        &self,
        instance: &InstanceRef,
        record: &mut crate::journal::TransactionRecord,
    ) -> Result<(), VmError> {
        let Some(fold) = self.loop_manager.as_deref() else {
            return Ok(());
        };
        let facts: Vec<_> = fold
            .runs()
            .filter_map(|(run, state)| {
                (state.instance == *instance
                    && !matches!(state.phase, Phase::Closed | Phase::Expired))
                .then_some(Fact::Close {
                    run: *run,
                    iteration: state.iteration,
                    reason: CloseReason::Stopped,
                })
            })
            .collect();
        if !facts.is_empty() {
            let packet = record
                .loop_record
                .get_or_insert_with(|| Box::new(crate::journal::LoopRecord::R2(Packet::default())));
            let crate::journal::LoopRecord::R2(packet) = packet.as_mut() else {
                return Err(loop_error(
                    instance,
                    "an instance closure mixed Loop record domains",
                ));
            };
            packet.facts.extend(facts);
        }
        Ok(())
    }

    pub(super) fn refresh_loop_manager(&mut self, journal: &dyn Journal) -> Result<(), VmError> {
        if self.has_loops {
            self.loop_manager = Some(Box::new(journal.loop_protocol().map_err(VmError::Journal)?));
        }
        Ok(())
    }

    pub(super) fn loop_manager_ready(&self) -> bool {
        self.loop_manager.as_deref().is_some_and(|fold| {
            fold.runs().any(|(_, state)| {
                state.pending.is_some()
                    || (state.phase == Phase::AwaitingAdmission
                        && state.due_at.is_none_or(|due| due <= self.now))
            })
        })
    }

    pub(super) fn loop_application(
        &self,
        instance: &InstanceRef,
        delivery: &Delivery,
    ) -> Option<(LoopRunRef, &protocol::RunState)> {
        let fold = self.loop_manager.as_deref()?;
        let reference = ids::delivery_ref(&delivery.edge, delivery.envelope.meta().sys().id());
        fold.runs().find_map(|(run, state)| {
            (state.instance == *instance
                && state.phase == Phase::Active
                && ((state.iteration == 0 && state.admission == reference)
                    || (state.iteration > 0
                        && state.admission == delivery.envelope.meta().sys().id()
                        && delivery.edge == manager_edge(*run, &state.entry))))
            .then_some((*run, state))
        })
    }

    pub(super) fn open_loop_application(
        &mut self,
        journal: &mut dyn Journal,
        slot: Slot,
        instance: &InstanceRef,
        delivery: &Delivery,
    ) -> Result<bool, VmError> {
        let owner = &self.plan.owners[self.plan.nodes[slot.index()].owner.index()];
        let Some(spec) = owner.loop_spec() else {
            return Ok(false);
        };
        if spec.entry != delivery.port || self.loop_application(instance, delivery).is_some() {
            return Ok(false);
        }
        let opening = protocol::OpeningAdmissionRef::Delivery(ids::delivery_ref(
            &delivery.edge,
            delivery.envelope.meta().sys().id(),
        ));
        let run = protocol::run_ref(instance, &opening);
        if let Some(existing) = self.loop_manager.as_deref().and_then(|fold| fold.run(run)) {
            if existing.phase == Phase::AwaitingAdmission && existing.iteration == 0 {
                return Ok(false);
            }
            return Err(loop_error(
                instance,
                "the opening delivery has already been admitted",
            ));
        }
        let fuel = match spec.fuel.as_ref().map(|fuel| &fuel.resolved) {
            Some(crate::value::Value::Int(value)) if *value > 0 => u64::try_from(*value).ok(),
            Some(crate::value::Value::Uint(value)) if *value > 0 => Some(*value),
            None => spec
                .proof
                .as_ref()
                .map(|proof| entry_measure_budget(proof, &delivery.envelope, instance))
                .transpose()?,
            _ => None,
        }
        .ok_or_else(|| {
            loop_error(
                instance,
                "this Loop application needs positive fuel or a validated measure",
            )
        })?;
        let record = manager_record(
            instance,
            delivery.envelope.meta().sys().id(),
            Packet {
                facts: vec![Fact::Open {
                    run,
                    instance: instance.clone(),
                    opening,
                    entry: spec.entry.clone(),
                    fuel,
                    state: delivery.envelope.clone(),
                }],
                ..Packet::default()
            },
        );
        journal.commit(record).map_err(VmError::Journal)?;
        self.refresh_loop_manager(journal)?;
        Ok(true)
    }

    // Queue time belongs to the Call budget. An entry may time out before it
    // reaches the runnable head, but its terminal still belongs to its own
    // Loop application. Retain that admission before closing the queued Call;
    // retrying an interrupted expiry cannot mint another run or regain fuel.
    pub(super) fn admit_expiring_call_loop(
        &mut self,
        journal: &mut dyn Journal,
        slot: Slot,
        instance: &InstanceRef,
        edge: &str,
        envelope: Id,
    ) -> Result<(), VmError> {
        if !self.has_loops
            || self.plan.owners[self.plan.nodes[slot.index()].owner.index()]
                .loop_spec()
                .is_none()
        {
            return Ok(());
        }
        let delivery = self
            .scheduler
            .mailboxes
            .find_by_ref(slot, instance, edge, envelope)
            .ok_or_else(|| loop_error(instance, "an expiring Loop entry lost its queued delivery"))?
            .clone();
        self.open_loop_application(journal, slot, instance, &delivery)?;
        if self.loop_application(instance, &delivery).is_some() {
            return Ok(());
        }
        let opening = protocol::OpeningAdmissionRef::Delivery(ids::delivery_ref(edge, envelope));
        let run = protocol::run_ref(instance, &opening);
        let record = manager_record(
            instance,
            envelope,
            Packet {
                facts: vec![Fact::Admit {
                    run,
                    iteration: 0,
                    admission: ids::delivery_ref(edge, envelope),
                    envelope: delivery.envelope,
                }],
                ..Packet::default()
            },
        );
        journal.commit(record).map_err(VmError::Journal)?;
        self.refresh_loop_manager(journal)?;
        Ok(())
    }

    pub(super) fn prepare_operator_loop_verdict(
        &self,
        slot: Slot,
        record: &mut crate::journal::TransactionRecord,
    ) -> Result<(), VmError> {
        let Some(crate::journal::LoopRecord::R2(packet)) = record.loop_record.as_deref_mut() else {
            return Ok(());
        };
        let owner = &self.plan.owners[self.plan.nodes[slot.index()].owner.index()];
        let spec = owner.loop_spec().ok_or_else(|| {
            loop_error(
                &record.instance,
                "a Loop application lost its specification",
            )
        })?;
        let run = *packet
            .context
            .last()
            .ok_or_else(|| loop_error(&record.instance, "a Loop verdict has no run context"))?;
        let current = self
            .loop_manager
            .as_deref()
            .and_then(|fold| fold.run(run))
            .ok_or_else(|| loop_error(&record.instance, "a Loop verdict names no active run"))?;
        let terminal = record
            .emissions
            .iter()
            .find(|emission| spec.terminals.contains(&emission.port));
        let stopped = record.close_scope.is_some();
        if packet.produced.len() > 1
            || (!packet.produced.is_empty() && (terminal.is_some() || stopped))
            || (terminal.is_some() && stopped)
        {
            return Err(loop_error(
                &record.instance,
                "LoopProtocolViolation: one application selected competing verdicts",
            ));
        }
        if let Some(command) = packet.produced.first() {
            if let CommandValue::Next(state) = &command.value {
                let expected = self
                    .plan
                    .resolved_ports()
                    .node_port_type(slot, &spec.entry)
                    .ok_or_else(|| {
                        loop_error(&record.instance, "the Loop entry type is unavailable")
                    })?;
                if let Data::Materialized(value) = state.data()
                    && !expected.accepts(value)
                {
                    return Err(loop_error(
                        &record.instance,
                        "Loop.next contradicts the recurrent input type",
                    ));
                }
            }
        } else if let Some(terminal) = terminal {
            packet.produced.push(protocol::CommandProduced::new(
                run,
                current.iteration,
                CommandValue::Done {
                    cause: terminal.envelope.meta().sys().id(),
                    kind: protocol::DoneKind::Natural,
                },
            ));
        } else if stopped {
            packet.facts.push(Fact::Close {
                run,
                iteration: current.iteration,
                reason: CloseReason::Stopped,
            });
        } else {
            return Err(loop_error(
                &record.instance,
                "LoopNoVerdict: the application ended without Next, Again, Done, or stop",
            ));
        }
        Ok(())
    }

    pub(super) fn prepare_call_completion(
        &self,
        slot: Slot,
        instance: &InstanceRef,
        delivery: &Delivery,
        record: &mut crate::journal::TransactionRecord,
    ) -> Result<(), VmError> {
        let terminal = self.validate_call_completion(slot, record)?;
        let Some((run, application)) = self.loop_application(instance, delivery) else {
            return Ok(());
        };
        let node = &self.plan.nodes[slot.index()];
        let terminal = terminal
            .ok_or_else(|| loop_error(instance, "the Call Loop lost its Contract profile"))?;
        let violation = |reason| VmError::ContractViolation {
            node: node.path.clone(),
            reason,
        };
        let mut packet = Packet {
            context: vec![run],
            ..Packet::default()
        };
        for emission in &record.emissions {
            for rule in self.plan.routes.iter().filter(|rule| {
                matches!(&rule.source.owner, crate::plan::EndpointOwner::Node(path) if *path == node.path)
                    && rule.source.port == emission.port
            }) {
                use crate::plan::{LoopCommandKind, RouteDestination, RouteSelection};
                let is_command = |destination: &RouteDestination| {
                    matches!(destination, RouteDestination::LoopCommand(_))
                };
                let destinations = match &rule.selection {
                    RouteSelection::All(destinations) => destinations.as_slice(),
                    RouteSelection::Cases(arms) => {
                        if !arms.iter().flat_map(|arm| &arm.destinations).any(is_command) {
                            continue;
                        }
                        // Only a case decision demands DATA. An unconditional
                        // command carries the opaque envelope or its cause.
                        let value = crate::runtime::data_value(emission.envelope.data())
                            .map_err(VmError::Fault)?;
                        rule.selection.selected(&value).ok_or_else(|| {
                            violation("a Call output has no selected route case")
                        })?
                    }
                };
                for destination in destinations {
                    let RouteDestination::LoopCommand(command) = destination else {
                        continue;
                    };
                    let value = match command.kind {
                        LoopCommandKind::Next => CommandValue::Next(emission.envelope.clone()),
                        LoopCommandKind::Again => CommandValue::Again(emission.envelope.meta().sys().id()),
                        LoopCommandKind::Done => CommandValue::Done {
                            cause: emission.envelope.meta().sys().id(),
                            kind: protocol::DoneKind::Explicit,
                        },
                    };
                    packet.produced.push(protocol::CommandProduced::new(run, application.iteration, value));
                }
            }
        }
        if packet.produced.len() > 1 {
            return Err(violation(
                "a Call completion selected competing Loop verdicts",
            ));
        }
        if packet.produced.is_empty() {
            packet.produced.push(protocol::CommandProduced::new(
                run,
                application.iteration,
                CommandValue::Done {
                    cause: terminal.envelope.meta().sys().id(),
                    kind: protocol::DoneKind::Natural,
                },
            ));
        }
        record.loop_record = Some(Box::new(crate::journal::LoopRecord::R2(packet)));
        Ok(())
    }

    pub(super) fn step_loop_manager(&mut self, journal: &mut dyn Journal) -> Result<bool, VmError> {
        let selected = self.loop_manager.as_deref().and_then(|fold| {
            fold.runs().find_map(|(run, state)| {
                (state.pending.is_some()
                    || (state.phase == Phase::AwaitingAdmission
                        && state.due_at.is_none_or(|due| due <= self.now)))
                .then(|| (*run, state.clone()))
            })
        });
        let Some((run, current)) = selected else {
            return Ok(false);
        };
        let (slot, owner) = self
            .plan
            .nodes
            .iter()
            .enumerate()
            .find_map(|(index, node)| {
                (node.path.segments() == current.instance.address().owner().segments()).then(|| {
                    (
                        Slot::new(u32::try_from(index).unwrap_or(u32::MAX)),
                        node.owner,
                    )
                })
            })
            .ok_or_else(|| {
                loop_error(&current.instance, "the Loop host is not an executable Node")
            })?;
        if !matches!(
            self.instances
                .get(&current.instance)
                .map(|instance| &instance.phase),
            Some(InstancePhase::Open)
        ) {
            let record = manager_record(
                &current.instance,
                current.state.meta().sys().id(),
                Packet {
                    facts: vec![Fact::Close {
                        run,
                        iteration: current.iteration,
                        reason: CloseReason::Stopped,
                    }],
                    ..Packet::default()
                },
            );
            journal.commit(record).map_err(VmError::Journal)?;
            self.refresh_loop_manager(journal)?;
            return Ok(true);
        }
        if current.phase == Phase::AwaitingAdmission {
            if current.iteration > 0
                && !self
                    .scheduler
                    .mailboxes
                    .has_room(slot, &current.instance, 0)
            {
                return Ok(false);
            }
            let envelope = match current.reentry_cause {
                None => current.state.clone(),
                Some(cause) => {
                    let source = journal.find_envelope(cause).ok_or_else(|| {
                        loop_error(
                            &current.instance,
                            "the recurrence command lost its causal envelope",
                        )
                    })?;
                    Envelope::new(
                        Meta::new(MetaSys::emission(
                            current.admission,
                            source.meta().sys(),
                            self.graph_version.0,
                        ))
                        .with_app(current.state.meta().app().clone())
                        .with_ext(current.state.meta().ext().clone()),
                        current.state.data().clone(),
                    )
                }
            };
            let mut record = manager_record(
                &current.instance,
                envelope.meta().sys().id(),
                Packet {
                    facts: vec![Fact::Admit {
                        run,
                        iteration: current.iteration,
                        admission: current.admission,
                        envelope: envelope.clone(),
                    }],
                    ..Packet::default()
                },
            );
            let edge = manager_edge(run, &current.entry);
            let call_timeout = self.plan.nodes[slot.index()]
                .connector()
                .and_then(|connector| match connector.attribute("timeout") {
                    Some(AttrSetting::Value(crate::value::Value::Duration(ns))) => Some(*ns),
                    _ => None,
                });
            if current.iteration > 0 {
                record.routes.push(RoutedDelivery {
                    delivery: envelope.meta().sys().id(),
                    edge: edge.clone(),
                    target: current.instance.clone(),
                });
                if call_timeout.is_some() {
                    record.timers.push((
                        crate::journal::timer_kind::CALL_TIMEOUT,
                        ids::delivery_ref(&edge, envelope.meta().sys().id()),
                    ));
                }
                if let Some(timeout) = call_timeout {
                    self.retain_timer_arms(
                        &mut record,
                        vec![crate::journal::timing::TimerArm {
                            kind: crate::journal::timer_kind::CALL_TIMEOUT,
                            reference: ids::delivery_ref(&edge, envelope.meta().sys().id()),
                            due_at: self.now.saturating_add(timeout),
                        }],
                    )?;
                }
            }
            journal.commit(record).map_err(VmError::Journal)?;
            if current.iteration > 0 {
                self.scheduler
                    .mailboxes
                    .deliver(
                        slot,
                        &current.instance,
                        &current.instance,
                        current.entry,
                        edge.clone(),
                        envelope.clone(),
                    )
                    .map_err(|_| {
                        loop_error(
                            &current.instance,
                            "a pre-reserved recurrence mailbox became full",
                        )
                    })?;
                if let Some(timeout) = call_timeout {
                    self.scheduler.arm(
                        self.now.saturating_add(timeout),
                        DatedPayload::DeliveryBudget {
                            connector: self.plan.nodes[slot.index()].path.clone(),
                            edge,
                            delivery: envelope.meta().sys().id(),
                            instance: current.instance.clone(),
                        },
                    );
                }
            }
            self.refresh_loop_manager(journal)?;
            return Ok(true);
        }
        let command = current.pending.as_ref().ok_or_else(|| {
            loop_error(&current.instance, "the manager selected no pending verdict")
        })?;
        let mut packet = Packet {
            consumed: vec![command.reference],
            ..Packet::default()
        };
        let mut emission = None;
        let mut due = None;
        match &command.value {
            CommandValue::Done { kind, .. } => packet.facts.push(Fact::Close {
                run,
                iteration: current.iteration,
                reason: match kind {
                    protocol::DoneKind::Natural => CloseReason::Natural,
                    protocol::DoneKind::Explicit => CloseReason::ExplicitDone,
                },
            }),
            CommandValue::Next(_) | CommandValue::Again(_) => {
                let state = match &command.value {
                    CommandValue::Next(state) => state.clone(),
                    CommandValue::Again(_) => current.state.clone(),
                    CommandValue::Done { .. } => {
                        return Err(loop_error(
                            &current.instance,
                            "a terminal verdict selected recurrence",
                        ));
                    }
                };
                if current.remaining > 1 {
                    let iteration = current
                        .iteration
                        .checked_add(1)
                        .ok_or_else(|| loop_error(&current.instance, "Loop ordinal overflow"))?;
                    due = self
                        .loop_delay_for(owner, iteration)?
                        .map(|delay| self.now.saturating_add(delay));
                    packet.facts.push(Fact::Advance {
                        run,
                        from_iteration: current.iteration,
                        to_iteration: iteration,
                        remaining: current.remaining - 1,
                        command: command.reference,
                        state,
                        due_at: due,
                    });
                } else {
                    let port = Self::loop_port(current.instance.address().owner(), "Loop.expired")?;
                    let source = journal
                        .find_envelope(command.value.cause())
                        .ok_or_else(|| {
                            loop_error(
                                &current.instance,
                                "the expiry command lost its causal envelope",
                            )
                        })?;
                    let value = crate::value::Value::TaggedUnion(Box::new(
                        crate::value::TaggedUnionValue {
                            case: String::from("LoopExpired"),
                            payload: vec![(
                                String::from("last_state"),
                                crate::runtime::data_value(state.data()).map_err(VmError::Fault)?,
                            )],
                        },
                    ));
                    let id = ids::graph_emission_id(
                        current.instance.address().owner(),
                        &node::Trigger::Lifecycle(node::Lifecycle::Next),
                        source.meta().sys().id(),
                        &port,
                        0,
                    );
                    let envelope = Envelope::new(
                        Meta::new(MetaSys::emission(
                            id,
                            source.meta().sys(),
                            self.graph_version.0,
                        ))
                        .with_app(state.meta().app().clone())
                        .with_ext(state.meta().ext().clone()),
                        Data::materialized(value),
                    );
                    packet.facts.push(Fact::Expired {
                        run,
                        iteration: current.iteration,
                        command: command.reference,
                        last_state: state,
                        output: envelope.clone(),
                    });
                    emission = Some(Emission {
                        port,
                        index: 0,
                        envelope,
                    });
                }
            }
        }
        let mut record = manager_record(&current.instance, command.value.cause(), packet);
        record.emissions.extend(emission);
        if let Some(due_at) = due {
            let timer = (
                crate::journal::timer_kind::LOOP_DELAY,
                protocol::admission_ref(run, current.iteration + 1, command.reference),
            );
            record.timers.push(timer);
            let schedule = crate::journal::timing::TimerSchedule::try_new(
                self.now,
                vec![crate::journal::timing::TimerArm {
                    kind: timer.0,
                    reference: timer.1,
                    due_at,
                }],
            )
            .map_err(|error| {
                VmError::Journal(crate::journal::Error::InvalidTimerSchedule(error))
            })?;
            record.timer_schedule = Some(Box::new(schedule));
        }
        let prepared =
            self.validate_and_prepare_emissions(journal, slot, &current.instance, &record)?;
        self.retain_emission_routes(&mut record, &prepared)?;
        journal.commit(record).map_err(VmError::Journal)?;
        self.route_emissions(slot, &current.instance, prepared);
        if let Some(at) = due {
            self.scheduler.arm(
                at,
                DatedPayload::LoopDelay {
                    owner,
                    instance: current.instance.clone(),
                    run,
                    iteration: current.iteration + 1,
                    state: protocol::admission_ref(run, current.iteration + 1, command.reference),
                },
            );
        }
        self.refresh_loop_manager(journal)?;
        Ok(true)
    }
}

fn entry_measure_budget(
    proof: &crate::loop_proof::Certificate,
    envelope: &Envelope,
    instance: &InstanceRef,
) -> Result<u64, VmError> {
    let invalid = || {
        loop_error(
            instance,
            "the admitted Loop state contradicts its direct integer measure",
        )
    };
    let crate::expr::Expr::Path(path) = &proof.measure.root else {
        return Err(invalid());
    };
    let path = match path.as_slice() {
        [root, state, data, ..] if root == "Loop" && state == "state" && data == "data" => {
            &path[3..]
        }
        [root, ..] if root == "data" => &path[1..],
        _ => return Err(invalid()),
    };
    let value = crate::runtime::data_value(envelope.data()).map_err(VmError::Fault)?;
    let mut selected = &value;
    for field in path {
        let crate::value::Value::Struct(fields) = selected else {
            return Err(invalid());
        };
        selected = fields
            .iter()
            .find(|(name, _)| name == field)
            .map(|(_, value)| value)
            .ok_or_else(invalid)?;
    }
    let crate::value::Value::Int(measure) = selected else {
        return Err(invalid());
    };
    Ok(u64::try_from(*measure).unwrap_or(0) + 1)
}

pub(super) fn manager_edge(run: LoopRunRef, entry: &node::Port) -> String {
    format!("@loop:{run}:{entry}")
}

fn loop_error(instance: &InstanceRef, reason: &'static str) -> VmError {
    VmError::InvalidLoopDecision {
        owner: instance.address().owner().clone(),
        reason,
    }
}

fn manager_record(
    instance: &InstanceRef,
    cause: Id,
    packet: Packet,
) -> crate::journal::TransactionRecord {
    crate::journal::TransactionRecord {
        owner_path: instance.address().owner().clone(),
        instance: instance.clone(),
        parent_instance: None,
        close_scope: None,
        exchange_exit: None,
        loop_record: Some(Box::new(crate::journal::LoopRecord::R2(packet))),
        logical_time: None,
        timer_schedule: None,
        trigger: node::Trigger::Lifecycle(node::Lifecycle::Next),
        cause: Some(cause),
        consumed: vec![],
        join_inputs: vec![],
        state_delta: vec![],
        emissions: vec![],
        routes: vec![],
        timers: vec![],
    }
}
