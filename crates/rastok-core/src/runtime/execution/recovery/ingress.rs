//! Reconstruct external admissions from their actual source, not provenance spelling.

use super::*;
use crate::instance;
use crate::journal::{DeadlineKind, InboundExchangeRecord, IngressRecord, SessionIngressRecord};

impl Execution<'_> {
    pub(super) fn restore_ingress(
        &mut self,
        admissions: &dyn InstanceAdmissions,
        index: usize,
        inbound: &IngressRecord,
    ) -> Result<(), ExecutionFault> {
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
                                    && port.direction == node::PortDirection::Output
                            }))
                        .then_some((NodeIndex::new(index as u32), spec.exchange))
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
            && !self.plan.boundary().iter().any(|port| {
                port.port == inbound.port && port.direction == node::PortDirection::Input
            })
        {
            return Err(invalid(index, "the retained ingress names no App input"));
        }
        let from = EdgeEnd {
            node: source.map(|(slot, _)| slot),
            port: inbound.port.clone(),
        };
        if let Data::Materialized(value) = inbound.envelope.data() {
            value
                .check_consistency()
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
                    &inbound.port,
                    &inbound.envelope,
                    &mut candidates,
                )?;
                timeout = self.inbound_call_timeout(slot, &selected.reference, &candidates)?;
                let expected_deadline_ids = timeout
                    .map(|_| vec![(DeadlineKind::InboundCallTimeout, call.reference.id())])
                    .unwrap_or_default();
                if call.reference != InboundCallId::from_root(inbound.envelope.meta().sys().id())
                    || call.instance != selected.reference
                    || call.deadline_ids != expected_deadline_ids
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
                    .and_then(|spec| session_opening(&spec.contract, spec.roles, spec.exchange))
                    != Some(SessionOpening::Peer)
                    || *reference
                        != ids::inbound_session_id(
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
                    &inbound.port,
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
        let commands = if opening {
            Vec::new()
        } else {
            self.prepare_instance_commands(
                admissions,
                &self.graph_endpoint(&from),
                target.as_ref().map(|(_, target)| &target.reference),
                &inbound.envelope,
            )?
        };
        self.check_instance_command_room(admissions, &commands)?;
        if inbound.instance_commands != Self::retained_commands(&commands) {
            return Err(invalid(
                index,
                "inbound instance commands contradict the exact accepting prefix",
            ));
        }
        let mut deadlines = Vec::new();
        let mut activity = BTreeMap::new();
        if let Some((_, target)) = &target
            && !target.chain.is_empty()
        {
            deadlines.extend(
                self.admission_deadlines(target, &candidates, &inbound.envelope, &mut activity)
                    .into_iter()
                    .map(|(deadline, _)| deadline),
            );
        }
        if let Some(timeout) = timeout {
            deadlines.push(crate::journal::deadline::Deadline {
                kind: DeadlineKind::InboundCallTimeout,
                reference: InboundCallId::from_root(inbound.envelope.meta().sys().id()).id(),
                due_at: self.now.saturating_add(timeout),
            });
        }
        deadlines.extend(self.route_deadlines_with_activity(
            &routes,
            &candidates,
            &inbound.envelope,
            &mut activity,
        ));
        if inbound
            .deadlines
            .as_deref()
            .map_or(&[][..], |schedule| schedule.items())
            != deadlines
        {
            return Err(invalid(
                index,
                "ingress deadlines differ from its exact source and routes",
            ));
        }
        if let Some((slot, target)) = &target {
            if !target.chain.is_empty() {
                self.apply_instance_admissions(target, &candidates, &inbound.envelope);
            }
            match exchange {
                Some(InboundExchangeRecord::Call(call)) => {
                    if let Some(timeout) = timeout {
                        self.scheduler.schedule(
                            self.now.saturating_add(timeout),
                            Obligation::InboundCallBudget {
                                call_id: call.reference,
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

    #[allow(clippy::too_many_arguments)]
    fn restore_source_target(
        &self,
        admissions: &dyn InstanceAdmissions,
        index: usize,
        slot: NodeIndex,
        port: &node::Port,
        envelope: &Envelope,
        candidates: &mut BTreeMap<instance::Generation, InstanceCandidate>,
    ) -> Result<PreparedTarget, ExecutionFault> {
        let scope = self
            .owner_chain(self.plan.nodes[slot.index()].owner)?
            .first()
            .copied()
            .ok_or_else(|| invalid(index, "an inbound source has no owner chain"))?;
        self.prepare_target(
            admissions,
            scope,
            None,
            &EdgeEnd {
                node: Some(slot),
                port: port.clone(),
            },
            envelope,
            candidates,
        )
    }
}
