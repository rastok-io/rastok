//! Stable Connector identities, role requirements, and exchange lifecycles.
//!
//! A Connector id is semantic plan data.  It names a contract independently
//! of the crate, Driver binary, Host or Worker that implements it. Roles and
//! exchange lifecycles are likewise contract facts: runtime registration may
//! satisfy them, but may not reinterpret them.

use self::stdlib::{
    PROCESS_CONNECTOR_ID, WEBSOCKET_CLIENT_CONNECTOR_ID, WEBSOCKET_SERVER_CONNECTOR_ID,
};

use alloc::boxed::Box;
use core::fmt;

mod call;

pub use call::{
    CallCommandError, CallSemantics, FrameProduction, RetryCaseError, TerminalVisibility,
    outbound_call_semantics,
};

/// Portable upper bound of one declared byte budget: 4 GiB inclusive.
pub const MAX_BYTE_SIZE: u64 = 4 * 1024 * 1024 * 1024;

/// Parses one exact portable byte-size spelling shared by sheets, package
/// validation and Drivers.
#[must_use]
pub fn byte_size(text: &str) -> Option<u64> {
    let text = text.trim_matches(|character: char| character.is_ascii_whitespace());
    let split = text.find(|character: char| !character.is_ascii_digit())?;
    let (digits, suffix) = text.split_at(split);
    if digits.is_empty() {
        return None;
    }
    let base: u64 = digits.parse().ok()?;
    let factor: u64 = match suffix.trim_matches(|character: char| character.is_ascii_whitespace()) {
        "B" => 1,
        "KiB" => 1024,
        "MiB" => 1024 * 1024,
        "GiB" => 1024 * 1024 * 1024,
        _ => return None,
    };
    let size = base.checked_mul(factor)?;
    (size <= MAX_BYTE_SIZE).then_some(size)
}

/// A validated, canonical Connector contract identity.
///
/// The spelling is `<namespace>/<name>@<major>`.  The namespace is one or
/// more dot-separated lowercase labels, with at least one dot outside the
/// standard-library namespace `std`; the name is one lowercase label;
/// labels may contain ASCII digits and internal `-`; the positive major has
/// no leading zero.  The constructor never normalizes aliases.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContractId(Box<str>);

impl ContractId {
    /// Parses one exact canonical spelling.
    ///
    /// # Errors
    ///
    /// Returns the first structural category that makes `spelling`
    /// non-canonical.
    pub fn parse(spelling: &str) -> Result<Self, ContractIdError> {
        let Some((qualified, major)) = spelling.rsplit_once('@') else {
            return Err(ContractIdError::Shape);
        };
        if qualified.contains('@') {
            return Err(ContractIdError::Shape);
        }
        let Some((namespace, name)) = qualified.split_once('/') else {
            return Err(ContractIdError::Shape);
        };
        if name.contains('/') {
            return Err(ContractIdError::Shape);
        }
        if (namespace != "std" && !namespace.contains('.'))
            || namespace.split('.').any(|label| !canonical_label(label))
        {
            return Err(ContractIdError::Namespace);
        }
        if !canonical_label(name) {
            return Err(ContractIdError::Name);
        }
        if major.is_empty()
            || major.starts_with('0')
            || !major.bytes().all(|byte| byte.is_ascii_digit())
            || major
                .parse::<u32>()
                .ok()
                .filter(|value| *value > 0)
                .is_none()
        {
            return Err(ContractIdError::Major);
        }
        Ok(Self(Box::from(spelling)))
    }

    /// The exact canonical spelling transported by the plan.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn canonical_label(label: &str) -> bool {
    let bytes = label.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_lowercase)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

impl fmt::Display for ContractId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<&str> for ContractId {
    type Error = ContractIdError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

/// Why a Connector contract identity is not canonical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractIdError {
    /// The `/` and `@` framing is absent or ambiguous.
    Shape,
    /// The namespace is not a dot-separated sequence of canonical labels.
    Namespace,
    /// The contract name is not one canonical label.
    Name,
    /// The major is not a positive canonical `u32` decimal.
    Major,
}

impl fmt::Display for ContractIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Shape => "expected exactly `<namespace>/<name>@<major>`",
            Self::Namespace => "expected `std` or canonical lowercase dotted namespace",
            Self::Name => "the name is not one canonical lowercase label",
            Self::Major => "the major is not a positive canonical u32 decimal",
        })
    }
}

