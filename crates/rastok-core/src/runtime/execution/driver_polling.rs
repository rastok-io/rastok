//! Offers committed Call openings to Drivers, checks their responses and
//! commits the winning closures before returning them to Execution.

use crate::connector::driver::*;
use crate::connector::exchange::*;
use crate::journal::{CloseVerdict, Closure, Journal};
use crate::node;
use alloc::{
    boxed::Box,
    collections::{BTreeSet, VecDeque},
    string::String,
    vec::Vec,
};
use core::fmt;

/// Validates a Driver completion before the poller commits its Call closure.
/// The check returns an acceptance or a reason for rejection; it neither
/// changes the journal nor constructs output envelopes.
pub trait CompletionCheck {
    /// `Ok` to admit; `Err(reason)` to refuse the closure.
    ///
    /// # Errors
    ///
    /// The refusal reason, surfaced with the rejected response.
    fn check(&self, connector: &node::Path, completion: &Completion) -> Result<(), String>;
}

/// Performs no additional completion validation. Used when the caller has
/// validated the completion elsewhere, and in tests of Call polling.
#[derive(Debug, Clone, Copy, Default)]
pub struct AcceptAnyCompletion;

impl CompletionCheck for AcceptAnyCompletion {
    fn check(&self, _: &node::Path, _: &Completion) -> Result<(), String> {
        Ok(())
    }
}

/// Result of one Call polling round. Accepted closures are ready for the
/// Execution's Connector transactions; other responses remain observations for
/// the host, not graph deliveries.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PollBatch {
    /// The won closures, routable to the Execution.
    pub events: Vec<ClosedCall>,
    /// Answers to Calls whose closure was already committed.
    pub discarded: Vec<CallResponse>,
    /// Answers correlating with no journaled call — correlation
    /// faults to surface.
    pub uncorrelated: Vec<CallResponse>,
    /// Answers refused by the completion check, with the reason. The Call
    /// stays open; a valid response or its timeout may still close it.
    pub rejected: Vec<(CallResponse, String)>,
}

/// Failure to poll or settle a Call because its request, Driver binding or
/// journal operation is invalid or unavailable.
#[derive(Debug, Clone, PartialEq)]
pub enum PollFault {
    /// A journaled intent whose request envelope cannot be found:
    /// an integrity fault in the causal history.
    MissingRequest(CallId),
    /// The verified Driver routing table was contradicted.
    Driver(DriverRouteError),
    /// The journal refused an operation.
    Journal(crate::journal::Error),
}

