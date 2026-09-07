//! Reconstruct external admissions from their actual source, not provenance spelling.

use super::*;
use crate::journal::{InboundExchangeRecord, InboundRecord, SessionIngressRecord, timer_kind};

impl Vm<'_> {
    pub(super) fn restore_ingress(
        &mut self,
        admissions: &impl InstanceAdmissions,
        index: usize,
        inbound: &InboundRecord,
    ) -> Result<(), VmError> {
        if inbound.envelope.meta().sys().graph_version() != self.graph_version.0 {
            return Err(invalid(
                index,
                "the ingress graph version differs from the deployed plan",
            ));
        }
        let source = inbound
            .connector_path
            .as_ref()
            .map(|path| {
                self.plan
                    .nodes
                    .iter()
                    .enumerate()
                    .find_map(|(index, node)| {
                        let spec = node.connector()?;
                        (node.path == *path
                            && spec.roles.has_inbound()
                            && node.ports.iter().any(|port| {
                                port.port == inbound.port
                                    && port.direction == node::Direction::Output
                            }))
                        .then_some((Slot::new(index as u32), spec.exchange))
                    })
                    .ok_or_else(|| {
                        invalid(
                            index,
                            "the retained ingress has no matching inbound Contract port",
                        )
                    })
            })
            .transpose()?;
        if source.is_none()
            && !self
                .plan
                .boundary
                .iter()
                .any(|port| port.port == inbound.port && port.direction == node::Direction::Input)
        {
            return Err(invalid(index, "the retained ingress names no App input"));
        }
        let from = EndpointRef {
            node: source.map(|(slot, _)| slot),
            port: inbound.port.clone(),
        };
        if let Data::Materialized(value) = inbound.envelope.data() {
            value
                .validate()
                .map_err(|_| invalid(index, "ingress contains an invalid value"))?;
            crate::runtime::crossing_pins_hold(self.plan, &from, value)
                .map_err(|_| invalid(index, "ingress contradicts its resolved pins"))?;
        }
        for (name, value) in inbound.envelope.meta().ext() {
            let ty = crate::plan::standard_ext_field_type(name).or_else(|| {
                self.plan
                    .ext_schema
                    .iter()
                    .find(|(field, _)| field == name)
                    .map(|(_, ty)| ty.clone())
            });
            if ty.is_none_or(|ty| !crate::runtime::meta_conforms(value, &ty)) {
                return Err(invalid(
                    index,
                    "ingress metadata contradicts the travelling schema",
                ));
            }
        }
        let mut candidates = BTreeMap::new();
        let mut target = None;
        let mut timeout = None;
        let exchange = inbound.exchange.as_deref();
        match (source, exchange) {
            (None | Some((_, ConnectorExchange::Event)), None) => {}
            (Some((slot, ConnectorExchange::Call)), Some(InboundExchangeRecord::Call(call))) => {
                let selected = self.restore_source_target(
                    admissions,
                    index,
                    slot,
                    &inbound.envelope,
                    &mut candidates,
                )?;
                timeout = self.inbound_call_timeout(slot, &selected.reference, &candidates)?;
                let expected_timers = timeout
                    .map(|_| vec![(timer_kind::INBOUND_CALL_TIMEOUT, call.reference.id())])
                    .unwrap_or_default();
                if call.reference != InboundCallRef::from_root(inbound.envelope.meta().sys().id())
                    || call.instance != selected.reference
                    || call.timers != expected_timers
                    || self.inbound_calls.contains_key(&call.reference)
                    || self
                        .inbound_calls
                        .values()
                        .filter(|pending| {
                            pending.slot == slot && pending.instance == selected.reference
                        })
                        .count()
                        >= WORKER_MAILBOX_CAPACITY
                {
                    return Err(invalid(
                        index,
                        "the inbound Call opening contradicts its source admission",
                    ));
                }
                target = Some((slot, selected));
            }
            (
                Some((slot, ConnectorExchange::Session)),
                Some(InboundExchangeRecord::Session(SessionIngressRecord::Open(reference))),
            ) => {
                if self.plan.nodes[slot.index()]
                    .connector()
                    .is_none_or(|spec| spec.effect != node::Effect::None)
                    || *reference
                        != ids::inbound_session_ref(
                            &inbound.provenance,
                            inbound.envelope.meta().sys().id(),
                        )
                    || self.sessions.contains_key(reference)
                {
                    return Err(invalid(
                        index,
                        "the inbound Session opening contradicts its Contract or root",
                    ));
                }
                let selected = self.restore_source_target(
                    admissions,
                    index,
                    slot,
                    &inbound.envelope,
                    &mut candidates,
                )?;
                if self
                    .sessions
                    .values()
                    .filter(|session| {
                        session.slot == slot && session.instance == selected.reference
                    })
                    .count()
                    >= WORKER_MAILBOX_CAPACITY
                {
                    return Err(invalid(
                        index,
                        "the retained Session exceeds its source capacity",
                    ));
                }
                target = Some((slot, selected));
            }
            (
                Some((slot, ConnectorExchange::Session)),
                Some(InboundExchangeRecord::Session(SessionIngressRecord::Message(reference))),
            ) => {
                let pending = self
                    .sessions
                    .get(reference)
                    .filter(|pending| pending.slot == slot && pending.opened)
                    .ok_or_else(|| {
                        invalid(index, "the peer message has no open Session at this source")
                    })?;
                target = Some((
                    slot,
                    PreparedTarget {
                        reference: pending.instance.clone(),
                        scope: pending.instance.clone(),
                        chain: Vec::new(),
                        admitted: Vec::new(),
                    },
                ));
            }
            _ => {
                return Err(invalid(
                    index,
                    "the ingress exchange contradicts the source profile",
                ));
            }
        }
        if let Some(InboundExchangeRecord::Session(session)) = exchange {
            let (SessionIngressRecord::Open(reference) | SessionIngressRecord::Message(reference)) =
                session;
            if inbound.envelope.meta().ext().get("session_id")
                != Some(&crate::envelope::MetaValue::Scalar(
                    crate::envelope::Scalar::Text(reference.to_string()),
                ))
            {
                return Err(invalid(
                    index,
                    "the peer message carries a crossed Session projection",
                ));
            }
        }
        let opening = matches!(
            exchange,
            Some(InboundExchangeRecord::Session(SessionIngressRecord::Open(
                _
            )))
        );
        let routes = if opening {
            Vec::new()
        } else {
            self.prepare_routes(admissions, &from, None, &inbound.envelope, &mut candidates)?
        };
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
        let mut arms = Vec::new();
        let mut activity = BTreeMap::new();
        if let Some((_, target)) = &target
            && !target.chain.is_empty()
        {
            arms.extend(
                self.admission_deadlines(target, &candidates, &inbound.envelope, &mut activity)
                    .into_iter()
                    .map(|(arm, _)| arm),
            );
        }
        if let Some(timeout) = timeout {
            arms.push(crate::journal::timing::TimerArm {
                kind: timer_kind::INBOUND_CALL_TIMEOUT,
                reference: InboundCallRef::from_root(inbound.envelope.meta().sys().id()).id(),
                due_at: self.now.saturating_add(timeout),
            });
        }
        arms.extend(self.route_timer_arms_with_activity(
            &routes,
            &candidates,
            &inbound.envelope,
            &mut activity,
        ));
        if inbound
            .timer_schedule
            .as_deref()
            .map_or(&[][..], |schedule| schedule.arms())
            != arms
        {
            return Err(invalid(
                index,
                "ingress deadlines differ from its exact source and routes",
            ));
        }
        if let Some((slot, target)) = &target {
            if !target.chain.is_empty() {
                self.install_target(target, &candidates, &inbound.envelope);
            }
            match exchange {
                Some(InboundExchangeRecord::Call(call)) => {
                    if let Some(timeout) = timeout {
                        self.scheduler.arm(
                            self.now.saturating_add(timeout),
                            DatedPayload::InboundCallBudget {
                                call_ref: call.reference,
                                instance: target.reference.clone(),
                            },
                        );
                    }
                    self.inbound_calls.insert(
                        call.reference,
                        PendingInboundCall {
                            slot: *slot,
                            request: inbound.envelope.clone(),
                            instance: target.reference.clone(),
                            response: None,
                            timeout: timeout.map(|_| PendingInboundCallTimeout {
                                requested: false,
                                started: false,
                            }),
                        },
                    );
                }
                Some(InboundExchangeRecord::Session(SessionIngressRecord::Open(reference))) => {
                    self.sessions.insert(
                        *reference,
                        PendingSession {
                            slot: *slot,
                            instance: target.reference.clone(),
                            opening: inbound.envelope.clone(),
                            open_intent: None,
                            opening_delivery: None,
                            opened: true,
                            opening_started: true,
                            opening_claimed: false,
                            close_started: false,
                        },
                    );
                }
                _ => {}
            }
        }
        self.route_prepared(
            routes,
            &candidates,
            &inbound.envelope,
            target.as_ref().map(|(_, target)| &target.reference),
        );
        Ok(())
    }

    fn restore_source_target(
        &self,
        admissions: &impl InstanceAdmissions,
        index: usize,
        slot: Slot,
        envelope: &Envelope,
        candidates: &mut BTreeMap<InstanceRef, InstanceCandidate>,
    ) -> Result<PreparedTarget, VmError> {
        let scope = self
            .owner_chain(self.plan.nodes[slot.index()].owner)?
            .first()
            .copied()
            .ok_or_else(|| invalid(index, "an inbound source has no owner chain"))?;
        self.prepare_target(admissions, scope, None, slot, envelope, candidates)
    }
}
