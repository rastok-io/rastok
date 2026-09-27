//! Lifecycle command preparation is pure. The queue below is only a projection
//! of accepted packets; recovery reconstructs it from the same journal prefix.

use super::*;
use crate::journal::instance_stop::{Command, Disposition, Error, Packet};
use crate::plan::{Endpoint, InstanceStopTarget, RouteDestination};

impl Vm<'_> {
    pub(super) fn prepare_instance_commands(
        &self,
        journal: &dyn InstanceAdmissions,
        source: &Endpoint,
        source_instance: Option<&InstanceGeneration>,
        envelope: &Envelope,
    ) -> Result<Vec<Command>, VmError> {
        if self.instance_commands.is_none() {
            return Ok(Vec::new());
        }
        let targets: Vec<InstanceStopTarget> = if self.graph_loops.is_some() {
            self.graph_instance_stop_targets(source, envelope)?
        } else {
            let mut targets = Vec::new();
            let mut value = None;
            for rule in self
                .plan
                .routes
                .iter()
                .filter(|rule| rule.source == *source)
            {
                let set = match &rule.selection {
                    crate::plan::RouteSelection::All(set) => set.as_slice(),
                    crate::plan::RouteSelection::Cases(_) => {
                        if value.is_none() {
                            value = Some(
                                crate::runtime::data_value(envelope.data())
                                    .map_err(VmError::Fault)?,
                            );
                        }
                        rule.selection
                            .selected(value.as_ref().ok_or_else(|| {
                                self.command_error(&OwnerPath::root(), Error::Source)
                            })?)
                            .ok_or_else(|| self.command_error(&OwnerPath::root(), Error::Source))?
                    }
                };
                targets.extend(set.iter().filter_map(|destination| match destination {
                    RouteDestination::InstanceStop(target) => Some(*target.clone()),
                    _ => None,
                }));
            }
            targets
        };
        if targets.is_empty() {
            return Ok(Vec::new());
        }
        let source_instance = if let Some(reference) = source_instance {
            reference.clone()
        } else {
            let owner = match &source.owner {
                crate::plan::EndpointOwner::Graph(path) => path.clone(),
                crate::plan::EndpointOwner::Node(path) => OwnerPath::from(path),
            };
            let slot = self
                .plan
                .owners
                .iter()
                .position(|entry| entry.path == owner)
                .map(|index| OwnerSlot::new(index as u32))
                .ok_or_else(|| self.command_error(&owner, Error::Source))?;
            // This is a pure selection. The bundle is not installed: a target
            // absent from the retained lifecycle prefix still refuses below.
            self.prepare_owner_target(
                journal,
                OwnerSlot::new(0),
                None,
                slot,
                envelope,
                &mut BTreeMap::new(),
            )?
            .reference
        };
        let fold = journal.instance_commands().map_err(VmError::Journal)?;
        let mut commands = Vec::with_capacity(targets.len());
        for target in targets {
            let (scope, target_ref) = fold
                .resolve(&source_instance, &target)
                .map_err(|reason| self.command_error(&target.scope, reason))?;
            commands.push(Command::new(
                source.clone(),
                source_instance.clone(),
                scope,
                target_ref,
                envelope.meta().sys().id(),
            ));
        }
        commands.sort_by_key(|command| command.id);
        Ok(commands)
    }

    fn command_error(&self, owner: &OwnerPath, reason: Error) -> VmError {
        VmError::InstanceCommand {
            owner: owner.clone(),
            reason,
        }
    }

    pub(super) fn check_instance_command_room(
        &self,
        journal: &dyn InstanceAdmissions,
        commands: &[Command],
    ) -> Result<(), VmError> {
        if commands.is_empty() {
            return Ok(());
        }
        let fold = journal.instance_commands().map_err(VmError::Journal)?;
        let occupied = fold
            .commands()
            .filter(|(_, disposition)| *disposition == Disposition::Pending)
            .count();
        let added = commands
            .iter()
            .filter(|command| fold.closing_record(&command.target).is_none())
            .count();
        if occupied.saturating_add(added) > WORKER_MAILBOX_CAPACITY {
            return Err(VmError::InstanceCommandQueueFull);
        }
        Ok(())
    }

    pub(super) fn enqueue_instance_commands(&mut self, commands: Vec<Command>) {
        if let Some(queue) = self.instance_commands.as_mut() {
            queue.extend(commands.into_iter().filter(|command| {
                self.instances
                    .get(&command.target)
                    .is_some_and(|instance| matches!(instance.phase, InstancePhase::Open))
            }));
        }
    }

    pub(super) fn instance_command_ready(&self) -> bool {
        self.instance_commands.as_ref().is_some_and(|queue| {
            queue.iter().any(|command| {
                self.instances
                    .get(&command.target)
                    .is_some_and(|instance| matches!(instance.phase, InstancePhase::Open))
            })
        })
    }

    pub(super) fn step_instance_commands(
        &mut self,
        journal: &mut dyn Journal,
    ) -> Result<bool, VmError> {
        loop {
            let Some(command) = self
                .instance_commands
                .as_ref()
                .and_then(|queue| queue.front())
                .cloned()
            else {
                return Ok(false);
            };
            let active = self
                .instances
                .get(&command.target)
                .is_some_and(|instance| matches!(instance.phase, InstancePhase::Open));
            if active {
                let mut cause = None;
                journal
                    .visit_records(&mut |record| match record {
                        crate::journal::Record::Inbound(record)
                            if record.envelope.meta().sys().id() == command.cause =>
                        {
                            cause = Some(record.envelope.clone())
                        }
                        crate::journal::Record::Transaction(record) => {
                            for emission in &record.emissions {
                                if emission.envelope.meta().sys().id() == command.cause {
                                    cause = Some(emission.envelope.clone());
                                }
                            }
                        }
                        _ => {}
                    })
                    .map_err(VmError::Journal)?;
                let cause = cause.ok_or_else(|| {
                    self.command_error(command.target.address().owner(), Error::Source)
                })?;
                self.request_close(journal, &command.target, cause, Some(command.id))?;
            }
            // Closing/stopped targets are settled by their retained close fact,
            // never by a second acknowledgement on a reclaimed generation.
            if active {
                return Ok(true);
            }
            if let Some(queue) = self.instance_commands.as_mut() {
                queue.pop_front();
            }
        }
    }

    pub(super) fn retained_command_packet(commands: &[Command]) -> Option<Box<Packet>> {
        (!commands.is_empty()).then(|| {
            Box::new(Packet {
                produced: commands.to_vec(),
                consumed: None,
            })
        })
    }
}
