//! Loop run state reconstructed from committed journal transactions.
//! Spatial quiescence and host admission are checked by the plan and Worker.

use super::{
    CommandProduced, CommandValue, Fact, LoopRecord, OpeningAdmissionId, admission_ref,
    command_ref, run_ref,
};
use crate::envelope::{Envelope, Id};
use crate::instance;

use crate::journal::{LoopRunId, Record, TransactionRecord};
use crate::node::{Lifecycle, Port, Trigger};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use core::fmt;

/// A rejected Loop record. No part of the record changes the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// An empty, redundant or non-canonical extension.
    RecordShape,
    /// A context repeats a run or names a run which is not active.
    Context,
    /// The carrying Executor or manager is outside the run's exact instance scope.
    ContextOwner,
    /// A routed context has no unique delivery in its accepting transaction.
    ContextDelivery,
    /// An expiry fact is not published as the same ordinary output emission.
    ExpiryEmission,
    /// An Opened fact's identity, entry or application budget contradicts its fields.
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
            Self::RecordShape => "Loop record is empty or non-canonical",
            Self::Context => "Loop context is repeated, unknown, or inactive",
            Self::ContextOwner => "Loop context or manager fact belongs to another instance scope",
            Self::ContextDelivery => "Loop routed context has no unique retained delivery",
            Self::ExpiryEmission => "Loop expiry fact has no matching manager emission",
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
pub struct Run {
    /// Exact enclosing applications retained by the opening transaction.
    pub enclosing: Vec<LoopRunId>,
    /// Exact owner generation.
    pub instance: instance::Generation,
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

/// A read-only view of Loop runs for consumers, rebuilt by folding committed
/// transactions. Its update methods validate and reconstruct that view; the
/// journal remains authoritative. No Driver, scheduler or clock is consulted.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoopView {
    runs: BTreeMap<LoopRunId, Run>,
    /// Active parent-generation links from lifecycle records. An owner path
    /// alone cannot distinguish tenants or successive enclosing generations.
    parents: BTreeMap<instance::Generation, Option<instance::Generation>>,
}

impl LoopView {
    /// Reconstructs every r2 run from committed transactions.
    ///
    /// # Errors
    /// The first contradictory record, without returning partial state.
    pub fn from_records(records: &[Record]) -> Result<Self, Error> {
        let mut fold = Self::default();
        for record in records {
            if let Record::Transaction(transaction) = record {
                fold.apply_transaction(transaction)?;
            }
        }
        Ok(fold)
    }

    /// Current state by exact run id, not business origin.
    #[must_use]
    pub fn run(&self, run: LoopRunId) -> Option<&Run> {
        self.runs.get(&run)
    }

    /// Retained runs in canonical identity order.
    pub fn runs(&self) -> impl Iterator<Item = (&LoopRunId, &Run)> {
        self.runs.iter()
    }

    /// Checks a Loop record without the transaction that carries it.
    /// Journal acceptance and replay must use [`Self::apply_transaction`] to
    /// also establish instance scope, routed deliveries and published expiry.
    ///
    /// # Errors
    /// A malformed record or invalid transition. `self` is unchanged.
    pub fn apply(&mut self, packet: &LoopRecord) -> Result<(), Error> {
        let mut candidate = self.clone();
        candidate.apply_inner(packet)?;
        *self = candidate;
        Ok(())
    }

