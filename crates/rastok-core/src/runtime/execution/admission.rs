//! Prepare admission against immutable execution and journal state, commit
//! its INGRESS record and routes, then apply the decision to live state.

use super::*;
use crate::journal::{CommittedIngress, PreparedIngress, SessionIngressRecord};

struct IngressEndpoint {
    from: Endpoint,
    exchange: Option<ConnectorExchange>,
}

/// The five source cases have different instance and routing obligations.
/// In particular, opening a Session creates its source but routes no message;
/// a later message reuses that source without admitting it again.
enum PreparedExchange {
    Boundary,
    Event,
    Call {
        source: NodeIndex,
        target: PreparedTarget,
        timeout: Option<i64>,
    },
    SessionOpen {
        source: NodeIndex,
        target: PreparedTarget,
        session: SessionId,
    },
    SessionMessage {
        target: PreparedTarget,
    },
}

impl PreparedExchange {
    fn source_instance(&self) -> Option<&instance::Generation> {
        match self {
            Self::Boundary | Self::Event => None,
            Self::Call { target, .. }
            | Self::SessionOpen { target, .. }
            | Self::SessionMessage { target } => Some(&target.reference),
        }
    }

    fn admitted_target(&self) -> Option<&PreparedTarget> {
        match self {
            Self::Call { target, .. } | Self::SessionOpen { target, .. } => Some(target),
            Self::Boundary | Self::Event | Self::SessionMessage { .. } => None,
        }
    }
}

struct PreparedAdmission {
    ingress: PreparedIngress,
    exchange: PreparedExchange,
    candidates: BTreeMap<instance::Generation, InstanceCandidate>,
    routes: Vec<PreparedEdge>,
    commands: Vec<crate::journal::instance_stop::Command>,
}

