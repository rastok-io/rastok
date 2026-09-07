//! Protocol reconstruction only. Spatial quiescence and host/effect admission
//! are obligations of the plan and Worker, not facts this fold invents.

use super::{
    CommandProduced, CommandValue, Fact, OpeningAdmissionRef, Packet, admission_ref, command_ref,
    run_ref,
};
use crate::envelope::{Envelope, Id};
use crate::identity::InstanceRef;
use crate::journal::{LoopRunRef, Record};
use crate::node::Port;
use alloc::collections::{BTreeMap, BTreeSet};
use core::fmt;

/// A rejected causal packet. No part of the packet changes the fold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// An empty, redundant or non-canonical extension.
    PacketShape,
    /// A context repeats a run or names a run which is not active.
    Context,
    /// Open identity, entry or application budget contradicts its fields.
    OpenIdentity,
    /// One opening was recorded twice.
    DuplicateRun,
    /// A transition names no retained run.
    UnknownRun,
    /// A command's id does not derive from its fields.
    CommandIdentity,
    /// An application produced more than one verdict command.
    DuplicateVerdict,
    /// A consumption has no matching unconsumed command.
    UnknownCommand,
    /// A phase, ordinal, budget or state transition contradicts the fold.
    Transition,
    /// A manager consumed a command without applying its verdict.
    UnappliedCommand,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::PacketShape => "Loop packet is empty or non-canonical",
            Self::Context => "Loop context is repeated, unknown, or inactive",
            Self::OpenIdentity => "Loop opening contradicts its admission, entry, or fuel",
            Self::DuplicateRun => "Loop admission opens the same run twice",
            Self::UnknownRun => "Loop transition has no opening fact",
            Self::CommandIdentity => "Loop command identity contradicts its fields",
            Self::DuplicateVerdict => "Loop application already has a verdict command",
            Self::UnknownCommand => "Loop command is unknown or already consumed",
            Self::Transition => "Loop transition contradicts its phase, ordinal, fuel, or state",
            Self::UnappliedCommand => "Loop consumption has no corresponding manager transition",
        })
    }
}

/// One application's progress, distinct from the owner instance's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// State is retained; capacity has not yet admitted it to the host.
    AwaitingAdmission,
    /// The host application is active.
    Active,
    /// A natural/explicit verdict or owner closure ended the run.
    Closed,
    /// A requested recurrence exhausted the budget.
    Expired,
}

/// Reconstructible state of one run, including an unconsumed command.
#[derive(Debug, Clone, PartialEq)]
pub struct RunState {
    /// Exact owner generation.
    pub instance: InstanceRef,
    /// Recurrent host input.
    pub entry: Port,
    /// Current zero-based application.
    pub iteration: u64,
    /// Applications still available, including this one.
    pub remaining: u64,
    /// Complete application state.
    pub state: Envelope,
    /// Exact admission for this application.
    pub admission: Id,
    /// Source of the recurrence command, retained until the next admission.
    pub reentry_cause: Option<Id>,
    /// Derived due instant; no spatial capacity is represented by it.
    pub due_at: Option<i64>,
    /// Current application phase.
    pub phase: Phase,
    /// Producer committed, manager has not consumed; recovery requeues this.
    pub pending: Option<CommandProduced>,
}

/// Pure replay fold, with no Driver, scheduler, clock or additional ledger.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fold {
    runs: BTreeMap<LoopRunRef, RunState>,
}

impl Fold {
    /// Reconstructs every r2 run from committed transactions.
    ///
    /// # Errors
    /// The first contradictory packet, without returning partial state.
    pub fn from_records(records: &[Record]) -> Result<Self, Error> {
        let mut fold = Self::default();
        for record in records {
            if let Record::Transaction(transaction) = record
                && let Some(packet) = transaction.loop_protocol()
            {
                fold.apply(packet)?;
            }
        }
        Ok(fold)
    }

    /// Current state by exact run id, not business origin.
    #[must_use]
    pub fn run(&self, run: LoopRunRef) -> Option<&RunState> {
        self.runs.get(&run)
    }

    /// Retained runs in canonical identity order.
    pub fn runs(&self) -> impl Iterator<Item = (&LoopRunRef, &RunState)> {
        self.runs.iter()
    }

