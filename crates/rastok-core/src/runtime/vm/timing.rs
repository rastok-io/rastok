//! Delivery deadlines are selected from the same resolved targets before
//! acceptance and when installing the scheduler projection after commit.

use super::*;
use crate::journal::timing::{TimerArm, TimerSchedule};

impl Vm<'_> {
    pub(super) fn admission_deadlines(
        &self,
        target: &PreparedTarget,
        candidates: &BTreeMap<InstanceRef, InstanceCandidate>,
        envelope: &Envelope,
        activity: &mut BTreeMap<InstanceRef, Id>,
    ) -> Vec<(TimerArm, DatedPayload)> {
        if !self.has_lifetime_deadlines {
            return Vec::new();
        }
        let mut arms = Vec::new();
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
                InstanceLifetime::Persistent | InstanceLifetime::SinglePass
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
                InstanceLifetime::Idle(after) if previous != Some(cause) => {
                    (self.now.saturating_add(*after), InstanceDeadline::Idle)
                }
                InstanceLifetime::Fixed(after) if previous.is_none() => {
                    (self.now.saturating_add(*after), InstanceDeadline::Fixed)
                }
                InstanceLifetime::Tumbling(every) if previous.is_none() => (
                    self.now
                        .div_euclid(*every)
                        .saturating_mul(*every)
                        .saturating_add(*every),
                    InstanceDeadline::Tumbling,
                ),
                _ => continue,
            };
            arms.push((
                TimerArm {
                    kind: crate::journal::timer_kind::INSTANCE_LIFETIME,
                    reference: ids::instance_lifetime_ref(reference),
                    due_at,
                },
                DatedPayload::InstanceLifetime {
                    owner,
                    instance: reference.clone(),
                    cause,
                    deadline,
                },
            ));
        }
        arms
    }

    pub(super) fn delivery_deadlines(
        &self,
        edge: &Edge,
        target: &PreparedTarget,
        candidates: &BTreeMap<InstanceRef, InstanceCandidate>,
        envelope: &Envelope,
    ) -> Vec<(TimerArm, DatedPayload)> {
        let Some(to) = &edge.to else {
            return Vec::new();
        };
        let Some(slot) = to.node else {
            return Vec::new();
        };
        let node = &self.plan.nodes[slot.index()];
        let edge = self.plan.edge_text(edge);
        let delivery = envelope.meta().sys().id();
        let reference = ids::delivery_ref(&edge, delivery);
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
                TimerArm {
                    kind: crate::journal::timer_kind::JOIN_WITHIN,
                    reference,
                    due_at: self.now.saturating_add(within),
                },
                DatedPayload::JoinWithin {
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
                TimerArm {
                    kind: if handshake {
                        crate::journal::timer_kind::SESSION_HANDSHAKE
                    } else {
                        crate::journal::timer_kind::CALL_TIMEOUT
                    },
                    reference,
                    due_at: self.now.saturating_add(budget),
                },
                DatedPayload::DeliveryBudget {
                    connector: node.path.clone(),
                    edge,
                    delivery,
                    instance: target.reference.clone(),
                },
            ));
        }
        deadlines
    }

    pub(super) fn route_timer_arms(
        &self,
        routes: &[PreparedEdge],
        candidates: &BTreeMap<InstanceRef, InstanceCandidate>,
        envelope: &Envelope,
    ) -> Vec<TimerArm> {
        self.route_timer_arms_with_activity(routes, candidates, envelope, &mut BTreeMap::new())
    }

    pub(super) fn route_timer_arms_with_activity(
        &self,
        routes: &[PreparedEdge],
        candidates: &BTreeMap<InstanceRef, InstanceCandidate>,
        envelope: &Envelope,
        activity: &mut BTreeMap<InstanceRef, Id>,
    ) -> Vec<TimerArm> {
        let mut arms = Vec::new();
        for route in routes {
            if let Some(target) = &route.target {
                arms.extend(
                    self.admission_deadlines(target, candidates, envelope, activity)
                        .into_iter()
                        .map(|(arm, _)| arm),
                );
                arms.extend(
                    self.delivery_deadlines(&route.edge, target, candidates, envelope)
                        .into_iter()
                        .map(|(arm, _)| arm),
                );
            }
        }
        arms
    }

    pub(super) fn emission_timer_arms(&self, prepared: &PreparedEmissions) -> Vec<TimerArm> {
        let mut activity = BTreeMap::new();
        prepared
            .emissions
            .iter()
            .flat_map(|emission| {
                self.route_timer_arms_with_activity(
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
    ) -> Result<(), VmError> {
        record.routes = self.routed_deliveries(prepared);
        let mut arms = record
            .close_scope
            .as_ref()
            .map(|instance| self.closing_timer_arms(instance))
            .unwrap_or_default();
        arms.extend(self.emission_timer_arms(prepared));
        self.retain_timer_arms(record, arms)
    }

    pub(super) fn closing_timer_arms(&self, reference: &InstanceRef) -> Vec<TimerArm> {
        let Some(instance) = self.instances.get(reference) else {
            return Vec::new();
        };
        let Some(policy) = self.plan.owners[instance.owner.index()].instance.closing else {
            return Vec::new();
        };
        let drain = self.now.saturating_add(policy.drain_for);
        [
            (crate::journal::timer_kind::INSTANCE_DRAIN, drain),
            (
                crate::journal::timer_kind::INSTANCE_SETTLEMENT,
                drain.saturating_add(policy.settle_for),
            ),
        ]
        .into_iter()
        .map(|(kind, due_at)| TimerArm {
            kind,
            reference: ids::instance_closing_ref(reference, kind),
            due_at,
        })
        .collect()
    }

    pub(super) fn retain_timer_arms(
        &self,
        record: &mut crate::journal::TransactionRecord,
        arms: Vec<TimerArm>,
    ) -> Result<(), VmError> {
        if arms.is_empty() {
            return Ok(());
        }
        let mut combined = record
            .timer_schedule
            .as_ref()
            .map(|schedule| schedule.arms().to_vec())
            .unwrap_or_default();
        combined.extend(arms);
        record.timer_schedule = Some(Box::new(Self::timer_schedule(self.now, combined)?));
        Ok(())
    }

    pub(super) fn timer_schedule(at: i64, arms: Vec<TimerArm>) -> Result<TimerSchedule, VmError> {
        TimerSchedule::try_new(at, arms)
            .map_err(|error| VmError::Journal(crate::journal::Error::InvalidTimerSchedule(error)))
    }
}
