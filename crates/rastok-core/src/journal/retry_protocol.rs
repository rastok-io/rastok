//! Retained Retry operations: a selected Call outcome is not a Loop command.
//!
//! These are the C1 algebra's typed facts. Package/journal admission must also
//! establish the original plan, exact carrying transaction and spatial context;
//! a successful protocol fold alone is not permission to execute a Driver.

use crate::connector::CallSemantics;
use crate::connector::ContractId;
use crate::connector::exchange::CallId;
use crate::connector::exchange::PreparedRequest;
use crate::envelope::{Envelope, Id};
use crate::instance;

use crate::node::Port;
use crate::plan::{DelayPolicy, SemanticHash};
use crate::value::Value;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

mod carriers;
pub(crate) mod encoding;
pub(crate) use carriers::validate_transaction;
mod retry_view;
pub use retry_view::{Error, Opening, Operation, Phase, RetryView, Settlement};

#[cfg(test)]
mod tests;

/// One operation opened by one exact delivery, not its shared business origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct OperationId(Id);

impl OperationId {
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
pub struct AttemptId {
    /// Original operation.
    pub operation: OperationId,
    /// First attempt is 1. A `Closed` fact with reason `Stopped` uses zero
    /// only when no attempt was admitted.
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
    delay: Option<DelayPolicy>,
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
        delay: Option<DelayPolicy>,
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
    pub attempt: AttemptId,
    /// The Call whose unique completion produced every emission below.
    pub call: CallId,
    /// Entire ordered emission list, including stdout/stderr and terminal.
    pub emissions: Vec<Id>,
}

/// Closure has no new output wrapper and no authored command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// An ordinary final attempt was published atomically with this Closed fact.
    Final,
    /// Exact-generation Closing cancelled further admissions, without output.
    Stopped,
}

/// Policy-specific facts carried by the existing journal species.
#[derive(Debug, Clone, PartialEq)]
pub enum Fact {
    /// Original preparation was retained before any attempt could reach a Driver.
    Opened {
        /// Derived from this instance, entry and exact opening delivery.
        operation: OperationId,
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
        attempt: AttemptId,
        /// Complete admitted envelope with its fresh temporal system identity.
        envelope: Envelope,
        /// Exact initial/spatial or subsequent/temporal delivery identity.
        delivery: Id,
    },
    /// Producer retained the complete batch for the manager, not downstream.
    Held(Batch),
    /// One pending batch was acknowledged in the same transaction as Advanced/Closed.
    Consumed(Batch),
    /// The consumed predecessor selected the following admission.
    Advanced {
        /// Attempt whose batch was consumed by this same transaction.
        from: AttemptId,
        /// Following one-based ordinal.
        to: u64,
        /// Winning Call, also the original deadline's causal authority.
        call: CallId,
    },
    /// Final batch uses ordinary authored routes, without changing envelope identities.
    Published(Batch),
    /// Publication or exact-instance Closing ended the operation.
    Closed {
        /// Last admitted ordinal, or zero only before any admission.
        attempt: AttemptId,
        /// Final or stopped, never Loop exhaustion.
        reason: CloseReason,
    },
}

impl Fact {
    /// Exact operation affected by this fact.
    #[must_use]
    pub const fn operation(&self) -> OperationId {
        match self {
            Self::Opened { operation, .. } => *operation,
            Self::Admitted { attempt, .. } | Self::Closed { attempt, .. } => attempt.operation,
            Self::Held(batch) | Self::Consumed(batch) | Self::Published(batch) => {
                batch.attempt.operation
            }
            Self::Advanced { from, .. } => from.operation,
        }
    }
}

/// The Retry protocol's part of a TRANSACTION or EGRESS Call record, never a
/// standalone journal record. Context alone correlates an EGRESS Call fact;
/// transaction records also carry operation facts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RetryRecord {
    /// Attempt of a Call or Connector completion, absent on manager records.
    pub context: Option<AttemptId>,
    /// Facts in causal commit order, not sorted by kind.
    pub facts: Vec<Fact>,
}

impl RetryRecord {
    /// Admissions are retained envelopes even though managers emit no port.
    pub fn envelopes(&self) -> impl Iterator<Item = &Envelope> {
        self.facts.iter().filter_map(|fact| match fact {
            Fact::Admitted { envelope, .. } => Some(envelope),
            _ => None,
        })
    }

    /// Canonical causal key 43 payload. Request bytes and original logical time
    /// belong to retention; the seal commits to the prepared digest instead.
    #[must_use]
    pub fn tree(&self) -> crate::canonical_cbor::Value {
        encoding::record_tree(self, encoding::causal_pin, crate::canonical::envelope)
    }

    /// Empty Retry records have no serialized presence.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.context.is_none() && self.facts.is_empty()
    }
}

/// Derive an operation from its exact opening admission.
#[must_use]
pub fn operation_ref(instance: &instance::Generation, entry: &Port, opening: Id) -> OperationId {
    OperationId(Id::derive(
        crate::envelope::IdKind::RetryOperation,
        &[
            instance.address().as_bytes(),
            &instance.generation().to_le_bytes(),
            entry.name().as_bytes(),
            opening.as_bytes(),
        ],
    ))
}

/// RetryAdmission's 0x04 domain: operation, following ordinal, previous winning Call.
#[must_use]
pub fn admission_ref(operation: OperationId, ordinal: u64, previous: CallId) -> Id {
    Id::derive(
        crate::envelope::IdKind::RetryAdmission,
        &[
            operation.id().as_bytes(),
            &ordinal.to_le_bytes(),
            previous.id().as_bytes(),
        ],
    )
}

/// Temporal cursor namespace, distinct from physical edges and `@loop`.
#[must_use]
pub fn admission_edge(operation: OperationId, entry: &Port) -> String {
    alloc::format!("@retry:{}:{}", operation.id(), entry.name())
}
