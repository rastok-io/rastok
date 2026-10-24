//! Connector Driver interfaces.
//!
//! A Driver exposes [`InboundRole`](crate::connector::driver::InboundRole) and
//! [`OutboundRole`](crate::connector::driver::OutboundRole) as required by its
//! Contract, over one implementation object.
//! [`OutboundDrivers`](crate::connector::driver::OutboundDrivers) is the separate
//! routing interface that Execution uses to reach all selected implementations.

use crate::connector::exchange::*;
use crate::envelope::{self, Envelope};
use crate::node;
use alloc::{string::String, vec::Vec};
use core::fmt;

/// One bound implementation object, potentially exposing both roles over the
/// same state.
///
/// Returning role views from one object makes the duplex invariant explicit:
/// a both-role factory cannot accidentally register two unrelated states.
pub trait Driver {
    /// The inbound role view, when implemented. Presence is immutable for the
    /// lifetime of this bound object; returning `None` after binding is an
    /// integrity fault.
    fn inbound_role(&mut self) -> Option<&mut dyn InboundRole>;

    /// The outbound role view under the same stable-presence invariant.
    fn outbound_role(&mut self) -> Option<&mut dyn OutboundRole>;
}

/// A route through an already-verified binding became impossible.
///
/// These errors are machine-integrity faults, not domain outcomes. The global
/// binder proves every route before the Execution and journal exist; retaining a
/// typed check here prevents a forged or internally inconsistent caller from
/// becoming a silent no-op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverRouteError {
    /// No outbound binding owns this planned Connector path.
    UnboundOutbound(node::Path),
    /// A role view proven at bind time disappeared later.
    UnavailableOutbound(node::Path),
    /// A Call exchange view proven at bind time disappeared later, or a
    /// Session-only Driver was routed through the Call machine.
    UnavailableCall(node::Path),
    /// No outbound Session binding owns this planned Connector path.
    UnboundSession(node::Path),
    /// A Session exchange view proven at bind time disappeared later.
    UnavailableSession(node::Path),
    /// An inbound role view proven at bind time disappeared later.
    UnavailableInbound(crate::connector::ContractId),
    /// An inbound Call view proven at bind time disappeared later.
    UnavailableInboundCall(crate::connector::ContractId),
    /// A caller attempted to decide a proposal other than the one retained
    /// by that Connector family.
    UnknownIngressProposal {
        /// Family whose retained proposal was addressed.
        connector: crate::connector::ContractId,
        /// Proposal reference supplied by the caller.
        reference: IngressProposalHandle,
    },
    /// The inbound proposal's session framing contradicts the bound
    /// contract's categorical exchange lifecycle.
    InboundExchangeMismatch {
        /// Driver family that proposed the fact.
        connector: crate::connector::ContractId,
        /// Exchange required by the validated plan.
        exchange: crate::connector::ExchangeKind,
    },
    /// A family proposed a root from a Connector node it did not bind.
    InboundSourceMismatch {
        /// Driver family that made the proposal.
        connector: crate::connector::ContractId,
        /// Proposed Connector node.
        source: node::Path,
    },
    /// One Driver answered a Call owned by another implementation object.
    CallOwnerMismatch {
        /// Correlated call.
        call_id: CallId,
        /// Description of the Driver object that started it.
        expected: String,
        /// Description of the Driver object that answered it.
        actual: String,
    },
    /// One Driver reported a terminal fact for an inbound Call admitted by
    /// another implementation object.
    InboundCallOwnerMismatch {
        /// Correlated inbound Call.
        call_id: InboundCallId,
        /// Description of the Driver object that admitted it.
        expected: String,
        /// Description of the Driver object that reported the observation.
        actual: String,
    },
    /// A Driver reported a terminal fact for no admitted inbound Call.
    UncorrelatedInboundCall(InboundCallId),
    /// One Driver reported a Session observation owned by another object.
    SessionOwnerMismatch {
        /// Correlated logical session.
        session: SessionId,
        /// Description of the Driver object that admitted or opened it.
        expected: String,
        /// Description of the Driver object that reported the observation.
        actual: String,
    },
    /// A Driver reported an observation for no admitted/opened session.
    UncorrelatedSession(SessionId),
}