    /// Applies a complete commit packet atomically.
    ///
    /// # Errors
    /// A malformed packet or invalid transition. `self` is unchanged.
    pub fn apply(&mut self, packet: &Packet) -> Result<(), Error> {
        let mut candidate = self.clone();
        candidate.apply_inner(packet)?;
        *self = candidate;
        Ok(())
    }

    fn context(&self, context: &[LoopRunRef]) -> Result<(), Error> {
        let mut unique = BTreeSet::new();
        for run in context {
            if !unique.insert(*run)
                || !self
                    .runs
                    .get(run)
                    .is_some_and(|state| state.phase == Phase::Active)
            {
                return Err(Error::Context);
            }
        }
        Ok(())
    }

    fn apply_inner(&mut self, packet: &Packet) -> Result<(), Error> {
        if packet.is_empty()
            || !packet.consumed.windows(2).all(|pair| pair[0] < pair[1])
            || !packet
                .routes
                .windows(2)
                .all(|pair| pair[0].delivery < pair[1].delivery)
        {
            return Err(Error::PacketShape);
        }
        self.context(&packet.context)?;
        for command in &packet.produced {
            if command.reference != command_ref(command.run, command.iteration, &command.value) {
                return Err(Error::CommandIdentity);
            }
            let state = self.runs.get_mut(&command.run).ok_or(Error::UnknownRun)?;
            if state.phase != Phase::Active
                || state.iteration != command.iteration
                || !packet.context.contains(&command.run)
            {
                return Err(Error::Transition);
            }
            if state.pending.is_some() {
                return Err(Error::DuplicateVerdict);
            }
            state.pending = Some(command.clone());
        }
        let mut consumed = BTreeMap::new();
        for reference in &packet.consumed {
            let state = self
                .runs
                .values_mut()
                .find(|state| {
                    state
                        .pending
                        .as_ref()
                        .is_some_and(|command| command.reference == *reference)
                })
                .ok_or(Error::UnknownCommand)?;
            let command = state.pending.take().ok_or(Error::UnknownCommand)?;
            consumed.insert(*reference, command);
        }
        for fact in &packet.facts {
            if let Fact::Open {
                run,
                instance,
                opening,
                entry,
                fuel,
                state,
            } = fact
            {
                if *fuel == 0 || *run != run_ref(instance, opening) {
                    return Err(Error::OpenIdentity);
                }
                let admission = match opening {
                    OpeningAdmissionRef::Ingress { inbound, port } => {
                        if port != entry || *inbound != state.meta().sys().id() {
                            return Err(Error::OpenIdentity);
                        }
                        *inbound
                    }
                    OpeningAdmissionRef::Delivery(delivery) => *delivery,
                };
                if self.runs.contains_key(run) {
                    return Err(Error::DuplicateRun);
                }
                self.runs.insert(
                    *run,
                    RunState {
                        instance: instance.clone(),
                        entry: entry.clone(),
                        iteration: 0,
                        remaining: *fuel,
                        state: state.clone(),
                        admission,
                        reentry_cause: None,
                        due_at: None,
                        phase: Phase::AwaitingAdmission,
                        pending: None,
                    },
                );
                continue;
            }
            let state = self.runs.get_mut(&fact.run()).ok_or(Error::UnknownRun)?;
            match fact {
                Fact::Open { .. } => return Err(Error::DuplicateRun),
                Fact::Admit {
                    iteration,
                    admission,
                    envelope,
                    ..
                } => {
                    if state.phase != Phase::AwaitingAdmission
                        || state.iteration != *iteration
                        || state.admission != *admission
                    {
                        return Err(Error::Transition);
                    }
                    let valid_entry = match state.reentry_cause {
                        None => *iteration == 0 && envelope == &state.state,
                        Some(cause) => {
                            let actual = envelope.meta();
                            let retained = state.state.meta();
                            *iteration > 0
                                && actual.sys().id() == *admission
                                && actual.sys().parent_id() == Some(cause)
                                && actual.sys().origin_id() == retained.sys().origin_id()
                                && actual.sys().graph_version() == retained.sys().graph_version()
                                && actual.sys().role() == crate::envelope::Role::Default
                                && actual.app() == retained.app()
                                && actual.ext() == retained.ext()
                                && envelope.data() == state.state.data()
                        }
                    };
                    if !valid_entry {
                        return Err(Error::Transition);
                    }
                    state.state = envelope.clone();
                    state.reentry_cause = None;
                    state.phase = Phase::Active;
                    state.due_at = None;
                }
                Fact::Advance {
                    run,
                    from_iteration,
                    to_iteration,
                    remaining,
                    command,
                    state: next,
                    due_at,
                } => {
                    let produced = consumed.remove(command).ok_or(Error::UnknownCommand)?;
                    if state.phase != Phase::Active
                        || state.iteration != *from_iteration
                        || from_iteration.checked_add(1) != Some(*to_iteration)
                        || state.remaining <= 1
                        || state.remaining.checked_sub(1) != Some(*remaining)
                        || produced.run != *run
                        || produced.iteration != *from_iteration
                        || next_state(&produced.value, &state.state)? != next
                    {
                        return Err(Error::Transition);
                    }
                    state.iteration = *to_iteration;
                    state.remaining = *remaining;
                    state.state = next.clone();
                    state.admission = admission_ref(*run, *to_iteration, *command);
                    state.reentry_cause = Some(produced.value.cause());
                    state.due_at = *due_at;
                    state.phase = Phase::AwaitingAdmission;
                }
                Fact::Expired {
                    run,
                    iteration,
                    command,
                    last_state,
                    output,
                } => {
                    let produced = consumed.remove(command).ok_or(Error::UnknownCommand)?;
                    if state.phase != Phase::Active
                        || state.iteration != *iteration
                        || state.remaining != 1
                        || produced.run != *run
                        || produced.iteration != *iteration
                        || next_state(&produced.value, &state.state)? != last_state
                        || !expiry_matches(
                            &state.instance,
                            produced.value.cause(),
                            last_state,
                            output,
                        )
                    {
                        return Err(Error::Transition);
                    }
                    state.phase = Phase::Expired;
                }
                Fact::Close {
                    run,
                    iteration,
                    reason,
                } => {
                    if state.iteration != *iteration
                        || matches!(state.phase, Phase::Closed | Phase::Expired)
                    {
                        return Err(Error::Transition);
                    }
                    match reason {
                        super::CloseReason::Natural | super::CloseReason::ExplicitDone
                            if state.phase == Phase::Active && state.pending.is_none() =>
                        {
                            let expected = match reason {
                                super::CloseReason::Natural => super::DoneKind::Natural,
                                super::CloseReason::ExplicitDone => super::DoneKind::Explicit,
                                super::CloseReason::Stopped => return Err(Error::Transition),
                            };
                            let reference = consumed
                                .iter()
                                .find_map(|(reference, command)| {
                                    (command.run == *run
                                        && command.iteration == *iteration
                                        && matches!(command.value, CommandValue::Done { kind, .. } if kind == expected))
                                    .then_some(*reference)
                                })
                                .ok_or(Error::UnknownCommand)?;
                            consumed.remove(&reference);
                        }
                        super::CloseReason::Stopped => {
                            state.pending = None;
                        }
                        _ => return Err(Error::Transition),
                    }
                    state.phase = Phase::Closed;
                }
            }
        }
        if !consumed.is_empty() {
            return Err(Error::UnappliedCommand);
        }
        for route in &packet.routes {
            self.context(&route.runs)?;
        }
        Ok(())
    }
}

