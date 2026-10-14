//! Check manager transitions against the exact admission and deployed Loop,
//! in addition to the journal's independent protocol algebra.

use super::*;
use crate::instance;
use protocol::{Fact, OpeningAdmissionId, Phase};

impl Execution<'_> {
    pub(super) fn check_node_manager(
        &mut self,
        journal: &dyn Journal,
        index: usize,
        slot: NodeIndex,
        record: &TransactionRecord,
        resume_at: i64,
    ) -> Result<(), ExecutionFault> {
        let packet = record
            .loop_protocol()
            .ok_or_else(|| invalid(index, "a manager transition has no protocol"))?;
        let owner = &self.plan.nodes[slot.index()];
        let spec = owner
            .loop_spec()
            .ok_or_else(|| invalid(index, "a manager transition has no Loop host"))?;
        if packet.facts.len() != 1
            || !record.consumed.is_empty()
            || !record.state_delta.is_empty()
            || !record.join_inputs.is_empty()
            || record.parent_instance.is_some()
            || record.close_scope.is_some()
        {
            return Err(invalid(
                index,
                "a manager transition contains application work",
            ));
        }
        let fact = &packet.facts[0];
        if matches!(fact, Fact::Expired { .. })
            && !matches!(record.emissions.as_slice(), [emission]
            if spec.expired.as_ref().is_some_and(|target|
                target.port == emission.port
                    && emission.envelope.data().materialized_value()
                        .is_some_and(|value| target.ty.accepts(value))))
        {
            return Err(invalid(
                index,
                "Loop exhaustion changed its resolved host terminal",
            ));
        }
        let expected_context = match fact {
            Fact::Open {
                run,
                instance,
                opening,
                entry,
                fuel,
                state,
            } => {
                let OpeningAdmissionId::Delivery(reference) = opening else {
                    return Err(invalid(
                        index,
                        "an Executor Loop needs a spatial opening delivery",
                    ));
                };
                let delivery = self
                    .scheduler
                    .mailboxes
                    .queued(slot, &record.instance)
                    .into_iter()
                    .flatten()
                    .find(|delivery| {
                        ids::delivery_id(
                            self.scheduler.mailboxes.routes.text(delivery.edge),
                            delivery.envelope.meta().sys().id(),
                        ) == *reference
                    })
                    .ok_or_else(|| invalid(index, "a Loop Open has no exact queued admission"))?;
                let expected_fuel = match spec.fuel.as_ref().map(|fuel| &fuel.resolved) {
                    Some(crate::value::Value::Int(n)) if *n > 0 => Some(*n as u64),
                    Some(crate::value::Value::Uint(n)) if *n > 0 => Some(*n),
                    None => spec
                        .proof
                        .as_ref()
                        .map(|proof| {
                            super::super::loop_manager::entry_measure_budget(
                                proof,
                                &delivery.envelope,
                                instance,
                            )
                        })
                        .transpose()?,
                    _ => None,
                };
                if instance != &record.instance
                    || *run != protocol::run_ref(instance, opening)
                    || entry != &spec.entry
                    || entry != self.scheduler.mailboxes.routes.port(delivery.edge)
                    || expected_fuel != Some(*fuel)
                    || state != &delivery.envelope
                    || record.cause != Some(state.meta().sys().id())
                    || !packet.consumed.is_empty()
                    || !record.routes.is_empty()
                {
                    return Err(invalid(
                        index,
                        "a Loop Open contradicts its admission or plan bound",
                    ));
                }
                self.graph_context(delivery).to_vec()
            }
            _ => {
                let current = self
                    .loop_manager
                    .as_deref()
                    .and_then(|fold| fold.run(fact.run()))
                    .filter(|state| state.instance == record.instance)
                    .ok_or_else(|| invalid(index, "a manager transition has no retained run"))?
                    .clone();
                match fact {
                    Fact::Admit {
                        iteration,
                        admission,
                        envelope,
                        ..
                    } => {
                        if current.phase != Phase::AwaitingAdmission
                            || *iteration != current.iteration
                            || *admission != current.admission
                            || current.due_at.is_some_and(|due| due > resume_at)
                            || *envelope
                                != super::super::loop_manager::application_envelope(
                                    journal,
                                    self.graph_version,
                                    &current,
                                )?
                            || record.cause != Some(envelope.meta().sys().id())
                            || !packet.consumed.is_empty()
                        {
                            return Err(invalid(
                                index,
                                "a Loop Admit changes its state, identity or deadline",
                            ));
                        }
                        if let Some(due) = current.due_at {
                            self.now = self.now.max(due);
                        }
                    }
                    _ => {
                        let stopped = matches!(
                            fact,
                            Fact::Close {
                                reason: protocol::CloseReason::Stopped,
                                ..
                            }
                        );
                        if !stopped
                            && (current.phase != Phase::Active
                                || self.graph_run_has_work(fact.run()))
                        {
                            return Err(invalid(
                                index,
                                "a manager transition precedes application quiescence",
                            ));
                        }
                        if stopped {
                            if self
                                .instances
                                .get(&record.instance)
                                .is_some_and(|instance| {
                                    matches!(instance.phase, instance::Phase::Open)
                                })
                                || !packet.consumed.is_empty()
                            {
                                return Err(invalid(
                                    index,
                                    "a manager stop has no Closing authority",
                                ));
                            }
                        } else {
                            let command = current.pending.as_ref().ok_or_else(|| {
                                invalid(index, "a manager transition has no produced command")
                            })?;
                            if packet.consumed.as_slice() != [command.reference]
                                || record.cause != Some(command.value.cause())
                            {
                                return Err(invalid(
                                    index,
                                    "a manager transition changed its command cause",
                                ));
                            }
                        }
                        if let Fact::Advance {
                            to_iteration,
                            due_at,
                            ..
                        } = fact
                        {
                            let expected = self
                                .loop_delay_for(slot, *to_iteration)?
                                .map(|delay| self.now.saturating_add(delay));
                            if *due_at != expected {
                                return Err(invalid(
                                    index,
                                    "a Loop Advance changes its configured delay",
                                ));
                            }
                        }
                    }
                }
                current.enclosing.clone()
            }
        };
        if packet.context != expected_context
            || (!matches!(fact, Fact::Expired { .. }) && !record.emissions.is_empty())
        {
            return Err(invalid(
                index,
                "a manager transition changes its enclosing context or outputs",
            ));
        }
        Ok(())
    }
}