impl Execution<'_> {
    pub(super) fn admit_proposal(
        &mut self,
        journal: &mut dyn Journal,
        submission: IngressProposal,
        connector_path: Option<node::Path>,
    ) -> Result<AdmittedIngress, ExecutionFault> {
        let prepared = self.prepare_admission(journal, submission, connector_path)?;
        let recorded_routes = self.inbound_route_targets(&prepared.routes);
        let PreparedAdmission {
            ingress,
            exchange,
            candidates,
            routes,
            commands,
        } = prepared;
        let committed = journal
            .commit_ingress(ingress, recorded_routes)
            .map_err(ExecutionFault::Journal)?;
        self.enqueue_instance_commands(commands);
        self.apply_exchange_admission(&exchange, &candidates, &committed.envelope);
        self.route_prepared(
            routes,
            &candidates,
            &committed.envelope,
            exchange.source_instance(),
        );
        Ok(Self::admitted_ingress(committed))
    }

    fn prepare_admission(
        &self,
        journal: &dyn Journal,
        submission: IngressProposal,
        connector_path: Option<node::Path>,
    ) -> Result<PreparedAdmission, ExecutionFault> {
        self.preflight(journal)?;
        let endpoint = self.resolve_ingress_endpoint(&submission, connector_path.as_ref())?;
        self.judge_ingress_metadata(&submission)?;
        self.judge_ingress_pins(&submission, &endpoint.from)?;

        // Root identity must be final before instance keys run. Preparation
        // does not advance its sequence; commit compares and advances it.
        let session = submission.session;
        let ingress = match connector_path {
            Some(_) => journal.prepare_connector_ingress(submission),
            None => journal.prepare_ingress(submission),
        }
        .map_err(ExecutionFault::Journal)?;
        let mut candidates = BTreeMap::new();
        let (mut ingress, exchange) =
            self.prepare_inbound_exchange(journal, &endpoint, session, ingress, &mut candidates)?;
        let opening_only = matches!(exchange, PreparedExchange::SessionOpen { .. });
        let routes = if opening_only {
            // The handshake is a Session fact, not an empty peer message.
            Vec::new()
        } else {
            self.prepare_routes(
                &journal,
                &endpoint.from,
                // Keep an admitted Session's ancestor generation. No key is
                // re-evaluated from the next message's external metadata.
                matches!(session, Some(InboundSession::Message(_)))
                    .then(|| exchange.source_instance())
                    .flatten(),
                ingress.envelope(),
                &mut candidates,
            )?
        };
        self.check_room_for_routes(core::iter::once(routes.as_slice()))?;
        let commands = if opening_only {
            Vec::new()
        } else {
            self.prepare_instance_commands(
                &journal,
                &endpoint.from,
                exchange.source_instance(),
                ingress.envelope(),
            )?
        };
        self.check_instance_command_room(&journal, &commands)?;
        if !commands.is_empty() {
            ingress =
                ingress.with_instance_commands(crate::journal::instance_stop::InstanceCommands {
                    produced: commands.clone(),
                    consumed: None,
                });
        }
        ingress = self.attach_ingress_deadlines(ingress, &exchange, &routes, &candidates)?;
        Ok(PreparedAdmission {
            ingress,
            exchange,
            candidates,
            routes,
            commands,
        })
    }

    fn resolve_ingress_endpoint(
        &self,
        submission: &IngressProposal,
        connector: Option<&node::Path>,
    ) -> Result<IngressEndpoint, ExecutionFault> {
        if let Some(connector) = connector {
            let source = self
                .plan
                .nodes
                .iter()
                .enumerate()
                .find(|(_, planned)| &planned.path == connector)
                .and_then(|(index, planned)| {
                    let spec = planned.connector()?;
                    let output = planned.ports.iter().any(|decl| {
                        decl.port == submission.draft.port
                            && decl.direction == node::PortDirection::Output
                    });
                    (spec.roles.has_inbound() && output).then_some((index, spec.exchange))
                });
            let unknown = || ExecutionFault::UnknownInboundSource {
                connector: connector.clone(),
                port: submission.draft.port.clone(),
            };
            let (source, exchange) = source.ok_or_else(unknown)?;
            let source = NodeIndex::new(u32::try_from(source).map_err(|_| unknown())?);
            Ok(IngressEndpoint {
                from: Endpoint {
                    node: source,
                    port: submission.draft.port.clone(),
                },
                exchange: Some(exchange),
            })
        } else {
            if !self.plan.boundary().iter().any(|decl| {
                decl.port == submission.draft.port && decl.direction == node::PortDirection::Input
            }) {
                return Err(ExecutionFault::UnknownIngress(
                    submission.draft.port.clone(),
                ));
            }
            Ok(IngressEndpoint {
                from: Endpoint {
                    node: crate::node::NodeIndex::ROOT,
                    port: submission.draft.port.clone(),
                },
                exchange: None,
            })
        }
    }

    fn judge_ingress_metadata(&self, submission: &IngressProposal) -> Result<(), ExecutionFault> {
        for (field, value) in &submission.draft.ext {
            // The journal derives Session correlation. Peer spelling is
            // replaced before routing or evaluating an instance key.
            if submission.session.is_some() && field == "session_id" {
                continue;
            }
            let ty = crate::plan::standard_ext_field_type(field).or_else(|| {
                self.plan
                    .plan()
                    .ext_schema
                    .iter()
                    .find(|(known, _)| known == field)
                    .map(|(_, ty)| ty.clone())
            });
            let Some(ty) = ty else {
                return Err(ExecutionFault::IngressMetadataViolation {
                    field: field.clone(),
                    reason: String::from("the field is outside the declared schema"),
                });
            };
            if !crate::runtime::meta_conforms(value, &ty) {
                return Err(ExecutionFault::IngressMetadataViolation {
                    field: field.clone(),
                    reason: alloc::format!("the field is declared {ty:?}; this value is not"),
                });
            }
        }
        Ok(())
    }

    fn judge_ingress_pins(
        &self,
        submission: &IngressProposal,
        from: &Endpoint,
    ) -> Result<(), ExecutionFault> {
        if let Data::Materialized(value) = &submission.draft.data {
            value.check_consistency().map_err(|error| {
                ExecutionFault::Fault(crate::runtime::Fault::InvalidValue(error))
            })?;
            crate::runtime::crossing_pins_hold(self.plan, from, value).map_err(|refused| {
                ExecutionFault::IngressPinViolation {
                    ingress: submission.draft.port.clone(),
                    rejected_at: refused.at,
                    violation: refused.violation,
                }
            })?;
        }
        Ok(())
    }

    /// Select and bound the source instance once, for both a Call and a
    /// Session opening. No candidate becomes live until the ingress commits.
    fn reserve_source_instance(
        &self,
        journal: &dyn Journal,
        endpoint: &IngressEndpoint,
        source: NodeIndex,
        envelope: &Envelope,
        candidates: &mut BTreeMap<instance::Generation, InstanceCandidate>,
    ) -> Result<PreparedTarget, ExecutionFault> {
        let owner = source;
        let scope = self.scope_chain(owner)?.first().copied().ok_or_else(|| {
            ExecutionFault::PlanInvariant {
                node: self.plan.nodes[source.index()].path.clone(),
                reason: match endpoint.exchange {
                    Some(ConnectorExchange::Call) => "an inbound Call source has no owner chain",
                    _ => "an inbound Session source has no owner chain",
                },
            }
        })?;
        let target =
            self.prepare_target(&journal, scope, None, &endpoint.from, envelope, candidates)?;
        let held = match endpoint.exchange {
            Some(ConnectorExchange::Call) => self
                .inbound_calls
                .values()
                .filter(|pending| pending.slot == source && pending.instance == target.reference)
                .count(),
            _ => self
                .sessions
                .values()
                .filter(|pending| pending.slot == source && pending.instance == target.reference)
                .count(),
        };
        if held >= WORKER_MAILBOX_CAPACITY {
            return Err(ExecutionFault::MailboxFull {
                node: self.plan.nodes[source.index()].path.clone(),
            });
        }
        Ok(target)
    }

    fn prepare_inbound_exchange(
        &self,
        journal: &dyn Journal,
        endpoint: &IngressEndpoint,
        session: Option<InboundSession>,
        mut ingress: PreparedIngress,
        candidates: &mut BTreeMap<instance::Generation, InstanceCandidate>,
    ) -> Result<(PreparedIngress, PreparedExchange), ExecutionFault> {
        let source = endpoint
            .exchange
            .map(|exchange| (endpoint.from.node, exchange));
        // Preserve the existing admission order: Call selection and its
        // journal marker precede refusal of contradictory Session framing.
        let call = if let Some((source, ConnectorExchange::Call)) = source {
            let target = self.reserve_source_instance(
                journal,
                endpoint,
                source,
                ingress.envelope(),
                candidates,
            )?;
            let timeout = self.inbound_call_timeout(source, &target.reference, candidates)?;
            let call_id = InboundCallId::from_root(ingress.envelope().meta().sys().id());
            let deadline_ids = timeout
                .map(|_| {
                    vec![(
                        crate::journal::DeadlineKind::InboundCallTimeout,
                        call_id.id(),
                    )]
                })
                .unwrap_or_default();
            ingress = ingress
                .into_call(target.reference.clone(), deadline_ids)
                .map_err(ExecutionFault::Journal)?;
            PreparedExchange::Call {
                source,
                target,
                timeout,
            }
        } else {
            PreparedExchange::Event
        };
        let exchange = match (source, session) {
            (Some((source, ConnectorExchange::Session)), Some(InboundSession::Open)) => {
                if self.plan.nodes[source.index()]
                    .connector()
                    .and_then(|spec| session_opening(&spec.contract, spec.roles, spec.exchange))
                    != Some(SessionOpening::Peer)
                {
                    return Err(ExecutionFault::PlanInvariant {
                        node: self.plan.nodes[source.index()].path.clone(),
                        reason: "this Contract does not admit peer Session openings",
                    });
                }
                let target = self.reserve_source_instance(
                    journal,
                    endpoint,
                    source,
                    ingress.envelope(),
                    candidates,
                )?;
                let Some(SessionIngressRecord::Open(session)) = ingress.session() else {
                    return Err(ExecutionFault::PlanInvariant {
                        node: self.plan.nodes[source.index()].path.clone(),
                        reason: "a prepared Session opening lost its logical reference",
                    });
                };
                PreparedExchange::SessionOpen {
                    source,
                    target,
                    session,
                }
            }
            (
                Some((source, ConnectorExchange::Session)),
                Some(InboundSession::Message(session)),
            ) => {
                let Some(pending) = self.sessions.get(&session) else {
                    return Err(ExecutionFault::PlanInvariant {
                        node: self.plan.nodes[source.index()].path.clone(),
                        reason: "an inbound Session message has no installed logical session",
                    });
                };
                if pending.slot != source || !pending.opened {
                    return Err(ExecutionFault::PlanInvariant {
                        node: self.plan.nodes[source.index()].path.clone(),
                        reason: "an inbound Session message crossed its bound source",
                    });
                }
                PreparedExchange::SessionMessage {
                    target: PreparedTarget {
                        reference: pending.instance.clone(),
                        scope: pending.instance.clone(),
                        chain: Vec::new(),
                        admitted: Vec::new(),
                    },
                }
            }
            (Some((source, ConnectorExchange::Session)), None) => {
                return Err(ExecutionFault::PlanInvariant {
                    node: self.plan.nodes[source.index()].path.clone(),
                    reason: "a Session source arrived without Session framing",
                });
            }
            (Some((source, ConnectorExchange::Event | ConnectorExchange::Call)), Some(_)) => {
                return Err(ExecutionFault::PlanInvariant {
                    node: self.plan.nodes[source.index()].path.clone(),
                    reason: "Session framing reached a non-Session source",
                });
            }
            (Some((_, ConnectorExchange::Event | ConnectorExchange::Call)), None) => call,
            (None, _) => PreparedExchange::Boundary,
        };
        Ok((ingress, exchange))
    }

    fn attach_ingress_deadlines(
        &self,
        mut ingress: PreparedIngress,
        exchange: &PreparedExchange,
        routes: &[PreparedEdge],
        candidates: &BTreeMap<instance::Generation, InstanceCandidate>,
    ) -> Result<PreparedIngress, ExecutionFault> {
        let mut activity = BTreeMap::new();
        let mut deadlines = Vec::new();
        if let Some(target) = exchange.admitted_target() {
            deadlines.extend(
                self.admission_deadlines(target, candidates, ingress.envelope(), &mut activity)
                    .into_iter()
                    .map(|(deadline, _)| deadline),
            );
        }
        if let PreparedExchange::Call {
            timeout: Some(timeout),
            ..
        } = exchange
        {
            deadlines.push(crate::journal::deadline::Deadline {
                kind: crate::journal::DeadlineKind::InboundCallTimeout,
                reference: InboundCallId::from_root(ingress.envelope().meta().sys().id()).id(),
                due_at: self.now.saturating_add(*timeout),
            });
        }
        deadlines.extend(self.route_deadlines_with_activity(
            routes,
            candidates,
            ingress.envelope(),
            &mut activity,
        ));
        if !deadlines.is_empty() {
            ingress = ingress.with_deadlines(Self::create_deadlines(self.now, deadlines)?);
        }
        Ok(ingress)
    }

    /// Apply the exchange accepted by the INGRESS commit. The prepared
    /// variant contains every value required here; no further check can refuse.
    fn apply_exchange_admission(
        &mut self,
        exchange: &PreparedExchange,
        candidates: &BTreeMap<instance::Generation, InstanceCandidate>,
        envelope: &Envelope,
    ) {
        match exchange {
            PreparedExchange::Call {
                source,
                target,
                timeout,
            } => {
                self.apply_instance_admissions(target, candidates, envelope);
                let call_id = InboundCallId::from_root(envelope.meta().sys().id());
                if let Some(timeout) = timeout {
                    self.scheduler.schedule(
                        self.now.saturating_add(*timeout),
                        Obligation::InboundCallBudget {
                            call_id,
                            instance: target.reference.clone(),
                        },
                    );
                }
                self.inbound_calls.insert(
                    call_id,
                    PendingInboundCall {
                        slot: *source,
                        request: envelope.clone(),
                        instance: target.reference.clone(),
                        response: None,
                        timeout: timeout.map(|_| PendingInboundCallTimeout {
                            requested: false,
                            started: false,
                        }),
                    },
                );
            }
            PreparedExchange::SessionOpen {
                source,
                target,
                session,
            } => {
                self.apply_instance_admissions(target, candidates, envelope);
                self.sessions.insert(
                    *session,
                    PendingSession {
                        slot: *source,
                        instance: target.reference.clone(),
                        opening: envelope.clone(),
                        open_intent: None,
                        opening_delivery: None,
                        opened: true,
                        opening_started: true,
                        opening_claimed: false,
                        close_started: false,
                    },
                );
            }
            PreparedExchange::Boundary
            | PreparedExchange::Event
            | PreparedExchange::SessionMessage { .. } => {}
        }
    }

    fn admitted_ingress(committed: CommittedIngress) -> AdmittedIngress {
        let exchange = committed
            .record
            .exchange
            .as_deref()
            .map(|exchange| match exchange {
                crate::journal::InboundExchangeRecord::Call(call) => {
                    crate::boundary::InboundExchangeId::Call(call.reference)
                }
                crate::journal::InboundExchangeRecord::Session(session) => {
                    let reference = match session {
                        SessionIngressRecord::Open(reference)
                        | SessionIngressRecord::Message(reference) => *reference,
                    };
                    crate::boundary::InboundExchangeId::Session(reference)
                }
            });
        AdmittedIngress {
            envelope: committed.envelope.meta().sys().id(),
            exchange,
        }
    }
}
