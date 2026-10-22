//! Resumable control of the existing discrete-time sweep.

use super::{ClosingDeadline, Execution, ExecutionFault, Journal, Obligation, OutboundDrivers};

/// An in-progress advance on one Execution. This is a caller-owned control cursor,
/// not a journal fact or a second timer queue. Do not interleave other Execution
/// commands until it completes. Batch and interactive execution use this same
/// sweep, including the settlement poll and same-instant expiry ordering.
#[derive(Debug)]
pub struct TimeAdvance {
    target: i64,
    phase: Phase,
    instant: Option<i64>,
    polled_at_settlement: bool,
    saw_drain: bool,
}

#[derive(Debug)]
enum Phase {
    Ready,
    Due,
    Settlement {
        key: (i64, u64),
        payload: Obligation,
    },
    Expiry {
        key: (i64, u64),
        payload: Obligation,
    },
    Done,
}

impl TimeAdvance {
    /// Starts a sweep toward an absolute logical nanosecond instant.
    #[must_use]
    pub const fn new(target: i64) -> Self {
        Self {
            target,
            phase: Phase::Ready,
            instant: None,
            polled_at_settlement: false,
            saw_drain: false,
        }
    }

    /// Whether the Execution reached the target with all intervening work drained.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        matches!(self.phase, Phase::Done)
    }

    /// Executes one ready transaction, one expiry, or one boundary poll.
    /// Returns false only when no advance work remains. The caller may inspect
    /// the actual Execution and journal after each successful step.
    ///
    /// # Errors
    /// The same transaction, expiry or binding refusals as [`Execution::advance_to`].
    /// A refused transaction/expiry retains its cursor for retry.
    pub fn advance_once(
        &mut self,
        execution: &mut Execution<'_>,
        journal: &mut dyn Journal,
        drivers: &mut dyn OutboundDrivers,
    ) -> Result<bool, ExecutionFault> {
        loop {
            match &self.phase {
                Phase::Ready => {
                    // A recovered Execution can already be at an unsealed deadline.
                    // Resolve that dated frontier before admitting ready
                    // recurrences, just as the uninterrupted sweep does.
                    // Lifecycle work queued by an expiry waits for the rest
                    // of that frontier. `peek_due` fences only the lifetime
                    // of an instance whose own start has not committed yet.
                    if self.instant.is_none()
                        && execution.peek_due(execution.now.min(self.target)).is_some()
                    {
                        self.phase = Phase::Due;
                        continue;
                    }
                    if execution.step_with_drivers(journal, drivers)? {
                        return Ok(true);
                    }
                    self.phase = Phase::Due;
                }
                Phase::Due => {
                    if let Some((key, payload)) =
                        execution.peek_due(self.instant.unwrap_or(self.target))
                    {
                        if self.instant.is_none() {
                            self.instant = Some(key.0);
                            self.polled_at_settlement = false;
                            self.saw_drain = false;
                        }
                        execution.now = execution.now.max(key.0);
                        if !self.polled_at_settlement
                            && matches!(
                                payload,
                                Obligation::InstanceClosing {
                                    deadline: ClosingDeadline::Settlement,
                                    ..
                                }
                            )
                        {
                            let report = execution.poll_outbound_driver(journal, drivers)?;
                            let progressed = report.settled > 0;
                            execution.buffered_poll.append(report);
                            self.polled_at_settlement = true;
                            self.phase = Phase::Settlement { key, payload };
                            if progressed {
                                return Ok(true);
                            }
                        } else {
                            self.phase = Phase::Expiry { key, payload };
                        }
                    } else if self.instant.take().is_some() {
                        self.phase = Phase::Ready;
                        if self.saw_drain {
                            let report = execution.poll_outbound_driver(journal, drivers)?;
                            let progressed = report.settled > 0;
                            execution.buffered_poll.append(report);
                            if progressed {
                                return Ok(true);
                            }
                        }
                    } else {
                        let changed = execution.now < self.target;
                        execution.now = execution.now.max(self.target);
                        self.phase = Phase::Done;
                        return Ok(changed);
                    }
                }
                Phase::Settlement { key, payload } => {
                    if execution.step_with_drivers(journal, drivers)? {
                        return Ok(true);
                    }
                    self.phase = Phase::Expiry {
                        key: *key,
                        payload: payload.clone(),
                    };
                }
                Phase::Expiry { key, payload } => {
                    self.saw_drain |= matches!(
                        payload,
                        Obligation::InstanceClosing {
                            deadline: ClosingDeadline::Drain,
                            ..
                        }
                    );
                    execution.expire(journal, drivers, payload.clone())?;
                    execution.scheduler.ack(*key);
                    self.phase = Phase::Due;
                    return Ok(true);
                }
                Phase::Done => return Ok(false),
            }
        }
    }
}
