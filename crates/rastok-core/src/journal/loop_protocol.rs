//! Loop r2 causal facts. Command production and consumption are distinct so
//! recovery never needs to re-run a committed producer to recover its verdict.

use super::LoopRunId;
use crate::envelope::{Envelope, Id};
use crate::instance;

use crate::node::Port;
use alloc::vec::Vec;

mod fold;
pub use fold::{Error, Fold, Phase, RunState};

/// Exact admission which opens a run, not its possibly shared business origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpeningAdmissionId {
    /// Public input admitted by one INGRESS fact.
    Ingress {
        /// Retained root envelope.
        inbound: Id,
        /// Public input selected by that admission.
        port: Port,
    },
    /// One envelope delivered along one particular spatial edge.
    Delivery(Id),
}

/// One command obligation, independently addressable after a crash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CommandId(Id);

impl CommandId {
    /// Raw identity for journal projection and correlation.
    #[must_use]
    pub const fn id(self) -> Id {
        self.0
    }
}

/// Retained state or cause supplied by a command producer.
#[derive(Debug, Clone, PartialEq)]
pub enum CommandValue {
    /// Complete replacement state envelope, not just its DATA.
    Next(Envelope),
    /// Keep the run's state; the supplied envelope is only the cause.
    Again(Id),
    /// End the run without a business emission.
    Done {
        /// Input or terminal envelope which selected completion.
        cause: Id,
        /// A terminal host output and an explicit command retain their distinction.
        kind: DoneKind,
    },
}

/// Provenance of Done, without adding another source-language command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoneKind {
    /// A designated host output or Call terminal.
    Natural,
    /// Authored `Loop.done`.
    Explicit,
}

impl DoneKind {
    /// Stable provenance tag within the Done payload.
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Natural => 1,
            Self::Explicit => 2,
        }
    }
}

impl CommandValue {
    /// Closed, extend-only command tag.
    #[must_use]
    pub const fn tag(&self) -> u8 {
        match self {
            Self::Next(_) => 1,
            Self::Again(_) => 2,
            Self::Done { .. } => 3,
        }
    }

    /// Exact source envelope, distinct from its origin lineage.
    #[must_use]
    pub fn cause(&self) -> Id {
        match self {
            Self::Next(state) => state.meta().sys().id(),
            Self::Again(cause) | Self::Done { cause, .. } => *cause,
        }
    }
}

/// A command produced atomically with its originating transaction.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandProduced {
    /// Derived command identity, verified against the other fields.
    pub reference: CommandId,
    /// Run selected by the consumed delivery's Loop context.
    pub run: LoopRunId,
    /// Zero-based application which issued the command.
    pub iteration: u64,
    /// The retained verdict.
    pub value: CommandValue,
}

impl CommandProduced {
    /// Constructs the command and its identity together.
    #[must_use]
    pub fn new(run: LoopRunId, iteration: u64, value: CommandValue) -> Self {
        let reference = command_ref(run, iteration, &value);
        Self {
            reference,
            run,
            iteration,
            value,
        }
    }
}

/// Why a run closed without exhaustion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// A designated output or unredirected Call terminal completed the run.
    Natural,
    /// The application issued `Loop.done`.
    ExplicitDone,
    /// Closing or `stop` ended the owner instance.
    Stopped,
}

