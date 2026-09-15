//! Contract-derived Call completion, frame production and repetition safety.

use alloc::string::{String, ToString};
use core::fmt;

use super::{ConnectorExchange, ConnectorRoles, ContractId};
use crate::node::{Effect, EffectClass};
use crate::plan::LoopCommandKind;

#[cfg(test)]
mod tests;

/// What a terminal outcome proves about external visibility, not business success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalVisibility {
    /// The Contract proves the external operation did not begin.
    BeforeEffect,
    /// Visibility cannot be ruled out, including cases used at several stages.
    OutcomeUnknown,
    /// The operation became externally visible.
    AfterEffect,
}

impl TerminalVisibility {
    /// Repeating an operation is safe only before visibility or with the
    /// guarantee supplied by its original validated Contract/plan.
    #[must_use]
    pub const fn allows_repetition(self, idempotent: bool) -> bool {
        matches!(self, Self::BeforeEffect) || idempotent
    }

    /// Compensation is an undo protocol, not permission to repeat the operation.
    #[must_use]
    pub const fn allows_recurrence(self, effect: Effect) -> bool {
        self.allows_repetition(matches!(
            effect,
            Effect::None | Effect::Class(EffectClass::Idempotent)
        ))
    }
}

impl fmt::Display for TerminalVisibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::BeforeEffect => "before_effect",
            Self::OutcomeUnknown => "outcome_unknown",
            Self::AfterEffect => "after_effect",
        })
    }
}

/// Number of frames on a port in one Call, before its sole terminal exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameProduction {
    /// This resolved profile never produces the port.
    Never,
    /// An ordered stream, without an at-most-one guarantee.
    ZeroOrMore,
    /// Exactly one frame iff the terminal is in this canonical case set;
    /// otherwise none. The Contract owns this nonempty, sorted, unique set.
    ExactlyOneOn(&'static [&'static str]),
}

impl FrameProduction {
    /// Checks the frame count after the terminal itself has been validated.
    #[must_use]
    pub fn accepts_count(self, terminal: &str, count: usize) -> bool {
        match self {
            Self::Never => count == 0,
            Self::ZeroOrMore => true,
            Self::ExactlyOneOn(cases) => count == usize::from(cases.contains(&terminal)),
        }
    }
}

/// Closed facts derived from an exact Contract and its selected profile.
///
/// Construction is private: transported or author-supplied production claims
/// cannot manufacture a more permissive profile. The terminal is exactly once
/// after every frame; request admission is exactly once per application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallSemantics {
    request: &'static str,
    terminal: &'static str,
    frames: &'static [(&'static str, FrameProduction)],
    cases: &'static [(&'static str, TerminalVisibility)],
}

impl CallSemantics {
    /// The sole outbound request input.
    #[must_use]
    pub const fn request(self) -> &'static str {
        self.request
    }

    /// The exactly-once terminal output, after all frames.
    #[must_use]
    pub const fn terminal(self) -> &'static str {
        self.terminal
    }

    /// Closed case inventory, in canonical case-name order.
    #[must_use]
    pub const fn terminal_cases(self) -> &'static [(&'static str, TerminalVisibility)] {
        self.cases
    }

    /// Production on a non-terminal output; absent means not a frame port.
    #[must_use]
    pub fn frame_production(self, port: &str) -> Option<FrameProduction> {
        self.frames
            .iter()
            .find_map(|(name, production)| (*name == port).then_some(*production))
    }

    /// Validates a canonical Retry selection against this exact Call profile.
    /// The caller resolves the plan-owned guarantee; neither an Intent nor a
    /// Driver can supply it. An empty selection never requests repetition.
    ///
    /// # Errors
    ///
    /// Refuses noncanonical selections, unknown terminals and unsafe repeated
    /// operations. This does not grant admission to an unfinished manager.
    pub fn validate_retry_cases(
        self,
        cases: &[String],
        idempotent: bool,
    ) -> Result<(), RetryCaseError> {
        if cases.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(RetryCaseError::Noncanonical);
        }
        for case in cases {
            let (_, visibility) = self
                .cases
                .iter()
                .find(|(name, _)| *name == case)
                .ok_or_else(|| RetryCaseError::Unknown(case.clone()))?;
            if !visibility.allows_repetition(idempotent) {
                return Err(RetryCaseError::Unsafe {
                    case: case.clone(),
                    visibility: *visibility,
                });
            }
        }
        Ok(())
    }

    /// Proves that a direct command from this Call's own output is bounded and
    /// that recurrence is repeat-safe. A frame cannot bypass the terminal exit.
    ///
    /// The caller must separately prove routing closure, command uniqueness
    /// across ports, exact run context, Next's type, and a finite Call timeout.
    /// No command is produced by this validation operation.
    ///
    /// # Errors
    ///
    /// Refuses an unknown port/case, a potentially repeated frame verdict, or
    /// an unsafe recurrence. `None` selects every possible terminal case.
    pub fn validate_command(
        self,
        port: &str,
        cases: Option<&[String]>,
        command: LoopCommandKind,
        effect: Effect,
    ) -> Result<(), CallCommandError> {
        if port == self.terminal {
            if let Some(cases) = cases {
                for case in cases {
                    let (_, visibility) = self
                        .cases
                        .iter()
                        .find(|(name, _)| *name == case)
                        .ok_or_else(|| CallCommandError::UnknownCase(case.clone()))?;
                    check_recurrence(*visibility, command, effect, case)?;
                }
            } else {
                for (case, visibility) in self.cases {
                    check_recurrence(*visibility, command, effect, case)?;
                }
            }
            return Ok(());
        }
        match self
            .frame_production(port)
            .ok_or(CallCommandError::UnknownPort)?
        {
            FrameProduction::Never => Ok(()),
            FrameProduction::ZeroOrMore => Err(CallCommandError::RepeatedFrames),
            FrameProduction::ExactlyOneOn(cases) => {
                for case in cases {
                    check_recurrence(TerminalVisibility::AfterEffect, command, effect, case)?;
                }
                Ok(())
            }
        }
    }
}