impl core::error::Error for ContractIdError {}

/// The cardinality and terminal lifecycle of one Connector interaction.
///
/// This is orthogonal to [`ConnectorDirection`]: roles say which side may move a
/// fact across the world boundary, while this value says whether those facts
/// are independent events, one bounded call, or members of one long-lived
/// session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ExchangeKind {
    /// One spontaneous INGRESS fact with no per-event terminal obligation.
    Event,
    /// One admitted operation with zero or more outputs and exactly one exit.
    Call,
    /// One open scope carrying messages and exactly one terminal exit.
    Session,
}

impl ExchangeKind {
    /// Stable categorical tag used by semantic identity and plan transport.
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Event => 1,
            Self::Call => 2,
            Self::Session => 3,
        }
    }

    /// Decodes one stable categorical tag.
    #[must_use]
    pub const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            1 => Some(Self::Event),
            2 => Some(Self::Call),
            3 => Some(Self::Session),
            _ => None,
        }
    }
}

impl fmt::Display for ExchangeKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Event => "event",
            Self::Call => "call",
            Self::Session => "session",
        })
    }
}

/// Directions in which a Connector exchanges data with external systems,
/// relative to the graph. Every Connector requires at least one direction;
/// a bidirectional Driver exposes both through the same implementation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ConnectorDirection {
    /// Proposes external arrivals to Rastok.
    Inbound,
    /// Executes already-journaled calls into the external world.
    Outbound,
    /// One implementation state exposes both roles.
    InboundAndOutbound,
}

impl ConnectorDirection {
    /// Stable categorical tag used by semantic identity and plan transport.
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Inbound => 1,
            Self::Outbound => 2,
            Self::InboundAndOutbound => 3,
        }
    }

    /// Decodes one stable categorical tag.
    #[must_use]
    pub const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            1 => Some(Self::Inbound),
            2 => Some(Self::Outbound),
            3 => Some(Self::InboundAndOutbound),
            _ => None,
        }
    }

    /// Whether these available roles satisfy all `required` roles.
    #[must_use]
    pub const fn includes(self, required: Self) -> bool {
        matches!(
            (self, required),
            (Self::InboundAndOutbound, _)
                | (Self::Inbound, Self::Inbound)
                | (Self::Outbound, Self::Outbound)
        )
    }

    /// Whether the contract has the inbound role.
    #[must_use]
    pub const fn has_inbound(self) -> bool {
        matches!(self, Self::Inbound | Self::InboundAndOutbound)
    }

    /// Whether the contract has the outbound role.
    #[must_use]
    pub const fn has_outbound(self) -> bool {
        matches!(self, Self::Outbound | Self::InboundAndOutbound)
    }
}

impl fmt::Display for ConnectorDirection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Inbound => "inbound",
            Self::Outbound => "outbound",
            Self::InboundAndOutbound => "inbound+outbound",
        })
    }
}

/// Which side may create the resource, independently of message direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionOpening {
    Local,
    Peer,
}