/// Transition of the manager's replay-derived state.
#[derive(Debug, Clone, PartialEq)]
pub enum Fact {
    /// Retain one admission before its first application can execute.
    Open {
        /// Derived run identity.
        run: LoopRunId,
        /// Exact owner generation.
        instance: instance::Generation,
        /// The admission disambiguating sibling deliveries of one origin.
        opening: OpeningAdmissionId,
        /// The host input to which every recurrence returns.
        entry: Port,
        /// Positive total application budget, including the first.
        fuel: u64,
        /// Complete initial state.
        state: Envelope,
    },
    /// A consumed recurrence command selected a new state; admission is later.
    Advance {
        /// Run being advanced.
        run: LoopRunId,
        /// Application which produced the command.
        from_iteration: u64,
        /// Following application ordinal.
        to_iteration: u64,
        /// Remaining application budget.
        remaining: u64,
        /// Consumed command, never inferred from a business origin.
        command: CommandId,
        /// Proposed state for Next, retained state for Again.
        state: Envelope,
        /// Explicit scheduler annotation, outside causal identity.
        due_at: Option<i64>,
    },
    /// Capacity was proved and the current state entered the host.
    Admit {
        /// Admitted run.
        run: LoopRunId,
        /// Admitted application ordinal.
        iteration: u64,
        /// Exact initial or recurrent admission identity.
        admission: Id,
        /// Entry envelope. Recurrence retains DATA and user metadata but
        /// mints a new causal identity; initial admission retains its root.
        envelope: Envelope,
    },
    /// No more applications of this run will execute.
    Close {
        /// Closed run.
        run: LoopRunId,
        /// Last application ordinal.
        iteration: u64,
        /// Natural completion, explicit Done, or owner closure.
        reason: CloseReason,
    },
    /// A recurrence would exceed fuel; no following Admit exists.
    Expired {
        /// Exhausted run.
        run: LoopRunId,
        /// Last permitted application ordinal.
        iteration: u64,
        /// Consumed recurrence command.
        command: CommandId,
        /// Proposed Next state or retained Again state.
        last_state: Envelope,
        /// Manager-produced `LoopExpired` envelope on the resolved host terminal.
        output: Envelope,
    },
}

impl Fact {
    /// The run whose state this fact changes.
    #[must_use]
    pub const fn run(&self) -> LoopRunId {
        match self {
            Self::Open { run, .. }
            | Self::Advance { run, .. }
            | Self::Admit { run, .. }
            | Self::Close { run, .. }
            | Self::Expired { run, .. } => *run,
        }
    }
}

/// Active runs on one routed delivery, outermost first. The envelope and its
/// origin id remain unchanged when routing enters or leaves a Loop scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutedContext {
    /// Exact `ids::delivery_id(edge, envelope)` identity.
    pub delivery: Id,
    /// Context selected before committing the producer.
    pub runs: Vec<LoopRunId>,
}

/// Sparse Loop contribution to an ordinary TRANSACTION. Its presence does not
/// create another ledger; ordinary transactions carry no allocation for it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Packet {
    /// Context inherited from the consumed admission.
    pub context: Vec<LoopRunId>,
    /// Contexts of outgoing deliveries, sorted by delivery reference.
    pub routes: Vec<RoutedContext>,
    /// Commands in deterministic producer order.
    pub produced: Vec<CommandProduced>,
    /// Commands consumed by this manager transaction, sorted by identity.
    pub consumed: Vec<CommandId>,
    /// Manager transitions in commit order.
    pub facts: Vec<Fact>,
}

impl Packet {
    /// State envelopes retained by this packet, in producer/fact order.
    pub fn envelopes(&self) -> impl Iterator<Item = &Envelope> {
        self.produced
            .iter()
            .filter_map(|command| match &command.value {
                CommandValue::Next(envelope) => Some(envelope),
                CommandValue::Again(_) | CommandValue::Done { .. } => None,
            })
            .chain(self.facts.iter().flat_map(|fact| {
                let (first, second) = match fact {
                    Fact::Open { state, .. } | Fact::Advance { state, .. } => (Some(state), None),
                    Fact::Expired {
                        last_state, output, ..
                    } => (Some(last_state), Some(output)),
                    Fact::Admit { envelope, .. } => (Some(envelope), None),
                    Fact::Close { .. } => (None, None),
                };
                first.into_iter().chain(second)
            }))
    }

    /// Empty packets have no canonical presence in the journal.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.context.is_empty()
            && self.routes.is_empty()
            && self.produced.is_empty()
            && self.consumed.is_empty()
            && self.facts.is_empty()
    }
}