fn check_recurrence(
    visibility: TerminalVisibility,
    command: LoopCommandKind,
    effect: Effect,
    case: &str,
) -> Result<(), CallCommandError> {
    if matches!(command, LoopCommandKind::Next | LoopCommandKind::Again)
        && !visibility.allows_recurrence(effect)
    {
        return Err(CallCommandError::UnsafeRecurrence {
            case: case.to_string(),
            visibility,
        });
    }
    Ok(())
}

/// A direct Call-to-manager route contradicts the Contract's guarantees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallCommandError {
    /// The named output is neither a frame nor the terminal.
    UnknownPort,
    /// A selected case is absent from the closed terminal union.
    UnknownCase(String),
    /// More than one frame could issue the command in one application.
    RepeatedFrames,
    /// The effect class cannot justify repeating after this visibility stage.
    UnsafeRecurrence {
        /// Terminal case establishing the stage.
        case: String,
        /// Conservative external visibility classification.
        visibility: TerminalVisibility,
    },
}

impl fmt::Display for CallCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownPort => f.write_str("the Contract declares no such Call output"),
            Self::UnknownCase(case) => write!(f, "the Contract declares no terminal case `{case}`"),
            Self::RepeatedFrames => f.write_str("this Contract may produce several frames on the port; a Loop application needs exactly one verdict"),
            Self::UnsafeRecurrence { case, visibility } => {
                write!(f, "repeating after `{case}` ({visibility}) requires effect = none or idempotent")
            }
        }
    }
}

impl core::error::Error for CallCommandError {}

/// A static Retry selection contradicts its Call Contract or guarantee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryCaseError {
    /// Case names must be sorted and unique before validation.
    Noncanonical,
    /// A name is outside this Contract's closed terminal domain.
    Unknown(String),
    /// The original operation has no guarantee permitting this repetition.
    Unsafe {
        /// Selected terminal case.
        case: String,
        /// Contract-owned conservative visibility.
        visibility: TerminalVisibility,
    },
}

impl fmt::Display for RetryCaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Noncanonical => f.write_str("Retry cases must be sorted and unique"),
            Self::Unknown(case) => write!(f, "the Contract declares no terminal case `{case}`"),
            Self::Unsafe { case, visibility } => write!(
                f,
                "Retry after `{case}` ({visibility}) requires idempotent = true"
            ),
        }
    }
}

impl core::error::Error for RetryCaseError {}

/// Resolves the standard finite-profile Call facts without a Driver registry.
///
/// Exact identity, direction, lifecycle and selected mode must agree. Unknown
/// contracts and unsupported/service profiles receive no inferred guarantees.
/// Timeout values and output types are separate validation obligations.
#[must_use]
pub fn outbound_call_semantics(
    contract: &ContractId,
    roles: ConnectorRoles,
    exchange: ConnectorExchange,
    mode: Option<&str>,
) -> Option<CallSemantics> {
    use FrameProduction::{ExactlyOneOn, Never, ZeroOrMore};
    use TerminalVisibility::{AfterEffect, BeforeEffect, OutcomeUnknown};

    if roles != ConnectorRoles::Outbound || exchange != ConnectorExchange::Call {
        return None;
    }
    let (frames, cases): (&'static [_], &'static [_]) = match (contract.as_str(), mode) {
        (super::PROCESS_CONNECTOR_ID, Some("job")) => (
            &[("err", ZeroOrMore), ("out", ZeroOrMore)],
            &[
                ("CaptureFailed", OutcomeUnknown),
                ("DecodeFailed", OutcomeUnknown),
                ("Exited", AfterEffect),
                ("OutcomeUnknown", OutcomeUnknown),
                ("OutputOverflow", OutcomeUnknown),
                ("SpawnFailed", BeforeEffect),
                ("TimeoutExpired", OutcomeUnknown),
            ],
        ),
        (super::FILE_CONNECTOR_ID, Some("read" | "replace")) => (
            if mode == Some("read") {
                &[("out", ExactlyOneOn(&["Completed"]))]
            } else {
                &[("out", Never)]
            },
            &[
                ("AtomicReplaceUnavailable", BeforeEffect),
                ("Completed", AfterEffect),
                ("InvalidPath", BeforeEffect),
                ("IoFailed", OutcomeUnknown),
                ("NotAFile", BeforeEffect),
                ("NotFound", BeforeEffect),
                ("OutcomeUnknown", OutcomeUnknown),
                ("PermissionDenied", BeforeEffect),
                ("ReadLimitExceeded", AfterEffect),
                ("TimeoutExpired", OutcomeUnknown),
            ],
        ),
        (super::HTTP_CLIENT_CONNECTOR_ID, None) => (
            &[("out", ExactlyOneOn(&["Completed"]))],
            &[
                ("BodyTooLarge", OutcomeUnknown),
                ("Completed", AfterEffect),
                ("ConnectFailed", BeforeEffect),
                ("DecodeFailed", OutcomeUnknown),
                ("DnsFailed", BeforeEffect),
                ("OutcomeUnknown", OutcomeUnknown),
                ("ProtocolFailed", OutcomeUnknown),
                ("TimeoutExpired", OutcomeUnknown),
                ("TlsFailed", BeforeEffect),
            ],
        ),
        _ => return None,
    };
    Some(CallSemantics {
        request: "in",
        terminal: "exit",
        frames,
        cases,
    })
}
