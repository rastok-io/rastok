//! Retained Retry operations: a selected Call outcome is not a Loop command.
//!
//! These are the C1 algebra's typed facts. Package/journal admission must also
//! establish the original plan, exact carrying transaction and spatial context;
//! a successful protocol fold alone is not permission to execute a Driver.

use crate::boundary::{CallRef, PreparedRequest};
use crate::connector::{CallSemantics, ContractId};
use crate::envelope::{Envelope, Id};
use crate::identity::InstanceRef;
use crate::node::Port;
use crate::plan::{LoopDelay, SemanticHash};
use crate::value::Value;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

mod carriers;
pub(crate) mod encoding;
pub(crate) use carriers::validate_transaction;
mod fold;
pub use fold::{Error, Fold, Opening, Operation, Phase, Settlement};

#[cfg(test)]
mod tests;

/// One operation opened by one exact delivery, not its shared business origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct OperationRef(Id);

impl OperationRef {
    /// Retained identity for decoding and inspection; the fold re-derives it.
    #[must_use]
    pub const fn from_id(id: Id) -> Self {
        Self(id)
    }

    /// Raw identity, never an attempt ordinal or a partner deduplication key.
    #[must_use]
    pub const fn id(self) -> Id {
        self.0
    }
}

/// One-based attempt correlation, shared by its Call and completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttemptRef {
    /// Original operation.
    pub operation: OperationRef,
    /// First attempt is 1; zero is reserved for a never-admitted stopped Close.
    pub ordinal: u64,
}

/// Preparation retained once, before any attempt reaches a Driver.
#[derive(Debug, Clone, PartialEq)]
pub struct Pin {
    /// Original request envelope retained by its opening admission.
    pub envelope: Id,
    /// Byte-identical operation across all attempts.
    pub prepared: PreparedRequest,
    /// Invocation settings reduced at operation opening, never re-evaluated.
    pub settings: BTreeMap<String, Value>,
    /// Optional external business-operation key; never suffixed by an attempt.
    pub key: Option<String>,
    /// Explicit original logical time, retained but not causally sealed.
    pub opened_at: i64,
}

/// Policy resolved from the original validated Contract/plan, not from a fact.
#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    call: CallSemantics,
    attempts: u64,
    cases: Vec<String>,
    delay: Option<LoopDelay>,
}

impl Policy {
    /// Checks the closed selection and repetition guarantee before construction.
    /// The source/package checker additionally proves constant expressions,
    /// exact host profile, timeout and output limits.
    ///
    /// # Errors
    /// Zero attempts, malformed delay, noncanonical/unknown/unsafe cases.
    pub fn try_new(
        call: CallSemantics,
        attempts: u64,
        cases: Vec<String>,
        delay: Option<LoopDelay>,
        idempotent: bool,
    ) -> Result<Self, Error> {
        if attempts == 0
            || call.validate_retry_cases(&cases, idempotent).is_err()
            || delay
                .as_ref()
                .is_some_and(|delay| delay.duration_after(1).is_none())
        {
            return Err(Error::Policy);
        }
        Ok(Self {
            call,
            attempts,
            cases,
            delay,
        })
    }

    /// Total bound including the first application of the operation.
    #[must_use]
    pub const fn attempts(&self) -> u64 {
        self.attempts
    }

    /// Canonically ordered terminal names that may select another attempt.
    #[must_use]
    pub fn cases(&self) -> &[String] {
        &self.cases
    }

    /// Whether this terminal has another permitted attempt, before Closing.
    #[must_use]
    pub fn repeats(&self, ordinal: u64, case: &str) -> bool {
        ordinal > 0 && ordinal < self.attempts && self.cases.iter().any(|name| name == case)
    }

    /// Original deadline from the winning closure, never a later manager time.
    #[must_use]
    pub fn due_after(&self, ordinal: u64, closed_at: i64) -> Option<i64> {
        self.delay
            .as_ref()
            .and_then(|delay| delay.duration_after(ordinal))
            .map(|delay| closed_at.saturating_add(delay))
    }
}

/// Retained disposition of one complete Connector emission batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    /// Exact operation and attempt.
    pub attempt: AttemptRef,
    /// The Call whose unique completion produced every emission below.
    pub call: CallRef,
    /// Entire ordered emission list, including stdout/stderr and terminal.
    pub emissions: Vec<Id>,
}

/// Closure has no new output wrapper and no authored command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// An ordinary final attempt was published atomically with this Close.
    Final,
    /// Exact-generation Closing cancelled further admissions, without output.
    Stopped,
}