impl fmt::Display for DriverRouteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnboundOutbound(connector) => {
                write!(formatter, "no outbound Driver binding owns `{connector}`")
            }
            Self::UnavailableOutbound(connector) => write!(
                formatter,
                "the outbound Driver role bound for `{connector}` is no longer available"
            ),
            Self::UnavailableCall(connector) => write!(
                formatter,
                "the Call exchange Driver bound for `{connector}` is no longer available"
            ),
            Self::UnboundSession(connector) => {
                write!(formatter, "no Session Driver binding owns `{connector}`")
            }
            Self::UnavailableSession(connector) => write!(
                formatter,
                "the Session exchange Driver bound for `{connector}` is no longer available"
            ),
            Self::UnavailableInbound(id) => write!(
                formatter,
                "the inbound Driver role bound for `{id}` is no longer available"
            ),
            Self::UnavailableInboundCall(id) => write!(
                formatter,
                "the inbound Call Driver bound for `{id}` is no longer available"
            ),
            Self::UnknownIngressProposal {
                connector,
                reference,
            } => write!(
                formatter,
                "Driver `{connector}` has no retained inbound proposal {}",
                reference.get()
            ),
            Self::InboundExchangeMismatch {
                connector,
                exchange,
            } => write!(
                formatter,
                "Driver `{connector}` proposed inbound framing outside its {exchange} exchange"
            ),
            Self::InboundSourceMismatch { connector, source } => write!(
                formatter,
                "Driver `{connector}` proposed an inbound root from unbound source `{source}`"
            ),
            Self::CallOwnerMismatch {
                call_id,
                expected,
                actual,
            } => write!(
                formatter,
                "Driver `{actual}` answered call {call_id} started by `{expected}`"
            ),
            Self::InboundCallOwnerMismatch {
                call_id,
                expected,
                actual,
            } => write!(
                formatter,
                "Driver `{actual}` terminated inbound call {call_id} admitted by `{expected}`"
            ),
            Self::UncorrelatedInboundCall(call_id) => {
                write!(formatter, "no bound Driver owns inbound call {call_id}")
            }
            Self::SessionOwnerMismatch {
                session,
                expected,
                actual,
            } => write!(
                formatter,
                "Driver `{actual}` reported session {session} owned by `{expected}`"
            ),
            Self::UncorrelatedSession(session) => {
                write!(formatter, "no bound Driver owns session {session}")
            }
        }
    }
}

impl core::error::Error for DriverRouteError {}

/// The inbound Driver role: supplies bounded ingress proposals under backpressure.
/// The Driver retains each proposal until the runtime reports acceptance or
/// final rejection; only journal admission assigns its envelope and exchange
/// identities.
pub trait InboundRole {
    /// The next retained proposal, if one is waiting. Repeated polls before a
    /// decision must describe the same handle and proposal contents.
    fn poll_proposal(&mut self) -> Option<PendingIngress>;

    /// Releases one retained proposal after a committed acceptance or a
    /// contract rejection. Pending admission deliberately makes no callback.
    fn notify_admission(
        &mut self,
        reference: IngressProposalHandle,
        decision: Result<AdmittedIngress, IngressRejection>,
    );

    /// Inbound Call completion view, when this bound contract is Call-shaped.
    /// Event and Session Drivers leave the default absent.
    fn call(&mut self) -> Option<&mut dyn InboundCall> {
        None
    }
}

/// Terminal-observation half of an inbound Call Driver.
///
/// Admission itself remains on [`InboundRole`]. Keeping this as a
/// categorical view makes an Event source unable to manufacture Call exits.
pub trait InboundCall {
    /// Starts one graph response whose producing transaction is already
    /// durable. Ack-only Drivers return [`InboundCallResponseStart::Pending`]
    /// only on plans that should have been rejected before binding.
    fn start_response(&mut self, response: &InboundCallResponse) -> InboundCallResponseStart;

    /// Starts the protocol-side timeout action selected by the scheduler.
    /// A graph response already owned by the Driver may win this race; the
    /// sole later event states which outcome became terminal.
    fn start_timeout(&mut self, timeout: &InboundCallTimeout) -> InboundCallTimeoutStart;

    /// Terminal facts observed since the previous bounded drain.
    fn poll_completions(&mut self) -> Vec<InboundCallObservation>;
}

/// The outbound Driver role: exposes the Call or Session interface required by
/// the bound Contract's exchange kind.
pub trait OutboundRole {
    /// Call protocol view, when this bound contract is Call-shaped.
    fn call(&mut self) -> Option<&mut dyn OutboundCall>;

    /// Session protocol view, when this bound contract is Session-shaped.
    fn session(&mut self) -> Option<&mut dyn Session>;
}

