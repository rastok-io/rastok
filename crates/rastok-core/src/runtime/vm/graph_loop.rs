//! Retained Loop Graph admissions stop boundary expansion before fan-out.
//! The attached manager owns these bounded deliveries; Graphs stay scopes.

use super::*;
use crate::journal::loop_protocol::{self as protocol, Fact, OpeningAdmissionRef, Packet};
use crate::plan::{Endpoint, EndpointOwner, LoopCommandTarget, RouteDestination, RouteSelection};

#[derive(Debug, Clone)]
pub(super) struct BoundaryRoute {
    pub source: Endpoint,
    pub destination: Endpoint,
}

#[derive(Debug, Clone)]
struct Admission {
    owner: OwnerSlot,
    instance: InstanceRef,
    opening: OpeningAdmissionRef,
    delivery: Delivery,
}

#[derive(Debug, Default)]
pub(super) struct GraphLoops {
    admissions: BTreeMap<Id, Admission>,
    sequences: BTreeMap<(String, InstanceRef), u64>,
    contexts: BTreeMap<Id, Vec<LoopRunRef>>,
}

struct Selection {
    destinations: BTreeSet<Endpoint>,
    commands: Vec<LoopCommandTarget>,
    terminals: BTreeSet<OwnerSlot>,
    instance_stops: Vec<crate::plan::InstanceStopTarget>,
}