/// Policy-specific facts carried by the existing journal species.
#[derive(Debug, Clone, PartialEq)]
pub enum Fact {
    /// Keep original preparation before an admission can reach the world.
    Opened {
        /// Derived from this instance, entry and exact opening delivery.
        operation: OperationRef,
        /// Canonical delivery identity, not only its EnvelopeId.
        opening: Id,
        /// Original request input.
        entry: Port,
        /// Original semantic plan binding.
        plan: SemanticHash,
        /// Exact versioned Contract.
        contract: ContractId,
        /// Original operation material, no idempotence copy.
        pin: Pin,
    },
    /// First or recurrent request became admissible, with ordinary Call budget.
    Admitted {
        /// One-based admitted attempt.
        attempt: AttemptRef,
        /// Complete admitted envelope with its fresh temporal system identity.
        envelope: Envelope,
        /// Exact initial/spatial or subsequent/temporal delivery identity.
        delivery: Id,
    },
    /// Producer retained the complete batch for the manager, not downstream.
    Held(Batch),
    /// One pending batch was acknowledged in the same transaction as Advance/Close.
    Consumed(Batch),
    /// The consumed predecessor selected the following admission.
    Advance {
        /// Attempt whose batch was consumed by this same transaction.
        from: AttemptRef,
        /// Following one-based ordinal.
        to: u64,
        /// Winning Call, also the original timer's causal authority.
        call: CallRef,
    },
    /// Final batch uses ordinary authored routes, without reminting envelopes.
    Published(Batch),
    /// Publication or exact-instance Closing ends the operation.
    Close {
        /// Last admitted ordinal, or zero only before any admission.
        attempt: AttemptRef,
        /// Final or stopped, never Loop exhaustion.
        reason: CloseReason,
    },
}

impl Fact {
    /// Exact operation affected by this fact.
    #[must_use]
    pub const fn operation(&self) -> OperationRef {
        match self {
            Self::Opened { operation, .. } => *operation,
            Self::Admitted { attempt, .. } | Self::Close { attempt, .. } => attempt.operation,
            Self::Held(batch) | Self::Consumed(batch) | Self::Published(batch) => {
                batch.attempt.operation
            }
            Self::Advance { from, .. } => from.operation,
        }
    }
}

/// Sparse contribution; context alone correlates an OUTBOUND Call fact.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Packet {
    /// Attempt of a Call or Connector completion, absent on manager records.
    pub context: Option<AttemptRef>,
    /// Facts in causal commit order, not sorted by kind.
    pub facts: Vec<Fact>,
}

impl Packet {
    /// Admissions are retained envelopes even though managers emit no port.
    pub fn envelopes(&self) -> impl Iterator<Item = &Envelope> {
        self.facts.iter().filter_map(|fact| match fact {
            Fact::Admitted { envelope, .. } => Some(envelope),
            _ => None,
        })
    }

    /// Canonical causal key 58 payload. Request bytes and original logical time
    /// belong to retention; the seal commits to the prepared digest instead.
    #[must_use]
    pub fn tree(&self) -> crate::canonical_cbor::Value {
        encoding::packet_tree(
            self,
            encoding::causal_pin,
            crate::identity::encoding::envelope_spine,
        )
    }

    /// Empty packets have no serialized presence.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.context.is_none() && self.facts.is_empty()
    }
}

/// C1's 0x16 domain: instance address, generation, entry, exact opening delivery.
const OPERATION_DOMAIN: u8 = 0x16;
/// A Retry admission is distinct from an ordinary emission and a Loop admission.
const ADMISSION_DOMAIN: u8 = 0x07;

/// Derive an operation from its exact opening admission.
#[must_use]
pub fn operation_ref(instance: &InstanceRef, entry: &Port, opening: Id) -> OperationRef {
    OperationRef(Id::derive(
        OPERATION_DOMAIN,
        &[
            instance.address().as_bytes(),
            &instance.generation().to_le_bytes(),
            entry.name().as_bytes(),
            opening.as_bytes(),
        ],
    ))
}

/// C1's 0x07 domain: operation, following ordinal, previous winning Call.
#[must_use]
pub fn admission_ref(operation: OperationRef, ordinal: u64, previous: CallRef) -> Id {
    Id::derive(
        ADMISSION_DOMAIN,
        &[
            operation.id().as_bytes(),
            &ordinal.to_le_bytes(),
            previous.id().as_bytes(),
        ],
    )
}

/// Temporal cursor namespace, distinct from physical edges and `@loop`.
#[must_use]
pub fn admission_edge(operation: OperationRef, entry: &Port) -> String {
    alloc::format!("@retry:{}:{}", operation.id(), entry.name())
}
