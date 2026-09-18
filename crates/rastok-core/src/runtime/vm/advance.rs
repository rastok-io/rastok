//! Resumable control of the existing discrete-time sweep.

use super::{ClosingDeadline, DatedPayload, Journal, OutboundConnectorBindings, Vm, VmError};

/// An in-progress advance on one VM. This is a caller-owned control cursor,
/// not a journal fact or a second timer queue. Do not interleave other VM
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
        payload: DatedPayload,
    },
    Expiry {
        key: (i64, u64),
        payload: DatedPayload,
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

    /// Whether the VM reached the target with all intervening work drained.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        matches!(self.phase, Phase::Done)
    }

    /// Executes one ready transaction, one expiry, or one boundary poll.
    /// Returns false only when no advance work remains. The caller may inspect
    /// the actual VM and journal after each successful step.
    ///
    /// # Errors
    /// The same transaction, expiry or binding refusals as [`Vm::advance`].
    /// A refused transaction/expiry retains its cursor for retry.
    pub fn step(
        &mut self,
        vm: &mut Vm<'_>,
        journal: &mut dyn Journal,
        bindings: &mut dyn OutboundConnectorBindings,
    ) -> Result<bool, VmError> {
        loop {
            match &self.phase {
                Phase::Ready => {
                    // A recovered VM can already be at a retained deadline.
                    // Resolve that dated frontier before admitting ready
                    // recurrences, just as the uninterrupted sweep does.
                    // Lifecycle work queued by an expiry waits for the rest
                    // of that frontier. `peek_due` fences only the lifetime
                    // of an instance whose own start has not committed yet.
                    if self.instant.is_none() && vm.peek_due(vm.now.min(self.target)).is_some() {
                        self.phase = Phase::Due;
                        continue;
                    }
                    if vm.step_with_bindings(journal, bindings)? {
                        return Ok(true);
                    }
                    self.phase = Phase::Due;
                }
                Phase::Due => {
                    if let Some((key, payload)) = vm.peek_due(self.instant.unwrap_or(self.target)) {
                        if self.instant.is_none() {
                            self.instant = Some(key.0);
                            self.polled_at_settlement = false;
                            self.saw_drain = false;
                        }
                        vm.now = vm.now.max(key.0);
                        if !self.polled_at_settlement
                            && matches!(
                                payload,
                                DatedPayload::InstanceClosing {
                                    deadline: ClosingDeadline::Settlement,
                                    ..
                                }
                            )
                        {
                            let report = vm.poll_outbound_driver(journal, bindings)?;
                            let progressed = report.settled > 0;
                            vm.buffered_pump.append(report);
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
                            let report = vm.poll_outbound_driver(journal, bindings)?;
                            let progressed = report.settled > 0;
                            vm.buffered_pump.append(report);
                            if progressed {
                                return Ok(true);
                            }
                        }
                    } else {
                        let changed = vm.now < self.target;
                        vm.now = vm.now.max(self.target);
                        self.phase = Phase::Done;
                        return Ok(changed);
                    }
                }
                Phase::Settlement { key, payload } => {
                    if vm.step_with_bindings(journal, bindings)? {
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
                        DatedPayload::InstanceClosing {
                            deadline: ClosingDeadline::Drain,
                            ..
                        }
                    );
                    vm.expire(journal, bindings, payload.clone())?;
                    vm.scheduler.ack(*key);
                    self.phase = Phase::Due;
                    return Ok(true);
                }
                Phase::Done => return Ok(false),
            }
        }
    }
}