impl fmt::Display for PollFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRequest(call_id) => {
                write!(f, "no journaled envelope for the request of call {call_id}")
            }
            Self::Driver(error) => error.fmt(f),
            Self::Journal(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for PollFault {}

/// Connects journaled Call openings to outbound Drivers and their observations.
///
/// A fresh round has two halves:
///
/// 1. Offer newly journaled Calls to their Drivers. Journal-before-effect:
///    the opening record exists before a Driver is asked to execute it.
/// 2. Collect completions, check them against the Contract, and settle them
///    through the journal's close-once rule into [`ClosedCall`]s.
///
/// A refused settlement keeps its batch for a later round; another batch is
/// not polled until that one is complete. The Execution builds the output envelopes
/// and commits the Connector transaction. The poller never creates an envelope.
/// A mock replaces the Driver, never this construction and commit.
///
/// Restart discipline: an old Call is never executed again by recovery.
/// A later Retry admission is a new, independently authorized Call.
/// A poller over an existing journal is therefore built with
/// [`DriverPoller::resume`], which treats every already-open call as
/// started and re-calls nothing.
#[derive(Debug, Default)]
pub struct DriverPoller {
    started: BTreeSet<CallId>,
    pending_settlement: Option<Box<PendingSettlement>>,
}

/// One handed-off Driver batch, including closures already committed before
/// a later refusal. Do not poll another batch until this one reaches the Execution.
#[derive(Debug)]
struct PendingSettlement {
    responses: VecDeque<CallResponse>,
    batch: PollBatch,
}

impl DriverPoller {
    /// A poller for a fresh run: nothing started yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A poller over an existing journal. Every currently open Call counts as
    /// started; recovery resolves its recorded state without executing the
    /// Driver operation again. A later Retry attempt opens a new Call.
    #[must_use]
    pub fn resume(journal: &dyn Journal) -> Self {
        Self {
            started: journal
                .open_calls()
                .into_iter()
                .map(|intent| intent.call_id)
                .collect(),
            pending_settlement: None,
        }
    }

    /// Drives one round: starts newly journaled open calls, then
    /// settles the driver's answers through the close-once race.
    ///
    /// # Errors
    ///
    /// [`PollFault::MissingRequest`] when the causal history lacks
    /// a journaled request — an integrity fault; journal refusals are
    /// propagated, never absorbed. A refused settlement retains that Driver
    /// batch, including earlier winning closures, until a later poll succeeds.
    pub fn poll_drivers<B: OutboundDrivers + ?Sized>(
        &mut self,
        journal: &mut dyn Journal,
        drivers: &mut B,
        gate: &dyn CompletionCheck,
        now: crate::time::LogicalTime,
    ) -> Result<PollBatch, PollFault> {
        self.poll_drivers_with_timing(journal, drivers, gate, now, &mut |_, _| Ok(None))
    }

    pub(crate) fn poll_drivers_with_timing<B: OutboundDrivers + ?Sized>(
        &mut self,
        journal: &mut dyn Journal,
        drivers: &mut B,
        gate: &dyn CompletionCheck,
        now: crate::time::LogicalTime,
        timing: &mut impl FnMut(
            &CallOpen,
            &Completion,
        ) -> Result<
            Option<alloc::boxed::Box<crate::journal::deadline::Deadlines>>,
            PollFault,
        >,
    ) -> Result<PollBatch, PollFault> {
        // A timeout or abandonment can close a call outside this response
        // loop. Keep the local start cache equal to the journal's open fold,
        // rather than retaining every historical call for the run's life.
        let open = journal.open_calls();
        let open_refs: BTreeSet<_> = open.iter().map(|intent| intent.call_id).collect();
        self.started.retain(|call_id| open_refs.contains(call_id));
        // A refused batch keeps ownership of its answers. Starting more work
        // or polling the Driver here would grow retention during store pressure.
        if self.pending_settlement.is_none() {
            for intent in &open {
                if self.started.contains(&intent.call_id) {
                    continue;
                }
                let request = journal
                    .find_envelope(intent.emitting_id)
                    .ok_or(PollFault::MissingRequest(intent.call_id))?;
                let start = drivers.start(intent, &request).map_err(PollFault::Driver)?;
                if start == CallStart::Started {
                    self.started.insert(intent.call_id);
                }
            }
        }
        // Settle phase: won closures become routable events; losers
        // and strays stay visible.
        let mut settlement = match self.pending_settlement.take() {
            Some(pending) => *pending,
            None => PendingSettlement {
                responses: drivers.poll_responses().map_err(PollFault::Driver)?.into(),
                batch: PollBatch::default(),
            },
        };
        while let Some(response) = settlement.responses.pop_front() {
            let Some(intent) = open
                .iter()
                .find(|intent| intent.call_id == response.call_id)
            else {
                if journal.find_call(&response.call_id).is_some() {
                    settlement.batch.discarded.push(response);
                } else {
                    settlement.batch.uncorrelated.push(response);
                }
                continue;
            };
            // Validation BEFORE the causal fact: a reply the contract
            // refuses must never close the call — journaled garbage
            // would leave an unclosable obligation.
            if let Err(reason) = gate.check(&intent.connector_path, &response.completion) {
                settlement.batch.rejected.push((response, reason));
                continue;
            }
            let closure = Closure::Response(response.completion.clone());
            let verdict = timing(intent, &response.completion).and_then(|schedule| {
                journal
                    .close_egress(&response.call_id, closure, now, schedule)
                    .map_err(PollFault::Journal)
            });
            let verdict = match verdict {
                Ok(verdict) => verdict,
                Err(error) => {
                    settlement.responses.push_front(response);
                    self.pending_settlement = Some(Box::new(settlement));
                    return Err(error);
                }
            };
            match verdict {
                CloseVerdict::Won => {
                    self.started.remove(&response.call_id);
                    settlement.batch.events.push(ClosedCall {
                        call_id: response.call_id,
                        connector_path: intent.connector_path.clone(),
                        emitting_id: intent.emitting_id,
                        completion: response.completion,
                    });
                }
                CloseVerdict::AlreadyClosed => settlement.batch.discarded.push(response),
            }
        }
        Ok(settlement.batch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connector::stdlib::*;
    use crate::envelope::{self, Data, Encoding, Envelope, Namespace};
    use crate::journal::{Journal, MemJournal, ids};
    use crate::node;
    use crate::plan::GraphVersion;
    use alloc::{
        string::{String, ToString},
        vec,
        vec::Vec,
    };

    /// A scripted driver: records what it was asked to start,
    /// answers from a script.
    #[derive(Default)]
    struct Scripted {
        started: Vec<(CallOpen, Envelope)>,
        replies: Vec<CallResponse>,
    }

    impl OutboundDrivers for Scripted {
        fn prepare_request(
            &mut self,
            _: &node::Path,
            data: &Data,
            _: &alloc::collections::BTreeMap<String, crate::value::Value>,
        ) -> Result<RequestPreparation, DriverRouteError> {
            Ok(match prepare_lines(data) {
                Ok(prepared) => RequestPreparation::Prepared(prepared),
                Err(message) => RequestPreparation::Rejected(Completion::process_exit(
                    ProcessExit::DecodeFailed {
                        message,
                        line: None,
                    },
                )),
            })
        }

        fn scheduler_completion(
            &mut self,
            _: &node::Path,
            completion: SchedulerCompletion,
        ) -> Result<Completion, DriverRouteError> {
            Ok(Completion::process_exit(match completion {
                SchedulerCompletion::TimeoutExpired => ProcessExit::TimeoutExpired,
                SchedulerCompletion::OutcomeUnknown => ProcessExit::OutcomeUnknown,
            }))
        }

        fn cancel(&mut self, _: &node::Path, _: &CallId) -> Result<(), DriverRouteError> {
            Ok(())
        }

        fn start(
            &mut self,
            intent: &CallOpen,
            request: &Envelope,
        ) -> Result<CallStart, DriverRouteError> {
            self.started.push((intent.clone(), request.clone()));
            Ok(CallStart::Started)
        }

        fn poll_responses(&mut self) -> Result<Vec<CallResponse>, DriverRouteError> {
            Ok(core::mem::take(&mut self.replies))
        }
    }

    struct StartRefusal {
        refuse: bool,
        pending: bool,
        attempts: usize,
    }

    impl OutboundDrivers for StartRefusal {
        fn prepare_request(
            &mut self,
            _: &node::Path,
            data: &Data,
            _: &alloc::collections::BTreeMap<String, crate::value::Value>,
        ) -> Result<RequestPreparation, DriverRouteError> {
            Ok(prepare_process_request(RequestCodec::Lines, data))
        }

        fn scheduler_completion(
            &mut self,
            _: &node::Path,
            completion: SchedulerCompletion,
        ) -> Result<Completion, DriverRouteError> {
            Ok(process_scheduler_completion(completion))
        }

        fn cancel(&mut self, _: &node::Path, _: &CallId) -> Result<(), DriverRouteError> {
            Ok(())
        }

        fn start(
            &mut self,
            intent: &CallOpen,
            _: &Envelope,
        ) -> Result<CallStart, DriverRouteError> {
            self.attempts += 1;
            if self.refuse {
                return Err(DriverRouteError::UnboundOutbound(
                    intent.connector_path.clone(),
                ));
            }
            if self.pending {
                return Ok(CallStart::Pending);
            }
            Ok(CallStart::Started)
        }

        fn poll_responses(&mut self) -> Result<Vec<CallResponse>, DriverRouteError> {
            Ok(Vec::new())
        }
    }

    fn seeded_journal() -> (MemJournal, CallOpen) {
        let mut journal = MemJournal::new(GraphVersion(1));
        let committed = journal
            .append_ingress(IngressProposal {
                provenance: IngressProvenance::single(node::Path::new(["inbound"]).unwrap()),
                draft: EnvelopeDraft {
                    port: node::Port::new("in").unwrap(),
                    data: Data::encoded(Encoding::Raw, *b"salut"),
                    ext: Namespace::new(),
                },
                sequence: IngressSequence::Next,
                session: None,
            })
            .unwrap();
        let connector_path = node::Path::new(["echo"]).unwrap();
        let emitting_id = committed.envelope.meta().sys().id();
        let edge = String::from("shout.out>echo.in");
        let prepared = PreparedRequest::seal(b"salut\n".to_vec());
        let intent = CallOpen {
            call_id: ids::call_id(ids::delivery_id(&edge, emitting_id), prepared.digest()),
            emitting_id,
            edge,
            connector_path,
            prepared,
            idempotency_key: None,
            resolved_settings: alloc::collections::BTreeMap::new(),
            logical_time: crate::time::LogicalTime::ZERO,
        };
        journal.open_egress(intent.clone(), None).unwrap();
        (journal, intent)
    }

    #[test]
    fn the_record_exists_before_the_driver_hears() {
        let (mut journal, intent) = seeded_journal();
        let mut driver = Scripted::default();
        let mut poller = DriverPoller::new();
        poller
            .poll_drivers(
                &mut journal,
                &mut driver,
                &AcceptAnyCompletion,
                crate::time::LogicalTime::ZERO,
            )
            .unwrap();
        assert_eq!(driver.started.len(), 1);
        assert_eq!(driver.started[0].0.call_id, intent.call_id);
        // A second poll never re-starts an already-started Call.
        poller
            .poll_drivers(
                &mut journal,
                &mut driver,
                &AcceptAnyCompletion,
                crate::time::LogicalTime::ZERO,
            )
            .unwrap();
        assert_eq!(driver.started.len(), 1);
    }

    #[test]
    fn a_refused_start_is_not_remembered_as_started() {
        let (mut journal, _) = seeded_journal();
        let mut drivers = StartRefusal {
            refuse: true,
            pending: false,
            attempts: 0,
        };
        let mut poller = DriverPoller::new();

        assert!(matches!(
            poller.poll_drivers(
                &mut journal,
                &mut drivers,
                &AcceptAnyCompletion,
                crate::time::LogicalTime::ZERO
            ),
            Err(PollFault::Driver(DriverRouteError::UnboundOutbound(_)))
        ));
        assert_eq!(drivers.attempts, 1);
        drivers.refuse = false;
        poller
            .poll_drivers(
                &mut journal,
                &mut drivers,
                &AcceptAnyCompletion,
                crate::time::LogicalTime::ZERO,
            )
            .unwrap();
        assert_eq!(drivers.attempts, 2, "the open call was retried");
    }

    #[test]
    fn a_backpressured_start_remains_open_and_is_offered_again() {
        let (mut journal, _) = seeded_journal();
        let mut drivers = StartRefusal {
            refuse: false,
            pending: true,
            attempts: 0,
        };
        let mut poller = DriverPoller::new();

        poller
            .poll_drivers(
                &mut journal,
                &mut drivers,
                &AcceptAnyCompletion,
                crate::time::LogicalTime::ZERO,
            )
            .unwrap();
        assert_eq!(drivers.attempts, 1);
        assert_eq!(journal.open_calls().len(), 1);

        drivers.pending = false;
        poller
            .poll_drivers(
                &mut journal,
                &mut drivers,
                &AcceptAnyCompletion,
                crate::time::LogicalTime::ZERO,
            )
            .unwrap();
        assert_eq!(drivers.attempts, 2, "capacity recovery retries the call");
        poller
            .poll_drivers(
                &mut journal,
                &mut drivers,
                &AcceptAnyCompletion,
                crate::time::LogicalTime::ZERO,
            )
            .unwrap();
        assert_eq!(drivers.attempts, 2, "a started call is never duplicated");
    }

    #[test]
    fn a_resumed_poller_re_calls_nothing() {
        let (mut journal, _) = seeded_journal();
        let mut driver = Scripted::default();
        // Restart over an existing journal: the open call counts as
        // started; re-calls are the recovery fold's decision.
        let mut poller = DriverPoller::resume(&journal);
        poller
            .poll_drivers(
                &mut journal,
                &mut driver,
                &AcceptAnyCompletion,
                crate::time::LogicalTime::ZERO,
            )
            .unwrap();
        assert!(driver.started.is_empty());
    }

    #[test]
    fn scheduler_closures_are_pruned_from_the_start_cache() {
        let (mut journal, intent) = seeded_journal();
        let mut driver = Scripted::default();
        let mut poller = DriverPoller::new();
        poller
            .poll_drivers(
                &mut journal,
                &mut driver,
                &AcceptAnyCompletion,
                crate::time::LogicalTime::ZERO,
            )
            .unwrap();
        assert!(poller.started.contains(&intent.call_id));
        journal
            .close_egress(
                &intent.call_id,
                Closure::Timeout,
                crate::time::LogicalTime::ZERO,
                None,
            )
            .unwrap();

        poller
            .poll_drivers(
                &mut journal,
                &mut driver,
                &AcceptAnyCompletion,
                crate::time::LogicalTime::ZERO,
            )
            .unwrap();
        assert!(poller.started.is_empty());
    }

    #[test]
    fn won_closures_become_routable_events() {
        let (mut journal, intent) = seeded_journal();
        let mut driver = Scripted::default();
        let mut poller = DriverPoller::new();
        poller
            .poll_drivers(
                &mut journal,
                &mut driver,
                &AcceptAnyCompletion,
                crate::time::LogicalTime::ZERO,
            )
            .unwrap();
        driver.replies = vec![
            CallResponse {
                call_id: intent.call_id,
                completion: Completion::process(
                    vec![Data::encoded(Encoding::Raw, *b"SALUT")],
                    vec![],
                    ProcessExit::Exited {
                        exit_code: 0,
                        signal: None,
                    },
                ),
            },
            // A duplicate answer for the same call: the race loser.
            CallResponse {
                call_id: intent.call_id,
                completion: Completion::process(
                    vec![Data::encoded(Encoding::Raw, *b"LATE")],
                    vec![],
                    ProcessExit::Exited {
                        exit_code: 0,
                        signal: None,
                    },
                ),
            },
        ];
        let batch = poller
            .poll_drivers(
                &mut journal,
                &mut driver,
                &AcceptAnyCompletion,
                crate::time::LogicalTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            batch.events.len(),
            1,
            "only the won closure feeds the Execution"
        );
        assert_eq!(batch.discarded.len(), 1, "the loser stays visible");
        assert!(batch.uncorrelated.is_empty());
        let event = &batch.events[0];
        assert_eq!(event.connector_path.to_string(), "echo");
        assert_eq!(event.emitting_id, intent.emitting_id);
        assert_eq!(event.completion.outputs()[0].port.name(), "out");
        assert_eq!(
            event
                .completion
                .outputs()
                .last()
                .map(|output| output.port.name()),
            Some("exit")
        );
        // The poller constructed no envelope: the journal's closure
        // fact carries the raw completion, and the connector's own
        // transaction will build the envelopes.
        assert!(journal.open_calls().is_empty());
    }

    #[test]
    fn a_completion_keeps_stderr_separate_from_its_exit() {
        let (mut journal, intent) = seeded_journal();
        let mut driver = Scripted::default();
        let mut poller = DriverPoller::new();
        poller
            .poll_drivers(
                &mut journal,
                &mut driver,
                &AcceptAnyCompletion,
                crate::time::LogicalTime::ZERO,
            )
            .unwrap();
        driver.replies = vec![CallResponse {
            call_id: intent.call_id,
            completion: Completion::process(
                vec![],
                vec![Data::encoded(Encoding::Raw, *b"diagnostic")],
                ProcessExit::Exited {
                    exit_code: 1,
                    signal: None,
                },
            ),
        }];
        let batch = poller
            .poll_drivers(
                &mut journal,
                &mut driver,
                &AcceptAnyCompletion,
                crate::time::LogicalTime::ZERO,
            )
            .unwrap();
        assert_eq!(batch.events.len(), 1);
        let outputs = batch.events[0].completion.outputs();
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].port.name(), "err");
        assert_eq!(outputs[1].port.name(), "exit");
    }

    #[test]
    fn an_uncorrelated_answer_is_a_visible_fault() {
        let (mut journal, _) = seeded_journal();
        let mut driver = Scripted::default();
        let mut poller = DriverPoller::new();
        driver.replies = vec![CallResponse {
            call_id: ids::call_id(
                ids::delivery_id("x.out>ghost.in", envelope::Id::from_bytes([9; 16])),
                &[0; 32],
            ),
            completion: Completion::process_exit(ProcessExit::Exited {
                exit_code: 0,
                signal: None,
            }),
        }];
        let batch = poller
            .poll_drivers(
                &mut journal,
                &mut driver,
                &AcceptAnyCompletion,
                crate::time::LogicalTime::ZERO,
            )
            .unwrap();
        assert!(batch.events.is_empty());
        assert_eq!(batch.uncorrelated.len(), 1, "correlation faults surface");
    }
}
