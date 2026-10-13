//! Delivery deadlines are selected from the same resolved targets before
//! acceptance and when installing the scheduler projection after commit.

use super::*;
use crate::instance;
use crate::journal::deadline::{Deadline, Deadlines};

impl Execution<'_> {
    pub(super) fn admission_deadlines(
        &self,
        target: &PreparedTarget,
        candidates: &BTreeMap<instance::Generation, InstanceCandidate>,
        envelope: &Envelope,
        activity: &mut BTreeMap<instance::Generation, Id>,
    ) -> Vec<(Deadline, Obligation)> {
        if !self.has_lifetime_deadlines {
            return Vec::new();
        }
        let mut deadlines = Vec::new();
        for reference in &target.admitted {
            let live = self.instances.get(reference);
            let waiting = self.waiting.get(reference);
            let owner = live
                .map(|instance| instance.owner)
                .or_else(|| waiting.map(|waiting| waiting.candidate.owner))
                .or_else(|| candidates.get(reference).map(|candidate| candidate.owner));
            let Some(owner) = owner else {
                continue;
            };
            let lifetime = &self.plan.owners[owner.index()].instance.lifetime;
            if matches!(
                lifetime,
                instance::Lifetime::Persistent | instance::Lifetime::SinglePass
            ) {
                continue;
            }
            let previous = activity
                .get(reference)
                .copied()
                .or_else(|| live.map(|instance| instance.last_activity.meta().sys().id()))
                .or_else(|| waiting.map(|waiting| waiting.last_activity.meta().sys().id()));
            let cause = envelope.meta().sys().id();
            activity.insert(reference.clone(), cause);
            let (due_at, deadline) = match lifetime {
                instance::Lifetime::Idle(after) if previous != Some(cause) => {
                    (self.now.saturating_add(*after), InstanceDeadline::Idle)
                }
                instance::Lifetime::Fixed(after) if previous.is_none() => {
                    (self.now.saturating_add(*after), InstanceDeadline::Fixed)
                }
                instance::Lifetime::Tumbling(every) if previous.is_none() => (
                    self.now
                        .div_euclid(*every)
                        .saturating_mul(*every)
                        .saturating_add(*every),
                    InstanceDeadline::Tumbling,
                ),
                _ => continue,
            };
            deadlines.push((
                Deadline {
                    kind: crate::journal::DeadlineKind::InstanceLifetime,
                    reference: ids::instance_lifetime_id(reference),
                    due_at,
                },
                Obligation::InstanceLifetime {
                    owner,
                    instance: reference.clone(),
                    cause,
                    deadline,
                },
            ));
        }
        deadlines
    }

    pub(super) fn delivery_deadlines(
        &self,
        route: &PreparedEdge,
        target: &PreparedTarget,
        candidates: &BTreeMap<instance::Generation, InstanceCandidate>,
        envelope: &Envelope,
    ) -> Vec<(Deadline, Obligation)> {
        let Some(to) = &route.edge.to else {
            return Vec::new();
        };
        let Some(slot) = to.node else {
            return Vec::new();
        };
        let node = &self.plan.nodes[slot.index()];
        // A Graph-boundary admission retains its own source path; its
        // executable projection alone cannot identify the delivery deadline.
        let edge = self.prepared_edge_text(route);
        let delivery = envelope.meta().sys().id();
        let reference = ids::delivery_id(&edge, delivery);
        let mut deadlines = Vec::new();
        let within = self.plan.handlers.iter().find_map(|handler| {
            (handler.owner == slot)
                .then_some(&handler.activation)
                .and_then(|activation| match activation {
                    node::HandlerActivation::Join {
                        ports, within_ns, ..
                    } if ports.contains(&to.port) => *within_ns,
                    _ => None,
                })
        });
        if let Some(within) = within {
            deadlines.push((
                Deadline {
                    kind: crate::journal::DeadlineKind::JoinWithin,
                    reference,
                    due_at: self.now.saturating_add(within),
                },
                Obligation::JoinWithin {
                    operator: node.path.clone(),
                    edge: edge.clone(),
                    delivery,
                    instance: target.reference.clone(),
                },
            ));
        }
        let settings = self
            .instances
            .get(&target.reference)
            .map(|instance| &instance.settings)
            .or_else(|| {
                self.waiting
                    .get(&target.reference)
                    .map(|waiting| &waiting.candidate.settings)
            })
            .or_else(|| {
                candidates
                    .get(&target.reference)
                    .map(|candidate| &candidate.settings)
            });
        let dynamic = settings
            .and_then(|settings| {
                settings
                    .get("timeout")
                    .or_else(|| settings.get("handshake_timeout"))
            })
            .and_then(|value| match value {
                crate::value::Value::Duration(ns) => Some(*ns),
                _ => None,
            });
        let static_budget = node
            .connector()
            .and_then(|spec| {
                spec.attribute("timeout")
                    .or_else(|| spec.attribute("handshake_timeout"))
            })
            .and_then(|setting| match setting {
                AttrSetting::Value(crate::value::Value::Duration(ns)) => Some(*ns),
                _ => None,
            });
        if let Some(budget) = dynamic.or(static_budget) {
            let handshake = node.connector().is_some_and(|spec| {
                spec.attribute("timeout").is_none() && spec.attribute("handshake_timeout").is_some()
            });
            deadlines.push((
                Deadline {
                    kind: if handshake {
                        crate::journal::DeadlineKind::SessionHandshake
                    } else {
                        crate::journal::DeadlineKind::CallTimeout
                    },
                    reference,
                    due_at: self.now.saturating_add(budget),
                },
                Obligation::DeliveryBudget {
                    connector: node.path.clone(),
                    edge,
                    delivery,
                    instance: target.reference.clone(),
                },
            ));
        }
        deadlines
    }

    pub(super) fn route_deadlines(
        &self,
        routes: &[PreparedEdge],
        candidates: &BTreeMap<instance::Generation, InstanceCandidate>,
        envelope: &Envelope,
    ) -> Vec<Deadline> {
        self.route_deadlines_with_activity(routes, candidates, envelope, &mut BTreeMap::new())
    }

    pub(super) fn route_deadlines_with_activity(
        &self,
        routes: &[PreparedEdge],
        candidates: &BTreeMap<instance::Generation, InstanceCandidate>,
        envelope: &Envelope,
        activity: &mut BTreeMap<instance::Generation, Id>,
    ) -> Vec<Deadline> {
        let mut deadlines = Vec::new();
        for route in routes {
            if let Some(target) = &route.target {
                deadlines.extend(
                    self.admission_deadlines(target, candidates, envelope, activity)
                        .into_iter()
                        .map(|(deadline, _)| deadline),
                );
                deadlines.extend(
                    self.delivery_deadlines(route, target, candidates, envelope)
                        .into_iter()
                        .map(|(deadline, _)| deadline),
                );
            }
        }
        deadlines
    }

    pub(super) fn emission_deadlines(&self, prepared: &PreparedEmissions) -> Vec<Deadline> {
        let mut activity = BTreeMap::new();
        prepared
            .emissions
            .iter()
            .flat_map(|emission| {
                self.route_deadlines_with_activity(
                    &emission.routes,
                    &prepared.candidates,
                    &emission.emission.envelope,
                    &mut activity,
                )
            })
            .collect()
    }

    pub(super) fn retain_emission_routes(
        &self,
        record: &mut crate::journal::TransactionRecord,
        prepared: &PreparedEmissions,
    ) -> Result<(), ExecutionFault> {
        record.routes = self.routed_deliveries(prepared);
        record.instance_commands = Self::retained_commands(&prepared.commands);
        if let Some(packet) = record.loop_record.as_deref_mut() {
            packet.routes = prepared.contexts.clone();
        }
        let mut deadlines = record
            .close_scope
            .as_ref()
            .map(|instance| self.closing_deadlines(instance))
            .unwrap_or_default();
        deadlines.extend(self.emission_deadlines(prepared));
        self.retain_deadlines(record, deadlines)
    }

    pub(super) fn closing_deadlines(&self, reference: &instance::Generation) -> Vec<Deadline> {
        let Some(instance) = self.instances.get(reference) else {
            return Vec::new();
        };
        let Some(policy) = self.plan.owners[instance.owner.index()].instance.closing else {
            return Vec::new();
        };
        let drain = self.now.saturating_add(policy.drain_for);
        [
            (crate::journal::DeadlineKind::InstanceDrain, drain),
            (
                crate::journal::DeadlineKind::InstanceSettlement,
                drain.saturating_add(policy.settle_for),
            ),
        ]
        .into_iter()
        .map(|(kind, due_at)| Deadline {
            kind,
            reference: ids::instance_closing_id(reference, kind),
            due_at,
        })
        .collect()
    }

    pub(super) fn retain_deadlines(
        &self,
        record: &mut crate::journal::TransactionRecord,
        deadlines: Vec<Deadline>,
    ) -> Result<(), ExecutionFault> {
        if deadlines.is_empty() {
            return Ok(());
        }
        let mut combined = record
            .deadlines
            .as_ref()
            .map(|schedule| schedule.items().to_vec())
            .unwrap_or_default();
        combined.extend(deadlines);
        record.deadlines = Some(Box::new(Self::create_deadlines(self.now, combined)?));
        Ok(())
    }

    pub(super) fn create_deadlines(
        at: i64,
        deadlines: Vec<Deadline>,
    ) -> Result<Deadlines, ExecutionFault> {
        Deadlines::try_new(at, deadlines).map_err(|error| {
            ExecutionFault::Journal(crate::journal::Error::InvalidDeadlines(error))
        })
    }
}