/// Starts external calls and supplies their answers. A Call driver
/// answers or stays silent — never a timeout and never an unknown
/// outcome: budgets belong to the scheduler, triage to the recovery
/// fold. Correlation uses the connector's own protocol.
pub trait OutboundCall {
    /// Frames one invocation under this bound contract before the machine
    /// opens its EGRESS record. The Driver chooses no defaults: binding has
    /// already resolved every effective non-plan setting for `connector`.
    fn prepare_request(
        &mut self,
        connector: &node::Path,
        data: &envelope::Data,
        resolved_settings: &alloc::collections::BTreeMap<String, crate::value::Value>,
    ) -> RequestPreparation;

    /// Builds the contract-typed representation of a machine-decided
    /// terminal fact. The scheduler decides that the fact occurred; this
    /// method only projects it into the Connector's ordinary output protocol.
    fn scheduler_completion(
        &mut self,
        connector: &node::Path,
        completion: SchedulerCompletion,
    ) -> Completion;

    /// Withdraws an open call: the machine closed it (a won timeout)
    /// and any in-flight work should stop. Default: nothing — a
    /// scripted world has nothing to stop.
    fn cancel(&mut self, call_id: &CallId) {
        let _ = call_id;
    }

    /// Offers an already-journaled open call for execution. A bounded Driver
    /// may leave it pending without starting an effect; the same intent is
    /// then offered again. The request envelope rides along because the
    /// intent seals only the digest of the request, not its bytes.
    fn start(&mut self, intent: &CallOpen, request: &Envelope) -> CallStart;

    /// The answers that arrived since the last drain.
    fn poll_responses(&mut self) -> Vec<CallResponse>;
}

impl<T> OutboundRole for T
where
    T: OutboundCall,
{
    fn call(&mut self) -> Option<&mut dyn OutboundCall> {
        Some(self)
    }

    fn session(&mut self) -> Option<&mut dyn Session> {
        None
    }
}

/// Synchronous effect boundary for one long-lived exchange.
///
/// Inbound peer messages use [`InboundRole`]'s retained proposal
/// protocol over the same bound object. These methods cover the outbound
/// opening/write half without adding a third role or async to the core.
pub trait Session {
    /// Frames one opening handshake or returns its typed terminal refusal.
    fn prepare_session_open(
        &mut self,
        connector: &node::Path,
        data: &envelope::Data,
        resolved_settings: &alloc::collections::BTreeMap<String, crate::value::Value>,
    ) -> RequestPreparation;

    /// Projects one machine-decided session terminal through this contract.
    fn scheduler_session_completion(
        &mut self,
        connector: &node::Path,
        completion: SchedulerCompletion,
    ) -> Completion;

    /// Executes an already-journaled opening intent.
    fn open_session(&mut self, intent: &SessionOpen, request: &Envelope) -> SessionStart;

    /// Frames one complete graph-to-world session message.
    fn prepare_session_write(
        &mut self,
        connector: &node::Path,
        session: SessionId,
        data: &envelope::Data,
        resolved_settings: &alloc::collections::BTreeMap<String, crate::value::Value>,
    ) -> RequestPreparation;

    /// Offers one already-journaled complete message to the protocol stack.
    fn write_session(&mut self, intent: &SessionWrite, message: &Envelope) -> SessionStart;

    /// Requests protocol closure. Default: no local action is needed.
    fn close_session(&mut self, session: SessionId) -> SessionStart {
        let _ = session;
        SessionStart::Started
    }

    /// Requests forced local shutdown at the retained Closing settlement
    /// deadline. Unlike protocol close, this handoff cannot wait for peer
    /// cooperation or a free message slot. It is idempotent for this exact
    /// resource and may be repeated after recovery of the closing scope.
    ///
    /// This does not prove an exit or release journal correlation. A physical
    /// Driver must retain its cleanup request without blocking the core; a
    /// resource-free scripted world has nothing to stop.
    fn abort_session(&mut self, session: SessionId) {
        let _ = session;
    }

    /// Releases local resources after the Session's terminal transaction
    /// commits, including a scheduler-owned unknown outcome. This is not a
    /// second protocol close and cannot publish another terminal or start a
    /// grace period. The exact reference fences any successor resource.
    ///
    /// Handoff must not block on the peer or child. A physical Driver owns
    /// any remaining cleanup; a resource-free scripted world has none.
    fn release_session(&mut self, session: SessionId) {
        let _ = session;
    }

    /// Session observations accumulated since the previous drain.
    fn poll_observations(&mut self) -> Vec<SessionObservation>;
}