    /// Applies lifecycle links and the transaction's Loop record atomically. Context
    /// scope follows exact parent-generation facts, never owner-name prefixes.
    ///
    /// This is not a spatial consumption or quiescence proof: those additionally
    /// need the plan and the retained per-delivery context/cursor fold.
    ///
    /// # Errors
    /// A malformed protocol, foreign instance context, unretained route or
    /// unpublished expiry. No part of `self` changes on refusal.
    pub fn apply_transaction(&mut self, record: &TransactionRecord) -> Result<(), Error> {
        let start = record.trigger == Trigger::Lifecycle(Lifecycle::Start);
        let stop = record.trigger == Trigger::Lifecycle(Lifecycle::Stop);
        if record.loop_protocol().is_none() && !start && !stop {
            return Ok(());
        }
        let mut candidate = self.clone();
        if start {
            if candidate.parents.contains_key(&record.instance)
                || record
                    .parent_instance
                    .as_ref()
                    .is_some_and(|parent| !candidate.parents.contains_key(parent))
            {
                return Err(Error::ContextOwner);
            }
            candidate
                .parents
                .insert(record.instance.clone(), record.parent_instance.clone());
        }
        if let Some(packet) = record.loop_protocol() {
            candidate.context(&packet.context)?;
            candidate.context_chain(&packet.context)?;
            for run in &packet.context {
                let state = candidate.runs.get(run).ok_or(Error::UnknownRun)?;
                if !candidate.within(&record.instance, &state.instance) {
                    return Err(Error::ContextOwner);
                }
            }
            candidate.apply_inner(packet)?;
            for fact in &packet.facts {
                let state = candidate.runs.get(&fact.run()).ok_or(Error::UnknownRun)?;
                if state.instance != record.instance
                    || !candidate.parents.contains_key(&record.instance)
                {
                    return Err(Error::ContextOwner);
                }
                if let Fact::Expired { output, .. } = fact {
                    let mut emissions = record.emissions.iter().filter(|emission| {
                        emission.envelope.meta().sys().id() == output.meta().sys().id()
                    });
                    if !emissions.next().is_some_and(|emission| {
                        emission.index == 0
                            && emission.envelope == *output
                            && record.cause.is_some_and(|cause| {
                                output.meta().sys().id()
                                    == crate::journal::ids::graph_emission_id(
                                        record.instance.address().owner(),
                                        &Trigger::Lifecycle(Lifecycle::Loop),
                                        cause,
                                        &emission.port,
                                        0,
                                    )
                            })
                    }) || emissions.next().is_some()
                    {
                        return Err(Error::ExpiryEmission);
                    }
                }
            }
            for route in &packet.routes {
                candidate.context_chain(&route.runs)?;
                let mut retained = record.routes.iter().filter(|delivery| {
                    crate::journal::ids::delivery_id(&delivery.edge, delivery.delivery)
                        == route.delivery
                });
                if retained.next().is_none() || retained.next().is_some() {
                    return Err(Error::ContextDelivery);
                }
            }
        }
        if stop {
            candidate.parents.remove(&record.instance);
        }
        *self = candidate;
        Ok(())
    }

    fn within(&self, instance: &instance::Generation, scope: &instance::Generation) -> bool {
        let mut cursor = instance;
        for _ in 0..self.parents.len() {
            let Some(parent) = self.parents.get(cursor) else {
                return false;
            };
            if cursor == scope {
                return true;
            }
            let Some(parent) = parent else { return false };
            cursor = parent;
        }
        false
    }

    fn context_chain(&self, context: &[LoopRunId]) -> Result<(), Error> {
        for pair in context.windows(2) {
            let outer = self.runs.get(&pair[0]).ok_or(Error::UnknownRun)?;
            let inner = self.runs.get(&pair[1]).ok_or(Error::UnknownRun)?;
            if outer.instance == inner.instance || !self.within(&inner.instance, &outer.instance) {
                return Err(Error::ContextOwner);
            }
        }
        Ok(())
    }