/// Duplex roles alone cannot distinguish a listener from a client or child.
/// Unknown Contracts acquire no opening authority by resemblance.
pub(crate) fn session_opening(
    contract: &ContractId,
    roles: ConnectorDirection,
    exchange: ExchangeKind,
) -> Option<SessionOpening> {
    if roles != ConnectorDirection::InboundAndOutbound || exchange != ExchangeKind::Session {
        return None;
    }
    match contract.as_str() {
        WEBSOCKET_SERVER_CONNECTOR_ID => Some(SessionOpening::Peer),
        WEBSOCKET_CLIENT_CONNECTOR_ID | PROCESS_CONNECTOR_ID => Some(SessionOpening::Local),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connector::stdlib::{
        FILE_CONNECTOR_ID, HTTP_CLIENT_CONNECTOR_ID, HTTP_SERVER_CONNECTOR_ID, PROCESS_CONNECTOR_ID,
    };

    #[test]
    fn session_opening_comes_from_the_exact_contract_not_duplex_roles() {
        for (id, opening) in [
            (WEBSOCKET_SERVER_CONNECTOR_ID, Some(SessionOpening::Peer)),
            (WEBSOCKET_CLIENT_CONNECTOR_ID, Some(SessionOpening::Local)),
            (PROCESS_CONNECTOR_ID, Some(SessionOpening::Local)),
            (HTTP_CLIENT_CONNECTOR_ID, None),
            (HTTP_SERVER_CONNECTOR_ID, None),
            (FILE_CONNECTOR_ID, None),
            ("std/websocket-server@99", None),
            ("org.third-party/websocket-server@2", None),
        ] {
            let contract = ContractId::parse(id).unwrap();
            for roles in [
                ConnectorDirection::Inbound,
                ConnectorDirection::Outbound,
                ConnectorDirection::InboundAndOutbound,
            ] {
                for exchange in [
                    ExchangeKind::Event,
                    ExchangeKind::Call,
                    ExchangeKind::Session,
                ] {
                    let expected = if roles == ConnectorDirection::InboundAndOutbound
                        && exchange == ExchangeKind::Session
                    {
                        opening
                    } else {
                        None
                    };
                    assert_eq!(
                        session_opening(&contract, roles, exchange),
                        expected,
                        "{id}"
                    );
                }
            }
        }
    }

    #[test]
    fn connector_ids_accept_one_canonical_spelling_only() {
        for valid in [
            "std/process@2",
            "org.example/http-client@12",
            "org.a/x1@4294967295",
        ] {
            assert_eq!(ContractId::parse(valid).unwrap().as_str(), valid);
        }
        for (invalid, reason) in [
            ("Process", ContractIdError::Shape),
            ("acme/process@1", ContractIdError::Namespace),
            ("rastok/process@4", ContractIdError::Namespace),
            ("rastok/process", ContractIdError::Shape),
            ("std/process@2@2", ContractIdError::Shape),
            ("rastok/more/process@1", ContractIdError::Shape),
            ("Rastok/process@1", ContractIdError::Namespace),
            ("rastok./process@1", ContractIdError::Namespace),
            ("std/Process@1", ContractIdError::Name),
            ("std/-process@1", ContractIdError::Name),
            ("std/process@0", ContractIdError::Major),
            ("std/process@01", ContractIdError::Major),
            ("std/process@4294967296", ContractIdError::Major),
        ] {
            assert_eq!(ContractId::parse(invalid), Err(reason), "{invalid}");
        }
    }

    #[test]
    fn role_inclusion_has_no_empty_or_partial_duplex_state() {
        assert!(ConnectorDirection::Inbound.includes(ConnectorDirection::Inbound));
        assert!(!ConnectorDirection::Inbound.includes(ConnectorDirection::Outbound));
        assert!(!ConnectorDirection::Inbound.includes(ConnectorDirection::InboundAndOutbound));
        assert!(ConnectorDirection::Outbound.includes(ConnectorDirection::Outbound));
        assert!(ConnectorDirection::InboundAndOutbound.includes(ConnectorDirection::Inbound));
        assert!(ConnectorDirection::InboundAndOutbound.includes(ConnectorDirection::Outbound));
        assert!(
            ConnectorDirection::InboundAndOutbound.includes(ConnectorDirection::InboundAndOutbound)
        );
        for role in [
            ConnectorDirection::Inbound,
            ConnectorDirection::Outbound,
            ConnectorDirection::InboundAndOutbound,
        ] {
            assert_eq!(ConnectorDirection::from_tag(role.tag()), Some(role));
        }
        assert_eq!(ConnectorDirection::from_tag(0), None);
        assert_eq!(ConnectorDirection::from_tag(4), None);
    }

    #[test]
    fn exchange_tags_are_closed_and_extend_only() {
        for exchange in [
            ExchangeKind::Event,
            ExchangeKind::Call,
            ExchangeKind::Session,
        ] {
            assert_eq!(ExchangeKind::from_tag(exchange.tag()), Some(exchange));
        }
        assert_eq!(ExchangeKind::from_tag(0), None);
        assert_eq!(ExchangeKind::from_tag(4), None);
    }
}

pub mod contract;
/// Driver protocol interfaces and execution routing.
pub mod driver;
/// Data exchanged with Connector Drivers.
pub mod exchange;
/// Portable standard Contract definitions.
pub mod stdlib;