/// The verified outbound routing table consumed by the core Execution.
///
/// The hosted `RuntimeDrivers` implementation selects among prepared Drivers
/// and returns a typed error if its pre-execution proof is ever contradicted. The Execution
/// therefore never matches `Process`, `File`, or another concrete contract.
/// Execution entry points borrow this object-safe boundary dynamically: the scheduler
/// and recovery code are shared across Driver bindings rather than duplicated
/// for each binding implementation in an embedded image.
pub trait OutboundDrivers {
    /// Frames one request through the Driver bound to `connector`.
    ///
    /// # Errors
    ///
    /// The planned path has no stable outbound route.
    fn prepare_request(
        &mut self,
        connector: &node::Path,
        data: &envelope::Data,
        resolved_settings: &alloc::collections::BTreeMap<String, crate::value::Value>,
    ) -> Result<RequestPreparation, DriverRouteError>;

    /// Projects a scheduler fact through the bound contract.
    ///
    /// # Errors
    ///
    /// The planned path has no stable outbound route.
    fn scheduler_completion(
        &mut self,
        connector: &node::Path,
        completion: SchedulerCompletion,
    ) -> Result<Completion, DriverRouteError>;

    /// Withdraws one call through its owning binding.
    ///
    /// # Errors
    ///
    /// The planned path has no stable outbound route.
    fn cancel(&mut self, connector: &node::Path, call_id: &CallId) -> Result<(), DriverRouteError>;

    /// Starts one already-journaled call through its owning binding.
    ///
    /// # Errors
    ///
    /// The planned path has no stable outbound route.
    fn start(
        &mut self,
        intent: &CallOpen,
        request: &Envelope,
    ) -> Result<CallStart, DriverRouteError>;

    /// Collects all answers since the previous round.
    ///
    /// # Errors
    ///
    /// A binding invariant was contradicted while polling.
    fn poll_responses(&mut self) -> Result<Vec<CallResponse>, DriverRouteError>;

    /// Frames one Session opening through the exchange-specific binding.
    fn prepare_session_open(
        &mut self,
        connector: &node::Path,
        data: &envelope::Data,
        resolved_settings: &alloc::collections::BTreeMap<String, crate::value::Value>,
    ) -> Result<RequestPreparation, DriverRouteError> {
        let _ = (data, resolved_settings);
        Err(DriverRouteError::UnboundSession(connector.clone()))
    }

    /// Projects a scheduler-owned Session terminal through its contract.
    fn scheduler_session_completion(
        &mut self,
        connector: &node::Path,
        completion: SchedulerCompletion,
    ) -> Result<Completion, DriverRouteError> {
        let _ = completion;
        Err(DriverRouteError::UnboundSession(connector.clone()))
    }

    /// Offers one already-journaled Session opening.
    fn open_session(
        &mut self,
        intent: &SessionOpen,
        request: &Envelope,
    ) -> Result<SessionStart, DriverRouteError> {
        let _ = request;
        Err(DriverRouteError::UnboundSession(
            intent.connector_path.clone(),
        ))
    }

    /// Frames one complete Session message.
    fn prepare_session_write(
        &mut self,
        connector: &node::Path,
        session: SessionId,
        data: &envelope::Data,
        resolved_settings: &alloc::collections::BTreeMap<String, crate::value::Value>,
    ) -> Result<RequestPreparation, DriverRouteError> {
        let _ = (session, data, resolved_settings);
        Err(DriverRouteError::UnboundSession(connector.clone()))
    }

    /// Offers one already-journaled Session write.
    fn write_session(
        &mut self,
        intent: &SessionWrite,
        message: &Envelope,
    ) -> Result<SessionStart, DriverRouteError> {
        let _ = message;
        Err(DriverRouteError::UnboundSession(
            intent.connector_path.clone(),
        ))
    }

    /// Requests closure of one live Session.
    fn close_session(
        &mut self,
        connector: &node::Path,
        session: SessionId,
    ) -> Result<SessionStart, DriverRouteError> {
        let _ = session;
        Err(DriverRouteError::UnboundSession(connector.clone()))
    }

    /// Requests forced local shutdown after a retained Closing deadline.
    fn abort_session(
        &mut self,
        connector: &node::Path,
        session: SessionId,
    ) -> Result<(), DriverRouteError> {
        let _ = session;
        Err(DriverRouteError::UnboundSession(connector.clone()))
    }

    /// Collects Session observations from every bound family.
    fn poll_observations(&mut self) -> Result<Vec<SessionObservation>, DriverRouteError> {
        Ok(Vec::new())
    }

    /// Releases one committed terminal Session's local resources and routing
    /// state. It must not perform another protocol exchange.
    fn forget_session(&mut self, session: SessionId) -> Result<(), DriverRouteError> {
        let _ = session;
        Ok(())
    }
}