impl Vm<'_> {
    pub(super) fn graph_endpoint(&self, physical: &EndpointRef) -> Endpoint {
        Endpoint {
            owner: physical.node.map_or_else(
                || EndpointOwner::Graph(OwnerPath::root()),
                |slot| EndpointOwner::Node(self.plan.nodes[slot.index()].path.clone()),
            ),
            port: physical.port.clone(),
        }
    }

    fn endpoint_owner(&self, endpoint: &Endpoint) -> Result<OwnerSlot, VmError> {
        let path = match &endpoint.owner {
            EndpointOwner::Node(path) => OwnerPath::from(path),
            EndpointOwner::Graph(path) => path.clone(),
        };
        self.plan
            .owners
            .iter()
            .position(|owner| owner.path == path)
            .map(|index| OwnerSlot::new(index as u32))
            .ok_or_else(|| graph_error(&path, "a routed endpoint lost its declared owner"))
    }

    fn endpoint_ref(&self, endpoint: &Endpoint) -> Result<EndpointRef, VmError> {
        let node = match &endpoint.owner {
            EndpointOwner::Graph(_) => None,
            EndpointOwner::Node(path) => Some(
                self.plan
                    .nodes
                    .iter()
                    .position(|node| node.path == *path)
                    .map(|index| Slot::new(index as u32))
                    .ok_or_else(|| {
                        graph_error(&OwnerPath::from(path), "a route names no executable Node")
                    })?,
            ),
        };
        Ok(EndpointRef {
            node,
            port: endpoint.port.clone(),
        })
    }

    fn graph_entry(&self, endpoint: &Endpoint) -> Option<OwnerSlot> {
        let EndpointOwner::Graph(path) = &endpoint.owner else {
            return None;
        };
        self.plan
            .owners
            .iter()
            .enumerate()
            .find_map(|(index, owner)| {
                (owner.path == *path
                    && owner
                        .loop_spec()
                        .is_some_and(|spec| spec.entry == endpoint.port))
                .then_some(OwnerSlot::new(index as u32))
            })
    }

    fn select_graph_routes(
        &self,
        source: &Endpoint,
        envelope: &Envelope,
    ) -> Result<Selection, VmError> {
        let mut selected = Selection {
            destinations: BTreeSet::new(),
            commands: Vec::new(),
            terminals: BTreeSet::new(),
            instance_stops: Vec::new(),
        };
        let mut pending = vec![source.clone()];
        let mut visited = BTreeSet::new();
        let mut value = None;
        while let Some(endpoint) = pending.pop() {
            if !visited.insert(endpoint.clone()) {
                return Err(graph_error(
                    &OwnerPath::root(),
                    "a selected boundary route repeats a crossing",
                ));
            }
            for rule in self
                .plan
                .boundary_routes
                .iter()
                .filter(|rule| rule.source == endpoint)
            {
                let destinations = match &rule.selection {
                    RouteSelection::All(destinations) => destinations.as_slice(),
                    RouteSelection::Cases(_) => {
                        if value.is_none() {
                            value = Some(
                                crate::runtime::data_value(envelope.data())
                                    .map_err(VmError::Fault)?,
                            );
                        }
                        rule.selection
                            .selected(value.as_ref().ok_or_else(|| {
                                graph_error(&OwnerPath::root(), "a case route has no value")
                            })?)
                            .ok_or_else(|| {
                                graph_error(&OwnerPath::root(), "a produced case has no route")
                            })?
                    }
                };
                for destination in destinations {
                    match destination {
                        RouteDestination::InstanceStop(target) => {
                            selected.instance_stops.push(*target.clone())
                        }
                        RouteDestination::Discard => {}
                        RouteDestination::LoopCommand(command) => {
                            selected.commands.push(command.clone())
                        }
                        RouteDestination::Port(target) => {
                            let owner = self.endpoint_owner(target)?;
                            if matches!(target.owner, EndpointOwner::Graph(_)) {
                                if self.graph_entry(target).is_some() {
                                    if !selected.destinations.insert(target.clone()) {
                                        return Err(graph_error(
                                            &self.plan.owners[owner.index()].path,
                                            "a Graph admission was selected twice",
                                        ));
                                    }
                                    continue;
                                }
                                if self.plan.owners[owner.index()]
                                    .loop_spec()
                                    .is_some_and(|spec| spec.terminals.contains(&target.port))
                                {
                                    selected.terminals.insert(owner);
                                }
                                if !self.plan.owners[owner.index()].path.is_root() {
                                    pending.push(target.clone());
                                    continue;
                                }
                            }
                            if !selected.destinations.insert(target.clone()) {
                                return Err(graph_error(
                                    &self.plan.owners[owner.index()].path,
                                    "a selected route repeats its destination",
                                ));
                            }
                        }
                    }
                }
            }
        }
        Ok(selected)
    }

    pub(super) fn graph_instance_stop_targets(
        &self,
        source: &Endpoint,
        envelope: &Envelope,
    ) -> Result<Vec<crate::plan::InstanceStopTarget>, VmError> {
        Ok(self.select_graph_routes(source, envelope)?.instance_stops)
    }

    pub(super) fn prepare_graph_routes(
        &self,
        journal: &(impl InstanceAdmissions + ?Sized),
        source: &Endpoint,
        source_instance: Option<&InstanceRef>,
        envelope: &Envelope,
        candidates: &mut BTreeMap<InstanceRef, InstanceCandidate>,
        expand_entry: bool,
    ) -> Result<Vec<PreparedEdge>, VmError> {
        let destinations =
            if !expand_entry && source_instance.is_none() && self.graph_entry(source).is_some() {
                BTreeSet::from([source.clone()])
            } else {
                self.select_graph_routes(source, envelope)?.destinations
            };
        let mut prepared = Vec::new();
        let source_owner = self.endpoint_owner(source)?;
        for destination in destinations {
            let target_owner = self.endpoint_owner(&destination)?;
            let source_chain = self.owner_chain(source_owner)?;
            let target_chain = self.owner_chain(target_owner)?;
            let scope = source_chain
                .iter()
                .zip(target_chain)
                .take_while(|(left, right)| left == right)
                .map(|(owner, _)| *owner)
                .last()
                .ok_or_else(|| graph_error(&OwnerPath::root(), "a route has no common scope"))?;
            let from = self.endpoint_ref(source)?;
            let to = self.endpoint_ref(&destination)?;
            let target = if to.node.is_some() || self.graph_entry(&destination).is_some() {
                Some(self.prepare_owner_target(
                    journal,
                    scope,
                    source_instance,
                    target_owner,
                    envelope,
                    candidates,
                )?)
            } else {
                None
            };
            prepared.push(PreparedEdge {
                edge: Edge {
                    scope,
                    from,
                    to: Some(to),
                },
                target,
                boundary: Some(Box::new(BoundaryRoute {
                    source: source.clone(),
                    destination,
                })),
            });
        }
        Ok(prepared)
    }

    pub(super) fn prepared_edge_text(&self, prepared: &PreparedEdge) -> String {
        let Some(boundary) = &prepared.boundary else {
            return self.edge_text(&prepared.edge);
        };
        if boundary.source == boundary.destination {
            // Public ingress is already a fact, not a spatial self-edge.
            return format!("@ingress:{}", boundary.source.port);
        }
        format!(
            "{}>{}",
            endpoint_text(&boundary.source),
            endpoint_text(&boundary.destination)
        )
    }

    pub(super) fn prepared_graph_entry(&self, prepared: &PreparedEdge) -> Option<OwnerSlot> {
        prepared
            .boundary
            .as_ref()
            .and_then(|boundary| self.graph_entry(&boundary.destination))
    }

    pub(super) fn deliver_graph_admission(
        &mut self,
        route: &PreparedEdge,
        candidates: &BTreeMap<InstanceRef, InstanceCandidate>,
        envelope: &Envelope,
        source: Option<&InstanceRef>,
    ) -> bool {
        let Some(owner) = self.prepared_graph_entry(route) else {
            return false;
        };
        let Some(target) = &route.target else {
            return false;
        };
        let Some(to) = route.edge.to.as_ref() else {
            return false;
        };
        let edge = self.prepared_edge_text(route);
        self.install_target(target, candidates, envelope);
        let source = source.unwrap_or(&target.scope).clone();
        let Some(graph) = self.graph_loops.as_deref_mut() else {
            return false;
        };
        let reference = ids::delivery_ref(&edge, envelope.meta().sys().id());
        let seq = graph
            .sequences
            .entry((edge.clone(), source.clone()))
            .or_default();
        *seq += 1;
        graph.admissions.insert(
            reference,
            Admission {
                owner,
                instance: target.reference.clone(),
                opening: if self.plan.owners[owner.index()].path.is_root() {
                    OpeningAdmissionRef::Ingress {
                        inbound: envelope.meta().sys().id(),
                        port: to.port.clone(),
                    }
                } else {
                    OpeningAdmissionRef::Delivery(reference)
                },
                delivery: Delivery {
                    port: to.port.clone(),
                    edge,
                    seq: *seq,
                    source_instance: source,
                    envelope: envelope.clone(),
                },
            },
        );
        true
    }

    pub(super) fn graph_admission_load(&self) -> usize {
        self.graph_loops.as_deref().map_or(0, |graph| {
            graph
                .admissions
                .values()
                .filter(|admission| {
                    self.loop_manager.as_deref().is_none_or(|fold| {
                        fold.run(protocol::run_ref(&admission.instance, &admission.opening))
                            .is_none()
                    })
                })
                .count()
        })
    }

    pub(super) fn graph_context(&self, delivery: &Delivery) -> &[LoopRunRef] {
        self.graph_context_for(ids::delivery_ref(
            &delivery.edge,
            delivery.envelope.meta().sys().id(),
        ))
    }

    pub(super) fn graph_context_for(&self, delivery: Id) -> &[LoopRunRef] {
        self.graph_loops
            .as_deref()
            .and_then(|graph| graph.contexts.get(&delivery))
            .map_or(&[], Vec::as_slice)
    }

    pub(super) fn graph_run_has_work(&self, run: LoopRunRef) -> bool {
        if self.graph_loops.is_none() {
            return false;
        }
        self.scheduler
            .mailboxes
            .queues
            .values()
            .flatten()
            .chain(self.pending.values().map(|pending| &pending.delivery))
            .any(|delivery| self.graph_context(delivery).contains(&run))
            || self.retry_manager.as_deref().is_some_and(|fold| {
                fold.operations().any(|(_, state)| {
                    state.phase != crate::journal::retry_protocol::Phase::Closed
                        && self.graph_context_for(state.opening).contains(&run)
                })
            })
            || self.graph_loops.as_deref().is_some_and(|graph| {
                graph
                    .admissions
                    .values()
                    .any(|admission| self.graph_context(&admission.delivery).contains(&run))
            })
            || self.loop_manager.as_deref().is_some_and(|fold| {
                fold.runs().any(|(_, state)| {
                    matches!(
                        state.phase,
                        protocol::Phase::Active | protocol::Phase::AwaitingAdmission
                    ) && state.enclosing.contains(&run)
                })
            })
    }

    pub(super) fn graph_run_awaits_verdict(
        &self,
        run: LoopRunRef,
        state: &protocol::RunState,
    ) -> bool {
        state.phase == protocol::Phase::Active
            && state.pending.is_none()
            && self.instances.get(&state.instance).is_some_and(|instance| {
                matches!(instance.phase, InstancePhase::Open)
                    && self.plan.owners[instance.owner.index()].kind == OwnerKind::Graph
            })
            && !self.graph_run_has_work(run)
    }

    pub(super) fn graph_stop_cursors(
        &self,
        instance: &InstanceRef,
    ) -> Vec<crate::journal::ConsumedPosition> {
        self.graph_loops
            .as_deref()
            .into_iter()
            .flat_map(|graph| graph.admissions.values())
            .filter(|admission| admission.instance == *instance)
            .map(|admission| crate::journal::ConsumedPosition {
                edge: admission.delivery.edge.clone(),
                source: admission.delivery.source_instance.clone(),
                seq: admission.delivery.seq,
            })
            .collect()
    }

    pub(super) fn discard_graph_admissions(&mut self, instance: &InstanceRef) {
        if let Some(graph) = self.graph_loops.as_deref_mut() {
            graph.admissions.retain(|reference, admission| {
                if admission.instance == *instance {
                    graph.contexts.remove(reference);
                    false
                } else {
                    true
                }
            });
        }
    }

    pub(super) fn open_graph_loop(&mut self, journal: &mut dyn Journal) -> Result<bool, VmError> {
        let next = self.next_graph_admission().cloned();
        let Some(admission) = next else {
            return Ok(false);
        };
        let spec = self.plan.owners[admission.owner.index()]
            .loop_spec()
            .ok_or_else(|| {
                graph_error(
                    admission.instance.address().owner(),
                    "a Graph admission lost its Loop specification",
                )
            })?;
        let fuel = match spec.fuel.as_ref().map(|fuel| &fuel.resolved) {
            Some(crate::value::Value::Int(value)) if *value > 0 => *value as u64,
            Some(crate::value::Value::Uint(value)) if *value > 0 => *value,
            None if spec.proof.is_some() => super::loop_manager::entry_measure_budget(
                spec.proof.as_ref().ok_or_else(|| {
                    graph_error(
                        admission.instance.address().owner(),
                        "a Loop admission lost its measure",
                    )
                })?,
                &admission.delivery.envelope,
                &admission.instance,
            )?,
            _ => {
                return Err(graph_error(
                    admission.instance.address().owner(),
                    "Graph Loop requires a positive retained fuel bound",
                ));
            }
        };
        let record = super::loop_manager::manager_record(
            &admission.instance,
            admission.delivery.envelope.meta().sys().id(),
            Packet {
                context: self.graph_context(&admission.delivery).to_vec(),
                facts: vec![Fact::Open {
                    run: protocol::run_ref(&admission.instance, &admission.opening),
                    instance: admission.instance.clone(),
                    opening: admission.opening,
                    entry: admission.delivery.port,
                    fuel,
                    state: admission.delivery.envelope,
                }],
                ..Packet::default()
            },
        );
        journal.commit(record).map_err(VmError::Journal)?;
        self.refresh_loop_manager(journal)?;
        Ok(true)
    }

    pub(super) fn graph_admission_ready(&self) -> bool {
        self.next_graph_admission().is_some()
    }

    pub(super) fn graph_admission_is_head(&self, run: LoopRunRef) -> bool {
        let Some(graph) = self.graph_loops.as_deref() else {
            return true;
        };
        let Some(admission) = graph
            .admissions
            .values()
            .find(|admission| protocol::run_ref(&admission.instance, &admission.opening) == run)
        else {
            return true;
        };
        !graph.admissions.values().any(|earlier| {
            earlier.instance == admission.instance
                && earlier.delivery.edge == admission.delivery.edge
                && earlier.delivery.source_instance == admission.delivery.source_instance
                && earlier.delivery.seq < admission.delivery.seq
        })
    }

    fn next_graph_admission(&self) -> Option<&Admission> {
        self.graph_loops.as_deref().and_then(|graph| {
            graph.admissions.values().find(|admission| {
                self.instances
                    .get(&admission.instance)
                    .is_some_and(|instance| matches!(instance.phase, InstancePhase::Open))
                    && self.loop_manager.as_deref().is_none_or(|fold| {
                        fold.run(protocol::run_ref(&admission.instance, &admission.opening))
                            .is_none()
                    })
            })
        })
    }

    pub(super) fn admit_graph_loop(
        &mut self,
        journal: &mut dyn Journal,
        owner: OwnerSlot,
        run: LoopRunRef,
        current: &protocol::RunState,
        envelope: Envelope,
    ) -> Result<bool, VmError> {
        let source = Endpoint {
            owner: EndpointOwner::Graph(self.plan.owners[owner.index()].path.clone()),
            port: current.entry.clone(),
        };
        let mut candidates = BTreeMap::new();
        let routes = self.prepare_graph_routes(
            journal,
            &source,
            Some(&current.instance),
            &envelope,
            &mut candidates,
            true,
        )?;
        match self.check_room_for_routes(core::iter::once(routes.as_slice())) {
            Err(VmError::MailboxFull { .. } | VmError::RecurrenceQueueFull { .. }) => {
                return Ok(false);
            }
            result => result?,
        }
        let mut record = super::loop_manager::manager_record(
            &current.instance,
            envelope.meta().sys().id(),
            Packet {
                context: current.enclosing.clone(),
                facts: vec![Fact::Admit {
                    run,
                    iteration: current.iteration,
                    admission: current.admission,
                    envelope: envelope.clone(),
                }],
                ..Packet::default()
            },
        );
        let opening = if current.iteration == 0 {
            let admission = self
                .graph_loops
                .as_deref()
                .and_then(|graph| {
                    graph.admissions.iter().find(|(_, admission)| {
                        protocol::run_ref(&admission.instance, &admission.opening) == run
                    })
                })
                .ok_or_else(|| {
                    graph_error(
                        current.instance.address().owner(),
                        "a Graph opening lost its retained admission",
                    )
                })?;
            record.consumed.push(crate::journal::ConsumedPosition {
                edge: admission.1.delivery.edge.clone(),
                source: admission.1.delivery.source_instance.clone(),
                seq: admission.1.delivery.seq,
            });
            Some(*admission.0)
        } else {
            None
        };
        record.routes = routes
            .iter()
            .filter_map(|route| {
                route.target.as_ref().map(|target| RoutedDelivery {
                    delivery: envelope.meta().sys().id(),
                    edge: self.prepared_edge_text(route),
                    target: target.reference.clone(),
                })
            })
            .collect();
        let context: Vec<_> = current
            .enclosing
            .iter()
            .copied()
            .chain(core::iter::once(run))
            .collect();
        let contexts = self.graph_route_contexts(&context, &routes, envelope.meta().sys().id());
        if let Some(packet) = record.loop_record.as_deref_mut() {
            packet.routes = contexts.clone();
            packet.routes.sort_by_key(|route| route.delivery);
        }
        self.retain_timer_arms(
            &mut record,
            self.route_timer_arms(&routes, &candidates, &envelope),
        )?;
        journal.commit(record).map_err(VmError::Journal)?;
        if let Some(graph) = self.graph_loops.as_deref_mut() {
            if let Some(opening) = opening {
                graph.admissions.remove(&opening);
                graph.contexts.remove(&opening);
            }
            graph.contexts.extend(
                contexts
                    .into_iter()
                    .map(|route| (route.delivery, route.runs)),
            );
        }
        self.route_prepared(routes, &candidates, &envelope, Some(&current.instance));
        self.refresh_loop_manager(journal)?;
        Ok(true)
    }

    pub(super) fn graph_emission_contexts(
        &self,
        record: &crate::journal::TransactionRecord,
        emissions: &[PreparedEmission],
    ) -> Vec<protocol::RoutedContext> {
        let Some(packet) = record.loop_protocol() else {
            return Vec::new();
        };
        let mut contexts = Vec::new();
        for emission in emissions {
            contexts.extend(self.graph_route_contexts(
                &packet.context,
                &emission.routes,
                emission.emission.envelope.meta().sys().id(),
            ));
        }
        contexts.sort_by_key(|route| route.delivery);
        contexts
    }

    fn graph_route_contexts(
        &self,
        context: &[LoopRunRef],
        routes: &[PreparedEdge],
        envelope: Id,
    ) -> Vec<protocol::RoutedContext> {
        let mut contexts = Vec::new();
        for route in routes {
            let Some(target) = &route.target else {
                continue;
            };
            let runs: Vec<_> = context
                .iter()
                .copied()
                .filter(|run| {
                    self.loop_manager
                        .as_deref()
                        .and_then(|fold| fold.run(*run))
                        .is_some_and(|state| target.chain.contains(&state.instance))
                })
                .collect();
            if !runs.is_empty() {
                contexts.push(protocol::RoutedContext {
                    delivery: ids::delivery_ref(&self.prepared_edge_text(route), envelope),
                    runs,
                });
            }
        }
        contexts.sort_by_key(|route| route.delivery);
        contexts
    }

    pub(super) fn retain_graph_verdicts(
        &self,
        record: &mut crate::journal::TransactionRecord,
    ) -> Result<(), VmError> {
        if self.graph_loops.is_none() {
            return Ok(());
        }
        let context = record
            .loop_protocol()
            .map_or(&[][..], |packet| packet.context.as_slice())
            .to_vec();
        let owner = self
            .plan
            .owners
            .iter()
            .find(|owner| owner.path == record.owner_path)
            .ok_or_else(|| graph_error(&record.owner_path, "a producer has no declared owner"))?;
        let produced = self.graph_verdicts(
            &context,
            record.emissions.iter().map(|emission| {
                (
                    Endpoint {
                        owner: match owner.kind {
                            OwnerKind::Graph => EndpointOwner::Graph(owner.path.clone()),
                            OwnerKind::Node(slot) => {
                                EndpointOwner::Node(self.plan.nodes[slot.index()].path.clone())
                            }
                        },
                        port: emission.port.clone(),
                    },
                    &emission.envelope,
                )
            }),
        )?;
        if !produced.is_empty() {
            let Some(packet) = record.loop_record.as_deref_mut() else {
                return Err(graph_error(
                    &record.owner_path,
                    "a Graph verdict lost its application context",
                ));
            };
            packet.produced.extend(produced);
        }
        Ok(())
    }

    fn graph_verdicts<'e>(
        &self,
        context: &[LoopRunRef],
        sources: impl IntoIterator<Item = (Endpoint, &'e Envelope)>,
    ) -> Result<Vec<protocol::CommandProduced>, VmError> {
        let mut produced = Vec::new();
        let mut terminals = BTreeMap::new();
        for (source, envelope) in sources {
            let selected = self.select_graph_routes(&source, envelope)?;
            for command in selected.commands {
                let EndpointOwner::Graph(owner) = command.owner else {
                    continue;
                };
                let (run, state) = context
                    .iter()
                    .find_map(|run| {
                        self.loop_manager
                            .as_deref()
                            .and_then(|fold| fold.run(*run))
                            .filter(|state| *state.instance.address().owner() == owner)
                            .map(|state| (*run, state))
                    })
                    .ok_or_else(|| {
                        graph_error(
                            &owner,
                            "a manager command has no exact consumed Loop context",
                        )
                    })?;
                let value = match command.kind {
                    crate::plan::LoopCommandKind::Next => {
                        protocol::CommandValue::Next(envelope.clone())
                    }
                    crate::plan::LoopCommandKind::Again => {
                        protocol::CommandValue::Again(envelope.meta().sys().id())
                    }
                    crate::plan::LoopCommandKind::Done => protocol::CommandValue::Done {
                        cause: envelope.meta().sys().id(),
                        kind: protocol::DoneKind::Explicit,
                    },
                };
                produced.push(protocol::CommandProduced::new(run, state.iteration, value));
            }
            for owner in selected.terminals {
                terminals.entry(owner).or_insert(envelope.meta().sys().id());
            }
        }
        for (owner, cause) in terminals {
            let path = &self.plan.owners[owner.index()].path;
            let (run, state) = context
                .iter()
                .find_map(|run| {
                    self.loop_manager
                        .as_deref()
                        .and_then(|fold| fold.run(*run))
                        .filter(|state| state.instance.address().owner() == path)
                        .map(|state| (*run, state))
                })
                .ok_or_else(|| {
                    graph_error(
                        path,
                        "a terminal boundary crossing has no exact Loop context",
                    )
                })?;
            produced.push(protocol::CommandProduced::new(
                run,
                state.iteration,
                protocol::CommandValue::Done {
                    cause,
                    kind: protocol::DoneKind::Natural,
                },
            ));
        }
        Ok(produced)
    }

    pub(super) fn graph_entry_verdict(
        &self,
        run: LoopRunRef,
        current: &protocol::RunState,
    ) -> Result<Option<crate::journal::TransactionRecord>, VmError> {
        if current.phase != protocol::Phase::Active
            || current.pending.is_some()
            || !self
                .instances
                .get(&current.instance)
                .is_some_and(|instance| matches!(instance.phase, InstancePhase::Open))
            || !self.plan.owners.iter().any(|owner| {
                owner.path == *current.instance.address().owner() && owner.kind == OwnerKind::Graph
            })
        {
            return Ok(None);
        }
        let context: Vec<_> = current
            .enclosing
            .iter()
            .copied()
            .chain(core::iter::once(run))
            .collect();
        let produced = self.graph_verdicts(
            &context,
            core::iter::once((
                Endpoint {
                    owner: EndpointOwner::Graph(current.instance.address().owner().clone()),
                    port: current.entry.clone(),
                },
                &current.state,
            )),
        )?;
        if produced.is_empty() {
            return Ok(None);
        }
        Ok(Some(super::loop_manager::manager_record(
            &current.instance,
            current.state.meta().sys().id(),
            Packet {
                context,
                produced,
                ..Packet::default()
            },
        )))
    }

    pub(super) fn install_graph_contexts(&mut self, contexts: Vec<protocol::RoutedContext>) {
        if self.graph_loops.is_none() {
            return;
        }
        let mut live: BTreeSet<_> = self
            .scheduler
            .mailboxes
            .queues
            .values()
            .flatten()
            .chain(self.pending.values().map(|pending| &pending.delivery))
            .map(|delivery| ids::delivery_ref(&delivery.edge, delivery.envelope.meta().sys().id()))
            .collect();
        // Consuming an attempt releases its mailbox delivery, not the enclosing
        // Loop obligation. Keep the opening context until the operation closes.
        if let Some(fold) = self.retry_manager.as_deref() {
            live.extend(fold.operations().filter_map(|(_, state)| {
                (state.phase != crate::journal::retry_protocol::Phase::Closed)
                    .then_some(state.opening)
            }));
        }
        if let Some(graph) = self.graph_loops.as_deref_mut() {
            graph.contexts.retain(|reference, _| {
                live.contains(reference) || graph.admissions.contains_key(reference)
            });
            graph.contexts.extend(
                contexts
                    .into_iter()
                    .map(|route| (route.delivery, route.runs)),
            );
        }
    }

    pub(super) fn prepare_graph_emissions(
        &self,
        journal: &(impl InstanceAdmissions + ?Sized),
        owner: OwnerSlot,
        instance: &InstanceRef,
        record: &crate::journal::TransactionRecord,
    ) -> Result<PreparedEmissions, VmError> {
        let mut candidates = BTreeMap::new();
        let mut emissions = Vec::new();
        let mut commands = Vec::new();
        for emission in &record.emissions {
            let source = Endpoint {
                owner: EndpointOwner::Graph(self.plan.owners[owner.index()].path.clone()),
                port: emission.port.clone(),
            };
            commands.extend(self.prepare_instance_commands(
                journal,
                &source,
                Some(instance),
                &emission.envelope,
            )?);
            emissions.push(PreparedEmission {
                emission: emission.clone(),
                routes: self.prepare_graph_routes(
                    journal,
                    &source,
                    Some(instance),
                    &emission.envelope,
                    &mut candidates,
                    true,
                )?,
            });
        }
        self.check_room_for_routes(emissions.iter().map(|emission| emission.routes.as_slice()))?;
        commands.sort_by_key(|command| command.id);
        self.check_instance_command_room(journal, &commands)?;
        Ok(PreparedEmissions {
            commands,
            contexts: self.graph_emission_contexts(record, &emissions),
            emissions,
            candidates,
        })
    }

    pub(super) fn route_graph_emissions(
        &mut self,
        owner: OwnerSlot,
        instance: &InstanceRef,
        prepared: PreparedEmissions,
    ) {
        self.enqueue_instance_commands(prepared.commands);
        for emission in prepared.emissions {
            if self.plan.owners[owner.index()].path.is_root() {
                self.outputs.push(BoundaryOutput {
                    port: emission.emission.port.clone(),
                    envelope: emission.emission.envelope.clone(),
                });
            }
            self.observed.push(Observed {
                node: node::Path::new(
                    self.plan.owners[owner.index()]
                        .path
                        .segments()
                        .iter()
                        .cloned(),
                )
                .ok(),
                port: emission.emission.port.clone(),
                envelope: emission.emission.envelope.clone(),
            });
            self.route_prepared(
                emission.routes,
                &prepared.candidates,
                &emission.emission.envelope,
                Some(instance),
            );
        }
        self.install_graph_contexts(prepared.contexts);
    }

    pub(super) fn restore_graph_manager(
        &mut self,
        journal: &dyn Journal,
        admissions: &impl InstanceAdmissions,
        index: usize,
        owner: OwnerSlot,
        record: &crate::journal::TransactionRecord,
        resume_at: i64,
    ) -> Result<(), VmError> {
        let invalid = |reason| VmError::InvalidRecovery {
            record: index,
            reason,
        };
        let packet = record
            .loop_protocol()
            .ok_or_else(|| invalid("a Graph manager record has no retained protocol"))?;
        if packet.facts.is_empty() {
            let run = packet
                .context
                .last()
                .ok_or_else(|| invalid("an entry verdict has no application context"))?;
            let state = self
                .loop_manager
                .as_deref()
                .and_then(|fold| fold.run(*run))
                .ok_or_else(|| invalid("an entry verdict has no admitted run"))?;
            if self.graph_entry_verdict(*run, state)?.as_ref() != Some(record) {
                return Err(invalid(
                    "a Graph entry verdict differs from its admitted source routes",
                ));
            }
            return Ok(());
        }
        if record.trigger != node::Trigger::Lifecycle(node::Lifecycle::Next)
            || record.parent_instance.is_some()
            || record.close_scope.is_some()
            || !record.state_delta.is_empty()
            || !record.join_inputs.is_empty()
            || packet.facts.len() != 1
        {
            return Err(invalid(
                "a Graph manager record carries application work or competing transitions",
            ));
        }
        let mut derived = record.clone();
        if let Some(packet) = derived.loop_record.as_deref_mut() {
            packet.produced.clear();
        }
        self.retain_graph_verdicts(&mut derived)?;
        if derived.loop_protocol().map(|packet| &packet.produced) != Some(&packet.produced) {
            return Err(invalid("a manager emission changed its enclosing verdicts"));
        }
        match &packet.facts[0] {
            Fact::Open {
                run,
                instance,
                opening,
                entry,
                fuel,
                state,
            } => {
                let admission = self
                    .graph_loops
                    .as_deref()
                    .and_then(|graph| {
                        graph.admissions.values().find(|admission| {
                            admission.instance == *instance && admission.opening == *opening
                        })
                    })
                    .ok_or_else(|| {
                        invalid("a Graph Open has no exact retained boundary admission")
                    })?;
                let spec = self.plan.owners[owner.index()]
                    .loop_spec()
                    .ok_or_else(|| invalid("a Graph opening has no Loop specification"))?;
                let expected = match spec.fuel.as_ref().map(|fuel| &fuel.resolved) {
                    Some(crate::value::Value::Int(fuel)) if *fuel > 0 => Some(*fuel as u64),
                    Some(crate::value::Value::Uint(fuel)) if *fuel > 0 => Some(*fuel),
                    None => spec
                        .proof
                        .as_ref()
                        .map(|proof| {
                            super::loop_manager::entry_measure_budget(
                                proof,
                                &admission.delivery.envelope,
                                &admission.instance,
                            )
                        })
                        .transpose()?,
                    _ => None,
                };
                if *run != protocol::run_ref(instance, opening)
                    || packet.context != self.graph_context(&admission.delivery)
                    || expected != Some(*fuel)
                    || *entry != admission.delivery.port
                    || *state != admission.delivery.envelope
                    || record.cause != Some(state.meta().sys().id())
                    || !record.consumed.is_empty()
                    || !record.routes.is_empty()
                    || !record.emissions.is_empty()
                    || !packet.routes.is_empty()
                    || !packet.consumed.is_empty()
                    || !record.timers.is_empty()
                    || record.timer_schedule.is_some()
                {
                    return Err(invalid(
                        "a Graph Open contradicts its admission or plan fuel",
                    ));
                }
            }
            Fact::Admit {
                run,
                iteration,
                admission,
                envelope,
            } => {
                let current = self
                    .loop_manager
                    .as_deref()
                    .and_then(|fold| fold.run(*run))
                    .ok_or_else(|| invalid("a Graph Admit has no retained opening"))?;
                if *iteration != current.iteration
                    || packet.context != current.enclosing
                    || *admission != current.admission
                    || !self.graph_admission_is_head(*run)
                    || record.cause != Some(envelope.meta().sys().id())
                    || !record.emissions.is_empty()
                    || !packet.consumed.is_empty()
                    || !record.timers.is_empty()
                    || current.due_at.is_some_and(|due| due > resume_at)
                    || *envelope
                        != super::loop_manager::application_envelope(
                            journal,
                            self.graph_version,
                            current,
                        )?
                {
                    return Err(invalid(
                        "a Graph Admit contradicts its retained state or deadline",
                    ));
                }
                let current = current.clone();
                if let Some(due) = current.due_at {
                    self.now = self.now.max(due);
                }
                let opening = if *iteration == 0 {
                    let opening = self
                        .graph_loops
                        .as_deref()
                        .and_then(|graph| {
                            graph.admissions.iter().find(|(_, admission)| {
                                protocol::run_ref(&admission.instance, &admission.opening) == *run
                            })
                        })
                        .ok_or_else(|| {
                            invalid("a Graph Admit has no unconsumed boundary delivery")
                        })?;
                    let expected = crate::journal::ConsumedPosition {
                        edge: opening.1.delivery.edge.clone(),
                        source: opening.1.delivery.source_instance.clone(),
                        seq: opening.1.delivery.seq,
                    };
                    if record.consumed.as_slice() != [expected] {
                        return Err(invalid(
                            "a Graph Admit does not consume its exact boundary cursor",
                        ));
                    }
                    Some(*opening.0)
                } else {
                    if !record.consumed.is_empty() {
                        return Err(invalid(
                            "a recurrent Graph Admit cannot consume another opening",
                        ));
                    }
                    None
                };
                let source = Endpoint {
                    owner: EndpointOwner::Graph(self.plan.owners[owner.index()].path.clone()),
                    port: current.entry.clone(),
                };
                let mut candidates = BTreeMap::new();
                let routes = self.prepare_graph_routes(
                    admissions,
                    &source,
                    Some(&record.instance),
                    envelope,
                    &mut candidates,
                    true,
                )?;
                self.check_room_for_routes(core::iter::once(routes.as_slice()))?;
                let mut expected: Vec<_> = routes
                    .iter()
                    .filter_map(|route| {
                        route.target.as_ref().map(|target| RoutedDelivery {
                            delivery: envelope.meta().sys().id(),
                            edge: self.prepared_edge_text(route),
                            target: target.reference.clone(),
                        })
                    })
                    .collect();
                expected.sort();
                let context: Vec<_> = current
                    .enclosing
                    .iter()
                    .copied()
                    .chain(core::iter::once(*run))
                    .collect();
                let contexts =
                    self.graph_route_contexts(&context, &routes, envelope.meta().sys().id());
                if record.routes != expected || packet.routes != contexts {
                    return Err(invalid(
                        "Graph expansion disagrees with its retained routes or contexts",
                    ));
                }
                self.check_recovered_delivery_timers(
                    index,
                    record.timer_schedule.as_deref(),
                    &self.route_timer_arms(&routes, &candidates, envelope),
                )?;
                if let Some(graph) = self.graph_loops.as_deref_mut() {
                    if let Some(opening) = opening {
                        graph.admissions.remove(&opening);
                        graph.contexts.remove(&opening);
                    }
                    graph.contexts.extend(
                        contexts
                            .into_iter()
                            .map(|route| (route.delivery, route.runs)),
                    );
                }
                self.route_prepared(routes, &candidates, envelope, Some(&record.instance));
            }
            fact => {
                if matches!(fact, Fact::Expired { .. })
                    && !matches!(record.emissions.as_slice(), [emission]
                    if self.plan.owners[owner.index()].loop_spec().and_then(|spec| spec.expired.as_ref())
                        .is_some_and(|target| target.port == emission.port))
                {
                    return Err(invalid(
                        "Loop exhaustion changed its resolved host terminal",
                    ));
                }
                let run = fact.run();
                let current = self
                    .loop_manager
                    .as_deref()
                    .and_then(|fold| fold.run(run))
                    .ok_or_else(|| invalid("a Graph transition has no retained run"))?;
                let command = current
                    .pending
                    .as_ref()
                    .ok_or_else(|| invalid("a Graph transition has no produced command"))?;
                if self.graph_run_has_work(run)
                    || packet.context != current.enclosing
                    || !record.consumed.is_empty()
                    || record.cause != Some(command.value.cause())
                    || packet.consumed.as_slice() != [command.reference]
                {
                    return Err(invalid(
                        "a Graph command was consumed before causal quiescence",
                    ));
                }
                let mut arms = Vec::new();
                if let Fact::Advance {
                    to_iteration,
                    due_at,
                    command,
                    ..
                } = fact
                {
                    let expected = self
                        .loop_delay_for(owner, *to_iteration)?
                        .map(|delay| self.now.saturating_add(delay));
                    if *due_at != expected {
                        return Err(invalid("a Graph recurrence changed its logical delay"));
                    }
                    if let Some(due_at) = due_at {
                        arms.push(crate::journal::timing::TimerArm {
                            kind: crate::journal::timer_kind::LOOP_DELAY,
                            reference: protocol::admission_ref(run, *to_iteration, *command),
                            due_at: *due_at,
                        });
                    }
                }
                let prepared =
                    self.prepare_graph_emissions(admissions, owner, &record.instance, record)?;
                let mut expected = self.routed_deliveries(&prepared);
                expected.sort();
                if expected != record.routes
                    || record.instance_commands != Self::retained_command_packet(&prepared.commands)
                    || packet.routes != prepared.contexts
                    || (!matches!(fact, Fact::Expired { .. }) && !record.emissions.is_empty())
                {
                    return Err(invalid("a Graph transition changed its expiry routes"));
                }
                arms.extend(self.emission_timer_arms(&prepared));
                if record
                    .timer_schedule
                    .as_deref()
                    .map_or(&[][..], |schedule| schedule.arms())
                    != arms
                {
                    return Err(invalid(
                        "Graph transition deadlines contradict its retained obligations",
                    ));
                }
                if let Fact::Advance {
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
                            run,
                            iteration: *to_iteration,
                            state: protocol::admission_ref(run, *to_iteration, *command),
                        },
                    );
                }
                self.route_graph_emissions(owner, &record.instance, prepared);
            }
        }
        Ok(())
    }
}

fn endpoint_text(endpoint: &Endpoint) -> String {
    match &endpoint.owner {
        EndpointOwner::Node(path) => format!("{path}.{}", endpoint.port),
        EndpointOwner::Graph(path) if path.is_root() => format!(".{}", endpoint.port),
        EndpointOwner::Graph(path) => format!("{path}.{}", endpoint.port),
    }
}

fn graph_error(owner: &OwnerPath, reason: &'static str) -> VmError {
    VmError::InvalidLoopDecision {
        owner: owner.clone(),
        reason,
    }
}