    fn context(&self, context: &[LoopRunId]) -> Result<(), Error> {
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

    fn apply_inner(&mut self, packet: &LoopRecord) -> Result<(), Error> {
        if packet.is_empty()
            || !packet.consumed.windows(2).all(|pair| pair[0] < pair[1])
            || !packet
                .routes
                .windows(2)
                .all(|pair| pair[0].delivery < pair[1].delivery)
        {
            return Err(Error::RecordShape);
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
            if let Fact::Opened {
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
                    OpeningAdmissionId::Ingress { inbound, port } => {
                        if port != entry || *inbound != state.meta().sys().id() {
                            return Err(Error::OpenIdentity);
                        }
                        *inbound
                    }
                    OpeningAdmissionId::Delivery(delivery) => *delivery,
                };
                if self.runs.contains_key(run) {
                    return Err(Error::DuplicateRun);
                }
                self.runs.insert(
                    *run,
                    Run {
                        enclosing: packet.context.clone(),
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
                Fact::Opened { .. } => return Err(Error::DuplicateRun),
                Fact::Admitted {
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
                Fact::Advanced {
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
                        || !expiry_matches(produced.value.cause(), last_state, output)
                    {
                        return Err(Error::Transition);
                    }
                    state.phase = Phase::Expired;
                }
                Fact::Closed {
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

fn expiry_matches(cause: Id, state: &Envelope, output: &Envelope) -> bool {
    let sys = output.meta().sys();
    if sys.parent_id() != Some(cause)
        || sys.origin_id() != state.meta().sys().origin_id()
        || sys.graph_version() != state.meta().sys().graph_version()
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
    // The payload states which logical reading it carries. Check that reading
    // against the original bytes; the plan-bound replay separately checks the
    // host terminal's declared type. Raw UTF-8 is not implicitly text here.
    let expected = match actual {
        crate::value::Value::Text(_) => {
            crate::runtime::DataSlot::typed(state.data(), &crate::datatype::DataType::Text)
                .materialize()
        }
        _ => crate::runtime::data_value(state.data()),
    };
    value.case == "LoopExpired"
        && field == "last_state"
        && expected.as_ref().is_ok_and(|expected| expected == actual)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::{Data, Meta, MetaSys, Value};
    use crate::node;

    use crate::journal::TransactionRecord;
    use crate::node::{Lifecycle, Trigger};

    use alloc::{boxed::Box, vec, vec::Vec};

    fn state(value: i64) -> Envelope {
        let root = MetaSys::root(
            Id::derive(crate::envelope::IdKind::Root, &[&2_i64.to_le_bytes()]),
            1,
        );
        let sys = if value == 2 {
            root
        } else {
            MetaSys::emission(
                Id::derive(crate::envelope::IdKind::Root, &[&value.to_le_bytes()]),
                &root,
                1,
            )
        };
        Envelope::new(Meta::new(sys), Data::materialized(Value::Int(value)))
    }

    fn open(fuel: u64) -> (LoopRunId, instance::Generation, Vec<LoopRecord>) {
        let instance = instance::Generation::new(
            instance::InstanceAddress::singleton(&node::OwnerPath::new(["host"]).unwrap()),
            0,
        );
        open_at(instance, fuel)
    }

    fn open_at(
        instance: instance::Generation,
        fuel: u64,
    ) -> (LoopRunId, instance::Generation, Vec<LoopRecord>) {
        let entry = Port::new("request").unwrap();
        let state = state(2);
        let opening = OpeningAdmissionId::Ingress {
            inbound: state.meta().sys().id(),
            port: entry.clone(),
        };
        let run = run_ref(&instance, &opening);
        let packets = vec![
            LoopRecord {
                facts: vec![Fact::Opened {
                    run,
                    instance: instance.clone(),
                    opening,
                    entry,
                    fuel,
                    state: state.clone(),
                }],
                ..LoopRecord::default()
            },
            LoopRecord {
                facts: vec![Fact::Admitted {
                    run,
                    iteration: 0,
                    admission: state.meta().sys().id(),
                    envelope: state.clone(),
                }],
                ..LoopRecord::default()
            },
        ];
        (run, instance, packets)
    }

    fn record(instance: &instance::Generation, packet: LoopRecord) -> Record {
        Record::Transaction(Box::new(TransactionRecord {
            owner_path: instance.address().owner().clone(),
            instance: instance.clone(),
            parent_instance: None,
            close_scope: None,
            trigger: Trigger::Lifecycle(Lifecycle::Loop),
            cause: None,
            exchange_exit: None,
            retry_record: None,
            loop_record: Some(Box::new(packet)),
            logical_time: None,
            deadlines: None,
            instance_commands: None,
            consumed: vec![],
            join_inputs: vec![],
            state_delta: vec![],
            emissions: vec![],
            routes: vec![],
            deadline_ids: vec![],
        }))
    }

    fn start(instance: &instance::Generation, parent: Option<&instance::Generation>) -> Record {
        let Record::Transaction(mut transaction) = record(instance, LoopRecord::default()) else {
            unreachable!()
        };
        transaction.trigger = Trigger::Lifecycle(Lifecycle::Start);
        transaction.parent_instance = parent.cloned();
        transaction.loop_record = None;
        Record::Transaction(transaction)
    }

    fn rooted(instance: &instance::Generation) -> Vec<Record> {
        let root = instance::Generation::new(
            instance::InstanceAddress::singleton(&node::OwnerPath::root()),
            0,
        );
        vec![start(&root, None), start(instance, Some(&root))]
    }

    #[test]
    fn recovery_retains_a_committed_command_without_rerunning_its_producer() {
        let (run, instance, packets) = open(2);
        let mut records = rooted(&instance);
        records.extend(packets.into_iter().map(|packet| record(&instance, packet)));
        let command = CommandProduced::new(run, 0, CommandValue::Next(state(1)));
        records.push(record(
            &instance,
            LoopRecord {
                context: vec![run],
                produced: vec![command.clone()],
                ..LoopRecord::default()
            },
        ));
        let recovered = LoopView::from_records(&records).unwrap();
        assert_eq!(recovered.run(run).unwrap().pending.as_ref(), Some(&command));
        let consumption = LoopRecord {
            consumed: vec![command.reference],
            facts: vec![Fact::Advanced {
                run,
                from_iteration: 0,
                to_iteration: 1,
                remaining: 1,
                command: command.reference,
                state: state(1),
                due_at: Some(100),
            }],
            ..LoopRecord::default()
        };
        let mut live = recovered;
        live.apply(&consumption).unwrap();
        records.push(record(&instance, consumption.clone()));
        assert_eq!(LoopView::from_records(&records).unwrap(), live);
        assert_eq!(live.run(run).unwrap().phase, Phase::AwaitingAdmission);
        assert_eq!(live.run(run).unwrap().due_at, Some(100));
        let checkpoint = live.clone();
        assert_eq!(live.apply(&consumption), Err(Error::UnknownCommand));
        assert_eq!(
            live, checkpoint,
            "repeating consumption cannot advance twice"
        );
        live.apply(&LoopRecord {
            facts: vec![Fact::Admitted {
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
            ..LoopRecord::default()
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
        live.apply(&LoopRecord {
            context: vec![run],
            produced: vec![done.clone()],
            ..LoopRecord::default()
        })
        .unwrap();
        live.apply(&LoopRecord {
            consumed: vec![done.reference],
            facts: vec![Fact::Closed {
                run,
                iteration: 1,
                reason: super::super::CloseReason::Natural,
            }],
            ..LoopRecord::default()
        })
        .unwrap();
        assert_eq!(live.run(run).unwrap().phase, Phase::Closed);
    }

    #[test]
    fn transaction_context_uses_exact_parent_generations_not_owner_prefixes() {
        let (run, instance, packets) = open(2);
        let mut records = rooted(&instance);
        records.extend(packets.into_iter().map(|packet| record(&instance, packet)));
        let child = instance::Generation::new(
            instance::InstanceAddress::singleton(&node::OwnerPath::new(["host", "child"]).unwrap()),
            0,
        );
        records.push(start(&child, Some(&instance)));
        let fold = LoopView::from_records(&records).unwrap();
        let context = LoopRecord {
            context: vec![run],
            ..LoopRecord::default()
        };
        let Record::Transaction(valid) = record(&child, context.clone()) else {
            unreachable!()
        };
        fold.clone().apply_transaction(&valid).unwrap();

        let impostor = instance::Generation::new(child.address().clone(), 1);
        let Record::Transaction(forged) = record(&impostor, context.clone()) else {
            unreachable!()
        };
        let mut attempted = fold.clone();
        assert_eq!(
            attempted.apply_transaction(&forged),
            Err(Error::ContextOwner)
        );
        assert_eq!(attempted, fold);

        let mut closed = start(&instance, None);
        let Record::Transaction(closed) = &mut closed else {
            unreachable!()
        };
        closed.trigger = Trigger::Lifecycle(Lifecycle::Stop);
        let mut expired_parent = fold.clone();
        expired_parent.apply_transaction(closed).unwrap();
        let successor = instance::Generation::new(instance.address().clone(), 1);
        let root = instance::Generation::new(
            instance::InstanceAddress::singleton(&node::OwnerPath::root()),
            0,
        );
        let Record::Transaction(successor_start) = start(&successor, Some(&root)) else {
            unreachable!()
        };
        expired_parent.apply_transaction(&successor_start).unwrap();
        // The child's retained link still names generation zero. A live parent
        // with identical address but generation one cannot repair that link.
        assert_eq!(
            expired_parent.apply_transaction(&valid),
            Err(Error::ContextOwner)
        );
    }

    #[test]
    fn transaction_context_stacks_follow_the_retained_instance_tree() {
        let (outer, parent, packets) = open(2);
        let mut records = rooted(&parent);
        records.extend(packets.into_iter().map(|packet| record(&parent, packet)));
        let child = instance::Generation::new(
            instance::InstanceAddress::singleton(&node::OwnerPath::new(["host", "child"]).unwrap()),
            0,
        );
        records.push(start(&child, Some(&parent)));
        let (inner, _, packets) = open_at(child.clone(), 2);
        records.extend(packets.into_iter().map(|packet| record(&child, packet)));
        let fold = LoopView::from_records(&records).unwrap();
        for (context, expected) in [
            (vec![outer, inner], Ok(())),
            (vec![inner, outer], Err(Error::ContextOwner)),
            (vec![outer, outer], Err(Error::Context)),
        ] {
            let Record::Transaction(transaction) = record(
                &child,
                LoopRecord {
                    context,
                    ..LoopRecord::default()
                },
            ) else {
                unreachable!()
            };
            let mut attempt = fold.clone();
            assert_eq!(attempt.apply_transaction(&transaction), expected);
            assert_eq!(
                attempt, fold,
                "a context-only record changes no protocol or lifecycle state"
            );
        }
    }

    #[test]
    fn a_context_cannot_cross_to_a_sibling_tenants_identical_owner_path() {
        use crate::datatype::DataType;

        let owner = node::OwnerPath::new(["host"]).unwrap();
        let child_owner = node::OwnerPath::new(["host", "child"]).unwrap();
        let tenant = |name: &str, path: &node::OwnerPath| {
            instance::Generation::new(
                instance::InstanceAddress::new(
                    path,
                    &[instance::Segment::SelectedKey {
                        scope: owner.clone(),
                        ty: DataType::Text,
                        value: Value::Text(name.into()),
                    }],
                )
                .unwrap(),
                0,
            )
        };
        let alice = tenant("alice", &owner);
        let bob = tenant("bob", &owner);
        let alice_child = tenant("alice", &child_owner);
        let bob_child = tenant("bob", &child_owner);
        let root = instance::Generation::new(
            instance::InstanceAddress::singleton(&node::OwnerPath::root()),
            0,
        );
        let (run, _, packets) = open_at(alice.clone(), 2);
        let mut records = vec![
            start(&root, None),
            start(&alice, Some(&root)),
            start(&bob, Some(&root)),
            start(&alice_child, Some(&alice)),
            start(&bob_child, Some(&bob)),
        ];
        records.extend(packets.into_iter().map(|packet| record(&alice, packet)));
        let fold = LoopView::from_records(&records).unwrap();
        for (actor, expected) in [
            (&alice_child, Ok(())),
            (&bob_child, Err(Error::ContextOwner)),
            (&bob, Err(Error::ContextOwner)),
        ] {
            let Record::Transaction(transaction) = record(
                actor,
                LoopRecord {
                    context: vec![run],
                    ..LoopRecord::default()
                },
            ) else {
                unreachable!()
            };
            let mut attempted = fold.clone();
            assert_eq!(attempted.apply_transaction(&transaction), expected);
            assert_eq!(attempted, fold);
        }
    }

    #[test]
    fn routed_context_names_the_exact_retained_edge_and_envelope_occurrence() {
        let (run, instance, packets) = open(2);
        let mut records = rooted(&instance);
        records.extend(packets.into_iter().map(|packet| record(&instance, packet)));
        let fold = LoopView::from_records(&records).unwrap();
        let target = instance::Generation::new(
            instance::InstanceAddress::singleton(&node::OwnerPath::new(["host", "child"]).unwrap()),
            0,
        );
        let envelope = state(1);
        let delivery = crate::journal::RoutedDelivery {
            delivery: envelope.meta().sys().id(),
            edge: "source.out>child.in".into(),
            target,
        };
        let context = super::super::RoutedContext {
            delivery: crate::journal::ids::delivery_id(&delivery.edge, delivery.delivery),
            runs: vec![run],
        };
        let Record::Transaction(mut transaction) = record(
            &instance,
            LoopRecord {
                context: vec![run],
                routes: vec![context],
                ..LoopRecord::default()
            },
        ) else {
            unreachable!()
        };
        transaction.routes.push(delivery);
        fold.clone().apply_transaction(&transaction).unwrap();
        for alteration in 0..3 {
            let mut forged = transaction.clone();
            match alteration {
                0 => forged.routes[0].edge = "other.out>child.in".into(),
                1 => forged.routes[0].delivery = state(0).meta().sys().id(),
                _ => forged.routes.push(forged.routes[0].clone()),
            }
            let mut attempt = fold.clone();
            assert_eq!(
                attempt.apply_transaction(&forged),
                Err(Error::ContextDelivery)
            );
            assert_eq!(attempt, fold);
        }
    }

    #[test]
    fn consumption_and_transition_are_atomic_and_again_cannot_replace_state() {
        let (run, _, packets) = open(2);
        let mut fold = LoopView::default();
        for packet in packets {
            fold.apply(&packet).unwrap();
        }
        let command = CommandProduced::new(
            run,
            0,
            CommandValue::Again(Id::derive(crate::envelope::IdKind::Root, &[b"exit"])),
        );
        let production = LoopRecord {
            context: vec![run],
            produced: vec![command.clone()],
            ..LoopRecord::default()
        };
        fold.apply(&production).unwrap();
        let before = fold.clone();
        assert_eq!(fold.apply(&production), Err(Error::DuplicateVerdict));
        assert_eq!(fold, before);
        assert_eq!(
            fold.apply(&LoopRecord {
                consumed: vec![command.reference],
                ..LoopRecord::default()
            }),
            Err(Error::UnappliedCommand)
        );
        assert_eq!(fold, before);
        let mut advance = LoopRecord {
            consumed: vec![command.reference],
            facts: vec![Fact::Advanced {
                run,
                from_iteration: 0,
                to_iteration: 1,
                remaining: 1,
                command: command.reference,
                state: state(1),
                due_at: None,
            }],
            ..LoopRecord::default()
        };
        assert_eq!(fold.apply(&advance), Err(Error::Transition));
        assert_eq!(fold, before);
        let Fact::Advanced { state: next, .. } = &mut advance.facts[0] else {
            panic!("advance")
        };
        *next = state(2);
        fold.apply(&advance).unwrap();
        assert_eq!(fold.run(run).unwrap().state, state(2));
    }

    #[test]
    fn again_retains_business_state_but_readmission_cannot_repeat_emission_identity() {
        let (run, _, packets) = open(3);
        let mut fold = LoopView::default();
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
            fold.apply(&LoopRecord {
                context: vec![run],
                produced: vec![command.clone()],
                ..LoopRecord::default()
            })
            .unwrap();
            fold.apply(&LoopRecord {
                consumed: vec![command.reference],
                facts: vec![Fact::Advanced {
                    run,
                    from_iteration: iteration,
                    to_iteration: iteration + 1,
                    remaining: 2 - iteration,
                    command: command.reference,
                    state: current.clone(),
                    due_at: None,
                }],
                ..LoopRecord::default()
            })
            .unwrap();
            let admission = admission_ref(run, iteration + 1, command.reference);
            let mut packet = LoopRecord {
                facts: vec![Fact::Admitted {
                    run,
                    iteration: iteration + 1,
                    admission,
                    envelope: current.clone(),
                }],
                ..LoopRecord::default()
            };
            let checkpoint = fold.clone();
            assert_eq!(fold.apply(&packet), Err(Error::Transition));
            assert_eq!(fold, checkpoint);
            let Fact::Admitted { envelope, .. } = &mut packet.facts[0] else {
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
        let mut fold = LoopView::default();
        for packet in packets {
            fold.apply(&packet).unwrap();
        }
        let command = CommandProduced::new(run, 0, CommandValue::Next(state(1)));
        fold.apply(&LoopRecord {
            context: vec![run],
            produced: vec![command.clone()],
            ..LoopRecord::default()
        })
        .unwrap();
        let checkpoint = fold.clone();
        assert_eq!(
            fold.apply(&LoopRecord {
                consumed: vec![command.reference],
                facts: vec![Fact::Advanced {
                    run,
                    from_iteration: 0,
                    to_iteration: 1,
                    remaining: 0,
                    command: command.reference,
                    state: state(1),
                    due_at: None
                }],
                ..LoopRecord::default()
            }),
            Err(Error::Transition)
        );
        assert_eq!(fold, checkpoint);
        let mut expiry = LoopRecord {
            consumed: vec![command.reference],
            facts: vec![Fact::Expired {
                run,
                iteration: 0,
                command: command.reference,
                last_state: state(1),
                output: state(-1),
            }],
            ..LoopRecord::default()
        };
        assert_eq!(
            fold.apply(&expiry),
            Err(Error::Transition),
            "an arbitrary envelope is not evidence of LoopExpired"
        );
        assert_eq!(fold, checkpoint);
        let id = crate::journal::ids::graph_emission_id(
            instance.address().owner(),
            &Trigger::Lifecycle(Lifecycle::Loop),
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
            fold.apply(&LoopRecord {
                facts: vec![Fact::Admitted {
                    run,
                    iteration: 1,
                    admission: admission_ref(run, 1, command.reference),
                    envelope: state(1),
                }],
                ..LoopRecord::default()
            }),
            Err(Error::Transition)
        );
    }

    #[test]
    fn unknown_contexts_and_command_field_forgery_are_rejected() {
        let (run, _, packets) = open(2);
        let mut fold = LoopView::default();
        for packet in packets {
            fold.apply(&packet).unwrap();
        }
        let mut command =
            CommandProduced::new(run, 0, CommandValue::Again(state(2).meta().sys().id()));
        command.iteration = 1;
        assert_eq!(
            fold.apply(&LoopRecord {
                context: vec![run],
                produced: vec![command],
                ..LoopRecord::default()
            }),
            Err(Error::CommandIdentity)
        );
        assert_eq!(
            fold.apply(&LoopRecord {
                context: vec![run, run],
                ..LoopRecord::default()
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
        fold.apply(&LoopRecord {
            context: vec![run],
            produced: vec![command.clone()],
            ..LoopRecord::default()
        })
        .unwrap();
        fold.apply(&LoopRecord {
            consumed: vec![command.reference],
            facts: vec![Fact::Closed {
                run,
                iteration: 0,
                reason: super::super::CloseReason::ExplicitDone,
            }],
            ..LoopRecord::default()
        })
        .unwrap();
        assert_eq!(
            fold.apply(&LoopRecord {
                context: vec![run],
                produced: vec![command],
                ..LoopRecord::default()
            }),
            Err(Error::Context)
        );
    }
}
