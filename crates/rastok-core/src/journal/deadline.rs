//! Dated obligations created and accepted with one record, outside the causal projection.
//!
//! A budget is an allowed duration; a deadline is the logical instant at which
//! an obligation becomes due; a timeout is the outcome when an operation has
//! not completed within its budget. Not every deadline is a timeout: a Loop
//! or Retry delay makes a new admission eligible instead.
//!
//! Creation order belongs to recovery: sorting equal-due deadlines by reference would
//! silently change which timeout wins after a restart.

use super::DeadlineKind;
use crate::envelope::Id;
use alloc::vec::Vec;
use core::fmt;

/// A deadline violates an acceptance invariant, independently of its encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// No obligations were supplied; the optional field must instead be absent.
    EmptySchedule,
    /// An obligation is already due before its recorded creation.
    DeadlinePrecedesCreation,
    /// A winning Call closure has no logical instant.
    MissingClosureInstant,
    /// An open Call incorrectly carries a closure instant.
    UnexpectedClosureInstant,
    /// A Call closes earlier than its original opening.
    ClosurePrecedesOpening,
    /// A Retry Call packet is missing its attempt context.
    MissingRetryContext,
    /// A Call carries facts or a nonpositive attempt ordinal.
    InvalidRetryContext,
    /// The next attempt ordinal cannot be represented.
    RetryOrdinalOverflow,
    /// A recurrence deadline is attached to a Call without Retry.
    DeadlineWithoutRetryContext,
    /// A Retry deadline does not identify this closure's next admission.
    DeadlineMismatch,
    /// A sealed obligation has no corresponding unsealed deadline.
    SealedDeadlineWithoutInstant,
    /// The observable transaction time disagrees with deadline creation.
    FiringTimeDisagrees,
    /// Loop Advance facts and recurrence deadlines do not agree exactly.
    LoopDeadlineMismatch,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::EmptySchedule => "empty deadlines must be omitted",
            Self::DeadlinePrecedesCreation => "a deadline precedes its creation",
            Self::MissingClosureInstant => "a winning Call closure requires its logical instant",
            Self::UnexpectedClosureInstant => "an open Call cannot carry a closure instant",
            Self::ClosurePrecedesOpening => "a Call closure precedes its opening",
            Self::MissingRetryContext => "a Retry Call requires an attempt context",
            Self::InvalidRetryContext => "a Call carries only a positive Retry context",
            Self::RetryOrdinalOverflow => "Retry admission ordinal overflows",
            Self::DeadlineWithoutRetryContext => {
                "only a Retry closure can create a recurrence deadline"
            }
            Self::DeadlineMismatch => {
                "the Retry deadline must identify this closure's next admission"
            }
            Self::SealedDeadlineWithoutInstant => "a sealed deadline id has no unsealed deadline",
            Self::FiringTimeDisagrees => "observable firing time disagrees with deadline creation",
            Self::LoopDeadlineMismatch => "Loop Advance and its exact recurrence deadline disagree",
        })
    }
}

impl core::error::Error for Error {}

/// One dated obligation, identified by its kind and exact subject id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deadline {
    /// The journal's typed kind of dated obligation.
    pub kind: DeadlineKind,
    /// Exact obligation, never a freshly derived restart reference.
    pub reference: Id,
    /// Absolute signed nanosecond instant on the run's logical timeline.
    pub due_at: i64,
}

/// An immutable schedule accepted with one journal record. The journal owns
/// it; the scheduler is only a disposable execution projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deadlines {
    /// Creation instant in signed logical nanoseconds, not wall-clock time.
    created_at: i64,
    items: Vec<Deadline>,
}

impl Deadlines {
    /// Validate before acceptance. An absent schedule is `None`, not an empty
    /// record field. Repeated references are ordered renewals, not a set.
    ///
    /// # Errors
    /// Empty schedules or deadlines preceding their creation instant.
    pub fn try_new(created_at: i64, items: Vec<Deadline>) -> Result<Self, Error> {
        if items.is_empty() {
            return Err(Error::EmptySchedule);
        }
        for deadline in &items {
            if deadline.due_at < created_at {
                return Err(Error::DeadlinePrecedesCreation);
            }
        }
        Ok(Self { created_at, items })
    }

    /// The explicit logical instant at acceptance, never OS or restart time.
    #[must_use]
    pub const fn created_at(&self) -> i64 {
        self.created_at
    }

    /// Creation order. Together with record order it breaks equal-date ties.
    #[must_use]
    pub fn items(&self) -> &[Deadline] {
        &self.items
    }
}