fn next_state<'a>(
    command: &'a CommandValue,
    retained: &'a Envelope,
) -> Result<&'a Envelope, Error> {
    match command {
        CommandValue::Next(next) => Ok(next),
        CommandValue::Again(_) => Ok(retained),
        CommandValue::Done { .. } => Err(Error::Transition),
    }
}

fn expiry_matches(instance: &InstanceRef, cause: Id, state: &Envelope, output: &Envelope) -> bool {
    let Ok(port) = Port::new("Loop.expired") else {
        return false;
    };
    let id = crate::journal::ids::graph_emission_id(
        instance.address().owner(),
        &crate::node::Trigger::Lifecycle(crate::node::Lifecycle::Next),
        cause,
        &port,
        0,
    );
    let sys = output.meta().sys();
    if sys.id() != id
        || sys.parent_id() != Some(cause)
        || sys.origin_id() != state.meta().sys().origin_id()
        || sys.graph_version() != state.meta().sys().graph_version()
        || sys.role() != crate::envelope::Role::Default
        || output.meta().app() != state.meta().app()
        || output.meta().ext() != state.meta().ext()
    {
        return false;
    }
    let crate::envelope::Data::Materialized(crate::value::Value::TaggedUnion(value)) =
        output.data()
    else {
        return false;
    };
    let [(field, actual)] = value.payload.as_slice() else {
        return false;
    };
    value.case == "LoopExpired"
        && field == "last_state"
        && crate::runtime::data_value(state.data())
            .as_ref()
            .is_ok_and(|expected| expected == actual)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::{Data, Meta, MetaSys, Value};
    use crate::identity::InstanceAddress;
    use crate::journal::TransactionRecord;
    use crate::node::{Lifecycle, Trigger};
    use crate::plan::OwnerPath;
    use alloc::{boxed::Box, vec, vec::Vec};

    fn state(value: i64) -> Envelope {
        let root = MetaSys::root(Id::derive(1, &[&2_i64.to_le_bytes()]), 1);
        let sys = if value == 2 {
            root
        } else {
            MetaSys::emission(Id::derive(1, &[&value.to_le_bytes()]), &root, 1)
        };
        Envelope::new(Meta::new(sys), Data::materialized(Value::Int(value)))
    }

    fn open(fuel: u64) -> (LoopRunRef, InstanceRef, Vec<Packet>) {
        let instance = InstanceRef::new(
            InstanceAddress::singleton(&OwnerPath::new(["host"]).unwrap()),
            0,
        );
        let entry = Port::new("request").unwrap();
        let state = state(2);
        let opening = OpeningAdmissionRef::Ingress {
            inbound: state.meta().sys().id(),
            port: entry.clone(),
        };
        let run = run_ref(&instance, &opening);
        let packets = vec![
            Packet {
                facts: vec![Fact::Open {
                    run,
                    instance: instance.clone(),
                    opening,
                    entry,
                    fuel,
                    state: state.clone(),
                }],
                ..Packet::default()
            },
            Packet {
                facts: vec![Fact::Admit {
                    run,
                    iteration: 0,
                    admission: state.meta().sys().id(),
                    envelope: state.clone(),
                }],
                ..Packet::default()
            },
        ];
        (run, instance, packets)
    }

    fn record(instance: &InstanceRef, packet: Packet) -> Record {
        Record::Transaction(Box::new(TransactionRecord {
            owner_path: instance.address().owner().clone(),
            instance: instance.clone(),
            parent_instance: None,
            close_scope: None,
            trigger: Trigger::Lifecycle(Lifecycle::Next),
            cause: None,
            exchange_exit: None,
            loop_record: Some(Box::new(crate::journal::LoopRecord::R2(packet))),
            logical_time: None,
            consumed: vec![],
            join_inputs: vec![],
            state_delta: vec![],
            emissions: vec![],
            routes: vec![],
            timers: vec![],
        }))
    }

    #[test]
    fn recovery_retains_a_committed_command_without_rerunning_its_producer() {
        let (run, instance, packets) = open(2);
        let mut records: Vec<_> = packets
            .into_iter()
            .map(|packet| record(&instance, packet))
            .collect();
        let command = CommandProduced::new(run, 0, CommandValue::Next(state(1)));
        records.push(record(
            &instance,
            Packet {
                context: vec![run],
                produced: vec![command.clone()],
                ..Packet::default()
            },
        ));
        let recovered = Fold::from_records(&records).unwrap();
        assert_eq!(recovered.run(run).unwrap().pending.as_ref(), Some(&command));
        let consumption = Packet {
            consumed: vec![command.reference],
            facts: vec![Fact::Advance {
                run,
                from_iteration: 0,
                to_iteration: 1,
                remaining: 1,
                command: command.reference,
                state: state(1),
                due_at: Some(100),
            }],
            ..Packet::default()
        };
        let mut live = recovered;
        live.apply(&consumption).unwrap();
        records.push(record(&instance, consumption.clone()));
        assert_eq!(Fold::from_records(&records).unwrap(), live);
        assert_eq!(live.run(run).unwrap().phase, Phase::AwaitingAdmission);
        assert_eq!(live.run(run).unwrap().due_at, Some(100));
        let checkpoint = live.clone();
        assert_eq!(live.apply(&consumption), Err(Error::UnknownCommand));
        assert_eq!(
            live, checkpoint,
            "repeating consumption cannot advance twice"
        );
        live.apply(&Packet {
            facts: vec![Fact::Admit {
                run,
                iteration: 1,
                admission: admission_ref(run, 1, command.reference),
                envelope: Envelope::new(
                    Meta::new(MetaSys::emission(
                        admission_ref(run, 1, command.reference),
                        state(1).meta().sys(),
                        1,
                    )),
                    state(1).data().clone(),
                ),
            }],
            ..Packet::default()
        })
        .unwrap();
        let done = CommandProduced::new(
            run,
            1,
            CommandValue::Done {
                cause: state(0).meta().sys().id(),
                kind: super::super::DoneKind::Natural,
            },
        );
        live.apply(&Packet {
            context: vec![run],
            produced: vec![done.clone()],
            ..Packet::default()
        })
        .unwrap();
        live.apply(&Packet {
            consumed: vec![done.reference],
            facts: vec![Fact::Close {
                run,
                iteration: 1,
                reason: super::super::CloseReason::Natural,
            }],
            ..Packet::default()
        })
        .unwrap();
        assert_eq!(live.run(run).unwrap().phase, Phase::Closed);
    }

    #[test]
    fn consumption_and_transition_are_atomic_and_again_cannot_replace_state() {
        let (run, _, packets) = open(2);
        let mut fold = Fold::default();
        for packet in packets {
            fold.apply(&packet).unwrap();
        }
        let command = CommandProduced::new(run, 0, CommandValue::Again(Id::derive(1, &[b"exit"])));
        let production = Packet {
            context: vec![run],
            produced: vec![command.clone()],
            ..Packet::default()
        };
        fold.apply(&production).unwrap();
        let before = fold.clone();
        assert_eq!(fold.apply(&production), Err(Error::DuplicateVerdict));
        assert_eq!(fold, before);
        assert_eq!(
            fold.apply(&Packet {
                consumed: vec![command.reference],
                ..Packet::default()
            }),
            Err(Error::UnappliedCommand)
        );
        assert_eq!(fold, before);
        let mut advance = Packet {
            consumed: vec![command.reference],
            facts: vec![Fact::Advance {
                run,
                from_iteration: 0,
                to_iteration: 1,
                remaining: 1,
                command: command.reference,
                state: state(1),
                due_at: None,
            }],
            ..Packet::default()
        };
        assert_eq!(fold.apply(&advance), Err(Error::Transition));
        assert_eq!(fold, before);
        let Fact::Advance { state: next, .. } = &mut advance.facts[0] else {
            panic!("advance")
        };
        *next = state(2);
        fold.apply(&advance).unwrap();
        assert_eq!(fold.run(run).unwrap().state, state(2));
    }

    #[test]
    fn again_retains_business_state_but_readmission_cannot_repeat_emission_identity() {
        let (run, _, packets) = open(3);
        let mut fold = Fold::default();
        for packet in packets {
            fold.apply(&packet).unwrap();
        }
        let host = crate::node::Path::new(["host"]).unwrap();
        let entry = Port::new("request").unwrap();
        let output = Port::new("out").unwrap();
        let trigger = Trigger::Port(entry);
        let initial = fold.run(run).unwrap().state.clone();
        let mut ids = BTreeSet::new();
        for iteration in 0..3 {
            let current = fold.run(run).unwrap().state.clone();
            assert_eq!(current.data(), initial.data());
            assert_eq!(current.meta().app(), initial.meta().app());
            assert_eq!(
                current.meta().sys().origin_id(),
                initial.meta().sys().origin_id()
            );
            assert!(ids.insert(crate::journal::ids::emission_id(
                &host,
                &trigger,
                current.meta().sys().id(),
                &output,
                0,
            )));
            if iteration == 2 {
                break;
            }
            let command = CommandProduced::new(
                run,
                iteration,
                CommandValue::Again(current.meta().sys().id()),
            );
            fold.apply(&Packet {
                context: vec![run],
                produced: vec![command.clone()],
                ..Packet::default()
            })
            .unwrap();
            fold.apply(&Packet {
                consumed: vec![command.reference],
                facts: vec![Fact::Advance {
                    run,
                    from_iteration: iteration,
                    to_iteration: iteration + 1,
                    remaining: 2 - iteration,
                    command: command.reference,
                    state: current.clone(),
                    due_at: None,
                }],
                ..Packet::default()
            })
            .unwrap();
            let admission = admission_ref(run, iteration + 1, command.reference);
            let mut packet = Packet {
                facts: vec![Fact::Admit {
                    run,
                    iteration: iteration + 1,
                    admission,
                    envelope: current.clone(),
                }],
                ..Packet::default()
            };
            let checkpoint = fold.clone();
            assert_eq!(fold.apply(&packet), Err(Error::Transition));
            assert_eq!(fold, checkpoint);
            let Fact::Admit { envelope, .. } = &mut packet.facts[0] else {
                panic!("admission")
            };
            *envelope = Envelope::new(
                Meta::new(MetaSys::emission(admission, current.meta().sys(), 1))
                    .with_app(current.meta().app().clone())
                    .with_ext(current.meta().ext().clone()),
                current.data().clone(),
            );
            fold.apply(&packet).unwrap();
        }
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn exhausted_fuel_publishes_expiry_without_an_admission() {
        let (run, instance, packets) = open(1);
        let mut fold = Fold::default();
        for packet in packets {
            fold.apply(&packet).unwrap();
        }
        let command = CommandProduced::new(run, 0, CommandValue::Next(state(1)));
        fold.apply(&Packet {
            context: vec![run],
            produced: vec![command.clone()],
            ..Packet::default()
        })
        .unwrap();
        let checkpoint = fold.clone();
        assert_eq!(
            fold.apply(&Packet {
                consumed: vec![command.reference],
                facts: vec![Fact::Advance {
                    run,
                    from_iteration: 0,
                    to_iteration: 1,
                    remaining: 0,
                    command: command.reference,
                    state: state(1),
                    due_at: None
                }],
                ..Packet::default()
            }),
            Err(Error::Transition)
        );
        assert_eq!(fold, checkpoint);
        let mut expiry = Packet {
            consumed: vec![command.reference],
            facts: vec![Fact::Expired {
                run,
                iteration: 0,
                command: command.reference,
                last_state: state(1),
                output: state(-1),
            }],
            ..Packet::default()
        };
        assert_eq!(
            fold.apply(&expiry),
            Err(Error::Transition),
            "an arbitrary envelope is not evidence of LoopExpired"
        );
        assert_eq!(fold, checkpoint);
        let id = crate::journal::ids::graph_emission_id(
            instance.address().owner(),
            &Trigger::Lifecycle(Lifecycle::Next),
            command.value.cause(),
            &Port::new("Loop.expired").unwrap(),
            0,
        );
        let Fact::Expired { output, .. } = &mut expiry.facts[0] else {
            panic!("expiry")
        };
        *output = Envelope::new(
            Meta::new(MetaSys::emission(id, state(1).meta().sys(), 1)),
            Data::materialized(Value::TaggedUnion(Box::new(
                crate::value::TaggedUnionValue {
                    case: "LoopExpired".into(),
                    payload: vec![("last_state".into(), Value::Int(1))],
                },
            ))),
        );
        fold.apply(&expiry).unwrap();
        assert_eq!(fold.run(run).unwrap().phase, Phase::Expired);
        assert_eq!(
            fold.apply(&Packet {
                facts: vec![Fact::Admit {
                    run,
                    iteration: 1,
                    admission: admission_ref(run, 1, command.reference),
                    envelope: state(1),
                }],
                ..Packet::default()
            }),
            Err(Error::Transition)
        );
    }

    #[test]
    fn unknown_contexts_and_command_field_forgery_are_rejected() {
        let (run, _, packets) = open(2);
        let mut fold = Fold::default();
        for packet in packets {
            fold.apply(&packet).unwrap();
        }
        let mut command =
            CommandProduced::new(run, 0, CommandValue::Again(state(2).meta().sys().id()));
        command.iteration = 1;
        assert_eq!(
            fold.apply(&Packet {
                context: vec![run],
                produced: vec![command],
                ..Packet::default()
            }),
            Err(Error::CommandIdentity)
        );
        assert_eq!(
            fold.apply(&Packet {
                context: vec![run, run],
                ..Packet::default()
            }),
            Err(Error::Context)
        );
        let command = CommandProduced::new(
            run,
            0,
            CommandValue::Done {
                cause: state(2).meta().sys().id(),
                kind: super::super::DoneKind::Explicit,
            },
        );
        fold.apply(&Packet {
            context: vec![run],
            produced: vec![command.clone()],
            ..Packet::default()
        })
        .unwrap();
        fold.apply(&Packet {
            consumed: vec![command.reference],
            facts: vec![Fact::Close {
                run,
                iteration: 0,
                reason: super::super::CloseReason::ExplicitDone,
            }],
            ..Packet::default()
        })
        .unwrap();
        assert_eq!(
            fold.apply(&Packet {
                context: vec![run],
                produced: vec![command],
                ..Packet::default()
            }),
            Err(Error::Context)
        );
    }
}