/// `rastok.loop-run.r2`: instance plus the exact opening admission.
#[must_use]
pub fn run_ref(instance: &instance::Generation, opening: &OpeningAdmissionId) -> LoopRunId {
    let (tag, reference, port) = match opening {
        OpeningAdmissionId::Ingress { inbound, port } => (1, *inbound, port.name().as_bytes()),
        OpeningAdmissionId::Delivery(delivery) => (2, *delivery, &[][..]),
    };
    LoopRunId::new(Id::derive(
        crate::envelope::IdKind::LoopRun,
        &[
            instance.address().as_bytes(),
            &instance.generation().to_le_bytes(),
            &[tag],
            reference.as_bytes(),
            port,
        ],
    ))
}

/// `rastok.loop-command.r1`: one verdict obligation for one application.
#[must_use]
pub fn command_ref(run: LoopRunId, iteration: u64, value: &CommandValue) -> CommandId {
    let provenance = match value {
        CommandValue::Done { kind, .. } => kind.tag(),
        _ => 0,
    };
    CommandId(Id::derive(
        crate::envelope::IdKind::LoopCommand,
        &[
            run.id().as_bytes(),
            &iteration.to_le_bytes(),
            &[value.tag(), provenance],
            value.cause().as_bytes(),
        ],
    ))
}

/// `rastok.loop-admission.r1`: temporal admission caused by one consumed command.
#[must_use]
pub fn admission_ref(run: LoopRunId, iteration: u64, command: CommandId) -> Id {
    Id::derive(
        crate::envelope::IdKind::LoopAdmission,
        &[
            run.id().as_bytes(),
            &iteration.to_le_bytes(),
            command.id().as_bytes(),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node;

    #[test]
    fn sibling_deliveries_of_one_origin_open_distinct_runs() {
        let owner = node::OwnerPath::new(["loop"]).unwrap();
        let instance = instance::Generation::new(instance::Identity::singleton(&owner), 0);
        let envelope = Id::derive(crate::envelope::IdKind::Root, &[b"same-origin"]);
        let left =
            OpeningAdmissionId::Delivery(super::super::ids::delivery_id("left>loop.in", envelope));
        let right =
            OpeningAdmissionId::Delivery(super::super::ids::delivery_id("right>loop.in", envelope));
        let root = OpeningAdmissionId::Ingress {
            inbound: envelope,
            port: Port::new("in").unwrap(),
        };
        let other_port = OpeningAdmissionId::Ingress {
            inbound: envelope,
            port: Port::new("request").unwrap(),
        };
        let refs = [
            run_ref(&instance, &left),
            run_ref(&instance, &right),
            run_ref(&instance, &root),
            run_ref(&instance, &other_port),
        ];
        for (index, reference) in refs.iter().enumerate() {
            assert!(!refs[..index].contains(reference));
            assert_ne!(reference.id(), envelope);
        }
        let successor = instance::Generation::new(instance.address().clone(), 1);
        assert_ne!(run_ref(&instance, &left), run_ref(&successor, &left));
        assert_eq!(run_ref(&instance, &left), refs[0]);
    }

    #[test]
    fn command_and_admission_domains_distinguish_verdict_iteration_and_cause() {
        let run = LoopRunId::new(Id::derive(crate::envelope::IdKind::Root, &[b"run"]));
        let cause = Id::derive(crate::envelope::IdKind::Root, &[b"cause"]);
        let again = command_ref(run, 0, &CommandValue::Again(cause));
        let done = command_ref(
            run,
            0,
            &CommandValue::Done {
                cause,
                kind: DoneKind::Explicit,
            },
        );
        assert_ne!(
            done,
            command_ref(
                run,
                0,
                &CommandValue::Done {
                    cause,
                    kind: DoneKind::Natural
                }
            )
        );
        assert_ne!(again, done);
        assert_ne!(again, command_ref(run, 1, &CommandValue::Again(cause)));
        assert_ne!(again, command_ref(run, 0, &CommandValue::Again(run.id())));
        let admission = admission_ref(run, 1, again);
        assert_ne!(admission, again.id());
        assert_ne!(admission, run.id());
        assert_ne!(admission, admission_ref(run, 2, again));
        assert_ne!(admission, admission_ref(run, 1, done));
    }
}
