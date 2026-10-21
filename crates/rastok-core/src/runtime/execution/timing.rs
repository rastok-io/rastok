//! Delivery deadlines are selected from the same resolved targets before
//! acceptance and when installing the scheduler projection after commit.

use super::*;
use crate::instance;
use crate::journal::deadline::{Deadline, Deadlines};

/// The work selected when a scheduler deadline is reached. Each variant
/// carries the exact delivery, run or instance to which the obligation belongs;
/// the scheduler does not infer a current generation when it fires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Obligation {
    /// A delivery's call budget expired. The budget runs from the
    /// DELIVERY into the connector's mailbox (execution-model §8) —
    /// queue time counts — and the obligation resolves by its
    /// delivery reference `(edge, envelope_id)`, whether the call is
    /// still queued or already open.
    DeliveryBudget {
        /// The connector whose budget this is (the mailbox to search).
        connector: node::Path,
        /// Half of the delivery reference: the canonical edge the
        /// envelope traveled. The obligation's identity is
        /// `budget_ref(edge, delivery)` — never the envelope id
        /// alone: two ports of one connector are two obligations.
        edge: String,
        /// The other half of the reference: the delivered envelope's
        /// id.
        delivery: envelope::Id,
        /// Exact connector instance owning the mailbox/call obligation.
        instance: instance::Generation,
    },
    /// An admitted inbound Call reached the request deadline published by
    /// its Connector contract. The Driver owns the protocol-side timeout
    /// response; this event only transfers the scheduler's won request.
    InboundCallBudget {
        /// Stable reference of the admitted root Call.
        call_id: InboundCallId,
        /// Exact source generation retaining the request.
        instance: instance::Generation,
    },
    /// One retained join candidate reached its `within` bound. The full
    /// delivery reference makes a consumed candidate's stale event harmless.
    JoinWithin {
        /// Operator owning the join buffer.
        operator: node::Path,
        /// Edge half of the buffered-delivery reference.
        edge: String,
        /// Envelope-id half of the buffered-delivery reference.
        delivery: envelope::Id,
        /// Exact Operator generation that retained it.
        instance: instance::Generation,
    },
    /// One committed Loop continuation reached its re-admission date. The
    /// journal fold remains authoritative for the state, ordinal and fuel;
    /// these fields only identify the exact obligation to re-check.
    LoopDelay {
        /// Owner carrying the Loop Extension.
        owner: node::NodeIndex,
        /// Exact generation in which the continuation was selected.
        instance: instance::Generation,
        /// Durable Loop run whose fold must still be open.
        run: crate::journal::LoopRunId,
        /// Ordinal of the manager-produced state awaiting admission.
        iteration: u64,
        /// Re-entry envelope id, also used as deadline kind 4's stable id (RecurrenceDelay).
        state: envelope::Id,
    },
    /// Original closure-based eligibility, on the same dated scheduler as Loop.
    RetryDelay {
        /// Exact generation retaining the operation.
        instance: instance::Generation,
        /// Predecessor whose winning closure created this deadline.
        attempt: crate::journal::retry_protocol::AttemptId,
        /// Following admission reference (deadline kind 4, RecurrenceDelay).
        admission: envelope::Id,
    },
    /// One owner lifetime reached a scheduler-owned deadline. The complete
    /// instance reference is the generation fence; idle additionally compares
    /// the causal admission id, while fixed and tumbling never move after
    /// `l:start`.
    InstanceLifetime {
        /// Plan owner whose policy scheduled the event.
        owner: node::NodeIndex,
        /// Exact generation whose deadline is being observed.
        instance: instance::Generation,
        /// Envelope whose arrival created this deadline and causes `l:stop`.
        cause: envelope::Id,
        /// Which profile determines the stale-event rule.
        deadline: InstanceDeadline,
    },
    /// One phase boundary of the explicit bounded Closing protocol.
    InstanceClosing {
        /// Root generation whose policy owns the whole closing subtree.
        instance: instance::Generation,
        /// Envelope that won the close request.
        cause: envelope::Id,
        /// Drain or settlement deadline.
        deadline: ClosingDeadline,
    },
}

/// Scheduler-owned stages of bounded instance Closing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosingDeadline {
    /// Stop executing admitted ordinary work and request cancellation.
    Drain,
    /// Stop waiting for cancellation observations; unresolved calls become
    /// `OutcomeUnknown` and their EGRESS records become `abandoned`.
    Settlement,
}

/// Scheduler semantics of a finite dated instance lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceDeadline {
    /// Superseded by any later admitted envelope in the same generation.
    Idle,
    /// Fixed relative to the creating `l:start`.
    Fixed,
    /// Fixed at the end of the address's epoch-aligned processing-time box.
    Tumbling,
}

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
            let lifetime = &self.plan.nodes[owner.index()].instance.lifetime;
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
        let slot = to.node;
        let node = &self.plan.nodes[slot.index()];
        let Some(path) = node.executor_path() else {
            return Vec::new();
        };
        // A Graph-boundary admission retains its own source path; its
        // executable projection alone cannot identify the delivery deadline.
        let edge = self.prepared_edge_text(route);
        let delivery = envelope.meta().sys().id();
        let reference = ids::delivery_id(&edge, delivery);
        let mut deadlines = Vec::new();
        let within = node
            .handlers()
            .iter()
            .find_map(|handler| match &handler.activation {
                node::HandlerActivation::Join {
                    ports, within_ns, ..
                } if ports.contains(&to.port) => *within_ns,
                _ => None,
            });
        if let Some(within) = within {
            deadlines.push((
                Deadline {
                    kind: crate::journal::DeadlineKind::JoinWithin,
                    reference,
                    due_at: self.now.saturating_add(within),
                },
                Obligation::JoinWithin {
                    operator: path.clone(),
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
                    connector: path.clone(),
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
        let Some(policy) = self.plan.nodes[instance.owner.index()].instance.closing else {
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
